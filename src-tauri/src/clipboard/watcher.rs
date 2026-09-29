//! OS 级剪贴板监听：把 [`clipboard_rs`] 的 watcher 接到「防抖 → 读取 → 去重入库 → emit」闭环。
//!
//! [`clipboard_rs`] 内部已实现 macOS（`NSPasteboard.changeCount` 轮询）/ Windows
//! （`AddClipboardFormatListener` → `WM_CLIPBOARDUPDATE`）的平台监听，这里不重复造。
//!
//! 线程模型：监听跑在两个线程上。
//! - **watcher 线程**：跑 [`ClipboardWatcherContext::start_watch()`]（阻塞调用），回调里
//!   只做「暂停判定 → 抓前台应用 → 过滤名单判定 → 投递信号」，立刻返回，不碰剪贴板数据。
//! - **ingest 线程**：消费信号做防抖（见 [`CLIPBOARD_DEBOUNCE`]），静默后在本线程内
//!   构造 [`ClipboardReader`] 读取并入库。`ClipboardContext` 等平台句柄在该线程内构造，
//!   不跨线程移动，绕开其 `Send` 约束；只有 `Send` 的数据会被投递进 Tauri 异步运行时。
//!
//! 防抖是必须的：不少 Windows 应用分多次写剪贴板（先 `CF_UNICODETEXT`，隔几十毫秒再补
//! `HTML Format` / RTF，每次 `CloseClipboard` 都触发一条 `WM_CLIPBOARDUPDATE`）。若每次
//! 都立即读取入库，同一份内容会先入一条纯文本、再入一条富文本——两条记录可见字符相同
//! 而长度不同。静默窗口内只处理最后状态，多次写入合并为一次采集。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use clipboard_rs::{ClipboardHandler, ClipboardWatcher, ClipboardWatcherContext};
use serde_json::json;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager};

use super::app_store::AppIconStore;
use super::apps_registry::AppsRegistry;
use super::guard::WritebackGuard;
use super::ingest::build_item_with_settings;
use super::read::ClipboardReader;
use super::sound;
use super::source::{self, FrontmostApp};
use super::storage::ImageStore;
use crate::db::apps::upsert_app;
use crate::db::items::{upsert_item, UpsertResult};
use crate::db::models::{ClipboardApp, ClipboardItem};
use crate::settings::SettingsStore;

/// 剪贴板更新事件名。前端监听此事件后增量刷新 / 重新拉取列表。
pub const CLIPBOARD_UPDATED_EVENT: &str = "clipboard://updated";

/// macOS 轮询 `changeCount` 的间隔。上游 clipboard-rs 默认 500ms，对复制响应（尤其图片）
/// 偏慢；fork 后的 `new_with_interval` 调到 120ms，跟手且 CPU 开销可忽略。
/// Windows 走事件驱动（`WM_CLIPBOARDUPDATE`），此值被忽略。
const CLIPBOARD_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(120);

/// 剪贴板变更防抖静默窗口：窗口内再无新变更才读取入库，新变更会重置窗口。
/// 覆盖源应用分多次写剪贴板的间隙（先纯文本、再补 HTML/RTF，通常几十毫秒），
/// 代价是每次复制到入库最多延迟一个窗口。
const CLIPBOARD_DEBOUNCE: Duration = Duration::from_millis(200);

/// 防抖窗口的总上限：持续高频写剪贴板（如两个剪贴板管理器互斗）不能无限饿死入库。
const CLIPBOARD_DEBOUNCE_MAX_WAIT: Duration = Duration::from_millis(1000);

/// Another clipboard listener can briefly hold the Windows clipboard open. Retry those read
/// failures within a bounded window before dropping the update.
const CLIPBOARD_READ_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_millis(15),
    Duration::from_millis(35),
    Duration::from_millis(75),
];

fn read_with_retry<T, E>(
    retry_delays: &[Duration],
    mut read: impl FnMut() -> Result<Option<T>, E>,
) -> Result<Option<T>, E> {
    let mut result = read();
    for delay in retry_delays {
        if result.is_ok() {
            return result;
        }
        std::thread::sleep(*delay);
        result = read();
    }
    result
}

/// 监听暂停开关。托盘菜单「停止监听」翻转，handler 早返回跳过整条入库链路。
/// 用 `Arc<AtomicBool>` 跨线程共享；不停 watcher 线程本身，避免反复重建平台句柄。
#[derive(Debug, Default, Clone)]
pub struct WatcherPause(Arc<AtomicBool>);

impl WatcherPause {
    pub fn is_paused(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    pub fn set_paused(&self, paused: bool) {
        self.0.store(paused, Ordering::Relaxed);
    }
}

/// 把探测到的 [`FrontmostApp`] 拼成可入库的 [`ClipboardApp`]。
///
/// 监听热路径的同步部分只做缓存查询：registry 命中缓存直接复用（多数复制事件
/// 来自已见过的应用，零 OS 调用）；未命中的应用返回无 icon 的记录并返回 `false`，
/// 由调用方在异步上下文里经 [`spawn_materialize_icon`] 补齐落库。
pub fn materialize_source(registry: &AppsRegistry, src: FrontmostApp) -> (ClipboardApp, bool) {
    if let Some(cached) = registry.get(&src.id) {
        return (cached, true);
    }

    let now = Utc::now();
    let app = ClipboardApp {
        id: src.id,
        name: src.name,
        icon_file: None,
        platform: src.platform,
        created_at: now,
        updated_at: now,
    };
    (app, false)
}

/// 异步补齐来源应用 icon 并落库：缓存未命中的应用在 `spawn_blocking` 里抽取 icon
/// （shell 查询 + PNG 编码的毫秒级 OS 调用，不占监听线程），store 落盘失败仅 warn。
/// 完成后回写 registry 缓存与 DB；图标缺失期间前端按应用名回退展示，下次拉取列表可见。
pub fn spawn_materialize_icon(
    app: &AppHandle,
    icon_store: &AppIconStore,
    registry: &AppsRegistry,
    src: FrontmostApp,
) {
    let icon_store = icon_store.clone();
    let registry = registry.clone();
    let app_handle = app.clone();

    tauri::async_runtime::spawn(async move {
        let icon_path = src.icon_path.clone();
        let extract_id = src.id.clone();
        let png = tauri::async_runtime::spawn_blocking(move || {
            icon_path
                .as_deref()
                .and_then(|path| super::icon::icon_png(path, None))
        })
        .await
        .unwrap_or_else(|err| {
            log::warn!("app icon extract task join failed for {extract_id}: {err}");
            None
        });

        let icon_file = png
            .as_deref()
            .and_then(|bytes| match icon_store.store(bytes) {
                Ok(name) => Some(name),
                Err(err) => {
                    log::warn!("app icon store failed for {}: {err}", src.id);
                    None
                }
            });

        // 期间并发的同 id 入库可能已补齐 icon；upsert 走「保留 created_at、刷新
        // name/icon_file」，重复补齐幂等，最后写入者胜出，无害。
        let now = Utc::now();
        let materialized = ClipboardApp {
            id: src.id,
            name: src.name,
            icon_file,
            platform: src.platform,
            created_at: now,
            updated_at: now,
        };
        registry.insert_into_cache(materialized.clone());

        let pool = app_handle.state::<crate::db::DatabaseState>().pool().await;
        if let Err(err) = upsert_app(&pool, &materialized).await {
            log::warn!("app icon upsert failed for {}: {err}", materialized.id);
        }
    });
}

/// 去重入库 + emit「剪贴板更新」事件。监听回调与 `read_clipboard` 命令共用，
/// 保证两条路径的入库语义与事件契约一致。失败仅记日志（监听场景无人接收 Result）。
///
/// `source_app` 为 `Some` 时先 upsert apps 表再写 item，满足 FK 约束。
/// 应用 upsert 失败不阻断条目入库——清掉 source_app_id 后继续，避免单次系统调用抽风丢内容。
/// 按值接 item（避免复制事件热路径上整结构 clone，content 最大 4MB），
/// 返回写入后的 item 供调用方继续消费（如 `read_clipboard` 回显）。
pub async fn persist_and_notify(
    app: &AppHandle,
    pool: &SqlitePool,
    mut item: ClipboardItem,
    source_app: Option<&ClipboardApp>,
) -> crate::core::Result<(UpsertResult, ClipboardItem)> {
    if let Some(src) = source_app {
        match upsert_app(pool, src).await {
            Ok(()) => {}
            Err(err) => {
                log::warn!("clipboard source app upsert failed ({}): {err}", src.id);
                item.source_app_id = None;
            }
        }
    }
    let result = upsert_item(pool, &item).await?;
    sound::maybe_play_copy(app);
    if let Err(err) = app.emit(
        CLIPBOARD_UPDATED_EVENT,
        json!({
            "id": result.id,
            "kind": item.kind,
            "deduplicated": result.deduplicated,
        }),
    ) {
        log::warn!("emit {CLIPBOARD_UPDATED_EVENT} failed: {err}");
    }
    Ok((result, item))
}

/// 启动监听：注册 [`WritebackGuard`] / [`ImageStore`] / [`AppIconStore`] 到 Tauri `State`
/// （供写回时打标记 / 取图 / 取来源应用图标），并在独立线程上跑 OS 级监听。
/// 应在 `setup` 中、连接池就绪后调用一次。store 创建失败属致命配置错误，直接返回错误。
pub fn init(app: &AppHandle) -> crate::core::Result<()> {
    let guard = Arc::new(WritebackGuard::new());
    app.manage(guard.clone());

    let store = ImageStore::new(app)?;
    app.manage(store.clone());

    let app_icon_store = AppIconStore::new(app)?;
    app.manage(app_icon_store.clone());

    let file_icon_store = super::FileIconStore::new(app)?;
    app.manage(file_icon_store);

    let registry = AppsRegistry::new(app.clone(), app_icon_store.clone());
    app.manage(registry.clone());

    let pause = WatcherPause::default();
    app.manage(pause.clone());

    // 启动期只把已落库的应用读进缓存；运行中应用由偏好页打开/刷新时补齐。
    {
        let registry = registry.clone();
        let excluded_app_ids = app
            .try_state::<SettingsStore>()
            .map(|store| store.snapshot().clipboard.filters.excluded_app_ids)
            .unwrap_or_default();
        tauri::async_runtime::spawn(async move {
            if let Err(err) = registry.load_from_db().await {
                log::warn!("apps registry: initial DB load failed: {err}");
            }
            if let Err(err) =
                super::apps_registry::add_apps_from_ids(registry.clone(), excluded_app_ids).await
            {
                log::warn!("apps registry: initial excluded app materialization failed: {err}");
            }
        });
    }

    super::cleanup::spawn(app.clone());

    // 事件信号通道：watcher 线程投递「剪贴板变了 + 事件当下的前台应用」，
    // ingest 线程消费并防抖。通道关闭（两侧线程结束）即进程退出场景。
    let (pending_tx, pending_rx) = std::sync::mpsc::channel::<Option<FrontmostApp>>();
    spawn_ingest_thread(
        app.clone(),
        guard,
        store,
        app_icon_store,
        registry,
        pause.clone(),
        pending_rx,
    );
    spawn_watch_thread(app.clone(), pending_tx, pause);
    Ok(())
}

fn spawn_watch_thread(app: AppHandle, pending: Sender<Option<FrontmostApp>>, pause: WatcherPause) {
    std::thread::Builder::new()
        .name("clipboard-watcher".to_owned())
        .spawn(move || {
            let mut watcher =
                match ClipboardWatcherContext::new_with_interval(CLIPBOARD_POLL_INTERVAL) {
                    Ok(watcher) => watcher,
                    Err(err) => {
                        log::error!("clipboard watcher: failed to create watcher: {err}");
                        return;
                    }
                };

            watcher.add_handler(ClipboardChangeHandler {
                app,
                pending,
                pause,
            });

            log::info!("clipboard watcher started");
            // 阻塞直至进程退出。
            watcher.start_watch();
        })
        .expect("failed to spawn clipboard watcher thread");
}

/// ingest 线程：防抖消费剪贴板变更信号，静默后读取 + 入库。
/// 平台剪贴板句柄在本线程内构造，不跨线程移动。
fn spawn_ingest_thread(
    app: AppHandle,
    guard: Arc<WritebackGuard>,
    store: ImageStore,
    app_icon_store: AppIconStore,
    registry: AppsRegistry,
    pause: WatcherPause,
    pending: Receiver<Option<FrontmostApp>>,
) {
    std::thread::Builder::new()
        .name("clipboard-ingest".to_owned())
        .spawn(move || {
            let reader = match ClipboardReader::new() {
                Ok(reader) => reader,
                Err(err) => {
                    log::error!("clipboard ingest: failed to create reader: {err}");
                    return;
                }
            };

            loop {
                let Ok(first) = pending.recv() else {
                    return;
                };
                let source = debounce_updates(
                    &pending,
                    first,
                    CLIPBOARD_DEBOUNCE,
                    CLIPBOARD_DEBOUNCE_MAX_WAIT,
                );

                // 防抖等待期间用户可能已关闭「监听」：丢弃整批变更。
                if pause.is_paused() {
                    continue;
                }

                process_clipboard_update(
                    &app,
                    &reader,
                    &guard,
                    &store,
                    &app_icon_store,
                    &registry,
                    source,
                );
            }
        })
        .expect("failed to spawn clipboard ingest thread");
}

/// 防抖：静默 [`CLIPBOARD_DEBOUNCE`] 内再无新变更才返回，期间到达的新变更重置窗口
/// （总时长受 [`CLIPBOARD_DEBOUNCE_MAX_WAIT`] 约束），前台应用取最后一刻的值。
/// 纯通道操作，不触碰剪贴板。
fn debounce_updates(
    pending: &Receiver<Option<FrontmostApp>>,
    first: Option<FrontmostApp>,
    quiet: Duration,
    max_wait: Duration,
) -> Option<FrontmostApp> {
    let mut latest = first;
    let started = Instant::now();

    loop {
        let remaining = max_wait.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return latest;
        }

        match pending.recv_timeout(quiet.min(remaining)) {
            Ok(source) => latest = source,
            Err(RecvTimeoutError::Timeout) => return latest,
            // 通道关闭：把手头的变更处理完，下一轮 recv 会让线程退出。
            Err(RecvTimeoutError::Disconnected) => return latest,
        }
    }
}

/// 防抖静默后处理一次剪贴板变更：读取 → 转换 → 回环抑制 → 入库 emit。
/// `source` 是事件当下（而非此刻）探测到的前台应用。
fn process_clipboard_update(
    app: &AppHandle,
    reader: &ClipboardReader,
    guard: &Arc<WritebackGuard>,
    store: &ImageStore,
    app_icon_store: &AppIconStore,
    registry: &AppsRegistry,
    source: Option<FrontmostApp>,
) {
    // 防抖后重新快照：过滤名单在事件回调已判定，这里取 capture / sensitive 的最新值。
    let settings = app
        .try_state::<SettingsStore>()
        .map(|s| s.snapshot())
        .unwrap_or_default();

    // 同步读取 + 转换（含图片落盘）：拿到 content_hash 才能判定是否为自身写回。
    // 读取带序列号一致性校验 + 重试梯子：读取期间剪贴板被并发改写（撕裂载荷）或
    // 被其他监听器短暂锁住，都按失败重读，拿到稳定后的最终状态。
    let payload = match read_with_retry(&CLIPBOARD_READ_RETRY_DELAYS, || {
        reader.read_with_capture_stable(&settings.clipboard.capture)
    }) {
        Ok(Some(payload)) => payload,
        Ok(None) => return,
        Err(err) => {
            log::warn!("clipboard watcher: read failed: {err}");
            return;
        }
    };

    let mut item = match build_item_with_settings(
        store,
        &payload,
        &settings.clipboard.capture,
        &settings.clipboard.sensitive,
        settings.clipboard.content.copy_plain,
    ) {
        Ok(Some(item)) => item,
        Ok(None) => return,
        Err(err) => {
            log::warn!("clipboard watcher: build item failed: {err}");
            return;
        }
    };

    // 自身写回触发的变更：跳过入库，避免回环。
    if guard.should_skip(&item.content_hash) {
        return;
    }

    let (source_app, cached) = match source.as_ref().map(|src| {
        let (app, cached) = materialize_source(registry, src.clone());
        (app, cached)
    }) {
        Some((app, cached)) => (Some(app), cached),
        None => (None, true),
    };
    if let Some(src) = &source_app {
        item.source_app_id = Some(src.id.clone());
    }

    // 入库与 emit 交给异步运行时；只移动 Send 数据，不碰平台句柄。
    // 缓存未命中的来源应用：把原始探测结果移进异步任务补抽 icon（监听线程零 OS 抽取）。
    let app = app.clone();
    let app_icon_store = app_icon_store.clone();
    let registry = registry.clone();
    let pending_icon_source = if cached { None } else { source };
    tauri::async_runtime::spawn(async move {
        if let Some(src) = pending_icon_source {
            spawn_materialize_icon(&app, &app_icon_store, &registry, src);
        }
        let pool = app.state::<crate::db::DatabaseState>().pool().await;
        if let Err(err) = persist_and_notify(&app, &pool, item, source_app.as_ref()).await {
            log::error!("clipboard watcher: persist failed: {err}");
        }
    });
}

struct ClipboardChangeHandler {
    app: AppHandle,
    pending: Sender<Option<FrontmostApp>>,
    pause: WatcherPause,
}

impl ClipboardHandler for ClipboardChangeHandler {
    fn on_clipboard_change(&mut self) {
        // 用户从托盘关掉「监听」时直接早退，不读取、不入库、不 emit。
        if self.pause.is_paused() {
            return;
        }

        // **先**抓前台应用：防抖 + 异步入库之后前台早就切回我们自己了。
        // 自身写回的事件会在 ingest 线程的 guard 处被丢弃，但 detect 仍会无害地返回
        // 我们自己的 bundle id。
        let source = source::detect_frontmost();

        // 用户在偏好里勾选了「过滤此应用」时，本次复制整条直接丢弃——不投递、不读取、不入库。
        // 回调只做轻量判定，设置快照留给 ingest 线程（防抖后用最新值读取）。
        let settings = self
            .app
            .try_state::<SettingsStore>()
            .map(|s| s.snapshot())
            .unwrap_or_default();
        if let Some(src) = &source {
            if settings
                .clipboard
                .filters
                .excluded_app_ids
                .iter()
                .any(|id| id == &src.id)
            {
                return;
            }
        }

        // 只投递信号，重活（防抖 + 读取 + 入库）由 ingest 线程做，让 watcher 的
        // 消息循环立刻回到 recv，防抖窗口内的后续变更得以排队。
        let _ = self.pending.send(source);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use clipboard_rs::{Clipboard, ClipboardContext};

    use super::*;
    use crate::clipboard::{build_item, ImageStore, WritebackGuard};
    use crate::db::items::find_item_by_id;
    use crate::db::test_support::memory_pool;

    const ZERO_DELAY_RETRIES: [Duration; 3] = [Duration::ZERO; 3];

    fn frontmost(id: &str) -> Option<FrontmostApp> {
        Some(FrontmostApp {
            id: id.to_owned(),
            name: id.to_owned(),
            platform: crate::db::models::Platform::Macos,
            icon_path: None,
        })
    }

    #[test]
    fn debounce_returns_last_source_after_quiet_window() {
        let (tx, rx) = std::sync::mpsc::channel::<Option<FrontmostApp>>();

        tx.send(frontmost("a")).unwrap();
        tx.send(frontmost("b")).unwrap();

        // 首个事件已消费（作为 first 传入），窗口内到达的 a/b 合并，取最后一刻的 b。
        let latest = debounce_updates(&rx, None, Duration::from_millis(50), Duration::from_secs(1));

        assert_eq!(latest.as_ref().map(|s| s.id.as_str()), Some("b"));
    }

    #[test]
    fn debounce_returns_sole_event_after_quiet_window() {
        let (tx, rx) = std::sync::mpsc::channel::<Option<FrontmostApp>>();
        tx.send(frontmost("solo")).unwrap();
        drop(tx);

        let latest = debounce_updates(
            &rx,
            frontmost("solo"),
            Duration::from_millis(50),
            Duration::from_secs(1),
        );

        assert_eq!(latest.as_ref().map(|s| s.id.as_str()), Some("solo"));
    }

    #[test]
    fn debounce_gives_up_after_max_wait_under_continuous_writes() {
        let (tx, rx) = std::sync::mpsc::channel::<Option<FrontmostApp>>();
        let writer = std::thread::spawn(move || {
            for i in 0..50 {
                let _ = tx.send(frontmost(&format!("event-{i}")));
                std::thread::sleep(Duration::from_millis(10));
            }
        });

        let started = Instant::now();
        let latest = debounce_updates(
            &rx,
            frontmost("first"),
            Duration::from_millis(80),
            Duration::from_millis(200),
        );
        let elapsed = started.elapsed();

        writer.join().unwrap();
        assert!(latest.is_some());
        // 持续写入会不断重置静默窗口，总上限必须兜底，不能无限饿死。
        assert!(
            elapsed < Duration::from_millis(800),
            "debounce should cap at max_wait, got {elapsed:?}"
        );
    }

    #[test]
    fn clipboard_read_retry_returns_immediate_success() {
        let attempts = Cell::new(0);

        let result = read_with_retry(&ZERO_DELAY_RETRIES, || {
            attempts.set(attempts.get() + 1);
            Ok::<_, &'static str>(Some("captured"))
        });

        assert_eq!(result, Ok(Some("captured")));
        assert_eq!(attempts.get(), 1);
    }

    #[test]
    fn clipboard_read_retry_recovers_after_transient_error() {
        let attempts = Cell::new(0);

        let result = read_with_retry(&ZERO_DELAY_RETRIES, || {
            attempts.set(attempts.get() + 1);
            if attempts.get() == 1 {
                Err("clipboard busy")
            } else {
                Ok(Some("captured"))
            }
        });

        assert_eq!(result, Ok(Some("captured")));
        assert_eq!(attempts.get(), 2);
    }

    #[test]
    fn clipboard_read_retry_does_not_retry_empty_content() {
        let attempts = Cell::new(0);

        let result = read_with_retry(&ZERO_DELAY_RETRIES, || {
            attempts.set(attempts.get() + 1);
            Ok::<Option<&'static str>, &'static str>(None)
        });

        assert_eq!(result, Ok(None));
        assert_eq!(attempts.get(), 1);
    }

    #[test]
    fn clipboard_read_retry_returns_final_error_after_exhaustion() {
        let attempts = Cell::new(0);

        let result = read_with_retry(&ZERO_DELAY_RETRIES, || {
            attempts.set(attempts.get() + 1);
            Err::<Option<&'static str>, _>(attempts.get())
        });

        assert_eq!(result, Err(4));
        assert_eq!(attempts.get(), 4);
    }

    fn temp_image_store() -> (TempDir, ImageStore) {
        let dir = TempDir::new();
        let store = ImageStore::for_test(dir.path().join("resources").join("clipboard-images"));
        (dir, store)
    }

    // 复刻 on_clipboard_change 的同步部分（读取 → 转换 → 去重判定）+ async 入库，
    // 但绕开 Tauri AppHandle / emit（无法在单测里构造），验证整条数据链路。
    // 触碰真实系统剪贴板，默认 ignore；本机用 `cargo test -- --ignored` 验证。
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "touches the real system clipboard; run with --ignored on a desktop session"]
    async fn end_to_end_text_ingests_once_then_dedups() {
        let pool = memory_pool().await;
        let guard = WritebackGuard::new();
        let (_dir, store) = temp_image_store();

        // 串行锁只覆盖触碰真实剪贴板的同步段，await DB 前即释放（不跨 await 持锁）。
        let item = {
            let _serial = crate::clipboard::test_lock::serial();
            let ctx = ClipboardContext::new().unwrap();
            ctx.set_text("e2e ecopaste watcher".to_owned()).unwrap();

            let reader = ClipboardReader::new().unwrap();
            let payload = reader
                .read_with_capture(&crate::settings::Capture::default())
                .unwrap()
                .expect("should read text");
            build_item(&store, &payload)
                .unwrap()
                .expect("should map to item")
        };
        assert!(!guard.should_skip(&item.content_hash));

        // 首次入库：新行。
        let first = upsert_item(&pool, &item).await.unwrap();
        assert!(!first.deduplicated);
        assert_eq!(
            find_item_by_id(&pool, &first.id)
                .await
                .unwrap()
                .unwrap()
                .content,
            "e2e ecopaste watcher"
        );

        // 同内容再来一次：命中去重，use_count 累加，不新增行。
        let second = upsert_item(&pool, &item).await.unwrap();
        assert!(second.deduplicated);
        assert_eq!(first.id, second.id);
        assert_eq!(
            find_item_by_id(&pool, &first.id)
                .await
                .unwrap()
                .unwrap()
                .use_count,
            2
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "touches the real system clipboard; run with --ignored on a desktop session"]
    async fn writeback_guard_suppresses_self_copy() {
        let (_dir, store) = temp_image_store();
        let _serial = crate::clipboard::test_lock::serial();
        let ctx = ClipboardContext::new().unwrap();
        ctx.set_text("self writeback content".to_owned()).unwrap();

        let reader = ClipboardReader::new().unwrap();
        let payload = reader
            .read_with_capture(&crate::settings::Capture::default())
            .unwrap()
            .unwrap();
        let item = build_item(&store, &payload).unwrap().unwrap();

        // 模拟写回前登记 → 监听读到同内容 → 被抑制。
        let guard = WritebackGuard::new();
        guard.suppress(item.content_hash.clone());
        assert!(guard.should_skip(&item.content_hash));
    }

    // 验证真实剪贴板图片链路：set_image（OS 原生 TIFF）→ read_with_capture 解码为 PNG →
    // build_item 落盘原图/缩略图 → upsert 入库。覆盖合成 PNG 测不到的 OS 解码段。
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "touches the real system clipboard; run with --ignored on a desktop session"]
    async fn end_to_end_image_stores_and_ingests() {
        use clipboard_rs::common::RustImage;

        let pool = memory_pool().await;
        let (_dir, store) = temp_image_store();

        let item = {
            let _serial = crate::clipboard::test_lock::serial();
            let png = {
                use std::io::Cursor;
                let buf = image::RgbaImage::from_pixel(40, 24, image::Rgba([7, 8, 9, 255]));
                let mut out = Cursor::new(Vec::new());
                image::DynamicImage::ImageRgba8(buf)
                    .write_to(&mut out, image::ImageFormat::Png)
                    .unwrap();
                out.into_inner()
            };
            let ctx = ClipboardContext::new().unwrap();
            ctx.set_image(clipboard_rs::RustImageData::from_bytes(&png).unwrap())
                .unwrap();

            let reader = ClipboardReader::new().unwrap();
            let payload = reader
                .read_with_capture(&crate::settings::Capture::default())
                .unwrap()
                .expect("should read image");
            build_item(&store, &payload)
                .unwrap()
                .expect("image should map to item")
        };

        assert_eq!(item.kind, crate::db::models::ClipboardKind::Image);
        assert!(item.content.ends_with(".png"));
        assert!(item.width.unwrap() > 0 && item.height.unwrap() > 0);
        // 复制热路径只落原图；缩略图懒生成，此刻尚未存在。
        assert!(store.origin_path(&item.content).exists());
        assert!(!store.thumbnail_path(&item.content).exists());
        // 模拟前端首次取图：按需生成缩略图。
        assert!(store.ensure_thumbnail(&item.content).unwrap().exists());

        let result = upsert_item(&pool, &item).await.unwrap();
        assert!(!result.deduplicated);
        assert_eq!(
            find_item_by_id(&pool, &result.id)
                .await
                .unwrap()
                .unwrap()
                .kind,
            crate::db::models::ClipboardKind::Image
        );
    }

    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("ecopaste-watcher-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }
}

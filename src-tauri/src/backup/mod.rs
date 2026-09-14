//! EcoPaste 历史备份包导出与接收壳识别。
//!
//! `.ecopastebak` 有两种格式：明文模式是标准 ZIP；加密模式是 EcoPaste 自有容器。

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Cursor, Read, Seek, Write};
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

use anyhow::{anyhow, Context};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{QueryBuilder, Sqlite, SqlitePool};
use tauri::{AppHandle, Emitter, Manager};
use tempfile::{NamedTempFile, TempDir};
use walkdir::WalkDir;
use zeroize::Zeroizing;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::core::{AppError, Result};
use crate::db::models::ClipboardGroup;

pub const BACKUP_EXTENSION: &str = "ecopastebak";
pub const BACKUP_RECEIVED_EVENT: &str = "backup://received";

const MAGIC: &[u8; 12] = b"ECOPASTEBAK1";
const ZIP_MAGIC: &[u8; 2] = b"PK";
const HEADER_LEN_BYTES: usize = 4;
const FORMAT_VERSION: u16 = 1;
const ARGON2_MEMORY_KIB: u32 = 64 * 1024;
const ARGON2_TIME_COST: u32 = 3;
const ARGON2_PARALLELISM: u32 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;
const SETTINGS_FILENAME: &str = "settings.json";
const DB_FILENAME: &str = "clipboard.db";
const DB_ARCHIVE_DIR: &str = "db";
const RESOURCES_ARCHIVE_DIR: &str = "resources";
const CONFIG_ARCHIVE_DIR: &str = "config";
const MANIFEST_FILENAME: &str = "manifest.json";

/// 偏好窗口被销毁时暂存的待处理备份接收事件。
///
/// `emit_received_backup` 在 preference 不存在（已空闲销毁）时无法 push 事件——重建是异步的，
/// 前端 listener 尚未挂载。改为存入此 slot，由前端重建后通过 `take_pending_backup` 主动拉取，
/// 两条路径（存活 push / 销毁 pull）互斥，避免事件丢失。
static PENDING_BACKUP: LazyLock<Mutex<Option<BackupReceivedPayload>>> =
    LazyLock::new(|| Mutex::new(None));

/// 应用是否已进入可安全建窗的就绪态（由 `RunEvent::Ready` 置位）。
///
/// macOS 冷启动用文件关联打开 app 时，系统会在事件循环刚起步、`setup` 尚未跑完时，
/// 经 ObjC `application:openURLs:` **同步**投递 `RunEvent::Opened`。此刻去建偏好窗口会
/// panic，而该回调跨 `extern "C"` 边界不可 unwind，直接 abort（应用闪退）。
/// 故未就绪时只把文件路径压入 [`PENDING_OPEN_FILES`]，待 `Ready` 后由
/// [`mark_app_ready`] 统一补投。
#[cfg(target_os = "macos")]
static APP_READY: AtomicBool = AtomicBool::new(false);

/// 就绪前到达的待处理打开文件路径队列（仅冷启动文件关联场景会用到）。
#[cfg(target_os = "macos")]
static PENDING_OPEN_FILES: LazyLock<Mutex<Vec<PathBuf>>> = LazyLock::new(|| Mutex::new(Vec::new()));

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportHistoryBackupOptions {
    pub mode: BackupExportMode,
    pub password: Option<String>,
}

/// 批量导出所选条目为 `.ecopastebak`：多选工具条「导出」入口。
/// 格式与全量备份完全一致（manifest + 过滤库 + 资源子集 + 当前设置快照），
/// 导入端无需感知「这是部分备份」。不存在的 id 静默跳过；全空时报错。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportItemsBackupInput {
    pub ids: Vec<String>,
    pub target_path: String,
    pub options: ExportHistoryBackupOptions,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportHistoryBackupInput {
    pub path: String,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportHistoryBackupOptions {
    pub strategy: BackupImportStrategy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackupImportStrategy {
    Merge,
    Overwrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackupExportMode {
    Encrypted,
    Plain,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportHistoryBackupResult {
    pub path: String,
    pub total_bytes: u64,
    pub item_count: i64,
    pub text_count: i64,
    pub image_count: i64,
    pub files_count: i64,
    pub resource_bytes: u64,
    pub exported_at: DateTime<Utc>,
    pub mode: BackupExportMode,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportItemsBackupResult {
    pub path: String,
    pub total_bytes: u64,
    /// 实际写入备份的条目数（源库中不存在的 id 已被剔除）。
    pub item_count: i64,
    pub text_count: i64,
    pub image_count: i64,
    pub files_count: i64,
    pub resource_bytes: u64,
    pub exported_at: DateTime<Utc>,
    pub mode: BackupExportMode,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportHistoryBackupResult {
    pub strategy: BackupImportStrategy,
    pub imported_items: u64,
    pub skipped_items: u64,
    pub imported_resources: u64,
    pub imported_settings: bool,
    pub requires_restart: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupReceivedPayload {
    pub path: String,
    pub source: BackupReceiveSource,
    pub mode: BackupContainerMode,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BackupReceiveSource {
    OpenFile,
    DragDrop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackupContainerMode {
    Encrypted,
    Plain,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContainerHeader {
    format_version: u16,
    mode: BackupContainerMode,
    kdf: Option<KdfHeader>,
    cipher: Option<CipherHeader>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KdfHeader {
    algorithm: String,
    memory_kib: u32,
    time_cost: u32,
    parallelism: u32,
    salt: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CipherHeader {
    algorithm: String,
    nonce: Vec<u8>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupManifest {
    format_version: u16,
    app_name: String,
    app_version: String,
    exported_at: DateTime<Utc>,
    platform: String,
    encryption: ManifestEncryption,
    item_count: i64,
    text_count: i64,
    image_count: i64,
    files_count: i64,
    resource_bytes: u64,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
enum ManifestEncryption {
    None,
    Password,
}

#[derive(Debug, Clone, Copy)]
struct BackupCounts {
    item_count: i64,
    text_count: i64,
    image_count: i64,
    files_count: i64,
}

#[derive(Debug, Clone)]
struct BackupSourcePaths {
    db_path: PathBuf,
    resources_dir: PathBuf,
    settings_path: PathBuf,
}

/// 导出当前环境历史数据库、资源文件和设置为 `.ecopastebak` 备份包。
pub async fn export_history_backup(
    app: &AppHandle,
    pool: &SqlitePool,
    target_path: String,
    options: ExportHistoryBackupOptions,
) -> Result<ExportHistoryBackupResult> {
    let target = normalize_backup_path(PathBuf::from(target_path))?;
    let password = validate_password_options(&options)?;
    let exported_at = Utc::now();
    let counts = load_counts(pool).await?;
    let source_paths = backup_source_paths(app)?;
    let resource_bytes = dir_size(&source_paths.resources_dir)?;
    let manifest = build_manifest(app, exported_at, options.mode, counts, resource_bytes)?;

    checkpoint_database(pool).await?;

    let payload_file = NamedTempFile::new().context("failed to create temporary backup payload")?;
    write_payload_zip(&source_paths, payload_file.path(), &manifest, &target)?;
    let total_bytes = write_container(&target, payload_file.path(), options.mode, password)?;

    Ok(ExportHistoryBackupResult {
        path: target.to_string_lossy().into_owned(),
        total_bytes,
        item_count: counts.item_count,
        text_count: counts.text_count,
        image_count: counts.image_count,
        files_count: counts.files_count,
        resource_bytes,
        exported_at,
        mode: options.mode,
    })
}

/// 把所选条目导出为 `.ecopastebak` 备份包（「所选条目」变体，见
/// `ExportItemsBackupInput` 文档）。与全量导出共用 payload / container 组装，
/// 区别只在数据源：临时目录里物化「过滤库 + 资源子集」，打包后整体丢弃。
pub async fn export_items_backup(
    app: &AppHandle,
    pool: &SqlitePool,
    input: ExportItemsBackupInput,
) -> Result<ExportItemsBackupResult> {
    if input.ids.is_empty() {
        return app_error("请选择要导出的记录");
    }

    let target = normalize_backup_path(PathBuf::from(input.target_path))?;
    let password = validate_password_options(&input.options)?;
    let exported_at = Utc::now();

    let work = tempfile::tempdir().context("failed to create items export work dir")?;
    let filtered_db_path = work.path().join(DB_FILENAME);
    let filtered_resources = work.path().join("resources");

    let items = build_filtered_db(pool, &input.ids, &filtered_db_path).await?;
    materialize_item_resources(pool, app, &items, &filtered_resources).await?;

    let counts = count_filtered_items(&items);
    let source_paths = BackupSourcePaths {
        db_path: filtered_db_path,
        resources_dir: filtered_resources.clone(),
        settings_path: backup_source_paths(app)?.settings_path,
    };
    let resource_bytes = dir_size(&filtered_resources)?;
    let package = app.package_info();
    let manifest = build_manifest_with_identity(
        &package.name,
        &package.version.to_string(),
        exported_at,
        input.options.mode,
        counts,
        resource_bytes,
    )?;

    let payload_file = NamedTempFile::new().context("failed to create temporary backup payload")?;
    write_payload_zip(&source_paths, payload_file.path(), &manifest, &target)?;
    let total_bytes = write_container(&target, payload_file.path(), input.options.mode, password)?;

    Ok(ExportItemsBackupResult {
        path: target.to_string_lossy().into_owned(),
        total_bytes,
        item_count: counts.item_count,
        text_count: counts.text_count,
        image_count: counts.image_count,
        files_count: counts.files_count,
        resource_bytes,
        exported_at,
        mode: input.options.mode,
    })
}

/// 从 `.ecopastebak` 导入历史和设置；合并写入当前库，覆盖热替换当前数据。
pub async fn import_history_backup(
    app: &AppHandle,
    db: &crate::db::DatabaseState,
    input: ImportHistoryBackupInput,
    options: ImportHistoryBackupOptions,
) -> Result<ImportHistoryBackupResult> {
    validate_import_options(&input, &options)?;

    let path = PathBuf::from(input.path);
    ensure_backup_extension(&path)?;
    let payload = read_backup_payload(&path, input.password.as_deref())?;
    let temp = extract_payload_zip(&payload)?;
    validate_extracted_payload(temp.path())?;

    match options.strategy {
        BackupImportStrategy::Merge => {
            let pool = db.pool().await;
            merge_import(app, &pool, temp.path(), &options).await
        }
        BackupImportStrategy::Overwrite => overwrite_import(app, db, temp.path(), &options).await,
    }
}

/// 识别 `.ecopastebak` 文件头并返回容器模式；不解密、不导入。
pub fn inspect_backup_file(path: &Path) -> Result<BackupContainerMode> {
    ensure_backup_extension(path)?;

    let mut file = File::open(path).with_context(|| format!("failed to open backup {path:?}"))?;
    inspect_backup_reader(&mut file)
}

/// 将系统打开文件或拖入文件统一转成偏好页接收事件。
///
/// preference 已改为空闲可销毁窗口：若窗口仍存活，照常 show + push 事件；
/// 若已销毁，先把 payload 存入 [`PENDING_BACKUP`]，再 show 触发重建——
/// 前端重建后经 `take_pending_backup` 主动拉取，规避「重建异步、push 丢失」竞态。
pub fn emit_received_backup(
    app: &AppHandle,
    path: PathBuf,
    source: BackupReceiveSource,
) -> Result<()> {
    let mode = inspect_backup_file(&path)?;

    let payload = BackupReceivedPayload {
        path: path.to_string_lossy().into_owned(),
        source,
        mode,
    };

    let exists = app
        .get_webview_window(crate::window::PREFERENCE_WINDOW_LABEL)
        .is_some();

    if !exists {
        set_pending_backup(payload.clone());
    }

    crate::window::show_window(app, crate::window::PREFERENCE_WINDOW_LABEL)?;

    if exists {
        app.emit(BACKUP_RECEIVED_EVENT, payload)
            .context("failed to emit backup received event")?;
    }

    Ok(())
}

/// 存入待处理备份接收事件，覆盖旧值（仅保留最近一次）。
fn set_pending_backup(payload: BackupReceivedPayload) {
    let mut guard = PENDING_BACKUP.lock().unwrap_or_else(|poisoned| {
        log::error!("pending backup mutex poisoned on set, recovering");
        poisoned.into_inner()
    });
    *guard = Some(payload);
}

/// 取走并清空待处理备份接收事件，供偏好窗口重建后首屏拉取。
pub fn take_pending_backup() -> Option<BackupReceivedPayload> {
    let mut guard = PENDING_BACKUP.lock().unwrap_or_else(|poisoned| {
        log::error!("pending backup mutex poisoned on take, recovering");
        poisoned.into_inner()
    });
    guard.take()
}

/// 处理一次「系统打开文件」请求，对应用就绪状态鲁棒。
///
/// macOS 冷启动双击文件关联时，系统会在 `setup` 完成前从 ObjC `application:openURLs:`
/// 同步投递 `RunEvent::Opened`。此时建窗会读取尚未 `manage` 的 state 而 panic，且 panic
/// 无法穿过 tao 的 `extern "C"` 回调边界（`panic_cannot_unwind`）→ abort。
///
/// 故未就绪时只把路径压入 [`PENDING_OPEN_FILES`]，待 [`mark_app_ready`] 在 `RunEvent::Ready`
/// 时排空处理；已就绪则立即处理。调用方仍应额外用 `catch_unwind` 兜底，杜绝任何残余 panic 越界。
#[cfg(target_os = "macos")]
pub fn handle_open_file(app: &AppHandle, path: PathBuf, source: BackupReceiveSource) {
    if !is_backup_path(&path) {
        return;
    }

    if !APP_READY.load(Ordering::Acquire) {
        let mut guard = PENDING_OPEN_FILES.lock().unwrap_or_else(|poisoned| {
            log::error!("pending open files mutex poisoned on push, recovering");
            poisoned.into_inner()
        });
        guard.push(path);
        return;
    }

    if let Err(err) = emit_received_backup(app, path, source) {
        log::error!("handle open file failed: {err:?}");
    }
}

/// 标记应用已就绪并排空启动期暂存的待打开文件。
///
/// 在 `RunEvent::Ready` 调用——此时事件循环已运行、所有 state 已 `manage`，建窗安全。
/// 排空前先置位 [`APP_READY`]，使其后到达的 `Opened` 直接走立即处理路径。
#[cfg(target_os = "macos")]
pub fn mark_app_ready(app: &AppHandle) {
    APP_READY.store(true, Ordering::Release);

    let pending: Vec<PathBuf> = {
        let mut guard = PENDING_OPEN_FILES.lock().unwrap_or_else(|poisoned| {
            log::error!("pending open files mutex poisoned on drain, recovering");
            poisoned.into_inner()
        });
        std::mem::take(&mut *guard)
    };

    for path in pending {
        if let Err(err) = emit_received_backup(app, path, BackupReceiveSource::OpenFile) {
            log::error!("drain pending open file failed: {err:?}");
        }
    }
}

/// 从进程参数中查找 `.ecopastebak` 路径，供 Windows 文件关联和第二实例回调使用。
pub fn backup_path_from_args(args: &[String]) -> Option<PathBuf> {
    args.iter()
        .map(PathBuf::from)
        .find(|path| is_backup_path(path))
}

/// 判断路径是否看起来是 EcoPaste 备份包。
pub fn is_backup_path(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case(BACKUP_EXTENSION))
}

fn validate_password_options(
    options: &ExportHistoryBackupOptions,
) -> Result<Option<Zeroizing<String>>> {
    match options.mode {
        BackupExportMode::Encrypted => {
            let Some(password) = options.password.as_ref() else {
                return app_error("请输入备份密码");
            };
            if password.chars().count() < 8 {
                return app_error("备份密码至少需要 8 个字符");
            }

            Ok(Some(Zeroizing::new(password.clone())))
        }
        BackupExportMode::Plain => {
            if options
                .password
                .as_deref()
                .is_some_and(|value| !value.is_empty())
            {
                return app_error("明文备份不应包含密码");
            }

            Ok(None)
        }
    }
}

fn validate_import_options(
    input: &ImportHistoryBackupInput,
    _options: &ImportHistoryBackupOptions,
) -> Result<()> {
    let path = PathBuf::from(input.path.as_str());
    match inspect_backup_file(&path)? {
        BackupContainerMode::Encrypted => {
            if input.password.as_deref().is_none_or(str::is_empty) {
                return app_error("请输入备份密码");
            }
        }
        BackupContainerMode::Plain => {
            if input
                .password
                .as_deref()
                .is_some_and(|value| !value.is_empty())
            {
                return app_error("明文备份不应包含密码");
            }
        }
    }

    Ok(())
}

fn normalize_backup_path(path: PathBuf) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return app_error("请选择备份保存位置");
    }

    match path.extension().and_then(|value| value.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case(BACKUP_EXTENSION) => Ok(path),
        Some(_) => app_error(format!("备份文件后缀必须是 .{BACKUP_EXTENSION}")),
        None => Ok(path.with_extension(BACKUP_EXTENSION)),
    }
}

fn ensure_backup_extension(path: &Path) -> Result<()> {
    if is_backup_path(path) {
        return Ok(());
    }

    app_error(format!("请选择 .{BACKUP_EXTENSION} 备份文件"))
}

/// 在临时目录构建「条目过滤库」：新库跑完整 migrations 后，把所选条目连同
/// 其引用的分组 / 来源应用维度行原样插入，FTS 由 INSERT 触发器自动重建。
/// file_type_icons 是纯缓存表（cache_key 依赖目标机文件路径），不随行迁移，
/// 导入端 [`crate::commands::clipboard::FileIconCache`] miss 后会重新抽取。
/// 返回实际命中的条目（源库中不存在的 id 已被剔除），供资源物化与计数复用。
async fn build_filtered_db(
    pool: &SqlitePool,
    ids: &[String],
    db_path: &Path,
) -> Result<Vec<crate::db::models::ClipboardItem>> {
    let items = crate::db::items::list_items_by_ids(pool, ids).await?;
    if items.is_empty() {
        return app_error("所选记录不存在");
    }

    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true);
    let filtered = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .with_context(|| format!("failed to open filtered backup database at {db_path:?}"))?;

    let result = async {
        sqlx::migrate!("./migrations")
            .run(&filtered)
            .await
            .context("failed to run migrations on filtered backup database")?;

        copy_referenced_dimensions(pool, &filtered, &items).await?;
        for item in &items {
            crate::db::items::insert_item(&filtered, item).await?;
        }

        crate::core::Result::Ok(())
    }
    .await;

    filtered.close().await;
    result?;

    Ok(items)
}

/// 把条目引用到的分组 / 来源应用行从源库复制到过滤库。
/// 应用表全量复制（表本身只有被引用过的应用，量级 = 来源应用数）；
/// 分组表只复制被引用的 id，避免把无关分组也带进「所选条目」备份。
async fn copy_referenced_dimensions(
    pool: &SqlitePool,
    filtered: &SqlitePool,
    items: &[crate::db::models::ClipboardItem],
) -> Result<()> {
    let mut group_ids: Vec<String> = items
        .iter()
        .filter_map(|item| item.group_id.clone())
        .collect();
    group_ids.sort_unstable();
    group_ids.dedup();

    for group_id in group_ids {
        let row = sqlx::query_as::<_, ClipboardGroup>(
            "SELECT id, name, icon, is_hidden, sort_order, created_at, updated_at \
             FROM clipboard_groups WHERE id = ?",
        )
        .bind(&group_id)
        .fetch_optional(pool)
        .await
        .context("failed to read referenced group for items export")?;

        if let Some(group) = row {
            crate::db::groups::insert_group(filtered, &group).await?;
        }
    }

    let apps = crate::db::apps::list_all_apps(pool).await?;
    for app in apps {
        crate::db::apps::upsert_app(filtered, &app).await?;
    }

    Ok(())
}

/// 物化所选条目引用的磁盘资源到临时 resources 目录，布局与真实目录一致：
/// - image 条目：`clipboard-images/origin/<分片>/<hash>.png` 原图（thumbnails /
///   previews 是懒生成档，目标机取图时按需重建，不打包）；
/// - 被引用来源应用的 icon：`app-icons/<hash>.png`（`icon_file = None` 的应用无文件）。
///
/// file-icons 目录是目标机路径相关的缓存，整体跳过（导入端 miss 后重新抽取）。
async fn materialize_item_resources(
    pool: &SqlitePool,
    app: &AppHandle,
    items: &[crate::db::models::ClipboardItem],
    target_root: &Path,
) -> Result<()> {
    let resources = crate::core::paths::resources_dir(app)?;
    let image_store = app.state::<crate::clipboard::ImageStore>();

    for item in items {
        if item.kind != crate::db::models::ClipboardKind::Image {
            continue;
        }

        let origin = image_store.origin_path(&item.content);
        if !origin.exists() {
            log::warn!(
                "export items backup: image origin missing for item {}, skip",
                item.id
            );
            continue;
        }

        let relative = origin
            .strip_prefix(&resources)
            .with_context(|| format!("failed to locate {origin:?} under resources dir"))?;
        copy_resource_file(&origin, &target_root.join(relative))?;
    }

    let mut app_ids: Vec<String> = items
        .iter()
        .filter_map(|item| item.source_app_id.clone())
        .collect();
    app_ids.sort_unstable();
    app_ids.dedup();

    let app_icon_store = app.state::<crate::clipboard::AppIconStore>();
    for app_row in crate::db::apps::list_apps_by_ids(pool, &app_ids).await? {
        let Some(icon_file) = app_row.icon_file.as_deref() else {
            continue;
        };
        let source = app_icon_store.icon_path(icon_file);
        if !source.exists() {
            continue;
        }

        let relative = source
            .strip_prefix(&resources)
            .with_context(|| format!("failed to locate {source:?} under resources dir"))?;
        copy_resource_file(&source, &target_root.join(relative))?;
    }

    Ok(())
}

/// 把一个资源文件复制进 items 导出的临时 resources 目录（自动建父目录）。
fn copy_resource_file(source: &Path, target: &Path) -> Result<()> {
    if target.exists() {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {parent:?} for items export"))?;
    }
    fs::copy(source, target)
        .with_context(|| format!("failed to copy resource {source:?} into items export"))?;
    Ok(())
}

/// 统计过滤条目的 kind 分布，manifest 与导出结果共用。
fn count_filtered_items(items: &[crate::db::models::ClipboardItem]) -> BackupCounts {
    let mut counts = BackupCounts {
        item_count: items.len() as i64,
        text_count: 0,
        image_count: 0,
        files_count: 0,
    };
    for item in items {
        match item.kind {
            crate::db::models::ClipboardKind::Image => counts.image_count += 1,
            crate::db::models::ClipboardKind::Files => counts.files_count += 1,
            _ => counts.text_count += 1,
        }
    }
    counts
}

async fn checkpoint_database(pool: &SqlitePool) -> Result<()> {
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(pool)
        .await
        .context("failed to checkpoint backup database")?;

    Ok(())
}

async fn load_counts(pool: &SqlitePool) -> Result<BackupCounts> {
    let item_count = count_items(pool, None).await?;
    let text_count = count_items(pool, Some("text")).await?;
    let image_count = count_items(pool, Some("image")).await?;
    let files_count = count_items(pool, Some("files")).await?;

    Ok(BackupCounts {
        item_count,
        text_count,
        image_count,
        files_count,
    })
}

async fn count_items(pool: &SqlitePool, kind: Option<&str>) -> Result<i64> {
    let count = if let Some(kind) = kind {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM clipboard_items WHERE kind = ?")
            .bind(kind)
            .fetch_one(pool)
            .await
    } else {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM clipboard_items")
            .fetch_one(pool)
            .await
    }
    .context("failed to count backup items")?;

    Ok(count)
}

fn build_manifest(
    app: &AppHandle,
    exported_at: DateTime<Utc>,
    mode: BackupExportMode,
    counts: BackupCounts,
    resource_bytes: u64,
) -> Result<BackupManifest> {
    let package = app.package_info();
    build_manifest_with_identity(
        &package.name,
        &package.version.to_string(),
        exported_at,
        mode,
        counts,
        resource_bytes,
    )
}

/// [`build_manifest`] 的可测版本：manifest 组装本身不依赖运行时环境，
/// 抽出字符串参数便于单测直接断言。
fn build_manifest_with_identity(
    app_name: &str,
    app_version: &str,
    exported_at: DateTime<Utc>,
    mode: BackupExportMode,
    counts: BackupCounts,
    resource_bytes: u64,
) -> Result<BackupManifest> {
    Ok(BackupManifest {
        format_version: FORMAT_VERSION,
        app_name: app_name.to_owned(),
        app_version: app_version.to_owned(),
        exported_at,
        platform: current_platform().to_owned(),
        encryption: match mode {
            BackupExportMode::Encrypted => ManifestEncryption::Password,
            BackupExportMode::Plain => ManifestEncryption::None,
        },
        item_count: counts.item_count,
        text_count: counts.text_count,
        image_count: counts.image_count,
        files_count: counts.files_count,
        resource_bytes,
    })
}

fn current_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else {
        "windows"
    }
}

/// 汇总备份允许写入包内的源路径，避免把日志、缓存、临时文件等环境杂项带进迁移包。
fn backup_source_paths(app: &AppHandle) -> Result<BackupSourcePaths> {
    Ok(BackupSourcePaths {
        db_path: crate::db::db_path(app)?,
        resources_dir: crate::core::paths::resources_dir(app)?,
        settings_path: crate::core::paths::config_dir(app)?.join(SETTINGS_FILENAME),
    })
}

fn write_payload_zip(
    source_paths: &BackupSourcePaths,
    path: &Path,
    manifest: &BackupManifest,
    target_path: &Path,
) -> Result<()> {
    let file = File::create(path).with_context(|| format!("failed to create payload {path:?}"))?;
    let mut zip = ZipWriter::new(BufWriter::new(file));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    zip.start_file("manifest.json", options)
        .context("failed to write backup manifest entry")?;
    let manifest_bytes =
        serde_json::to_vec_pretty(manifest).context("failed to serialize backup manifest")?;
    zip.write_all(&manifest_bytes)
        .context("failed to write backup manifest")?;

    add_optional_file(
        &mut zip,
        &source_paths.db_path,
        &archive_path(DB_ARCHIVE_DIR, Path::new("clipboard.db"))?,
        options,
        target_path,
    )?;
    add_dir_contents(
        &mut zip,
        &source_paths.resources_dir,
        RESOURCES_ARCHIVE_DIR,
        options,
        target_path,
    )?;
    add_optional_file(
        &mut zip,
        &source_paths.settings_path,
        &archive_path(CONFIG_ARCHIVE_DIR, Path::new(SETTINGS_FILENAME))?,
        options,
        target_path,
    )?;

    zip.finish()
        .context("failed to finish backup payload archive")?;

    Ok(())
}

fn add_file<W: Write + Seek>(
    zip: &mut ZipWriter<W>,
    path: &Path,
    archive_name: &str,
    options: SimpleFileOptions,
) -> Result<()> {
    zip.start_file(archive_name, options)
        .with_context(|| format!("failed to start archive file {archive_name}"))?;
    let mut file = File::open(path).with_context(|| format!("failed to open {path:?}"))?;
    std::io::copy(&mut file, zip).with_context(|| format!("failed to archive {path:?}"))?;

    Ok(())
}

fn add_optional_file<W: Write + Seek>(
    zip: &mut ZipWriter<W>,
    path: &Path,
    archive_name: &str,
    options: SimpleFileOptions,
    target_path: &Path,
) -> Result<()> {
    if !path.exists() || same_path(path, target_path) {
        return Ok(());
    }

    add_file(zip, path, archive_name, options)
}

fn add_dir_contents<W: Write + Seek>(
    zip: &mut ZipWriter<W>,
    root: &Path,
    archive_root: &str,
    options: SimpleFileOptions,
    target_path: &Path,
) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }

    for entry in WalkDir::new(root)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        let path = entry.path();
        let metadata = entry
            .metadata()
            .with_context(|| format!("failed to read metadata at {path:?}"))?;
        if !metadata.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if should_skip_backup_path(path, file_name, target_path) {
            continue;
        }

        let relative = path
            .strip_prefix(root)
            .with_context(|| format!("failed to strip root {root:?} from {path:?}"))?;
        let archive_name = archive_path(archive_root, relative)?;
        add_file(zip, path, &archive_name, options)?;
    }

    Ok(())
}

fn archive_path(prefix: &str, relative: &Path) -> Result<String> {
    let mut parts = Vec::new();
    if !prefix.is_empty() {
        parts.push(prefix.to_owned());
    }
    for part in relative.components() {
        let std::path::Component::Normal(value) = part else {
            return app_error("backup path contains unsupported component");
        };
        let value = value
            .to_str()
            .ok_or_else(|| anyhow!("backup path is not valid utf-8"))?;
        parts.push(value.to_owned());
    }

    Ok(parts.join("/"))
}

fn should_skip_backup_path(path: &Path, file_name: &str, target_path: &Path) -> bool {
    if same_path(path, target_path) {
        return true;
    }

    file_name.ends_with(".tmp")
        || file_name.ends_with(".temp")
        || file_name.starts_with(".tmp")
        || file_name == ".DS_Store"
}

fn same_path(left: &Path, right: &Path) -> bool {
    let left = fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());

    left == right
}

fn write_container(
    target: &Path,
    payload_path: &Path,
    mode: BackupExportMode,
    password: Option<Zeroizing<String>>,
) -> Result<u64> {
    let parent = target
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).with_context(|| format!("failed to create {parent:?}"))?;

    let mut temp = NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file under {parent:?}"))?;
    match mode {
        BackupExportMode::Plain => {
            copy_file_into_writer(payload_path, &mut temp)?;
        }
        BackupExportMode::Encrypted => {
            let password = password.ok_or_else(|| anyhow!("missing backup password"))?;
            let mut payload = Vec::new();
            File::open(payload_path)
                .with_context(|| format!("failed to open payload {payload_path:?}"))?
                .read_to_end(&mut payload)
                .context("failed to read backup payload")?;

            let encrypted = encrypt_payload(&payload, &password)?;
            write_header(&mut temp, &encrypted.header)?;
            temp.write_all(&encrypted.ciphertext)
                .context("failed to write encrypted backup payload")?;
        }
    }

    temp.flush().context("failed to flush backup file")?;
    temp.as_file()
        .sync_all()
        .context("failed to sync backup file")?;
    temp.persist(target)
        .map_err(|err| anyhow!(err))
        .with_context(|| format!("failed to persist backup to {target:?}"))?;

    let total_bytes = fs::metadata(target)
        .with_context(|| format!("failed to read backup metadata {target:?}"))?
        .len();

    Ok(total_bytes)
}

struct EncryptedPayload {
    header: ContainerHeader,
    ciphertext: Vec<u8>,
}

fn encrypt_payload(payload: &[u8], password: &str) -> Result<EncryptedPayload> {
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    rand::rngs::OsRng.fill_bytes(&mut nonce);

    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    let params = Params::new(
        ARGON2_MEMORY_KIB,
        ARGON2_TIME_COST,
        ARGON2_PARALLELISM,
        Some(KEY_LEN),
    )
    .context("failed to build argon2 params")?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    argon2
        .hash_password_into(password.as_bytes(), &salt, key.as_mut())
        .context("failed to derive backup key")?;

    let cipher = XChaCha20Poly1305::new_from_slice(key.as_ref())
        .context("failed to initialize backup cipher")?;
    let nonce = XNonce::from(nonce);
    let ciphertext = cipher
        .encrypt(&nonce, payload)
        .map_err(|_| anyhow!("failed to encrypt backup payload"))?;

    Ok(EncryptedPayload {
        header: ContainerHeader {
            format_version: FORMAT_VERSION,
            mode: BackupContainerMode::Encrypted,
            kdf: Some(KdfHeader {
                algorithm: "argon2id".to_owned(),
                memory_kib: ARGON2_MEMORY_KIB,
                time_cost: ARGON2_TIME_COST,
                parallelism: ARGON2_PARALLELISM,
                salt: salt.to_vec(),
            }),
            cipher: Some(CipherHeader {
                algorithm: "xchacha20poly1305".to_owned(),
                nonce: nonce.to_vec(),
            }),
        },
        ciphertext,
    })
}

fn write_header<W: Write>(writer: &mut W, header: &ContainerHeader) -> Result<()> {
    let header_bytes = serde_json::to_vec(header).context("failed to serialize backup header")?;
    let header_len: u32 = header_bytes
        .len()
        .try_into()
        .context("backup header is too large")?;

    writer
        .write_all(MAGIC)
        .context("failed to write backup magic")?;
    writer
        .write_all(&header_len.to_le_bytes())
        .context("failed to write backup header length")?;
    writer
        .write_all(&header_bytes)
        .context("failed to write backup header")?;

    Ok(())
}

fn read_backup_payload(path: &Path, password: Option<&str>) -> Result<Vec<u8>> {
    let mut file = File::open(path).with_context(|| format!("failed to open backup {path:?}"))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .with_context(|| format!("failed to read backup {path:?}"))?;

    if bytes.starts_with(ZIP_MAGIC) {
        return Ok(bytes);
    }
    if !bytes.starts_with(MAGIC) {
        return app_error("不是有效的 EcoPaste 备份文件");
    }

    let mut cursor = Cursor::new(bytes.as_slice());
    cursor.set_position(MAGIC.len() as u64);
    let header = read_container_header_after_magic(&mut cursor)?;
    let ciphertext_start = cursor.position() as usize;
    let Some(kdf) = header.kdf else {
        return app_error("加密备份文件头无效");
    };
    let Some(cipher) = header.cipher else {
        return app_error("加密备份文件头无效");
    };
    let Some(password) = password else {
        return app_error("请输入备份密码");
    };

    decrypt_payload(&bytes[ciphertext_start..], password, &kdf, &cipher)
}

fn decrypt_payload(
    ciphertext: &[u8],
    password: &str,
    kdf: &KdfHeader,
    cipher_header: &CipherHeader,
) -> Result<Vec<u8>> {
    if kdf.algorithm != "argon2id" || cipher_header.algorithm != "xchacha20poly1305" {
        return app_error("暂不支持该备份加密格式");
    }
    if kdf.salt.len() != SALT_LEN || cipher_header.nonce.len() != NONCE_LEN {
        return app_error("加密备份文件头无效");
    }

    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    let params = Params::new(
        kdf.memory_kib,
        kdf.time_cost,
        kdf.parallelism,
        Some(KEY_LEN),
    )
    .context("failed to build argon2 params")?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    argon2
        .hash_password_into(password.as_bytes(), &kdf.salt, key.as_mut())
        .context("failed to derive backup key")?;

    let cipher =
        XChaCha20Poly1305::new_from_slice(key.as_ref()).context("failed to initialize cipher")?;
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&cipher_header.nonce);
    let nonce = XNonce::from(nonce);
    cipher
        .decrypt(&nonce, ciphertext)
        .map_err(|_| AppError::Other(anyhow!("备份密码不正确或文件已损坏")))
}

fn extract_payload_zip(payload: &[u8]) -> Result<TempDir> {
    let temp = tempfile::tempdir().context("failed to create temporary import directory")?;
    let cursor = Cursor::new(payload);
    let mut archive = ZipArchive::new(cursor).context("failed to read backup zip payload")?;
    archive
        .extract(temp.path())
        .context("failed to extract backup payload")?;

    Ok(temp)
}

fn validate_extracted_payload(root: &Path) -> Result<()> {
    if !root.join(MANIFEST_FILENAME).exists() {
        return app_error("备份文件缺少 manifest.json");
    }
    if !root.join(DB_ARCHIVE_DIR).join(DB_FILENAME).exists() {
        return app_error("备份文件缺少历史数据库");
    }
    if !root
        .join(CONFIG_ARCHIVE_DIR)
        .join(SETTINGS_FILENAME)
        .exists()
    {
        return app_error("备份文件缺少设置文件");
    }

    Ok(())
}

fn inspect_backup_reader<R: Read>(reader: &mut R) -> Result<BackupContainerMode> {
    let mut magic = [0u8; MAGIC.len()];
    let read = reader
        .read(&mut magic)
        .context("failed to read backup magic")?;
    if read >= MAGIC.len() && &magic == MAGIC {
        let header = read_container_header_after_magic(reader)?;
        return Ok(header.mode);
    }
    if read >= ZIP_MAGIC.len() && &magic[..ZIP_MAGIC.len()] == ZIP_MAGIC {
        return Ok(BackupContainerMode::Plain);
    }

    app_error("不是有效的 EcoPaste 备份文件")
}

fn read_container_header_after_magic<R: Read>(reader: &mut R) -> Result<ContainerHeader> {
    let mut len = [0u8; HEADER_LEN_BYTES];
    reader
        .read_exact(&mut len)
        .context("failed to read backup header length")?;
    let header_len = u32::from_le_bytes(len) as usize;
    if header_len == 0 || header_len > 64 * 1024 {
        return app_error("备份文件头无效");
    }

    let mut header = vec![0u8; header_len];
    reader
        .read_exact(&mut header)
        .context("failed to read backup header")?;
    let header: ContainerHeader =
        serde_json::from_slice(&header).context("failed to parse backup header")?;
    if header.format_version != FORMAT_VERSION {
        return app_error("暂不支持该备份格式版本");
    }

    Ok(header)
}

async fn merge_import(
    app: &AppHandle,
    pool: &SqlitePool,
    root: &Path,
    _options: &ImportHistoryBackupOptions,
) -> Result<ImportHistoryBackupResult> {
    let db_path = root.join(DB_ARCHIVE_DIR).join(DB_FILENAME);
    let backup_pool = open_backup_db(&db_path).await?;
    let outcome = merge_history(pool, &backup_pool).await?;
    let resources = root.join(RESOURCES_ARCHIVE_DIR);
    let imported_resources =
        copy_dir_missing(&resources, &crate::core::paths::resources_dir(app)?)?;
    emit_clipboard_imported(app);

    let settings_path = root.join(CONFIG_ARCHIVE_DIR).join(SETTINGS_FILENAME);
    let patch = read_json_file(&settings_path)?;
    let next = app
        .state::<crate::settings::SettingsStore>()
        .update(patch)?;
    emit_settings_updated(app, &next);

    Ok(ImportHistoryBackupResult {
        strategy: BackupImportStrategy::Merge,
        imported_items: outcome.imported_items,
        skipped_items: outcome.skipped_items,
        imported_resources,
        imported_settings: true,
        requires_restart: false,
    })
}

async fn overwrite_import(
    app: &AppHandle,
    db: &crate::db::DatabaseState,
    root: &Path,
    _options: &ImportHistoryBackupOptions,
) -> Result<ImportHistoryBackupResult> {
    let _pause_guard = pause_watcher(app);

    let db_src = root.join(DB_ARCHIVE_DIR).join(DB_FILENAME);
    let app_for_db = app.clone();
    db.close_and_replace(|| {
        let app = app_for_db.clone();
        let db_src = db_src.clone();
        async move {
            replace_live_database(&app, &db_src)?;
            crate::db::init(&app).await
        }
    })
    .await?;

    let resources_src = root.join(RESOURCES_ARCHIVE_DIR);
    if resources_src.exists() {
        replace_dir(&resources_src, &crate::core::paths::resources_dir(app)?)?;
    }

    refresh_apps_registry(app).await;
    emit_clipboard_imported(app);

    let settings_path = root.join(CONFIG_ARCHIVE_DIR).join(SETTINGS_FILENAME);
    let next = app
        .state::<crate::settings::SettingsStore>()
        .replace_from_file(&settings_path)?;
    emit_settings_updated(app, &next);

    Ok(ImportHistoryBackupResult {
        strategy: BackupImportStrategy::Overwrite,
        imported_items: 0,
        skipped_items: 0,
        imported_resources: 0,
        imported_settings: true,
        requires_restart: false,
    })
}

struct MergeOutcome {
    imported_items: u64,
    skipped_items: u64,
}

async fn open_backup_db(path: &Path) -> Result<SqlitePool> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true);

    Ok(SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .with_context(|| format!("failed to open backup database at {path:?}"))?)
}

async fn merge_history(current: &SqlitePool, backup: &SqlitePool) -> Result<MergeOutcome> {
    let mut tx = current.begin().await.context("failed to begin import")?;
    merge_groups(&mut tx, backup).await?;
    merge_apps(&mut tx, backup).await?;
    merge_file_type_icons(&mut tx, backup).await?;
    let outcome = merge_items(&mut tx, backup).await?;
    tx.commit().await.context("failed to commit import")?;

    Ok(outcome)
}

async fn merge_groups(tx: &mut sqlx::Transaction<'_, Sqlite>, backup: &SqlitePool) -> Result<()> {
    let rows = sqlx::query_as::<
        _,
        (
            String,
            String,
            String,
            bool,
            i64,
            DateTime<Utc>,
            DateTime<Utc>,
        ),
    >(
        "SELECT id, name, icon, is_hidden, sort_order, created_at, updated_at FROM clipboard_groups",
    )
    .fetch_all(backup)
    .await
    .context("failed to read backup groups")?;

    for row in rows {
        sqlx::query(
            "INSERT OR IGNORE INTO clipboard_groups \
             (id, name, icon, is_hidden, sort_order, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(row.0)
        .bind(row.1)
        .bind(row.2)
        .bind(row.3)
        .bind(row.4)
        .bind(row.5)
        .bind(row.6)
        .execute(&mut **tx)
        .await
        .context("failed to import group")?;
    }

    Ok(())
}

async fn merge_apps(tx: &mut sqlx::Transaction<'_, Sqlite>, backup: &SqlitePool) -> Result<()> {
    let rows = sqlx::query_as::<
        _,
        (
            String,
            String,
            Option<String>,
            String,
            DateTime<Utc>,
            DateTime<Utc>,
        ),
    >(
        "SELECT id, name, icon_file, platform, created_at, updated_at FROM clipboard_apps"
    )
    .fetch_all(backup)
    .await
    .context("failed to read backup apps")?;

    for row in rows {
        sqlx::query(
            "INSERT OR IGNORE INTO clipboard_apps \
             (id, name, icon_file, platform, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(row.0)
        .bind(row.1)
        .bind(row.2)
        .bind(row.3)
        .bind(row.4)
        .bind(row.5)
        .execute(&mut **tx)
        .await
        .context("failed to import app")?;
    }

    Ok(())
}

async fn merge_file_type_icons(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    backup: &SqlitePool,
) -> Result<()> {
    let rows = sqlx::query_as::<_, (String, String, String, DateTime<Utc>, DateTime<Utc>)>(
        "SELECT cache_key, platform, icon_file, created_at, updated_at FROM file_type_icons",
    )
    .fetch_all(backup)
    .await
    .context("failed to read backup file type icons")?;

    for row in rows {
        sqlx::query(
            "INSERT OR IGNORE INTO file_type_icons \
             (cache_key, platform, icon_file, created_at, updated_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(row.0)
        .bind(row.1)
        .bind(row.2)
        .bind(row.3)
        .bind(row.4)
        .execute(&mut **tx)
        .await
        .context("failed to import file type icon")?;
    }

    Ok(())
}

async fn merge_items(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    backup: &SqlitePool,
) -> Result<MergeOutcome> {
    let rows = sqlx::query_as::<_, BackupItemRow>(
        "SELECT id, kind, sub_kind, group_id, source_app_id, content, content_hash, search_text, \
         summary, file_types, size, width, height, use_count, is_favorite, is_pinned, is_sensitive, platform, note, \
         created_at, updated_at FROM clipboard_items ORDER BY created_at ASC",
    )
    .fetch_all(backup)
    .await
    .context("failed to read backup items")?;

    // 查重集合一次拉取到内存（kind + content_hash 与 upsert_item 同语义），
    // 替代旧实现的逐行 SELECT——万条历史从 ~2N 次 DB 往返降到 1 次。
    let existing: std::collections::HashSet<(String, String)> =
        sqlx::query_as::<_, (String, String)>("SELECT kind, content_hash FROM clipboard_items")
            .fetch_all(&mut **tx)
            .await
            .context("failed to read existing item hashes")?
            .into_iter()
            .collect();

    let pending: Vec<&BackupItemRow> = rows
        .iter()
        .filter(|row| !existing.contains(&(row.kind.clone(), row.content_hash.clone())))
        .collect();

    let skipped_items = (rows.len() - pending.len()) as u64;
    let imported_items = insert_items_in_chunks(tx, &pending).await? as u64;

    Ok(MergeOutcome {
        imported_items,
        skipped_items,
    })
}

/// 分块批量插入：单条多值 `INSERT OR IGNORE` 每块最多 500 行，
/// 21 列 × 500 行 = 10500 绑定参数，低于 SQLite 32766 上限。
const MERGE_INSERT_CHUNK: usize = 500;

async fn insert_items_in_chunks(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    pending: &[&BackupItemRow],
) -> Result<usize> {
    let mut imported = 0;

    for chunk in pending.chunks(MERGE_INSERT_CHUNK) {
        let mut qb = QueryBuilder::new(
            "INSERT OR IGNORE INTO clipboard_items \
             (id, kind, sub_kind, group_id, source_app_id, content, content_hash, search_text, \
              summary, file_types, size, width, height, use_count, is_favorite, is_pinned, is_sensitive, platform, note, \
              created_at, updated_at) ",
        );

        qb.push_values(chunk, |mut builder, row| {
            builder
                .push_bind(row.id.clone())
                .push_bind(row.kind.clone())
                .push_bind(row.sub_kind.clone())
                .push_bind(row.group_id.clone())
                .push_bind(row.source_app_id.clone())
                .push_bind(row.content.clone())
                .push_bind(row.content_hash.clone())
                .push_bind(row.search_text.clone())
                .push_bind(row.summary.clone())
                .push_bind(row.file_types.clone())
                .push_bind(row.size)
                .push_bind(row.width)
                .push_bind(row.height)
                .push_bind(row.use_count)
                .push_bind(row.is_favorite)
                .push_bind(row.is_pinned)
                .push_bind(row.is_sensitive)
                .push_bind(row.platform.clone())
                .push_bind(row.note.clone())
                .push_bind(row.created_at)
                .push_bind(row.updated_at);
        });

        let result = qb
            .build()
            .execute(&mut **tx)
            .await
            .context("failed to import items chunk")?;
        imported += result.rows_affected() as usize;
    }

    Ok(imported)
}

#[derive(sqlx::FromRow)]
struct BackupItemRow {
    id: String,
    kind: String,
    sub_kind: Option<String>,
    group_id: Option<String>,
    source_app_id: Option<String>,
    content: String,
    content_hash: String,
    search_text: Option<String>,
    summary: Option<String>,
    file_types: Option<String>,
    size: Option<i64>,
    width: Option<i64>,
    height: Option<i64>,
    use_count: i64,
    is_favorite: bool,
    is_pinned: bool,
    is_sensitive: bool,
    platform: String,
    note: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

struct WatcherPauseRestore {
    pause: Option<crate::clipboard::WatcherPause>,
    previous: bool,
}

impl Drop for WatcherPauseRestore {
    fn drop(&mut self) {
        if let Some(pause) = &self.pause {
            pause.set_paused(self.previous);
        }
    }
}

/// 暂停剪贴板监听，并在 guard drop 时恢复导入前的暂停状态。
fn pause_watcher(app: &AppHandle) -> WatcherPauseRestore {
    let pause = app.try_state::<crate::clipboard::WatcherPause>();
    let previous = pause.as_ref().is_some_and(|state| state.is_paused());
    if let Some(state) = pause.as_ref() {
        state.set_paused(true);
    }

    WatcherPauseRestore {
        pause: pause.map(|state| state.inner().clone()),
        previous,
    }
}

/// 用备份数据库替换当前主数据库，并移除旧 WAL / SHM sidecar。
fn replace_live_database(app: &AppHandle, src: &Path) -> Result<()> {
    let dst = crate::db::db_path(app)?;
    copy_file_to(src, &dst)?;

    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", dst.display()));
        if sidecar.exists() {
            fs::remove_file(&sidecar).with_context(|| format!("failed to remove {sidecar:?}"))?;
        }
    }

    Ok(())
}

/// 覆盖导入后从新数据库重建来源应用内存缓存。
async fn refresh_apps_registry(app: &AppHandle) {
    let Some(registry) = app.try_state::<crate::clipboard::AppsRegistry>() else {
        return;
    };

    if let Err(err) = registry.load_from_db().await {
        log::warn!("refresh apps registry after backup overwrite failed: {err}");
    }
}

fn read_json_file(path: &Path) -> Result<serde_json::Value> {
    let content = fs::read_to_string(path).with_context(|| format!("failed to read {path:?}"))?;
    Ok(serde_json::from_str(&content).with_context(|| format!("failed to parse {path:?}"))?)
}

fn copy_file_to(src: &Path, dst: &Path) -> Result<()> {
    let parent = dst
        .parent()
        .ok_or_else(|| anyhow!("target path has no parent"))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {parent:?}"))?;
    fs::copy(src, dst).with_context(|| format!("failed to copy {src:?} to {dst:?}"))?;

    Ok(())
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<u64> {
    if !src.exists() {
        return Ok(0);
    }

    let mut copied = 0;
    for entry in WalkDir::new(src)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        let path = entry.path();
        let relative = path
            .strip_prefix(src)
            .with_context(|| format!("failed to strip {src:?} from {path:?}"))?;
        if relative.as_os_str().is_empty() {
            continue;
        }

        let target = dst.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)
                .with_context(|| format!("failed to create directory {target:?}"))?;
            continue;
        }

        copy_file_to(path, &target)?;
        copied += 1;
    }

    Ok(copied)
}

fn copy_dir_missing(src: &Path, dst: &Path) -> Result<u64> {
    if !src.exists() {
        return Ok(0);
    }

    let mut copied = 0;
    for entry in WalkDir::new(src)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        let path = entry.path();
        if !entry.file_type().is_file() {
            continue;
        }

        let relative = path
            .strip_prefix(src)
            .with_context(|| format!("failed to strip {src:?} from {path:?}"))?;
        let target = dst.join(relative);
        if target.exists() {
            continue;
        }

        copy_file_to(path, &target)?;
        copied += 1;
    }

    Ok(copied)
}

fn replace_dir(src: &Path, dst: &Path) -> Result<()> {
    if dst.exists() {
        fs::remove_dir_all(dst).with_context(|| format!("failed to remove {dst:?}"))?;
    }
    copy_dir_all(src, dst)?;

    Ok(())
}

fn emit_clipboard_imported(app: &AppHandle) {
    if let Err(err) = app.emit(
        "clipboard://updated",
        serde_json::json!({
            "id": null,
            "deduplicated": false,
            "imported": true,
        }),
    ) {
        log::warn!("emit clipboard import update failed: {err}");
    }
}

fn emit_settings_updated(app: &AppHandle, settings: &crate::settings::Settings) {
    if let Err(err) = app.emit("settings://updated", settings) {
        log::warn!("emit settings import update failed: {err}");
    }
}

fn copy_file_into_writer<W: Write>(path: &Path, writer: &mut W) -> Result<()> {
    let file = File::open(path).with_context(|| format!("failed to open {path:?}"))?;
    let mut reader = BufReader::new(file);
    std::io::copy(&mut reader, writer).with_context(|| format!("failed to copy {path:?}"))?;

    Ok(())
}

fn dir_size(path: &Path) -> Result<u64> {
    if !path.exists() {
        return Ok(0);
    }

    let mut total = 0;
    for entry in WalkDir::new(path)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        let path = entry.path();
        let metadata = entry
            .metadata()
            .with_context(|| format!("failed to read metadata at {path:?}"))?;

        if metadata.is_file() {
            total += metadata.len();
        }
    }

    Ok(total)
}

fn app_error<T>(message: impl Into<String>) -> Result<T> {
    Err(AppError::Other(anyhow!(message.into())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tempfile::tempdir;
    use zip::ZipArchive;

    #[test]
    fn backup_extension_validation_accepts_expected_suffix() {
        let path = normalize_backup_path(PathBuf::from("demo.ecopastebak")).unwrap();
        assert_eq!(path, PathBuf::from("demo.ecopastebak"));
    }

    #[test]
    fn backup_extension_validation_appends_missing_suffix() {
        let path = normalize_backup_path(PathBuf::from("demo")).unwrap();
        assert_eq!(path, PathBuf::from("demo.ecopastebak"));
    }

    #[test]
    fn backup_extension_validation_rejects_other_suffix() {
        assert!(normalize_backup_path(PathBuf::from("demo.zip")).is_err());
    }

    #[test]
    fn encrypted_payload_header_round_trips() {
        let encrypted = encrypt_payload(b"hello", "password-123").unwrap();
        let mut bytes = Vec::new();
        write_header(&mut bytes, &encrypted.header).unwrap();
        bytes.extend_from_slice(&encrypted.ciphertext);

        let mut cursor = Cursor::new(bytes);

        assert_eq!(
            inspect_backup_reader(&mut cursor).unwrap(),
            BackupContainerMode::Encrypted
        );
    }

    #[test]
    fn inspect_backup_reader_recognizes_plain_zip() {
        let mut cursor = Cursor::new(b"PK\x03\x04demo".to_vec());

        assert_eq!(
            inspect_backup_reader(&mut cursor).unwrap(),
            BackupContainerMode::Plain
        );
    }

    #[test]
    fn inspect_backup_reader_recognizes_encrypted_container() {
        let encrypted = encrypt_payload(b"hello", "password-123").unwrap();
        let mut bytes = Vec::new();
        write_header(&mut bytes, &encrypted.header).unwrap();
        bytes.extend_from_slice(&encrypted.ciphertext);
        let mut cursor = Cursor::new(bytes);

        assert_eq!(
            inspect_backup_reader(&mut cursor).unwrap(),
            BackupContainerMode::Encrypted
        );
    }

    #[test]
    fn payload_zip_contains_only_backup_whitelist() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        let resources = root.join("resources");
        fs::create_dir_all(resources.join("clipboard-images/origin")).unwrap();
        fs::write(root.join("clipboard.db"), b"db").unwrap();
        fs::write(root.join("settings.json"), b"{}").unwrap();
        fs::write(root.join("window-state.json"), b"skip").unwrap();
        fs::write(root.join("settings.json.bak"), b"skip").unwrap();
        fs::write(resources.join("clipboard-images/origin/demo.png"), b"image").unwrap();

        let payload = root.join("payload.zip");
        let target = root.join("backup.ecopastebak");
        let manifest = BackupManifest {
            format_version: FORMAT_VERSION,
            app_name: "EcoPaste".to_owned(),
            app_version: "0.0.0".to_owned(),
            exported_at: Utc::now(),
            platform: "macos".to_owned(),
            encryption: ManifestEncryption::None,
            item_count: 1,
            text_count: 1,
            image_count: 0,
            files_count: 0,
            resource_bytes: 5,
        };
        let source_paths = BackupSourcePaths {
            db_path: root.join("clipboard.db"),
            resources_dir: resources,
            settings_path: root.join("settings.json"),
        };

        write_payload_zip(&source_paths, &payload, &manifest, &target).unwrap();

        let file = File::open(payload).unwrap();
        let mut archive = ZipArchive::new(file).unwrap();
        let mut names = Vec::new();
        for index in 0..archive.len() {
            let file = archive.by_index(index).unwrap();
            names.push(file.name().to_owned());
        }
        names.sort();

        assert_eq!(
            names,
            vec![
                "config/settings.json",
                "db/clipboard.db",
                "manifest.json",
                "resources/clipboard-images/origin/demo.png",
            ]
        );
    }

    #[tokio::test]
    async fn merge_preserves_is_sensitive_flag() {
        use crate::db::items::{content_hash, insert_item};
        use crate::db::models::{ClipboardItem, ClipboardKind, Platform};
        use crate::db::test_support::memory_pool;
        use chrono::DateTime;

        // The backup pool stands in for a real backup database, which is a
        // snapshot of the live clipboard.db and therefore already carries the
        // `is_sensitive` flag written by the production insert path.
        let backup = memory_pool().await;
        let current = memory_pool().await;

        let ts = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let content = "AKIAIOSFODNN7EXAMPLE".to_owned();
        let item = ClipboardItem {
            id: "secret-1".to_owned(),
            kind: ClipboardKind::Text,
            sub_kind: None,
            group_id: None,
            source_app_id: None,
            content_hash: content_hash(ClipboardKind::Text, &content),
            content,
            search_text: None,
            summary: None,
            file_types: None,
            size: None,
            width: None,
            height: None,
            use_count: 1,
            is_favorite: false,
            is_pinned: false,
            is_sensitive: true,
            platform: Platform::Macos,
            note: None,
            created_at: ts,
            updated_at: ts,
            source_app_name: None,
            source_app_icon_file: None,
            source_app_icon_path: None,
            image_thumbnail_path: None,
            file_entries: None,
            files_preview_kind: None,
            available_actions: Vec::new(),
            color_preview: None,
            display_created_at: String::new(),
        };
        insert_item(&backup, &item).await.unwrap();

        let outcome = merge_history(&current, &backup).await.unwrap();
        assert_eq!(
            outcome.imported_items, 1,
            "sensitive item should be imported"
        );

        // Regression: the merge SELECT/INSERT used to omit `is_sensitive`, so
        // every imported row silently fell back to the column DEFAULT (0) and
        // secrets were rendered in plaintext. The flag must survive the merge.
        let imported: bool =
            sqlx::query_scalar("SELECT is_sensitive FROM clipboard_items WHERE id = ?")
                .bind("secret-1")
                .fetch_one(&current)
                .await
                .unwrap();
        assert!(
            imported,
            "merged sensitive item must keep is_sensitive = true"
        );
    }

    /// 批量导出核心装配：过滤库只含所选条目 + 其引用的分组；无关分组被剔除、
    /// 来源应用维度保留；FTS 在过滤库内可命中；不存在的 id 静默跳过；全空报错。
    #[tokio::test]
    async fn build_filtered_db_keeps_selected_items_and_referenced_dimensions() {
        use crate::db::apps::upsert_app;
        use crate::db::groups::insert_group;
        use crate::db::items::{content_hash, insert_item};
        use crate::db::models::{
            ClipboardApp, ClipboardGroup, ClipboardItem, ClipboardKind, Platform,
        };
        use crate::db::test_support::memory_pool;
        use chrono::DateTime;

        let ts = DateTime::from_timestamp(1_700_000_000, 0).unwrap();

        let make_item = |id: &str, group_id: Option<&str>, search_text: Option<&str>| {
            let content = format!("export body {id}");
            ClipboardItem {
                id: id.to_owned(),
                kind: ClipboardKind::Text,
                sub_kind: None,
                group_id: group_id.map(str::to_owned),
                source_app_id: Some("app.a".to_owned()),
                content_hash: content_hash(ClipboardKind::Text, &content),
                content,
                search_text: search_text.map(str::to_owned),
                summary: None,
                file_types: None,
                size: None,
                width: None,
                height: None,
                use_count: 1,
                is_favorite: false,
                is_pinned: false,
                is_sensitive: false,
                platform: Platform::Macos,
                note: None,
                created_at: ts,
                updated_at: ts,
                source_app_name: None,
                source_app_icon_file: None,
                source_app_icon_path: None,
                image_thumbnail_path: None,
                file_entries: None,
                files_preview_kind: None,
                available_actions: Vec::new(),
                color_preview: None,
                display_created_at: String::new(),
            }
        };

        let pool = memory_pool().await;
        insert_group(
            &pool,
            &ClipboardGroup {
                id: "keep".to_owned(),
                name: "Keep".to_owned(),
                icon: "i-lucide:folder".to_owned(),
                is_hidden: false,
                sort_order: 0,
                created_at: ts,
                updated_at: ts,
            },
        )
        .await
        .unwrap();
        insert_group(
            &pool,
            &ClipboardGroup {
                id: "drop".to_owned(),
                name: "Drop".to_owned(),
                icon: "i-lucide:folder".to_owned(),
                is_hidden: false,
                sort_order: 1,
                created_at: ts,
                updated_at: ts,
            },
        )
        .await
        .unwrap();
        upsert_app(
            &pool,
            &ClipboardApp {
                id: "app.a".to_owned(),
                name: "App A".to_owned(),
                icon_file: Some("aaaa.png".to_owned()),
                platform: Platform::Macos,
                created_at: ts,
                updated_at: ts,
            },
        )
        .await
        .unwrap();
        for (id, group) in [("e1", Some("keep")), ("e2", None), ("e3", Some("keep"))] {
            let mut item = make_item(id, group, Some(&format!("export note {id}")));
            item.content = format!("export body {id}");
            item.content_hash = content_hash(ClipboardKind::Text, &item.content);
            insert_item(&pool, &item).await.unwrap();
        }
        let mut unselected = make_item("e9", Some("drop"), Some("unselected body"));
        unselected.content = "unselected body".to_owned();
        unselected.content_hash = content_hash(ClipboardKind::Text, &unselected.content);
        insert_item(&pool, &unselected).await.unwrap();

        let temp = tempdir().unwrap();
        let filtered_path = temp.path().join("filtered.db");
        let items = build_filtered_db(
            &pool,
            &["e1".to_owned(), "e2".to_owned(), "missing".to_owned()],
            &filtered_path,
        )
        .await
        .unwrap();

        assert_eq!(items.len(), 2, "missing id is skipped silently");

        let counts = count_filtered_items(&items);
        assert_eq!(counts.item_count, 2);
        assert_eq!(counts.text_count, 2);
        assert_eq!(counts.image_count, 0);

        // Re-open the filtered database and assert its contents.
        let options = SqliteConnectOptions::new()
            .filename(&filtered_path)
            .read_only(true);
        let filtered = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();

        let item_rows: Vec<String> =
            sqlx::query_scalar("SELECT id FROM clipboard_items ORDER BY id")
                .fetch_all(&filtered)
                .await
                .unwrap();
        assert_eq!(item_rows, vec!["e1".to_owned(), "e2".to_owned()]);

        let group_rows: Vec<String> = sqlx::query_scalar("SELECT id FROM clipboard_groups")
            .fetch_all(&filtered)
            .await
            .unwrap();
        assert_eq!(
            group_rows,
            vec!["keep".to_owned()],
            "unreferenced groups must not leak into the filtered db"
        );

        let app_rows: Vec<String> = sqlx::query_scalar("SELECT id FROM clipboard_apps")
            .fetch_all(&filtered)
            .await
            .unwrap();
        assert_eq!(app_rows, vec!["app.a".to_owned()]);

        // FTS was rebuilt by the INSERT triggers inside the filtered db.
        let fts_hits: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM clipboard_items_fts WHERE clipboard_items_fts MATCH 'export'",
        )
        .fetch_one(&filtered)
        .await
        .unwrap();
        assert_eq!(fts_hits, 2, "FTS must be rebuilt for the filtered rows");

        filtered.close().await;

        // All-missing ids is a user-facing error, not an empty backup.
        let empty = build_filtered_db(
            &pool,
            &["nope-1".to_owned(), "nope-2".to_owned()],
            &temp.path().join("empty.db"),
        )
        .await;
        assert!(empty.is_err(), "exporting zero existing items must fail");
    }

    /// manifest 身份字段与 kind 计数：`build_manifest_with_identity` 直填字符串，
    /// 加密模式映射为 `password`、明文映射为 `none`。
    #[test]
    fn build_manifest_with_identity_maps_mode_and_counts() {
        let counts = BackupCounts {
            item_count: 3,
            text_count: 1,
            image_count: 1,
            files_count: 1,
        };
        let manifest = build_manifest_with_identity(
            "EcoPaste",
            "1.2.3",
            Utc::now(),
            BackupExportMode::Encrypted,
            counts,
            42,
        )
        .unwrap();

        assert_eq!(manifest.app_name, "EcoPaste");
        assert_eq!(manifest.app_version, "1.2.3");
        assert_eq!(manifest.encryption, ManifestEncryption::Password);
        assert_eq!(manifest.item_count, 3);
        assert_eq!(manifest.text_count, 1);
        assert_eq!(manifest.image_count, 1);
        assert_eq!(manifest.files_count, 1);
        assert_eq!(manifest.resource_bytes, 42);
    }

    /// 端到端（无 AppHandle）：`export_items_backup` 的三段装配——过滤库、
    /// 资源子集物化、payload zip + container——在纯文件/内存环境下串起来，
    /// 产出真实 `.ecopastebak`，再走导入端的识别 / 解包 / 校验路径读回来。
    /// 证明「所选条目」备份能被现有导入链路无损消费。
    #[tokio::test]
    async fn items_export_payload_round_trips_through_import_reader() {
        use crate::db::groups::insert_group;
        use crate::db::items::{content_hash, insert_item};
        use crate::db::models::ClipboardGroup;
        use crate::db::test_support::memory_pool;
        use chrono::DateTime;

        let ts = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let pool = memory_pool().await;

        insert_group(
            &pool,
            &ClipboardGroup {
                id: "g1".to_owned(),
                name: "G1".to_owned(),
                icon: "i-lucide:folder".to_owned(),
                is_hidden: false,
                sort_order: 0,
                created_at: ts,
                updated_at: ts,
            },
        )
        .await
        .unwrap();

        for id in ["t1", "t2"] {
            let content = format!("round trip {id}");
            let mut item = round_trip_item(id, "g1", &content, ts);
            item.content_hash = content_hash(crate::db::models::ClipboardKind::Text, &content);
            insert_item(&pool, &item).await.unwrap();
        }

        // Temp work dir mirrors what export_items_backup stages for the zip.
        let work = tempdir().unwrap();
        let items = build_filtered_db(
            &pool,
            &["t1".to_owned(), "t2".to_owned(), "nope".to_owned()],
            &work.path().join("clipboard.db"),
        )
        .await
        .unwrap();
        assert_eq!(items.len(), 2);

        let resources = work.path().join("resources");
        fs::create_dir_all(resources.join("clipboard-images/origin/ab")).unwrap();
        fs::write(
            resources.join("clipboard-images/origin/ab/abcd.png"),
            b"png-bytes",
        )
        .unwrap();

        let settings = work.path().join("settings.json");
        fs::write(&settings, b"{\"locale\":\"zh-CN\"}").unwrap();

        let counts = count_filtered_items(&items);
        let manifest = build_manifest_with_identity(
            "EcoPaste",
            "test",
            Utc::now(),
            BackupExportMode::Plain,
            counts,
            9,
        )
        .unwrap();
        let source_paths = BackupSourcePaths {
            db_path: work.path().join("clipboard.db"),
            resources_dir: resources.clone(),
            settings_path: settings,
        };

        let payload = NamedTempFile::new().unwrap();
        let target = work.path().join("items.ecopastebak");
        write_payload_zip(&source_paths, payload.path(), &manifest, &target).unwrap();
        let _total_bytes =
            write_container(&target, payload.path(), BackupExportMode::Plain, None).unwrap();

        // Import side recognizes the container and the extracted payload passes
        // the same validation gate real imports run.
        assert_eq!(
            inspect_backup_file(&target).unwrap(),
            BackupContainerMode::Plain
        );
        let bytes = read_backup_payload(&target, None).unwrap();
        let extracted = extract_payload_zip(&bytes).unwrap();
        validate_extracted_payload(extracted.path()).unwrap();

        let names = archive_file_names(&bytes);
        assert!(names.contains(&"config/settings.json".to_owned()));
        assert!(names.contains(&"db/clipboard.db".to_owned()));
        assert!(names.contains(&"manifest.json".to_owned()));
        assert!(names.contains(&"resources/clipboard-images/origin/ab/abcd.png".to_owned()));

        // The filtered db inside the payload keeps only the selected rows.
        let inner = open_backup_db(&extracted.path().join("db").join("clipboard.db"))
            .await
            .unwrap();
        let inner_items: Vec<String> =
            sqlx::query_scalar("SELECT id FROM clipboard_items ORDER BY id")
                .fetch_all(&inner)
                .await
                .unwrap();
        assert_eq!(inner_items, vec!["t1".to_owned(), "t2".to_owned()]);
        inner.close().await;
    }

    /// 单测用最小条目骨架（batch export round-trip 专属，字段全默认值）。
    fn round_trip_item(
        id: &str,
        group_id: &str,
        content: &str,
        ts: DateTime<Utc>,
    ) -> crate::db::models::ClipboardItem {
        crate::db::models::ClipboardItem {
            id: id.to_owned(),
            kind: crate::db::models::ClipboardKind::Text,
            sub_kind: None,
            group_id: Some(group_id.to_owned()),
            source_app_id: None,
            content: content.to_owned(),
            content_hash: String::new(),
            search_text: Some(content.to_owned()),
            summary: None,
            file_types: None,
            size: None,
            width: None,
            height: None,
            use_count: 1,
            is_favorite: false,
            is_pinned: false,
            is_sensitive: false,
            platform: crate::db::models::Platform::Macos,
            note: None,
            created_at: ts,
            updated_at: ts,
            source_app_name: None,
            source_app_icon_file: None,
            source_app_icon_path: None,
            image_thumbnail_path: None,
            file_entries: None,
            files_preview_kind: None,
            available_actions: Vec::new(),
            color_preview: None,
            display_created_at: String::new(),
        }
    }

    /// 列出 zip payload 内全部条目名，供断言备份内容完整性。
    fn archive_file_names(payload: &[u8]) -> Vec<String> {
        let cursor = Cursor::new(payload.to_vec());
        let mut archive = ZipArchive::new(cursor).unwrap();
        (0..archive.len())
            .map(|index| archive.by_index(index).unwrap().name().to_owned())
            .collect()
    }
}

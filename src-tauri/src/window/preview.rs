//! 系统级剪贴板预览窗口骨架。
//!
//! 预览窗口是透明的 full-screen overlay：Rust 负责按需建窗、复用窗口、
//! 收集坐标上下文并广播，前端在该窗口内渲染预览内容与连接曲线。
//! 窗口接入生命周期管理（`DestroyWhenIdle`）：隐藏空闲后销毁 WebView 释放内存，
//! 仅在预览请求到达时按需建窗，不随剪贴板窗口显示预热。

#![allow(clippy::unused_unit)]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalRect, PhysicalSize, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder,
};

use crate::core::Result;

use super::{get_window, lifecycle, CLIPBOARD_PREVIEW_WINDOW_LABEL, CLIPBOARD_WINDOW_LABEL};

#[cfg(target_os = "macos")]
use tauri_nspanel::{tauri_panel, ManagerExt, PanelLevel, StyleMask, WebviewWindowExt};

#[cfg(target_os = "windows")]
use windows::Win32::Foundation::HWND;
#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{
    SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
};

const PREVIEW_UPDATED_EVENT: &str = "preview://updated";
/// 与前端 `src/constants/events.ts` 的 `TAURI_EVENT.PREVIEW_POINTER` 一一对应。
pub const PREVIEW_POINTER_EVENT: &str = "preview://pointer";
const PREVIEW_PANEL_WIDTH: f64 = 480.0;
const PREVIEW_PANEL_HEIGHT: f64 = 480.0;
const PREVIEW_PANEL_GAP: f64 = 40.0;
const PREVIEW_PANEL_MARGIN: f64 = 32.0;
const PREVIEW_POINTER_ANCHOR_SIZE: f64 = 1.0;
const PREVIEW_HIDE_DELAY_MS: u64 = 180;
/// 指针采样周期（预览可见时）。
const PREVIEW_POINTER_POLL_INTERVAL: Duration = Duration::from_millis(40);
/// 指针采样周期（预览不可见时的空转，只等下一次 show）。
const PREVIEW_POINTER_IDLE_INTERVAL: Duration = Duration::from_millis(200);
/// 命中矩形外扩（逻辑 px）：面板带阴影与边框，点击面板边缘（视觉上仍在面板上）
/// 不应被鼠标钩子判成外部点击；取值刻意压得很小，避免在面板外圈留下一段
/// 「采样认为在面板内、DOM 认为在面板外」的死区。
const PREVIEW_PANEL_HIT_PADDING: f64 = 4.0;

static PREVIEW_REQUEST_ID: AtomicU64 = AtomicU64::new(0);
static PREVIEW_SESSION_ID: AtomicU64 = AtomicU64::new(0);
static PREVIEW_SUPPRESSED: AtomicBool = AtomicBool::new(false);
static PREVIEW_STATE: LazyLock<Mutex<Option<ClipboardPreviewState>>> =
    LazyLock::new(|| Mutex::new(None));
/// 串行化建窗：多个预览请求（如连续 hover）可能并发走到「检查不存在 → 建窗」，
/// 都过了存在性检查会触发重复 label 建窗报错。建窗都来自命令/后台线程、主线程从不持锁，
/// 不会与 builder 内部的主线程派发互锁。
static PREVIEW_BUILD_LOCK: Mutex<()> = Mutex::new(());
/// 前端上报的面板实测矩形（overlay 局部逻辑坐标）；未上报时回退本次 layout 的 `panel_rect`。
static PREVIEW_PANEL_RECT: LazyLock<Mutex<Option<PreviewRect>>> =
    LazyLock::new(|| Mutex::new(None));
/// 面板命中矩形的物理像素边界缓存：指针采样与鼠标钩子每 tick 都要读，
/// 预先算好避免反复克隆整个预览状态。
static PREVIEW_HIT_BOUNDS: LazyLock<Mutex<Option<PreviewHitBounds>>> =
    LazyLock::new(|| Mutex::new(None));
/// 预览 overlay 是否可见；鼠标钩子据此做零成本门控。
static PREVIEW_VISIBLE: AtomicBool = AtomicBool::new(false);
/// 指针当前是否落在面板命中矩形内（穿透开关的镜像）。
static PREVIEW_POINTER_INSIDE: AtomicBool = AtomicBool::new(false);
/// 指针采样线程单飞位。线程随应用存活，预览不可见时空转。
static PREVIEW_POINTER_WATCHER: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "macos")]
tauri_panel! {
    panel!(PreviewPanel {
        config: {
            is_floating_panel: true,
            can_become_key_window: false,
            can_become_main_window: false
        }
    })
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewAnchorRect {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
    pub pointer_y: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewWorkArea {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewClipboardWindowRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRect {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

/// 面板命中矩形的物理像素边界（左闭右开）。
#[derive(Clone, Copy, Debug)]
struct PreviewHitBounds {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl PreviewHitBounds {
    fn contains(self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

/// `preview://pointer` 的载荷：指针是否落在面板命中矩形内。
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewPointerPayload {
    inside: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PreviewPlacement {
    Right,
    Left,
    Bottom,
    Top,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewLayout {
    pub overlay_rect: PreviewRect,
    pub source_rect: PreviewRect,
    pub panel_rect: PreviewRect,
    pub placement: PreviewPlacement,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardPreviewState {
    pub request_id: u64,
    pub session_id: u64,
    pub item_id: String,
    pub anchor: PreviewAnchorRect,
    pub scale_factor: f64,
    pub work_area: PreviewWorkArea,
    pub clipboard_window: Option<PreviewClipboardWindowRect>,
    pub layout: PreviewLayout,
}

/// 打开或重定向预览窗口，并把最新预览状态广播到预览 webview。
pub fn show_clipboard_preview(
    app: &AppHandle,
    item_id: String,
    anchor: PreviewAnchorRect,
) -> Result<Option<ClipboardPreviewState>> {
    validate_anchor(&anchor)?;

    if PREVIEW_SUPPRESSED.load(Ordering::SeqCst) || !is_clipboard_window_visible(app) {
        close_clipboard_preview_now(app)?;
        return Ok(None);
    }

    let request_id = PREVIEW_REQUEST_ID.fetch_add(1, Ordering::SeqCst) + 1;
    let session_id = preview_session_id_for_show();
    let window = ensure_preview_window(app)?;
    let monitor = resolve_preview_monitor(app)?;
    let work_area = preview_overlay_bounds(&monitor);
    let scale_factor = monitor.scale_factor();
    let clipboard_window = clipboard_window_rect(app);
    let layout = build_preview_layout(&anchor, scale_factor, &work_area, clipboard_window.as_ref());

    let state = ClipboardPreviewState {
        request_id,
        session_id,
        item_id,
        anchor,
        scale_factor,
        work_area: PreviewWorkArea {
            x: work_area.position.x,
            y: work_area.position.y,
            width: work_area.size.width,
            height: work_area.size.height,
        },
        layout,
        clipboard_window,
    };

    // 状态先落库再 prepare：retarget 时命中矩形要按**本次** layout 重算。
    set_preview_state(Some(state.clone()));
    prepare_preview_window_for_show(app, &window, &work_area)?;
    window
        .emit(PREVIEW_UPDATED_EVENT, &state)
        .map_err(|e| anyhow::anyhow!(e))?;
    show_preview_window(app, &window, &work_area)?;

    Ok(Some(state))
}

/// 隐藏预览窗口并清空当前预览状态。
///
/// `reason` 由前端各关闭路径传入并落进本地日志：前端日志不会写进 Rust 日志文件，
/// 而「预览为什么自己关了」几乎只能从这里回溯（哪条路径、当时指针是否在面板上）。
pub fn close_clipboard_preview(app: &AppHandle, reason: &str) -> Result<()> {
    log::info!(
        "preview close requested: reason={reason}, visible={}, pointer_inside={}",
        PREVIEW_VISIBLE.load(Ordering::Relaxed),
        PREVIEW_POINTER_INSIDE.load(Ordering::Relaxed)
    );

    let request_id = PREVIEW_REQUEST_ID.fetch_add(1, Ordering::SeqCst) + 1;
    set_preview_state(None);

    if let Some(window) = app.get_webview_window(CLIPBOARD_PREVIEW_WINDOW_LABEL) {
        window
            .emit(PREVIEW_UPDATED_EVENT, Option::<ClipboardPreviewState>::None)
            .map_err(|e| anyhow::anyhow!(e))?;
        schedule_preview_window_hide(app.clone(), window, request_id);
    }

    Ok(())
}

/// 立即隐藏预览窗口并清空状态；用于剪贴板窗口隐藏等不需要退出动画的路径。
pub fn close_clipboard_preview_now(app: &AppHandle) -> Result<()> {
    PREVIEW_REQUEST_ID.fetch_add(1, Ordering::SeqCst);
    set_preview_state(None);

    if let Some(window) = app.get_webview_window(CLIPBOARD_PREVIEW_WINDOW_LABEL) {
        window
            .emit(PREVIEW_UPDATED_EVENT, Option::<ClipboardPreviewState>::None)
            .map_err(|e| anyhow::anyhow!(e))?;
        hide_preview_window(app, &window)?;
    } else {
        // 窗口已被空闲销毁：没有 hide 收口点，这里补齐指针状态复位。
        reset_pointer_tracking();
    }

    Ok(())
}

/// 剪贴板窗口开始隐藏时压制后续过期 show 请求，并立即收起预览窗口。
pub fn suppress_for_clipboard_hide(app: &AppHandle) {
    log::info!("preview suppressed: clipboard window is hiding");

    PREVIEW_SUPPRESSED.store(true, Ordering::SeqCst);
    if let Err(error) = close_clipboard_preview_now(app) {
        log::error!("suppress preview on clipboard hide failed: {error}");
    }
}

/// 剪贴板窗口重新显示后允许新的预览请求进入。预览窗口不随剪贴板窗口预创建，
/// 由首次 [`show_clipboard_preview`] 经 `ensure_preview_window` 按需建窗。
pub fn resume_after_clipboard_show() {
    PREVIEW_SUPPRESSED.store(false, Ordering::SeqCst);
}

/// 返回预览窗口最近一次收到的状态，供预览页首屏补拉。
pub fn get_clipboard_preview_state() -> Result<Option<ClipboardPreviewState>> {
    let guard = PREVIEW_STATE.lock().unwrap_or_else(|poisoned| {
        log::error!("preview state mutex poisoned on get, recovering");
        poisoned.into_inner()
    });

    Ok(guard.clone())
}

/// 记录前端上报的面板实测矩形（overlay 局部逻辑坐标）；非法值直接忽略。
///
/// 前端的面板尺寸按内容动态计算（宽度上限 480、高度按内容与交互态变化），
/// Rust 只能拿到本次 layout 的 480 框，故命中判定以前端实测值为准。
pub fn update_panel_rect(rect: PreviewRect) {
    if !is_valid_rect(rect) {
        log::warn!("ignore invalid preview panel rect: {rect:?}");
        return;
    }

    {
        let mut guard = PREVIEW_PANEL_RECT.lock().unwrap_or_else(|poisoned| {
            log::error!("preview panel rect mutex poisoned on set, recovering");
            poisoned.into_inner()
        });
        *guard = Some(rect);
    }

    refresh_hit_bounds();
    log::info!(
        "preview panel rect reported: logical={rect:?}, hit_bounds={:?}",
        read_hit_bounds()
    );
}

/// 前端面板 `pointerenter` / `pointerleave` 的即时回报：离开方向不能等采样周期，
/// 否则翻转期间整个全屏 overlay 会短暂吞掉面板外的点击。
///
/// `inside = false` 不直接置穿透，而是按真实光标位置重判——DOM 的 leave 与命中矩形
/// 在面板边缘会有 1-4px 的差异，交给同一套判定收口，避免两个真相源互相打脸。
pub fn report_pointer_inside(app: &AppHandle, inside: bool) {
    if !PREVIEW_VISIBLE.load(Ordering::Relaxed) {
        return;
    }

    log::info!(
        "preview panel pointer report: inside={inside}, hit_bounds={:?}",
        read_hit_bounds()
    );

    if inside {
        apply_pointer_inside(app, true);
        return;
    }

    reconcile_pointer(app, false);
}

/// 判断 physical 坐标是否落在当前面板命中矩形内。预览不可见时立即返回 `false`，
/// 供 Windows 鼠标钩子在 button-down 判定链里做零成本门控。
pub fn panel_contains_physical_point(x: i32, y: i32) -> bool {
    if !PREVIEW_VISIBLE.load(Ordering::Relaxed) {
        return false;
    }

    read_hit_bounds().is_some_and(|bounds| bounds.contains(x, y))
}

/// 按真实光标位置校对穿透开关；跨边界时才翻转。
///
/// `enter_only = true` 用于周期采样：**只允许从穿透翻到可交互**。离开方向由面板前端的
/// `pointerleave` 精确驱动——采样若也负责离开，拖选滑出面板边缘（DOM 侧已按「按住期间
/// 不回报」挡掉）就会被中途打回穿透，拖选与滚动会当场断掉。
/// 布局变化（retarget）等显式场景用 `enter_only = false` 做完整校对。
fn reconcile_pointer(app: &AppHandle, enter_only: bool) {
    if !PREVIEW_VISIBLE.load(Ordering::Relaxed) {
        return;
    }

    let Some(bounds) = read_hit_bounds() else {
        return;
    };
    let Some((origin_x, origin_y, scale_factor)) = preview_overlay_metrics() else {
        return;
    };
    let Some((x, y)) = current_cursor_physical(origin_x, origin_y, scale_factor) else {
        return;
    };

    let inside = bounds.contains(x, y);

    if enter_only && !inside {
        return;
    }

    if inside == PREVIEW_POINTER_INSIDE.load(Ordering::Relaxed) {
        return;
    }

    log::info!(
        "preview pointer -> {inside} (cursor={x},{y}, bounds={bounds:?}, enter_only={enter_only})"
    );
    apply_pointer_inside(app, inside);
}

/// 翻转穿透开关并广播 `preview://pointer`；重复调用幂等。
fn apply_pointer_inside(app: &AppHandle, inside: bool) {
    if PREVIEW_POINTER_INSIDE.swap(inside, Ordering::SeqCst) == inside {
        return;
    }

    let handle = app.clone();
    if let Err(err) = app.run_on_main_thread(move || {
        let Some(window) = handle.get_webview_window(CLIPBOARD_PREVIEW_WINDOW_LABEL) else {
            return;
        };
        if let Err(err) = window.set_ignore_cursor_events(!inside) {
            log::warn!("set preview ignore cursor events failed: {err}");
        }
        // macOS：面板转换后 `set_ignore_cursor_events` 走的是 NSWindow 语义，
        // 这里再按 NSPanel 自己的 API 设一次，避免依赖 Tauri 对面板的实现细节。
        #[cfg(target_os = "macos")]
        if let Ok(panel) = handle.get_webview_panel(CLIPBOARD_PREVIEW_WINDOW_LABEL) {
            panel.set_ignores_mouse_events(!inside);
        }
    }) {
        log::warn!("schedule preview pointer toggle failed: {err}");
        return;
    }

    if let Err(err) = app.emit(PREVIEW_POINTER_EVENT, PreviewPointerPayload { inside }) {
        log::warn!("emit {PREVIEW_POINTER_EVENT} failed: {err}");
    }
}

/// 预览隐藏收口：清空命中缓存并复位指针状态，避免残留状态让鼠标钩子误判。
fn reset_pointer_tracking() {
    PREVIEW_VISIBLE.store(false, Ordering::Relaxed);
    PREVIEW_POINTER_INSIDE.store(false, Ordering::SeqCst);

    let mut guard = PREVIEW_HIT_BOUNDS.lock().unwrap_or_else(|poisoned| {
        log::error!("preview hit bounds mutex poisoned on reset, recovering");
        poisoned.into_inner()
    });
    *guard = None;
}

/// 指针采样线程：面板初始对鼠标穿透，前端拿不到 `pointerenter`，
/// 必须由 Rust 主动判断「光标已进入面板矩形」才能翻转，故用低频采样而不是平台事件钩子
/// ——省掉 Windows 低层鼠标钩子的 per-move 开销与 macOS NSEvent 监听的平台依赖。
/// 线程随应用存活，预览不可见时空转。
fn ensure_pointer_watcher(app: &AppHandle) {
    if PREVIEW_POINTER_WATCHER.swap(true, Ordering::SeqCst) {
        return;
    }

    let handle = app.clone();

    thread::spawn(move || loop {
        let visible = PREVIEW_VISIBLE.load(Ordering::Relaxed);

        if visible {
            reconcile_pointer(&handle, true);
        }

        thread::sleep(if visible {
            PREVIEW_POINTER_POLL_INTERVAL
        } else {
            PREVIEW_POINTER_IDLE_INTERVAL
        });
    });
}

/// 按当前预览状态重算命中矩形缓存。
fn refresh_hit_bounds() {
    let bounds = resolve_hit_bounds();

    let mut guard = PREVIEW_HIT_BOUNDS.lock().unwrap_or_else(|poisoned| {
        log::error!("preview hit bounds mutex poisoned on set, recovering");
        poisoned.into_inner()
    });
    *guard = bounds;
}

fn read_hit_bounds() -> Option<PreviewHitBounds> {
    let guard = PREVIEW_HIT_BOUNDS.lock().unwrap_or_else(|poisoned| {
        log::error!("preview hit bounds mutex poisoned on read, recovering");
        poisoned.into_inner()
    });

    *guard
}

/// 面板逻辑矩形 → 屏幕物理像素边界；前端尚未上报实测矩形时回退本次 layout。
fn resolve_hit_bounds() -> Option<PreviewHitBounds> {
    let state = get_clipboard_preview_state().ok().flatten()?;
    let reported = {
        let guard = PREVIEW_PANEL_RECT.lock().unwrap_or_else(|poisoned| {
            log::error!("preview panel rect mutex poisoned on read, recovering");
            poisoned.into_inner()
        });
        *guard
    };
    let rect = reported.unwrap_or(state.layout.panel_rect);

    Some(physical_bounds(
        rect,
        state.scale_factor,
        state.work_area.x,
        state.work_area.y,
    ))
}

/// overlay 局部逻辑矩形 → 屏幕物理像素边界（左闭右开），含命中容差外扩。
fn physical_bounds(
    rect: PreviewRect,
    scale_factor: f64,
    origin_x: i32,
    origin_y: i32,
) -> PreviewHitBounds {
    let scale = if scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let origin_x = f64::from(origin_x);
    let origin_y = f64::from(origin_y);

    PreviewHitBounds {
        bottom: (origin_y + (rect.bottom() + PREVIEW_PANEL_HIT_PADDING) * scale).ceil() as i32,
        left: (origin_x + (rect.left - PREVIEW_PANEL_HIT_PADDING) * scale).floor() as i32,
        right: (origin_x + (rect.right() + PREVIEW_PANEL_HIT_PADDING) * scale).ceil() as i32,
        top: (origin_y + (rect.top - PREVIEW_PANEL_HIT_PADDING) * scale).floor() as i32,
    }
}

fn is_valid_rect(rect: PreviewRect) -> bool {
    [rect.left, rect.top, rect.width, rect.height]
        .iter()
        .all(|value| value.is_finite())
        && rect.width > 0.0
        && rect.height > 0.0
}

/// 取 overlay 的原点（屏幕物理像素）与缩放；无预览状态时为 `None`。
/// 独立于 [`get_clipboard_preview_state`]，采样线程每 tick 调用，不做状态克隆。
fn preview_overlay_metrics() -> Option<(i32, i32, f64)> {
    let guard = PREVIEW_STATE.lock().unwrap_or_else(|poisoned| {
        log::error!("preview state mutex poisoned on metrics, recovering");
        poisoned.into_inner()
    });
    let state = guard.as_ref()?;

    Some((state.work_area.x, state.work_area.y, state.scale_factor))
}

/// 取当前光标的屏幕物理像素坐标。
#[cfg(target_os = "windows")]
fn current_cursor_physical(
    _origin_x: i32,
    _origin_y: i32,
    _scale_factor: f64,
) -> Option<(i32, i32)> {
    use winapi::shared::windef::POINT;
    use winapi::um::winuser::GetCursorPos;

    let mut point = POINT { x: 0, y: 0 };

    if unsafe { GetCursorPos(&mut point) } == 0 {
        return None;
    }

    Some((point.x, point.y))
}

/// Quartz 的全局显示坐标是**点**（原点在主显示器左上、y 向下），需要按 overlay 显示器
/// 换算成屏幕物理像素：显示器原点（物理）加上「光标相对显示器原点的点距离 × 缩放」。
#[cfg(target_os = "macos")]
fn current_cursor_physical(origin_x: i32, origin_y: i32, scale_factor: f64) -> Option<(i32, i32)> {
    use core_graphics::event::CGEvent;
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState).ok()?;
    // `CGEventCreate(NULL)` 返回的事件位置即当前鼠标位置；只读位置、不投递事件，无需额外权限。
    let event = CGEvent::new(source).ok()?;
    let location = event.location();

    Some(screen_points_to_physical(
        location.x,
        location.y,
        origin_x,
        origin_y,
        scale_factor,
    ))
}

/// 屏幕点坐标 → 屏幕物理像素坐标的纯换算，独立出来便于单测。
#[cfg(target_os = "macos")]
fn screen_points_to_physical(
    point_x: f64,
    point_y: f64,
    origin_x: i32,
    origin_y: i32,
    scale_factor: f64,
) -> (i32, i32) {
    let scale = if scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let origin_x = f64::from(origin_x);
    let origin_y = f64::from(origin_y);
    let origin_points_x = origin_x / scale;
    let origin_points_y = origin_y / scale;

    (
        (origin_x + (point_x - origin_points_x) * scale).round() as i32,
        (origin_y + (point_y - origin_points_y) * scale).round() as i32,
    )
}

/// 按需重建预览窗口。预览窗口不再由 Tauri 配置预创建（改为空闲销毁 + 按需重建），
/// 所有选项必须在此用 builder 完整复刻原 `tauri.conf.json` 声明，否则重建后行为漂移。
///
/// 建窗后保持 `visible: false`：定位与显示由预览 show 流程统一处理；
/// macOS 的 NSPanel 转换由 [`ensure_preview_window`] 在每次取窗时兜底执行。
pub fn build_clipboard_preview_window(app: &AppHandle) -> Result<()> {
    let _guard = PREVIEW_BUILD_LOCK.lock().unwrap_or_else(|poisoned| {
        log::error!("preview build mutex poisoned, recovering");
        poisoned.into_inner()
    });

    if app
        .get_webview_window(CLIPBOARD_PREVIEW_WINDOW_LABEL)
        .is_some()
    {
        return Ok(());
    }

    WebviewWindowBuilder::new(
        app,
        CLIPBOARD_PREVIEW_WINDOW_LABEL,
        WebviewUrl::App("index.html/#/preview".into()),
    )
    .title("EcoPaste Preview")
    .inner_size(1.0, 1.0)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .always_on_top(true)
    .decorations(false)
    .shadow(false)
    .transparent(true)
    .skip_taskbar(true)
    .focused(false)
    .focusable(false)
    // macOS：面板转为可交互后，第一次点击要直达内容（滚动 / 拖选），
    // 否则会被当成「激活点击」吃掉。
    .accept_first_mouse(true)
    .disable_drag_drop_handler()
    .visible(false)
    .build()
    .map_err(|err| anyhow::anyhow!("build clipboard preview window: {err}"))?;

    Ok(())
}

fn set_preview_state(state: Option<ClipboardPreviewState>) {
    let mut guard = PREVIEW_STATE.lock().unwrap_or_else(|poisoned| {
        log::error!("preview state mutex poisoned on set, recovering");
        poisoned.into_inner()
    });
    *guard = state;
}

/// 判断剪贴板窗口是否仍处于可见状态，防止过期 hover 请求在剪贴板窗口隐藏后唤起预览。
fn is_clipboard_window_visible(app: &AppHandle) -> bool {
    app.get_webview_window(CLIPBOARD_WINDOW_LABEL)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false)
}

/// 延迟隐藏真实窗口，为前端退出动画留出一小段可见时间。
fn schedule_preview_window_hide(app: AppHandle, window: WebviewWindow, request_id: u64) {
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(PREVIEW_HIDE_DELAY_MS));

        if PREVIEW_REQUEST_ID.load(Ordering::SeqCst) != request_id {
            return;
        }
        if get_clipboard_preview_state().ok().flatten().is_some() {
            return;
        }

        if let Err(error) = hide_preview_window(&app, &window) {
            log::error!("hide preview window after exit animation failed: {error}");
        }
    });
}

/// 返回本次 show 所属的可见会话 id；隐藏后再次 show 会开启新会话。
fn preview_session_id_for_show() -> u64 {
    let guard = PREVIEW_STATE.lock().unwrap_or_else(|poisoned| {
        log::error!("preview state mutex poisoned on session, recovering");
        poisoned.into_inner()
    });

    if guard.is_some() {
        return PREVIEW_SESSION_ID.load(Ordering::SeqCst);
    }

    PREVIEW_SESSION_ID.fetch_add(1, Ordering::SeqCst) + 1
}

fn validate_anchor(anchor: &PreviewAnchorRect) -> Result<()> {
    let values = [anchor.left, anchor.top, anchor.width, anchor.height];
    if !values.iter().all(|value| value.is_finite()) || anchor.width <= 0.0 || anchor.height <= 0.0
    {
        return Err(anyhow::anyhow!("preview anchor is invalid").into());
    }
    if let Some(pointer_y) = anchor.pointer_y {
        if !pointer_y.is_finite() {
            return Err(anyhow::anyhow!("preview pointer is invalid").into());
        }
    }

    Ok(())
}

/// 取预览窗口；已被空闲销毁（或尚未创建）时按需重建。
/// macOS 下每次都兜底确保 NSPanel 转换完成，覆盖重建后的全新窗口。
fn ensure_preview_window(app: &AppHandle) -> Result<WebviewWindow> {
    if app
        .get_webview_window(CLIPBOARD_PREVIEW_WINDOW_LABEL)
        .is_none()
    {
        build_clipboard_preview_window(app)?;
    }

    let window = get_window(app, CLIPBOARD_PREVIEW_WINDOW_LABEL)?;

    #[cfg(target_os = "macos")]
    ensure_macos_preview_panel(app, &window)?;

    Ok(window)
}

fn raise_preview_window(app: &AppHandle, window: &WebviewWindow) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let _ = window;
        set_macos_preview_panel_level(app)
    }

    #[cfg(target_os = "windows")]
    {
        let _ = app;
        window
            .set_always_on_top(true)
            .map_err(|e| anyhow::anyhow!(e))?;
        raise_windows_preview_window(window, false)?;

        Ok(())
    }
}

fn prepare_preview_window_for_show(
    app: &AppHandle,
    window: &WebviewWindow,
    work_area: &PhysicalRect<i32, u32>,
) -> Result<()> {
    apply_preview_window_bounds(window, work_area)?;
    sync_pointer_for_show(app, window)?;
    raise_preview_window(app, window)
}

/// show 前的穿透开关同步：
/// - 新一次 show（此前不可见）：锚点来自指针所在的列表卡片，先按穿透呈现，
///   采样线程会在下一个周期把真实指针状态纠正回来；
/// - retarget（此前可见）：按当前指针位置重新判定并广播——写死穿透会把正在
///   面板上拖选的用户瞬间打回穿透，选中与滚动都会被打断。
fn sync_pointer_for_show(app: &AppHandle, window: &WebviewWindow) -> Result<()> {
    let retarget = PREVIEW_VISIBLE.load(Ordering::Relaxed);

    if !retarget {
        // 前端上报的实测矩形属于上一次会话，先清掉，回退到本次 layout 的面板框。
        clear_reported_panel_rect();
        PREVIEW_POINTER_INSIDE.store(false, Ordering::SeqCst);
    }

    refresh_hit_bounds();
    log::info!(
        "preview show sync: retarget={retarget}, hit_bounds={:?}",
        read_hit_bounds()
    );

    if retarget {
        // 面板换了位置：完整校对一次，指针被留在外面时立即交还穿透。
        reconcile_pointer(app, false);
        return Ok(());
    }

    window
        .set_ignore_cursor_events(true)
        .map_err(|e| anyhow::anyhow!(e))?;

    Ok(())
}

fn clear_reported_panel_rect() {
    let mut guard = PREVIEW_PANEL_RECT.lock().unwrap_or_else(|poisoned| {
        log::error!("preview panel rect mutex poisoned on clear, recovering");
        poisoned.into_inner()
    });
    *guard = None;
}

/// 平台 show 收口点；成功后推进生命周期到 `Visible`，使未触发的空闲销毁计时器过期。
fn show_preview_window(
    app: &AppHandle,
    window: &WebviewWindow,
    work_area: &PhysicalRect<i32, u32>,
) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let _ = window;
        show_macos_preview_panel(app, work_area)?;
    }

    #[cfg(target_os = "windows")]
    {
        let _ = work_area;
        window.show().map_err(|e| anyhow::anyhow!(e))?;
        raise_windows_preview_window(window, true)?;
    }

    PREVIEW_VISIBLE.store(true, Ordering::Relaxed);
    ensure_pointer_watcher(app);
    lifecycle::on_shown(app, CLIPBOARD_PREVIEW_WINDOW_LABEL);

    Ok(())
}

/// 平台 hide 收口点；成功后推进生命周期到 `HiddenWarm`，启动空闲销毁计时。
/// 对已隐藏窗口的重复 hide（如剪贴板窗口隐藏时的压制路径）也会走到这里，
/// 由生命周期管理器对重复进入 `HiddenWarm` 去重计时。
fn hide_preview_window(app: &AppHandle, window: &WebviewWindow) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let _ = window;
        hide_macos_preview_panel(app)?;
    }

    #[cfg(target_os = "windows")]
    window.hide().map_err(|e| anyhow::anyhow!(e))?;

    reset_pointer_tracking();
    lifecycle::on_hidden(app, CLIPBOARD_PREVIEW_WINDOW_LABEL, "preview-hide");

    Ok(())
}

#[cfg(target_os = "macos")]
fn ensure_macos_preview_panel(app: &AppHandle, window: &WebviewWindow) -> Result<()> {
    let handle = app.clone();
    let preview_window = window.clone();
    let (tx, rx) = std::sync::mpsc::channel();

    app.run_on_main_thread(move || {
        let result = setup_macos_preview_panel(&handle, &preview_window);
        let _ = tx.send(result);
    })
    .map_err(|e| anyhow::anyhow!(e))?;

    rx.recv()
        .map_err(|e| anyhow::anyhow!("preview panel setup channel closed: {e}"))?
}

#[cfg(target_os = "macos")]
fn setup_macos_preview_panel(app: &AppHandle, window: &WebviewWindow) -> Result<()> {
    let panel = match app.get_webview_panel(CLIPBOARD_PREVIEW_WINDOW_LABEL) {
        Ok(panel) => panel,
        Err(_) => window
            .to_panel::<PreviewPanel>()
            .map_err(|e| anyhow::anyhow!("to_panel failed: {e:?}"))?,
    };

    panel.set_level(PanelLevel::Status.value());
    // 面板要接收鼠标事件才能滚动与拖选，但必须保持「不激活 App、不抢 key」：
    // 否则点击面板会让主 panel resign key，连带把预览收起。
    panel.set_style_mask(StyleMask::empty().nonactivating_panel().into());

    Ok(())
}

#[cfg(target_os = "macos")]
fn set_macos_preview_panel_level(app: &AppHandle) -> Result<()> {
    let handle = app.clone();

    app.run_on_main_thread(move || {
        if let Ok(panel) = handle.get_webview_panel(CLIPBOARD_PREVIEW_WINDOW_LABEL) {
            panel.set_level(PanelLevel::Status.value());
        }
    })
    .map_err(|e| anyhow::anyhow!(e))?;

    Ok(())
}

#[cfg(target_os = "macos")]
fn show_macos_preview_panel(app: &AppHandle, work_area: &PhysicalRect<i32, u32>) -> Result<()> {
    let handle = app.clone();
    let preview_window = get_window(app, CLIPBOARD_PREVIEW_WINDOW_LABEL)?;
    let work_area = *work_area;
    let (tx, rx) = std::sync::mpsc::channel();

    app.run_on_main_thread(move || {
        let result = (|| -> Result<()> {
            apply_preview_window_bounds(&preview_window, &work_area)?;
            let panel = handle
                .get_webview_panel(CLIPBOARD_PREVIEW_WINDOW_LABEL)
                .map_err(|e| anyhow::anyhow!("preview panel not found: {e:?}"))?;
            panel.set_level(PanelLevel::Status.value());
            panel.show();
            Ok(())
        })();
        let _ = tx.send(result);
    })
    .map_err(|e| anyhow::anyhow!(e))?;

    rx.recv()
        .map_err(|e| anyhow::anyhow!("preview panel show channel closed: {e}"))?
}

#[cfg(target_os = "macos")]
fn hide_macos_preview_panel(app: &AppHandle) -> Result<()> {
    let handle = app.clone();

    app.run_on_main_thread(move || {
        if let Ok(panel) = handle.get_webview_panel(CLIPBOARD_PREVIEW_WINDOW_LABEL) {
            panel.hide();
        }
    })
    .map_err(|e| anyhow::anyhow!(e))?;

    Ok(())
}

/// 将预览窗口重新压到 Windows topmost 栈顶，避免被同为 always-on-top 的剪贴板窗口盖住。
#[cfg(target_os = "windows")]
fn raise_windows_preview_window(window: &WebviewWindow, show: bool) -> Result<()> {
    let raw_hwnd = window.hwnd().map_err(|e| anyhow::anyhow!(e))?;
    let hwnd = HWND(raw_hwnd.0 as isize);
    let mut flags = SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE;

    if show {
        flags |= SWP_SHOWWINDOW;
    }

    unsafe {
        SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, flags).map_err(|e| anyhow::anyhow!(e))?;
    }

    Ok(())
}

fn resolve_preview_monitor(app: &AppHandle) -> Result<tauri::Monitor> {
    let clipboard_window = get_window(app, CLIPBOARD_WINDOW_LABEL)?;
    if let Some(monitor) = clipboard_window
        .current_monitor()
        .map_err(|e| anyhow::anyhow!(e))?
    {
        return Ok(monitor);
    }

    clipboard_window
        .primary_monitor()
        .map_err(|e| anyhow::anyhow!(e))?
        .ok_or_else(|| anyhow::anyhow!("primary monitor not found").into())
}

/// 用完整显示器区域作为预览 overlay 边界，避免 macOS Dock 压缩 `work_area` 后截断连线。
fn preview_overlay_bounds(monitor: &tauri::Monitor) -> PhysicalRect<i32, u32> {
    PhysicalRect {
        position: *monitor.position(),
        size: *monitor.size(),
    }
}

fn apply_preview_window_bounds(
    window: &WebviewWindow,
    work_area: &PhysicalRect<i32, u32>,
) -> Result<()> {
    let size = preview_window_size(work_area);

    window
        .set_position(PhysicalPosition::new(
            work_area.position.x,
            work_area.position.y,
        ))
        .map_err(|e| anyhow::anyhow!(e))?;
    window.set_size(size).map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}

/// 返回实际预览窗口尺寸；Windows 避免精确全屏触发系统勿扰模式。
fn preview_window_size(work_area: &PhysicalRect<i32, u32>) -> PhysicalSize<u32> {
    #[cfg(target_os = "windows")]
    {
        PhysicalSize::new(
            work_area.size.width.saturating_sub(1),
            work_area.size.height,
        )
    }

    #[cfg(target_os = "macos")]
    {
        PhysicalSize::new(work_area.size.width, work_area.size.height)
    }
}

fn build_preview_layout(
    anchor: &PreviewAnchorRect,
    scale_factor: f64,
    work_area: &PhysicalRect<i32, u32>,
    clipboard_window: Option<&PreviewClipboardWindowRect>,
) -> PreviewLayout {
    let overlay_rect = PreviewRect {
        left: 0.0,
        top: 0.0,
        width: work_area.size.width as f64 / scale_factor,
        height: work_area.size.height as f64 / scale_factor,
    };
    let source_rect = resolve_source_rect(
        anchor,
        scale_factor,
        work_area,
        clipboard_window,
        overlay_rect,
    );
    let (panel_rect, placement) = resolve_panel_rect(source_rect, overlay_rect);

    PreviewLayout {
        overlay_rect,
        source_rect,
        panel_rect,
        placement,
    }
}

fn resolve_source_rect(
    anchor: &PreviewAnchorRect,
    scale_factor: f64,
    work_area: &PhysicalRect<i32, u32>,
    clipboard_window: Option<&PreviewClipboardWindowRect>,
    overlay_rect: PreviewRect,
) -> PreviewRect {
    let source = if let Some(clipboard_window) = clipboard_window {
        let main_rect = PreviewRect {
            left: (clipboard_window.x - work_area.position.x) as f64 / scale_factor,
            top: (clipboard_window.y - work_area.position.y) as f64 / scale_factor,
            width: clipboard_window.width as f64 / scale_factor,
            height: clipboard_window.height as f64 / scale_factor,
        };
        let source = PreviewRect {
            left: main_rect.left + anchor.left,
            top: main_rect.top + anchor.top,
            width: anchor.width,
            height: anchor.height,
        };

        let pointer_y = anchor.pointer_y.map(|y| main_rect.top + y);

        resolve_pointer_anchor_rect(source, pointer_y)
    } else {
        let source = PreviewRect {
            left: anchor.left,
            top: anchor.top,
            width: anchor.width,
            height: anchor.height,
        };

        resolve_pointer_anchor_rect(source, anchor.pointer_y)
    };

    intersect_rect(source, overlay_rect).unwrap_or_else(|| clamp_rect(source, overlay_rect))
}

fn resolve_pointer_anchor_rect(source: PreviewRect, pointer_y: Option<f64>) -> PreviewRect {
    let Some(pointer_y) = pointer_y else {
        return source;
    };
    let center_y = pointer_y.clamp(source.top, source.bottom());
    let top = center_y - PREVIEW_POINTER_ANCHOR_SIZE / 2.0;

    PreviewRect {
        left: source.left,
        top,
        width: source.width,
        height: PREVIEW_POINTER_ANCHOR_SIZE,
    }
}

fn resolve_panel_rect(
    source_rect: PreviewRect,
    overlay_rect: PreviewRect,
) -> (PreviewRect, PreviewPlacement) {
    let panel_size = (PREVIEW_PANEL_WIDTH, PREVIEW_PANEL_HEIGHT);
    let candidates = [
        (
            PreviewPlacement::Right,
            source_rect.right() + PREVIEW_PANEL_GAP + PREVIEW_PANEL_WIDTH + PREVIEW_PANEL_MARGIN
                <= overlay_rect.right(),
        ),
        (
            PreviewPlacement::Left,
            source_rect.left - PREVIEW_PANEL_GAP - PREVIEW_PANEL_WIDTH - PREVIEW_PANEL_MARGIN
                >= overlay_rect.left,
        ),
        (
            PreviewPlacement::Bottom,
            source_rect.bottom() + PREVIEW_PANEL_GAP + PREVIEW_PANEL_HEIGHT + PREVIEW_PANEL_MARGIN
                <= overlay_rect.bottom(),
        ),
        (
            PreviewPlacement::Top,
            source_rect.top - PREVIEW_PANEL_GAP - PREVIEW_PANEL_HEIGHT - PREVIEW_PANEL_MARGIN
                >= overlay_rect.top,
        ),
    ];

    let placement = candidates
        .iter()
        .find_map(|(placement, fits)| fits.then_some(*placement))
        .unwrap_or(PreviewPlacement::Right);
    let raw = raw_panel_rect(source_rect, placement, panel_size);

    (
        clamp_rect(raw, inset_rect(overlay_rect, PREVIEW_PANEL_MARGIN)),
        placement,
    )
}

fn raw_panel_rect(
    source_rect: PreviewRect,
    placement: PreviewPlacement,
    (width, height): (f64, f64),
) -> PreviewRect {
    let centered_top = source_rect.center_y() - height / 2.0;
    let centered_left = source_rect.center_x() - width / 2.0;

    match placement {
        PreviewPlacement::Right => PreviewRect {
            left: source_rect.right() + PREVIEW_PANEL_GAP,
            top: centered_top,
            width,
            height,
        },
        PreviewPlacement::Left => PreviewRect {
            left: source_rect.left - PREVIEW_PANEL_GAP - width,
            top: centered_top,
            width,
            height,
        },
        PreviewPlacement::Bottom => PreviewRect {
            left: centered_left,
            top: source_rect.bottom() + PREVIEW_PANEL_GAP,
            width,
            height,
        },
        PreviewPlacement::Top => PreviewRect {
            left: centered_left,
            top: source_rect.top - PREVIEW_PANEL_GAP - height,
            width,
            height,
        },
    }
}

fn clamp_rect(rect: PreviewRect, bounds: PreviewRect) -> PreviewRect {
    let max_left = (bounds.right() - rect.width).max(bounds.left);
    let max_top = (bounds.bottom() - rect.height).max(bounds.top);

    PreviewRect {
        left: rect.left.clamp(bounds.left, max_left),
        top: rect.top.clamp(bounds.top, max_top),
        width: rect.width,
        height: rect.height,
    }
}

fn intersect_rect(a: PreviewRect, b: PreviewRect) -> Option<PreviewRect> {
    let left = a.left.max(b.left);
    let top = a.top.max(b.top);
    let right = a.right().min(b.right());
    let bottom = a.bottom().min(b.bottom());

    if right <= left || bottom <= top {
        return None;
    }

    Some(PreviewRect {
        left,
        top,
        width: right - left,
        height: bottom - top,
    })
}

fn inset_rect(rect: PreviewRect, amount: f64) -> PreviewRect {
    PreviewRect {
        left: rect.left + amount,
        top: rect.top + amount,
        width: (rect.width - amount * 2.0).max(1.0),
        height: (rect.height - amount * 2.0).max(1.0),
    }
}

/// 返回剪贴板窗口内容区的屏幕几何，用于映射 WebView DOM rect 到预览 overlay 坐标。
fn clipboard_window_rect(app: &AppHandle) -> Option<PreviewClipboardWindowRect> {
    let window = app.get_webview_window(CLIPBOARD_WINDOW_LABEL)?;
    let pos = window.inner_position().ok()?;
    let size = window.inner_size().ok()?;

    Some(PreviewClipboardWindowRect {
        x: pos.x,
        y: pos.y,
        width: size.width,
        height: size.height,
    })
}

impl PreviewRect {
    fn right(self) -> f64 {
        self.left + self.width
    }

    fn bottom(self) -> f64 {
        self.top + self.height
    }

    fn center_x(self) -> f64 {
        self.left + self.width / 2.0
    }

    fn center_y(self) -> f64 {
        self.top + self.height / 2.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlay() -> PreviewRect {
        PreviewRect {
            left: 0.0,
            top: 0.0,
            width: 1200.0,
            height: 800.0,
        }
    }

    #[test]
    fn places_panel_on_right_when_space_exists() {
        let source = PreviewRect {
            left: 240.0,
            top: 200.0,
            width: 120.0,
            height: 40.0,
        };
        let (panel, placement) = resolve_panel_rect(source, overlay());

        assert!(matches!(placement, PreviewPlacement::Right));
        assert!(panel.left > source.right());
    }

    #[test]
    fn places_panel_on_left_when_right_side_is_tight() {
        let source = PreviewRect {
            left: 980.0,
            top: 200.0,
            width: 120.0,
            height: 40.0,
        };
        let (panel, placement) = resolve_panel_rect(source, overlay());

        assert!(matches!(placement, PreviewPlacement::Left));
        assert!(panel.right() < source.left);
    }

    #[test]
    fn clamps_panel_inside_overlay_margin() {
        let source = PreviewRect {
            left: 580.0,
            top: 740.0,
            width: 80.0,
            height: 40.0,
        };
        let (panel, _) = resolve_panel_rect(source, overlay());

        assert!(panel.left >= PREVIEW_PANEL_MARGIN);
        assert!(panel.top >= PREVIEW_PANEL_MARGIN);
        assert!(panel.right() <= overlay().right() - PREVIEW_PANEL_MARGIN);
        assert!(panel.bottom() <= overlay().bottom() - PREVIEW_PANEL_MARGIN);
    }

    #[test]
    fn maps_anchor_from_clipboard_window_to_overlay_local_rect() {
        let work_area = PhysicalRect {
            position: PhysicalPosition::new(100, 50),
            size: PhysicalSize::new(2400, 1600),
        };
        let clipboard = PreviewClipboardWindowRect {
            x: 300,
            y: 250,
            width: 800,
            height: 600,
        };
        let anchor = PreviewAnchorRect {
            left: 20.0,
            pointer_y: None,
            top: 30.0,
            width: 100.0,
            height: 40.0,
        };
        let source = resolve_source_rect(
            &anchor,
            2.0,
            &work_area,
            Some(&clipboard),
            PreviewRect {
                left: 0.0,
                top: 0.0,
                width: 1200.0,
                height: 800.0,
            },
        );

        assert_eq!(source.left, 120.0);
        assert_eq!(source.top, 130.0);
        assert_eq!(source.width, 100.0);
        assert_eq!(source.height, 40.0);
    }

    #[test]
    fn keeps_source_rect_on_clipboard_card_when_card_touches_main_edge() {
        let work_area = PhysicalRect {
            position: PhysicalPosition::new(100, 50),
            size: PhysicalSize::new(2400, 1600),
        };
        let clipboard = PreviewClipboardWindowRect {
            x: 300,
            y: 250,
            width: 800,
            height: 600,
        };
        let anchor = PreviewAnchorRect {
            left: -8.0,
            pointer_y: None,
            top: 40.0,
            width: 120.0,
            height: 80.0,
        };
        let source = resolve_source_rect(
            &anchor,
            2.0,
            &work_area,
            Some(&clipboard),
            PreviewRect {
                left: 0.0,
                top: 0.0,
                width: 1200.0,
                height: 800.0,
            },
        );

        assert_eq!(source.left, 92.0);
        assert_eq!(source.width, 120.0);
    }

    #[test]
    fn follows_pointer_y_inside_clipboard_card() {
        let work_area = PhysicalRect {
            position: PhysicalPosition::new(100, 50),
            size: PhysicalSize::new(2400, 1600),
        };
        let clipboard = PreviewClipboardWindowRect {
            x: 300,
            y: 250,
            width: 800,
            height: 600,
        };
        let anchor = PreviewAnchorRect {
            left: 20.0,
            pointer_y: Some(92.0),
            top: 40.0,
            width: 120.0,
            height: 100.0,
        };
        let source = resolve_source_rect(
            &anchor,
            2.0,
            &work_area,
            Some(&clipboard),
            PreviewRect {
                left: 0.0,
                top: 0.0,
                width: 1200.0,
                height: 800.0,
            },
        );

        assert_eq!(source.left, 120.0);
        assert_eq!(source.top, 191.5);
        assert_eq!(source.width, 120.0);
        assert_eq!(source.height, PREVIEW_POINTER_ANCHOR_SIZE);
    }

    #[test]
    fn physical_bounds_maps_local_logical_rect_to_screen_pixels() {
        let bounds = physical_bounds(
            PreviewRect {
                left: 10.0,
                top: 20.0,
                width: 100.0,
                height: 50.0,
            },
            2.0,
            300,
            150,
        );

        // 逻辑 (10,20)-(110,70) → 物理 (320,190)-(520,290)，再外扩 4 逻辑 px（=8 物理 px）。
        assert_eq!(bounds.left, 312);
        assert_eq!(bounds.top, 182);
        assert_eq!(bounds.right, 528);
        assert_eq!(bounds.bottom, 298);
    }

    #[test]
    fn physical_bounds_handles_fractional_scale_and_negative_origin() {
        let bounds = physical_bounds(
            PreviewRect {
                left: 4.0,
                top: 4.0,
                width: 40.0,
                height: 40.0,
            },
            1.25,
            -100,
            -50,
        );

        assert_eq!(bounds.left, -100);
        assert_eq!(bounds.top, -50);
        assert_eq!(bounds.right, -40);
        assert_eq!(bounds.bottom, 10);
    }

    #[test]
    fn hit_bounds_use_half_open_range() {
        let bounds = physical_bounds(
            PreviewRect {
                left: 0.0,
                top: 0.0,
                width: 10.0,
                height: 10.0,
            },
            1.0,
            0,
            0,
        );

        // 外扩后逻辑 (-4,-4)-(14,14)：左/上闭、右/下开。
        assert!(bounds.contains(-4, -4));
        assert!(bounds.contains(13, 13));
        assert!(!bounds.contains(-5, 0));
        assert!(!bounds.contains(14, 8));
        assert!(!bounds.contains(8, 14));
    }

    #[test]
    fn zero_scale_factor_falls_back_to_one() {
        let bounds = physical_bounds(
            PreviewRect {
                left: 0.0,
                top: 0.0,
                width: 10.0,
                height: 10.0,
            },
            0.0,
            0,
            0,
        );

        assert_eq!(bounds.left, -4);
        assert_eq!(bounds.right, 14);
    }

    #[test]
    fn rejects_non_finite_or_degenerate_rects() {
        assert!(is_valid_rect(PreviewRect {
            left: 0.0,
            top: 0.0,
            width: 10.0,
            height: 10.0,
        }));
        assert!(!is_valid_rect(PreviewRect {
            left: f64::NAN,
            top: 0.0,
            width: 10.0,
            height: 10.0,
        }));
        assert!(!is_valid_rect(PreviewRect {
            left: 0.0,
            top: 0.0,
            width: 0.0,
            height: 10.0,
        }));
    }

    /// Quartz 的全局显示坐标是点，需按 overlay 显示器原点与缩放换算成物理像素；
    /// 该换算只在 macOS 参与编译，Windows 上无法验证真机行为，先把纯数学锁住。
    #[cfg(target_os = "macos")]
    #[test]
    fn screen_points_to_physical_scales_relative_to_overlay_origin() {
        // 主屏（原点 0,0）scale 2。
        assert_eq!(
            screen_points_to_physical(300.0, 200.0, 0, 0, 2.0),
            (600, 400)
        );

        // overlay 在主屏右侧（物理 2880 = 1440pt，自身 scale 2）：点坐标先减 1440pt 再乘缩放。
        assert_eq!(
            screen_points_to_physical(1500.0, 100.0, 2880, 0, 2.0),
            (3000, 200)
        );

        // 副屏在主屏上方（物理 y 为负，原点 -900 物理 = -600pt）。
        assert_eq!(
            screen_points_to_physical(500.0, 100.0, 0, -900, 1.5),
            (750, 150)
        );
    }
}

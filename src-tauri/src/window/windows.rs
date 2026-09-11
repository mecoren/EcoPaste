//! Windows 窗口管理：剪贴板窗口默认不可聚焦，输入控件编辑期间临时恢复可聚焦。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, IsWindow, SetForegroundWindow};

use super::{get_window, CLIPBOARD_WINDOW_LABEL};
use crate::core::Result;
use crate::{keyboard, mouse};

static PRE_EDIT_FOREGROUND_HWND: Mutex<Option<isize>> = Mutex::new(None);
/// editing 焦点观察线程单飞位：重复进入 editing 不叠加线程。
static EDITING_FOCUS_WATCHER_RUNNING: AtomicBool = AtomicBool::new(false);

/// 等剪贴板窗口真正成为前台的最长时间；超时视为 SetFocus 被系统拒绝。
const EDITING_FOCUS_TIMEOUT: Duration = Duration::from_millis(300);
const EDITING_FOCUS_POLL_INTERVAL: Duration = Duration::from_millis(10);

pub fn show_window(app_handle: &AppHandle, label: &str) -> Result<()> {
    let window = get_window(app_handle, label)?;
    if label == CLIPBOARD_WINDOW_LABEL {
        window
            .set_focusable(false)
            .map_err(|e| anyhow::anyhow!(e))?;
        clear_pre_edit_foreground();
    }

    window.show().map_err(|e| anyhow::anyhow!(e))?;
    window.unminimize().map_err(|e| anyhow::anyhow!(e))?;

    if label == CLIPBOARD_WINDOW_LABEL {
        keyboard::enable_navigation_keys(app_handle);
        mouse::enable_outside_click_hide(app_handle);
    } else {
        window.set_focus().map_err(|e| anyhow::anyhow!(e))?;
    }

    Ok(())
}

pub fn set_clipboard_window_editing(app_handle: &AppHandle, editing: bool) -> Result<()> {
    let window = get_window(app_handle, CLIPBOARD_WINDOW_LABEL)?;
    let raw_hwnd = window.hwnd().map_err(|e| anyhow::anyhow!(e))?;
    let hwnd = HWND(raw_hwnd.0 as isize);

    if editing {
        remember_pre_edit_foreground(hwnd);
        window.set_focusable(true).map_err(|e| anyhow::anyhow!(e))?;
        window.set_focus().map_err(|e| anyhow::anyhow!(e))?;

        // set_focus 是异步派发：等窗口真正成为前台后才禁导航钩子。
        // 提前禁钩子会让「开始打字到焦点完成」之间连打的字符穿过钩子落进用户前台应用。
        // 空窗期内钩子继续吞可打印字符入队（typeahead 回放），与打字即搜索机制衔接。
        spawn_editing_focus_watcher(app_handle);

        return Ok(());
    }

    let should_restore_foreground = unsafe { GetForegroundWindow() == hwnd };
    window
        .set_focusable(false)
        .map_err(|e| anyhow::anyhow!(e))?;

    if window.is_visible().unwrap_or(false) {
        keyboard::enable_navigation_keys(app_handle);
        mouse::enable_outside_click_hide(app_handle);
    }

    if should_restore_foreground {
        restore_pre_edit_foreground(hwnd);
    } else {
        clear_pre_edit_foreground();
    }

    Ok(())
}

/// 轮询剪贴板窗口前台状态：焦点到位 → 禁导航钩子；超时 → 回滚可聚焦并告警。
fn spawn_editing_focus_watcher(app_handle: &AppHandle) {
    if EDITING_FOCUS_WATCHER_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }

    let app = app_handle.clone();

    std::thread::spawn(move || {
        let Some(window) = app.get_webview_window(CLIPBOARD_WINDOW_LABEL) else {
            EDITING_FOCUS_WATCHER_RUNNING.store(false, Ordering::SeqCst);
            return;
        };

        let Ok(raw_hwnd) = window.hwnd() else {
            EDITING_FOCUS_WATCHER_RUNNING.store(false, Ordering::SeqCst);
            return;
        };

        let hwnd = HWND(raw_hwnd.0 as isize);
        let deadline = std::time::Instant::now() + EDITING_FOCUS_TIMEOUT;

        loop {
            let foreground_matches = unsafe { GetForegroundWindow() == hwnd };

            if foreground_matches {
                keyboard::disable_navigation_keys();
                EDITING_FOCUS_WATCHER_RUNNING.store(false, Ordering::SeqCst);
                return;
            }

            if std::time::Instant::now() >= deadline {
                log::warn!("clipboard window editing focus not acquired, rolling back");
                let _ = window.set_focusable(false);
                EDITING_FOCUS_WATCHER_RUNNING.store(false, Ordering::SeqCst);
                return;
            }

            std::thread::sleep(EDITING_FOCUS_POLL_INTERVAL);
        }
    });
}

pub fn hide_window(app_handle: &AppHandle, label: &str) -> Result<()> {
    let window = get_window(app_handle, label)?;
    window.hide().map_err(|e| anyhow::anyhow!(e))?;
    if label == CLIPBOARD_WINDOW_LABEL {
        if let Err(err) = window.set_focusable(false) {
            log::warn!("reset clipboard window focusable on hide failed: {err:?}");
        }
        clear_pre_edit_foreground();
        keyboard::disable_navigation_keys();
        mouse::disable_outside_click_hide();
        crate::menu::context_window::hide(app_handle);
    }

    Ok(())
}

fn remember_pre_edit_foreground(clipboard_hwnd: HWND) {
    let mut guard = PRE_EDIT_FOREGROUND_HWND
        .lock()
        .expect("pre edit foreground hwnd poisoned");
    if guard.is_some() {
        return;
    }

    let foreground = unsafe { GetForegroundWindow() };
    if foreground.0 == 0 || foreground == clipboard_hwnd {
        return;
    }

    *guard = Some(foreground.0);
}

fn restore_pre_edit_foreground(clipboard_hwnd: HWND) {
    let previous = PRE_EDIT_FOREGROUND_HWND
        .lock()
        .expect("pre edit foreground hwnd poisoned")
        .take();
    let Some(previous) = previous else {
        return;
    };

    let previous_hwnd = HWND(previous);
    if previous_hwnd == clipboard_hwnd || !unsafe { IsWindow(previous_hwnd).as_bool() } {
        return;
    }

    if !unsafe { SetForegroundWindow(previous_hwnd).as_bool() } {
        log::debug!("restore pre-edit foreground window was rejected by Windows");
    }
}

fn clear_pre_edit_foreground() {
    PRE_EDIT_FOREGROUND_HWND
        .lock()
        .expect("pre edit foreground hwnd poisoned")
        .take();
}

pub fn show_taskbar_icon(app_handle: &AppHandle, visible: bool) -> Result<()> {
    let window = get_window(app_handle, CLIPBOARD_WINDOW_LABEL)?;
    window
        .set_skip_taskbar(!visible)
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}

use std::collections::{HashSet, VecDeque};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};
use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM};
use winapi::um::processthreadsapi::GetCurrentThreadId;
use winapi::um::winuser::{
    CallNextHookEx, GetAsyncKeyState, GetForegroundWindow, GetMessageW, PostThreadMessageW,
    SetWindowsHookExW, UnhookWindowsHookEx, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, VK_BACK,
    VK_CONTROL, VK_DELETE, VK_DOWN, VK_ESCAPE, VK_LCONTROL, VK_LEFT, VK_MENU, VK_RCONTROL,
    VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP,
    WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use super::{NAV_EVENT, SEARCH_TYPING_EVENT};

static NAV_ENABLED: AtomicBool = AtomicBool::new(false);
static HOOK_THREAD_ID: Mutex<Option<u32>> = Mutex::new(None);
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

/// 前端已聚焦搜索框的回执；回放线程等到它才注入按键。
static TYPEAHEAD_ACK: AtomicBool = AtomicBool::new(false);
/// 回放工作线程单飞位：避免连打时每键起一个线程。
static TYPEAHEAD_WORKER_RUNNING: AtomicBool = AtomicBool::new(false);
/// 等待回放的被吞按键（VK + Shift 按下状态），保持用户按键顺序。
static TYPEAHEAD_QUEUE: Mutex<VecDeque<(u32, bool)>> = Mutex::new(VecDeque::new());

/// 等前端 ack 的最长时间；超时视为焦点未达成，丢弃队列不注入。
const TYPEAHEAD_ACK_TIMEOUT: Duration = Duration::from_millis(150);
/// 等剪贴板窗口成为前台的最长时间（set_focus 异步派发）。
const TYPEAHEAD_FOREGROUND_TIMEOUT: Duration = Duration::from_millis(250);
const TYPEAHEAD_POLL_INTERVAL: Duration = Duration::from_millis(5);

fn consumed_keys() -> &'static Mutex<HashSet<u32>> {
    static SET: OnceLock<Mutex<HashSet<u32>>> = OnceLock::new();
    SET.get_or_init(|| Mutex::new(HashSet::new()))
}

/// 仅放行当前前端需要的 Ctrl 快捷键：C、D、F、K、M、N、O、P、Q、T、Enter、Backspace、Delete、逗号与数字 0-9。
fn ctrl_shortcut_key(vk: u32) -> Option<String> {
    match vk as i32 {
        0x43 => Some("c".to_string()),
        0x44 => Some("d".to_string()),
        0x46 => Some("f".to_string()),
        0x4B => Some("k".to_string()),
        0x4D => Some("m".to_string()),
        0x4E => Some("n".to_string()),
        0x4F => Some("o".to_string()),
        0x50 => Some("p".to_string()),
        0x51 => Some("q".to_string()),
        0x54 => Some("t".to_string()),
        0xBC => Some(",".to_string()),
        0x30..=0x39 => Some(((vk as u8) as char).to_string()),
        VK_RETURN => Some("Enter".to_string()),
        VK_BACK => Some("Backspace".to_string()),
        VK_DELETE => Some("Delete".to_string()),
        _ => None,
    }
}

/// 可直接落入搜索框的字符键（数字 / 字母 / OEM 符号）与 Backspace（删搜索词）。
/// 不含 VK_SPACE（预览键）与修饰键；组合键（Ctrl/Alt 按下）由调用方排除。
/// Backspace 与字母一视同仁：窗口可见但未聚焦时同样入队回放到搜索框，
/// 搜索词为空时删除是无害空操作，语义与「打字即搜索」保持一致。
fn typeahead_key(vk: u32) -> bool {
    matches!(vk as i32,
        VK_BACK         // 删搜索词
        | 0x30..=0x39   // 数字主行
        | 0x41..=0x5A   // 字母
        | 0xBA..=0xBF   // ;=,-./`
        | 0xC0          // `
        | 0xDB..=0xDF   // [\]'
        | 0xE2,         // OEM 102（部分欧洲布局）
    )
}

fn nav_key(vk: u32) -> Option<&'static str> {
    match vk as i32 {
        VK_LEFT => Some("ArrowLeft"),
        VK_RIGHT => Some("ArrowRight"),
        VK_UP => Some("ArrowUp"),
        VK_DOWN => Some("ArrowDown"),
        VK_RETURN => Some("Enter"),
        VK_ESCAPE => Some("Escape"),
        VK_TAB => Some("Tab"),
        _ => None,
    }
}

/// 预览按键需要 keydown / keyup 配对发送，供前端按住显示、松开关闭。
fn preview_key(vk: u32) -> Option<&'static str> {
    match vk as i32 {
        VK_SPACE => Some(" "),
        _ => None,
    }
}

pub fn enable_navigation_keys(app: &AppHandle) {
    let _ = APP_HANDLE.set(app.clone());
    NAV_ENABLED.store(true, Ordering::Relaxed);

    // 已有钩子线程时不再起新线程；NAV_ENABLED 的恢复就够了。
    if HOOK_THREAD_ID
        .lock()
        .expect("hook thread id poisoned")
        .is_some()
    {
        return;
    }

    std::thread::spawn(|| unsafe {
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), null_mut(), 0);
        if hook.is_null() {
            log::error!("SetWindowsHookExW failed");
            return;
        }

        *HOOK_THREAD_ID.lock().expect("hook thread id poisoned") = Some(GetCurrentThreadId());

        let mut msg: MSG = std::mem::zeroed();
        // GetMessageW 收到 WM_QUIT 返回 0 → 消息泵自然退出。
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {}

        UnhookWindowsHookEx(hook);
        *HOOK_THREAD_ID.lock().expect("hook thread id poisoned") = None;
        consumed_keys()
            .lock()
            .expect("consumed keys poisoned")
            .clear();
    });
}

pub fn disable_navigation_keys() {
    NAV_ENABLED.store(false, Ordering::Relaxed);

    // 隐藏窗口时主动通知前端 Ctrl 已松开：隐藏后钩子线程随即退出，
    // 此后真实的 Ctrl keyup 不会再被捕获，否则前端会残留"Ctrl 按下"状态。
    if let Some(app) = APP_HANDLE.get() {
        if let Err(err) = app.emit(
            NAV_EVENT,
            json!({ "type": "keyup", "key": "Control", "ctrlKey": false }),
        ) {
            log::warn!("emit nav event failed: {err:?}");
        }
    }

    // 迟到的回放绝不能触发：队列与 ack 一并复位（回放线程在注入前会再验前台）。
    TYPEAHEAD_QUEUE
        .lock()
        .expect("typeahead queue poisoned")
        .clear();
    TYPEAHEAD_ACK.store(false, Ordering::Relaxed);

    let tid = HOOK_THREAD_ID
        .lock()
        .expect("hook thread id poisoned")
        .take();
    if let Some(tid) = tid {
        unsafe {
            PostThreadMessageW(tid, WM_QUIT, 0, 0);
        }
    }
}

/// 前端聚焦搜索框后调用，解锁回放线程注入被吞的按键。
pub fn ack_typeahead_focus() {
    TYPEAHEAD_ACK.store(true, Ordering::Relaxed);
}

/// 被吞的可打印字符入队并通知前端聚焦搜索框；单飞启动回放工作线程。
fn enqueue_typeahead_key(vk: u32, shift_down: bool) {
    TYPEAHEAD_QUEUE
        .lock()
        .expect("typeahead queue poisoned")
        .push_back((vk, shift_down));

    if let Some(app) = APP_HANDLE.get() {
        if let Err(err) = app.emit(SEARCH_TYPING_EVENT, json!({})) {
            log::warn!("emit search typing event failed: {err:?}");
        }
    }

    if TYPEAHEAD_WORKER_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }

    std::thread::spawn(run_typeahead_worker);
}

/// 回放被吞的字符：等前端 ack + 剪贴板窗口成为前台，再 SendInput 注入。
/// 注入的按键带 LLKHF_INJECTED 标志，钩子顶部会放行，不会再次被吞。
fn run_typeahead_worker() {
    let clipboard_hwnd = get_clipboard_hwnd();

    if !wait_for_condition(TYPEAHEAD_ACK_TIMEOUT, || {
        TYPEAHEAD_ACK.load(Ordering::Relaxed)
    }) {
        log::debug!("typeahead replay dropped: frontend focus ack timeout");
        clear_typeahead_worker_state();
        return;
    }

    if !wait_for_condition(TYPEAHEAD_FOREGROUND_TIMEOUT, || {
        (unsafe { GetForegroundWindow() } as isize) == clipboard_hwnd
    }) {
        log::debug!("typeahead replay dropped: clipboard window not foreground");
        clear_typeahead_worker_state();
        return;
    }

    let queued: Vec<(u32, bool)> = TYPEAHEAD_QUEUE
        .lock()
        .expect("typeahead queue poisoned")
        .drain(..)
        .collect();
    TYPEAHEAD_ACK.store(false, Ordering::Relaxed);

    for (vk, shift_down) in queued {
        if let Err(err) = crate::keystroke::send_keystroke(vk as u16, shift_down) {
            log::warn!("typeahead replay keystroke failed: {err:?}");
        }
    }

    clear_typeahead_worker_state();
}

fn get_clipboard_hwnd() -> isize {
    let Some(app) = APP_HANDLE.get() else {
        return 0;
    };

    app.get_webview_window(crate::window::CLIPBOARD_WINDOW_LABEL)
        .and_then(|window| window.hwnd().ok())
        .map(|hwnd| hwnd.0 as isize)
        .unwrap_or(0)
}

fn wait_for_condition(timeout: Duration, check: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + timeout;

    while std::time::Instant::now() < deadline {
        if check() {
            return true;
        }

        std::thread::sleep(TYPEAHEAD_POLL_INTERVAL);
    }

    check()
}

fn clear_typeahead_worker_state() {
    TYPEAHEAD_QUEUE
        .lock()
        .expect("typeahead queue poisoned")
        .clear();
    TYPEAHEAD_ACK.store(false, Ordering::Relaxed);
    TYPEAHEAD_WORKER_RUNNING.store(false, Ordering::SeqCst);
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 || !NAV_ENABLED.load(Ordering::Relaxed) {
        return CallNextHookEx(null_mut(), code, wparam, lparam);
    }

    let kbd = &*(lparam as *const KBDLLHOOKSTRUCT);
    let vk = kbd.vkCode;
    let msg = wparam as UINT;

    // 注入事件一律放行：包括本进程回放 / 模拟粘贴的按键与外部注入工具的输入，
    // 否则回放会被自己吞掉形成循环。
    let injected = kbd.flags & LLKHF_INJECTED != 0;
    if injected {
        return CallNextHookEx(null_mut(), code, wparam, lparam);
    }

    let ctrl_down = (GetAsyncKeyState(VK_CONTROL) as u16) & 0x8000 != 0;
    let alt_down = (GetAsyncKeyState(VK_MENU) as u16) & 0x8000 != 0;

    let is_ctrl = matches!(vk as i32, VK_CONTROL | VK_LCONTROL | VK_RCONTROL);

    if is_ctrl {
        if let Some(app) = APP_HANDLE.get() {
            let event_type = if msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN {
                Some("keydown")
            } else if msg == WM_KEYUP || msg == WM_SYSKEYUP {
                Some("keyup")
            } else {
                None
            };

            if let Some(event_type) = event_type {
                if let Err(err) = app.emit(
                    NAV_EVENT,
                    json!({ "type": event_type, "key": "Control", "ctrlKey": event_type == "keydown" }),
                ) {
                    log::warn!("emit nav event failed: {err:?}");
                }
            }
        }

        // Ctrl 状态只用于前端展示与组合键识别，不在此处吞键，避免影响系统行为。
        return CallNextHookEx(null_mut(), code, wparam, lparam);
    }

    let nav_key = nav_key(vk);
    let preview_key = preview_key(vk);
    let shortcut_key = if ctrl_down {
        ctrl_shortcut_key(vk)
    } else {
        None
    };

    if msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN {
        if let Some(shortcut_key) = shortcut_key {
            if let Some(app) = APP_HANDLE.get() {
                if let Err(err) = app.emit(
                    NAV_EVENT,
                    json!({ "type": "keydown", "key": shortcut_key, "ctrlKey": true }),
                ) {
                    log::warn!("emit nav event failed: {err:?}");
                }
            }

            consumed_keys()
                .lock()
                .expect("consumed keys poisoned")
                .insert(vk);
            return 1;
        }

        if let Some(preview_key) = preview_key {
            let mut consumed = consumed_keys().lock().expect("consumed keys poisoned");
            if consumed.contains(&vk) {
                return 1;
            }

            consumed.insert(vk);
            drop(consumed);

            if let Some(app) = APP_HANDLE.get() {
                if let Err(err) = app.emit(
                    NAV_EVENT,
                    json!({ "type": "keydown", "key": preview_key, "code": "Space" }),
                ) {
                    log::warn!("emit nav event failed: {err:?}");
                }
            }

            return 1;
        }

        if let Some(nav_key) = nav_key {
            if let Some(app) = APP_HANDLE.get() {
                let shift_down = if nav_key == "Tab" {
                    (GetAsyncKeyState(VK_SHIFT) as u16) & 0x8000 != 0
                } else {
                    false
                };

                if let Err(err) = app.emit(
                    NAV_EVENT,
                    json!({ "type": "keydown", "key": nav_key, "shiftKey": shift_down }),
                ) {
                    log::warn!("emit nav event failed: {err:?}");
                }
            }
            // 记下 KEYDOWN 的 VK，配对的 KEYUP 也要吞——
            // 否则背后被聚焦的应用会收到孤立 KEYUP，造成奇怪行为。
            consumed_keys()
                .lock()
                .expect("consumed keys poisoned")
                .insert(vk);
            return 1;
        }

        // 无修饰的可打印字符：吞掉并通知前端聚焦搜索框（Ditto 式随时输入即搜索）。
        // 回放线程等前端 ack + 窗口成为前台后注入，否则用户前台应用会收到字符。
        if !ctrl_down && !alt_down && typeahead_key(vk) {
            let shift_down = (GetAsyncKeyState(VK_SHIFT) as u16) & 0x8000 != 0;

            enqueue_typeahead_key(vk, shift_down);
            consumed_keys()
                .lock()
                .expect("consumed keys poisoned")
                .insert(vk);
            return 1;
        }
    } else if msg == WM_KEYUP || msg == WM_SYSKEYUP {
        if let Some(preview_key) = preview_key {
            if consumed_keys()
                .lock()
                .expect("consumed keys poisoned")
                .remove(&vk)
            {
                if let Some(app) = APP_HANDLE.get() {
                    if let Err(err) = app.emit(
                        NAV_EVENT,
                        json!({ "type": "keyup", "key": preview_key, "code": "Space" }),
                    ) {
                        log::warn!("emit nav event failed: {err:?}");
                    }
                }

                return 1;
            }
        }

        if consumed_keys()
            .lock()
            .expect("consumed keys poisoned")
            .remove(&vk)
        {
            return 1;
        }
    }

    CallNextHookEx(null_mut(), code, wparam, lparam)
}

#[cfg(test)]
mod tests {
    use super::typeahead_key;
    use winapi::um::winuser::{
        VK_BACK, VK_CONTROL, VK_DOWN, VK_LEFT, VK_MENU, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE,
        VK_TAB, VK_UP,
    };

    #[test]
    fn typeahead_key_accepts_letters_digits_and_oem_symbols() {
        // 字母 A-Z。
        assert!(typeahead_key(0x41));
        assert!(typeahead_key(0x5A));
        // 数字主行 0-9。
        assert!(typeahead_key(0x30));
        assert!(typeahead_key(0x39));
        // 小键盘数字不纳入：与 Ctrl+1-9 快捷键语义区分，且多数用户搜索用主行。
        assert!(!typeahead_key(0x60));
        assert!(!typeahead_key(0x69));
        // OEM 符号（US 布局 ;=,-./` [\]')。
        assert!(typeahead_key(0xBA));
        assert!(typeahead_key(0xC0));
        assert!(typeahead_key(0xDB));
        assert!(typeahead_key(0xDF));
        // OEM 102（欧洲布局附键）。
        assert!(typeahead_key(0xE2));
    }

    #[test]
    fn typeahead_key_accepts_backspace_for_search_editing() {
        // Backspace 与字符键一视同仁入队回放；空搜索词时删除是无害空操作。
        assert!(typeahead_key(VK_BACK as u32));
    }

    #[test]
    fn typeahead_key_rejects_navigation_and_modifier_keys() {
        // Space 是预览专用键，不进入搜索框。
        assert!(!typeahead_key(VK_SPACE as u32));
        // 方向 / 功能键不纳入。
        assert!(!typeahead_key(VK_UP as u32));
        assert!(!typeahead_key(VK_DOWN as u32));
        assert!(!typeahead_key(VK_LEFT as u32));
        assert!(!typeahead_key(VK_RIGHT as u32));
        assert!(!typeahead_key(VK_RETURN as u32));
        assert!(!typeahead_key(VK_TAB as u32));
        // 修饰键不纳入（组合键路径由调用方排除）。
        assert!(!typeahead_key(VK_CONTROL as u32));
        assert!(!typeahead_key(VK_SHIFT as u32));
        assert!(!typeahead_key(VK_MENU as u32));
        // F1-F12 (0x70-0x7B) 不纳入。
        assert!(!typeahead_key(0x70));
        assert!(!typeahead_key(0x7B));
    }
}

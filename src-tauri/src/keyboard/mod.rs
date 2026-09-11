//! 剪贴板窗口 focusable=false 后，Windows 上 WebView 收不到键盘事件，
//! 需要装 OS 级低级钩子捕获导航键再 emit 给前端。
//! 与 `keystroke/`（向外注入按键模拟粘贴）方向相反：本模块是向内捕获。
//! macOS 走 NSPanel 自己接管键盘事件，无需本模块——故仅 windows target 启用。

pub const NAV_EVENT: &str = "keyboard://nav";

/// 搜索回放握手事件：钩子吞下首个可打印字符后通知前端聚焦搜索框，
/// 前端 ack 后由 Rust 回放被吞的按键（保证 IME 从首字符起正常组合）。
pub const SEARCH_TYPING_EVENT: &str = "clipboard://search-typing";

mod windows;
pub use windows::{ack_typeahead_focus, disable_navigation_keys, enable_navigation_keys};

# Implement: 剪贴板窗口搜索框常驻就绪（Ditto 式 type-to-search）

> 顺序执行；每步末尾的验证命令通过后再进下一步。Rust 步骤（S1-S6）与前端步骤（S7-S11）分别内部有序，S1-S6 全部完成后再开始 S7 可减少中间态编译失败。

## S1 Rust: keystroke 抽出单键发送函数

- [x] `src-tauri/src/keystroke/windows.rs`：新增 `pub fn send_keystroke(vk: u16, shift_down: bool) -> Result<()>`（构造 Shift↓?/VK↓/VK↑/Shift↑? 的 SendInput 序列）；`simulate_paste` 不动。
- 验证：`cargo clippy -- -D warnings`。

## S2 Rust: 钩子放行可打印字符 + 回放工作线程 + injected 过滤

- [x] `src-tauri/src/keyboard/mod.rs`：`pub const SEARCH_TYPING_EVENT: &str = "clipboard://search-typing";`
- [x] `src-tauri/src/keyboard/windows.rs`：
  - `hook_proc` 顶部过滤 `LLKHF_INJECTED`（`kbd.flags & LLKHF_INJECTED != 0` → 放行）。
  - `fn typeahead_key(vk: u32) -> bool`：数字/字母/OEM 符号；不含 VK_SPACE、修饰键。
  - keydown 分支末尾新增：`!ctrl_down && !alt_down(GetAsyncKeyState(VK_MENU)) && typeahead_key(vk)` → 记 consumed_keys（吞配对 keyup）→ 记录 `(vk, shift)` 入 `TYPEAHEAD_QUEUE` → emit `SEARCH_TYPING_EVENT` → spawn/唤醒单飞回放线程 → `return 1`。
  - 回放线程：等 `TYPEAHEAD_ACK`（≤150ms）→ 等前台==剪贴板 HWND（≤250ms）→ 终检前台 → 逐键 `send_keystroke` → 清 ack；超时丢队列 log debug。
  - `pub fn ack_typeahead_focus()`：置 ack。
  - `disable_navigation_keys()`：清队列 + 复位 ack。
- 验证：`cargo clippy -- -D warnings`；`cargo test`（含新增表驱动单测）。

## S3 Rust: editing 进入改为焦点完成后禁钩子

- [x] `src-tauri/src/window/windows.rs::set_clipboard_window_editing`：editing=true 分支保留 HWND → `set_focusable(true)` → `set_focus()` 后启动单飞焦点观察线程（轮询 `GetForegroundWindow()==hwnd`，≤300ms），成功才 `keyboard::disable_navigation_keys(app)`；超时 `set_focusable(false)` 回滚 + warn。
- 验证：`cargo clippy -- -D warnings`。

## S4 Rust: 新命令 + 注册

- [x] `src-tauri/src/commands/window.rs`：`#[tauri::command] pub async fn search_typing_ack(...)`（Windows 调 `keyboard::ack_typeahead_focus()`，其它平台 no-op success）。
- [x] `src-tauri/src/lib.rs`：invoke_handler 注册。
- 验证：`cargo clippy -- -D warnings`。

## S5 Rust: 粘贴前强制退出 editing

- [x] `src-tauri/src/commands/clipboard.rs::paste_clipboard_item`：simulate 前 `#[cfg(windows)] window::set_clipboard_window_editing(&app, false)`（放 hide/resign 之后；`set_clipboard_window_editing(false)` 本身幂等且未 editing 时无副作用——核对 `editing` 状态位，若 Rust 侧无状态位则直接调用，其内部已有幂等路径）。
- 验证：`cargo clippy -- -D warnings`。

## S6 Rust: macOS defaultFocus 默认值

- [x] `src-tauri/src/settings/model.rs::Search::default()`：`default_focus` 按 `#[cfg(target_os = "macos")]` true / Windows false。
- 验证：`cargo test`（确认相关 settings 默认值测试不冲突，必要时补断言）。

## S7 前端: 常量 + 命令包装 + dom 工具

- [x] `src/constants/events.ts`：`SEARCH_TYPING: "clipboard://search-typing"`。
- [x] `src/commands/index.ts`：`searchTypingAck()`。
- [x] `src/utils/dom.ts`：抽出 `findEditableElement` / `isEditableElement`；`useKeyboardEvent.ts`、`useClipboardWindowEditableFocus.ts` 改为引用。
- 验证：`pnpm tsc`、`pnpm lint`。

## S8 前端: editable-focus 退出时机 + store 搜索状态收口

- [x] `src/hooks/useClipboardWindowEditableFocus.ts`：移除 `focusout → scheduleRestore`（保留 window blur / visibilitychange / focusin / pointerdown 路径）。
- [x] `src/stores/clipboardView.ts`：新增 `searchClearToken`；模块级 200ms 防抖 `setClipboardSearchKeyword`；`clearClipboardSearch()`（取消防抖 + keyword="" + token++）。
- 验证：`pnpm tsc`。

## S9 前端: type-ahead hook

- [x] `src/pages/Clipboard/hooks/useSearchTypeahead.ts`：真实 keydown 守卫劫持聚焦（单字符、" "排除、无修饰、非 composing、activeElement 非可编辑、无 `[role="dialog"]`）+ `useTauriListen(SEARCH_TYPING)` → prepare → focus(cursor:end) → ack。
- 验证：`pnpm tsc`、`pnpm lint`。

## S10 前端: 接线 Header/SearchInput/List + 文案

- [x] `SearchInput.tsx`：接受 `ref` prop（React 19 ref-as-prop），透传 antd Input；focus/blur/clear token 效果优先用传入 ref。
- [x] `Header.tsx`：`searchInputRef` + `useSearchTypeahead`；onChange → `setClipboardSearchKeyword`；clearSearch → `clearClipboardSearch`；移除本地 `searchClearToken` state 与 `useDebounceFn`。
- [x] `List.tsx::closeTopEscapeLayer`：预览 → **搜索词** → 分组 → 分类 → 隐藏。
- [x] `ShortcutList.tsx` + `clipboard.json`(zh/en)：`shortcuts.typeToSearch`。
- 验证：`pnpm tsc`、`pnpm lint`。

## S11 全量验证 + 实机操作（A8/A9）

- [x] `cd src-tauri && cargo fmt && cargo clippy -- -D warnings && cargo test`
- [x] `pnpm lint && pnpm tsc`
- [ ] `pnpm tauri dev` 实机验证清单：
  - 开窗直接打英文 → 过滤；打中文（IME）→ 首字正确组合
  - ↑↓ 浏览中打字 → 进搜索框且列表导航不乱
  - Enter 粘贴（非 pinned）→ 注入前台应用；pinned 同样
  - Esc 层序：预览→搜索词→分组→分类→隐藏
  - Ctrl 系（F/Q/C/O/D/T/M/Backspace/1-0/N/P/K/,）逐一回归
  - 备注弹窗/删除确认打开时打字 → 进弹窗输入框
  - 点击外部应用 → 窗口隐藏 → 在外部应用打字正常（无字符泄漏）
- [ ] 提交（Conventional Commits，单行）：`feat: keep clipboard search box ready for type-to-search like Ditto`

## 回退点

- S2 钩子分支去掉即恢复 Windows 透传（其余步骤无害共存）。
- S8 store 收口如出问题可回退为 Header 本地防抖（保留 focusout 移除）。
- 整体：git 单 commit，revert 即全量回退。

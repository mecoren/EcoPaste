# 剪贴板窗口搜索框常驻就绪，随时输入即过滤（Ditto 式）

## Goal

剪贴板窗口打开后，搜索框始终处于"就绪"状态：用户在用上下键浏览列表的任意时刻，只要开始打字，字符就**直接落入搜索框**并实时过滤列表（Ditto 的交互）。不需要先按 ⌘F / Ctrl+F 或点击搜索框。

## Background（现状）

- 搜索过滤链路已存在：`SearchInput.onChange` → 200ms 防抖 → `clipboardViewState.keyword` → 列表自动查询（`src/pages/Clipboard/components/Header.tsx`、`src/hooks/useClipboardItems.ts`）。
- 焦点是瓶颈：
  - `clipboard.search.defaultFocus` 默认 `false`，窗口显示时不会聚焦搜索框。
  - Windows 剪贴板窗口 `focusable=false`，键盘导航靠 Rust 低级钩子（`src-tauri/src/keyboard/windows.rs`）转发 `keyboard://nav` 事件；搜索框进入编辑态时钩子会先 `disable_navigation_keys`（`src-tauri/src/window/windows.rs:37`）。焦点不在输入框时，打字要么完全丢失（Windows），要么没有输入目标（macOS）。
  - `useKeyboardEvent` 的 handoff 列表（`src/hooks/useKeyboardEvent.ts:10`）只放行方向键/Enter/Escape/Tab 等，**可打印字符不转发**。
- macOS：剪贴板窗口是 NSPanel（`show_and_make_key`），显示时 webview 有键盘焦点，常驻聚焦搜索框可行；但非激活面板，打字聚焦需不推走用户前台 App（现有编辑态机制已处理）。

## Requirements

### R1 打字即入搜索框（核心）

剪贴板窗口可见期间，任意时刻按下可打印字符（含 IME 拼音起始键），字符进入搜索框并触发过滤：

- macOS：webview 内焦点不在可编辑元素时，可打印字符键按下后焦点自动落到搜索框（保留已输入字符，不丢首字符）。
- Windows：焦点不在输入框时，低级钩子需把可打印字符事件传给前端（转发或等效机制），前端把字符写入搜索框；期间导航键语义不变。

### R2 导航键语义保持

- 上下键：焦点在搜索框时移动**列表选中项**（不移动输入光标）；焦点不在搜索框时行为同现状。
- 左右键：焦点在搜索框时移动**输入光标**（不切分类）；现状保持。
- Enter：粘贴当前选中项（现状语义不变）。
- Space：焦点在搜索框时按住预览当前选中项；不向输入框插入空格（优先级：预览 > 输入）。
- Escape：逐层退出顺序保持（预览 → 搜索词 → 分组 → 分类 → 隐藏窗口）；**新增：搜索词非空时第一优先清空搜索词**。
- 所有 Cmd/Ctrl 组合键（C/O/D/T/M/Backspace/Delete/数字 1-0 等）行为不变。

### R3 IME 兼容

- 中文拼音等 IME 输入期间，组合态字符不触发列表过滤中间抖动（现有 `SearchInput` composition 处理保持）。
- Windows：搜索框编辑态期间导航钩子已禁用（现状），常驻聚焦后 Enter/Escape 需仍能到达前端（现状已可用，验证即可）。

### R4 粘贴流程不回归

- Enter / 点击粘贴：非固定窗口粘贴前先 hide（Windows 前台回原应用），固定窗口 macOS resign key——均已有机制；搜索框聚焦状态不影响 simulate_paste 的目标窗口。需实际验证固定 + 非固定两种模式。

### R5 显式不做（Out of Scope）

- 搜索词非空时 Enter 的"搜索命中优先"语义（Ditto 语义同样是粘贴选中项，保持一致）。
- `search.defaultFocus` 设置项的新增/删除（见 A3 兼容策略后不需要）。
- 分组栏 SourceApp 弹层内搜索的常驻焦点（弹层内已有自己的焦点管理）。

## Acceptance Criteria

- [ ] A1 Windows：打开剪贴板窗口 → 不做任何点击/聚焦操作 → 直接打字 → 字符出现在搜索框且列表按输入过滤（含中文 IME）。
- [ ] A2 macOS：同 A1。
- [ ] A3 老用户设置兼容：已发布的 `settings.json` 无新字段时按 `#[serde(default)]` 回落，现有 `defaultFocus`/`clearOnHide` 语义不破坏（不新增设置项）。
- [ ] A4 上下键在搜索框聚焦时移动列表选中项；Space 预览、Enter 粘贴、Escape 逐层退出（新增清搜索词优先级）全部可用。
- [ ] A5 ⌘F/Ctrl+F 聚焦、⌘Q、左右键、Tab、数字 1-0、⌘C/⌘O/⌘D/⌘T/⌘M/⌘Backspace、⌘N、⌘P、⌘K、⌘, 等既有快捷键全部不回归。
- [ ] A6 固定窗口（pinned）与非固定窗口下 Enter 粘贴均正常注入到前台应用。
- [ ] A7 偏好设置里既有搜索相关开关（defaultFocus/clearOnHide）与新行为不冲突。
- [ ] A8 `pnpm lint`、`cd src-tauri && cargo clippy -- -D warnings && cargo test`、`cargo fmt` 全部通过。
- [ ] A9 改 UI 后实际操作验证主路径与边界（Tauri dev 运行验证 A1-A7 中 Windows 侧）。

## Constraints

- 遵守 AGENTS.md：Rust-First（Windows 按键放行必须在 Rust 低级钩子层做；macOS 可在前端处理）、仅 macOS+Windows、提交信息 Conventional Commits。
- 不回滚 worktree 中已有未提交改动（来源应用筛选功能与本任务无重叠：`Group.tsx` 两边都改但区域不同，先读后改）。
- 事件名复用既有常量（`TAURI_EVENT.KEYBOARD_NAV` / Rust `NAV_EVENT`），新增字面量按 AGENTS.md 集中维护。

# Design: 剪贴板窗口搜索框常驻就绪（Ditto 式 type-to-search）

## 1. 总体方案

"搜索框常驻就绪"的本质：**剪贴板窗口可见期间，任意时刻按下可打印字符，字符必须落入搜索框并触发过滤**。两平台输入通路不同，方案分平台实现，共享一个前端 type-ahead 层：

- **macOS**：NSPanel 显示即成为 key window，webview 直收浏览器键盘事件。前端在 window 级监听 keydown，发现"可打印字符 + 焦点不在可编辑元素"时**同步聚焦搜索框**（keydown 派发期间聚焦，浏览器默认动作把字符插入新聚焦的 input）。焦点在搜索框时上下键经冒泡继续驱动列表导航（现状已如此）。
- **Windows**：剪贴板窗口默认不可聚焦（不抢前台），键盘走 Rust 低级钩子。钩子此前只放行导航键/快捷键，**可打印字符直接漏到用户前台应用**。新增"惰性接管"：
  1. 钩子吞掉无修饰的可打印字符 keydown（VK 字母/数字/OEM 符号；排除 Space=预览、Ctrl/Alt 组合）；
  2. 发 `clipboard://search-typing` 事件 → 前端聚焦搜索框并 ack；
  3. Rust 工作线程等 ack + 前台==剪贴板窗口后，用 `SendInput` **回放**被吞的按键 → WebView2 以真实按键收到 → IME 从第一个字符就正常组合（拼音不丢首字母）；
  4. 聚焦即进入既有 editing 模式（focusable+set_focus+禁钩子），后续按键全部走浏览器原生路径，无需再回放。

**为什么不回放而是前端直接 append**：synthetic 事件没有默认插入动作，前端 append 只能写入 ASCII 字面量，中文拼音首键会变成裸字母（"nihao"→"n你好"）。SendInput 回放让 IME 正确接管，这是中文默认语言产品的硬需求。

**为什么不打开窗口即抢焦点（Ditto 原生做法）**：违背本仓库既有契约——Windows 剪贴板窗口显示不抢前台（spec 记载的 Good case）。惰性接管只在用户真的开始打字时才拿焦点，打开→浏览→Enter 粘贴的主路径完全不变。

## 2. 模块改动

### 2.1 Rust `src-tauri/src/keyboard/windows.rs`

- `hook_proc` 顶部新增 **LLKHF_INJECTED 过滤**：注入事件（含我们回放的、模拟粘贴的）直接 `CallNextHookEx` 放行，防止回放按键被钩子再次吞掉形成循环，也让外部注入工具的按键不被劫持。
- 新增 `fn typeahead_key(vk: u32) -> bool`：`0x30..=0x39`（数字）、`0x41..=0x5A`（字母）、`0xBA..=0xBF`、`0xC0`、`0xDB..=0xDF`、`0xE2`（OEM 符号）。**不含** VK_SPACE（预览键）、修饰键。
- keydown 分支顺序不变（Ctrl 快捷键 → Space 预览 → 导航键），其后新增：`!ctrl_down && !alt_down && typeahead_key(vk)` → `consumed_keys` 记录 VK（吞配对 keyup，现有机制）→ 入队 → emit `SEARCH_TYPING_EVENT` → 确保回放工作线程在跑 → `return 1`。Alt 检查用 `GetAsyncKeyState(VK_MENU)`，放行 AltGr 布局用户。
- 回放工作线程（同文件）：static `TYPEAHEAD_QUEUE: Mutex<VecDeque<(vk, shift)>>` + `TYPEAHEAD_ACK: AtomicBool` + `WORKER_WAITING: AtomicBool` 单飞。流程：等 ack（≤150ms，5ms 轮询）→ 等 `GetForegroundWindow()==clipboard hwnd`（≤250ms）→ 取 clipboard HWND → 终检 foreground → 逐键 `keystroke::send_keystroke(vk, shift)`（保持按键顺序）→ 清 ack。任一超时：**丢弃队列并 log debug**（绝不向非剪贴板前台注入，避免把字符打进用户文档）。
- `disable_navigation_keys()` 时清空队列 + 复位 ack（窗口隐藏后迟到的回放绝不触发）。
- 纯函数 `typeahead_key` 写表驱动单测。

### 2.2 Rust `src-tauri/src/keyboard/mod.rs`

- 新增 `pub const SEARCH_TYPING_EVENT: &str = "clipboard://search-typing";`（与前端 `TAURI_EVENT.SEARCH_TYPING` 镜像，遵循 `domain://action`）。

### 2.3 Rust `src-tauri/src/window/windows.rs` — editing 进入改为"焦点完成后才禁钩子"

现状 `set_clipboard_window_editing(true)` 立即 `disable_navigation_keys`。问题：`set_focus` 是异步派发，禁钩子到焦点完成之间有几十 ms 空窗——用户连打"hello"，首键被吞排队，第 2、3 键穿过钩子落进**用户的前台应用**（stray char）。

改为：`set_focusable(true)` + `set_focus()` 后启动**单飞焦点观察线程**（static `WATCHER_RUNNING`）：轮询 `GetForegroundWindow()==hwnd`（≤300ms，10ms 步进）→ 成功才 `keyboard::disable_navigation_keys(app)`；超时（SetForegroundWindow 被拒）→ `set_focusable(false)` 回滚 + warn。空窗期内钩子继续吞可打印字符入队，与回放机制天然衔接；Ctrl+F / 点击输入框 / type-ahead 三条进入路径统一。

### 2.4 Rust `src-tauri/src/keystroke/windows.rs`

- 抽出 `pub fn send_keystroke(vk: u16, shift_down: bool) -> Result<()>`：按 shift 状态构造 `[Shift↓?] Vk↓ Vk↑ [Shift↑?]` 的 SendInput 序列，回放工作线程复用；`simulate_paste` 保持不动。

### 2.5 Rust `src-tauri/src/commands/window.rs` + `lib.rs`

- 新增 `#[tauri::command] search_typing_ack()`：Windows 调 `keyboard::ack_typeahead_focus()`，其它平台 no-op success。注册进 invoke_handler。

### 2.6 Rust `src-tauri/src/commands/clipboard.rs` — 粘贴前强制退出 editing

editing 持久化后（见 2.9），Windows 下"搜索框聚焦时按 Enter 粘贴"链路里剪贴板窗口仍是前台。`paste_clipboard_item` 在 simulate 前增加：`#[cfg(windows)] window::set_clipboard_window_editing(&app, false)`——显式把前台还给用户应用（restore_pre_edit_foreground 既有逻辑），不依赖"隐藏窗口后系统自然交还前台"的不确定行为。pinned（窗口保持可见）与非 pinned（hide）两条路径都覆盖。

### 2.7 Rust `src-tauri/src/settings/model.rs`

- `Search::default()` 的 `default_focus` 改为平台条件默认：macOS `true`（panel 已是 key window，聚焦无副作用，且让 IME 首字符正确）、Windows `false`（保持不抢前台契约）。用 `#[cfg(target_os = ...)]` 分支。
- 兼容性：`#[serde(default)]` 语义 = 配置文件里**已有该字段的用户保持原值**；只有"从未写过该字段的老配置 + 全新安装"吃到新默认。无 migration。

### 2.8 前端常量 / 命令 / 工具

- `src/constants/events.ts`：`SEARCH_TYPING: "clipboard://search-typing"`。
- `src/commands/index.ts`：`searchTypingAck()` 包装。
- `src/utils/dom.ts`（新建）：把 `findEditableElement` / `isEditableElement` 从 `useKeyboardEvent` / `useClipboardWindowEditableFocus` 的两份私有实现抽出共享（type-ahead 是第三个消费方，避免第三份拷贝）。

### 2.9 前端 `src/hooks/useClipboardWindowEditableFocus.ts` — editing 退出时机

- **移除 focusout 触发的 restore**（这是 handoff blur 后窗口失焦、打字回到用户应用的根因）。
- 保留：`focusin`（可编辑聚焦→editing=true）、`pointerdown` 激活、`window blur` → 80ms 后 restore（用户点到别的应用时交还前台）、`visibilitychange hidden` → 立即 editing=false。
- 效果：handoff blur（方向键导航）后窗口**保持**前台与 editing，后续打字走原生浏览器事件；窗口隐藏或用户点击外部应用才退出。

### 2.10 前端 `src/stores/clipboardView.ts` — 搜索状态收口

新增（valtio UI 临时状态，符合"store 只存 UI 状态"边界；**不进** `useClipboardItems` 的查询参数——List 是显式传参）：

- `searchClearToken: number`（驱动 SearchInput `key` 重挂载清空文本，沿用现有机制）。
- `setClipboardSearchKeyword(value)`：模块级 200ms 防抖写 `keyword`（把 Header 里 `useDebounceFn` 的职责移入 store，List 的 Escape 清空才能取消同一份 pending 写入，消除"清空后 200ms 关键词复活"竞态）。
- `clearClipboardSearch()`：取消防抖 + `keyword=""` + token 自增。

### 2.11 前端 `src/pages/Clipboard/hooks/useSearchTypeahead.ts`（新建）

```ts
useSearchTypeahead({ inputRef }) {
  // 1) 浏览器真实 keydown（macOS 全程；Windows 窗口聚焦但输入框已 handoff blur 时）
  //    守卫：单字符 key、非 " "、无 ctrl/meta/alt、非 isComposing、
  //         activeElement 非可编辑、无 [role="dialog"]（antd Modal 确认框/备注框打开时不劫持）
  //    动作：inputRef.current?.focus({ cursor: "end" }) —— 同步聚焦，浏览器默认动作插入字符
  // 2) useTauriListen(SEARCH_TYPING)：await prepareClipboardWindowEditableFocus()
  //    → focus({ cursor:"end" }) → await searchTypingAck()（触发 Rust 回放）
}
```

cursor 用 `"end"`（追加，不选中文本——否则导航后继续打字会覆盖已有关键词）；Ctrl+F / 窗口打开聚焦保持既有 `cursor:"all"`（覆盖式重输）。

### 2.12 前端 Header / SearchInput / List

- `SearchInput`：接受 `ref` prop（React 19 ref-as-prop，透传给 antd `Input`；内部 blur/focus token 效果改用传入 ref，无传入时回落本地 ref）。
- `Header`：`searchInputRef` 创建并传给 SearchInput；挂 `useSearchTypeahead`；`clearSearch` 改用 store 的 `clearClipboardSearch`（本地 `searchClearToken` state 与 `useDebounceFn` 移除）；`onChange` 改调 `setClipboardSearchKeyword`。
- `List.closeTopEscapeLayer`：插入关键词层，顺序 **预览 → 搜索词 → 分组 → 分类 → 隐藏窗口**（预览是最顶层瞬态，先关；搜索词次之）。

### 2.13 文案

- `ShortcutList` 增加一行"任意字符 → 直接搜索"提示；`clipboard.json` zh-CN/en-US 各加 `shortcuts.typeToSearch`。

## 3. 事件与命令契约（跨端镜像）

| 层 | 名称 |
| --- | --- |
| Rust 常量 | `keyboard::SEARCH_TYPING_EVENT = "clipboard://search-typing"` |
| 前端常量 | `TAURI_EVENT.SEARCH_TYPING = "clipboard://search-typing"` |
| Rust command | `search_typing_ack()` |
| TS wrapper | `searchTypingAck()` |
| store | `searchClearToken` / `setClipboardSearchKeyword` / `clearClipboardSearch` |

## 4. 关键竞态与对策

| 竞态 | 对策 |
| --- | --- |
| 回放键再次进钩子（循环吞键） | LLKHF_INJECTED 过滤放行 |
| 禁钩子→焦点完成间连打，字符漏进用户应用 | editing 进入改为焦点完成后才禁钩子；空窗期钩子继续吞入队 |
| 焦点被 Windows 拒绝（SetForegroundWindow 限制） | 观察线程超时回滚 focusable；回放线程超时丢键（log），绝不注入非剪贴板前台 |
| 窗口隐藏后迟到的回放把字符打进用户应用 | 回放前终检 foreground==clipboard；`disable_navigation_keys` 清队列/ack |
| Escape 清空关键词 vs 防抖 pending 复活 | 防抖收口进 store，clear 时取消 |
| 确认弹窗/备注框打开时打字被劫持到搜索框 | `[role="dialog"]` 守卫 + 弹窗输入框聚焦时 hooks 已禁（editing 期间钩子本来就关） |
| 双份 keydown（真实 + 钩子转发）双跳列表 | 互斥：钩子事件只在窗口未聚焦时产生，浏览器事件只在聚焦时产生；禁钩子原子位在焦点完成后 |
| 手型键序：输入框聚焦→方向键 handoff blur→再打字 | 真实事件同步 refocus（cursor:end），首字符可能裸插入（IME 第二键起恢复组合）——已知可接受边角，与浏览器 type-ahead-find 行为一致 |

## 5. 行为变化（需在总结中明示用户）

1. Windows：剪贴板窗口可见期间，无修饰可打印字符不再透传给前台应用，而是进入搜索框（Ditto 契约）。
2. Windows：从"开始打字"起窗口持有前台焦点，直到隐藏或用户点击其它应用（pinned 窗口同理）。
3. Escape 多了一层"先清搜索词"。
4. 新安装默认：macOS 打开窗口即聚焦搜索框（`defaultFocus`），Windows 保持不聚焦。已有用户设置原样保留。

## 6. 测试策略

- Rust：`typeahead_key` 表驱动单测；`cargo clippy -- -D warnings`、`cargo fmt`、`cargo test`。
- 前端：`pnpm lint`（biome）、`pnpm tsc`。
- 手动（Windows 实机，A9）：开窗不点击直接打英文/中文搜索；方向键浏览后继续打字；Enter 粘贴（pinned/非 pinned）；Escape 层序；Ctrl 系快捷键回归；备注/确认弹窗打开时打字不劫持；点击外部应用后打字回自己应用。

## 7. 风险与回退

- WKWebView/WebView2 "keydown 派发中聚焦→默认动作插入字符"行为若有差异（首字符丢失），回退方案：type-ahead 真实路径 preventDefault + 手动 append（dispatch input event 触发 React onChange），IME 首键语义不变（本来就不组合）。实现时先验证，再决定是否需要。
- 整体回退点：Rust 钩子改动与前端 type-ahead 各自独立可关（去掉 `typeahead_key` 分支即恢复透传）。

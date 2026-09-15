# Design: 悬浮预览可交互

## 1. 边界与所有权

| 能力 | 归属 | 说明 |
| --- | --- | --- |
| 鼠标穿透开关、面板命中矩形、指针 inside 状态 | Rust `window/preview.rs` | OS 级窗口属性，前端不可见 |
| 指针位置采样（进入面板的引导） | Rust 后台线程 | Windows `GetCursorPos` / macOS `CGEvent` 当前位置 |
| 滚动 / 拖选 / 选区渲染 / 浮动复制按钮 | 前端 `src/pages/Preview/` | 纯 DOM 交互 |
| 片段写回剪贴板 | Rust `clipboard/write.rs` + 新命令 | 跨进程副作用，Rust-first |
| 跨窗口「当前选中片段」同步 | 前端 `emitTo` + 主窗 ref | 不落库，纯 UI 协作态 |
| 图片文件读图 / 判据 | Rust 命令层（既有 `FilesPreviewKind`） | 与列表卡片同源，前端不重算 |

## 2. 关键决策

### D1 指针跟踪用「低频采样 + 前端事件」而不是 OS 鼠标钩子 / NSEvent 监听

原方案计划在 Windows `WH_MOUSE_LL` 里处理 `WM_MOUSEMOVE`、macOS 装 NSEvent local/global monitor。改为一颗 Rust 后台采样线程，原因：

- **引导问题只能靠主动查询解决**：面板初始 `ignore_cursor_events(true)`，前端拿不到 `pointerenter`；必须由 Rust 侧判断「光标已进入面板矩形」才能翻转穿透。既然必须主动查一次位置，采样线程就够用，不必再接两个平台的事件机制。
- **成本更低、风险更小**：`WH_MOUSE_LL` 每 move 触发，需要额外的原子门控与 LowLevelHooksTimeout 保护；NSEvent monitor 需要 `block2` 依赖 + points→物理坐标换算；采样线程一次 tick 只是一次 `GetCursorPos` / `CGEvent` + 一次矩形比较。
- **延迟可接受**：采样周期 40ms，主窗 hover 关闭缓冲是 240ms，进入方向有 6 倍余量。

采样线程只在预览可见时工作（不可见时降频到 200ms 空转），预览隐藏后不引入任何开销。

**实现期收敛**：采样线程**只负责「进入」方向**（`reconcile_pointer(app, enter_only = true)`），离开方向完全由面板前端的 `pointerleave` 驱动。原因：拖选常常把指针带出面板边缘，采样若也负责离开，就会在拖动途中把面板打回穿透，拖选与滚动当场断掉。显式布局变化（retarget）用 `enter_only = false` 做完整校对。

### D2 离开方向由前端事件驱动，避免透明 overlay 吞点击的窗口期

`set_ignore_cursor_events(false)` 作用在整个 overlay 窗口上，而 overlay 是覆盖整个显示器的透明窗口：翻转期间「面板外」的区域理论上也会接收鼠标事件。为把风险压到最小：

- **进入方向**（穿透 → 可交互）：采样线程翻转，最坏 40ms 内面板外可能短暂吞掉一次点击。实际上光标必须先移动到面板外，而移动会在下一个 tick 被采样到并立即翻回，物理上几乎不可能在 40ms 内移出并完成点击。
- **离开方向**（可交互 → 穿透）：面板前端 `pointerleave` 立即回调 Rust 翻转，不等采样周期，窗口期≈0。

**拖选期间的例外与本机限制**：`pointerleave` 在 `event.buttons !== 0`（按住左键）时不回报，避免拖选滑出面板边缘时被打回穿透；松手时由 `pointerup` 回报一次，Rust 侧按真实光标位置重判（`report_pointer_inside(false)` 不是直接置穿透，而是走同一套命中判定），避免「松手在面板外」把面板永久卡在可交互态。原本设想在 Rust 侧按「鼠标左键是否按下」保持交互，但 core-graphics 0.25 未提供 `CGEventSource::button_state`，跨平台按键状态查询成本高于收益，故把保护放在 DOM 侧。

### D3 片段入库走「不登记回环抑制」的写回

`WritebackGuard` 的目的是阻止自身写回被 OS 监听当成新复制再入库。片段复制的产品语义恰恰相反——**希望**它成为一条新记录，因此新命令写剪贴板时**不**调用 `guard.suppress`，让监听管线自然捕获：去重、FTS、`clipboard://updated`、来源应用、脱敏判定全部复用，零新增入库逻辑。

副作用（接受）：片段与剪贴板现状内容相同时 OS 可能不发变更事件 → 不产生新记录，与去重语义一致。

### D4 Ctrl/Cmd+C 仲裁放在主窗前端，不动键盘钩子白名单

Windows 上 Ctrl+C 已被 `ctrl_shortcut_key` 白名单吞掉并广播 `keyboard://nav`；macOS 主 panel 是 key window，走真实浏览器 keydown。两个平台最终都落到 `List.tsx::handleKeyDown`，所以在同一个分支里加优先级即可，`settings-window-platform.md` 的 Ctrl 白名单契约完全不受影响。

优先级（在现有分支前插入一条）：主窗原生选区（`shouldUseNativeCopy`，保搜索框复制）→ **预览选区非空 → `writeTextToClipboard(片段)`** → 现有「复制 active item」。

### D5 图片文件预览复用列表卡片的判据，不新增业务规则

`commands/clipboard.rs::attach_file_entries` 已经在算 `FilesPreviewKind::ImagePreview`（单文件 + `is_image` + `exists`）。预览 payload 直接带上这个字段即可让两端语义一致。前端 `ImageViewer` 复用现有灯箱路径，`AssetImage` 已支持任意本地绝对路径（内部 `convertFileSrc`）。

### D6 面板内两条复制入口：整条走命令、片段走新命令

面板里只做「选中片段复制」不够——复制整条内容必须回到主窗用键盘。补一个头部常驻复制按钮后，两条入口职责分明：

| 入口 | 复制内容 | 走哪条命令 | 为什么 |
| --- | --- | --- | --- |
| 头部常驻按钮 | 整条记录 | 既有 `writeToClipboard(id, plain)` | 默认复制格式、`copy_then_hide_window`、复用计数、回环抑制全部与列表「复制」一致，零新增语义 |
| 选区浮动按钮 | 选中片段 | 新 `writeTextToClipboard(text)` | 片段不是既有记录，需要「写剪贴板但期望监听管线入库」的相反语义 |

两条入口都用按钮自身的对勾做反馈并传 `silent`：预览窗与主窗各自持有 antd message 上下文，不静默会在两个窗口各飘一条 toast。

## 3. 契约

### 3.1 事件（Rust 常量 + `src/constants/events.ts` 同步）

| 事件名 | payload | 发出方 | 消费方 |
| --- | --- | --- | --- |
| `preview://pointer` | `{ inside: boolean }` | Rust `preview.rs`（`app.emit` 广播） | 预览页（放大/回缩）、主窗（取消/触发 hover 隐藏缓冲） |
| `preview://selection` | `{ text: string \| null }` | 预览页 `emitTo(WINDOW_LABEL.CLIPBOARD, ...)` | 主窗 `List.tsx`（Ctrl+C 仲裁） |

`preview://selection` 由前端发出、Rust 仅需镜像常量（保持「跨端字面量集中维护」约定）。

### 3.2 命令（`src/constants/commands.ts` + `lib.rs` 注册）

| 命令 | 入参 | 返回 | 说明 |
| --- | --- | --- | --- |
| `write_text_to_clipboard` | `text: string` | `()` | trim 后非空校验 → 写纯文本（不登记 guard）→ 按 `copy_then_hide_window` + pin 决定是否隐藏主窗 |
| `set_clipboard_preview_panel_rect` | `rect: PreviewRect` | `()` | 前端上报面板实测逻辑矩形，供 Rust 换算命中矩形 |
| `set_clipboard_preview_pointer` | `inside: boolean` | `()` | 面板前端 `pointerenter` / `pointerleave` 回报，立即翻转穿透（离开方向零延迟） |

### 3.3 面板命中矩形

- 前端算出的面板 rect 是 **overlay 局部逻辑 CSS px**（与 `layout.overlayRect` 同坐标系）。
- Rust 换算物理坐标：`physical = work_area.position + logical * scale_factor`（`PREVIEW_STATE` 已持有 `work_area` 与 `scale_factor`）。
- 未收到前端上报时回退本次 layout 的 480×480 框，保证 hover 延迟（≥300ms）覆盖上报延迟。
- 命中判定含少量外扩（面板有阴影/边框），避免贴边抖动导致反复翻转。

## 4. 前端交互模型

### 4.1 预览页 `src/pages/Preview/index.tsx`

- 新增 `interactive` state：`preview://pointer {inside:true}` → true，`false` → false。
- `interactive` 时把高度上限覆写为 `overlayRect.height - 2 * PREVIEW_PANEL_MARGIN`，传给 `resolveEffectivePanelSize` / `resolveDynamicPanelRect`；宽度上限不变。
- panelRect 变化且预览激活时上报 `setClipboardPreviewPanelRect`（rAF 节流，随现有 retarget 频率）。
- 面板 `onPointerEnter` / `onPointerLeave` 调 `setClipboardPreviewPointer`；`pointerleave` 同时清空 DOM 选区并向主窗发 `preview://selection { text: null }`。
- 面板 `overflow-hidden` 改为内容区 `overflow-hidden` + 内容滚动容器（`VirtuosoScroller` 已自带滚动），保证 `max-h` 由 style 控制而不是写死 class。
- 右侧/左侧贴边时放大方向天然背向 source（四种 placement 均如此）；靠屏幕边缘时由既有 `clampRect` 收进 overlay inset，极端情况下面板可能视觉盖住主窗（预览窗 z 序本就在上，接受）。

### 4.2 选区与浮动复制按钮

- `PreviewContent` 侧放开选中：纯文本行容器、文件行、Markdown 容器、富文本降级行加 `select-text`（全局根是 `select-none`），内容区 `cursor-text`；选区配色走 antd token 类。
- 新组件 `SelectionCopyButton`：监听 `document.selectionchange`（~150ms 防抖），选区非空且落在面板内时，按 `getRangeAt(0).getBoundingClientRect()` 在选区尾端上方渲染悬浮按钮（fixed 定位 + clamp 进面板）；`onPointerDown` 里 `preventDefault` 保住选区；点击 → `writeTextToClipboard(选中文本)` → 图标变 ✓（约 1s）→ 清选区并隐藏。
- 选区稳定后 `emitTo(WINDOW_LABEL.CLIPBOARD, PREVIEW_SELECTION, { text })`；清空 / 面板失活 / 预览关闭时发 `null`，防陈旧选区劫持 Ctrl+C。

### 4.3 主窗 `src/pages/Clipboard/`

- `useClipboardPreviewController` 订阅 `preview://pointer`：
  - `inside:true` → `cancelHoverHide()`（抵消离开列表区后的关闭缓冲）。
  - `inside:false` → `scheduleHoverHide("previewPanelLeave")`（指针随即落回卡片时，现有 `pointerenter` 取消逻辑自然接住）。
- `List.tsx` 订阅 `preview://selection` 存入 ref（预览关闭时清空），在 Ctrl+C 分支插入 D4 的优先级判断。

### 4.4 图片文件按图片预览

- `ClipboardPreviewPayload` 新增 `files_preview_kind`（复用 `db::models::FilesPreviewKind`，serde camelCase），`files` 分支按既有 `attach_file_entries` 同判据填充；前端类型同步 `filesPreviewKind`。
- `FilesViewer`：`filesPreviewKind === "imagePreview"` 时取 `files[0].path` 走图片渲染分支（与 `ImageViewer` 同构，含灯箱），`onError` 时回退文件行列表。
- 尺寸：该分支不做 Rust 侧解码，交给既有隐藏测量层——`shouldRenderMeasuredContent` 对「files + imagePreview」返回 true，`resolveEffectivePanelSize` 对该分支返回实测尺寸（等同「图片无 DB 尺寸」的既有降级路径）。
- `PREVIEW_FILE_ROW_HEIGHT` 等估算常量仅用于文件行列表分支，不受影响。

## 5. 兼容与回滚

- 不新增设置项、不改数据库、不改已发布设置结构 → 无迁移。
- 若 macOS 真机验证发现面板点击导致主 panel resign key（预览被连带关闭），回滚面可控：仅 `window/preview.rs` 的 macOS 分支（panel style mask / 是否翻转 `ignores_mouse_events`），前端交互层可保留。
- 采样线程、新命令、新事件均为新增；删掉 `set_ignore_cursor_events` 条件化改动即可退回「全穿透」旧行为。

## 6. 风险与对策

| 风险 | 对策 |
| --- | --- |
| 透明 overlay 翻转期间吞掉面板外点击 | 仅指针在面板内时翻转；离开方向由前端事件立即翻回；接受 ≤40ms 极端竞态（见 D2） |
| 面板矩形上报前的命中判定偏差 | Rust 回退 480×480 框；hover 延迟 ≥300ms 足以覆盖上报 |
| 采样线程开销 | 预览不可见时降频空转；tick 内只做一次坐标查询 + 一次矩形比较 |
| 翻转与点击的亚毫秒竞态（跨界瞬间点击） | 接受：指针必然先移动到位才可能点到面板 |
| 相同片段复制不产生新记录 | 与去重语义一致，接受 |
| webview 不支持的图片格式 | `onError` 回退文件行列表 |
| macOS 分支本机不可编译验证 | `cargo check --target aarch64-apple-darwin` 在本机依赖构建脚本阶段就因缺 C 工具链失败，**实际未能编译验证**；改为按 crate 源码逐个核对 API 形状（过程中修正了 2 处凭印象写错的用法），并把坐标换算抽成纯函数 + 单测；真机行为列入待人工验证清单 |
| 面板外圈「采样在面板内、DOM 在面板外」的死区 | 命中矩形外扩压到 4px；`report_pointer_inside(false)` 走同一套命中判定而非直接置穿透，两个真相源收口到一处 |
| **点击面板让主窗失焦 → 预览被自己关掉**（实测踩中） | 点击可交互的兄弟 overlay 会让主窗 `window.blur`；`handleWindowBlur` / `handleWindowResize` 加「指针在面板上则不关」守卫。诊断靠 `preview close requested: reason=…` 日志定位 |

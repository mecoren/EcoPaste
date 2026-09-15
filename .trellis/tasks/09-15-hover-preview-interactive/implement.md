# Implement: 悬浮预览可交互

> 状态：代码完成，自动化校验全绿；**Windows 真机手动走查已通过（用户确认）**；macOS 真机行为待人工确认（见文末）。

## S1 Rust: 预览命中矩形 + 指针状态 + 采样线程 ✅

- `src-tauri/src/window/preview.rs`：
  - `pub const PREVIEW_POINTER_EVENT`、`PREVIEW_PANEL_RECT`、`PREVIEW_HIT_BOUNDS`（物理像素缓存）、`PREVIEW_VISIBLE`、`PREVIEW_POINTER_INSIDE`、`PREVIEW_POINTER_WATCHER`。
  - `update_panel_rect` / `panel_contains_physical_point` / `report_pointer_inside` / `apply_pointer_inside` / `refresh_hit_bounds` / `resolve_hit_bounds` / `physical_bounds` / `is_valid_rect`。
  - 采样线程 `ensure_pointer_watcher`（预览可见时 40ms、不可见时 200ms 空转）。
  - 单测：`physical_bounds_*`、`hit_bounds_use_half_open_range`、`zero_scale_factor_falls_back_to_one`、`rejects_non_finite_or_degenerate_rects`。

## S2 Rust: 平台光标查询 ✅

- Windows：`GetCursorPos`（已是物理像素）。
- macOS：`CGEventSource::new(CombinedSessionState)` + `CGEvent::new(source).location()`（Quartz 点是主显示器左上原点、y 向下），经 `screen_points_to_physical` 换算成物理像素。
- **API 形状按 crate 源码逐个核对**（本机无法交叉编译 macOS）：`core_graphics::event::CGEvent::{new, location}`、`core_graphics::event_source::{CGEventSource, CGEventSourceStateID}`、`CGPoint{x,y: CGFloat}`。过程中修正了两处先前凭印象写错的用法（`get_location` → `location`、`CGEventSource` 所属模块）。

## S3 Rust: 条件化穿透 + macOS panel 语义 ✅

- `prepare_preview_window_for_show` → `sync_pointer_for_show`：新一次 show 先按穿透呈现并复位指针态；retarget（此前可见）改用完整校对，避免把正在面板上拖选的用户瞬间打回穿透。
- 三处 macOS `set_ignores_mouse_events(true)` 全部移除（统一由 `apply_pointer_inside` 收口，并在该函数里对 NSPanel 追加一次同义设置）。
- `setup_macos_preview_panel` 补 `set_style_mask(StyleMask::empty().nonactivating_panel())`：面板可交互但不激活 App、不抢 key。
- 建窗补 `.accept_first_mouse(true)`。

## S4 Rust: 纯文本片段写回 ✅

- `clipboard/write.rs::write_plain_text(text)`：只写纯文本，**不**登记 `WritebackGuard`（期望监听管线入库）。
- `clipboard/mod.rs` 导出。

## S5 Rust: 新命令 + 注册 ✅

- `commands/clipboard.rs::write_text_to_clipboard(app, text)`：trim 非空校验 → 写剪贴板 → 按 `copy_then_hide_window` 复用 `hide_clipboard_window_after_copy`。
- `commands/window.rs::set_clipboard_preview_panel_rect` / `set_clipboard_preview_pointer`；re-export `PreviewRect`。
- `lib.rs` 注册三个命令。

## S6 Rust: Windows 鼠标钩子面板豁免 ✅

- `mouse/windows.rs`：button-down 判定链在 `cursor_outside_clipboard_window` 前加 `!preview_panel_contains(cursor)`；未引入 `WM_MOUSEMOVE` 分支（指针跟踪交给采样线程）。

## S7 前端: 常量与命令包装 ✅

- `src/constants/events.ts`：`PREVIEW_POINTER`、`PREVIEW_SELECTION`。
- `src/constants/commands.ts`：三个新命令常量。
- `src/commands/index.ts`：`writeTextToClipboard(text, { silent })`、`setClipboardPreviewPanelRect`、`setClipboardPreviewPointer`；`ClipboardPreviewPayload` 补 `filesPreviewKind`。

## S8 前端: 预览页交互层 ✅

- `layout.ts`：三个尺寸估算函数增加 `maxHeight`，新增 `resolvePanelHeightCap`（取入参上限与 overlay inset 的较小值，替代原先基于固定 480 框的 clamp）。
- `index.tsx`：`interactive` 态、`PREVIEW_POINTER` 订阅、放大上限、面板矩形上报（按字段依赖避免每帧 IPC）、面板 `pointerenter/pointerleave/pointerup` 回报、内容区 `select-text cursor-text`、面板去掉写死的 `max-h-120`。

## S9 前端: 选区与浮动复制按钮 ✅

- 新 `src/pages/Preview/selection.ts`：`readSelectionText` / `isSelectionInside` / `clearSelection` / `publishSelection`。
- 新 `src/pages/Preview/components/SelectionCopyButton.tsx`：`selectionchange` 防抖 150ms、按钮定位（选区尾端上方 / 贴顶改放下方 / 横向收敛进面板）、`pointerdown` preventDefault 保选区、复制 → ✓ 1s → 清选区、跨窗广播选区。
- i18n：`preview.json`（zh-CN / en-US）补 `selection.copy` / `selection.copied`。

## S10 前端: 图片文件按图片预览 ✅

- Rust `ClipboardPreviewPayload` 增 `files_preview_kind`（与 `attach_file_entries` 同判据；`resolve_preview_files_kind`）。
- `PreviewContent.tsx`：抽出 `ImageStage`（封面 + 灯箱），`ImageViewer` 与「单图文件」共用；`FilesViewer` 命中 `imagePreview` 且存在时走图片分支，`onError` 回退文件行列表。
- `index.tsx`：`shouldRenderMeasuredContent` 对「files + imagePreview」返回 true；`layout.ts` 对该分支返回实测尺寸。

## S11 前端: 主窗协同 ✅

- `useClipboardPreviewController.ts`：订阅 `PREVIEW_POINTER`；新增 `panelPointerInsideRef`，`scheduleHoverHide` 在指针停在面板上时不再安排关闭。
- `List.tsx`：订阅 `PREVIEW_SELECTION` 存 ref；预览会话结束时清空；Ctrl+C 分支插入「有选中片段 → 复制片段」优先级。

## S13 前端: 面板头部常驻复制按钮 ✅

需求补充（「预览要支持鼠标复制」）落地：面板里原先只有「选中后浮现的浮动按钮」，
复制**整条**内容必须用键盘；补一个头部常驻复制按钮，让预览框内可以纯鼠标完成复制。

- `src/pages/Preview/components/PreviewContent.tsx::PreviewHeader`：右侧类型徽标旁加复制按钮（`size-6`，与徽标同组），点击 → `writeToClipboard(payload.id, false, { silent: true })` → ✓ 1s 后复位。
- `src/commands/index.ts`：`writeToClipboard` 补 `options?: { silent?: boolean }`。
- `src/pages/Preview/constants.ts`：`PREVIEW_COPY_FEEDBACK_MS` 抽出，头部按钮与选区浮动按钮共用。
- i18n：`preview.json`（zh-CN / en-US）补 `copy.item` / `copy.itemCopied`。

## S14 修复：点击面板时主窗失焦把预览自己关掉 ✅

**现象**：面板已经可交互（能放大、能收到鼠标事件），但**左键点一下面板预览就消失**，主窗仍在。

**定位过程（日志直接给出结论）**：加了诊断日志后，`%LOCALAPPDATA%\com.ayangweb.eco-paste\logs\EcoPaste.log` 出现决定性一行：

```
preview panel rect reported: logical=…{height: 1016}      ← 面板已放大到 overlay 可用高度，interactive 生效
preview panel pointer report: inside=true                 ← 面板确实收到鼠标事件，穿透翻转生效
preview close requested: reason=windowBlur, pointer_inside=true   ← 关闭来自「主窗失焦」
```

即：指针链路全通，问题出在**点击面板会让主窗 `window.blur`**（焦点被面板所在的 overlay 拿走），而 `handleWindowBlur` 无条件 `closePreview("windowBlur")`——自己点自己的 UI 被判成了「用户离开」。

**修复**（`useClipboardPreviewController.ts`）：
- `handleWindowBlur`：指针在面板上时不关（`panelPointerInsideRef` 守卫）。
- `handleWindowResize`：同样加指针守卫（点击面板会带一次样式/焦点扰动，可能连带产生 resize 事件；指针离开后关闭缓冲会自然收掉）。

**保留诊断日志**：`close requested: reason=…` 一行把「预览为什么自己关了」变成可回溯信息，本次就是靠它定位的。

**复验（第二次会话）**：指针在面板上停留 6 秒、期间无 `windowBlur`，直到主动移开才按 `previewPanelLeave` 正常关闭 —— 点击不再自杀。

**顺带补齐日志盲区**（此前「复制片段」整条链路一条日志都没有，出问题只能靠猜）：
- 前端 `log.*`（走 `tauri-plugin-log`，与 Rust 落同一文件）：选区发布（`preview selection publish`）、浮动按钮复制成功/失败（`preview selection copied by button`）、Ctrl+C 走片段分支（`ctrl+c copied preview selection`）、失焦/尺寸被守卫忽略（`preview blur|resize ignored`）。
- Rust `write_text_to_clipboard` 写入成功后记 `clipboard fragment written: chars=N` —— 这是「命令是否真的执行」的权威信号。
- 三者串起来即可判定：选区有没有被识别 → 复制命令有没有跑 → 剪贴板有没有被写（第 3 步之后的新记录由既有 watcher 管线负责，`write_plain_text` 不登记回环抑制）。

## S12 全量校验 ✅ / Windows 手动走查 ✅

- `cargo fmt` ✅ / `cargo clippy --all-targets -- -D warnings` ✅ / `cargo test` ✅（219 passed, 8 ignored）。
- `pnpm lint` ✅ / `pnpm tsc` ✅ / `vite build` ✅。
- **Windows 真机手动走查 ✅（用户确认）**：悬停出预览 → 移入面板保持并放大到整屏可用高度 → 滚轮滚完全部内容 → 拖选文字 → 浮动按钮 / 头部按钮 / Ctrl+C 复制 → 列表顶部出现新记录 → 移出面板按 240ms 缓冲自动关闭 → 点击面板不再关闭预览。
- 日志侧证据：`preview close requested: reason=…` 在正常路径只出现 `previewPanelLeave` / `windowHidden`，不再出现 `windowBlur`。

## 相对计划的设计偏差（实现期收敛）

1. **采样线程只负责「进入」方向**（`reconcile_pointer(app, enter_only)`）。离开方向完全由面板前端的 `pointerleave` 驱动：采样若也负责离开，拖选滑出面板边缘会被中途打回穿透。显式布局变化（retarget）用完整校对。
2. **拖选保护改在 DOM 侧**：`pointerleave` 在 `event.buttons !== 0` 时不回报；`pointerup` 再回报一次，由 Rust 按真实光标位置重判（避免「松手在面板外」把面板永久卡在可交互态）。原计划的「Rust 侧按住保持」因需要额外的平台按键状态 API（core-graphics 0.25 未提供 `button_state`）而放弃。
3. **命中矩形外扩从 8px 收到 4px**：外扩过大时会在面板外圈留下一段「采样认为在面板内、DOM 认为在面板外」的死区。
4. **预览窗内的复制静默化**（`silent: true`）：预览窗与主窗共用 antd message 上下文，不静默会在两个窗口各飘一条 toast；按钮自身的 ✓ 已是反馈。
5. **浮动复制按钮渲染在面板之外**：面板被 motion 加了 transform，`fixed` 子元素会以面板为包含块导致定位随动画漂移。

## 待人工验证

- Windows 手动矩阵（prd 验收项逐条走查），含面板头部复制按钮的纯鼠标路径。
- macOS 真机：本机无 C 工具链，`cargo check --target aarch64-apple-darwin` 在依赖构建脚本阶段即失败，macOS 分支**未经编译验证**（已按 crate 源码核对 API 形状）。重点确认：面板可交互后点击是否让主 panel resign key（面板已设 `nonactivating_panel`，预期不会）、穿透翻转的观感与 40ms 采样延迟、Quartz 点 → 物理像素换算在多屏 / 非整数缩放下的正确性。
- 本机合成输入限制（锁屏残留 / UIA 劫持）：UI 自动化不稳时以人工点验为准。

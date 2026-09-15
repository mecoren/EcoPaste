# 悬浮预览可交互：预览全部内容、拖选复制片段、图片文件按图片预览

## Goal

让剪贴板悬浮预览从「只读、不可操作」的展示层，变成「可停留、可滚动、可选中」的交互层：

1. 鼠标移入预览面板时预览不关闭，面板放大到接近屏幕可用高度，超长内容可滚动看到全部。
2. 纯文本 / Markdown / 文件路径文本支持鼠标拖选。
3. 选中片段后可复制（快捷键 + 浮动按钮），片段作为新记录进入历史列表。
4. 剪贴板里是图片文件（png / jpg 等）时，预览面板直接按图片渲染（并可点击放大灯箱），而不是只显示一行文件条目。

## 现状（已核验）

- 预览窗是覆盖整个显示器的独立透明 overlay（`clipboard-preview` → `#/preview`）；Rust 每次 show 前强制 `set_ignore_cursor_events(true)`（`window/preview.rs` 的 `prepare_preview_window_for_show`），macOS 侧三处 `panel.set_ignores_mouse_events(true)`，建窗参数还带 `focusable(false)`。→ 面板对鼠标完全穿透：指针移入面板约 240ms（`HOVER_HIDE_BUFFER_MS`）后预览就关，内容超 480×480 上限完全看不到，文本不可选；连现有的图片灯箱、Markdown 切换按钮实际也点不到。
- 前端预览面板写死 480×480 上限（`PREVIEW_PANEL_MAX_HEIGHT/WIDTH`），面板 `overflow-hidden`，根节点全局 `select-none`。
- Windows 主窗靠 `WH_MOUSE_LL`（`mouse/windows.rs`）按「光标是否在主窗矩形内」判定外部点击隐藏；右键菜单窗已有「矩形内不算外部点击」的豁免先例（`menu/context_window.rs::contains_physical_point`）。预览面板矩形不在任何豁免集合里。
- Windows 键盘走 `keyboard://nav` 广播；Ctrl+C 已在 `ctrl_shortcut_key` 白名单内（0x43）。
- macOS 主 panel 靠 `window_did_resign_key` 隐藏（`window/macos.rs`）；预览 panel `can_become_key_window: false`。
- 现有写回命令只能按记录 id 整条写回（`write_to_clipboard` / `paste_clipboard_item`，带 `WritebackGuard` 回环抑制），没有「写任意文本」的命令。
- 列表卡片已有「单图文件按图片渲染」语义：Rust 在 `attach_file_entries` 里算出 `FilesPreviewKind::ImagePreview`（单文件 + 是图片 + 存在），`FilesCard` 据此走 `ImageCard`；但预览 payload 不含这个字段，预览面板的 `FilesViewer` 只渲染文件行。

## Requirements

### R1 面板可停留 + 放大

- 指针进入预览面板矩形时：预览保持显示（不被 240ms 缓冲收起），面板高度上限放大到 `overlayRect.height - 2 * PREVIEW_PANEL_MARGIN`（宽度上限仍 480），内容超出时面板内滚动。
- 指针离开面板矩形时：恢复原 480 上限，并回到现有的缓冲关闭语义（离开后 240ms 内回到卡片则不关）。
- 放大/回缩走同一套 spring 动画，不闪跳。

### R2 面板内鼠标交互

- 面板内可滚动（滚轮 / 触控板）。
- 纯文本（含 Markdown 原文视图）、文件路径行可拖选；富文本 iframe 内不要求（见 R6）。
- 指针在面板内时，点击面板不隐藏剪贴板主窗口，也不收起预览。

### R3 面板内可复制（鼠标 + 快捷键全覆盖）

- **整条复制（鼠标）**：预览面板头部常驻一个复制按钮，点一下把整条记录写回剪贴板（与列表里的「复制」同一条命令，默认复制格式 / 复制后隐藏窗口 / 复用计数设置全部复用）。
- **片段复制（鼠标）**：选中片段后，在选区旁浮现一个小复制按钮，点击即复制；复制成功后短暂反馈，然后清空选区并隐藏按钮。
- **快捷键**：有选中片段时 Ctrl/Cmd+C 复制片段；无选中时保持现有「复制整条记录」语义。
- 面板内的复制一律用按钮自身的对勾做反馈，不弹 antd toast（预览窗与主窗各自持有 message 上下文，两个窗口同时飘一条会显重复）。
- 片段写回后：**作为新记录进入历史列表**（写系统剪贴板、不登记回环抑制，让 OS 监听管线自然入库，复用去重 / 搜索 / `clipboard://updated` 语义）。
- 复制后是否隐藏窗口跟随既有 `copy_then_hide_window` 设置与 pin 状态（与「复制整条」一致）。

### R4 图片文件按图片预览

- `files` 类型记录若命中「单文件 + 是图片 + 文件存在」（与列表卡片同判据），预览面板直接渲染图片，而非文件行列表。
- 图片可点击展开灯箱（复用现有 `image` 类型的 antd Image 灯箱路径）。
- 面板尺寸按图片自然比例计算，超出上限时缩放。
- 若 webview 无法解码该格式（heic / tiff / svg 等），或文件读取失败，自动回退为文件行列表。

### R5 与键盘预览（按住 Space）共存

- 按住 Space 的键盘预览会话不受影响：面板上仍可交互、可滚动、可选中；Space 松开仍按现有语义关闭。

### R6 已知限制（明确不做）

- 富文本 iframe（`sandbox=""`，无 `allow-same-origin`，安全取舍）内不做拖选复制；整条复制仍可用。
- 右键菜单不加「复制选中」；不新增设置开关（跟随现有 hover / space 预览开关）。
- 不动数据库 schema、不动已发布设置契约、不动 Windows Ctrl 白名单。

## Acceptance Criteria

- [ ] 悬停出预览 → 指针移入面板：预览保持显示，面板放大且带动画，超长文本可滚动看到末尾。
- [ ] 指针移出面板：面板回缩到 480 上限；240ms 内回到卡片不关闭，彻底离开则关闭。
- [ ] 点击面板内任意位置：主窗口不隐藏、预览不收起。
- [ ] 点击面板外：主窗口隐藏并收起预览（Windows 钩子判定）。
- [ ] 纯文本 / Markdown 原文 / 文件路径可拖选高亮，选区颜色符合主题 token。
- [ ] 选中后出现浮动复制按钮；点击复制 → 剪贴板内容等于片段 → 列表顶部出现新记录 → 按钮变对勾后消失。
- [ ] 预览面板头部复制按钮：点一下复制整条记录（纯鼠标，无需键盘），按钮变对勾后复位，不弹 toast。
- [ ] 有选中时 Ctrl/Cmd+C 复制片段；无选中时 Ctrl/Cmd+C 仍复制整条记录；搜索框内 Ctrl/Cmd+C 仍是原生复制。
- [ ] 剪贴板里复制一个 png / jpg 文件（单文件）→ 预览面板按图片渲染并可点开灯箱。
- [ ] 单文件但不是图片（如 .zip）、或多文件混合、或文件已删除 → 仍是文件行列表。
- [ ] 按住 Space 的键盘预览期间，面板同样可交互。
- [ ] `cargo fmt && cargo clippy -- -D warnings && cargo test` 通过。
- [ ] `pnpm lint` + 前端类型检查 / 构建通过。

## Notes

- 验收以 Windows 本机手动走查为主（macOS 侧本机不可验，先保证类型检查与代码路径完整，真机行为留待后续人工验证）。
- 本机合成输入存在已知限制（锁屏残留 / UIA 劫持），UI 自动化不稳时以人工点验 + Rust 单测为准。

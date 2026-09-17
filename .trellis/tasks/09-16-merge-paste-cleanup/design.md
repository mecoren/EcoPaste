# Design: 合并粘贴与粘贴文本清理

## Boundaries

- Rust 拥有：拼接语义、transform 字符串处理、写回剪贴板、回环抑制登记、use_count 累加、粘贴时序（hide/50ms/pinned 恢复）。
- 前端拥有：按钮/菜单渲染、选中排序、快捷键分发、toast 文案、设置镜像。
- 复用：`paste_clipboard_item`（`src-tauri/src/commands/clipboard.rs:420-481`）的隐藏窗口/时序/pinned 恢复序列抽成共享函数；`write_to_clipboard` 写回；`mark_item_reused_if_enabled` 计数。

## Contracts

- 新命令 `paste_clipboard_items(ids: Vec<i64>, separator: String, plain: bool)`：
  - 按传入序（前端已按列表显示序排好）取条目；仅文本类参与，非文本直接跳过（前端已拦截，后端再兜底）。
  - 取 `search_text.unwrap_or(content)` 拼接（富文本取纯文本表示）。
  - 构造合成串走纯文本写回；`guard.suppress` 登记合成串哈希 → 合并不进历史。
  - 逐条 `mark_item_reused_if_enabled` 累加 use_count。
- `paste_clipboard_item` 扩参 `transform: Option<PasteTransform>`：
  - `enum PasteTransform { StripNewlines, TrimLines, TrimWhitespace, UpperCase, LowerCase }`，Rust 纯字符串处理，仅 Text 类。
  - transform 隐含纯文本写回，且抑制变换后哈希。
- 设置 `clipboard.content.merge_paste_separator`：`newline|space|none|comma`，默认 `newline`，serde default 回落。
- 命令/事件字面量走 Rust 常量 + `src/constants/` 同步（AGENTS 跨端契约）。

## Data Flow

- 合并：前端排序 ids → `paste_clipboard_items` → 拼接 → write → suppress 合成哈希 → hide/paste/pinned 恢复 → 前端 toast。
- 清理：QuickActions 菜单/`Ctrl+Shift+Enter` → `paste_clipboard_item(id, transform)` → 变换 → 写回 → 抑制变换后哈希 → 同粘贴序列。

## Tradeoffs

- 合并走内存合成不落库：对齐 Ditto，零 migration，代价是合并串不可回溯（符合需求）。
- transform 放 Rust 而非前端：保证快捷键/按钮/命令三路语义一致，且抑制哈希口径统一。

## Compatibility / Rollback

- 新增命令参数均为 Option/新增命令，老前端调旧命令不受影响。
- 新增设置字段缺失时回落默认换行；回滚只需 revert 命令与前端调用。

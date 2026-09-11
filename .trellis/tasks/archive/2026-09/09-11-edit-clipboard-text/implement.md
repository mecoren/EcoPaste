# 实施计划：剪贴板文本条目内容编辑

前置：按顺序执行；每个阶段末尾有验证命令（在 `src-tauri/` 与仓库根分别执行）。回滚点 = 每个阶段完成后 git 可暂存；最终提交为单条 `feat: support editing text clipboard item content`。

## 阶段 1：Rust 侧（db + 命令）

1. `src-tauri/src/db/items.rs`：新增 `update_item_text(pool, id, content)`。
   - UPDATE `content, content_hash, search_text, summary, sub_kind, size`；不触碰 `updated_at` 等。
   - summary/sub_kind/size/hash 在仓储内计算（`make_summary` 规则从 ingest 导出复用：把 `make_summary` 与 `SUMMARY_MAX_CHARS` 改 pub(crate)；`detect_text_sub_kind` 已 pub；`content_hash` 已 pub）。
   - 附单元测试（见 design.md 测试设计）。
2. `src-tauri/src/clipboard/ingest.rs`：`make_summary`、`SUMMARY_MAX_CHARS` 改为 `pub(crate)`；`exceeds_limit` 改 `pub(crate)`。
3. `src-tauri/src/commands/clipboard.rs`：
   - `compute_available_actions`：text 时在 EditNote 前插 `EditContent`。
   - 新命令 `get_clipboard_item_edit_text(id)`：`find_item_by_id` → html/rtf 用 search_text，否则 content；返回 `Option<String>`。
   - 新命令 `update_clipboard_item_text(app, db, stores..., id, content)`：校验（存在/kind/空白/大小）→ `update_item_text` → 返回 enrich 后的列表视图条目。
   - 把 `get_clipboard_item` 的 enrich 主体抽为 `enrich_list_item(...)` 私有 helper 复用（保持行为零变化：抽完跑既有测试）。
   - 命令层测试：非 text、空白校验。
4. `src-tauri/src/lib.rs`：注册两个新命令；`commands/mod.rs` 导出。
5. `src-tauri/src/menu/clipboard_item.rs`：`ClipboardMenuAction::EditContent` + label + accelerator(`CmdOrCtrl+E`) + macOS id 表 + ACTION_GROUPS（EditNote 前一位）。
6. `src-tauri/src/i18n/keys.rs` + `zh_cn/clipboard_menu.rs` + `en_us/clipboard_menu.rs`：`EditContent` key 与双语文案。

验证：`cargo fmt && cargo clippy -- -D warnings && cargo test`（在 src-tauri/）。

## 阶段 2：前端侧

1. `src/constants/commands.ts`：`UPDATE_CLIPBOARD_ITEM_TEXT`、`GET_CLIPBOARD_ITEM_EDIT_TEXT`。
2. `src/types/clipboard.ts`：`ClipboardAction` 加 `"editContent"`。
3. `src/commands/index.ts`：`updateClipboardItemText`（label `commands:labels.saveText`，成功 toast `commands:messages.textSaved`，返回 `ClipboardItem`）、`getClipboardItemEditText`。
4. 新组件 `src/pages/Clipboard/components/EditModal.tsx`（对照 NoteModal）：
   - 打开时 `getClipboardItemEditText(item.id)` 拉源文本；html/rtf 显示转纯文本提示 Alert。
   - TextArea autoSize `{minRows: 4, maxRows: 12}`；trim 空白禁用 ok；afterOpenChange rAF 聚焦末尾。
   - 保存 → `onSaved(item.id, updated)` → `onClose()`。
5. `src/pages/Clipboard/components/List.tsx`：
   - `editTarget` state、`handleOpenEdit`（closePreview 同款）、`handleEditSaved`（patchItemById 整体替换）、`handleCloseEdit`。
   - menu-action switch 加 `case "editContent"`。
   - `handleKeyDown` 加 `CmdOrCtrl+E` 分支（activeItem 且 kind=text）。
   - 渲染 `<EditModal />`。

验证：`pnpm lint` + `pnpm tsc --noEmit`（或项目等价 type-check 命令）。

## 阶段 3：i18n 文案

1. `src/locales/zh-CN/clipboard.json` + `en-US/clipboard.json`：`edit` 段（title/placeholder/richTextNotice/emptyHint）+ shortcuts 段补 `editSelected`。
2. `src/locales/zh-CN/commands.json` + `en-US/commands.json`：`labels.saveText`、`labels.getEditText`、`messages.textSaved`。

验证：`pnpm lint`（含 i18n key 完整性检查若有）。

## 阶段 4：操作验证（Windows 本机）

- `pnpm tauri dev` 启动；复制一段文本 → 右键「编辑内容」→ 修改 → 保存 → 卡片摘要即时更新。
- Cmd/Ctrl+E 快捷键路径；image/files 条目菜单无该项。
- 富文本（从 Word/浏览器带格式复制）编辑 → 提示可见 → 保存后 subKind 变化（类型标签从 HTML 变 Text/URL）。
- 改成 URL → 右键出现「打开链接」；搜索新关键词命中。
- 空白保存禁用；收藏/置顶/备注/分组在编辑后保持。
- 敏感条目（如密码复制）可编辑、保存。

## 阶段 5：收尾

- trellis-check 全量质量检查。
- spec 更新（若有值得沉淀的契约：如「编辑走 UPDATE 保 FTS 触发器」「updated_at 语义」已覆盖则不重复写）。
- 提交（单行 Conventional Commits）。

## 回滚点

- 阶段 1/2/3 各自独立可 revert；无数据迁移风险（无 schema/设置变更）。

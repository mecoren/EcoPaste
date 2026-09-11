# 技术设计：剪贴板文本条目内容编辑

## 数据流总览

```text
右键菜单「编辑内容」/ CmdOrCtrl+E (List.tsx)
  -> EditModal 打开（getClipboardItemPreviewText 按 id 拉完整文本）
  -> 用户编辑，确定
  -> updateClipboardItemText(id, text)  [src/commands/index.ts 包装]
  -> Rust update_clipboard_item_text 命令
     - 校验：存在、kind=text、trim 非空、大小上限
     - db::items::update_item_text：单条 UPDATE（content/search_text/summary/sub_kind/content_hash/size）
     - FTS 由既有 clipboard_items_au 触发器自动同步
     - 返回列表视图条目（复用 enrich 管线）
  -> 前端 toast + patchItemById 回填本地镜像（不 emit clipboard://updated）
```

## 边界与契约

### 为什么必须走 UPDATE（不能 DELETE+INSERT）

- `clipboard_items_au` AFTER UPDATE 触发器负责把 `search_text`/`note` 的变更同步进 FTS5 表；DELETE+INSERT 虽然理论上也走 `clipboard_items_ad`/`ai` 触发器，但会丢失行 id、created_at、use_count 等语义，破坏「就地编辑」的产品承诺。所以编辑只做一条 UPDATE。
- FTS 触发器读的是 `new.search_text`/`new.note`，因此 UPDATE 语句必须同时写入新的 `search_text`，这是搜索能命中新内容的唯一机制。

### 字段更新矩阵（`update_item_text`）

| 字段 | 更新为 | 说明 |
| --- | --- | --- |
| `content` | 新纯文本 | 富文本条目保存后即为纯文本，`sub_kind` 同步重识别 |
| `search_text` | 新纯文本 | FTS 索引源；与 content 同串（plain 语义） |
| `summary` | trim 后前 256 chars（Unicode 标量） | 复用 `ingest::make_summary` 规则，需从 ingest 模块导出复用 |
| `sub_kind` | `detect_text_sub_kind(new)` | url/email/color/path 重识别；html/rtf 归 None 或识别结果 |
| `content_hash` | `content_hash(Text, new)` | 与 upsert 去重同一指纹函数 |
| `size` | `new.len()`（UTF-8 字节） | 与 `count_text_bytes` 语义一致 |
| 其余字段 | 不动 | use_count / favorite / pinned / note / group_id / created_at / updated_at / width / height / file_types / platform / source_app_id / is_sensitive |

`updated_at` 不刷新：该时间戳语义是「内容被重新使用」（最近使用排序），编辑属于元数据级修改，与 note/group 语义对齐（spec: database-and-storage.md 的 Timestamp Semantics 节明确要求）。

### 去重与哈希语义

- 编辑后 `content_hash` 指向新指纹。若库里已有同指纹记录，**两条共存**：upsert 去重只发生在「复制进监听管线」时，编辑路径不做合并。这与 Ditto 行为一致（edit 后是一条独立修改过的记录），PRD 已确认为预期。
- 未来同内容再被复制时，`upsert_item` 会命中新指纹那条并 use_count+1——可接受（最近编辑过的条目获得使用计数，符合直觉）。

### 校验规则（命令层，`commands/clipboard.rs`）

1. `find_item_by_id` → None：`AppError::Clipboard("剪贴板记录不存在")` 风格错误（对齐 `open_clipboard_item_link` 的现有写法）。
2. `kind != Text`：返回「仅支持编辑文本内容」。
3. `content.trim().is_empty()`：返回「内容不能为空」。
4. `exceeds_limit(content.len(), capture.max_text_bytes())`：返回「内容大小超出上限」。`exceeds_limit` 目前是 ingest.rs 私有函数，导出复用（`pub(crate)`）。

### 返回值：列表视图条目

命令返回 `Option<ClipboardItem>`（列表视图裁剪 + 全量 enrich：source app icon / displayCreatedAt / availableActions / colorPreview / redact），**直接复用 `get_clipboard_item` 命令的内部逻辑**。为避免两处维护 enrich 管线，把 `get_clipboard_item` 的 enrich 主体抽成私有 helper `enrich_list_item(app, pool, stores..., item)`，两个命令共用。

前端拿到后整体替换本地镜像中该 id 的条目（比逐字段 patch 更简单、不会漏掉 summary/subKind/colorPreview 等派生字段）。`patchItemById` 本身就是浅合并 patch，传完整对象即可。

### 动作枚举扩展（跨端契约，全部同步改）

`ClipboardAction`（`db/models.rs`，serde camelCase）新增 `EditContent`：

- `compute_available_actions`（commands/clipboard.rs）：`kind == Text` 时在 `EditNote` 前插入 `EditContent`。
- `ClipboardMenuAction`（menu/clipboard_item.rs，与前端 ClipboardAction 对齐）：新增 `EditContent` 变体 + macOS muda id `cim::editContent` + `from_id` 表 + `label`（新增 `ClipboardMenuKey::EditContent`）+ accelerator `CmdOrCtrl+E`。
- `ACTION_GROUPS`：放入 `[ToggleFavorite, TogglePinned, MoveToGroup, EditContent, EditNote]` 组（EditContent 在 EditNote 前）。macOS 与 Windows（context_window 复用同一 ACTION_GROUPS/build_groups）自动获得。
- 前端 `ClipboardAction`（src/types/clipboard.ts）加 `"editContent"`。
- Windows 菜单窗前端（ContextMenu/index.tsx）按 `item.action` 泛型渲染，无需为 editContent 单独写 UI。

### 敏感条目编辑

`is_sensitive` 保持不变。编辑明文可见（本机数据库本就存明文，预览也仅在 redact 开启时遮罩），保存后如果新文本仍含敏感特征也不重新检测——标记语义是「入库时的判定」，编辑不推翻（重检测会导致用户微调一个字符后标记意外翻转）。列表视图的脱敏遮罩由 enrich 管线基于 `is_sensitive` 现算，自动生效。

## 前端设计

### EditModal（`src/pages/Clipboard/components/EditModal.tsx`，对照 NoteModal）

- Props：`item: ClipboardItem | null`、`onClose`、`onSaved(id, updated: ClipboardItem)`。
- 打开（item 变化）时异步拉取编辑文本源：
  - subKind 为 html/rtf：编辑 `searchText`（预览同款语义），弹窗内展示 `Alert`（info）：「保存后将转为纯文本，格式样式不会被保留」（仅这两种 subKind 显示）。
  - 其余 text 条目：编辑 `content`。
  - 拉取方式：新增命令 `get_clipboard_item_edit_text(id)` 返回 `Option<String>`（Rust 侧：`find_item_by_id` 取完整行，按 subKind 选择 content 或 search_text；None → 前端不打开弹窗）。不复用 `get_clipboard_preview_payload`（结构面向预览窗，字段冗余）也不复用 `get_clipboard_item`（text 的 content 已被置空）。
- 保存：`await updateClipboardItemText(item.id, value)` → `onSaved(item.id, updated)` → `onClose()`；`confirmLoading` 防重。
- 空白校验：`value.trim().length === 0` 时 okButtonProps.disabled（后端兜底）。
- TextArea autoSize `{minRows: 4, maxRows: 12}`，无 maxLength（超大由后端上限校验）；打开后 `afterOpenChange` 聚焦光标至末尾（NoteModal 同款 rAF 方案）。

### List.tsx 接线

- 新增 `editTarget` state + `handleOpenEdit(item, reason)`（先 closePreview 再 setEditTarget，与 handleOpenNote 同款）。
- `handleMenuActionRef` switch 新增 `case "editContent"`。
- `handleKeyDown` 新增 `CmdOrCtrl+E` 分支：activeItem 存在且 `item.kind === "text"` 才打开。
- `handleEditSaved(id, updated)`：`patchItemById(id, updated)`（完整对象浅合并，等价整体替换）。
- 渲染 `<EditModal item={editTarget} onClose={handleCloseEdit} onSaved={handleEditSaved} />`。

### 命令封装（src/commands/index.ts）

- `updateClipboardItemText(id, text)`：call → label `commands:labels.saveText` → toast `commands:messages.textSaved`；返回 `ClipboardItem`（完整更新后条目）。
- `getClipboardItemEditText(id)`：call → label 复用现有 `commands:labels.getClipboardItem`（若已有）或新增 `getEditText`；失败静默需求不强，走标准 call 即可。

### 类型（src/types/clipboard.ts）

- `ClipboardAction` 加 `"editContent"`。

## 兼容性 / 回滚

- 无 schema 变更、无设置变更、无事件变更；旧前端 + 新后端（或反之）仅在 `editContent` 动作上不匹配，本项目前后端同包发布，无实际风险。
- 回滚 = revert 提交，无数据残留问题（UPDATE 只改了既有行的既有字段）。
- 备份/恢复：`.ecopastebak` 复制整库，编辑后的条目天然被包含。

## 测试设计

### Rust 单元测试（db::items，memory_pool）

- `update_item_text_updates_derived_fields`：插入 plain 文本 → 更新 → content/search_text/summary/sub_kind/hash/size 全部按预期，其余字段（note/favorite/pinned/use_count/created_at/updated_at）不变。
- `update_item_text_reidentifies_sub_kind`：普通文本 → 改为 URL → sub_kind = url。
- `update_item_text_rich_text_flattens`：html 条目更新后 sub_kind 为识别结果（None）。
- `update_item_text_syncs_fts`：更新后 FTS 能命中新关键词、不再命中旧关键词（走 query_items_page keyword ≥3 分支）。
- `update_item_text_empty_idempotent_fields`：update 不存在 id 不报错（UPDATE 0 行）、updated_at 不变。

### Rust 命令层校验测试（commands/clipboard.rs 现有 tests 模块风格）

- 非 text 条目（image）→ 错误「仅支持编辑文本内容」。
- 空白 → 错误「内容不能为空」。

### 前端

- tsc + biome lint；实际操作验证（Windows 主路径）。

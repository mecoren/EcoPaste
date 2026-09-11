# 剪贴板文本条目内容编辑（Ditto 式 edit entry）

## Goal

为 EcoPaste 的剪贴板历史条目新增「编辑内容」能力：用户可以直接修改已保存的**文本**条目内容，而不必在外部应用改好后重新复制（当前唯一途径，且会产生一条新记录）。对齐 Ditto 的 edit entry 体验。

## 范围界定

**MVP 范围（本任务）：**

- 仅 `kind = text` 条目可编辑内容；image / files 条目不提供编辑入口（右键菜单中不出现）。
- 编辑后该条记录**就地更新**（同一条 id 保留：分组、收藏、置顶、备注、use_count、created_at 均不动），不产生新记录。
- 编辑入口：右键菜单「编辑内容」项 + 快捷键（Cmd/Ctrl+E）。
- 保存后本地列表镜像即时更新，无需手动刷新。
- 双语文案（zh-CN 默认 + en-US）全量补齐。

**明确不做（Out of Scope）：**

- 不支持 HTML / RTF 富文本源码编辑：富文本条目（sub_kind = html/rtf）的编辑以「纯文本表示」为准——保存后转为普通纯文本条目（sub_kind 重新识别），样式丢失是预期行为，需在 UI 上提示。
- 不支持图片像素编辑、文件路径列表编辑。
- 不提供「编辑后是否同步写回系统剪贴板」选项；保存仅落库，不触碰系统剪贴板。
- 不新增设置项（偏好页不改动）。
- 不新增数据库 migration（复用现有字段）。

## Requirements

### R1 编辑入口

1. 文本条目右键菜单出现「编辑内容 / Edit Content」项，位于「编辑备注」之前；快捷键 `CmdOrCtrl+E`。
2. macOS 原生菜单（muda）与 Windows 自定义菜单窗（context_window）两条路径都展示该项。
3. 前端键盘快捷键 Cmd/Ctrl+E 打开当前活跃（选中/可视首项）文本条目的编辑弹窗；非文本条目不响应。
4. `availableActions` 由 Rust `compute_available_actions` 计算：仅 `kind = text` 时包含 `editContent`；前端不自行判定。

### R2 编辑弹窗（EditModal）

1. 打开时按 id 拉取完整内容（text 条目的 `content` 在列表视图中被 Rust 置空，必须走单条完整读取）。
   - 富文本条目（html/rtf）编辑其纯文本表示（`search_text`，与预览窗口一致），并在弹窗内提示「保存后将转为纯文本」。
   - 普通文本条目编辑 `content` 原文（trim 判空，纯空白不可保存）。
   - 敏感条目（`is_sensitive`）允许编辑（用户本机查看明文是既有能力），不额外脱敏。
2. 弹窗风格与 NoteModal 一致：antd Modal + TextArea，autoSize，打开后聚焦光标置于末尾，保存按钮 loading 态防重复提交。
3. 取消 / ESC 关闭不保存。
4. 编辑后内容为纯空白时禁用保存（后端同样校验，返回用户可读错误）。

### R3 保存语义（Rust 命令）

1. 新命令 `update_clipboard_item_text(id, content)`：
   - 校验条目存在且 `kind = text`，否则返回用户可读错误（中文，遵循 AppError 现有风格）。
   - 校验 trim 后非空。
   - 尊重采集大小上限：超出 `capture.max_text_bytes()` 时拒绝保存，提示过大。
2. 更新字段（单条 UPDATE）：
   - `content` = 新文本（若原为富文本，存纯文本并把 `sub_kind` 重识别）；
   - `search_text` = 新文本；
   - `summary` = 新文本前 256 字符（复用 ingest 的截断规则）；
   - `sub_kind` = 按新文本重新识别（url/email/color/path/None；html/rtf 编辑保存后归 None 或识别结果）；
   - `content_hash` = 按新内容重算（blake3 `"<kind>:<content>"`）；
   - `size` = 新文本 UTF-8 字节数。
3. **不更新**：`use_count`、`is_favorite`、`is_pinned`、`note`、`group_id`、`created_at`、`updated_at`（元数据编辑不刷最近使用时间，与 note/group 语义一致；FTS 同步由既有 AFTER UPDATE 触发器自动完成）。
4. 命令返回更新后的列表视图条目（复用 `get_clipboard_item` 的裁剪 + enrich 逻辑），前端直接回填本地镜像。
5. 不 emit `clipboard://updated`（与 delete/note 一致，前端就地 patch 本地镜像）。

### R4 前端接线

1. `TAURI_COMMAND` 常量、`src/commands/index.ts` 包装（带 label + 成功 toast）。
2. `ClipboardAction` 类型（TS + Rust serde 枚举）新增 `editContent`。
3. List.tsx 订阅派发新增 `editContent` case；快捷键 Cmd/Ctrl+E 分支；编辑保存成功后 patch 本地镜像（含 summary/subKind 等 Rust 返回字段）。
4. 若目标条目正打开预览，打开编辑弹窗前先关预览（与 note 一致）。

### R5 i18n

1. Rust `i18n/`：`ClipboardMenuKey::EditContent`，zh-CN「编辑内容」、en-US「Edit Content」。
2. 前端 `src/locales/*/clipboard.json`：弹窗标题、占位、富文本转纯文本提示、空内容校验提示、快捷键说明文案。
3. 前端 `src/locales/*/commands.json`：命令 label（保存内容）与成功消息。

## Acceptance Criteria

- [ ] 文本条目右键菜单（Windows 实测 + macOS 代码路径审查）出现「编辑内容」，Cmd/Ctrl+E 快捷键可打开；image/files 条目菜单中无此项、快捷键无效。
- [ ] 编辑弹窗回显完整内容（含超长文本、多行文本）；富文本条目回显纯文本表示并显示转换提示。
- [ ] 保存后：同一条记录（id 不变）内容更新；收藏/置顶/分组/备注/使用次数/创建时间不变；「最近使用」排序位置不变（updated_at 未刷新）。
- [ ] 保存后列表卡片摘要立即更新；搜索（含 FTS）能命中新内容；子类型随新内容重识别（例：改成 URL 后右键出现「打开链接」）。
- [ ] 编辑为纯空白：保存按钮禁用；绕过前端（直接调命令）时后端返回可读错误。
- [ ] 超大文本（超采集上限）保存被拒并提示。
- [ ] 编辑前的哈希去重语义不变：编辑后内容若与另一条已有记录相同，两条记录共存（不合并、不迁移），符合「就地更新」预期。
- [ ] zh-CN / en-US 双语完整；`cargo clippy -D warnings`、`cargo test`、`pnpm lint`、tsc 全绿。
- [ ] Windows 上实际操作验证主路径（右键编辑保存、快捷键、空白校验、敏感条目）。

## Notes

- 数据库无 schema 变更，无需 migration；FTS 同步依赖既有 `clipboard_items_au` 触发器（编辑必须走 UPDATE 而非 DELETE+INSERT，否则触发器不生效——见 design.md）。
- 与既有「备注编辑」明确区分：备注是元数据注解（note 字段），本任务是内容本体编辑。

# 合并粘贴与粘贴文本清理

## Goal

多选条目一键合并粘贴（Ditto 刚需），单条粘贴支持去换行等文本清理（网页/PDF 复制乱换行痛点）。

## Requirements

### 合并粘贴（Enter + 工具栏按钮）

- 多选 ≥2 且全为文本类时可合并粘贴；选中集含 image/files 非文本条目时合并按钮禁用 + tooltip 说明，Enter 多选此时 no-op + toast 提示。
- 合并结果不进历史（对齐 Ditto）；逐条累加 use_count（复用既有开关语义）。
- 分隔符为设置项 `clipboard.content.merge_paste_separator`：换行/空格/无/逗号，默认换行；偏好页 + 双语 i18n。
- 单选/无选 Enter 语义不变；Esc 仍先清多选。
- 选中集无序时按当前列表显示序（顶→底）排序后传参。

### 粘贴文本清理（单条 quick action）

- 支持 5 种变换：stripNewlines（换行→空格）、trimLines（每行去空白+去空行）、trimWhitespace、upperCase、lowerCase；仅 Text 类。
- 前端 ClipboardQuickActions 加「清理粘贴」Dropdown 子菜单。
- `Ctrl+Shift+Enter` = 去换行粘贴（最高频痛点给快捷键）。
- ShortcutList 快捷键说明同步。

## Acceptance Criteria

- [ ] 多选全文本 Enter 合并粘贴成功，分隔符符合设置；合并结果不出现在历史列表。
- [ ] 多选含图片/文件时合并按钮禁用且有说明；Enter 给 toast 提示且不粘贴。
- [ ] 单选 Enter、Esc 清多选语义不变。
- [ ] 清理粘贴 5 种变换逐一可用；Ctrl+Shift+Enter 等价去换行粘贴。
- [ ] `cargo test` 覆盖合并拼接/transform；clippy + lint + tsc 全绿。
- [ ] Windows 真机点验：多选 Enter、工具栏按钮、清理菜单、快捷键。

## Constraints

- 零 schema migration；只增设置字段且 serde default 回落。
- transform 隐含纯文本写回；抑制变换后哈希（不污染历史）。
- 富文本取纯文本表示参与合并；image/files 不参与。

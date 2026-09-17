# 搜索过滤语法

## Goal

键盘流用户可用结构化 token 快速过滤：按类型、子类型、来源应用、收藏/置顶、体积过滤；普通搜索零影响。

## Requirements

- 支持 token：`kind:text|image|files`、`sub:url|email|color|path|html|rtf`、`app:名称模糊`（引号支持空格名）、`is:favorite|pinned`、`size:>1mb|<500kb`。
- 未知 token 原样回落 keyword；命中 token 时其余词仍走 FTS 高亮不变。
- `app:` 在 TS 层从已预载 sourceApps store 模糊匹配名称→ids；0 命中给 hint 不应用该 token。
- UI：搜索框聚焦且为空时显示语法速查 Popover（复用 SearchShortKeywordHint 定位模式），双语。
- 全部基于现有列（sub_kind/source_app_id/size），无 migration。

## Acceptance Criteria

- [ ] 各 token 单独与组合过滤正确；未知 token 回落 keyword。
- [ ] `app:"Visual Studio Code"` 引号名可用；0 命中给 hint。
- [ ] 空搜索框聚焦显示双语速查 Popover。
- [ ] 普通关键词搜索行为不变；FTS 高亮不变。
- [ ] parser 单测通过；clippy + lint + tsc 全绿。

## Constraints

- 零 schema migration；SQL 拼进现有 WHERE 链，FTS5 keyword 路径兼容。
- 不新增设置项。

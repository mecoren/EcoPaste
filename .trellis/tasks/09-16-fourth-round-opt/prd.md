# 第四轮优化：合并粘贴×搜索语法×内存渲染×统计维护

## Goal

补齐与 Ditto/CopyQ 对标的功能缺口（合并粘贴、粘贴清理、搜索语法、使用统计），并修复前端内存与渲染硬伤。全轮零 schema migration，低风险串行交付。

现状（已核验）：纯文本粘贴/复制、应用忽略列表、自动清理（时长+条数，豁免收藏/置顶）、敏感检测、批量操作、FTS5 trigram 搜索、三级图片、mimalloc、窗口空闲销毁均已成熟。

## Task Map

- 子① `09-16-merge-paste-cleanup`：合并粘贴 + 粘贴文本清理。先做，无前置。
- 子② `09-16-search-filter-syntax`：搜索过滤语法。依赖子①的 Enter/多选语义不变（见其 prd），实现独立。
- 子③ `09-16-frontend-mem-render`：路由代码分割、i18n 按语言、memo 失效链、索引、AssetImage lazy。与①②无文件强冲突，可独立。
- 子④ `09-16-db-stats-macos`：PRAGMA optimize、使用统计、macOS Concealed/Transient 忽略。与前三批独立。

顺序：① → ② → ③ → ④，逐批单 commit（feat:/perf:/refactor:），pre-commit 即质量门。

## Requirements（父级约束）

- 全部四批零 schema migration；已发布 migration 不回改；已发布设置契约只增字段且 `#[serde(default)]` 回落。
- 仅 macOS + Windows；新能力两端同步或显式标注 TODO。
- Rust-First：拼接/transform/查询扩展放 Rust；前端只做展示交互。
- 命令名、事件名、storage key 等跨端字面量走 Rust 模块常量 + `src/constants/` 同步。
- 每批验证：Rust 单测 + clippy + tsc + biome；UI 人工点验主路径与边界。

## Acceptance Criteria（跨子）

- [ ] 四子任务全部通过各自验收并串行合入 `wait` 分支。
- [ ] `cargo fmt && cargo clippy -- -D warnings && cargo test` 全绿。
- [ ] `pnpm lint` + `pnpm tsc` + 构建通过；批次③前后对比 chunk 尺寸确认下降。
- [ ] 普通搜索（无语法 token）行为零回归。

## Explicitly Out of Scope（本轮不做）

- OCR；条目加密存储（SQLCipher/应用层 AES）；Snippets 模板；多机同步。
- tauri `memory_usage_level`（锁定版 2.11.5 无此 API）；page_size/VACUUM 自动化。

## Notes

- 子任务验收细节见各自 `prd.md`；设计与执行计划见各自 `design.md` / `implement.md`。
- hover-preview 任务（09-15）代码已在 88931f9 合入，仅剩归档，不 blocking 本轮。

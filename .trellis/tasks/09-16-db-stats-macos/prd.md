# DB维护统计面板与macOS隐私协议

## Goal

低风险 DB 维护（PRAGMA optimize）、使用统计面板（盘活已有的 use_count）、macOS 密码管理器标记忽略（行业标准）。

## Requirements

- `PRAGMA optimize`：app 退出时 + cleanup ≥500 行删除的 checkpoint 处顺带执行。
- 使用统计（设置页 data tab）：新命令聚合返回总条数/按 kind 分布/Top10 高频条目（summary 现有列，走现有脱敏）/近 7 日新增量；antd Statistic + List + Progress 展示，不引图表库；展示「高频条目建议收藏」提示。
- macOS ConcealedType/TransientType 忽略：read.rs macOS 读取路径检查 pasteboard types 命中即丢弃；设置 `capture.respect_concealed_types` 默认开，偏好页五件套；i18n 双语。
- macOS 分支本机不可编译验证时按 crate 源码核对 API，真机行为列欠账。

## Acceptance Criteria

- [ ] 退出与大清理后 optimize 执行无报错；长库无锁死。
- [ ] 统计面板四项数据正确；Top10 脱敏展示；无新图表依赖。
- [ ] 1Password/Bitwarden 复制密码类内容不入库（macOS 真机或类型单测佐证）。
- [ ] 设置缺字段时 serde default 回落；clippy + test + lint + tsc 全绿。

## Constraints

- 零 schema migration；统计只读现有数据。
- 不做 page_size/VACUUM 自动化。

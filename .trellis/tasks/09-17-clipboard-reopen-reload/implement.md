# Implement: clipboard reopen reload

## Checklist

- [ ] I1：在 `handleWindowVisibility` 顶部快照 `hadDeferred`，三个 show 分支显式消费（分支一/分支二 `if (hadDeferred) reload()`；默认路径直接 reload 不走 `isAtTop` 门控）。
- [ ] I2：确认 `closePreview("windowOpenReset")` 与选择重置写入顺序不变；显式 reload 放在选择重置写入之后、return/scroll 之前。
- [ ] I3：验证 preserve + 关回顶确定性复现自愈；默认开回顶 + 停深处概率复现自愈；切 tab 路径不受影响。
- [ ] I4：跑 `pnpm lint` 相关检查与前端类型检查；如有单测覆盖则跑对应单测。

## Validation

- 手动复现 A1：设置回顶关 + 三项 preserve → 隐藏窗口 → 系统复制新文本 → reopen → 新条目可见、滚动位置保持。
- 手动复现 A2：默认设置 → 滚动到深处 → 隐藏 → 复制 → reopen → 回顶且新数据可见。
- 回归：切收藏/分组/分类/来源应用仍正常刷新；搜索态 reopen 不丢。

## Risky files / rollback

- `src/pages/Clipboard/components/List.tsx:409-454` 唯一改动点；回滚即 revert 该分支。
- 注意 `List.tsx:216-222` snapshot-effect 清 pending 与 `useClipboardItems.ts:350-372` query-effect 重拉的耦合：本改动用同步快照 + 显式 reload 绕开该竞态，不要顺手改这两个 effect。

## Review gate

- PRD `R1-R4 / A1-A3` 已对齐用户选择（静默刷新保位置）。
- 等用户 review 通过后方可 `task.py start` 进入实现。

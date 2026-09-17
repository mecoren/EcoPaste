# Design: clipboard reopen reload

## Boundaries

- 只改前端 `src/pages/Clipboard/components/List.tsx` 的 `handleWindowVisibility` 补刷分支；不动 Rust 监听、窗口 emit、事件名常量。
- 复用现有 `reload()`（`useClipboardItems.ts:161-190`，force + replace 首屏 + 重带 COUNT），不新增 command/event。
- 不动 `shouldRefreshCurrentGroup` 过滤语义、不动增量插入队列。

## Data flow

- 隐藏期间：`handleClipboardUpdated` 早退记 `deferredReloadRef=true`（`List.tsx:289-292`）。DB 已新、前端旧。
- Reopen：`window://visibility visible=true` → `handleWindowVisibility`（`List.tsx:409-454`）→ 按打开偏好分支 → 补刷。
- 自愈对照：切 tab 写 `clipboardViewState` → `useClipboardItems.ts:350-372` query-effect → `resetAndReload`。reopen 的 preserve 路径缺的就是这次拉取。

## Root cause

1. 确定性：`List.tsx:428` 在 preserve + 关回顶时直接 return，从不消费 deferred。
2. 概率性：默认开回顶时同步 `consume → requestReloadAtTop` 受陈旧 `isAtTopRef=false` 拦截成 no-op（`List.tsx:1221-1229`），全靠 Virtuoso 异步 `atTopStateChange(true)` 第二次补刷，时序错位即漏。

## Changes

- show 入口先快照 `hadDeferred = deferredReloadRef.current`（写入 `clipboardViewState` 前的同步读取，避免后写 snapshot-effect 清 pending 的竞态）。
- 分支一（`!scrollToTopOnOpen && !shouldResetSelection`）：原来直接 return；改为 `if (hadDeferred) reload()` 后 return。静默刷新保位置：只刷首屏 + total，Virtuoso 保留滚动，深行按需 `loadRange` 懒加载。
- 分支二（`!scrollToTopOnOpen`，有选择重置）：原来直接 return 靠 query-effect 兜底；改为同样 `if (hadDeferred) reload()` 后 return。写入若为同值 no-op 时 query-effect 不触发，显式补刷兜底；写入真的改变 query 时与 `resetAndReload` 重复一次首屏 IPC，可接受（token 机制保证后胜，不错乱）。
- 默认路径（开回顶）：`scrollToIndex(0)` 保留，`consumeDeferredReloadAtTop()` 改为直接 `if (deferred) { deferred=false; reload(); }`，不再经过 `isAtTopRef` 门控。`atTopStateChange` 的第二次 consume 届时发现 `deferred=false` 直接返回，不会重复拉取。

## Trade-offs

- 选择直接 `reload()` 而非 `requestReloadAtTop()`：show 时刻用户即将可见，deferred 已证明有新数据，保位置需求由“不滚动”满足，而非“不拉取”。若仍走顶部 Branch，preserve + 停在深处会无限 re-defer。
- 选择接受选择重置路径可能的重复首屏 IPC（显式 reload + query-effect resetAndReload），换取同值写入 no-op 时不丢补刷。重复成本为一次 30 条分页查询，可接受。
- cleanup 期间 total 修正丢失（`List.tsx:299-304` 被隐藏早退跳过）由 `reload()` 的 `totalKnownRef=false` 重带 COUNT 自愈，不单独补 total 算术。

## Compatibility / rollback

- 无 schema、无事件名、无设置项变更；纯前端行为修复，回滚即 revert 单文件分支。
- macOS 16ms 延迟 emit 下：show 事件到达即补刷；若复制恰好落在 show 发起后、visibility(true) 到达前，该事件走隐藏分支记 deferred，随后被同一次 show 消费，自愈。

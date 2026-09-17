# 剪贴板重新打开页面新数据不加载

## Goal

重新打开剪贴板窗口时，隐藏期间产生的新剪贴板数据必须可见，无需用户手动切换收藏/分组来触发刷新。

## Background（已确认事实）

- Rust 侧监听在窗口隐藏期间照常入库 + 全局 `emit clipboard://updated`，无可见性门控（`src-tauri/src/clipboard/watcher.rs:166-194,298-303`）。DB 已新、前端旧是预期差值。
- 前端订阅在隐藏期间不卸载（`src/hooks/useTauriListen.ts:14-35`），冻结靠 `clipboardWindowVisibleRef` 门控：隐藏事件只记 `deferredReloadRef=true` 不拉取（`src/pages/Clipboard/components/List.tsx:287-292`）。
- 补刷责任完全在 reopen 的 `handleWindowVisibility`（`src/pages/Clipboard/components/List.tsx:409-454`）。
- 确定性漏刷：`preserve` + 关闭回顶时 `List.tsx:428` 直接 return，不消费 `deferredReload`，不做任何 reload。
- 概率性漏刷：默认开启回顶时同步 `consume` 撞上陈旧 `isAtTopRef=false` 变成 no-op（`List.tsx:1221-1229,1234-1238`），全靠 Virtuoso 异步 `atTopStateChange(true)` 第二次补刷（`List.tsx:1167-1173`），时序错位即漏刷。
- 自愈路径：切换收藏/分组写 `clipboardViewState` 必经 `useClipboardItems.ts:350-372` 的 `resetAndReload` 全量重拉，所以切 tab 必好。
- 证据链详见 `research/reopen-reload-investigation.md`。

## Requirements

- [ ] R1：reopen（`window://visibility visible=true`）时若存在 `deferredReload`，必须触发一次数据补刷，不受 `scrollToTopOnOpen / select*OnOpen` 是否为 preserve 的影响。preserve + 不回顶时采用静默刷新保位置（只刷首屏 + total，不滚动）。
- [ ] R2：补刷必须覆盖隐藏期间的 cleanup/imported 计数修正丢失问题，靠 `reload()` 重带 COUNT 自愈（`useClipboardItems.ts:161-190`）。
- [ ] R3：修复不得破坏现有语义：浏览中不打断（非顶部不前插）、过滤视图按 `shouldRefreshCurrentGroup`语义（`List.tsx:1995-2007`）正确过滤。
- [ ] R4：macOS 16ms 延迟 emit（`src-tauri/src/window/macos.rs:170-198`）与 show 竞态下不丢补刷。

## Acceptance Criteria

- [ ] A1：配置 `preserve` + 关闭回顶时复现：隐藏期间复制新内容 → reopen 后新条目出现在列表首屏（或对应过滤视图正确位置），无需切 tab。
- [ ] A2：默认开启回顶 + 上次停在列表深处时复现：隐藏期间复制 → reopen 后新数据可见，回顶行为与设置一致。
- [ ] A3：切 tab 自愈路径仍正常；现有相关测试/检查通过。

## Out of Scope

- Rust 监听暂停/排除应用过滤逻辑不动。
- 不引入新的跨端事件名；沿用 `clipboard://updated` 与 `window://visibility`。
- 不改 `query!` 宏/SQL schema。

## Open Questions

- 无（Q1 已决策：preserve + 不回顶时静默刷新保位置）。

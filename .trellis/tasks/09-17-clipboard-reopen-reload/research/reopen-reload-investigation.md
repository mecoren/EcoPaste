# Research: clipboard reopen reload investigation

- **Query**: 有时候重新打开页面，剪切板新数据不会加载，需要切换到收藏再切换回来才会加载 — find root cause chain for clipboard list not reloading on window reopen
- **Scope**: internal (frontend `src/` + Rust `src-tauri/src/window/`, `src-tauri/src/clipboard/watcher.rs`, `src-tauri/src/settings/`)
- **Date**: 2026-09-17

## Findings

### Files Found

| File Path | Description |
|---|---|
| `src/pages/Clipboard/components/List.tsx` | 列表主组件：订阅 `clipboard://updated` + `window://visibility`，持有 `deferredReload` / `isAtTop` / `visible` 三个 ref 的冻结-延后-补刷状态机 |
| `src/hooks/useClipboardItems.ts` | range-cache 数据层：`reload` / `resetAndReload` / `reloadCurrentRange` / `loadRange` / `fetchRange`，query 变化时 `resetAndReload` |
| `src/stores/clipboardView.ts` | 列表过滤真相源（`range` / `category` / `groupId` / `sourceAppId` / `keyword`），tab 切换直接写这里 |
| `src/pages/Clipboard/components/Group.tsx` | tab/分类/分组/来源应用切换入口：写 `clipboardViewState` 触发 query 变化 |
| `src/pages/Clipboard/index.tsx` | 剪贴板窗口装配：`Header` + `Group` + `List` + `Footer`，无额外 reload 逻辑 |
| `src/hooks/useTauriListen.ts` | Tauri 事件订阅封装：`useMount` 订阅一次，`useUnmount` 解绑；窗口 hide 不卸载组件所以订阅一直存活 |
| `src/constants/events.ts` | 事件名常量：`CLIPBOARD_UPDATED = "clipboard://updated"`，`WINDOW_VISIBILITY = "window://visibility"` |
| `src/constants/windowOpenSelection.ts` | 打开偏好哨兵：`preserve` / `all` / `group:<id>`，`parseWindowOpenGroupId` 解析 |
| `src/commands/index.ts` | IPC 封装：`listClipboardItems` 分页拉取，`getClipboardItem(id)` 单条增量拉取 |
| `src-tauri/src/window/mod.rs` | 窗口显隐统一入口：`show_window` / `hide_window` / `intercept_close_request` 内 `emit_visibility` + `lifecycle::on_shown/on_hidden` |
| `src-tauri/src/window/macos.rs` | macOS 剪贴板 panel 化 show/hide：show 延迟 ~16ms 后才 `emit_visibility(true)` |
| `src-tauri/src/window/windows.rs` | Windows 剪贴板窗口 show/hide：同步 `show()`，无延迟 emit（由上层统一 emit） |
| `src-tauri/src/window/lifecycle/mod.rs` | 生命周期管理器：`HiddenWarm` → 5s 后 `Dormant`（剪贴板窗口默认不销毁，只冻结） |
| `src-tauri/src/clipboard/watcher.rs` | OS 剪贴板监听：无窗口可见性门控，隐藏期间照常入库 + `emit clipboard://updated` |
| `src-tauri/src/settings/model.rs` | 窗口偏好默认值：`scroll_to_top_on_open: true`，三项 `select_*_on_open: Preserve`，`lightweight_mode: true` |

### Code Patterns

#### 1. 前端列表数据拉取入口（谁负责拉）

- 数据层唯一真相是 Rust，分页查询入口是 `src/hooks/useClipboardItems.ts:109-114` 的 `listClipboardItems({...queryRef.current, limit, offset, skipCount})`，单条增量入口是 `src/commands/index.ts:926-934` 的 `getClipboardItem(id)`（注释 `src/commands/index.ts:921-924` 明确说供 `clipboard://updated` 增量刷新替代整页重拉）。
- `List.tsx:167-188` 用 `useClipboardItems({favorite, groupId, keyword, kind, sort, sourceAppId})` 把 `clipboardViewState` 映射成查询参数；`List.tsx:189` 由 `getItem` 计算置顶块。
- 三个刷新原语语义不同（`src/hooks/useClipboardItems.ts`）：
  - `reload`（`src/hooks/useClipboardItems.ts:161-190`）：token+1 作废在途请求，`totalKnownRef=false`（下次带 COUNT），`viewRange` 重置到 `[0, PAGE_SIZE-1]`，`force:true + replace:true` 拉首屏。注释 `src/hooks/useClipboardItems.ts:165-167` 说明这是事件驱动全量刷新。
  - `resetAndReload`（`src/hooks/useClipboardItems.ts:192-217`）：比 `reload` 更彻底，清 items/total/loadedInitial 回 loading 态；唯一被 query-effect 调用的就是它（`src/hooks/useClipboardItems.ts:350-372`，deps 含 `query.favorite/group/groupId/keyword/kind/pinned/sort/sourceAppId`，`queryRef.current` 更新后调 `resetAndReload()`）。
  - `reloadCurrentRange`（`src/hooks/useClipboardItems.ts:219-231`）：按当前 `viewRange ± PRELOAD_ROWS` 重拉；批量收藏/置顶/移分组后用它（`List.tsx:760,781,802,830`）。
  - `loadRange`（`src/hooks/useClipboardItems.ts:233-245`）：Virtuoso `rangeChanged` 驱动（`List.tsx:1154-1165` → `loadRange(startIndex, endIndex)`），非 force，有缓存命中早退。
- 事件订阅位置：
  - `clipboard://updated` 订阅在 `List.tsx:398-403`（`useTauriListen(CLIPBOARD_UPDATED, ...)` → `handleClipboardUpdated`，定义 `List.tsx:287-364`）。
  - `window://visibility` 订阅在 `List.tsx:456-459`（`useTauriListen(WINDOW_VISIBILITY, ...)` → `handleWindowVisibility`，定义 `List.tsx:409-454`）。
  - 分组变化订阅 `List.tsx:258`（`CLIPBOARD_GROUPS_UPDATED` 只刷分组名，不刷列表）。
- `useTauriListen`（`src/hooks/useTauriListen.ts:14-35`）在 `useMount` 时 `listen` 一次并把最新 handler 存 `handlerRef`；窗口隐藏 ≠ React 卸载，所以隐藏期间监听仍在收事件——冻结全靠 `List.tsx` 内部的 ref 门控，不是靠取消订阅。

#### 2. 窗口显示/隐藏逻辑（reopen 是否 emit 事件）

- Rust 统一入口 `src-tauri/src/window/mod.rs:112-153`（`show_window`）：先重建（`rebuild_fn`，`mod.rs:115-119`）、恢复几何（`mod.rs:121-139`），再调平台 `show_window`，成功后同步 `emit_visibility(label, true)`（`mod.rs:149`）+ `lifecycle::on_shown`（`mod.rs:150`）。payload 定义 `mod.rs:91-95`（`{label, visible}`），事件名 `mod.rs:89`（`"window://visibility"`）。
- `hide_window`（`mod.rs:160-179`）：保存几何 → `preview::suppress_for_clipboard_hide` → 平台 hide → 同步 `emit_visibility(label, false)`（`mod.rs:175`）+ `on_hidden(..., "hide")`（`mod.rs:176`）。关闭按钮路径 `intercept_close_request`（`mod.rs:243-263`）同样 `hide()` + `emit_visibility(false)`（`mod.rs:259`）+ `on_hidden(..., "close")`（`mod.rs:260`）。
- macOS 例外：`delays_clipboard_visibility_event`（`mod.rs:156-158`）让剪贴板窗口跳过同步 emit；真正的 emit 在 `src-tauri/src/window/macos.rs:170-198` 的 `show_clipboard_panel` 里，`spawn + sleep(16ms)`（`macos.rs:17,174`）后再 `run_on_main_thread` 内 `panel.show_and_make_key()` → `resume_after_clipboard_show()` → `emit_visibility(..., true)`（`macos.rs:189`）→ `on_shown`（`macos.rs:190`）。即 macOS reopen 的 `visible=true` 比 `show_window` 返回晚约 16ms+主线程调度。
- Windows 路径 `src-tauri/src/window/windows.rs:23-43` 无延迟，`show()` 后由上层同步 emit；`hide_window`（`windows.rs:125-139`）同步 `hide()`，同样由上层同步 emit。
- 前端可见性镜像初值 `List.tsx:133-135`：`clipboardWindowVisibleRef = false`（注释明说启动即隐藏，首个 show 事件翻正）。`handleWindowVisibility`（`List.tsx:409-416`）先按 `label !== WINDOW_LABEL.CLIPBOARD` 过滤（`WINDOW_LABEL` 定义见 `src/constants/windows.ts`），再写 `clipboardWindowVisibleRef.current = visible`，`!visible` 直接 return。
- 生命周期侧：`show/hide` 同时推进 `window://lifecycle`（`lifecycle/mod.rs:294-306`），但 `List.tsx` 完全不订阅 `WINDOW_LIFECYCLE`（全 repo 只有 `src/stores/windowLifecycle.ts:54-68` 订阅做镜像，无列表刷新消费；grep `WINDOW_LIFECYCLE` 命中见本报告 §1）。`dormant` 语义在 `lifecycle/mod.rs:31-32`（`CLIPBOARD_DORMANT_SECS = 5`）、`lifecycle/mod.rs:54-55`（`Dormant` 定义保留实例但前端应暂停非必要刷新）、`lifecycle/mod.rs:249-261`（HiddenWarm 5s 后进 Dormant）。前端 `List.tsx:134` 注释的 dormant 延后与此对应，但实现上只认 `window://visibility` 布尔值，不认 lifecycle phase。

#### 3. 两条路径对比（为什么切 tab 能好，reopen 不行）

- Tab 切换路径（必定重拉）：
  - `Group.tsx:261-263`（`selectRange`）、`Group.tsx:268-271`（`toggleCategory`）、`Group.tsx:276-278`（`toggleCustomGroup`）、`Group.tsx:283-286`（来源应用）、`Group.tsx:395-398`（`toggleRange`，all↔favorite）都是直接写 `clipboardViewState`。
  - 写操作触发 `useClipboardItems` 的 query-effect（`src/hooks/useClipboardItems.ts:350-372`）→ `resetAndReload()` → 无条件全量重拉首屏。所以“切到收藏再切回来”是两次 `resetAndReload`，DB 里隐藏期间写入的新行此时被读回——症状自愈。
  - 附带：`List.tsx:216-222` 的 snapshot-effect 在任何 `clipboardViewState` 变化时清 `deferredReloadRef=false` + 清选中/预览，但它本身不拉取；重拉全靠上一条的 query-effect。这是 fragile 耦合：清 pending 与重拉分属两个 effect，但正常 tab 切换时两者同 tick 触发所以不丢。
- Reopen 路径（条件重拉，大概率漏）：
  - `handleWindowVisibility` 全文 `List.tsx:409-454`。show 时先读四个打开偏好（`List.tsx:418-423`：`scrollToTopOnOpen/selectRangeOnOpen/selectCategoryOnOpen/selectGroupOnOpen`），计算 `shouldResetSelection`（`List.tsx:424-427`）。
  - **Early-return #1**（`List.tsx:428`）：`if (!scrollToTopOnOpen && !shouldResetSelection) return;` —— preserve + 不回顶配置下，函数在更新 `clipboardWindowVisibleRef=true` 之后直接返回，**不调 `consumeDeferredReloadAtTop()`，不调任何 reload，不重置滚动**。隐藏期间攒的 `deferredReloadRef=true` 被原样留下。
  - 选择重置段（`List.tsx:432-447`）：只有非 preserve 才写 `clipboardViewState`；写了才会间接触发 `resetAndReload`（同 tab 路径）。preserve 配置下这段无操作。
  - **Early-return #2**（`List.tsx:449`）：`if (!scrollToTopOnOpen) return;` ——即使发生了选择重置，只要没开回顶，同样不执行 `scrollToIndex + consumeDeferredReloadAtTop`。此分支靠 query-effect 的 `resetAndReload` 兜底，所以一般仍能刷；但纯 preserve + 关回顶时连 query-effect 都没有。
  - 正常消费段（`List.tsx:451-453`）：`setSelectedId(null)` + `virtuosoRef.scrollToIndex({index:0})` + `consumeDeferredReloadAtTop()`。只有默认配置（`scroll_to_top_on_open: true`，见 `src-tauri/src/settings/model.rs:636`）能走到这里。
  - `consumeDeferredReloadAtTop`（`List.tsx:1234-1238`）→ `requestReloadAtTop`（`List.tsx:1221-1229`）：`if (!isAtTopRef.current) { deferredReloadRef=true; return; }`，否则 `deferred=false; reload()`。而 `handleAtTopStateChange`（`List.tsx:1167-1173`）在 `atTop===true` 时才补消费。所以默认配置下的 reopen 实际是两步：同步 `consume`（此时 `isAtTopRef` 还是隐藏前的旧值，若用户上次停在半中则本次 consume 是 no-op）+ 等 `scrollToIndex(0)` 触发 Virtuoso `atTopStateChange(true)` 后的第二次 `consume`。中间任何一次 Virtuoso 回调丢失/时序错位就表现为“有时候”不刷。

#### 4. 防抖/节流/缓存/early-return 清单（reopen 时谁会跳过刷新）

- 隐藏期间冻结（`List.tsx:287-292`）：`if (!clipboardWindowVisibleRef.current) { deferredReloadRef.current = true; return; }`。注释（`List.tsx:288`）明说避免隐藏期间反复 IPC + 重渲染。这是预期设计，但把补刷责任完全推给 reopen 的 `consume`。
- 顶部门控（`List.tsx:1221-1229` + `List.tsx:1167-1173`）：可见态收到的更新若 `isAtTopRef===false` 也只记 `deferred=true`（`requestReloadAtTop`），等回顶消费。`isAtTopRef` 初值 `true`（`List.tsx:124`），由 Virtuoso `atTopStateChange`（`List.tsx:1354` 接线）维护。reopen 时若旧值为 false，同步 consume 无效，必须等滚动回调。
- 增量插入分支（`List.tsx:354-363`）：只有 `payload.id && isAtTopRef.current` 才进 `pendingInsertIdsRef + drainPendingInserts`（`List.tsx:370-396` 经 `getClipboardItem` 单条拉取后 `insertItemAtTop`）；否则一律 `requestReloadAtTop()`（同样受顶部/隐藏门控）。串行队列本身（`insertingRef/pendingInsertIdsRef`，`List.tsx:141-142`）只保序，不丢事件，但隐藏期间根本不进队列。
- 过滤归属 early-return（`List.tsx:317-326` 的 `shouldRefreshCurrentGroup`，定义 `List.tsx:1995-2007`）：`groupId!=null → false`（`List.tsx:2001`），`range==="favorite" → false`（`List.tsx:2002`），有 `category` 但 `kind` 缺失/不等 → false（`List.tsx:2003-2006`）。即收藏视图/自定义分组视图/分类不匹配时更新直接丢弃（连 deferred 都不记）。这解释了“切到收藏能看到，切回全部才看到”的另一半：如果新条目 kind 与当前 category 不一致，当前视图本来就不该出现，切 tab 只是换了个会显示它的查询。
- 其它整页 reload 分支（`List.tsx:328-352`）：去重（`deduplicated`，`List.tsx:330-333`）、搜索态（`keyword.length>0`，`List.tsx:336-339`）、`useCountDesc` 排序（`List.tsx:342-345`）、来源应用过滤（`List.tsx:349-352`）全部走 `requestReloadAtTop()`，同样受隐藏/顶部两道门控。注：去重注释（`List.tsx:328-329`）说明精确 patch 不可靠才走 reload。
- cleanup/imported 分支（`List.tsx:294-315`）：`cleanup` 带删除计数并调 `requestReloadAtTop`（`List.tsx:305`），`imported` 同理（`List.tsx:313`）；隐藏期间同样被顶部的 `!visible` 早退拦截（`List.tsx:289-292` 在所有分支之前），所以隐藏期间的清理/导入也只留一个 `deferred=true`，丢失了 `clipboardStatsState.total` 修正（`List.tsx:299-304` 被跳过）。
- 缓存早退（`src/hooks/useClipboardItems.ts:82-99`）：`normalizeFetchRange` 为 null、已加载区间、已有覆盖中请求都会 return。但 `reload/resetAndReload/reloadCurrentRange` 都带 `force:true`（`useClipboardItems.ts:179-183,206-210,226-230`），绕过这两个检查；`loadRange`（滚动触发）不带 force 才受影响。所以缓存本身不是 reopen 不刷的原因——原因是 reopen 根本没调任何 fetch。
- COUNT 治理（`useClipboardItems.ts:103-106,118-121`）：同过滤下首次带 COUNT、之后 `skipCount` 沿用本地 total；`reload/resetAndReload` 重置 `totalKnownRef=false`（`useClipboardItems.ts:167,198`），所以补刷缺失时 total 也陈旧，Footer 计数（`List.tsx:210-213` 同步 `clipboardStatsState.total`）同样停留。
- 搜索防抖（`src/stores/clipboardView.ts:35-55`，200ms）：只影响 keyword 写入时机，与 reopen 无直接关系；但注意搜索态的更新走整页 reload（`List.tsx:336-339`），隐藏期间的搜索态更新同样被延后。
- snapshot-effect 清 pending（`List.tsx:216-222`，`deferredReloadRef.current=false`）：任何过滤变化都清 pending，依赖 query-effect 的 `resetAndReload` 兜底。若未来出现“只清 pending 不触发 query 变化”的写入者，会直接丢一次补刷。

#### 5. Rust 侧隐藏期间是否继续写 DB

- 是，继续写。`watcher.rs:298-303` 的唯一早退是托盘“停止监听”（`pause.is_paused()`）；`watcher.rs:311-331` 是偏好排除应用过滤；`watcher.rs:360-363` 是自身写回 guard。这三者之外没有任何窗口可见性/lifecycle/dormant 门控。
- 入库+通知在 `watcher.rs:166-194` 的 `persist_and_notify`：`upsert_item`（`watcher.rs:181`）→ `maybe_play_copy` → `app.emit("clipboard://updated", {id, kind, deduplicated})`（`watcher.rs:183-190`）。`app.emit` 是全局广播，隐藏窗口的 webview 仍能收到（前端订阅未卸载，见 §1），所以隐藏期间的新复制：DB 已有新行 + 前端收到了事件但主动 `return` 只记 `deferred=true`（`List.tsx:289-292`）。reopen 时 DB 与列表的差值正是这批被延后的行。
- 常驻性：监听跑在独立线程阻塞 `start_watch`（`watcher.rs:250-286`），macOS 120ms 轮询（`watcher.rs:40`）、Windows 事件驱动；`init`（`watcher.rs:199-240`）在 setup 期调用一次，与窗口生命周期无关。dormant（`lifecycle/mod.rs:54-55`）只冻结前端刷新，不停 Rust 监听。

### External References

- 无外部依赖问题。本 bug 链路完全在仓库内（Tauri `app.emit`/`listen` 语义为框架既定行为，无需外部文档佐证版本约束）。

### Related Specs

- 未在 `.trellis/spec/` 下发现与剪贴板 reopen 刷新直接相关的 spec（仅有 `backend/`、`frontend/`、`guides/` 目录结构，无条目可引用）。行为契约以 `AGENTS.md` 的跨端事件约定（`domain://action`）和 Rust-First 边界为准。

## Caveats / Not Found

- 未实际复现 UI（无运行态日志/录屏），“有时候”的触发条件是基于代码分支推断的：默认开启 `scrollToTopOnOpen=true` 时 reopen 应走消费段，失败需 `isAtTopRef` 旧值 false + Virtuoso `atTopStateChange` 时序配合；关闭回顶或三项全 preserve 时 reopen 必定不消费 deferred（确定性复现）。
- `consumeDeferredReloadAtTop` 在 `scrollToIndex` 同步调用之后立即执行（`List.tsx:452-453`），而 `scrollToIndex` 的滚动完成与 `atTopStateChange(true)` 是异步的——此竞态的实际命中率需运行时验证（如 reopen 前停在列表深处 vs 顶部）。
- macOS 的 16ms 延迟 emit（`macos.rs:173-190`）与隐藏期间到达事件的交错窗口未量化；若复制恰好发生在 show 发起后、visibility(true) 到达前，该事件仍走隐藏分支记 deferred，随后被同一次 show 的 consume 带走（默认配置下自愈），非默认配置下仍丢失。
- `shouldRefreshCurrentGroup` 在收藏/分组视图下直接 return 且不记 deferred（`List.tsx:317-326`）是按设计的过滤语义，但 reopen 时若用户停留在这些视图，隐藏期间其它分组的新数据本来就不该出现——报告 bug 时需确认 reopen 失败时的当前 range/group/category，否则会把“过滤正确”误判为“刷新丢失”。

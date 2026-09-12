# EcoPaste 性能与功能差距优化报告

> 调研日期：2026-09-12。对标项目：Maccy、CopyQ、Ditto、PasteBar。
> 结论先行：本项目在**搜索（FTS5 trigram）、分页（range cache ≤180 行）、图片链路（缩略图 + asset protocol，无 base64 IPC）、防重复（content_hash upsert）**上已达到或超过市面水准；真正的短板集中在**热路径的冗余开销**（每次复制触发两次 Settings 深拷贝 + 前端整页重拉 + 全列表重渲染）与**列表查询缺失排序索引 / N+1 图标解析**。本轮直接实施下表 P0/P1 全部项。

## 一、功能对标结论（Feature Gap）

「市场标配」（3/4 以上产品具备）：置顶、纯文本粘贴、按应用过滤、搜索、键盘优先、图片+文件支持、按条数保留、防重复、快速预览 —— **EcoPaste 全部具备**。

| 能力 | Maccy | CopyQ | Ditto | PasteBar | EcoPaste | 判定 |
| --- | :-: | :-: | :-: | :-: | :-: | --- |
| 多选批量操作 | 部分 | ✅ | ✅ | ❌ | ❌ | **差距**（市场 2/4，Ditto/CopyQ 标配；后端 `delete_items` 已存在未暴露） |
| 设备间同步 | ❌ | 目录式 | 网络 | 目录式 | ❌ | 差距，但涉及网络/云，本轮不做 |
| 内置加密 | ❌ | ✅ | 网络层 | PIN | 仅备份容器 | 可接受（local-first 定位） |
| OCR / 二维码 | ❌ | ❌ | 仅 QR | ❌ | ❌ | 非标配，不做 |
| 脚本/自动化 | ❌ | ✅ | ✅ | ❌ | ❌ | 重依赖大功能，独立立项 |
| FTS5 全文搜索 | 模糊 | 模糊 | 模糊 | 模糊 | ✅ | **领先** |
| 秘密检测+脱敏 | ❌ | ❌ | ❌ | ❌ | ✅ | **领先** |

本轮不做新大功能（多选、同步、OCR、脚本）的原因：用户目标明确为「性能优化、UI 优化、功能优化、内存优化」的直接落地，多选批量删除需要新命令 + 菜单 + 键盘交互三端联动，属于独立 feature，塞进本轮会稀释优化质量且验证面过大。已列为后续建议。

## 二、热路径性能问题（P0，全部实施）

每次用户在任意应用里按一次 Ctrl+C，当前发生：

1. **watcher 深拷贝整个 Settings 两次**（`watcher.rs:276` 排除名单判定 + `:293` 完整快照）——Settings 含 `excluded_app_ids`、capture order、itemActions 等多个 Vec，每次复制都整结构 clone 两遍；`persist_and_notify`（`watcher.rs:133`）再 clone 一次 item。
2. **前端整页重拉**：`clipboard://updated` 事件携带 `id/kind/deduplicated`，但 `List.tsx` 的 `handleClipboardUpdated` 一律 `requestReloadAtTop() → reload()` 重拉 30 行建新 Map——后端专门为增量刷新准备的 `get_clipboard_item`（`commands/clipboard.rs:559`，注释明说「避免事件驱动刷新整页 refetch」）前端从未接线（constants 已注册，TS 包装从未写）。
3. **全列表重渲染**：每次修饰键按下/抬起都 `setIsModifierPressed` 触发整个 List 重渲染（`List.tsx:620-622/766-770`），仅为给 URL/Email 卡片加链接态样式；且 `ClipboardCard`/`TextCard` 无 `React.memo`，选中态、firstVisibleIndex 任何变化都重渲染所有可视卡片。
4. **排序零索引**：`ORDER BY is_pinned DESC, updated_at DESC`（默认排序）在 `created_at`/`updated_at`/`use_count` 上没有任何索引，历史越多排序越慢（万条历史 = 每次翻页全表排序）。清理任务 `ORDER BY created_at DESC LIMIT -1 OFFSET n` 同样受益于 `created_at` 索引。
5. **N+1 文件图标解析**：列表页每个 files 条目 × 每个路径一次 DB 查询 + 一次 `path.exists()`（`commands/clipboard.rs:542-550 → attach_file_entries → resolve_file_icon_path`），20 行/页最多 ~100 次串行 DB 往返 + fs stat；同一 cache_key 在一页内反复解析。

## 三、内存问题（P0/P1）

1. **P0 · 备份导入内存峰值**（不改，见下）**——评估后保持现状**：整个 `.ecopastebak` 读入 `Vec<u8>` 再解密出另一个 `Vec<u8>`（`backup/mod.rs:724-728, 823-851`），merge 时 `fetch_all` 全量历史（`:1172-1179`）。但这属于低频路径（用户手动备份/恢复时才发生），流式改造牵动加密格式兼容与进度上报，收益/风险比不划算，本轮不动。
2. **P1 · 预览 LRU 无字节预算**：`Preview/cache.ts` 仅按条目数（16）驱逐，单条大文本（数 MB 的完整 content）会常驻常驻 webview——补字节数上限。
3. **P1 · sqlx pool 无显式上限**：默认 10 连接对单 SQLite 文件 + WAL 场景偏多，每连接都有页缓存；显式收紧 + `busy_timeout` 消除偶发 `database is locked`。

## 四、实施清单

| # | 层 | 改动 | 优先级 | 类型 |
| --- | --- | --- | --- | --- |
| 1 | Rust | migration `0002`：`created_at`、`updated_at`、`use_count` 排序索引 | P0 | 性能 |
| 2 | Rust | sqlx pool：`max_connections(5)` + `busy_timeout` | P0 | 性能/内存 |
| 3 | Rust | watcher 每次事件只取一次 Settings 快照，复用做排除判定与 capture | P0 | 性能/内存 |
| 4 | Rust | 列表页文件图标：页内 cache_key→icon 预取（单查询 IN 批量），去 N+1 | P1 | 性能 |
| 5 | 前端 | 接线 `get_clipboard_item`：新条目事件 → 单条拉取 + 本地前插，不再整页重拉 | P0 | 性能/IPC |
| 6 | 前端 | `ClipboardCard`/`TextCard` `React.memo`；修饰键链接态改用 `data-mod-pressed` 属性 + CSS `:has()`，去掉修饰键 setState | P0 | UI 性能 |
| 7 | 前端 | 预览 LRU 增加字节预算，超限驱逐最旧 | P1 | 内存 |

保持现状（有意不动）：180 行窗口缓存、缩略图+asset protocol、FTS 双路径、dormant 冻结、监听器清理——这些是对标下来的优点，报告存档防止后续误改。

## 五、后续建议（不在本轮）

多选批量删除（后端 `delete_items` 已就绪，只欠命令暴露 + UI）、Shift+数字键纯文本粘贴（ShortcutList 已留注释位）、iOS/多设备同步（Ditto 式网络同步 or 目录式）、退出时清空历史选项、i18n 扩展语言。

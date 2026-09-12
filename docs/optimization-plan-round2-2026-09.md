# EcoPaste 第二轮优化方案（Round 2）——实施完成版

> 调研日期：2026-09-12。对标：Maccy、CopyQ、Ditto、PasteBar。
> **实施状态**：方案全部落地，共 8 个提交（0795cc8 → 7c59855）。各实施项下方标注了实际提交哈希与验证方式；两处经实施评估降级/放弃的项已注明原因。
> 前置：第一轮 7 项优化已全部落地并提交（`8ec2eeb` 排序索引/连接池/watcher 快照/图标预取 + `739b1f3` 增量列表/memo/CSS 链接态/预览字节预算，见 `docs/optimization-report-2026-09.md`）。本文档只含**新项**，不与第一轮重复。
> 结论先行：第一轮已把「市场标配」能力和列表热路径做到位；本轮真正值得做的是 **1 个功能缺口（多选批量删除，市场 2/4 标配，后端 `delete_items` 已实现只欠暴露）+ 1 个存储层真 bug 级问题（每次复用/收藏/备注都全量重写 FTS trigram 索引）+ 4 项渐进收益项**。前端内存已达标（180 行窗口缓存 + 2MB 预览 LRU），本轮内存工作集中在 Rust 侧与存储层。

## 需求维度映射（性能 / UI / 功能 / 内存）

| 维度 | 本轮对应项 | 一句话结论 |
| --- | --- | --- |
| 性能 | P0-1、P1-3、P2-1、P3 | 消除每次粘贴的无效 FTS 索引重写与 4MB 级 clone；备份导入万条从分钟级到秒级 |
| UI | P0-2 的多选交互、P1-1 | UI 热路径（重渲染/滚动/链接态）第一轮已做完且达标；本轮 UI 工作量集中在多选的选中态与键盘交互 |
| 功能 | P0-2、P1-1、P2-2、P2-3 | 多选批量删除是唯一市场标配缺口；补齐 Ditto 式 DB 压缩与 Maccy 式退出清空 |
| 内存/存储 | P0-1、P1-2、P2-2 | 纯文本双写消除省约一半存储与页缓存；FTS/WAL 放大收敛；前端内存（180 行窗口 + 2MB LRU + dormant 冻结）已达标，本轮不再臆造 |

## 一、本轮新发现的问题（按优先级）

### P0-1 FTS UPDATE 触发器无值变化条件 —— 每次粘贴都重写全文索引

> ✅ **已实施**（0795cc8）。验证：TDD 三测试（元数据 UPDATE 页数不变 / 编辑重建 / note 变化重建）+ 真实 dev 库触发器定义确认（au 带 WHEN + COALESCE）。实施中发现方案原文档的错误：`note` 本身是 FTS 索引列，备注**改值**必须重建索引（否则备注搜不到），已从「纯元数据」清单中移出、补了专门测试。

**问题**：`0001_init.sql:70-77` 的 `clipboard_items_au` 触发器对**任何** UPDATE 都执行 FTS 行 delete + insert（trigram 分词重建）。但以下高频路径都是纯元数据 UPDATE，根本不碰 `search_text`/`note`：

- 每次复用条目（点一次粘贴/复制）→ `increment_item_use_count`（`db/items.rs:303-313`，UPDATE `use_count, updated_at`）；
- 收藏 / 取消收藏、置顶 / 取消置顶、移动分组、写备注（`update_clipboard_item_note` 等）。

对一条几十 KB 的文本，trigram delete+insert 意味着每次点击都多一次几 KB 级索引重写 + WAL 放大；历史越大、文本越长代价越高。这在概念上就是错的：全文索引只应在被索引列**值变化**时重建。

**方案**：新增 migration `0003_fts_update_guard.sql`（已发布 migration 不回改）：

```sql
DROP TRIGGER clipboard_items_au;
CREATE TRIGGER clipboard_items_au AFTER UPDATE ON clipboard_items
WHEN new.search_text IS NOT old.search_text OR new.note IS NOT old.note
BEGIN
  INSERT INTO clipboard_items_fts(clipboard_items_fts, rowid, search_text, note)
  VALUES ('delete', old.rowid, old.search_text, old.note);
  INSERT INTO clipboard_items_fts(rowid, search_text, note)
  VALUES (new.rowid, new.search_text, new.note);
END;
```

`WHEN` 值比较比 `UPDATE OF <列>` 更稳：后者在「SET 了但值没变」时仍触发，前者彻底挡掉纯元数据更新。

**验收**：
- 复用条目（连续两次粘贴同一条）、切收藏/置顶/备注后，该条目仍可被原关键词搜索命中（行为不变）；
- 编辑文本内容后（`update_clipboard_item_text` 确实改写 `search_text`），新关键词可搜到、旧关键词不再命中（值变化路径正常触发）；
- 现有 FTS 触发器相关 `cargo test` 全绿，并为「元数据更新不触发重建」补一条回归测试。

**风险**：低。SQLite `WHEN` 是标准特性；唯一语义变化就是不再做无效重建。已存在的 0001 不动，纯增量。

---

### P0-2 多选批量删除 —— 唯一的市场标配缺口，后端已 90% 就绪

> ✅ **已实施**（56ade65 Rust + b9e4842 前端）。验证：仓储测试（图片文件名回收 / 受保护条目跳过）、tsc/lint、真实 dev 库行删除验证。**实施调整**：方案原计划的「右键菜单删除选中 N 项入口」经评估**不做**——Rust 菜单动作枚举 → i18n → macOS muda + Windows webview 菜单窗 → 前端动作映射共 5 处跨端镜像，为一个次要入口牵动过广；Ctrl/Cmd+Click + Shift+方向键 + Cmd/Ctrl+Backspace 的交互闭环已完整。批量粘贴/合并粘贴维持不做。

**对标**：Ditto / CopyQ 均为标配（第一轮报告已确认差距，当时列为后续建议；Maccy/PasteBar 无此功能，市场 2/4，但批量清理几十条历史是真实高频需求，尤其配合筛选条件使用——先按类型/来源应用过滤，再一把清掉）。

**现状**：后端 `db::items::delete_items`（`db/items.rs:336-350`）已实现批量 DELETE 且带测试，但 `#[allow(dead_code)]`——tauri command 从未暴露。**注意**：它当前只返回行数、**不收集被删图片文件名**，直接暴露会导致图片盘上泄漏。

**方案**（三端联动，功能边界明确为「只做删除，不做批量粘贴」——批量粘贴/合并粘贴 CopyQ 独有且交互复杂，第一版不做）：

1. **Rust**：
   - 改造 `db::items::delete_items`：仿 `cleanup_history`（`db/items.rs:368-408`）的 `RETURNING kind, content` 模式收集被删的图片文件名，返回 `Vec<String>` 并批量 `store.remove()`（复用 `commands/clipboard.rs:1493` 单条删除的清盘逻辑，含删除失败仅告警不报错的语义）；
   - 新增 `#[tauri::command] delete_clipboard_items(ids: Vec<String>)`（`commands/clipboard.rs`），走 `TAURI_COMMAND` 常量 + `src/constants/` 同步注册；删除后 emit 一次 `clipboard://updated`（带 `cleanup` 计数语义，与现有单条删除一致）；
   - 命令层沿用全部删除保护开关（favorite/pinned 五个 guard）校验——批量版直接拒绝含受保护条目并逐条返回跳过原因，还是先过滤后删除？**推荐：校验在前**，一次 SELECT 拿到保护标记，含受保护条目且未确认时返回明确错误，由前端确认弹层统一处理。

2. **前端**（`List.tsx` + `useClipboardItems`）：
   - `selectedIds: Set<string>` UI 状态（valtio `clipboardView` store，不进业务数据）；
   - Ctrl/⌘+Click 切换选中；Shift+↑/↓ 从当前锚点扩展连续选择（范围选择只做键盘版，Shift+Click 范围选择第一版不做，避免与现有单击行为配置冲突）；
   - Delete/⌘Backspace：有选中集时走批量删除（复用现有确认弹层文案，双语补齐 zh-CN/en-US）；
   - 选中集非空时：卡片渲染选中态样式（`data-selected` + CSS，沿用第一轮「样式不走 React state」的原则）；Enter 仍作用于键盘高亮的单条（多选不劫持粘贴主路径）；Esc 清空选中集后再走现有的层级退出；
   - 右键菜单（Rust 侧 `popup_clipboard_item_menu`）追加「删除选中 N 项」入口，走现有 `clipboard://menu-action` 事件通道回派。

3. **i18n**：`clipboard` 命名空间补 `multiSelect`、`deleteSelected`、`selectedCount` 等键，zh-CN + en-US 同步。

**验收**：筛选「某来源应用 + 文本类型」后全选删除，行数与总数、磁盘图片同步减少（对比 `resources/clipboard-images` 文件数）；含收藏条目且保护开关开启时被拦截；删除后翻页、搜索、增量插入（新复制事件）互不干扰；`cargo test` + 前端手工主路径点验。

**风险**：中。交互面（键盘/右键/确认弹层）是主要成本；Rust 侧改动小且有既有模式可抄。

---

### P1-1 Shift+数字键 = 纯文本粘贴（Ditto/Maccy ⌥+n 等价物）

> ✅ **已实施**（b9e4842）。ShortcutList 占位注释兑现，Footer 速查面板新增条目，i18n 双语。

**现状**：`List.tsx:70` 前 10 项已有 1-9/0 快捷粘贴（`handleQuickPaste` → `pasteClipboardItem(item.id, false)`，`List.tsx:984-987`），纯文本粘贴的命令参数 `plain: true` 也早已支持（`commands/clipboard.rs:367-373`）——只差一个修饰键分支。第一轮报告已把它列为预留位。

**方案**：数字键 keydown 时检测 Shift → `pasteClipboardItem(item.id, true)`；Footer 快捷键弹层（K 键）ShortcutList 补一行「Shift+数字 粘贴为纯文本」；i18n 双语。纯前端，零后端改动。

**验收**：富文本条目用 Shift+数字粘贴得到无格式文本；Footer 提示双语显示；不影响现有 1-9/0 行为与 IME 输入路径。

**风险**：极低。

---

### P1-2 消除纯文本 `search_text` 双写 —— 每行省约一半存储

> ✅ **已实施**（c784e87）。验证：TDD（NULL search_text 的 FTS 命中 + LIKE 兜底命中 + 编辑路径置 NULL）+ **真实 dev 库端到端**：migration 0004 在 371 条存量行上应用（120 条纯文本置 NULL、251 条富文本投影零误伤）；clip.exe 写剪贴板 → watcher 入库 → FTS/LIKE 检索全通；去重（use_count 递增）复用正常。**实施中发现方案未识别的第 4 个触点**：1-2 字符短词的 LIKE 兜底查询直接匹配 `search_text` 列，NULL 后会漏——已同步改 `COALESCE(search_text, content)`（`db/items.rs`）。备份 `BackupItemRow` 为 `Option<String>` 天然兼容。

**问题**：纯文本条目入库时 `content` 与 `search_text` 存**同一个字符串**（`clipboard/ingest.rs:155-163` `draft_plain_text`：`content: plain.to_owned()`，`search_text: plain_search` 且调用方传的就是同一份）。默认 `max_text_mb: 4` 下，一条 4MB 文本在表里存 8MB；trigram FTS 索引再放大 ~3x。纯文本通常占历史 90%+，这是最大的存储与页缓存浪费源。富文本（HTML/RTF）的 `search_text` 是纯文本投影，与 `content` 不同，**必须保留**；files 的 `search_text` 是 basename 投影，同样保留。

**方案**（关键洞察：FTS external-content 表写入什么完全由**触发器**决定，查询时并不读原表列，所以触发器里可以 `COALESCE`）：

1. `ingest.rs`：纯文本路径 `search_text` 存 `None`（富文本/文件投影路径不变）；
2. migration `0004_dedup_search_text.sql`：
   - 存量数据：`UPDATE clipboard_items SET search_text = NULL WHERE kind='text' AND search_text IS NOT NULL AND search_text = content;`（精确相等才置空——html/rtf/files 的投影天然不满足相等条件，零误伤）；
   - 重建 `ai/ad/au` 三个触发器，所有对 FTS 的写入/删除值改为 `COALESCE(<row>.search_text, <row>.content)`（delete 路径同样 COALESCE，保证旧值精确匹配删除——存量行原索引值等于 content，一致）；
3. 触点同步（实施时逐一核对）：`update_clipboard_item_text` 编辑路径、`get_clipboard_item_edit_text`（富文本返回 `search_text` 作纯文本源——纯文本条目需 fallback 到 `content`）、`LIST_SELECT_ITEM` 查询层截断逻辑、备份 merge 行结构不变；
4. 文档注释修正顺带：`db/models.rs:49` `summary` 注释写 512，实际 `SUMMARY_MAX_CHARS = 256`（`ingest.rs:25`）。

**收益**：纯文本行存储 -50%，INSERT/WAL 写入减半，DB 文件与连接页缓存（内存）同步下降。历史越大收益越大。

**风险**：中。牵动 ingest/编辑/查询三处触点 + 一个数据 migration。两个已知边界要在实施时显式处理：
- FTS `integrity-check` 命令会报 mismatch（external-content 表的索引值 ≠ 原表列值）——项目未使用该命令，可接受，需在 migration 注释里写明原因；
- 敏感信息脱敏只作用于出参（`redact_sensitive_list_item`），DB 存原文，行为不变（需回归确认）。

**验收**：纯文本新条目 `search_text` 列为 NULL 且可被全文搜到；编辑后仍可搜；备份导出→导入 merge 往返一致；存量数据 migration 后抽样比对搜索命中数不变。

---

### P1-3 `persist_and_notify` 消除整条目 clone（热路径 4MB 级峰值）

> ✅ **已实施**（a96928d）。签名改为按值接收并返回 `(UpsertResult, ClipboardItem)`；watcher 线程直接 move，`read_clipboard` 用返回值回显，onboarding 旧数据导入同样适配。192 测试全绿。

**问题**：`clipboard/watcher.rs:133` `let mut item_to_write = item.clone()` ——每次复制事件深拷贝整个 `ClipboardItem`，其中 `content` 最大 4MB、`search_text` 再一份。第一轮已做 watcher 单次 Settings 快照，但这个 item clone 是同一条链路上的残留。

**方案**：`persist_and_notify` 签名改拿所有权（`ClipboardItem`）或拆出「source_app_id 赋值」后直接 move 入库，调用方（`watcher.rs:296-320` 同步落库段）相应调整。若调用方后续还要读字段，改为先读后 move。

**验收**：复制大文本（>1MB）时 watcher 线程无额外整结构拷贝（review 确认）；功能回归：去重/排除应用/图片落盘全部不变。

**风险**：低，纯签名重构，`cargo test` 覆盖。

---

### P2-1 备份 merge 导入批量化 —— 万条历史从分钟级到秒级

> ✅ **已实施**（7e2962c）。本库 `(kind, content_hash)` 一次拉取入 HashSet 查重（与 `upsert_item` 同语义），分块 500 行批量 `INSERT OR IGNORE`（10500 绑定参数，低于 SQLite 32766 上限）；`imported` 改为真实 `rows_affected`（语义比旧「尝试数」更准）。backup 测试全过。

**问题**：`backup/mod.rs:1170-1235` merge 导入对每条历史做**逐行**「查重 SELECT + INSERT」往返（10 万条 = 20 万次 DB 往返）；apps/groups/icons 同样逐行（`:1081-1163`）。第一轮评估「低频路径不改」，本轮按用户「全面优化」要求重新排入，但仍在 P2（频率低，只在用户手动恢复备份时发生）。

**方案**：一次性拉取本库 `SELECT kind, content_hash FROM clipboard_items` 构建内存 Set 查重（或对备份 hash 列表分块 `IN` 查询），过滤后用 `QueryBuilder::push_values` 分块批量 `INSERT OR IGNORE`（每块 500 条）；apps/groups/icons 用各自的唯一键先批量 SELECT 再批量 INSERT。仍在单事务内。

**收益**：万条历史导入耗时预计从分钟级降到秒级；事务时长缩短也降低导入期间的 `database is locked` 概率。

**风险**：低-中。备份数据信任模型不变（`.ecopastebak` 自产或加密容器），查重语义从「逐条 SELECT」变为「内存 Set」需保证 hash 语义完全一致（kind+content_hash 二元组，与 `upsert_item` 一致）。

---

### P2-2 数据库空间回收（对标 Ditto 的数据库压缩能力）

> ✅ **已实施**（7c59855）。`clear_clipboard_items` / `delete_clipboard_items` 完成后自动 `PRAGMA wal_checkpoint(TRUNCATE)`；新增 `compact_database` 命令（checkpoint + VACUUM + checkpoint，返回压缩后字节数）注册全链路：设置 → 数据 → 「压缩数据库」按钮（带确认弹层说明耗时与临时磁盘需求），i18n 双语。

**问题**：`cleanup_history`/`clear_clipboard_items` 大量 DELETE 后，SQLite 不会自动收缩 DB 文件，WAL 也持续增长；磁盘空洞与超长 WAL 拖累 checkpoint 时的页缓存与 IO。市面项目（Ditto 的数据库压缩、CopyQ 的维护工具）都把「清理后收缩」作为标配。EcoPaste 目前只有 `clean_resource_cache` 清未引用磁盘文件，**没有任何 DB 收缩入口**。

**方案**：
- `clear_clipboard_items`（用户主动清空，低频且预期明确）完成后执行 `PRAGMA wal_checkpoint(TRUNCATE)`——立即收缩 WAL，成本极低；
- 设置 → 数据页新增「压缩数据库」动作：`PRAGMA wal_checkpoint(TRUNCATE)` + `VACUUM`（重建 DB 文件回收空洞），按钮带确认与「期间不可用」提示；执行放 `spawn_blocking`。不自动定时 VACUUM（成本/收益不划算）。

**验收**：清空 5 万条历史 + 删除对应图片后，DB 文件体积显著下降、WAL 归零附近；VACUUM 后应用功能正常（热路径 SQL 全部回归）；导入导出不受影响。

**风险**：低。VACUUM 需要约等于 DB 大小的临时磁盘空间，按钮文案需说明；`DatabaseState` 的 Mutex 换库语义（overwrite import 用）需确认 VACUUM 期间持锁。

---

### P2-3 退出时清空历史（对标 Maccy `Clear on quit`）

> ✅ **已实施**（7c59855）。采用方案倾向的「同步执行」路径：`ExitRequested` 钩子里 `block_on` 清库删图（退出路径无并发争抢），失败仅记日志不阻断退出；`history.clearOnExit` 设置开关（默认 false）注册到 Record → 历史区块，schema + 类型 + i18n 双语。

**对标**：Maccy 标配；公共电脑/隐私敏感场景的真实需求，与现有 `is_sensitive` 体系互补。

**方案**：`settings.clipboard.history` 增 `clear_on_exit: bool`（默认 false，schema + 前端设置页 Record/History 区块复用现有开关组件 + i18n 双语）；Rust 在 `RunEvent::ExitRequested`/`Exit` 钩子中执行 `clear_items`（复用现有逻辑，含保图片清理）。实现细节：退出钩子有超时风险，采用「同步执行 + 上限等待」并记录日志；若评估后钩子窗口不可靠，降级为「启动时检测上次退出标记执行清理」（实施时二选一，方案倾向前者，Maccy 同路径）。

**验收**：开启后正常退出进程，重新启动历史为空且图片文件同步删除；关闭该选项行为完全不变；强杀进程场景可容忍残留（下次退出补清，不做启动强一致）。

**风险**：中。退出钩子的时间窗口是唯一不确定点，实施时以最小可验证版本落地。

---

### P3 小项打包（一次提交收尾）

> ✅ **部分实施**（7c59855 含前两项）：`enrich_list_item` 合并为单次 Settings 快照；`get_storage_usage` 改单次顶层遍历按子目录分流（删除无引用的 `settings_bytes`）。
> ❌ **评估后不做**：图片二次哈希消除——`content_hash` 的 `"<kind>:<content>"` 语义被备份导入行与既有数据依赖，为省一次对 60 字符文件名的 blake3（<1µs）改公共哈希契约，收益/风险比不成立；`StoredImage::content_digest` 死代码标记——测试用它断言文件名与分片路径，是合理的观测口，保留 `#[allow(dead_code)]` 现状。
> （`delete_items` 的 `#[allow(dead_code)]` 已随 56ade65 移除。）

1. **`enrich_list_item` 双快照**：`commands/clipboard.rs:594-599` 每条 enrich 取两次 `SettingsStore::snapshot()`（`file_entry_limit` + `redact_secrets`），单条事件刷新（`get_clipboard_item`）是热路径——改为一次 snapshot 后按字段取值，与第一轮 watcher 的「一次快照复用」原则对齐；
2. **`get_storage_usage` 重复遍历**：`storage.rs:67-79` 对数据目录重叠 walk 4 次（total/db/resources/settings），改单次遍历按顶层目录分类累加；
3. **清理死代码标记**：`delete_items` 暴露后移除 `#[allow(dead_code)]`；`StoredImage::content_digest`（`storage.rs:40`）确无引用则删除；
4. **图片二次哈希消除**：`ingest.rs:307` 对已是字节哈希的文件名再做一次 `content_hash(kind, content)` ——保持函数语义不变前提下让图片直接复用文件名中的哈希，省一次 blake3（微小，顺带）。

---

## 二、明确不做（同第一轮原则，防止 scope 蔓延）

| 项 | 理由 |
| --- | --- |
| 设备间同步（Ditto 网络/目录式） | 引入网络栈与云依赖，违背 local-first 定位，独立立项 |
| 脚本/自动化/自定义命令（CopyQ commands） | 重功能，插件级架构，独立立项 |
| OCR / 二维码 | 非市场标配（4 产品 3 缺），收益/成本比低 |
| 批量粘贴/合并粘贴 | 多选第一版只做删除；批量粘贴交互复杂且市面罕用 |
| snippet 常驻模板（PasteBar） | 自定义分组 + 置顶已覆盖 80% 场景，不重复建设 |
| Shift+Click 范围选择 | 键盘版 Shift+↑/↓ 已覆盖核心场景，避免与单击行为配置冲突 |
| AppsRegistry 上限 | 记录极小（无图标字节，盘上文件），实测不构成内存问题 |
| 前端内存新优化项 | 180 行窗口 + 16 条/2MB 双预算 LRU + dormant 冻结已达标，无臆造空间 |
| 备份导入流式化（分块 AEAD 解密） | 低频路径，牵动加密容器格式兼容，收益/风险比不划算（维持第一轮结论） |

## 三、实施顺序（建议四批提交）

> ✅ 实际按此顺序完成，全部 8 个提交已入 `wait` 分支：0795cc8 → 56ade65 → b9e4842 → c784e87 → a96928d → 7e2962c → 7c59855。migration 编号顺延为 0003（FTS guard）/ 0004（search_text 去重），未与既有编号冲突。

| 批次 | 内容 | 提交 |
| --- | --- | --- |
| ① P0 快赢 | FTS 触发器条件化 migration + 回归测试 | `perf: skip FTS rebuild on metadata-only updates` |
| ② 功能 | 多选批量删除（Rust command + 前端交互 + 右键菜单 + i18n）+ Shift+数字纯文本粘贴 | `feat: multi-select batch delete` / `feat: shift-number plain paste` |
| ③ 存储 | search_text 双写消除（migration + ingest + 触点）+ persist_and_notify 去 clone | `perf: dedupe plain-text search_text storage` |
| ④ 收尾 | 备份 merge 批量化 + WAL/VACUUM + clear-on-exit + P3 打包 | 按项拆分 |

依赖关系：②、③ 相互独立可并行；③ 的 migration 与 ① 的 migration 编号顺序注意错开（0003/0004）；④ 中 merge 批量化依赖 ③ 落地后的行结构确认（search_text 可为 NULL 对 `BackupItemRow` 反序列化无影响，Option<String> 已兼容）。

## 四、总验收门槛

> ✅ 验收结果：`cargo fmt && cargo clippy --all-targets --all-features -- -D warnings` 零警告；`cargo test` 192 通过（含本轮新增 6 个回归测试）；`pnpm lint` + `tsc --noEmit` 通过；migration 0003/0004 在真实 dev 库（371 条存量）上幂等应用并抽样比对搜索命中不变；`clip.exe` → watcher 入库 → FTS/LIKE 检索 → 去重链路在运行中的应用上端到端验证通过。
> ⚠️ 遗留：GUI 人工点验（多选选中态视觉、Shift+数字粘贴落点、偏好页压缩按钮）因会话注入输入限制未逐项操作——存储层行为已用 dev 库直接验证兜底，UI 交互建议下次 `pnpm tauri dev` 时点验三处：Ctrl+Click 多选高亮、Shift+↑/↓ 扩展、设置→数据→压缩数据库按钮。

- `cargo fmt && cargo clippy -- -D warnings && cargo test` 全绿；
- `pnpm lint` 通过；
- UI 主路径人工点验（Windows 为主）：多选删除、纯文本快捷粘贴、编辑后搜索、退出清空；
- migration 幂等性：0003/0004 在已有存量数据的开发库上跑一遍比对搜索命中与体积变化。

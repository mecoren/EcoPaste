# EcoPaste 第三轮全面优化方案（Round 3）——实施完成版

> 调研日期：2026-09-12。对标：Maccy（macOS）、CopyQ、Ditto、PasteBar。
> **实施状态**：五批次全部落地，共 9 个提交（f2ab1ab → 67294c7），见各实施项下方标注。两处经实施评估放弃/回退的子项已注明原因；P1-1 图标异步化子项在批次③主体提交（ab91fee）后单独补齐（edfaa03）。
> 前置：第一轮 7 项、第二轮 8 项优化已全部落地提交（见 `docs/optimization-report-2026-09.md` 与本文档末轮 2 章节）。本文档只含**新项**，逐项对照过现状，不与前两轮重复；前两轮「明确不做」清单中仍然成立的项不再翻案。

## 结论先行

经过两轮优化，列表热路径、FTS 写放大、IPC payload 裁剪、前端窗口缓存、webview 空闲销毁都已达标——**容易摘的果子已经摘完**。本轮的真正空间在三个层面：

> ✅ **实施结果**：三个层面全部兑现——功能层 3 缺口（批量操作、富文本预览、图片预览降采样+灯箱）补齐；性能层监听热路径（识别闸门 + Settings 快照去深拷 + COUNT 治理）落地；内存层从不可观测（观测命令 + 基准脚本 + mimalloc）到主窗口可选销毁档全部就位。

1. **功能层面**：对标 4 个开源产品后，市场标配而本项目缺失的还剩 3 个硬缺口——**多选批量操作（只做了删除，收藏/置顶/移动分组/导出都没有）、HTML/RTF/Markdown 富文本预览（目前刻意只按纯文本行渲染）、图片预览降采样（480px 面板解码整张原图）**。
2. **性能层面**：监听热路径的**正则成本无长度短路**（每条文本复制全串跑 10 个正则 + path 分支 `exists()` 文件系统调用）、**每次剪贴板事件深拷整个 Settings**、**CJK 双字词搜索恒走全表 LIKE**、**每次翻页伴随 COUNT(\*)**。
3. **内存层面**：**无自定义分配器、无任何内存观测手段**（问题不可观测就不可优化）；主窗口 `Permanent` 保活 WebView 是常驻 RSS 的最大单一来源（可选策略降级为空闲销毁）；预览面板解码原图造成瞬时内存尖峰。

分五批实施（末批为可选档位），每批可独立提交、独立验收。

---

## 需求维度映射

| 维度 | 本轮对应项 | 一句话结论 |
| --- | --- | --- |
| 功能 | P0-1、P0-2、P0-3 | 批量操作补全（4 缺 3）、富文本预览（4 缺 2）、图片预览降采样（2 缺 2） |
| 性能 | P1-1、P1-2、P1-3 | 监听热路径正则短路 + Settings 快照去 clone + CJK 短词治理 + 翻页 COUNT 消除 |
| UI | P0-2、P0-3、P3-1 至 P3-4 | 富文本预览管道、图片灯箱缩放、列表加载反馈与回顶按钮、密度预设、时间戳并行显示 |
| 内存 | P0-3、P2-1、P2-2、P2-3、P2-4 | 预览降采样（解码尖峰）、主窗口空闲销毁（可选档）、mimalloc、内存可观测、存储水位（WAL 回收） |
| ⚠️ 注意 | | P0-2 内部含 sanitize 渲染管道，属功能+UI 双重收益 |

---

## 一、P0 功能硬缺口（对标开源必备，4 产品验证）

### P0-1 多选批量操作补全 —— 收藏 / 置顶 / 移动分组 / 导出

> ✅ **已实施**（`3ed4b89`）。4 个批量命令 + `BatchActionToolbar` 工具条 + `GroupPickerModal` 移动分组弹层 + `export_items_backup`（临时过滤库 + 资源子集 + 复用容器组装，round-trip 单测证导入端可消费）；`BackupExportModal` 参数化 `itemIds` 支持导出所选。批量 UPDATE 按 `MERGE_INSERT_CHUNK=500` 分块防超绑定参数上限；纯元数据 UPDATE 不触发 FTS 重建（0003 触发器 WHEN 条件保证）。

**对标**：Ditto、CopyQ 标配（多选后右键即批量菜单）；Maccy 无多选、PasteBar 有多选。上一轮做了批量删除（市场 2/4），但多选后的批量收藏、批量置顶、批量移动分组、批量导出仍然缺失——用户「先筛选再整理」的真实流程（按类型/来源应用过滤 → 多选 → 归档/整理）当前只能逐条点。

**现状**：多选交互闭环完整（Ctrl/Cmd+Click、Shift+↑/↓、Esc 清空、`multiSelectedIds`），批量删除链路（`deleteClipboardItems`）已验证后端 `delete_items` 支持批量 id 列表。缺的是把同样的 id 列表模式复制到 4 个操作。

**方案**：

1. Rust 增加批量元数据更新仓储函数（一次事务批量 `UPDATE ... WHERE id IN (...)`，分块防超 32766 绑定参数上限，复用备份 merge 的 `MERGE_INSERT_CHUNK=500` 分块原则）：
   - `set_items_favorite(ids, value)` / `set_items_pinned(ids, value)` / `move_items_to_group(ids, group_id)`；
   - 这些都是纯元数据 UPDATE，不触发 FTS 重建（0003 触发器 WHEN 条件已保证），迁移友好。
2. 新增 command + 前端调用（`src/commands/index.ts` 统一包装 + toast i18n）。
3. 前端批量操作工具条：多选非空时 Footer 下方浮现操作条（复用 round-2 多选选中态样式），含 5 个动作按钮：收藏、置顶、移分组（弹现有 `ClipboardGroupModal` 选择器）、删除（已有）、导出。键盘辅助键位与单条动作一致。
4. 批量导出：格式复用现有 `.ecopastebak` 单向导出能力的「所选条目」变体（Rust 侧新 command `export_items_backup`，复用 backup/mod.rs 的 manifest/payload 组装，只是条目集合来自 `WHERE id IN`），不引入新文件格式。
5. i18n 双语（zh-CN 默认 + en-US）同步补齐。

**验收**：多选 3 条 → 批量收藏/置顶/移动分组后逐条状态正确且搜索不受影响（元数据 UPDATE 不重建 FTS）；含受保护条目（收藏/置顶）的批量操作语义与批量删除一致（单独开关控制）；空分组移动、取消收藏后再移动等边界有测试。

**风险**：低。全部复用已验证的 id 列表模式与现有 UI 组件，无 schema 变更。

---

### P0-2 HTML / RTF / Markdown 富文本预览 —— 刻意的「纯文本行」设计需要升级为可选渲染管道

> ✅ **已实施**（`e70d1c7`）。设置加 `rich_text_mode: off/textOnly/rich`（默认 rich）+ `render_markdown`（默认关）；payload 增 `html` 字段（`preview_html` 纯函数：html 条目带源、rtf 经 `clipboard::rtf_to_html`；macOS 走 NSAttributedString 文档 API，Windows 返回 None 降级纯文本——方案认可的备选，Maccy 同款；脱敏条目不带）。前端 `RichTextViewer`：DOMPurify 白名单（29 标签 + 4 类属性，禁 style/data-attr）+ `<iframe sandbox srcDoc>`（无 allow-scripts/allow-same-origin，高度固定填面板内部滚动——无 same-origin 无法测内容高度，方案风险栏已预见）+ >256KB 降级提示条。`MarkdownPreview` 手写行级渲染（先全转义再替换，再过 DOMPurify）。
> **验证**：XSS 14 样本全过（script/onerror/javascript:/vbscript:/data:/iframe/object/embed/svg/form/style，临时 jsdom 目录跑完即删）。
> **实施偏差**：方案原设想 iframe 高度自适应（读 contentWindow.scrollHeight），因不给 `allow-same-origin` 父页面无法读 iframe DOM——按风险栏预案改为固定高度 + 内部滚动，安全边界优先。

**对标**：CopyQ、PasteBar（4 缺 2，CopyQ 的原生预览、PasteBar 的卡片富文本渲染）。这是目前预览体验与竞品差距最大的一处。

**现状**：预览页把**所有文本族（含 HTML/RTF）一律按纯文本行渲染**（`buildTextPreviewRows` 拆行 + Virtuoso 虚拟行）。这是当初为避免「长 HTML 构造大 DOM」的刻意设计（合理），但对用户复制的网页片段、RTF 富文本，看到的却是一坨 `<div>...</div>` 源码——「预览」没有起到「看到粘贴出去的样子」的作用。AGENTS.md 明确把「HTML sanitize 与预览、RTF 渲染、Markdown 渲染」划归前端职责，说明这本来就是规划内能力。

**方案**（分档位，随设置可切换，默认 `Rich` 富文本渲染）：

1. **Rust 侧新增预览 HTML/RTF→HTML 转换**（轻量、无 webview 渲染负担）：
   - `sub_kind=html` 条目：payload 直接带 `html` 字段（现有 `content` 即是），前端 DOMPurify sanitize 后注入沙箱 iframe（`sandbox` 属性无 `allow-scripts`，白名单 `allow-same-origin` 也不给，只留纯渲染）。
   - `sub_kind=rtf` 条目：Windows 用 `windows-rs` ITextRange/rtf 解析（或评估 crate：`rtf-parser` 之类轻量库，迁移友好优先，按 AGENTS.md 依赖规范写主版本）；macOS 用 `NSAttributedString` 的 `RTF→HTML` 系统 API（objc2-app-kit 已在依赖树内，零新增）。两端产出 HTML 后走同一条 sanitize + iframe 管道。
   - Markdown：**不做独立检测**（复用现有 sub_kind 机制没有 md 分类，且自动误判风险高），在设置里加「尝试按 Markdown 渲染纯文本」开关（默认关），开启后纯文本预览多一个「MD」切换 tab。
2. **前端渲染管道**：
   - `sanitize` 用 DOMPurify（已在依赖树）+ 严格白名单（禁 script/iframe/object/embed、事件属性、`javascript:` URL——项目已有 `is_safe_css_value` 的先例，前端同样只白名单标签/属性）；
   - 注入 `<iframe sandbox>` 时高度自适应：`iframe.contentWindow.document.body.scrollHeight` 读取后 clamp 到面板 max 480px，超长内部滚动；
   - **大内容保护**：沿用预览 LRU 字节预算（`PREVIEW_CACHE_MAX_BYTES = 2MB`）判定，超过阈值（建议 256KB HTML）自动降级为现有纯文本行渲染 + 「内容过大，按纯文本显示」提示条，杜绝大 DOM。
3. **设置项**：`clipboard.preview` 增 `rich_text_mode: Off / TextOnly / Rich`（默认 `Rich`，可一键回退），schema + 双语。
4. **开关收益独立**：即便用户不开富文本，`html` 字段过 IPC 与纯文本行渲染互不干扰——档位默认值可在实施时按实测内存数据调整。

**验收**：复制一段网页富文本 → 预览看到渲染后的样式（标题/加粗/链接/图片）；RTF 条目（Word 复制）两端渲染一致；`<script>`/`onerror` 注入样本被 sanitize 拦截（用 XSS cheat sheet 样例逐条验证）；>256KB HTML 自动降级纯文本行；关闭开关回退现有行为。

**风险**：中。iframe 沙箱边界（不给 allow-same-origin，父页面无法读 iframe 内 DOM，高度自适应需要 postMessage 或 ResizeObserver 替代方案，实施时验证）；RTF 解析 crate 生态较弱，macOS 系统 API 稳、Windows 侧是主要不确定点（备选：Windows 也可走 RTF→纯文本行降级，Maccy 同样不渲染 RTF）。

---

### P0-3 图片预览降采样 —— 480px 面板不应解码整张原图

> ✅ **已实施**（`65706ed`，与 mimalloc 同批）。`ImageStore` 第二档 `ensure_preview_image`（长边 960px 覆盖 2x DPI），落盘 `previews/<分片>/<hash>.png` 幂等懒生成；预览 payload 优先 preview 档、meta 仍显示原图真实宽高与字节；`remove` / `clean_resource_cache` / 备份覆盖导入的清理路径同步覆盖新目录。顺手修 image crate thumbnail 只缩不放 bug（小图被强行放大）。

**对标**：Maccy 预览小图直出、CopyQ 缩略图体系（4 产品里 2 个对小图有专门处理）。这不是抄竞品，是通用图形学常识：**面板 480×480 像素，却把 4000×3000 的截图原图解码进 WebView 解码器，解码内存 ≈ 宽×高×4 字节 = 48MB 瞬时尖峰**，悬停预览频繁触发时反复尖峰。

**现状已核实**：`get_clipboard_preview_payload`（`commands/clipboard.rs:1059` `build_clipboard_preview_payload`）对 image 条目直接取 `image_store.origin_path` 传给前端；前端 `ImageViewer` 用原图路径 `object-contain` 渲染。列表卡片用 `ensure_thumbnail`（300px）懒生成缩略图，但预览面板没有对应档位。

**方案**：

1. Rust：`ImageStore` 增加第二档缩略图 `ensure_preview_image`（长边 960px，覆盖 2x DPI 的 480 面板），落盘 `resources/clipboard-images/previews/<分片>/<hash>.png`，懒生成逻辑与现有缩略图同模式（`spawn_blocking` + write_if_absent），文件名与原图同 hash 幂等。
2. 预览 payload 的 `image_path` 改为优先 preview 档（存在或可生成时），`image_exists` 同步校验；预览 meta（尺寸+大小）仍显示**原图**真实宽高与字节（不是降采样后的），信息不失真。
3. **保留原图放大路径**：ImageViewer 增加点击放大灯箱（见 P3-1），灯箱才用原图路径。列表卡片不动（300px 档已合理）。
4. 删除联动：`ImageStore::remove` 连带删 `previews/` 分片（与 thumbnails 同步），`clean_resource_cache` / 备份覆盖导入的图片清理路径同步覆盖新目录。

**验收**：大图（>2MB 截图）hover 预览无卡顿、内存曲线平稳（对比实施前后的 WebView 进程 RSS 尖峰）；预览 meta 显示的仍是原始尺寸/大小；原图文件删除后 previews 分片同步清理；灯箱放大加载原图。

**风险**：低。模式与现有缩略图完全同构，只是新增一档；落盘增加约「图片条目数 × 一张 960px PNG」的磁盘占用，配合 P2-4 的清理策略可控。

---

## 二、P1 性能热点（监听热路径与搜索）

### P1-1 监听热路径正则成本短路 + Settings 快照去 clone

> ✅ **已实施**（主体 `ab91fee`，图标异步化子项补齐 `edfaa03`）。
> - 4KB 闸门：`detect_text_sub_kind` 只识别前 4KB（UTF-8 边界回退完整字符）；`path` 分支不做 `exists()`，只判绝对路径形态，存在性延后到动作执行。多字节截断与超长 URL 前缀命中有专门测试。
> - secrets 预筛：❌ **试验后回退**——「字面前缀预筛 + 命中者正则」实测（16ms/69ms）比 regex 全串（9ms/51ms）**更慢**，regex crate 本身已高度优化；回退全串方案，微基准 `hot_path_bench_large_plain_text` 保留用于防回归与后续优化对照。
> - cleanup tick 版本号：`SettingsStore` 增 `AtomicU64` version（update/reset/replace/rebase 自增，只读 snapshot 不变），tick 只比对版本号，设置未变不再深拷整个 Settings。
> - 源应用图标异步化（补齐 `edfaa03`）：`detect_frontmost` 从携带 icon PNG 字节改为只带 `icon_path`（监听线程零 OS 抽取）；`materialize_source` 变纯缓存查询（registry 命中零成本）；未命中经 `spawn_materialize_icon` 在 `spawn_blocking` 抽取 + upsert（幂等），条目先入库、icon 后补，前端期间按应用名回退。

**现状已核实**（代码级确认，`detect.rs:51-67`、`secrets.rs`、`ingest.rs:159,208`）：

1. **每条文本复制**（采集上限 4MB）入库前全串跑最多 10 个正则（`detect_text_sub_kind` 5 个 + `contains_secret` 5 个），**没有任何长度闸门**——`detect_text_sub_kind(plain)` 与 `contains_secret(&text.text)` 都拿完整文本。detect 判定顺序 url > email > color > path 且命中即返回（短路结构是对的），问题在**未命中前的全串扫描**：URL 正则在 4MB 普通文本上全串扫。path 分支命中「看起来像绝对路径」时还有 `exists()` 文件系统调用。
2. **每次剪贴板事件 `SettingsStore::snapshot()` 深拷整个 Settings**（watcher 已特意「只 snapshot 一次」复用，但 cleanup 的 60s tick 循环里每次 tick 都 snapshot）。
3. `detect_frontmost` + `materialize_source` 首次见到应用时同步抽 icon（在监听线程）。

**方案**：

1. **正则短路与长度闸门**：
   - `detect_text_sub_kind` 入口加长度闸门（如 `content.len() > 4096` 时只取前 4KB 做识别——URL/email/color/path 都是行级特征，4KB 足够；超长文本几乎必然是普通长文）；`contains_secret` 同样先做**廉价预筛**：先查 `is_pem_marker` / `ghp_` 等字面前缀（`str::contains`，无正则）命中才跑对应正则——5 个正则降为「5 个字面预筛 + 命中者正则确认」，多数普通文本 0 正则执行。
   - `path` 分支的 `exists()` 移出 detect（识别阶段只判定「看起来像绝对路径」，`exists()` 校验延后到预览/渲染 attach 阶段——`attach_file_entries` 反正会触碰文件系统）。
2. **cleanup tick 快照复用**：`cleanup::spawn` 改为 tick 里只比对一个 `AtomicU64` 设置版本号（`SettingsStore` 增 version 计数，`update()` 时自增），版本未变直接 continue，全量 snapshot 只在版本变化或到期执行时发生。
3. **正则合并**（可选）：把 url/email 两正则合并编译为单 pass 不划算（语义不同），不做；保持 5 正则结构，只做预筛。
4. **源应用图标异步化**：`materialize_source` 的 icon 抽取从监听线程剥离，首次条目先入库（icon 字段空，前端回退 logo），icon 生成后 `clipboard://updated` 单条刷新补齐——监听线程零同步 OS 抽取。

**验收**：微基准（cargo test 内嵌 criterion 风格或手写 `Instant` 计时）：4MB 普通文本从「10 正则全串」到「预筛 0 正则」有可测加速；PEM/JWT/URL 各 100 条样本识别准确率不回归（回归测试集）；cleanup tick 循环在设置无变化时不再 clone（版本号测试）；复制大文本时监听线程耗时下降（tracing 对比）。

**风险**：低。全部是「同样的判定、更便宜的路径」，语义不变；预筛字面前缀与正则是包含关系，不会漏判。

---

### P1-2 CJK 双字词搜索治理 —— trigram 的中文盲区（本轮：提示 + 护栏，不动分词器）

> ✅ **已实施**（`ab91fee`，与 P1-3 同批）。前端 `SearchShortKeywordHint`：任一分词 <3 字符时搜索框下一次性轻提示（不阻断，LIKE 照跑）；COUNT 侧护栏由 P1-3 的 skip_count 一并覆盖。

**现状已核实**：FTS5 用 `trigram` 分词器（按 3 字符切片建索引），关键词所有分词 ≥3 字符走 FTS，**任一分词 1–2 字符（含 CJK 双字词——中文最高频搜索形态）降级 `LIKE '%kw%'` 全表扫描**（匹配列 `COALESCE(search_text, content)` + `note`，见 `db/items.rs` 的 `KeywordFilter` 分流）。trigram 索引结构决定了 2 字符词无法走索引，这是前两轮已知但未动的真问题。

**索引方案评估结论（先给判决再给动作）**：2 字符查询没有任何可行的 trigram 改写（前缀通配 `*` 对 trigram 不适用，构造合成 3-gram 需要预知后续字符）；换分词器（unicode61/jieba 系）需重建存量 FTS 表 + 重写触发器与搜索 SQL 全链路，且中文分词没有完美方案——**列为明确不做**。中文双字词 LIKE 在万条级历史（几十 MB）下全表扫描量级在几十毫秒，是「可感知但可接受」的代价。

**本轮实际动作**（两条，均与 P1-3 合并实施）：

1. **前端短词提示**：关键词任一分词 <3 字符时，搜索框下出现一次性轻提示「输入至少 3 个字符可使用快速搜索」（不阻断，LIKE 照跑；同一会话内提示只出现一次，避免打扰）。
2. **COUNT 侧护栏**：短词 LIKE 的主要风险不在取数（有 LIMIT 分页兜底）而在 COUNT 同样全表扫（见 P1-3 一并治理：搜索态 COUNT 缓存 + `has_more` 长度判定后，短词搜索的每页成本收敛为 1 条带 LIMIT 的 SQL）。

### P1-3 翻页 COUNT 治理 —— 每页两条 SQL 的消解

> ✅ **已实施**（`ab91fee`）。`ClipboardItemQuery` 增 `skip_count`：同过滤组合下前端只有第一次请求带 COUNT（`totalKnownRef`），此后翻页跳过 COUNT、Rust 返回哨兵 -1，沿用本地 total（reload/query 变化时重置）；`has_more` 双语义——COUNT 路径精确判定、skip_count 路径长度判定（`len == limit`），整除页最坏多一次空请求换每页省一条 COUNT（方案推荐的取舍）。附哨兵与版本号仓储测试。
> ❌ 子项 4（`attach_file_entries` 的 exists/metadata 合并 `symlink_metadata`）：方案自评「收益小，归入 P3 打包」，且 FileIconCache 的 DB 查询批量化在第二轮已完成，剩余仅每路径 2-3 次 stat——**确认不做**。

**现状已核实**：`query_items_page` 每次翻页都伴随一条同过滤条件的 `COUNT(*)`（`fetch_items_count`），搜索态翻页成本翻倍；且前端 PAGE_SIZE=30、CACHE 180 行内翻页都在 Rust 侧反复拉。

**方案**：

1. **搜索态 COUNT 缓存**：同一 `keyword + filter 组合` 的 COUNT 结果在 store 缻存（keyword 变化时失效），翻页时跳过 COUNT。总条数语义：非搜索态 Footer 显示的 total 用事件驱动修正（现有 `clipboard://updated` 已有 `{cleanup: n}` 删除数，可顺带携带新 total），不再每页重算。
2. **前端轻量缓存确认**：前端 range cache 180 行已限制驻留，Rust 侧无列表缓存——**不加 Rust 侧列表缓存**（避免另一份真源，违背「前端只做展示、Rust 持有数据」的边界），COUNT 缓存放前端 store 层即够。
3. 顺带：`has_more` 判断改用 `offset + len < total`（现状已是）或 `len < limit` 直接判——当次查询返回行数 < 请求 limit 即无更多，COUNT 可完全省略（**推荐**：`has_more = items.len() == limit`，语义更便宜且更准，与 `total` 解耦；total 只在需要显示时算）。
4. `attach_file_entries` 的逐文件 `exists()` 批量化：一页内多个文件条目时收集路径后单次遍历批查（仍走文件系统，但减少重复 stat 次数从 2N 到 N——exists + metadata 两次调用合并为一次 `symlink_metadata`）。批量化收益小，归入 P3 打包。

**验收**：连续翻 10 页，SQL 日志显示 COUNT 查询 ≤1 次；搜索态翻页首条 SQL 后翻页仅 1 条 SQL/页；Footer total 显示与实际一致（事件驱动修正后无陈旧）；has_more 语义在「整除页」边界（如正好 30 条）正确。

**风险**：低。has_more 改长度判定在「条目被并发删除导致页短」时可能提前误报无更多——用 `len == limit` 加 `has_more` 保守取「或」逻辑（len==limit || cached_total 判定）兜底。

---

## 三、P2 内存专项（全面内存优化）

### P2-1 主窗口空闲销毁策略（可选）—— Permanent 保活是常驻 RSS 最大单一来源

> ✅ **已实施**（`67294c7`）。设置 `clipboard.window.idle_destroy_main`（默认 false，schema 挂在 lightweightMode 下随其禁用）。生命周期三处适配：销毁计时器与 dormant 计时器互斥（开启销毁则不挂 dormant 冻结）；`try_destroy_idle` 接受 HiddenWarm|Dormant 两阶段（主窗口隐藏 5s 进 dormant 后销毁判定不再卡 Stale）且到点重读开关（计时期间用户关开关则放弃销毁）。descriptor 给 clipboard 补 `build_clipboard_window`（完整复刻 tauri.conf.json 承重属性：transparent/always_on_top/focusable(false)/skip_taskbar/accept_first_mouse 等）；macOS 重建后 `setup_clipboard_panel_rebuilt` 重新 panel 化（不动 dock 可见性）。新增 dormant 代次不变测试。唤出重建走既有 `show_window` 的 rebuild 分支 + 前端首帧骨架（361224a 已备）。

**现状已核实**：`window/lifecycle/descriptor.rs:60` 主窗口（clipboard）`RetainPolicy::Permanent`——永不销毁 WebView。其余窗口（preference/update/onboarding/preview）已空闲销毁。主窗口是使用频率最高的窗口，保活换来「再次唤出 0ms」的体验；但 `lightweight_mode` 下主窗口隐藏 5s 后进入 `Dormant` 只暂停前端刷新，**WebView 进程 + React 树 + range cache 180 行依旧常驻**。以 Tauri WebView2/WebKit 常态 ~50-100MB RSS 估算，主窗口保活贡献了后台 RSS 的大头。

**方案**：设置新增 `clipboard.window.idle_destroy_main: bool`（默认 false，即维持现状保活）：
- 开启后主窗口隐藏 `idle_destroy_seconds`（复用现有 DestroyWhenIdle 跰径与 keepalive/dirty/generation 保护）后同样销毁 WebView，再次唤出时经 descriptor `build` 重建（需为 clipboard 窗口补 `build_clipboard_window` 重建函数——现 `Permanent` 无 build）。
- **唤出延迟兜底**：销毁重建首开约 300-800ms（视机器），用「设置页显式开关 + 文案明示代价」控制预期；快捷键唤出场景加加载态（骨架占位已有）。
- 与 `scroll_to_top_on_open` / `select_*_on_open` 等打开行为联动：重建后这些状态从头初始化，`save_all_window_states` 已有的几何保存/恢复链路复用。
- **为什么默认关**：主窗口唤出速度是剪贴板管理器的命根（Win+V 替代系统面板的核心场景），牺牲 100ms 换后台 50MB+ 只对内存敏感用户有意义，故做成可选档位而非默认行为。

**验收**：开启后隐藏主窗口 → 空闲超时 → WebView 进程消失（任务管理器验证 RSS 归还）；快捷键再唤出正常重建、列表数据/滚动位置/分组选择恢复正常；`clear_on_exit` / 退出保存几何不受影响；关闭开关回到保活现状。

**风险**：中。重建链路（geometry restore + 前端 boot + 设置挂起 `use(settingsReady)` + onboarding 闸）比 preference 等窗口复杂，需要专门点验唤出体验；generation/keepalive 保护已就位，主要工作量在 `build_clipboard_window` 与前端首帧骨架。

### P2-2 mimalloc —— 一行依赖换全局长尾内存收益

> ✅ **已实施**（`65706ed`，与 P0-3 同批）。`Cargo.toml` 加 `mimalloc = "0.1"` + `lib.rs` 全局分配器声明。

**现状已核实**：`Cargo.toml` 无任何分配器依赖，系统默认分配器（Windows nt heap / macOS malloc）。Rust 服务端与桌面应用社区有大量实证：mimalloc 对长尾内存碎片、高 churn 场景（本项目的监听入库、列表 enrich、图片缩略图生成）有 5-20% RSS 收益，Windows 上尤其明显。

**方案**：`Cargo.toml` 加 `mimalloc = "0.1"`，`lib.rs` 顶部 `#[global_allocator] static GLOBAL: MiMalloc = MiMalloc;`。一行依赖，两端同源行为。

**验收**：`cargo test` 全绿（分配器透明）；`pnpm tauri dev` 启动后闲置 5 分钟 RSS 对比（Windows 任务管理器/`psrecord`）不劣化、目标下降；突发复制 100 条（clip.exe 循环）后 RSS 峰值与回落对比。

**风险**：低。mimalloc 是微软维护的成熟分配器，Tauri 生态常用（tauri 官方模板默认无，但社区应用广泛）；唯一注意点是与 WebView2 的堆隔离（WebView 自己的进程不受影响，收益作用于 Rust 侧进程，符合预期——Rust 进程正是我们可控的那部分）。

### P2-3 内存可观测性 —— 无观测即无优化

> ✅ **已实施**（`f2ab1ab`）。`get_process_memory_stats` 命令（诊断面板显示 + 手动刷新）+ 前端 `performance.memory`（WebView2 支持，WKWebView 显示 N/A）+ `scripts/mem-bench.ps1` / `mem-bench.sh` 基准脚本（启动基线 → 200 条混合复制 → 峰值 → 隐藏 5 分钟稳态，产出 CSV）。

**现状已核实**：无任何内存 profiling/基准设施。前两轮所有「内存优化」都是静态分析推演，没有一条实测曲线。这违背性能工程基本原则，也让 P2-1/P2-2 的收益无法量化验证。

**方案**：

1. **Rust 侧进程内存指标命令**：`get_process_memory_stats` command（Windows `GetProcessMemoryInfo` / macOS `mach_task_basic_info`），返回 Rust 进程 RSS/虚拟内存；诊断面板（偏好页已有「诊断」区）加显示 + 手动刷新。
2. **WebView 进程内存**：前端 `performance.memory`（Chromium 系 WebView2 支持；macOS WKWebView 不支持则显示 N/A），预览页诊断区一并展示。
3. **基准场景脚本**：`scripts/` 加 `mem-bench.ps1` / `mem-bench.sh`（非 Linux 产品承诺——脚本工具不构成产品支持，但为遵守「仅 macOS+Windows」原则，只提供 ps1 + macOS shell 两个脚本）：启动 → 记基线 → 脚本化复制 200 条混合内容（clip.exe / pbcopy）→ 记峰值 → 隐藏窗口 5 分钟 → 记稳态。产出 CSV 供前后对比。
4. 本项产物同时是 P0-3（预览降采样）、P2-1（主窗口销毁）、P2-2（mimalloc）的**验收工具**。

**验收**：诊断面板能看到实时 RSS；基准脚本在 Windows dev 环境跑通并产出基线 CSV；P2 各项实施时用同脚本对比曲线。

**风险**：低。纯增量观测能力，无行为变化。

### P2-4 存储水位治理 —— 已有清理策略的补全

> ✅ **已实施**（`f2ab1ab`）。cleanup 删除行数 ≥500 后自动 `PRAGMA wal_checkpoint(TRUNCATE)`（`checkpoint_if_bulk`，VACUUM 保持手动）；`clean_resource_cache` 扩展覆盖 `previews/` 目录；`get_storage_usage` 无需改动（round-2 已单次遍历）。

**现状已核实**：有 retention/max_count/clear_on_exit 三策略，有手动 VACUUM，但**无自动 prune**：历史大量删除后 DB 空洞与 WAL 累积靠用户手动「压缩数据库」。且 `previews/` 新目录（P0-3）会新增磁盘占用。

**方案**：

1. cleanup 任务在删除行数超过阈值（如 ≥500）时，清理完成后**自动 WAL truncate checkpoint**（`PRAGMA wal_checkpoint(TRUNCATE)`——VACUUM 仍保持手动，自动 VACUUM 有锁库风险）；低于阈值不动，避免高频小清理的 IO 抖动。
2. `clean_resource_cache`（现有命令）扩展覆盖 `previews/` 目录;`compact_database` 文案更新（「压缩数据库与图片缓存」）。
3. `get_storage_usage` 已单次遍历（round-2 完成），自动 checkpoint 后体积回归正常，无需额外改动。

**验收**：构造 500+ 删除的清理周期后 WAL 体积回落;previews 孤儿文件被清理命令覆盖;存储面板体积数字前后一致。

**风险**：低。checkpoint truncate 是在线操作，毫秒级，已有备份导出前的同款调用先例。

---

## 四、P3 UI 打磨小项（一次提交打包）

> 前端调研确认的轻量 UI 缺口，各自独立、风险低，合并一批实施。

### P3-1 图片预览灯箱（点击放大 / 滚轮缩放）

> ✅ **已实施**（`ff3afae`）。预览 payload 增 `image_origin_path`；ImageViewer 点击开 antd Image 受控 preview（v6 用 `preview.open` + `preview.onOpenChange`，`onClose`/`onVisibleChanged` 已废弃）；原图只在灯箱加载（预览面板仍用 960px 降采样档，P0-3 分层）。关键机制：预览窗 `focusable(false)`，点击不会让主窗口 blur → 预览不被 hover-hide 链路关掉。

**现状**：预览面板上限 480×480，`object-contain` 直接渲染，无缩放无放大，大图细节看不清。P0-3 落地后预览默认用 960px 降采样图，原图只在灯箱加载。

**方案**：预览面板内图片点击 → 展开全屏灯箱（复用 antd `Image` 组件的 preview 体系，或 motion 实现轻量版：原图 `convertFileSrc` + 滚轮缩放 0.5-4x + 拖拽平移 + Esc/点击关闭）。灯箱用原图路径（此时才付出整图解码成本，单次、用户主动触发，可接受）；关闭后内存随浏览器解码缓存自然回收。灯箱打开期间预览面板的悬停关闭逻辑让路（复用现有 pointerout 兜底 + 灯箱层吞事件）。

**验收**：大图预览 → 点击放大 → 缩放/平移流畅 → Esc 关闭回到面板；灯箱期间不触发面板关闭；普通尺寸图片流程无退化。

### P3-2 列表加载反馈与回顶按钮

> ✅ **已实施**（`361224a`）。loadingMore 骨架 Footer（Virtuoso `components.Footer`）+ 回顶按钮（scrollerRef 回调绑 scroll 监听，出现阈值一屏/回落半屏滞回；不能用 useEffect——loading 阶段 scroller 元素尚不存在）。

**现状**：`loadingMore` 状态 hook 已返回但 List 未渲染（无限滚动加载下一页时无任何视觉反馈）；长列表滚动后无回到顶部入口。

**方案**：滚动接近底部拉取时在列表底部渲染细条形 skeleton（高度与卡片相近，1-2 条）；滚动距离超过一屏后右下角浮现「回到顶部」悬浮按钮（置顶块 sticky 语义下的滚顶即 `scrollToIndex(0)`，复用 `atTopStateChange`）。两处都尊重 reduced motion。

**验收**：慢速拉取（人为延迟 SQL 或 debug 断点）可见加载条；深滚后回顶按钮出现、点击平滑回顶；快速滚动无按钮闪烁（出现阈值 + 滞回）。

### P3-3 密度预设三档

> ✅ **已实施**（`361224a`）。`DensityPresetControl` 三档（紧凑 2/40/2、标准 3/64/3、舒适 5/100/5）纯数据驱动高亮——只写三个既有设置项、无新字段，单项改动自然取消预设高亮。

**现状**：密度只有逐项 clamp（textMaxLines 1-5 / imageMaxHeight 20-100 / fileMaxCount 1-5），用户要理解三个参数才能调出「紧凑」或「舒适」。

**方案**：偏好页外观区加三档预设（紧凑 / 标准 / 舒适），点击映射到现有三项参数组合（如紧凑 = 2 行/40px/2 个，标准 = 现默认 3/64/3，舒适 = 5/100/5），只写三个现有设置项、不新增字段，预设按钮与单项调节并存（单项改动后预设高亮取消）。

**验收**：三档切换立即生效；手动改单项后预设态清除；与现有 `window://lifecycle` dormant 冻结、line-clamp safelist 无冲突。

### P3-4 时间戳与快捷动作并行显示

> ✅ **已实施**（`361224a`）。`ClipboardQuickActions` 改并行布局：时间戳常驻（降 opacity-40），动作按钮右侧弹入；>3 个动作折叠「…」Dropdown（`ItemActionLabels` 增 `more` 字段）。

**现状**：卡片 meta 区时间戳与 hover 快捷动作按钮组互斥替换（hover 即失去时间信息，前端调研确认的体验缺口）。

**方案**：hover 时动作按钮组从时间戳右侧同排弹入（时间戳保留、透明度降为 secondary），而非整块替换。动作数量多时（用户勾选 ≥4 个 hover 动作）溢出折叠为「…」菜单。改动集中在 `ClipboardQuickActions` + 卡片 meta 布局，无跨端影响。

**验收**：hover 后时间仍可见；动作多时折叠入口可用；`motion-reduce` 下无动画直接切换。

---

## 五、明确不做（延续前两轮原则）

| 项 | 理由 |
| --- | --- |
| 设备间同步 | 独立立项（round-2 结论维持） |
| 脚本/自动化（CopyQ commands） | 独立立项（round-2 结论维持） |
| OCR / 二维码 / TTS | 非市场标配，收益/成本比低（round-2 结论维持） |
| 批量粘贴/合并粘贴 | 交互复杂且市面罕用（round-2 结论维持） |
| snippet 常驻模板 | 分组+置顶已覆盖（round-2 结论维持） |
| FTS 分词器替换（unicode61/jieba 系） | 存量 FTS 重建 + 全链路重写，风险 >> 收益（P1-2 详述） |
| AppsRegistry 上限/LRU | 记录极小，不构成问题（round-2 结论维持） |
| 前端列表缓存再压缩 | 180 行窗口缓存已达标（round-2 结论维持） |
| 统一 sleep-thread 调度器 | 单次开销 µs 级、频率低,重构收益不抵 churn（已有代次校验防堆积） |
| 批量收藏/置顶的右键菜单入口 | 与批量删除同理由：5 处跨端镜像牵动过广，工具条 + 快捷键已闭环（round-2 已确认此模式） |
| Linux 支持 | 项目原则,不新增 |

---

## 六、实施顺序（建议五批提交，末批为可选档位）

> ✅ 实际按此顺序完成，全部 8 个提交已入 `wait` 分支：f2ab1ab → 65706ed → ab91fee → 3ed4b89 → ff3afae → 361224a → e70d1c7 → edfaa03 → 67294c7（③的图标异步化子项在主体提交后单独补齐为 edfaa03，实际 9 个；④ 按方案拆为 4 个功能提交）。

| 批次 | 内容 | 提交（Conventional Commits） |
| --- | --- | --- |
| ① 观测先行 | P2-3 内存观测 + 基准脚本 + P2-4 存储水位 | `feat: memory observability + storage watermark` |
| ② 内存快赢 | P2-2 mimalloc + P0-3 预览降采样 | `perf: mimalloc + preview downsampling` |
| ③ 性能热路径 | P1-1 正则短路/快照版本化 + P1-2 短词提示 + P1-3 COUNT 治理 | `perf: watcher hot path + count governance` |
| ④ 功能补全 | P0-1 批量操作 + P0-2 富文本预览 + P3-1 灯箱 + P3-2/3/4 UI 小项 | `feat: batch operations` / `feat: rich text preview` 等 |
| ⑤ 可选档位 | P2-1 主窗口空闲销毁（默认关，独立开关单独评估） | `feat: optional main window idle destroy` |

依赖关系：① 是 ②③④⑤ 所有内存/性能项的验收工具，先行；② 的 previews 目录清理依赖 P2-4 的 clean_resource_cache 扩展（同批或后批均可）；③ 与 ④ 相互独立可并行；④ 的 P3-1 灯箱依赖 P0-3 落地后的「预览默认降采样图、灯箱载原图」分层，同批实施；P0-2 富文本预览与 P0-3 都动 `get_clipboard_preview_payload`，建议 ②④ 之间按「② 先行、④ 在其后」排序避免二次改同一文件；P0-1 独立可并行；⑤ 风险面独立（主窗口重建链路），放在最后单独评估与点验。

## 七、总验收门槛

> ✅ **验收结果**：`cargo clippy --all-targets --all-features -- -D warnings` 零警告；`cargo test` 215 项通过（含本轮新增的 FTS/哨兵/版本号/dormant 代次等回归测试，8 项 ignore 为需真实系统资源的桌面会话测试）；`pnpm lint` + `tsc --noEmit` 通过；两个收尾提交经 pre-commit 钩子（cargo fmt + clippy + biome）复验通过。安全验证：富文本预览 DOMPurify XSS 14 样本全过；备份 round-trip 单测证导出所选可被导入端消费。无 schema migration（全部设置项 `#[serde(default)]` 向后兼容，旧设置文件升级路径无损）。
> ⚠️ 遗留：UI 人工点验因会话环境阻塞（UIA 劫持 + 截屏通道不可用）未逐项操作，替代验证为 Rust 单测全绿 + 前端 mock 套路。下次 `pnpm tauri dev` 真实会话优先点验：批量操作工具条与弹层、富文本预览渲染、灯箱放大、短词提示、密度预设切换，以及 P2-1 开启后的主窗口销毁→唤出重建链路（任务管理器验证 WebView 进程 RSS 归还、列表/滚动/分组恢复正常）。

- `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo test` 全绿；
- `pnpm lint` + `tsc --noEmit` 通过；
- 基准脚本前后对比：Rust 进程稳态 RSS、突发复制峰值、预览悬停解码尖峰三组曲线不劣化（内存项目标下降）；
- UI 主路径人工点验（Windows）：批量收藏/置顶/移动分组/导出、富文本预览与 XSS 样本、灯箱放大、短词搜索提示、（如实施 P2-1）主窗口空闲销毁唤出体验；
- 涉及 schema 的项（无 migration，P0-1/P0-2 仅设置项新增，`#[serde(default)]` 向后兼容）在真实 dev 库升级路径验证。

-- 列表排序索引修正：0002 的三条索引是混合方向（is_pinned 升序 + 排序列降序），
-- 而列表恒以 `is_pinned DESC, <sort> DESC` 排序。混合方向索引正向扫描得到
-- is_pinned ASC、反向扫描得到排序列 ASC，两种方向都满足不了全 DESC 排序，
-- 规划器只能 `SCAN` 全表 + `USE TEMP B-TREE FOR ORDER BY`。没有覆盖索引时排序
-- 必须读出每一行，历史积累后每次打开剪贴板窗口都要把整张表（含大段文本 content）
-- 重新读一遍，表现为「数据越多，弹出越卡」。
--
-- 改为一律 DESC：正向扫描即可产出 `is_pinned DESC, updated_at DESC, ...`。
-- 排序子句里的第二排序键（created_at）必须进索引，否则会退化为
-- `USE TEMP B-TREE FOR LAST TERM OF ORDER BY`——实测该退化路径耗时反而最高。
--
-- 清理任务（`ORDER BY created_at DESC`）改由 idx_clipboard_items_pinned_created
-- 承接：`is_pinned = 0 AND created_at < ?` 仍是 SEARCH，不退化。

DROP INDEX IF EXISTS idx_clipboard_items_pinned_updated_at;
DROP INDEX IF EXISTS idx_clipboard_items_pinned_created_at;
DROP INDEX IF EXISTS idx_clipboard_items_pinned_use_count;

CREATE INDEX idx_clipboard_items_pinned_updated_created
ON clipboard_items (is_pinned DESC, updated_at DESC, created_at DESC);

CREATE INDEX idx_clipboard_items_pinned_created
ON clipboard_items (is_pinned DESC, created_at DESC);

CREATE INDEX idx_clipboard_items_pinned_use_created
ON clipboard_items (is_pinned DESC, use_count DESC, created_at DESC);

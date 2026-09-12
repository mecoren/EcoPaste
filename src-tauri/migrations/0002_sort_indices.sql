-- 列表排序与历史清理的支撑索引。
-- `ORDER BY is_pinned DESC, <sort列> DESC` 与清理任务 `ORDER BY created_at DESC` 此前均无索引，
-- 历史增长后每次翻页 / 清理都要全表排序。置顶过滤列固定为 is_pinned，直接并入索引首列。

CREATE INDEX idx_clipboard_items_pinned_updated_at ON clipboard_items (is_pinned, updated_at DESC);
CREATE INDEX idx_clipboard_items_pinned_created_at ON clipboard_items (is_pinned, created_at DESC);
CREATE INDEX idx_clipboard_items_pinned_use_count ON clipboard_items (is_pinned, use_count DESC, created_at DESC);

-- FTS UPDATE 触发器条件化：只在被索引列（search_text / note）的值真正变化时才重建
-- 全文索引行。此前任何 UPDATE（复用计数、收藏、置顶、分组、备注清空等纯元数据
-- 更新）都会对 trigram 索引执行一次 delete + insert，产生无效的索引重写与 WAL 放大。
-- `WHEN` 值比较比 `UPDATE OF <列>` 更稳：后者在「SET 了但值没变」时仍会触发。

DROP TRIGGER clipboard_items_au;
CREATE TRIGGER clipboard_items_au AFTER UPDATE ON clipboard_items
WHEN new.search_text IS NOT old.search_text OR new.note IS NOT old.note
BEGIN
    INSERT INTO clipboard_items_fts(clipboard_items_fts, rowid, search_text, note)
    VALUES ('delete', old.rowid, old.search_text, old.note);
    INSERT INTO clipboard_items_fts(rowid, search_text, note)
    VALUES (new.rowid, new.search_text, new.note);
END;

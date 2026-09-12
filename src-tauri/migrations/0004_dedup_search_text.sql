-- 纯文本 search_text 双写消除。
--
-- 纯文本条目入库时 `search_text` 与 `content` 是同一字符串（富文本的
-- `search_text` 是纯文本投影、files 的是 basename 投影，二者不同串）。
-- 一条 4MB 文本等于在表里存 8MB，FTS trigram 索引再放大 ~3x。
--
-- 方案：纯文本条目 `search_text` 置 NULL，三个 FTS 触发器统一改用
-- `COALESCE(search_text, content)` 作为索引列——external-content FTS 表
-- 写入什么完全由触发器决定，查询不读原表列，行为不变。
-- 注意：`fts5cmd integrity-check` 会报 external-content mismatch（索引值
-- 不等于原表列值），本项目未使用该命令，可接受。

-- 存量去重：仅精确相等才置空——html/rtf/files 的投影天然不满足
-- `search_text = content`，零误伤。
UPDATE clipboard_items
SET search_text = NULL
WHERE kind = 'text' AND search_text IS NOT NULL AND search_text = content;

DROP TRIGGER clipboard_items_ai;
CREATE TRIGGER clipboard_items_ai AFTER INSERT ON clipboard_items BEGIN
    INSERT INTO clipboard_items_fts(rowid, search_text, note)
    VALUES (new.rowid, COALESCE(new.search_text, new.content), new.note);
END;

DROP TRIGGER clipboard_items_ad;
CREATE TRIGGER clipboard_items_ad AFTER DELETE ON clipboard_items BEGIN
    INSERT INTO clipboard_items_fts(clipboard_items_fts, rowid, search_text, note)
    VALUES ('delete', old.rowid, COALESCE(old.search_text, old.content), old.note);
END;

-- 重建 0003 条件化的 UPDATE 触发器：索引列改为 COALESCE 值后，「值变化」
-- 判定同样基于 COALESCE（content 编辑也要重建索引）。
DROP TRIGGER clipboard_items_au;
CREATE TRIGGER clipboard_items_au AFTER UPDATE ON clipboard_items
WHEN COALESCE(new.search_text, new.content) IS NOT COALESCE(old.search_text, old.content)
  OR new.note IS NOT old.note
BEGIN
    INSERT INTO clipboard_items_fts(clipboard_items_fts, rowid, search_text, note)
    VALUES ('delete', old.rowid, COALESCE(old.search_text, old.content), old.note);
    INSERT INTO clipboard_items_fts(rowid, search_text, note)
    VALUES (new.rowid, COALESCE(new.search_text, new.content), new.note);
END;

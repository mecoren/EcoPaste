-- 跨表示去重的查找索引：text 行的「纯文本投影」COALESCE(search_text, content)。
--
-- 纯文本行 search_text 为 NULL（migration 0004），投影即 content；HTML/RTF 行的
-- search_text 是 OS 提供的纯文本表示。同一段可见文字先后以不同表示复制时
-- content_hash 不同，但投影一致——入库去重按投影补查既有条目并提升它，
-- 而不是新插一行。部分索引（kind = 'text'）避免 files/image 行参与投影维护。

CREATE INDEX IF NOT EXISTS idx_clipboard_items_plain_projection
ON clipboard_items (COALESCE(search_text, content))
WHERE kind = 'text';

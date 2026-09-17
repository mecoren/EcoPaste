//! 合并粘贴与粘贴文本清理的纯字符串语义（Rust-First）。
//!
//! - [`PasteTransform`]:单条粘贴的 5 种文本清理变换，仅 Text 类；变换结果恒走纯文本写回。
//! - 合并拼接：调用方按列表显示序传 `ids`，这里按传入序取纯文本表示后连接
//!   （`search_text` 回退 `content`，富文本即取 OS 纯文本表示）；合成串走纯文本写回，
//!   登记合成串哈希抑制回环（不进历史，对齐 Ditto）。
//!
//! 写回与模拟粘贴仍由调用方经 [`super::write`] 与 `keystroke` 完成，本模块只做字符串变换与拼接。

use crate::core::{AppError, Result};
use crate::db::models::{ClipboardItem, ClipboardKind};

/// 单条粘贴的文本清理变换；IPC 字面量走 `camelCase`，与前端 `PasteTransform` 一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PasteTransform {
    /// 换行变空格（网页 / PDF 复制乱换行的最高频清理）。
    StripNewlines,
    /// 每行去首尾空白并丢弃空行。
    TrimLines,
    /// 去整体首尾空白。
    TrimWhitespace,
    /// 转大写。
    UpperCase,
    /// 转小写。
    LowerCase,
}

/// 对纯文本应用单种清理变换。
pub fn apply_paste_transform(text: &str, transform: PasteTransform) -> String {
    match transform {
        PasteTransform::StripNewlines => strip_newlines(text),
        PasteTransform::TrimLines => trim_lines(text),
        PasteTransform::TrimWhitespace => text.trim().to_owned(),
        PasteTransform::UpperCase => text.to_uppercase(),
        PasteTransform::LowerCase => text.to_lowercase(),
    }
}

/// 换行（`\r\n` / `\n` / `\r`）逐个替换为空格；不做多空格归并，保留原文词间距。
fn strip_newlines(text: &str) -> String {
    text.replace("\r\n", " ").replace(['\n', '\r'], " ")
}

/// 每行 trim 后丢弃空行，再用 `\n` 连接。
fn trim_lines(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 按给定顺序用已解析的分隔符连接合并文本。
/// 分隔符字面量（`newline` / `space` / `none` / `comma`）由调用方经
/// [`crate::settings::MergePasteSeparator::as_str`] 解析，本函数只做连接。
pub fn join_merge_texts(parts: &[&str], separator: &str) -> String {
    parts.join(separator)
}

/// 取条目参与合并 / 变换的纯文本表示：富文本取 OS 纯文本（`search_text`），缺失回退 `content`。
pub fn merge_plain_text(item: &ClipboardItem) -> &str {
    item.search_text.as_deref().unwrap_or(&item.content)
}

/// 按调用方传入的 `ids` 顺序取出各条目的纯文本表示；供合并粘贴命令复用。
///
/// - 缺失 id 直接报错，避免静默丢条目造成「选了 N 条只粘了 N-1 条」。
/// - 非文本条目直接报错（前端已拦截禁用，后端再兜底不做部分粘贴）。
pub fn order_merge_parts<'a>(ids: &[String], rows: &'a [ClipboardItem]) -> Result<Vec<&'a str>> {
    let mut parts = Vec::with_capacity(ids.len());

    for id in ids {
        let Some(item) = rows.iter().find(|row| &row.id == id) else {
            return Err(AppError::Clipboard(format!(
                "clipboard item not found: {id}"
            )));
        };

        if item.kind != ClipboardKind::Text {
            return Err(AppError::Clipboard(
                "only text items can be merged".to_owned(),
            ));
        }

        parts.push(merge_plain_text(item));
    }

    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::items::{content_hash, insert_item};
    use crate::db::models::{ClipboardSubKind, Platform};
    use chrono::{DateTime, Utc};

    #[test]
    fn strip_newlines_replaces_all_breaks_with_space() {
        let out = apply_paste_transform("a\nb\r\nc\rd", PasteTransform::StripNewlines);

        assert_eq!(out, "a b c d");
    }

    #[test]
    fn trim_lines_trims_and_drops_empty_lines() {
        let out = apply_paste_transform("  a  \n\n\tb\t\n   \nc ", PasteTransform::TrimLines);

        assert_eq!(out, "a\nb\nc");
    }

    #[test]
    fn trim_whitespace_trims_edges_only() {
        let out = apply_paste_transform("  a\n  b  ", PasteTransform::TrimWhitespace);

        assert_eq!(out, "a\n  b");
    }

    #[test]
    fn case_transforms_cover_ascii_and_unicode() {
        assert_eq!(
            apply_paste_transform("abc 测试", PasteTransform::UpperCase),
            "ABC 测试"
        );
        assert_eq!(
            apply_paste_transform("ABC 测试", PasteTransform::LowerCase),
            "abc 测试"
        );
    }

    #[test]
    fn merge_separator_setting_maps_to_join_string() {
        use crate::settings::MergePasteSeparator;

        assert_eq!(MergePasteSeparator::Newline.as_str(), "\n");
        assert_eq!(MergePasteSeparator::Space.as_str(), " ");
        assert_eq!(MergePasteSeparator::None.as_str(), "");
        assert_eq!(MergePasteSeparator::Comma.as_str(), ",");
    }

    #[test]
    fn join_merge_texts_preserves_input_order() {
        let parts = ["b", "a", "c"];

        assert_eq!(join_merge_texts(&parts, "\n"), "b\na\nc");
        assert_eq!(join_merge_texts(&parts, " "), "b a c");
        assert_eq!(join_merge_texts(&parts, ""), "bac");
        assert_eq!(join_merge_texts(&parts, ","), "b,a,c");
    }

    fn sample_item(id: &str, kind: ClipboardKind, content: &str) -> ClipboardItem {
        let ts: DateTime<Utc> = DateTime::from_timestamp(1_700_000_000, 0).unwrap();

        ClipboardItem {
            id: id.to_owned(),
            kind,
            sub_kind: None,
            group_id: None,
            source_app_id: None,
            content_hash: content_hash(kind, content),
            content: content.to_owned(),
            search_text: None,
            summary: None,
            file_types: None,
            size: None,
            width: None,
            height: None,
            use_count: 1,
            is_favorite: false,
            is_pinned: false,
            is_sensitive: false,
            platform: Platform::Macos,
            note: None,
            created_at: ts,
            updated_at: ts,
            source_app_name: None,
            source_app_icon_file: None,
            source_app_icon_path: None,
            image_thumbnail_path: None,
            file_entries: None,
            files_preview_kind: None,
            available_actions: Vec::new(),
            color_preview: None,
            display_created_at: String::new(),
        }
    }

    /// 合并取数走完整入库行：富文本取 `search_text`，纯文本回退 `content`，且保持调用方传入序。
    #[tokio::test]
    async fn order_merge_parts_prefers_plain_text_in_input_order() {
        let pool = crate::db::test_support::memory_pool().await;

        let mut rich = sample_item("rich", ClipboardKind::Text, "<b>hi</b>");
        rich.sub_kind = Some(ClipboardSubKind::Html);
        rich.search_text = Some("hi".to_owned());
        insert_item(&pool, &rich).await.unwrap();
        insert_item(&pool, &sample_item("plain-b", ClipboardKind::Text, "B"))
            .await
            .unwrap();
        insert_item(&pool, &sample_item("plain-a", ClipboardKind::Text, "A"))
            .await
            .unwrap();

        let rows = crate::db::items::list_items_by_ids(
            &pool,
            &[
                "rich".to_owned(),
                "plain-a".to_owned(),
                "plain-b".to_owned(),
            ],
        )
        .await
        .unwrap();

        let ids = [
            "plain-b".to_owned(),
            "rich".to_owned(),
            "plain-a".to_owned(),
        ];
        let parts = order_merge_parts(&ids, &rows).unwrap();

        assert_eq!(parts, ["B", "hi", "A"]);
        assert_eq!(join_merge_texts(&parts, "\n"), "B\nhi\nA");
    }

    /// 非文本条目兜底报错，不做部分合并粘贴。
    #[tokio::test]
    async fn order_merge_parts_rejects_non_text() {
        let pool = crate::db::test_support::memory_pool().await;

        insert_item(&pool, &sample_item("text", ClipboardKind::Text, "hi"))
            .await
            .unwrap();
        insert_item(
            &pool,
            &sample_item("image", ClipboardKind::Image, "hash.png"),
        )
        .await
        .unwrap();

        let rows =
            crate::db::items::list_items_by_ids(&pool, &["text".to_owned(), "image".to_owned()])
                .await
                .unwrap();

        let err = order_merge_parts(&["text".to_owned(), "image".to_owned()], &rows)
            .expect_err("image item should be rejected");

        assert!(err.to_string().contains("only text items can be merged"));
    }

    /// 缺失 id 直接报错，避免静默少粘。
    #[tokio::test]
    async fn order_merge_parts_rejects_missing_id() {
        let pool = crate::db::test_support::memory_pool().await;

        insert_item(&pool, &sample_item("text", ClipboardKind::Text, "hi"))
            .await
            .unwrap();

        let rows = crate::db::items::list_items_by_ids(&pool, &["text".to_owned()])
            .await
            .unwrap();

        let err = order_merge_parts(&["text".to_owned(), "gone".to_owned()], &rows)
            .expect_err("missing id should be rejected");

        assert!(err.to_string().contains("clipboard item not found"));
    }
}

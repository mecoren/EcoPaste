//! 文本子类型识别与富文本转纯文本（纯逻辑，便于单测）。
//!
//! 判定顺序：`url` > `email` > `color` > `path`。

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::db::models::ClipboardSubKind;

/// URL：要求带协议头（http/https/ftp/file）或 `www.` 开头的单行串。
/// 规则保持收紧，避免把任意带点的词误判为链接。
static URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(https?|ftp|file)://[^\s]+$|^www\.[^\s]+\.[^\s]+$")
        .expect("invalid URL regex")
});

/// Email：本地部分允许字母、数字、中文及 `.` `+` `_` `-` `%`（RFC 5321 常见字符）。
static EMAIL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z0-9._%+\-\u{4e00}-\u{9fa5}]+@[a-zA-Z0-9_-]+(\.[a-zA-Z0-9_-]+)+$")
        .expect("invalid email regex")
});

/// Hex 颜色：#RGB / #RGBA / #RRGGBB / #RRGGBBAA。
static HEX_COLOR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^#([0-9a-f]{3}|[0-9a-f]{4}|[0-9a-f]{6}|[0-9a-f]{8})$")
        .expect("invalid hex color regex")
});

/// 颜色函数：覆盖经典 `rgb()/rgba()/hsl()/hsla()`（含逗号 / 空格语法）
/// 以及 CSS Color 4/5：`hwb() / lab() / lch() / oklab() / oklch() / color() / color-mix()`。
/// 括号内不限定（含嵌套函数），由 [`is_safe_css_value`] 做安全 + 括号平衡校验。
static COLOR_FN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(rgba?|hsla?|hwb|lab|lch|oklab|oklch|color|color-mix)\(.+\)$")
        .expect("invalid color fn regex")
});

/// CSS 渐变：`linear-gradient(...)` / `radial-gradient(...)` / `conic-gradient(...)`
/// 及其 `repeating-*` 变体。括号内不限定内容，由 [`is_safe_css_value`] 做安全/平衡校验。
static GRADIENT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(repeating-)?(linear|radial|conic)-gradient\(.+\)$")
        .expect("invalid gradient regex")
});

/// 子类型识别的长度闸门：URL/email/color/path 都是行级短特征，超长文本几乎必然
/// 是普通长文；只取前 4KB 判定，避免 4MB 级复制内容全串跑正则。
const DETECT_MAX_BYTES: usize = 4096;

/// 识别纯文本的子类型。判定顺序：url > email > color > path。
/// 均不命中返回 `None`（普通文本）。
///
/// 性能约束（监听热路径）：超 [`DETECT_MAX_BYTES`] 的文本只识别前 4KB；
/// `path` 分支只判定「看起来像绝对路径」的形态（`is_absolute`），**不做 `exists()**
/// 文件系统调用——存在性校验延后到 Reveal / 预览动作执行时（届时打开资源管理器
/// 本来就会失败提示）。相对路径仍不判（存在性取决于进程 cwd，误判面大）。
pub fn detect_text_sub_kind(text: &str) -> Option<ClipboardSubKind> {
    let value = text.trim();
    if value.is_empty() {
        return None;
    }

    let head = truncate_head_bytes(value);
    if URL_RE.is_match(head) {
        return Some(ClipboardSubKind::Url);
    }
    if EMAIL_RE.is_match(head) {
        return Some(ClipboardSubKind::Email);
    }
    if is_css_color_value(head) {
        return Some(ClipboardSubKind::Color);
    }
    if Path::new(head).is_absolute() {
        return Some(ClipboardSubKind::Path);
    }
    None
}

/// 截取文本头部的字节上限切片；UTF-8 边界处回退到最近完整字符。
fn truncate_head_bytes(value: &str) -> &str {
    if value.len() <= DETECT_MAX_BYTES {
        return value;
    }

    let mut end = DETECT_MAX_BYTES;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

/// 把任意字符串规范化为可信的 CSS 颜色串：trim 后必须命中颜色 / 渐变规则
/// 且通过 [`is_safe_css_value`] 才返回 `Some`。命令层用它给前端 `colorPreview` 兜底，
/// 避免前端把任意文本塞进 CSS `background` 触发样式注入。
pub fn sanitize_css_color(text: &str) -> Option<String> {
    let value = text.trim();

    if value.is_empty() || !is_css_color_value(value) {
        return None;
    }

    Some(value.to_owned())
}

/// 统一判定：hex 直接命中即可（无括号无注入面）；
/// 函数式 / 渐变需要再过 [`is_safe_css_value`] 防注入与括号平衡。
fn is_css_color_value(value: &str) -> bool {
    if HEX_COLOR_RE.is_match(value) {
        return true;
    }

    if (COLOR_FN_RE.is_match(value) || GRADIENT_RE.is_match(value)) && is_safe_css_value(value) {
        return true;
    }

    false
}

/// CSS background 值安全校验：用于 gradient 这类括号内容自由的语法。
/// - 拒绝 `;` / `<` / `>` 等可能跳出声明 / 注入标签的字符；
/// - 拒绝 `url(` / `expression(` / `javascript:` / `@import` 等已知风险 token；
/// - 要求括号平衡，避免半截语法被浏览器宽松解析后吞掉后续 CSS。
fn is_safe_css_value(value: &str) -> bool {
    if value.contains(';') || value.contains('<') || value.contains('>') {
        return false;
    }

    let lower = value.to_ascii_lowercase();
    if lower.contains("url(")
        || lower.contains("expression(")
        || lower.contains("javascript:")
        || lower.contains("@import")
    {
        return false;
    }

    let mut depth: i32 = 0;
    for ch in value.chars() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }

    depth == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_url() {
        for s in [
            "https://example.com",
            "http://a.b/c?d=1",
            "ftp://host/file",
            "www.example.com",
            "  https://trimmed.com  ",
        ] {
            assert_eq!(
                detect_text_sub_kind(s),
                Some(ClipboardSubKind::Url),
                "should be url: {s:?}"
            );
        }
        // 不带协议的裸域名/普通词不判 url。
        assert_eq!(detect_text_sub_kind("example.com"), None);
        assert_eq!(detect_text_sub_kind("hello world"), None);
    }

    #[test]
    fn detects_email() {
        assert_eq!(
            detect_text_sub_kind("user@example.com"),
            Some(ClipboardSubKind::Email)
        );
        assert_eq!(
            detect_text_sub_kind("张三@example.com.cn"),
            Some(ClipboardSubKind::Email)
        );
        // 常见但此前被 EMAIL_RE 漏判的本地部分：含 . + _ - %
        assert_eq!(
            detect_text_sub_kind("first.last@example.com"),
            Some(ClipboardSubKind::Email),
            "dotted local part (first.last@)"
        );
        assert_eq!(
            detect_text_sub_kind("user+tag@gmail.com"),
            Some(ClipboardSubKind::Email),
            "plus-tagged local part (user+tag@)"
        );
        assert_eq!(
            detect_text_sub_kind("name_surname@example.com"),
            Some(ClipboardSubKind::Email),
            "underscore local part (name_surname@)"
        );
        assert_eq!(
            detect_text_sub_kind("user-name@example.com"),
            Some(ClipboardSubKind::Email),
            "hyphen local part (user-name@)"
        );
        assert_eq!(
            detect_text_sub_kind("user%tag@example.com"),
            Some(ClipboardSubKind::Email),
            "percent local part (user%tag@)"
        );
        assert_eq!(detect_text_sub_kind("not@an@email"), None);
    }

    #[test]
    fn detects_color() {
        for s in [
            "#fff",
            "#FFFF",
            "#ffffff",
            "#ffffffff",
            "rgb(1,2,3)",
            "rgba(1,2,3,0.5)",
            "hsl(0, 100%, 50%)",
            "hsla(0,100%,50%,.5)",
        ] {
            assert_eq!(
                detect_text_sub_kind(s),
                Some(ClipboardSubKind::Color),
                "should be color: {s:?}"
            );
        }
        // CSS 关键字 / 含 url 的值不判 color。
        assert_eq!(detect_text_sub_kind("inherit"), None);
        assert_eq!(detect_text_sub_kind("url(#abc)"), None);
        assert_eq!(detect_text_sub_kind("#xyz"), None);
    }

    #[test]
    fn detects_modern_color_functions() {
        for s in [
            // rgb() / hsl() 现代空格语法 + slash alpha
            "rgb(255 87 51)",
            "rgb(255 87 51 / 0.5)",
            "hsl(0 100% 50% / 80%)",
            // CSS Color 4 现代色彩空间
            "hwb(120 10% 20%)",
            "lab(50% 40 30)",
            "lch(50% 40 30)",
            "oklab(0.7 0.1 0.05)",
            "oklch(0.7 0.15 30)",
            "color(display-p3 1 0 0)",
            "color(rec2020 0.5 0.2 0.8 / 0.6)",
            // CSS Color 5 color-mix（含嵌套函数）
            "color-mix(in srgb, #fff 50%, #000)",
            "color-mix(in oklch, oklch(0.7 0.15 30), red 20%)",
        ] {
            assert_eq!(
                detect_text_sub_kind(s),
                Some(ClipboardSubKind::Color),
                "should be color (modern fn): {s:?}"
            );
            assert_eq!(sanitize_css_color(s).as_deref(), Some(s));
        }
    }

    #[test]
    fn detects_gradient_as_color() {
        for s in [
            "linear-gradient(to right, #ffdde1, #ee9ca7)",
            "radial-gradient(circle, rgba(0,0,0,0.5) 0%, #fff 100%)",
            "conic-gradient(from 45deg, red, blue)",
            "repeating-linear-gradient(45deg, #000 0 10px, #fff 10px 20px)",
        ] {
            assert_eq!(
                detect_text_sub_kind(s),
                Some(ClipboardSubKind::Color),
                "should be color (gradient): {s:?}"
            );
            assert_eq!(sanitize_css_color(s).as_deref(), Some(s));
        }

        // 含注入风险或括号不平衡的渐变串：识别命中但 sanitize 拒绝。
        for bad in [
            "linear-gradient(to right, url(http://x))",
            "linear-gradient(to right, #fff;color:red)",
            "linear-gradient(to right, #fff",
        ] {
            assert_eq!(sanitize_css_color(bad), None, "should reject: {bad:?}");
        }
    }

    #[test]
    fn detects_absolute_path_by_shape_without_fs_access() {
        let dir = std::env::temp_dir();
        let file = dir.join(format!("ecopaste-detect-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&file, b"x").unwrap();

        // 存在与否都判 path：识别只看绝对路径形态，exists() 延后到动作执行。
        assert_eq!(
            detect_text_sub_kind(file.to_str().unwrap()),
            Some(ClipboardSubKind::Path)
        );
        // 平台本位的绝对路径形态（Windows 带盘符、macOS 带根斜杠）。
        let non_existing = if cfg!(windows) {
            "C:\\nope\\does\\not\\exist\\xyz"
        } else {
            "/nope/does/not/exist/xyz"
        };
        assert_eq!(
            detect_text_sub_kind(non_existing),
            Some(ClipboardSubKind::Path)
        );
        // 相对路径不判。
        assert_eq!(detect_text_sub_kind("src"), None);

        std::fs::remove_file(&file).ok();
    }

    #[test]
    fn long_text_only_scans_head() {
        // 4MB 多行普通文本：不命中任何子类型（URL/email/color 都是单行整串特征，
        // 多行长文天然不命中；闸门保证只扫前 4KB 而非全串）。
        let mut long_plain = String::from("普通文本行\n");
        while long_plain.len() < 4 * 1024 * 1024 {
            long_plain.push_str("更多普通文本内容，不构成任何子类型特征。\n");
        }
        assert_eq!(detect_text_sub_kind(&long_plain), None);

        // 4MB 单行超长 URL（trim 后整串仍是合法 URL 形态）：头部截断后 URL 前缀
        // + 无空白仍命中——证明截断不破坏行首特征识别。
        let long_url = format!("https://example.com/{}", "a".repeat(4 * 1024 * 1024));
        assert_eq!(detect_text_sub_kind(&long_url), Some(ClipboardSubKind::Url));

        // 多字节截断：4KB 边界落在 UTF-8 字符中间时回退到完整字符边界，不 panic。
        let cjk_tail = "中文内容重复";
        let mut multi_byte = String::new();
        while multi_byte.len() < 4096 + 16 {
            multi_byte.push_str(cjk_tail);
        }
        assert_eq!(detect_text_sub_kind(&multi_byte), None);
    }

    /// 监听热路径微基准（手动跑：`cargo test --release --lib hot_path_bench -- --ignored --nocapture`）。
    /// 覆盖 4MB 无特征普通文本与 4MB 高频含 label 词的最坏文本两类形态。
    /// 基线（2026-09 实测，release 10 轮）：普通 ~9ms / 最坏 ~51ms；
    /// secrets 曾试验「字面预筛 + 窗口正则」，实测（16ms / 69ms）反而更慢——
    /// regex crate 本身已高度优化，故回退；本基准保留用于防回归与后续优化的对照。
    /// debug 构建受无优化惩罚主导，性能结论以 release 为准。
    #[test]
    #[ignore = "manual microbenchmark; run with --release"]
    fn hot_path_bench_large_plain_text() {
        use std::time::Instant;

        let mut plain = String::from("普通文本起始行\n");
        while plain.len() < 4 * 1024 * 1024 {
            plain.push_str("普通剪贴板正文内容，没有任何英文关键词。\n");
        }

        let start = Instant::now();
        for _ in 0..10 {
            assert_eq!(detect_text_sub_kind(&plain), None);
            assert!(!super::super::secrets::contains_secret(&plain));
        }
        let plain_elapsed = start.elapsed();
        println!("10x (detect+secret) on 4MB plain text: {plain_elapsed:?}");

        let mut worst = String::from("普通文本起始行\n");
        while worst.len() < 4 * 1024 * 1024 {
            worst.push_str("普通剪贴板正文内容，不含任何 secret 或子类型特征 token。\n");
        }
        let start = Instant::now();
        for _ in 0..10 {
            assert_eq!(detect_text_sub_kind(&worst), None);
            assert!(!super::super::secrets::contains_secret(&worst));
        }
        let worst_elapsed = start.elapsed();
        println!("10x (detect+secret) on 4MB label-heavy text: {worst_elapsed:?}");

        assert!(
            plain_elapsed.as_millis() < 100,
            "plain-text hot path regression: {plain_elapsed:?}"
        );
        assert!(
            worst_elapsed.as_millis() < 500,
            "worst-case hot path regression: {worst_elapsed:?}"
        );
    }
}

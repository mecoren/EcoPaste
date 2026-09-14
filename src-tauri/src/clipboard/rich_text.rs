//! RTF → HTML 转换：预览窗口富文本档（P0-2）。
//!
//! macOS 用 `NSAttributedString` 的文档读写 API（objc2-app-kit 已在依赖树内，
//! 零新增依赖）；Windows 侧 TOM/ITextServices 的 COM 序列重且需要隐藏控件与
//! 消息泵，收益不抵复杂度——返回 `None` 走纯文本行降级（与 Maccy 的
//! 「不渲染 RTF」一致）。两端 HTML 产出都只是「尽力而为」：最终安全性由
//! 前端 DOMPurify 白名单兜底，这里不做任何安全假设。

use crate::core::Result;

/// 把 RTF 源转成 HTML；无法转换（Windows / RTF 不合法）时返回 `None`，
/// 调用方降级为现有纯文本行预览。
pub fn rtf_to_html(rtf: &str) -> Result<Option<String>> {
    if rtf.trim().is_empty() {
        return Ok(None);
    }

    platform_rtf_to_html(rtf)
}

#[cfg(target_os = "macos")]
fn platform_rtf_to_html(rtf: &str) -> Result<Option<String>> {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::AnyThread;
    use objc2_app_kit::NSAttributedStringAppKitDocumentFormats;
    use objc2_app_kit::NSAttributedStringDocumentFormats;
    use objc2_app_kit::{NSDocumentTypeDocumentOption, NSHTMLTextDocumentType};
    use objc2_foundation::{NSAttributedString, NSData, NSDictionary, NSString};

    // NSAttributedString 文档 API 要求主线程调用（预览命令本身在主线程）。
    let data = NSData::with_bytes(rtf.as_bytes());

    let Some(attributed) = (unsafe {
        NSAttributedString::initWithRTF_documentAttributes(NSAttributedString::alloc(), &data, None)
    }) else {
        return Ok(None);
    };

    // 文档键与文档类型常量都是 NSString 的 type alias；extern static 读取需 unsafe。
    let keys: [&NSString; 1] = [unsafe { NSDocumentTypeDocumentOption }];
    let values = [Retained::<AnyObject>::from(Retained::<NSString>::from(
        unsafe { NSHTMLTextDocumentType },
    ))];
    let writing = NSDictionary::from_retained_objects(&keys, &values);

    let html_data = (unsafe {
        attributed.dataFromRange_documentAttributes_error(
            objc2_foundation::NSRange::new(0, attributed.length()),
            &writing,
        )
    })
    .ok();

    let Some(html_data) = html_data else {
        return Ok(None);
    };

    let html = String::from_utf8_lossy(&html_data.to_vec()).into_owned();

    Ok(if html.trim().is_empty() {
        None
    } else {
        Some(html)
    })
}

#[cfg(target_os = "windows")]
fn platform_rtf_to_html(_rtf: &str) -> Result<Option<String>> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_rtf_returns_none() {
        assert!(rtf_to_html("").unwrap().is_none());
        assert!(rtf_to_html("   \n ").unwrap().is_none());
    }
}

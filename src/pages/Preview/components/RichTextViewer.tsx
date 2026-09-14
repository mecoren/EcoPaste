import DOMPurify from "dompurify";
import type { FC } from "react";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "@/utils/cn";

/** 富文本 sanitize 白名单：只保留常见排版标签与无害属性，其余（script /
 * iframe / 事件属性 / style 脚本等）一律剔除。 */
const RICH_TEXT_ALLOWED_TAGS = [
  "a",
  "b",
  "blockquote",
  "br",
  "code",
  "div",
  "em",
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
  "hr",
  "i",
  "img",
  "li",
  "ol",
  "p",
  "pre",
  "s",
  "span",
  "strong",
  "sub",
  "sup",
  "table",
  "tbody",
  "td",
  "th",
  "thead",
  "tr",
  "u",
  "ul",
];

const RICH_TEXT_ALLOWED_ATTRS = ["alt", "colspan", "href", "rowspan", "src"];

/** 超过该字节数的 HTML 降级为纯文本行渲染，杜绝大 DOM。 */
const RICH_TEXT_MAX_BYTES = 256 * 1024;

interface RichTextViewerProps {
  html: string;
  /** 降级纯文本行的内容（html 超限或 sanitize 后为空时展示）。 */
  fallbackText: string;
}

/**
 * 富文本预览（P0-2）：DOMPurify 严格白名单 sanitize 后注入无脚本沙箱 iframe。
 *
 * 高度策略：iframe 固定填充预览面板剩余高度、内容超长在 iframe 内部滚动——
 * `sandbox` 不给 `allow-same-origin`，父页面无法读内容高度做自适应
 *（方案 P0-2 风险栏预见的取舍，安全优先）。超过 256KB 的 HTML 直接降级
 * 纯文本行并出提示条。
 */
const RichTextViewer: FC<RichTextViewerProps> = (props) => {
  const { html, fallbackText } = props;
  const { t } = useTranslation("preview");
  const oversized = html.length > RICH_TEXT_MAX_BYTES;

  const sanitized = useMemo(() => {
    if (oversized) return "";

    return DOMPurify.sanitize(html, {
      ALLOW_DATA_ATTR: false,
      ALLOWED_ATTR: RICH_TEXT_ALLOWED_ATTRS,
      ALLOWED_TAGS: RICH_TEXT_ALLOWED_TAGS,
      FORBID_ATTR: ["style"],
    });
  }, [html, oversized]);

  if (oversized) {
    return (
      <div className="flex h-full min-h-0 flex-col">
        <div className="flex items-center gap-1.5 border-ant-border border-b px-4 py-1 text-ant-secondary text-xs">
          <i aria-hidden="true" className="i-lucide:info shrink-0" />
          <span>{t("rich.tooLarge")}</span>
        </div>
        <PlainRows text={fallbackText} />
      </div>
    );
  }

  if (sanitized.trim().length === 0) {
    return <PlainRows text={fallbackText} />;
  }

  const srcDoc = `<!DOCTYPE html><html><head><meta charset="utf-8"><style>
    :root { color-scheme: light dark; }
    body { margin: 8px; font: 12px/1.6 -apple-system, "Segoe UI", sans-serif; overflow-wrap: break-word; }
    img { max-width: 100%; height: auto; }
    a { color: inherit; }
    ::-webkit-scrollbar { width: 6px; height: 6px; }
    ::-webkit-scrollbar-thumb { background: rgba(127,127,127,.45); border-radius: 3px; }
  </style></head><body>${sanitized}</body></html>`;

  return (
    <iframe
      className={cn("h-full min-h-0 w-full border-0")}
      referrerPolicy="no-referrer"
      sandbox=""
      srcDoc={srcDoc}
      title={t("rich.frameTitle")}
    />
  );
};

/**
 * 紧凑版纯文本行渲染（降级路径）：非虚拟化的简单行拆分——降级是罕见路径，
 * 无长列表性能诉求。
 */
const PlainRows: FC<{ text: string }> = (props) => {
  const { text } = props;
  const rows = useMemo(() => {
    return text.split("\n");
  }, [text]);

  return (
    <div className="min-h-0 flex-1 overflow-y-auto whitespace-pre px-4 py-2 font-mono text-xs leading-5.5">
      {rows.map((row, index) => {
        return (
          // rows 是纯静态文本行（不重排不增删），行内容可重复（连续空行），
          // 序号必须参与 key——index key 在这里是正确选择。
          // biome-ignore lint/suspicious/noArrayIndexKey: static deprecation rows never reorder
          <div key={`${index}-${row}`}>{row.length === 0 ? " " : row}</div>
        );
      })}
    </div>
  );
};

export default RichTextViewer;

import DOMPurify from "dompurify";
import type { FC } from "react";
import { useMemo } from "react";

/**
 * 轻量 Markdown 预览渲染（P0-2 的 MD 开关）：行级解析常见语法——标题、加粗、
 * 斜体、行内代码、围栏代码块、链接、无序 / 有序列表、引用、分割线。
 * 刻意不引入 md 依赖库（预览用途、非完整 CommonMark），转义所有 HTML 后再
 * 注入行内标记，最后整体过 DOMPurify（同一白名单兜底），无 XSS 面。
 */

const INLINE_CODE = /`([^`]+)`/g;
const BOLD = /\*\*([^*]+)\*\*/g;
const ITALIC = /\*([^*]+)\*/g;
const LINK = /\[([^\]]+)\]\(([^)\s]+)\)/g;

interface MarkdownPreviewProps {
  text: string;
}

const MarkdownPreview: FC<MarkdownPreviewProps> = (props) => {
  const { text } = props;
  const html = useMemo(() => {
    return renderMarkdown(text);
  }, [text]);

  return (
    <div
      className="min-h-0 flex-1 overflow-y-auto px-4 py-2 text-xs leading-relaxed"
      // renderMarkdown 自身已转义全部输入并只生成受控标签结构，
      // DOMPurify 再过一道同款白名单作为纵深防御。
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
};

/** 行级解析 Markdown，产出受控标签结构的 HTML 字符串。 */
function renderMarkdown(text: string) {
  const lines = text.split("\n");
  const parts: string[] = [];
  let listType: "ol" | "ul" | null = null;
  let codeLines: string[] | null = null;

  const closeList = () => {
    if (listType) {
      parts.push(`</${listType}>`);
      listType = null;
    }
  };

  for (const line of lines) {
    if (codeLines !== null) {
      if (line.trim() === "```") {
        parts.push(`<pre>${codeLines.map(escapeHtml).join("\n")}</pre>`);
        codeLines = null;
      } else {
        codeLines.push(line);
      }
      continue;
    }

    if (line.trim() === "```") {
      closeList();
      codeLines = [];
      continue;
    }

    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) {
      closeList();
      const level = heading[1].length;
      parts.push(`<h${level}>${renderInline(heading[2])}</h${level}>`);
      continue;
    }

    if (/^\s*(---|\*\*\*)\s*$/.test(line)) {
      closeList();
      parts.push("<hr />");
      continue;
    }

    const quote = /^>\s?(.*)$/.exec(line);
    if (quote) {
      closeList();
      parts.push(`<blockquote>${renderInline(quote[1])}</blockquote>`);
      continue;
    }

    const ulItem = /^\s*[-*+]\s+(.*)$/.exec(line);
    if (ulItem) {
      if (listType !== "ul") {
        closeList();
        parts.push("<ul>");
        listType = "ul";
      }
      parts.push(`<li>${renderInline(ulItem[1])}</li>`);
      continue;
    }

    const olItem = /^\s*\d+\.\s+(.*)$/.exec(line);
    if (olItem) {
      if (listType !== "ol") {
        closeList();
        parts.push("<ol>");
        listType = "ol";
      }
      parts.push(`<li>${renderInline(olItem[1])}</li>`);
      continue;
    }

    if (line.trim().length === 0) {
      closeList();
      continue;
    }

    closeList();
    parts.push(`<p>${renderInline(line)}</p>`);
  }

  if (codeLines !== null) {
    parts.push(`<pre>${codeLines.map(escapeHtml).join("\n")}</pre>`);
  }
  closeList();

  return DOMPurify.sanitize(parts.join(""), {
    ALLOW_DATA_ATTR: false,
    ALLOWED_ATTR: ["href"],
    ALLOWED_TAGS: [
      "a",
      "b",
      "blockquote",
      "code",
      "em",
      "h1",
      "h2",
      "h3",
      "h4",
      "h5",
      "h6",
      "hr",
      "i",
      "li",
      "ol",
      "p",
      "pre",
      "strong",
      "ul",
    ],
  });
}

/** 行内标记：先整体转义，再按受控模式替换为标签（无拼接注入面）。 */
function renderInline(text: string) {
  let out = escapeHtml(text);

  out = out.replace(INLINE_CODE, "<code>$1</code>");
  out = out.replace(BOLD, "<strong>$1</strong>");
  out = out.replace(LINK, (_match: string, label: string, href: string) => {
    const safeHref = /^(https?:|mailto:)/i.test(href) ? href : "#";
    return `<a href="${safeHref}">${label}</a>`;
  });
  out = out.replace(ITALIC, "<em>$1</em>");

  return out;
}

/** HTML 转义：标签字符全部实体化，源文中的 HTML 不会被执行。 */
function escapeHtml(text: string) {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

export default MarkdownPreview;

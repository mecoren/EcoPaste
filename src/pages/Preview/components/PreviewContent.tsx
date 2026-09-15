import { convertFileSrc } from "@tauri-apps/api/core";
import { useUnmount } from "ahooks";
import { Image as AntImage, Empty } from "antd";
import type { TFunction } from "i18next";
import type { FC } from "react";
import { useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Virtuoso } from "react-virtuoso";
import { useSnapshot } from "valtio";
import {
  type ClipboardPreviewFileEntry,
  type ClipboardPreviewPayload,
  writeToClipboard,
} from "@/commands";
import AssetImage from "@/components/AssetImage";
import VirtuosoScroller, {
  type VirtuosoScrollerChildrenProps,
} from "@/components/VirtuosoScroller";
import { settingsState } from "@/stores/settings";
import { cn } from "@/utils/cn";
import {
  PREVIEW_COPY_FEEDBACK_MS,
  PREVIEW_TEXT_SOFT_WRAP_CHARS,
} from "../constants";
import MarkdownPreview from "./MarkdownPreview";
import RichTextViewer from "./RichTextViewer";

export interface PreviewContentProps {
  payload: ClipboardPreviewPayload | null;
}

export interface PreviewHeaderProps {
  payload: ClipboardPreviewPayload | null;
}

interface PayloadViewerProps {
  payload: ClipboardPreviewPayload;
}

interface FilePreviewRowProps {
  file: ClipboardPreviewFileEntry;
}

interface ImageStageProps {
  alt: string;
  /** 面板内渲染用的图片路径（本地绝对路径或托管预览档）。 */
  src: string;
  /** 灯箱加载的原图路径；缺失时退回 `src`。 */
  originSrc?: string | null;
  width?: number;
  height?: number;
  /** 图片解析失败（webview 不支持的格式 / 文件不可读）时通知调用方降级。 */
  onError?: () => void;
}

const TEXT_VIRTUOSO_COMPONENTS = {
  Footer: PreviewTextPadding,
  Header: PreviewTextPadding,
};

const FILES_VIRTUOSO_COMPONENTS = {
  Footer: PreviewFilesPadding,
  Header: PreviewFilesPadding,
};

/**
 * Content Viewer 顶部元信息区。
 * 右侧复制按钮是预览面板内的纯鼠标复制入口（复制整条记录）；片段复制走选区浮动按钮。
 */
export const PreviewHeader: FC<PreviewHeaderProps> = (props) => {
  const { payload } = props;
  const { t } = useTranslation(["preview", "clipboard"]);
  const [copied, setCopied] = useState(false);
  const feedbackTimerRef = useRef<number | null>(null);
  const title = payload ? previewTitle(t, payload) : t("title.loading");
  const meta = payload ? previewMeta(t, payload) : t("meta.contentViewer");
  const typeKey = payload ? (payload.subKind ?? payload.kind) : null;
  const typeLabel = typeKey ? t(`clipboard:types.${typeKey}`) : "";

  useUnmount(cancelFeedback);

  /**
   * 复制整条记录：与列表里的「复制」同一条命令，默认复制格式与「复制后隐藏窗口」
   * 设置全部复用；只在预览窗内静默 —— 按钮自身对勾已是反馈，否则两个窗口各飘一条 toast。
   */
  const handleCopy = async () => {
    if (!payload) return;

    cancelFeedback();
    setCopied(true);

    try {
      await writeToClipboard(payload.id, false, { silent: true });
    } catch {
      setCopied(false);
      return;
    }

    feedbackTimerRef.current = window.setTimeout(() => {
      feedbackTimerRef.current = null;
      setCopied(false);
    }, PREVIEW_COPY_FEEDBACK_MS);
  };

  function cancelFeedback() {
    if (feedbackTimerRef.current === null) return;

    window.clearTimeout(feedbackTimerRef.current);
    feedbackTimerRef.current = null;
  }

  return (
    <div className="flex h-12 shrink-0 items-center justify-between gap-3 border-ant-border border-b px-4">
      <div className="min-w-0">
        <div className="truncate font-medium text-sm">{title}</div>
        <div className="truncate text-ant-secondary text-xs">{meta}</div>
      </div>

      <div className="flex shrink-0 items-center gap-2">
        {payload && (
          <span className="rounded-1 bg-ant-fill-secondary px-2 py-0.5 text-ant-secondary text-xs">
            {typeLabel}
          </span>
        )}

        {payload && (
          <button
            aria-label={t("copy.item")}
            className={cn(
              "flex size-6 cursor-pointer items-center justify-center rounded-1 text-ant-secondary transition-colors hover:text-ant-text",
              { "text-ant-primary": copied },
            )}
            onClick={handleCopy}
            title={copied ? t("copy.itemCopied") : t("copy.item")}
            type="button"
          >
            <i
              aria-hidden="true"
              className={copied ? "i-lucide:check" : "i-lucide:copy"}
            />
          </button>
        )}
      </div>
    </div>
  );
};

/**
 * 按 payload kind 分发到基础 viewer。
 */
export const PreviewContent: FC<PreviewContentProps> = (props) => {
  const { payload } = props;
  const { t } = useTranslation("preview");

  if (!payload) {
    return (
      <div className="flex min-h-24 items-center justify-center">
        <Empty
          description={t("empty.content")}
          image={Empty.PRESENTED_IMAGE_SIMPLE}
        />
      </div>
    );
  }

  if (payload.kind === "image") return <ImageViewer payload={payload} />;

  if (payload.kind === "files") return <FilesViewer payload={payload} />;

  return <TextViewer payload={payload} />;
};

/**
 * 文本预览分流（P0-2）：富文本条目（payload.html）走沙箱 iframe 渲染；
 * 纯文本条目在开启 MD 开关时可切换 Markdown / 原文视图；默认纯文本虚拟行。
 */
const TextViewer: FC<PayloadViewerProps> = (props) => {
  const { payload } = props;
  const { t } = useTranslation("preview");
  const { clipboard } = useSnapshot(settingsState);
  const renderMarkdown = clipboard.preview.renderMarkdown;
  const [mdView, setMdView] = useState(false);
  const text = payload.text ?? "";

  if (payload.html) {
    return <RichTextViewer fallbackText={text} html={payload.html} />;
  }

  if (text.length === 0) {
    return (
      <div className="flex min-h-24 items-center justify-center">
        <Empty
          description={t("empty.text")}
          image={Empty.PRESENTED_IMAGE_SIMPLE}
        />
      </div>
    );
  }

  if (renderMarkdown) {
    return (
      <div className="flex h-full min-h-0 flex-col">
        <div className="flex shrink-0 select-none justify-end gap-1 border-ant-border border-b px-4 py-1">
          <button
            className={cn(
              "rounded-1 px-1.5 text-ant-secondary text-xs transition-colors hover:text-ant-text",
              { "font-medium text-ant-primary": mdView },
            )}
            onClick={() => {
              setMdView(true);
            }}
            type="button"
          >
            {t("text.markdownView")}
          </button>
          <button
            className={cn(
              "rounded-1 px-1.5 text-ant-secondary text-xs transition-colors hover:text-ant-text",
              { "font-medium text-ant-primary": !mdView },
            )}
            onClick={() => {
              setMdView(false);
            }}
            type="button"
          >
            {t("text.plainView")}
          </button>
        </div>

        {mdView ? (
          <MarkdownPreview text={text} />
        ) : (
          <PlainTextViewer text={text} />
        )}
      </div>
    );
  }

  return <PlainTextViewer text={text} />;
};

/**
 * 纯文本虚拟行渲染：所有文本族内容按纯文本虚拟行展示，避免长内容构造大 DOM。
 */
const PlainTextViewer: FC<{ text: string }> = (props) => {
  const { text } = props;
  const rows = useMemo(() => {
    return buildTextPreviewRows(text);
  }, [text]);

  return <VirtuosoScroller>{renderTextVirtuoso}</VirtuosoScroller>;

  function renderTextVirtuoso(props: VirtuosoScrollerChildrenProps) {
    const { scrollerRef } = props;

    return (
      <Virtuoso
        components={TEXT_VIRTUOSO_COMPONENTS}
        computeItemKey={computeTextRowKey}
        itemContent={renderTextRow}
        scrollerRef={scrollerRef}
        totalCount={rows.length}
      />
    );
  }

  function computeTextRowKey(index: number) {
    return index;
  }

  function renderTextRow(index: number) {
    const row = rows[index] ?? "";

    return (
      <div className="min-h-5.5 whitespace-pre px-4 font-mono text-xs leading-5.5">
        {row.length === 0 ? " " : row}
      </div>
    );
  }
};

/**
 * 图片预览：面板用 960px 预览档渲染；点击展开灯箱（antd Image preview 体系，
 * 自带滚轮/按钮缩放、拖拽平移、Esc 关闭），灯箱才加载原图——此时才付出整图
 * 解码成本，单次、用户主动触发。原图缺失时灯箱退回预览档。
 */
const ImageViewer: FC<PayloadViewerProps> = (props) => {
  const { payload } = props;
  const { t } = useTranslation("preview");

  if (!payload.imagePath || !payload.imageExists) {
    return (
      <div className="flex min-h-24 items-center justify-center">
        <Empty
          description={t("empty.imageMissing")}
          image={Empty.PRESENTED_IMAGE_SIMPLE}
        />
      </div>
    );
  }

  return (
    <ImageStage
      alt={t("image.alt")}
      height={payload.imageHeight ?? void 0}
      originSrc={payload.imageOriginPath}
      src={payload.imagePath}
      width={payload.imageWidth ?? void 0}
    />
  );
};

/**
 * 图片展示台：`image` 条目与「单图文件」共用同一套渲染与灯箱。
 * `src` 走 `AssetImage`（内部 `convertFileSrc`），灯箱需要自己把路径转成可加载 URL。
 */
const ImageStage: FC<ImageStageProps> = (props) => {
  const { alt, height, onError, originSrc, src, width } = props;
  const [lightboxOpen, setLightboxOpen] = useState(false);
  const lightboxSrc = originSrc ?? src;
  const lightboxUrl = lightboxSrc ? convertFileSrc(lightboxSrc) : void 0;

  /**
   * 点击图片展开灯箱；button 原生 Enter / Space 同效（预览窗 focusable=false，
   * 键盘路径仅主窗口 keydown 重定向场景可达，鼠标点击是主路径）。
   */
  const openLightbox = () => {
    setLightboxOpen(true);
  };

  return (
    <div className="flex h-full min-h-0 items-center justify-center p-4">
      {/* AssetImage 自身 pointer-events-none（防 hover 干扰），点击目标放外层。 */}
      <button className="cursor-zoom-in" onClick={openLightbox} type="button">
        <AssetImage
          alt={alt}
          className="h-auto max-h-full max-w-full object-contain"
          draggable={false}
          height={height}
          onError={onError}
          src={src}
          width={width}
        />
      </button>

      {/* 隐藏承载节点：只为 antd 灯箱提供原图 src 与受控开关，面板内不渲染。 */}
      <AntImage
        alt={alt}
        hidden
        preview={{
          onOpenChange: (open) => {
            setLightboxOpen(open);
          },
          open: lightboxOpen,
        }}
        src={lightboxUrl}
        style={{ display: "none" }}
      />
    </div>
  );
};

/**
 * 文件预览：单图文件直接按图片渲染（与列表卡片同判据，由 Rust 算好 `filesPreviewKind`），
 * 其余情况走虚拟列表展示路径、文件名、存在状态与基础大小。
 */
const FilesViewer: FC<PayloadViewerProps> = (props) => {
  const { payload } = props;
  const { t } = useTranslation("preview");
  // 图片解析失败（webview 不支持的格式 / 文件不可读）时退回文件行列表。
  const [imageBroken, setImageBroken] = useState(false);

  if (payload.filesPreviewKind === "imagePreview" && !imageBroken) {
    const [imageEntry] = payload.files;

    if (imageEntry) {
      return (
        <ImageStage
          alt={t("image.alt")}
          onError={handleImageError}
          originSrc={imageEntry.path}
          src={imageEntry.path}
        />
      );
    }
  }

  if (payload.files.length === 0) {
    return (
      <div className="flex min-h-24 items-center justify-center">
        <Empty
          description={t("empty.files")}
          image={Empty.PRESENTED_IMAGE_SIMPLE}
        />
      </div>
    );
  }

  return <VirtuosoScroller>{renderFilesVirtuoso}</VirtuosoScroller>;

  function renderFilesVirtuoso(props: VirtuosoScrollerChildrenProps) {
    const { scrollerRef } = props;
    const components =
      payload.totalFiles > payload.files.length
        ? {
            Footer: renderFilesFooter,
            Header: PreviewFilesPadding,
          }
        : FILES_VIRTUOSO_COMPONENTS;

    return (
      <Virtuoso
        components={components}
        computeItemKey={computeFileRowKey}
        itemContent={renderFileRow}
        scrollerRef={scrollerRef}
        totalCount={payload.files.length}
      />
    );
  }

  function computeFileRowKey(index: number) {
    return payload.files[index]?.path ?? index;
  }

  function handleImageError() {
    setImageBroken(true);
  }

  function renderFileRow(index: number) {
    const file = payload.files[index];
    if (!file) return <div className="h-10" />;

    return (
      <div className="px-2">
        <FilePreviewRow file={file} />
      </div>
    );
  }

  function renderFilesFooter() {
    return (
      <div className="px-4 py-2 text-ant-secondary text-xs">
        {t("file.shownCount", {
          shown: payload.files.length,
          total: payload.totalFiles,
        })}
      </div>
    );
  }
};

/**
 * 虚拟文本列表上下留白。
 */
function PreviewTextPadding() {
  return <div className="h-4" />;
}

/**
 * 虚拟文件列表顶部留白。
 */
function PreviewFilesPadding() {
  return <div className="h-2" />;
}

/**
 * 文件 viewer 的单行展示。
 */
const FilePreviewRow: FC<FilePreviewRowProps> = (props) => {
  const { file } = props;
  const { t } = useTranslation("preview");
  const kindLabel = file.isDir ? t("file.folder") : t("file.item");
  const sizeLabel = file.size === null ? kindLabel : formatBytes(file.size);

  return (
    <div
      className={cn(
        "flex min-h-10 items-center gap-2 rounded-1.5 px-2 py-1.5",
        { "opacity-50": !file.exists },
      )}
      title={file.path}
    >
      {file.iconPath ? (
        <AssetImage className="size-6 shrink-0" src={file.iconPath} />
      ) : (
        <i
          aria-hidden
          className="i-lucide:file size-5 shrink-0 text-ant-secondary"
        />
      )}

      <div className="min-w-0 flex-1">
        <div
          className={cn("truncate text-xs", {
            "line-through": !file.exists,
          })}
        >
          {file.name}
        </div>
        <div className="truncate text-ant-secondary text-xs">
          {file.exists ? file.path : t("file.missingPath")}
        </div>
      </div>

      <span className="shrink-0 text-ant-secondary text-xs">{sizeLabel}</span>
    </div>
  );
};

/**
 * 将长文本拆成虚拟行，超长单行按固定字符数软切块。
 */
function buildTextPreviewRows(text: string) {
  const rows: string[] = [];

  for (const line of text.split("\n")) {
    if (line.length === 0) {
      rows.push("");
      continue;
    }

    for (
      let start = 0;
      start < line.length;
      start += PREVIEW_TEXT_SOFT_WRAP_CHARS
    ) {
      rows.push(line.slice(start, start + PREVIEW_TEXT_SOFT_WRAP_CHARS));
    }
  }

  return rows;
}

/**
 * 生成 Content Viewer 标题。
 */
function previewTitle(
  t: TFunction<"preview">,
  payload: ClipboardPreviewPayload,
) {
  if (payload.kind === "files") {
    return t("title.files", { count: payload.totalFiles });
  }

  if (payload.kind === "image") {
    return t("title.image");
  }

  return t("title.text");
}

/**
 * 生成 Content Viewer 元信息。
 */
function previewMeta(
  t: TFunction<"preview">,
  payload: ClipboardPreviewPayload,
) {
  if (payload.kind === "files") {
    return t("meta.filesLoaded", { count: payload.files.length });
  }

  if (payload.kind === "image") {
    const dimensions =
      payload.imageWidth && payload.imageHeight
        ? `${payload.imageWidth} x ${payload.imageHeight}`
        : t("meta.unknownSize");
    const size = payload.size === null ? "" : ` · ${formatBytes(payload.size)}`;

    return `${dimensions}${size}`;
  }

  return t("meta.characters", {
    count: payload.size ?? payload.text?.length ?? 0,
  });
}

/**
 * 格式化字节大小为紧凑文本。
 */
function formatBytes(value: number) {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let size = value;
  let unitIndex = 0;

  while (size >= 1024 && unitIndex < units.length - 1) {
    size /= 1024;
    unitIndex += 1;
  }

  const fractionDigits = unitIndex === 0 || size >= 10 ? 0 : 1;

  return `${size.toFixed(fractionDigits)} ${units[unitIndex]}`;
}

export const PREVIEW_CACHE_LIMIT = 16;
/**
 * 预览 LRU 的字节预算：按 payload 估算占用（text 长度 + files 路径长度）驱逐，
 * 避免几条超大文本（如整段日志）把常驻 webview 内存撑到几十 MB。
 */
export const PREVIEW_CACHE_MAX_BYTES = 2 * 1024 * 1024;
export const PREVIEW_PANEL_GAP = 40;
export const PREVIEW_PANEL_MARGIN = 32;
export const PREVIEW_PANEL_MIN_HEIGHT = 96;
export const PREVIEW_PANEL_MIN_WIDTH = 288;
export const PREVIEW_PANEL_MAX_HEIGHT = 480;
export const PREVIEW_PANEL_MAX_WIDTH = 480;
export const PREVIEW_PANEL_HEADER_HEIGHT = 48;
export const PREVIEW_PANEL_IMAGE_PADDING_X = 32;
export const PREVIEW_PANEL_IMAGE_PADDING_Y = 32;
export const PREVIEW_EMPTY_CONTENT_HEIGHT = 96;
export const PREVIEW_TEXT_ROW_HEIGHT = 22;
export const PREVIEW_TEXT_VERTICAL_PADDING = 32;
export const PREVIEW_TEXT_SOFT_WRAP_CHARS = 32;
export const PREVIEW_FILE_ROW_HEIGHT = 40;
export const PREVIEW_FILE_VERTICAL_PADDING = 16;
export const PREVIEW_FILE_MORE_FOOTER_HEIGHT = 40;
export const PREVIEW_PANEL_FALLBACK_SIZE = {
  height: 160,
  width: 320,
};
export const PREVIEW_SPRING = {
  damping: 34,
  mass: 0.9,
  stiffness: 420,
};
export const PREVIEW_EXIT_ANIMATION_MS = 160;
/** 复制按钮成功后的对勾停留时长（面板头部与选区浮动按钮共用）。 */
export const PREVIEW_COPY_FEEDBACK_MS = 1000;
export const PREVIEW_PANEL_TRANSITION = {
  duration: 0.18,
  ease: [0.22, 1, 0.36, 1],
} as const;
export const PREVIEW_CONTENT_TRANSITION = {
  duration: 0.12,
  ease: "easeOut",
} as const;
export const PREVIEW_PANEL_VARIANTS = {
  closed: {
    opacity: 0,
    scale: 0.96,
  },
  open: {
    opacity: 1,
    scale: 1,
  },
};
export const PREVIEW_CONNECTOR_VARIANTS = {
  closed: {
    opacity: 0,
  },
  open: {
    opacity: 1,
  },
};

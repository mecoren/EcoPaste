import { useEventListener, useUnmount } from "ahooks";
import type { FC, PointerEvent as ReactPointerEvent, RefObject } from "react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { writeTextToClipboard } from "@/commands";
import { cn } from "@/utils/cn";
import { log } from "@/utils/log";
import { PREVIEW_COPY_FEEDBACK_MS } from "../constants";
import {
  clearSelection,
  isSelectionInside,
  publishSelection,
  readSelectionText,
} from "../selection";

/** 选区变化防抖：拖选过程中 `selectionchange` 触发极密。 */
const SELECTION_SYNC_DEBOUNCE_MS = 150;
/** 复制按钮边长与面板内边距（px），与下方 `size-6` 类保持一致。 */
const BUTTON_SIZE = 24;
const BUTTON_GAP = 6;
const PANEL_PADDING = 4;

interface SelectionAnchor {
  left: number;
  top: number;
}

interface SelectionCopyButtonProps {
  /** 预览面板元素：选区必须落在它内部才算「预览内容里的选中」。 */
  containerRef: RefObject<HTMLDivElement | null>;
  /** 预览可交互且处于打开态；关闭或回到穿透态时立即清空选区并通知主窗。 */
  enabled: boolean;
}

/**
 * 选区浮动复制按钮：面板内选中文本后，在选区尾端上方浮现一个复制按钮。
 *
 * 面板的鼠标穿透开关由 Rust 控制，这里只负责 DOM 选区、按钮定位与复制，
 * 并把「当前选中片段」同步给主窗口供 Ctrl/Cmd+C 仲裁。
 */
const SelectionCopyButton: FC<SelectionCopyButtonProps> = (props) => {
  const { containerRef, enabled } = props;
  const { t } = useTranslation("preview");
  const [anchor, setAnchor] = useState<SelectionAnchor | null>(null);
  const [copied, setCopied] = useState(false);
  const enabledRef = useRef(enabled);
  const textRef = useRef<string | null>(null);
  const syncTimerRef = useRef<number | null>(null);
  const feedbackTimerRef = useRef<number | null>(null);

  enabledRef.current = enabled;

  // biome-ignore lint/correctness/useExhaustiveDependencies: 只在可交互开关翻转时复位局部状态；复位函数每轮 render 都是新引用，入依赖会退化成每帧执行
  useEffect(() => {
    if (enabled) return;

    cancelSyncTimer();
    cancelFeedbackTimer();
    clearSelection();
    resetButton();
    void publishSelection(null);
  }, [enabled]);

  useEventListener("selectionchange", handleSelectionChange, {
    target: document,
  });

  useUnmount(() => {
    cancelSyncTimer();
    cancelFeedbackTimer();
    void publishSelection(null);
  });

  if (!anchor) return null;

  return (
    <button
      aria-label={t("selection.copy")}
      className={cn(
        "fixed z-20 flex size-6 cursor-pointer items-center justify-center rounded-1 border border-ant-border bg-ant-container/95 text-ant-secondary shadow-md transition-colors hover:text-ant-text",
        { "text-ant-primary": copied },
      )}
      onClick={handleCopy}
      onPointerDown={handlePointerDown}
      style={anchor}
      title={copied ? t("selection.copied") : t("selection.copy")}
      type="button"
    >
      <i
        aria-hidden="true"
        className={copied ? "i-lucide:check" : "i-lucide:copy"}
      />
    </button>
  );

  function handleSelectionChange() {
    cancelSyncTimer();
    syncTimerRef.current = window.setTimeout(
      syncFromSelection,
      SELECTION_SYNC_DEBOUNCE_MS,
    );
  }

  /**
   * 把 DOM 选区同步成按钮位置与跨窗片段：选区消失 / 在面板外时清空并广播 `null`。
   */
  function syncFromSelection() {
    syncTimerRef.current = null;

    if (!enabledRef.current) return;

    const container = containerRef.current;
    if (!container) return;

    const selection = window.getSelection();
    const text = readSelectionText();

    if (!text || !selection || selection.rangeCount === 0) {
      clearButtonState();
      return;
    }

    const range = selection.getRangeAt(0);
    if (!isSelectionInside(container, range)) {
      clearButtonState();
      return;
    }

    textRef.current = text;
    setCopied(false);
    setAnchor(resolveButtonAnchor(container, range));
    void publishSelection(text);
  }

  /** 选区消失路径：清按钮 + 广播 `null`（保留 DOM 选区，交给浏览器）。 */
  function clearButtonState() {
    resetButton();
    void publishSelection(null);
  }

  function resetButton() {
    cancelFeedbackTimer();
    textRef.current = null;
    setAnchor(null);
    setCopied(false);
  }

  /**
   * 按下时保住选区：不 `preventDefault` 的话浏览器会先把选区折叠掉，点击时就取不到文本了。
   */
  function handlePointerDown(event: ReactPointerEvent<HTMLButtonElement>) {
    event.preventDefault();
  }

  async function handleCopy() {
    const text = textRef.current;
    if (!text) return;

    cancelFeedbackTimer();
    setCopied(true);

    try {
      // 静默写入：按钮自身的 ✓ 就是反馈，避免预览窗与主窗各飘一条 toast。
      await writeTextToClipboard(text, { silent: true });
      log.info("preview selection copied by button", { chars: text.length });
    } catch (error) {
      log.error("preview selection copy failed", error);
      setCopied(false);
      return;
    }

    feedbackTimerRef.current = window.setTimeout(() => {
      feedbackTimerRef.current = null;
      // 复制完清掉选区：`selectionchange` 会再走一次同步并广播 `null`。
      clearSelection();
    }, PREVIEW_COPY_FEEDBACK_MS);
  }

  function cancelSyncTimer() {
    if (syncTimerRef.current === null) return;

    window.clearTimeout(syncTimerRef.current);
    syncTimerRef.current = null;
  }

  function cancelFeedbackTimer() {
    if (feedbackTimerRef.current === null) return;

    window.clearTimeout(feedbackTimerRef.current);
    feedbackTimerRef.current = null;
  }
};

/**
 * 把复制按钮放到选区尾端上方；贴近面板顶部时改放选区下方，并横向收敛进面板内。
 * 全部用 viewport 坐标——按钮是 `fixed` 定位，而预览窗本身就是整块显示器的 overlay。
 */
function resolveButtonAnchor(
  container: HTMLElement,
  range: Range,
): SelectionAnchor {
  const selectionRect = range.getBoundingClientRect();
  const panelRect = container.getBoundingClientRect();
  const minLeft = panelRect.left + PANEL_PADDING;
  const maxLeft = Math.max(
    minLeft,
    panelRect.right - BUTTON_SIZE - PANEL_PADDING,
  );
  const minTop = panelRect.top + PANEL_PADDING;
  const maxTop = Math.max(
    minTop,
    panelRect.bottom - BUTTON_SIZE - PANEL_PADDING,
  );
  const fitsAbove =
    selectionRect.top - BUTTON_GAP - BUTTON_SIZE >=
    panelRect.top + PANEL_PADDING;
  const preferredTop = fitsAbove
    ? selectionRect.top - BUTTON_GAP - BUTTON_SIZE
    : selectionRect.bottom + BUTTON_GAP;

  return {
    left: clamp(selectionRect.right - BUTTON_SIZE, minLeft, maxLeft),
    top: clamp(preferredTop, minTop, maxTop),
  };
}

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

export default SelectionCopyButton;

import { useEventListener } from "ahooks";
import type { PointerEvent as ReactPointerEvent } from "react";
import { useEffect, useRef, useState } from "react";
import { useSnapshot } from "valtio";
import { showClipboardPreview } from "@/commands";
import { TAURI_EVENT } from "@/constants/events";
import { WINDOW_LABEL } from "@/constants/windows";
import { useKeyboardEvent } from "@/hooks/useKeyboardEvent";
import { useTauriListen } from "@/hooks/useTauriListen";
import { settingsState } from "@/stores/settings";
import type { ClipboardItem } from "@/types/clipboard";
import { log } from "@/utils/log";
import {
  clearHoverTimer,
  closeClipboardPreviewSilently,
  HOVER_DELAY_MS,
  HOVER_HIDE_BUFFER_MS,
  isSpaceKey,
  type PreviewSession,
  type PreviewTrigger,
  type UseClipboardPreviewControllerOptions,
  type WindowVisibilityPayload,
} from "./previewController";

const KEYBOARD_PREVIEW_MAX_FRAMES = 36;
const KEYBOARD_PREVIEW_STABLE_FRAMES = 2;
const KEYBOARD_PREVIEW_RECT_EPSILON = 0.5;

interface PreviewRectSnapshot {
  left: number;
  top: number;
  width: number;
  height: number;
}

interface KeyboardPreviewTarget {
  frames: number;
  item: ClipboardItem;
  lastRect: PreviewRectSnapshot | null;
  stableFrames: number;
}

/**
 * 管理剪贴板预览的 hover / keyboard 生命周期和窗口隐藏兜底。
 */
export function useClipboardPreviewController(
  options: UseClipboardPreviewControllerOptions,
) {
  const { getActiveItem, itemElementMapRef, onHoverSelect } = options;
  const [previewSession, setPreviewSession] = useState<PreviewSession | null>(
    null,
  );
  const hoverTimerRef = useRef<number | null>(null);
  const hoverHideTimerRef = useRef<number | null>(null);
  // 指针是否停在预览面板上（由 Rust 的 `preview://pointer` 事件驱动）。
  const panelPointerInsideRef = useRef(false);
  const clipboardWindowVisibleRef = useRef(true);
  const previewSessionRef = useRef<PreviewSession | null>(null);
  const previewOpenRequestIdRef = useRef(0);
  const previewMoveFrameRef = useRef<number | null>(null);
  const previewMoveTargetRef = useRef<{
    item: ClipboardItem;
    pointerY: number;
  } | null>(null);
  const keyboardPreviewFrameRef = useRef<number | null>(null);
  const keyboardPreviewTargetRef = useRef<KeyboardPreviewTarget | null>(null);
  const pendingHoverTargetRef = useRef<{
    item: ClipboardItem;
    pointerY: number;
  } | null>(null);
  const settingsSnapshot = useSnapshot(settingsState);
  const previewSettings = settingsSnapshot.clipboard.preview;

  useEffect(() => {
    if (!previewSettings.hoverEnabled && previewSession?.trigger === "hover") {
      clearHoverTimer(hoverTimerRef);
      clearHoverTimer(hoverHideTimerRef);
      previewSessionRef.current = null;
      setPreviewSession(null);
      closeClipboardPreviewSilently("hoverDisabled");
      return;
    }

    if (
      !previewSettings.spaceEnabled &&
      previewSession?.trigger === "keyboard"
    ) {
      clearHoverTimer(hoverTimerRef);
      clearHoverTimer(hoverHideTimerRef);
      previewSessionRef.current = null;
      setPreviewSession(null);
      closeClipboardPreviewSilently("spaceDisabled");
    }
  }, [
    previewSettings.hoverEnabled,
    previewSettings.spaceEnabled,
    previewSession?.trigger,
  ]);

  useEffect(() => {
    return () => {
      clearHoverTimer(hoverTimerRef);
      clearHoverTimer(hoverHideTimerRef);
      pendingHoverTargetRef.current = null;
      if (keyboardPreviewFrameRef.current !== null) {
        window.cancelAnimationFrame(keyboardPreviewFrameRef.current);
        keyboardPreviewFrameRef.current = null;
      }
      keyboardPreviewTargetRef.current = null;
      if (previewMoveFrameRef.current !== null) {
        window.cancelAnimationFrame(previewMoveFrameRef.current);
        previewMoveFrameRef.current = null;
        previewMoveTargetRef.current = null;
      }
    };
  }, []);

  const handleWindowVisibility = (event: {
    payload: WindowVisibilityPayload;
  }) => {
    if (event.payload.label !== WINDOW_LABEL.CLIPBOARD) return;

    clipboardWindowVisibleRef.current = event.payload.visible;
    if (event.payload.visible) return;

    closePreview("windowHidden");
  };

  useTauriListen(TAURI_EVENT.WINDOW_VISIBILITY, handleWindowVisibility);

  /**
   * 指针进出预览面板：进入时抵消「离开列表区」的关闭缓冲，离开时重新进入缓冲。
   * 主窗的 `pointerleave` 与 Rust 的指针采样有先后不确定性，故同时用 ref 记录面板状态，
   * 让 `scheduleHoverHide` 在指针仍停在面板上时不再安排关闭（见其实现）。
   */
  const handlePreviewPointer = (event: { payload: { inside: boolean } }) => {
    const { inside } = event.payload;
    panelPointerInsideRef.current = inside;

    if (inside) {
      cancelHoverHide();
      return;
    }

    scheduleHoverHide("previewPanelLeave");
  };

  useTauriListen<{ inside: boolean }>(
    TAURI_EVENT.PREVIEW_POINTER,
    handlePreviewPointer,
  );

  /**
   * 主窗失焦：用户切到别的应用时收起预览。
   *
   * 例外：指针停在预览面板上时不关。面板是可交互的兄弟窗口，**点击它会让主窗失焦**，
   * 那不是「用户离开」，而是用户正在面板里操作；不豁免的话点一下面板预览就被自己关掉。
   */
  const handleWindowBlur = () => {
    if (!previewSessionRef.current) return;
    if (panelPointerInsideRef.current) {
      log.debug("preview blur ignored: pointer inside panel");
      return;
    }

    closePreview("windowBlur");
  };

  useEventListener("blur", handleWindowBlur, { target: window });

  /**
   * 主窗尺寸变化后面板定位就失效了，正常应收起；但指针停在面板上时不关
   * （点击面板会让主窗短暂走一次样式/焦点扰动，可能连带产生 resize 事件）。
   * 等指针离开面板，关闭缓冲会自然收掉。
   */
  const handleWindowResize = () => {
    if (panelPointerInsideRef.current) {
      log.debug("preview resize ignored: pointer inside panel");
      return;
    }

    closePreview("windowResize");
  };

  useEventListener("resize", handleWindowResize, { target: window });

  /**
   * 关闭预览窗口并清理本地预览会话。
   */
  const closePreview = (reason: string) => {
    previewOpenRequestIdRef.current += 1;
    cancelHoverPreview();
    cancelHoverHide();
    cancelPreviewMoveFrame();
    cancelKeyboardPreviewFrame();
    commitPreviewSession(null);
    closeClipboardPreviewSilently(reason);
  };

  /**
   * 卡片指针进入：keyboard 预览直接复用键盘重定向；hover 预览按延迟打开或重定向。
   */
  const handleItemPointerEnter = (
    item: ClipboardItem,
    event: ReactPointerEvent<HTMLDivElement>,
  ) => {
    const pointerY = event.clientY;

    onHoverSelect(item.id);

    if (!clipboardWindowVisibleRef.current) return;
    if (previewSessionRef.current?.trigger === "keyboard") {
      cancelHoverPreview();
      cancelHoverHide();

      if (previewSessionRef.current.itemId === item.id) {
        cancelKeyboardPreviewFrame();
        return;
      }

      scheduleKeyboardPreviewMove(item);
      return;
    }
    if (!previewSettings.hoverEnabled) return;

    cancelHoverPreview();
    cancelHoverHide();
    pendingHoverTargetRef.current = { item, pointerY };

    if (previewSessionRef.current?.trigger === "hover") {
      if (previewSessionRef.current.itemId === item.id) return;

      void openPreviewForItem(item, "hover", pointerY);
      return;
    }

    hoverTimerRef.current = window.setTimeout(() => {
      hoverTimerRef.current = null;

      if (!clipboardWindowVisibleRef.current) return;
      if (!settingsState.clipboard.preview.hoverEnabled) return;

      const target = pendingHoverTargetRef.current;

      if (!target || target.item.id !== item.id) return;

      void openPreviewForItem(target.item, "hover", target.pointerY);
    }, HOVER_DELAY_MS[previewSettings.hoverDelayMs]);
  };

  /**
   * Hover 预览打开后，同一卡片内移动鼠标会持续重定向锚点和预览卡片。
   */
  const handleItemPointerMove = (
    item: ClipboardItem,
    event: ReactPointerEvent<HTMLDivElement>,
  ) => {
    const pointerY = event.clientY;

    if (hoverTimerRef.current !== null) {
      pendingHoverTargetRef.current = { item, pointerY };
    }

    if (previewSessionRef.current?.trigger !== "hover") return;
    if (previewSessionRef.current.itemId !== item.id) return;

    previewMoveTargetRef.current = {
      item,
      pointerY,
    };

    if (previewMoveFrameRef.current !== null) return;

    previewMoveFrameRef.current = window.requestAnimationFrame(() => {
      previewMoveFrameRef.current = null;

      const target = previewMoveTargetRef.current;
      previewMoveTargetRef.current = null;

      if (!target) return;
      if (previewSessionRef.current?.trigger !== "hover") return;
      if (previewSessionRef.current.itemId !== target.item.id) return;

      void openPreviewForItem(target.item, "hover", target.pointerY);
    });
  };

  /**
   * Hover 离开单个卡片时进入准备隐藏状态；进入新卡片会取消隐藏。
   */
  const handleItemPointerLeave = () => {
    scheduleHoverHide("itemPointerLeave");
  };

  /**
   * 指针离开列表区域时结束 hover preview，卡片间移动只做 retarget。
   */
  const handlePreviewAreaPointerLeave = () => {
    scheduleHoverHide("hoverAreaLeave");
  };

  /**
   * 指针离开当前 document/window 时兜底关闭，避免 webview 边界漏掉元素级 leave。
   */
  const handleDocumentPointerOut = (event: PointerEvent) => {
    if (event.relatedTarget !== null) return;

    scheduleHoverHide("documentPointerOut");
  };

  useEventListener("pointerout", handleDocumentPointerOut, {
    target: document,
  });

  /**
   * 浏览器取消后续指针事件时关闭 hover preview，避免预览残留。
   */
  const handleDocumentPointerCancel = () => {
    scheduleHoverHide("documentPointerCancel");
  };

  useEventListener("pointercancel", handleDocumentPointerCancel, {
    target: document,
  });

  /**
   * Space 按下打开当前 active item；忽略重复 keydown，避免重复 IPC。
   */
  const handlePreviewSpaceDown = (event: KeyboardEvent) => {
    event.preventDefault();

    if (!previewSettings.spaceEnabled) return;
    if (event.repeat && previewSession?.trigger === "keyboard") return;

    cancelHoverPreview();
    cancelHoverHide();

    const activeItem = getActiveItem();

    if (!activeItem) return;

    void openPreviewForItem(activeItem, "keyboard");
  };

  /**
   * Space 松开关闭 keyboard preview。
   */
  const handlePreviewSpaceUp = (event: KeyboardEvent) => {
    if (!isSpaceKey(event)) return;

    event.preventDefault();

    if (previewSession?.trigger !== "keyboard") return;

    closePreview("spaceUp");
  };

  useKeyboardEvent("keyup", handlePreviewSpaceUp);

  /**
   * 方向键移动到新 active item 时同步 keyboard preview。
   */
  const handleKeyboardPreviewMove = (item: ClipboardItem) => {
    if (previewSessionRef.current?.trigger !== "keyboard") return;

    scheduleKeyboardPreviewMove(item);
  };

  /**
   * 取消等待中的 hover preview。
   */
  const cancelHoverPreview = () => {
    clearHoverTimer(hoverTimerRef);
    pendingHoverTargetRef.current = null;
  };

  /**
   * 滚动列表时关闭 hover preview，保留 keyboard preview。
   */
  const closeHoverPreviewForScroll = () => {
    if (previewSession?.trigger === "hover") {
      closePreview("scroll");
      return;
    }

    cancelHoverPreview();
  };

  /**
   * 打开或重定向指定条目的预览 overlay。
   */
  const openPreviewForItem = async (
    item: ClipboardItem,
    trigger: PreviewTrigger,
    pointerY?: number,
  ) => {
    if (!clipboardWindowVisibleRef.current) return;

    const element = itemElementMapRef.current.get(item.id);
    const rect = element?.getBoundingClientRect();

    if (!rect || rect.width <= 0 || rect.height <= 0) return;

    const requestId = previewOpenRequestIdRef.current + 1;
    previewOpenRequestIdRef.current = requestId;
    commitPreviewSession({ itemId: item.id, trigger });

    try {
      if (!clipboardWindowVisibleRef.current) return;

      const state = await showClipboardPreview(item.id, {
        height: rect.height,
        left: rect.left,
        pointerY,
        top: rect.top,
        width: rect.width,
      });

      if (requestId !== previewOpenRequestIdRef.current) return;
      if (!state || !clipboardWindowVisibleRef.current) {
        closePreview("previewShowSuppressed");
      }
    } catch (error) {
      if (requestId === previewOpenRequestIdRef.current) {
        log.error("show clipboard preview failed", { error, trigger });
        commitPreviewSession(null);
      }
      return;
    }
  };

  /**
   * 取消等待中的 hover 隐藏缓冲。
   */
  const cancelHoverHide = () => {
    clearHoverTimer(hoverHideTimerRef);
  };

  /**
   * 结束 hover preview；keyboard preview 不受指针离开影响。
   */
  const closeHoverPreview = (reason: string) => {
    cancelHoverPreview();
    cancelHoverHide();

    if (previewSessionRef.current?.trigger !== "hover") return;

    closePreview(reason);
  };

  /**
   * 鼠标离开剪贴板项后进入准备隐藏状态，短时间内进入新项会取消隐藏。
   * 指针已停在预览面板上时不安排关闭：主窗 `pointerleave` 与 Rust 指针采样的先后
   * 顺序不确定，若此时排上缓冲会因为没有后续事件来抵消而误关正在交互的面板。
   */
  const scheduleHoverHide = (reason: string) => {
    cancelHoverPreview();
    cancelHoverHide();

    if (previewSessionRef.current?.trigger !== "hover") return;
    if (panelPointerInsideRef.current) return;

    hoverHideTimerRef.current = window.setTimeout(() => {
      hoverHideTimerRef.current = null;
      closeHoverPreview(reason);
    }, HOVER_HIDE_BUFFER_MS);
  };

  /**
   * 取消等待中的 mousemove retarget 帧。
   */
  function cancelPreviewMoveFrame() {
    if (previewMoveFrameRef.current === null) return;

    window.cancelAnimationFrame(previewMoveFrameRef.current);
    previewMoveFrameRef.current = null;
    previewMoveTargetRef.current = null;
    pendingHoverTargetRef.current = null;
  }

  /**
   * 方向键会先触发虚拟列表滚动，等待目标卡片位置稳定后再重定向预览。
   */
  function scheduleKeyboardPreviewMove(item: ClipboardItem) {
    cancelKeyboardPreviewFrame();
    keyboardPreviewTargetRef.current = {
      frames: 0,
      item,
      lastRect: null,
      stableFrames: 0,
    };
    requestKeyboardPreviewFrame();
  }

  /**
   * 请求下一帧键盘预览位置采样。
   */
  function requestKeyboardPreviewFrame() {
    keyboardPreviewFrameRef.current = window.requestAnimationFrame(
      handleKeyboardPreviewFrame,
    );
  }

  /**
   * 等待目标卡片 DOMRect 在滚动后稳定，再用最新位置打开预览。
   */
  function handleKeyboardPreviewFrame() {
    keyboardPreviewFrameRef.current = null;

    const target = keyboardPreviewTargetRef.current;

    if (!target) return;

    if (previewSessionRef.current?.trigger !== "keyboard") {
      keyboardPreviewTargetRef.current = null;
      return;
    }

    target.frames += 1;

    const rect = resolveItemRect(target.item.id);

    if (!rect) {
      retryKeyboardPreviewFrame(target);
      return;
    }

    if (
      target.lastRect &&
      isPreviewRectStable(target.lastRect, rect, KEYBOARD_PREVIEW_RECT_EPSILON)
    ) {
      target.stableFrames += 1;
    } else {
      target.stableFrames = 0;
    }

    target.lastRect = rect;

    if (
      target.stableFrames >= KEYBOARD_PREVIEW_STABLE_FRAMES ||
      target.frames >= KEYBOARD_PREVIEW_MAX_FRAMES
    ) {
      keyboardPreviewTargetRef.current = null;
      void openPreviewForItem(target.item, "keyboard");
      return;
    }

    requestKeyboardPreviewFrame();
  }

  /**
   * 目标卡片尚未挂载或滚动仍在进行时继续等待，超过上限后放弃旧目标。
   */
  function retryKeyboardPreviewFrame(target: KeyboardPreviewTarget) {
    if (target.frames >= KEYBOARD_PREVIEW_MAX_FRAMES) {
      keyboardPreviewTargetRef.current = null;
      return;
    }

    requestKeyboardPreviewFrame();
  }

  /**
   * 取消等待中的键盘预览位置采样。
   */
  function cancelKeyboardPreviewFrame() {
    if (keyboardPreviewFrameRef.current !== null) {
      window.cancelAnimationFrame(keyboardPreviewFrameRef.current);
      keyboardPreviewFrameRef.current = null;
    }

    keyboardPreviewTargetRef.current = null;
  }

  /**
   * 读取列表项当前 DOMRect，并转成可比较的普通对象。
   */
  function resolveItemRect(id: string): PreviewRectSnapshot | null {
    const element = itemElementMapRef.current.get(id);
    const rect = element?.getBoundingClientRect();

    if (!rect || rect.width <= 0 || rect.height <= 0) return null;

    return {
      height: rect.height,
      left: rect.left,
      top: rect.top,
      width: rect.width,
    };
  }

  /**
   * 同步预览会话 state 与 ref，供 hover 定时器和快速切换时读取最新会话。
   */
  function commitPreviewSession(session: PreviewSession | null) {
    previewSessionRef.current = session;
    setPreviewSession(session);
  }

  return {
    closeHoverPreviewForScroll,
    closePreview,
    handleItemPointerEnter,
    handleItemPointerLeave,
    handleItemPointerMove,
    handleKeyboardPreviewMove,
    handlePreviewAreaPointerLeave,
    handlePreviewSpaceDown,
    previewSession,
  };
}

/**
 * 判断两帧 DOMRect 是否已经稳定，避免 smooth scroll 中途采样旧坐标。
 */
function isPreviewRectStable(
  prev: PreviewRectSnapshot,
  next: PreviewRectSnapshot,
  epsilon: number,
) {
  return (
    Math.abs(prev.left - next.left) <= epsilon &&
    Math.abs(prev.top - next.top) <= epsilon &&
    Math.abs(prev.width - next.width) <= epsilon &&
    Math.abs(prev.height - next.height) <= epsilon
  );
}

/**
 * 判断 Space 键。
 */
export { isSpaceKey } from "./previewController";

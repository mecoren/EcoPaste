import { useEventListener, useLatest } from "ahooks";
import { TAURI_EVENT } from "@/constants/events";
import { findEditableElement } from "@/utils/dom";
import { isWinClipboardWindow } from "@/utils/is";
import { useTauriListen } from "./useTauriListen";

type KeyboardEventType = "keydown" | "keyup";

const EDITABLE_GLOBAL_KEYBOARD_ATTRIBUTE = "data-allow-global-keyboard";
const EDITABLE_GLOBAL_KEYBOARD_SELECTOR = `[${EDITABLE_GLOBAL_KEYBOARD_ATTRIBUTE}="true"]`;

/**
 * 允许从全局键盘输入控件（搜索框）交还给全局导航的按键。
 *
 * 只收垂直导航与提交类按键：左右键在输入控件内是光标移动，
 * 交还会先 blur 输入框，用户将彻底失去光标控制，因此始终留在输入控件内。
 */
const EDITABLE_GLOBAL_HANDOFF_KEYS = new Set([
  "ArrowDown",
  "ArrowUp",
  "Control",
  "Enter",
  "Escape",
  "Tab",
]);

interface NavEventPayload {
  code?: string;
  type: KeyboardEventType;
  key: string;
  ctrlKey?: boolean;
  shiftKey?: boolean;
}

/**
 * 跨平台键盘事件监听 hook。
 *
 * macOS 与可聚焦窗口直接监听浏览器键盘事件；Windows 剪贴板窗口默认不可聚焦，
 * 导航键通常来自 Rust 低级钩子，但输入控件仍保留浏览器原生输入行为。
 *
 * 优先级：焦点落在可编辑控件内时输入状态优先——只有 `EDITABLE_GLOBAL_HANDOFF_KEYS`
 * 会交还给全局 handler，其余按键（含左右键）留作控件原生行为。
 */
export const useKeyboardEvent = (
  type: KeyboardEventType,
  handler: (event: KeyboardEvent) => void,
) => {
  const isWindowsClipboardWindow = isWinClipboardWindow();
  const handlerRef = useLatest(handler);

  const handleBrowserEvent = (event: KeyboardEvent) => {
    if (isWindowsClipboardWindow) {
      const editableTarget = findEditableElement(event.target);
      if (editableTarget) {
        if (!shouldHandoffEditableKeyboard(editableTarget, event)) return;

        editableTarget.blur();
      }
    }

    handlerRef.current(event);
  };

  useEventListener(type, handleBrowserEvent);

  useTauriListen<NavEventPayload>(TAURI_EVENT.KEYBOARD_NAV, (event) => {
    if (!isWindowsClipboardWindow) return;
    if (shouldUseNativeEditableKeyboard(document.activeElement)) return;

    const { type: payloadType, ...rest } = event.payload;

    if (payloadType !== type || !rest.key) return;

    handlerRef.current(
      new KeyboardEvent(payloadType, { cancelable: true, ...rest }),
    );
  });
};

function shouldUseNativeEditableKeyboard(target: EventTarget | null) {
  return findEditableElement(target) !== null;
}

function shouldHandoffEditableKeyboard(
  target: HTMLElement,
  event: KeyboardEvent,
) {
  if (event.type !== "keydown") return false;
  if (!target.closest(EDITABLE_GLOBAL_KEYBOARD_SELECTOR)) return false;

  return EDITABLE_GLOBAL_HANDOFF_KEYS.has(event.key);
}

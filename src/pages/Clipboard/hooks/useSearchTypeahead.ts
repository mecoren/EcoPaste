import { useEventListener } from "ahooks";
import type { InputRef } from "antd";
import type { RefObject } from "react";
import { useCallback } from "react";
import { searchTypingAck } from "@/commands";
import { TAURI_EVENT } from "@/constants/events";
import { prepareClipboardWindowEditableFocus } from "@/hooks/useClipboardWindowEditableFocus";
import { useTauriListen } from "@/hooks/useTauriListen";
import { findEditableElement } from "@/utils/dom";

interface SearchTypeaheadOptions {
  /** 搜索框 ref；type-ahead 把焦点送回这里。 */
  inputRef: RefObject<InputRef | null>;
}

/**
 * 搜索框常驻就绪（Ditto 式随时输入即搜索）：
 *
 * 1. 浏览器真实 keydown 路径（macOS 全程；Windows 窗口已聚焦但输入框被 handoff blur 后）：
 *    可打印字符 / Backspace 且焦点不在可编辑元素时，同步聚焦搜索框，
 *    浏览器默认动作把字符落在输入框。聚焦必须在事件循环本轮完成——
 *    先 await 窗口命令再聚焦会错过默认动作，首字符丢失。
 * 2. Rust `clipboard://search-typing` 路径（Windows 窗口未聚焦）：钩子已吞掉字符，
 *    这里负责聚焦搜索框并 ack，Rust 随后回放被吞的按键（保证 IME 从首字符正确组合）。
 *
 * 守卫：弹窗打开（antd Modal `[role="dialog"]`）时不劫持——输入应落入弹窗内的输入框。
 */
export const useSearchTypeahead = (options: SearchTypeaheadOptions) => {
  const { inputRef } = options;

  const focusSearch = useCallback(() => {
    inputRef.current?.focus({ cursor: "end" });
  }, [inputRef]);

  const handleKeyDown = useCallback(
    (event: KeyboardEvent) => {
      if (shouldSteerToSearch(event)) {
        focusSearch();
      }
    },
    [focusSearch],
  );

  // window 级监听真实浏览器事件：macOS 全程；Windows 窗口已聚焦但输入框被 handoff blur 后。
  useEventListener("keydown", handleKeyDown, { target: window });

  const handleTyping = useCallback(() => {
    const focusAndAck = async () => {
      // 弹窗打开时不抢焦点（与真实 keydown 路径守卫一致）；不 ack 时
      // Rust 回放等 150ms 超时后丢弃队列，字符不会落进弹窗背后的搜索框。
      if (hasOpenDialog()) return;

      await prepareClipboardWindowEditableFocus();
      inputRef.current?.focus({ cursor: "end" });
      await searchTypingAck();
    };

    void focusAndAck();
  }, [inputRef]);

  // Windows 钩子路径：钩子已吞掉字符，聚焦 + ack 后由 Rust 回放。
  useTauriListen<null>(TAURI_EVENT.SEARCH_TYPING, handleTyping);
};

/**
 * 判断 keydown 是否应把焦点引向搜索框：
 * 单个可打印字符或 Backspace、无修饰键、非 IME 组合中、
 * 焦点不在任何可编辑元素、无弹窗打开。
 *
 * Backspace 无条件引导：搜索词为空时聚焦空输入框是无害空操作，
 * 而 store 的 keyword 带 200ms 防抖、晚于输入框真实值，
 * 按它判断会丢掉「打字后立刻按方向键再退格」的首个退格。
 */
function shouldSteerToSearch(event: KeyboardEvent) {
  if (event.key !== "Backspace" && event.key.length !== 1) return false;
  if (event.key === " ") return false;
  if (event.ctrlKey || event.metaKey || event.altKey) return false;
  if (event.isComposing) return false;
  if (findEditableElement(event.target)) return false;
  if (hasOpenDialog()) return false;

  return true;
}

/**
 * 判断当前是否有 antd Modal / 类弹窗打开。
 */
function hasOpenDialog() {
  return document.querySelector('[role="dialog"]') !== null;
}

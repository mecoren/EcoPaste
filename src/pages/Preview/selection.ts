import { emitTo } from "@tauri-apps/api/event";
import { TAURI_EVENT } from "@/constants/events";
import { WINDOW_LABEL } from "@/constants/windows";
import { log } from "@/utils/log";

/**
 * 读取预览窗内当前 DOM 选区的文本；无选区、折叠选区或纯空白返回 `null`。
 */
export function readSelectionText(): string | null {
  const selection = window.getSelection();
  if (!selection || selection.isCollapsed) return null;

  const text = selection.toString();
  if (text.trim().length === 0) return null;

  return text;
}

/**
 * 判断选区是否落在预览面板内：面板外的选区不参与片段复制。
 */
export function isSelectionInside(container: HTMLElement, range: Range) {
  return container.contains(range.commonAncestorContainer);
}

/**
 * 清空预览窗内的 DOM 选区。
 */
export function clearSelection() {
  window.getSelection()?.removeAllRanges();
}

/**
 * 把「当前选中片段」同步给剪贴板主窗口，供 Ctrl/Cmd+C 仲裁；`null` 表示选区已清空。
 *
 * 主窗必须知道此刻有没有选区，否则陈旧片段会劫持「复制整条记录」的快捷键。
 */
export async function publishSelection(text: string | null) {
  log.debug("preview selection publish", {
    chars: text?.length ?? 0,
    hasSelection: text !== null,
  });

  try {
    await emitTo(WINDOW_LABEL.CLIPBOARD, TAURI_EVENT.PREVIEW_SELECTION, {
      text,
    });
  } catch (error) {
    log.error("publish preview selection failed", error);
  }
}

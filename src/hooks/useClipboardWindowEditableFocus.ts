import { useEffect } from "react";
import { setClipboardWindowEditing } from "@/commands";
import { findEditableElement } from "@/utils/dom";
import { isWinClipboardWindow } from "@/utils/is";

const EDITABLE_BLUR_RESTORE_DELAY_MS = 80;

export const prepareClipboardWindowEditableFocus = async () => {
  if (!isWinClipboardWindow()) return;

  await setClipboardWindowEditing(true);
};

/**
 * Windows 剪贴板窗口输入控件激活期间临时允许窗口聚焦，编辑结束后恢复不可聚焦。
 *
 * 不监听 focusout：方向键 handoff 等导航键会先 blur 输入框再执行全局快捷键，
 * 若 blur 即退出 editing，窗口会在用户还在窗口内浏览列表时把前台还给原应用，
 * 导致「导航后再打字」丢失。只有用户真正离开窗口（点击其它应用 / 窗口隐藏）才退出。
 */
export const useClipboardWindowEditableFocus = () => {
  useEffect(() => {
    if (!isWinClipboardWindow()) return;

    let editing = false;
    let restoreTimer = 0;

    const clearRestoreTimer = () => {
      if (restoreTimer === 0) return;

      window.clearTimeout(restoreTimer);
      restoreTimer = 0;
    };

    const setEditing = async (nextEditing: boolean) => {
      if (editing === nextEditing) return;

      editing = nextEditing;
      await setClipboardWindowEditing(nextEditing);
    };

    const activateEditableTarget = async (target: HTMLElement) => {
      clearRestoreTimer();
      await setEditing(true);

      if (!document.contains(target)) return;
      if (document.activeElement === target) return;
      if (findEditableElement(document.activeElement)) return;

      target.focus();
    };

    const handlePointerDown = (event: PointerEvent) => {
      const target = findEditableElement(event.target);
      if (!target) return;

      void activateEditableTarget(target);
    };

    const handleFocusIn = (event: FocusEvent) => {
      const target = findEditableElement(event.target);
      if (!target) return;

      clearRestoreTimer();
      void setEditing(true);
    };

    const scheduleRestore = () => {
      clearRestoreTimer();

      restoreTimer = window.setTimeout(() => {
        restoreTimer = 0;
        if (findEditableElement(document.activeElement)) return;

        void setEditing(false);
      }, EDITABLE_BLUR_RESTORE_DELAY_MS);
    };

    const handleVisibilityChange = () => {
      if (document.visibilityState === "visible") return;

      clearRestoreTimer();
      void setEditing(false);
    };

    window.addEventListener("pointerdown", handlePointerDown, true);
    window.addEventListener("focusin", handleFocusIn, true);
    // window blur 才是「用户离开窗口」信号（点击其它应用 / 系统转移前台）。
    window.addEventListener("blur", scheduleRestore);
    document.addEventListener("visibilitychange", handleVisibilityChange);

    return () => {
      clearRestoreTimer();
      window.removeEventListener("pointerdown", handlePointerDown, true);
      window.removeEventListener("focusin", handleFocusIn, true);
      window.removeEventListener("blur", scheduleRestore);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
      void setClipboardWindowEditing(false);
    };
  }, []);
};

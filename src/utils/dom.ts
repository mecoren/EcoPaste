/**
 * DOM 可编辑元素工具：多个键盘 / 焦点相关 hook 共用的判定逻辑。
 */

/**
 * 从事件目标向上查找所属的可编辑元素（input / textarea / contenteditable）。
 */
export const findEditableElement = (
  target: EventTarget | null,
): HTMLElement | null => {
  if (!(target instanceof Element)) return null;

  let element: Element | null = target;
  while (element) {
    if (element instanceof HTMLElement && isEditableElement(element)) {
      return element;
    }

    element = element.parentElement;
  }

  return null;
};

/**
 * 判断元素本身是否为可编辑控件。
 */
export const isEditableElement = (element: HTMLElement) => {
  if (element.isContentEditable) return true;

  const tagName = element.tagName.toLowerCase();

  return tagName === "input" || tagName === "textarea";
};

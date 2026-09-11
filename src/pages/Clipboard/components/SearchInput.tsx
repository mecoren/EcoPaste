import { Input, type InputProps, type InputRef } from "antd";
import type { ChangeEvent, CompositionEvent, FC, Ref } from "react";
import { useCallback, useEffect, useRef } from "react";
import KeyHint from "@/components/KeyHint";
import { prepareClipboardWindowEditableFocus } from "@/hooks/useClipboardWindowEditableFocus";

interface SearchInputProps extends Omit<InputProps, "prefix"> {
  blurToken?: number;
  clearToken?: number;
  focusToken?: number;
  /** 外部受控 ref（React 19 ref-as-prop）；type-ahead、Header 焦点效果共用。 */
  inputRef?: Ref<InputRef>;
}

/**
 * 带快捷键提示的搜索输入框，支持 ⌘F / Ctrl+F 聚焦。
 * IME 拼音/日文组合输入期间抑制 onChange，待 compositionend 再补发一次，
 * 避免上层防抖/受控逻辑被中间态拼字串污染。
 */
const SearchInput: FC<SearchInputProps> = (props) => {
  const {
    blurToken = 0,
    clearToken = 0,
    focusToken = 0,
    inputRef,
    onChange,
    onCompositionStart,
    onCompositionEnd,
    ...rest
  } = props;

  const localInputRef = useRef<InputRef>(null);
  // 外部传入 ref 时优先使用；内部 blur 效果与外部聚焦共用同一目标。
  const activeInputRef = inputRef ?? localInputRef;

  const composingRef = useRef(false);

  /**
   * 聚焦搜索框并选中已有内容，便于直接覆盖输入。
   */
  const focusSearch = useCallback(async () => {
    // 外部传入 ref 时 antd Input 挂在外部 ref 上，从这里取真实 input 实例。
    const externalRef = inputRef ?? null;
    const input =
      externalRef && typeof externalRef !== "function"
        ? externalRef.current
        : localInputRef.current;
    if (!input) return;

    await prepareClipboardWindowEditableFocus();
    input?.focus({ cursor: "all" });
  }, [inputRef]);

  useEffect(() => {
    if (blurToken <= 0) return;

    if (typeof activeInputRef === "function") {
      return;
    }

    activeInputRef?.current?.blur();
  }, [blurToken, activeInputRef]);

  useEffect(() => {
    if (focusToken <= 0) return;

    const frame = requestAnimationFrame(() => {
      void focusSearch();
    });

    return () => {
      cancelAnimationFrame(frame);
    };
  }, [focusToken, focusSearch]);

  const handleChange = (event: ChangeEvent<HTMLInputElement>) => {
    if (composingRef.current) return;

    onChange?.(event);
  };

  const handleCompositionStart = (
    event: CompositionEvent<HTMLInputElement>,
  ) => {
    composingRef.current = true;

    onCompositionStart?.(event);
  };

  const handleCompositionEnd = (event: CompositionEvent<HTMLInputElement>) => {
    composingRef.current = false;

    onCompositionEnd?.(event);
    // composition 结束时浏览器已派发最后一次 input，但被上面挡掉了，这里补一次。
    onChange?.(event as unknown as ChangeEvent<HTMLInputElement>);
  };

  return (
    <Input
      autoCapitalize="off"
      autoCorrect="off"
      data-allow-global-keyboard="true"
      key={clearToken}
      onChange={handleChange}
      onCompositionEnd={handleCompositionEnd}
      onCompositionStart={handleCompositionStart}
      prefix={
        <KeyHint
          hintKey="F"
          iconName="i-lucide:search"
          onKeyPress={focusSearch}
        />
      }
      ref={activeInputRef}
      spellCheck={false}
      {...rest}
    />
  );
};

export default SearchInput;

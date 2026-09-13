import type { FC } from "react";
import { useTranslation } from "react-i18next";
import { useSnapshot } from "valtio";
import { clipboardViewState } from "@/stores/clipboardView";
import { cn } from "@/utils/cn";

/** 短词提示的会话级一次性开关：提示过一次就不再打扰。 */
let shortKeywordHintShown = false;

/** Rust FTS trigram 分词的最短可索引长度；短于它的关键词降级 LIKE 全表扫描。 */
const FTS_MIN_TOKEN_CHARS = 3;

interface SearchShortKeywordHintProps {
  className?: string;
}

/**
 * 短词搜索提示：任一关键词分词 <3 字符（含 CJK 双字词）时展示一次轻提示，
 * 告知「至少 3 字符才能走索引快速搜索」。LIKE 兜底照常执行，不阻断搜索；
 * 同一会话只提示一次，避免高频打扰。
 */
const SearchShortKeywordHint: FC<SearchShortKeywordHintProps> = (props) => {
  const { t } = useTranslation("clipboard");
  const { className } = props;
  const { keyword } = useSnapshot(clipboardViewState);

  if (shortKeywordHintShown) return null;
  if (!hasShortToken(keyword)) return null;
  shortKeywordHintShown = true;

  return (
    <div
      className={cn(
        "pointer-events-none mx-auto w-fit rounded-1 border border-ant-border bg-ant-container px-1.5 py-0.5 text-ant-secondary text-xs shadow-md",
        className,
      )}
      data-allow-global-keyboard="true"
    >
      {t("header.shortKeywordHint")}
    </div>
  );
};

/**
 * 判定关键词是否存在 <3 字符的分词。与 Rust `KeywordFilter::from_keyword` 的
 * 分流口径一致：`split_whitespace` 后任一 token 字符数 <3 即命中。
 */
function hasShortToken(keyword: string) {
  if (keyword.length === 0) return false;

  return keyword
    .split(" ")
    .some((token) => token.length > 0 && token.length < FTS_MIN_TOKEN_CHARS);
}

export default SearchShortKeywordHint;

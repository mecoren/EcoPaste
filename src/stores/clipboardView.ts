import { proxy } from "valtio";
import type { ClipboardCategory, ClipboardRange } from "@/types/clipboard";

interface ClipboardViewState {
  category: ClipboardCategory | null;
  keyword: string;
  /** 自定义分组筛选（分组 id）；null 表示不筛。 */
  groupId: string | null;
  range: ClipboardRange;
  /**
   * 搜索框清空重挂载 token：自增触发 SearchInput 以 `key` 重新挂载，输入文本随之清空。
   * 不放 keyword 之外的输入态——输入框本体保持非受控。
   */
  searchClearToken: number;
}

/**
 * 剪贴板窗口的 UI 临时状态（非持久化）。
 * 跨组件共享：Header 搜索框写入 `keyword`，Group 写入范围/分类/分组，List 监听后驱动查询。
 * 注意：这里的字段会被 List 用 `...rest` 透传成查询参数，**不要**塞进与 `ClipboardItemQuery` 同名
 * 但语义不同的字段（例如「窗口是否固定」要另起 store，否则会被当成 `pinned`(条目置顶) 过滤）。
 * `limit` / `offset` 不在这里——分页由 `useClipboardItems` 内部 `useInfiniteScroll` 管理。
 */
export const clipboardViewState = proxy<ClipboardViewState>({
  category: null,
  groupId: null,
  keyword: "",
  range: "all",
  searchClearToken: 0,
});

const KEYWORD_DEBOUNCE_MS = 200;

let keywordDebounceTimer = 0;

/**
 * 防抖写入搜索关键词（连续打字只保留最后一次）。
 * 防抖收口在 store 层，保证 `clearClipboardSearch` 能取消同一份 pending 写入，
 * 避免「清空后关键词在 200ms 内复活」。
 */
export const setClipboardSearchKeyword = (value: string) => {
  const next = value.trim();

  if (keywordDebounceTimer !== 0) {
    window.clearTimeout(keywordDebounceTimer);
  }

  keywordDebounceTimer = window.setTimeout(() => {
    keywordDebounceTimer = 0;
    clipboardViewState.keyword = next;
  }, KEYWORD_DEBOUNCE_MS);
};

/**
 * 立即清空搜索：取消防抖中的 pending 写入、重置关键词、递增 token 让搜索框重挂载清文本。
 */
export const clearClipboardSearch = () => {
  if (keywordDebounceTimer !== 0) {
    window.clearTimeout(keywordDebounceTimer);
    keywordDebounceTimer = 0;
  }

  clipboardViewState.keyword = "";
  clipboardViewState.searchClearToken += 1;
};

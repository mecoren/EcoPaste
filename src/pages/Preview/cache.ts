import type { ClipboardPreviewPayload } from "@/commands";
import { PREVIEW_CACHE_LIMIT, PREVIEW_CACHE_MAX_BYTES } from "./constants";

/**
 * 估算单个 payload 的常驻占用：文本按 UTF-16 近似 2 字节/码元，文件按路径长度累计。
 * 只求量级正确——用于驱逐决策，不用于精确统计。
 */
export function estimatePayloadBytes(payload: ClipboardPreviewPayload) {
  const textBytes = (payload.text?.length ?? 0) * 2;
  const filesBytes = payload.files.reduce(
    (sum, file) => sum + file.path.length * 2,
    0,
  );

  return textBytes + filesBytes;
}

/**
 * 从 LRU cache 读取同 item 的最新 payload，命中后刷新插入顺序。
 */
export function readCachedPayload(
  cache: Map<string, ClipboardPreviewPayload>,
  itemId: string,
  redactSecrets: boolean,
) {
  const keyPrefix = `${itemId}:`;
  const keySuffix = `:${redactSecrets ? "redacted" : "full"}`;
  const key = [...cache.keys()].find((entryKey) => {
    return entryKey.startsWith(keyPrefix) && entryKey.endsWith(keySuffix);
  });

  if (!key) return null;

  const cached = cache.get(key) ?? null;
  if (cached) {
    cache.delete(key);
    cache.set(key, cached);
  }

  return cached;
}

/**
 * 写入最近预览 payload，key 绑定 updatedAt 避免内容复用过期。
 * 双重预算：条目数（PREVIEW_CACHE_LIMIT）+ 字节（PREVIEW_CACHE_MAX_BYTES），
 * 任一超限都从最旧端驱逐——大文本条目不再按条目数常驻内存。
 */
export function writeCachedPayload(
  cache: Map<string, ClipboardPreviewPayload>,
  nextPayload: ClipboardPreviewPayload,
  redactSecrets: boolean,
) {
  const key = cacheKey(nextPayload, redactSecrets);
  cache.delete(key);
  cache.set(key, nextPayload);

  let totalBytes = 0;
  for (const payload of cache.values()) {
    totalBytes += estimatePayloadBytes(payload);
  }

  while (cache.size > 0) {
    const overCount = cache.size > PREVIEW_CACHE_LIMIT;
    const overBytes = totalBytes > PREVIEW_CACHE_MAX_BYTES;
    if (!overCount && !overBytes) return;

    const [oldestKey, oldestPayload] = [...cache.entries()][0] ?? [];
    if (!oldestKey || !oldestPayload) return;

    totalBytes -= estimatePayloadBytes(oldestPayload);
    cache.delete(oldestKey);
  }
}

/**
 * 生成预览 payload 的缓存 key。
 */
export function cacheKey(
  payload: ClipboardPreviewPayload,
  redactSecrets = payload.isSensitive,
) {
  return `${payload.id}:${payload.updatedAt}:${redactSecrets ? "redacted" : "full"}`;
}

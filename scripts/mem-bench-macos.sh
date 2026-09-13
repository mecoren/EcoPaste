#!/usr/bin/env bash
# EcoPaste macOS 内存基准脚本（开发工具，非产品功能）。
# 场景：启动 -> 基线 -> 批量写入混合内容 -> 峰值 -> 隐藏静置 -> 稳态，产出 CSV。
# 用法：在 EcoPaste 运行中执行：
#   ./scripts/mem-bench-macos.sh [rounds] [idle_minutes]
# 产出：mem-bench-<timestamp>.csv（stage,time,rss,virtual）
# 依赖：pbcopy（macOS 自带）；RSS 经 `ps` 读取。

set -euo pipefail

PROCESS_NAME="${ECOPASTE_PROCESS_NAME:-EcoPaste}"
ROUNDS="${1:-200}"
IDLE_MINUTES="${2:-5}"
CSV_PATH="mem-bench-$(date +%Y%m%d-%H%M%S).csv"

if [ "$ROUNDS" -lt 1 ] || [ "$ROUNDS" -gt 5000 ]; then
  echo "rounds 必须在 1..5000 之间" >&2
  exit 1
fi

get_mem_row() {
  local stage="$1"
  # rss=物理驻留(KB*1024) vsz=虚拟内存(KB*1024)
  local stats
  stats=$(ps -axco rss,vsz,command | awk -v name="$PROCESS_NAME" '$0 ~ name {print $1, $2; exit}')
  if [ -z "$stats" ]; then
    echo "找不到进程 $PROCESS_NAME（可用 ECOPASTE_PROCESS_NAME 指定）" >&2
    exit 1
  fi
  local rss vsz
  rss=$(echo "$stats" | awk '{print $1 * 1024}')
  vsz=$(echo "$stats" | awk '{print $2 * 1024}')
  echo "$stage,$(date +%s),$(printf '%.0f' "$rss"),$(printf '%.0f' "$vsz")" >>"$CSV_PATH"
  printf "%-12s RSS=%-12s VM=%s\n" "$stage" "$rss" "$vsz"
}

echo "stage,time,rss,virtual" >"$CSV_PATH"
echo "内存基准：进程=$PROCESS_NAME 轮次=$ROUNDS 静置=$IDLE_MINUTES 分钟"
echo "CSV 产出：$CSV_PATH"

# --- 基线 ---
sleep 10
get_mem_row "baseline"

# --- 批量复制：短文本、长文本、URL、JSON 混合 ---
for ((i = 0; i < ROUNDS; i++)); do
  case $((i % 4)) in
    0) printf '短文本基准样本 中文' | pbcopy ;;
    1) { printf '长文本样本 '; for _ in $(seq 1 256); do printf '0123456789abcdef'; done; } | pbcopy ;;
    2) printf 'https://github.com/EcoPasteHub/EcoPaste' | pbcopy ;;
    3) printf '{"kind":"bench","round":1,"tags":["memory","baseline"]}' | pbcopy ;;
  esac
  # 给 watcher 的去抖与入库留出间隔，模拟真实复制节奏。
  sleep 0.2

  if [ $(((i + 1) % 50)) -eq 0 ]; then
    get_mem_row "copy-$((i + 1))"
  fi
done

get_mem_row "peak"

# --- 隐藏静置 ---
echo "静置 $IDLE_MINUTES 分钟后取稳态……"
sleep $((IDLE_MINUTES * 60))
get_mem_row "steady"

echo "完成。基准 CSV 已写入 $CSV_PATH（可与优化前后版本对比 rss 列）"

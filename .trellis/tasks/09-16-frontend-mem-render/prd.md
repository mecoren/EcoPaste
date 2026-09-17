# 前端内存与渲染优化

## Goal

修复前端内存硬伤：每窗全量加载 7 页、i18n 全量打包、memo 失效导致全列表重渲染、批量操作 O(n) 扫描。

## Requirements

- 路由代码分割：全 7 页 `React.lazy(() => import(...))` + 根部 `<Suspense>`；每窗只载自己 chunk + shared chunk。
- i18n 按语言：静态 import zh-CN（默认+fallback），en-US 改 `import.meta.glob` 动态加载，changeLanguage 前注入 resourceBundle。
- memo 失效链修复：`visibleQuickActions`/`availableActions` 每 render 新数组击穿 ClipboardCard memo → 改 per-item Map 缓存；FilesCard/ImageCard/Highlight/NoteContentSwitcher/ClipboardQuickActions 补 `memo`；`buildItemActionLabels(t)`/`countLeadingPinnedItems` 结果缓存。
- `useClipboardItems` 索引：`findItemById`/`getItemIndexById` O(n) → `Map<id, index>` 增量索引。
- AssetImage 加 `loading="lazy" decoding="async"`。

## Acceptance Criteria

- [ ] 构建产物每窗 chunk 独立；对比分割前后 chunk 尺寸下降（Onboarding/Preference/Update 不进主列表窗）。
- [ ] 默认语言首屏无 en-US 包；切换英文后文案完整。
- [ ] 滚动/hover 时非可视卡片不重渲染（profiler 抽查）。
- [ ] 批量操作大列表无可感知卡顿；lint + tsc + 构建全绿。

## Constraints

- 不改视觉与交互语义；只做加载与渲染优化。
- 语言切换低频，允许毫秒级磁盘加载。

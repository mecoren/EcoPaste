# Implement: 合并粘贴与粘贴文本清理

## Checklist（按序）

- [ ] S1 Rust：从 `paste_clipboard_item` 抽出 hide/50ms/pinned 恢复共享函数。
- [ ] S2 Rust：`PasteTransform` enum + 5 种纯字符串变换单测。
- [ ] S3 Rust：`paste_clipboard_item` 加 `transform` 参数；transform 隐含纯文本 + 抑制变换后哈希。
- [ ] S4 Rust：新命令 `paste_clipboard_items(ids, separator, plain)`：排序取条目→拼接→纯文本写回→suppress 合成哈希→逐条 use_count。
- [ ] S5 Rust：设置 `merge_paste_separator` + serde default + validation；注册命令。
- [ ] S6 前端：constants/commands 包装；BatchActionToolbar 合并按钮（非文本禁用+tooltip）；Enter 多选分发（≥2全文本合并/含非文本toast/单选不变）；选中按显示序排序。
- [ ] S7 前端：QuickActions「清理粘贴」Dropdown（5项）；`Ctrl+Shift+Enter` 去换行；ShortcutList 同步；偏好页分隔符设置 + 双语 i18n（zh-CN/en-US）。
- [ ] S8 全量校验：`cargo fmt && cargo clippy -- -D warnings && cargo test`；`pnpm lint` + `pnpm tsc` + 构建；Windows 真机点验。

## Validation Commands

```bash
cd src-tauri
cargo fmt
cargo clippy -- -D warnings
cargo test
```

```bash
pnpm lint
pnpm tsc
```

## Review Gates

- 跨层：命令名/参数/事件名字面量 Rust ↔ `src/constants/` 一致；payload 无多余字段。
- 回归：单选 Enter、Esc 清多选、普通复制/粘贴、固定/非固定窗口粘贴。
- 设置兼容：删掉新字段的 settings.json 启动不报错且回落默认。

## Rollback Points

- S1–S5 后任一步失败：revert Rust 侧新增命令/参数，旧命令路径不受影响。
- S6–S7 失败：前端按钮/快捷键开关回退，不动 Rust。

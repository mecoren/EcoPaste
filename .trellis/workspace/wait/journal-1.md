# Journal - wait (Part 1)

> AI development session journal
> Started: 2026-09-11

---

## Session 1: Ditto-Style Type-to-Search in Clipboard Window

**Date**: 2026-09-11
**Task**: 09-11-search-ready-type-to-filter
**Branch**: `wait`

### Summary

Made the clipboard window search box permanently ready: typing any printable character at any moment (including mid arrow-key navigation) now lands in the search box and filters the list, Ditto-style, on both Windows and macOS.

### Main Changes

- Rust `keyboard/windows.rs`: hook swallows unmodified printable keydowns, queues them, emits `clipboard://search-typing`; single-flight replay worker waits for frontend ack + clipboard-window foreground, then re-injects keys via injected `SendInput` (LLKHF_INJECTED passes through the hook); added `typeahead_key` unit tests.
- Rust `window/windows.rs`: `set_clipboard_window_editing(true)` no longer disables nav hooks synchronously — a focus watcher thread disables them only after the clipboard window actually becomes foreground (closes the key-leak window); rollback on focus timeout.
- Rust `commands/clipboard.rs`: `paste_clipboard_item` exits editing on Windows before simulate so Ctrl+V reaches the user's app.
- Rust `settings/model.rs`: `search.default_focus` platform default — true on macOS, false on Windows (existing configs unaffected via serde defaults).
- New command `search_typing_ack` (Rust + TS mirrors), new event `SEARCH_TYPING`, shared `findEditableElement` in `src/utils/dom.ts`.
- `useClipboardWindowEditableFocus`: removed focusout restore (handoff blur keeps editing; restore only on window blur / hide).
- `clipboardView` store: debounce moved into store (`setClipboardSearchKeyword`), `clearClipboardSearch` cancels pending write + bumps `searchClearToken`.
- New `useSearchTypeahead` hook (real-keydown steer + Rust event path); Header wires it; List Escape layering now clears keyword after preview.
- Shortcut list + zh/en locales: new "type anywhere to search" hint.

### Testing

- [OK] `cargo clippy -- -D warnings`, `cargo fmt`, `cargo test` (180 passed)
- [OK] `pnpm tsc`, `pnpm lint`
- [P] Live GUI verification blocked: machine was on the lock screen during the session (shortcuts can't fire). Code review pass done instead; manual checklist in task `implement.md` S11 awaits an unlocked session.

### Status

# **In Progress** — code complete, commit pending user decision (branch policy: current branch `wait` vs new branch).

### Next Steps

- User unlocks machine → run `pnpm tauri dev` and walk S11 manual checklist (type-to-search ASCII + IME, mid-nav typing, Enter paste pinned/unpinned, Escape layering, Ctrl shortcuts, dialogs, click-away).
- Decide commit branch, commit, archive task.


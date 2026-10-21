# Database and Storage

## SQLite Access

The app owns one active SQLite pool through `DatabaseState`. Commands call
`db.pool().await` to clone the current pool. This indirection matters because
backup overwrite import and storage directory switching can close and replace
the active pool atomically.

Reference files:
- `src-tauri/src/db/init.rs`
- `src-tauri/src/db/state.rs`
- `src-tauri/src/commands/storage.rs`
- `src-tauri/src/backup/mod.rs`

Do not create a fresh pool in normal commands. Repository functions accept
`&SqlitePool` and return `crate::core::Result<T>`.

## Migrations and Models

Migrations live in `src-tauri/migrations/`. `0001_init.sql` defines
`clipboard_groups`, `clipboard_apps`, `clipboard_items`, and `file_type_icons`.
`0002_fts.sql` defines the FTS5 table and triggers. `0006_clipboard_list_sort_indexes.sql`
replaces the 0002 sort indexes with all-DESC composite ones (see
[List Sort Indexes](#list-sort-indexes)).

The app has shipped a stable release line, so schema changes must be expressed
as new migrations with an explicit upgrade path. When changing schema, update all
of these together:

- SQL migrations.
- `src-tauri/src/db/models.rs`.
- Repository `SELECT`, `INSERT`, `UPDATE`, `bind`, and `query_as` calls.
- TypeScript mirrors in `src/types/`.
- Tests that construct model literals.

`sqlx::query` and `query_as` are the local style. Avoid `query!` macros because
this project does not maintain offline sqlx metadata.

### Shipped Migrations Are Immutable

sqlx stores `Sha384(file contents)` per migration in `_sqlx_migrations.checksum`.
Editing an already-shipped migration file by even one character makes
`validate_applied_migrations` return `VersionMismatch` on every upgraded install;
`sqlx::migrate!` then fails, `db::init` propagates the error, and `lib.rs`'s
`.expect("error while running tauri application")` aborts the process. The user
sees the app simply not starting — there is no readable error surface.

Two consequences follow from the same mechanism:

- **Downgrading is not a recovery path.** An older binary has no entry for that
  version, so it fails with `VersionMissing` instead. Once shipped, users cannot
  start the previous release either.
- **Deleting a shipped migration file is equivalent** to the above.

To correct a shipped migration, add a **new higher-numbered migration** (e.g.
`0007_*.sql`) that supersedes it. Never edit `0006_*` in place.

Migrations run blocking inside `db::init` before the clipboard watcher starts, so
their duration is dead time at launch and a failure is a launch failure. Measure
large-table DDL first: this repository measured ~107 ms at 60 MB, ~1 s at 450 MB,
and ~1.9 s at 880 MB for `0006`.

Before committing a migration, actually start the app once and confirm the row in
`_sqlx_migrations` has `success = 1`. Running the SQL by hand with `sqlite3` does
not validate the real path: `sqlx::migrate!` is a compile-time macro, so filename
parsing, transaction wrapping, and checksum recording are all its responsibility.

## Timestamp Semantics

All tables have `created_at` and `updated_at` as `TEXT NOT NULL`.

For `clipboard_items`, `updated_at` means the content was reused or deduped.
Do not update it for metadata-only edits:

- `toggle_item_favorite` and `toggle_item_pinned` return state without changing
  `updated_at`.
- `update_item_note` and `update_item_group` preserve recent-use ordering.
- `increment_item_use_count` is the path that refreshes `updated_at`.

This distinction feeds the default `updatedAtDesc` sort used by the clipboard
list.

## Search and Pagination

List search is owned by `db::items::query_items_page`:

- Keywords with 3 or more characters use FTS5 trigram search.
- Keywords with 1 or 2 characters use escaped `LIKE` across `search_text` and
  `note`.
- UI tab filters are represented by `ClipboardGroupFilter` and override explicit
  `kind`/`favorite` fields.
- Pinned items are always sorted before the chosen sort order.
- The query returns `(items, total)`, and commands return `hasMore` as
  `offset + list.len() < total`.

React should not approximate `hasMore` with page size or reimplement short-keyword
search.

## List Sort Indexes

The list always sorts `is_pinned DESC, <sort> DESC` (see
`db::items::sort_order_clause`), so its indexes must be **all-DESC and cover the
secondary sort key** (`created_at`):

- `idx_clipboard_items_pinned_updated_created` — `updatedAtDesc` (default).
- `idx_clipboard_items_pinned_created` — `createdAtDesc`, and also the retention /
  `max_count` cleanup scan.
- `idx_clipboard_items_pinned_use_created` — `useCountDesc`.

Two counter-intuitive rules follow from this:

- **Mixed-direction indexes do not satisfy an all-DESC ORDER BY.** Forward scan
  yields `is_pinned ASC`, backward scan yields the sort column `ASC`; SQLite gives
  up and falls back to `SCAN` + `USE TEMP B-TREE FOR ORDER BY`. Without a covering
  index the sort must read every row, which is what made opening the clipboard
  window get slower as history grew.
- **Omitting the secondary sort key silently degrades** to
  `USE TEMP B-TREE FOR LAST TERM OF ORDER BY`, which measured *slower* than the
  full-scan path for `createdAtDesc`.

Changing the sort keys therefore requires a new migration that keeps index column
order and direction in sync with `sort_order_clause`. The regression test
`db::items::tests::list_query_uses_sort_index_without_temp_btree` asserts each sort
mode names its expected index and produces no `TEMP B-TREE`; it fails as intended
when the indexes are missing.

## Repository Tests

Use `db::test_support::memory_pool()` for repository tests. It creates a
single-connection in-memory SQLite pool, enables foreign keys, and runs the real
migrations. This catches contracts such as `clipboard_groups` deletion setting
`clipboard_items.group_id` to `NULL`.

Examples:
- `src-tauri/src/db/items.rs`
- `src-tauri/src/db/groups.rs`
- `src-tauri/src/db/apps.rs`

## Resource Paths

`core::paths` is the single entry for persistent roots:

- `app_data_dir` resolves the environment-specific data root.
- `db_dir`, `resources_dir`, `config_dir`, and `state_dir` derive semantic
  subdirectories.
- Dev and release data are separated by `dev/` and `prod/`.
- A bootstrap `storage.json` can point at a custom data root with an
  `.ecopaste-storage.json` identity file.

Do not resolve `app_local_data_dir` independently in feature modules. Leaf file
names remain owned by the modules that write them, such as `settings.json`,
`clipboard.db`, and image/icon stores.

## Storage Switching and Backups

`commands/storage.rs` can move the data root while the app is running. It pauses
the clipboard watcher, copies known content directories, replaces the database
pool through `DatabaseState::close_and_replace`, rebases settings and resource
stores, emits `settings://updated`, and emits a clipboard import refresh.

`backup/mod.rs` exports `.ecopastebak` packages in two modes:

- Plain mode: ZIP payload.
- Encrypted mode: EcoPaste container with Argon2id and XChaCha20Poly1305.

The backup receive flow uses a pending slot when the preference window has been
destroyed, mirroring the preference-highlight pending-slot pattern in
`window/mod.rs`. Do not emit directly to a possibly rebuilding preference
window.

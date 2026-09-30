//! 设置持久化。
//!
//! - 落盘位置：`<app_data_dir>/config/settings.json`（dev/prod 由 `core::paths` 的环境子目录隔离）。
//! - 写入流程：先写到 `settings.json.tmp`，再原子替换主文件，避免中途断电留下半截 JSON。
//! - 缺字段兼容：`Settings` 各结构体都 `#[serde(default)]`，新版本新增字段不影响旧文件。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

use anyhow::Context;
use tauri::AppHandle;

use crate::core::{AppError, Result};

use super::model::{
    Language, Settings, SETTINGS_VERSION, WINDOW_OPEN_GROUP_PREFIX, WINDOW_OPEN_SELECTION_ALL,
    WINDOW_OPEN_SELECTION_PRESERVE,
};

const FILENAME: &str = "settings.json";

pub struct SettingsStore {
    path: RwLock<PathBuf>,
    current: RwLock<Settings>,
    /// 设置变更代次：`update` / `reset` / `replace_from_file` / `rebase` 时自增。
    /// 周期任务（如历史清理 tick）用它做「版本未变 → 跳过 snapshot 深拷」的廉价比对，
    /// 避免每次 tick 都克隆整个 Settings 结构。
    version: AtomicU64,
}

impl SettingsStore {
    pub fn new(app: &AppHandle) -> Result<Self> {
        let dir = crate::core::paths::config_dir(app)?;
        fs::create_dir_all(&dir).with_context(|| format!("failed to create dir at {dir:?}"))?;

        let path = dir.join(FILENAME);

        let current = match load_from_disk(&path) {
            Some(settings) => settings,
            None => {
                // 真·首次启动：用系统 locale 推导默认语言并落盘，之后所有读取都走常规分支。
                let settings = default_settings_with_system_locale();
                if let Err(err) = write_atomic(&path, &settings) {
                    log::warn!("persist first-run settings failed: {err}");
                }
                settings
            }
        };
        log::info!("settings store ready at {path:?}");

        Ok(Self {
            path: RwLock::new(path),
            current: RwLock::new(current),
            version: AtomicU64::new(1),
        })
    }

    /// 当前设置代次。后台周期任务比对「上次看到的代次」判断设置是否变过，
    /// 未变时不必 `snapshot()` 深拷整个 Settings。
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    pub fn snapshot(&self) -> Settings {
        self.current.read().expect("settings poisoned").clone()
    }

    /// 恢复默认设置并落盘，返回新的完整快照。
    pub fn reset(&self) -> Result<Settings> {
        let next = default_settings_with_system_locale();

        let path = self.path();
        write_atomic(&path, &next)?;
        *self.current.write().expect("settings poisoned") = next.clone();
        self.version.fetch_add(1, Ordering::Release);
        Ok(next)
    }

    /// 用 JSON patch 深度合并到当前设置，落盘后返回新快照。
    /// patch 必须是 object；非 object 视为「整个替换」语义不友好，直接报错。
    pub fn update(&self, patch: serde_json::Value) -> Result<Settings> {
        if !patch.is_object() {
            return Err(AppError::Other(anyhow::anyhow!(
                "settings patch must be a JSON object"
            )));
        }

        let mut guard = self.current.write().expect("settings poisoned");

        let mut merged = serde_json::to_value(&*guard)
            .context("failed to serialize current settings for merge")?;
        deep_merge(&mut merged, patch);

        // 内存里的设置恒为已迁移到当前版本的状态；patch 不允许回退文件版本号，
        // 否则下一次加载会重复执行迁移、覆盖用户在 v1 之后改过的值。
        let mut next: Settings = serde_json::from_value(merged)
            .map_err(|err| AppError::Other(anyhow::anyhow!("invalid settings patch: {err}")))?;
        next.settings_version = SETTINGS_VERSION;

        validate_settings(&next)?;

        let path = self.path();
        write_atomic(&path, &next)?;
        *guard = next.clone();
        self.version.fetch_add(1, Ordering::Release);
        Ok(next)
    }

    /// 用完整设置文件替换当前设置；覆盖导入专用。旧版本文件先迁移再落盘。
    pub fn replace_from_file(&self, path: &Path) -> Result<Settings> {
        let content =
            fs::read_to_string(path).with_context(|| format!("failed to read {path:?}"))?;
        let mut next: Settings = serde_json::from_str(&content)
            .map_err(|err| AppError::Other(anyhow::anyhow!("invalid settings file: {err}")))?;
        migrate_settings(&mut next);

        validate_settings(&next)?;

        let path = self.path();
        write_atomic(&path, &next)?;
        *self.current.write().expect("settings poisoned") = next.clone();
        self.version.fetch_add(1, Ordering::Release);
        Ok(next)
    }

    /// 数据目录热切换后重新绑定设置文件，并把新路径里的设置加载进内存。
    pub fn rebase(&self, app: &AppHandle) -> Result<Settings> {
        let dir = crate::core::paths::config_dir(app)?;
        fs::create_dir_all(&dir).with_context(|| format!("failed to create dir at {dir:?}"))?;
        let path = dir.join(FILENAME);
        let current = match load_from_disk(&path) {
            Some(settings) => settings,
            None => {
                let settings = default_settings_with_system_locale();
                write_atomic(&path, &settings)?;
                settings
            }
        };

        *self.path.write().expect("settings path poisoned") = path;
        *self.current.write().expect("settings poisoned") = current.clone();
        self.version.fetch_add(1, Ordering::Release);
        Ok(current)
    }

    fn path(&self) -> PathBuf {
        self.path.read().expect("settings path poisoned").clone()
    }
}

/// 生成默认设置，并沿用首次启动的系统语言推导规则。
fn default_settings_with_system_locale() -> Settings {
    let mut settings = Settings::default();
    if let Some(tag) = tauri_plugin_os::locale() {
        settings.appearance.language = Language::from_system_locale(&tag);
        log::info!(
            "default settings language from locale {tag}: {:?}",
            settings.appearance.language
        );
    }
    settings
}

/// 返回 `None` 表示主文件不存在（首次启动），调用方据此走「初始化默认」分支；
/// 读取过程中遇到 IO/解析错误会打 warn，并返回 `Settings::default()` 包装在 `Some` 里——
/// 这条路径表示「文件存在但坏了」，不要当成首次启动覆盖系统 locale。
/// 解析成功后先跑版本迁移，有变更立即回写，保证迁移一次性完成不随进程反复触发。
fn load_from_disk(path: &Path) -> Option<Settings> {
    if !path.exists() {
        return None;
    }

    let mut settings = match fs::read_to_string(path).and_then(|content| {
        serde_json::from_str::<Settings>(&content)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))
    }) {
        Ok(settings) => settings,
        Err(err) => {
            log::warn!("settings file {path:?} unreadable, using defaults: {err}");
            return Some(Settings::default());
        }
    };

    if migrate_settings(&mut settings) {
        if let Err(err) = write_atomic(path, &settings) {
            log::warn!("persist migrated settings failed: {err}");
        }
    }

    Some(settings)
}

/// 把旧版本设置文件迁移到当前版本，返回是否发生变更（调用方需回写磁盘）。
///
/// v0 → v1：`updateOnReuse` 旧默认是 `false`，旧文件把旧默认值显式落了盘，
/// 与用户主动关闭无法区分，故一次性统一翻正为 `true`（粘贴 / 复用后条目
/// 按 `updated_at` 顶到置顶块之后的第一位）。v1 起用户再关闭不会被迁移覆盖。
fn migrate_settings(settings: &mut Settings) -> bool {
    if settings.settings_version >= SETTINGS_VERSION {
        return false;
    }

    settings.clipboard.content.update_on_reuse = true;
    settings.settings_version = SETTINGS_VERSION;
    true
}

/// 写入策略：把新内容写到 tmp 后 rename 成主文件；rename 在同一文件系统下是原子的。
fn write_atomic(path: &Path, settings: &Settings) -> Result<()> {
    let json = serde_json::to_string_pretty(settings).context("failed to serialize settings")?;

    let tmp = path.with_extension("json.tmp");
    {
        let mut file = fs::File::create(&tmp)
            .with_context(|| format!("failed to create tmp settings at {tmp:?}"))?;
        file.write_all(json.as_bytes())
            .with_context(|| format!("failed to write tmp settings at {tmp:?}"))?;
        file.sync_all().ok();
    }
    fs::rename(&tmp, path)
        .with_context(|| format!("failed to promote tmp settings to {path:?}"))?;
    Ok(())
}

/// 校验设置之间的跨字段约束，避免非法配置写入磁盘。
fn validate_settings(settings: &Settings) -> Result<()> {
    validate_window_open_group(&settings.clipboard.window.select_group_on_open)?;

    let open_clipboard = normalize_shortcut_value(&settings.shortcuts.open_clipboard);
    let open_preference = normalize_shortcut_value(&settings.shortcuts.open_preference);

    if open_clipboard.is_empty() || open_preference.is_empty() {
        return Ok(());
    }

    if open_clipboard == open_preference {
        return Err(AppError::Other(anyhow::anyhow!(
            "global shortcuts must be unique"
        )));
    }

    Ok(())
}

/// 校验打开剪贴板窗口时选中分组的字符串编码，避免非法设置值落盘。
fn validate_window_open_group(value: &str) -> Result<()> {
    if value == WINDOW_OPEN_SELECTION_PRESERVE || value == WINDOW_OPEN_SELECTION_ALL {
        return Ok(());
    }

    let Some(group_id) = value.strip_prefix(WINDOW_OPEN_GROUP_PREFIX) else {
        return Err(AppError::Other(anyhow::anyhow!(
            "open group selection is invalid"
        )));
    };

    if group_id.trim().is_empty() {
        return Err(AppError::Other(anyhow::anyhow!(
            "open group selection is invalid"
        )));
    }

    Ok(())
}

/// 归一化快捷键字面量，供跨字段校验忽略大小写和多余空白。
fn normalize_shortcut_value(value: &str) -> String {
    value
        .split('+')
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join("+")
}

fn deep_merge(base: &mut serde_json::Value, patch: serde_json::Value) {
    match (base, patch) {
        (serde_json::Value::Object(base_map), serde_json::Value::Object(patch_map)) => {
            for (k, v) in patch_map {
                match base_map.get_mut(&k) {
                    Some(existing) if existing.is_object() && v.is_object() => {
                        deep_merge(existing, v);
                    }
                    _ => {
                        base_map.insert(k, v);
                    }
                }
            }
        }
        (slot, patch) => {
            *slot = patch;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_merge_overrides_leaves_and_recurses_objects() {
        let mut base = serde_json::json!({
            "general": {"autoStart": false, "trayIcon": true},
            "clipboard": {"history": {"maxCount": 0}},
        });
        let patch = serde_json::json!({
            "general": {"autoStart": true},
            "clipboard": {"history": {"maxCount": 500}},
        });
        deep_merge(&mut base, patch);
        assert_eq!(
            base,
            serde_json::json!({
                "general": {"autoStart": true, "trayIcon": true},
                "clipboard": {"history": {"maxCount": 500}},
            })
        );
    }

    #[test]
    fn deep_merge_replaces_arrays_wholesale() {
        let mut base = serde_json::json!({"itemActions": ["copy", "star", "delete"]});
        let patch = serde_json::json!({"itemActions": ["copy", "pastePlain"]});
        deep_merge(&mut base, patch);
        assert_eq!(
            base,
            serde_json::json!({"itemActions": ["copy", "pastePlain"]})
        );
    }

    #[test]
    fn validate_settings_rejects_duplicate_global_shortcuts() {
        let mut settings = Settings::default();
        settings.shortcuts.open_preference = settings.shortcuts.open_clipboard.clone();

        assert!(validate_settings(&settings).is_err());
    }

    #[test]
    fn validate_settings_allows_empty_global_shortcuts() {
        let mut settings = Settings::default();
        settings.shortcuts.open_preference = String::new();

        assert!(validate_settings(&settings).is_ok());
    }

    /// 直接构造带版本计数与临时文件的 store，供版本号语义测试复用。
    fn store_for_version_test() -> (tempfile::TempDir, SettingsStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore {
            path: RwLock::new(dir.path().join(FILENAME)),
            current: RwLock::new(Settings::default()),
            version: AtomicU64::new(1),
        };
        (dir, store)
    }

    #[test]
    fn version_increments_on_each_mutation() {
        let (_dir, store) = store_for_version_test();
        let before = store.version();

        // 同一 patch 再写：内容不变也自增（代次只关心「发生过写」，不 diff 内容），
        // 保证消费方（cleanup tick）宁可多取一次快照也不漏变更。
        store
            .update(serde_json::json!({"general": {"trayIcon": true}}))
            .unwrap();
        let after_update = store.version();
        assert_eq!(after_update, before + 1);

        store.reset().unwrap();
        assert_eq!(store.version(), after_update + 1);
    }

    #[test]
    fn version_stable_when_no_mutation() {
        let (_dir, store) = store_for_version_test();
        let _ = store.snapshot();

        // 只读 snapshot 不改变代次——tick 比对依赖这一点。
        assert_eq!(store.version(), store.version());
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let partial = r#"{"general": {"autoStart": true}}"#;
        let parsed: Settings = serde_json::from_str(partial).unwrap();
        assert!(parsed.general.auto_start);
        assert!(!parsed.general.run_as_admin);
        assert!(parsed.general.tray_icon, "default kept");
        assert_eq!(parsed.shortcuts.open_clipboard, "Alt+C");
        assert_eq!(
            parsed.update.frequency,
            crate::settings::UpdateFrequency::Daily
        );
        assert_eq!(
            parsed.clipboard.content.sort,
            crate::db::models::ClipboardItemSort::UpdatedAt
        );
        assert!(!parsed.clipboard.content.copy_then_hide_window);
        assert!(
            parsed
                .clipboard
                .content
                .delete_favorite_items_only_in_favorite_group
        );
        assert!(!parsed.clipboard.content.delete_favorite_items);
        assert!(parsed.clipboard.content.delete_favorite_confirm);
        assert!(!parsed.clipboard.content.delete_pinned_items);
        assert!(parsed.clipboard.content.delete_pinned_confirm);
        assert!(
            parsed.clipboard.content.update_on_reuse,
            "v1 起默认开启，复用后条目顶到第一位"
        );
        assert_eq!(
            parsed.clipboard.content.merge_paste_separator,
            crate::settings::MergePasteSeparator::Newline
        );
        assert_eq!(parsed.clipboard.history.cleanup_interval_hours, 0);
        assert!(parsed.clipboard.window.scroll_to_top_on_open);
        assert_eq!(
            parsed.clipboard.window.select_range_on_open,
            crate::settings::WindowOpenRangeSelection::Preserve
        );
        assert_eq!(
            parsed.clipboard.window.select_category_on_open,
            crate::settings::WindowOpenCategorySelection::Preserve
        );
        assert_eq!(
            parsed.clipboard.window.select_group_on_open,
            crate::settings::WINDOW_OPEN_SELECTION_PRESERVE
        );
    }

    #[test]
    fn validate_settings_rejects_invalid_open_group_selection() {
        let mut settings = Settings::default();
        settings.clipboard.window.select_group_on_open = "invalid".to_owned();

        assert!(validate_settings(&settings).is_err());
    }

    #[test]
    fn fresh_settings_carry_current_version() {
        let settings = Settings::default();
        assert_eq!(settings.settings_version, SETTINGS_VERSION);
    }

    #[test]
    fn migrate_settings_noop_for_current_version() {
        let mut settings = Settings::default();
        settings.clipboard.content.update_on_reuse = false;

        assert!(!migrate_settings(&mut settings));
        assert!(
            !settings.clipboard.content.update_on_reuse,
            "v1 之后用户主动关闭的值不被迁移覆盖"
        );
    }

    #[test]
    fn migration_flips_legacy_update_on_reuse_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILENAME);
        fs::write(
            &path,
            r#"{"clipboard": {"content": {"updateOnReuse": false}}}"#,
        )
        .unwrap();

        let settings = load_from_disk(&path).unwrap();
        assert!(settings.clipboard.content.update_on_reuse);
        assert_eq!(settings.settings_version, SETTINGS_VERSION);

        // 迁移结果立即回写：二次加载读到的已是 v1 文件。
        let raw: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(raw["settingsVersion"], 1);

        fs::write(
            &path,
            r#"{"clipboard": {"content": {"updateOnReuse": false}}, "settingsVersion": 1}"#,
        )
        .unwrap();
        let settings = load_from_disk(&path).unwrap();
        assert!(
            !settings.clipboard.content.update_on_reuse,
            "v1 文件里显式的 false 是用户选择，保持不动"
        );
    }

    #[test]
    fn settings_patch_cannot_rewind_file_version() {
        let (_dir, store) = store_for_version_test();
        let next = store
            .update(serde_json::json!({"settingsVersion": 0}))
            .unwrap();

        assert_eq!(next.settings_version, SETTINGS_VERSION);
    }
}

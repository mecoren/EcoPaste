//! 历史清理后台任务：按 `clipboard.history.retention` + `maxCount` 定期裁剪。
//!
//! 启动即跑一次；之后按用户设置的清理周期触发，每次都从 `SettingsStore` 取最新配置——
//! 用户在偏好里调时长 / 上限后不必重启即可生效。置顶与收藏项一律保留（由 [`cleanup_history`] 保证）。

use std::time::{Duration, Instant};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

use super::storage::ImageStore;
use super::watcher::CLIPBOARD_UPDATED_EVENT;
use crate::db::items::cleanup_history;
use crate::settings::{Retention, RetentionUnit, SettingsStore};

/// 调度器检查设置与到期状态的频率；真正清理只在用户设置周期到期后执行。
const SCHEDULER_TICK_INTERVAL: Duration = Duration::from_secs(60);

/// 单次清理删除行数达到该阈值时，清理完成后自动做一次 WAL truncate checkpoint，
/// 回收大量 DELETE 留下的 WAL 累积；低于阈值不动，避免高频小清理的 IO 抖动。
const WAL_CHECKPOINT_THRESHOLD_ROWS: u64 = 500;

/// 启动历史清理后台任务：启动立即清理一次，之后按设置周期到点清理。
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        run_once(&app).await;
        let mut last_cleanup_at = Instant::now();
        let mut ticker = tokio::time::interval(SCHEDULER_TICK_INTERVAL);
        ticker.tick().await;

        loop {
            ticker.tick().await;
            let Some(interval) = cleanup_interval(&app) else {
                continue;
            };

            if last_cleanup_at.elapsed() < interval {
                continue;
            }

            run_once(&app).await;
            last_cleanup_at = Instant::now();
        }
    });
}

async fn run_once(app: &AppHandle) {
    let history = match app.try_state::<SettingsStore>() {
        Some(store) => store.snapshot().clipboard.history,
        None => return,
    };

    let cutoff = retention_cutoff(&history.retention, Utc::now());
    let max = (history.max_count > 0).then_some(history.max_count);

    if cutoff.is_none() && max.is_none() {
        return;
    }

    let pool = app.state::<crate::db::DatabaseState>().pool().await;
    match cleanup_history(&pool, cutoff, max).await {
        Ok(outcome) if outcome.removed == 0 => {}
        Ok(outcome) => {
            remove_images(app, &outcome.image_files);
            log::info!("history cleanup removed {} item(s)", outcome.removed);
            checkpoint_if_bulk(&pool, outcome.removed).await;
            if let Err(err) = app.emit(
                CLIPBOARD_UPDATED_EVENT,
                json!({ "cleanup": outcome.removed }),
            ) {
                log::warn!("emit cleanup event failed: {err}");
            }
        }
        Err(err) => log::warn!("history cleanup failed: {err}"),
    }
}

/// 大批量删除后截断 WAL 回收磁盘；在线操作、毫秒级，失败只记日志不影响清理结果。
/// VACUUM 仍保持手动（设置页「压缩数据库」），自动 VACUUM 有锁库风险。
async fn checkpoint_if_bulk(pool: &sqlx::SqlitePool, removed: u64) {
    if removed < WAL_CHECKPOINT_THRESHOLD_ROWS {
        return;
    }

    match sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(pool)
        .await
    {
        Ok(_) => log::info!("wal truncated after bulk cleanup ({removed} rows removed)"),
        Err(err) => log::warn!("wal truncate after bulk cleanup failed: {err}"),
    }
}

/// 读取当前清理周期。`0` 表示关闭周期性清理。
fn cleanup_interval(app: &AppHandle) -> Option<Duration> {
    let store = app.try_state::<SettingsStore>()?;
    let hours = store.snapshot().clipboard.history.cleanup_interval_hours;

    if hours == 0 {
        return None;
    }

    Some(Duration::from_secs(u64::from(hours) * 60 * 60))
}

/// 删除被清理图片记录的落盘文件（原图 + 缩略图）。`ImageStore` 未注册或单个文件删除失败
/// 都只记日志、不阻断——清理本身已成功，残留文件最坏只是占用磁盘，不影响功能。
fn remove_images(app: &AppHandle, file_names: &[String]) {
    if file_names.is_empty() {
        return;
    }
    let Some(store) = app.try_state::<ImageStore>() else {
        log::warn!(
            "image store unavailable; skip removing {} image file(s)",
            file_names.len()
        );
        return;
    };
    for file_name in file_names {
        if let Err(err) = store.remove(file_name) {
            log::warn!("remove cleaned image {file_name} failed: {err}");
        }
    }
}

/// `Retention` → 绝对截止时间。`Forever` 或 `value == 0` 表示禁用。
/// 月份近似按 30 天处理（与前端展示口径一致，不引日历库）。
fn retention_cutoff(r: &Retention, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    if r.value == 0 {
        return None;
    }
    let dur = match r.unit {
        RetentionUnit::Forever => return None,
        RetentionUnit::Hours => ChronoDuration::hours(r.value as i64),
        RetentionUnit::Days => ChronoDuration::days(r.value as i64),
        RetentionUnit::Weeks => ChronoDuration::weeks(r.value as i64),
        RetentionUnit::Months => ChronoDuration::days((r.value as i64) * 30),
    };
    Some(now - dur)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).unwrap()
    }

    /// 大批量删除（≥ 阈值）后 checkpoint 把 WAL 截断到 0 页；小批量不触发。
    /// 在真实 sqlx/libsqlite3 运行时上执行与 `run_once` 相同的 SQL 路径。
    #[tokio::test]
    async fn checkpoint_if_bulk_truncates_wal_only_above_threshold() {
        use sqlx::sqlite::SqliteConnectOptions;
        use std::str::FromStr;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("checkpoint-test.db");
        let wal_path = dir.path().join("checkpoint-test.db-wal");
        let options = SqliteConnectOptions::from_str(db_path.to_str().unwrap())
            .unwrap()
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);

        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();

        sqlx::query("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        for i in 0..600 {
            sqlx::query("INSERT INTO t (v) VALUES (?)")
                .bind(format!("row-{i}"))
                .execute(&pool)
                .await
                .unwrap();
        }

        sqlx::query("DELETE FROM t WHERE id <= 100")
            .execute(&pool)
            .await
            .unwrap();
        checkpoint_if_bulk(&pool, 100).await;
        assert!(wal_size(&wal_path) > 0, "低于阈值的清理不应截断 WAL");

        sqlx::query("DELETE FROM t").execute(&pool).await.unwrap();
        checkpoint_if_bulk(&pool, 500).await;
        assert_eq!(
            wal_size(&wal_path),
            0,
            "达到阈值后 WAL 应被 truncate checkpoint 清零"
        );

        pool.close().await;
    }

    /// WAL sidecar 文件字节数；TRUNCATE checkpoint 后归零。文件不存在按 0 处理。
    fn wal_size(path: &std::path::Path) -> u64 {
        std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    }

    #[test]
    fn retention_cutoff_returns_none_when_disabled() {
        assert!(retention_cutoff(
            &Retention {
                value: 0,
                unit: RetentionUnit::Days
            },
            now()
        )
        .is_none());
        assert!(retention_cutoff(
            &Retention {
                value: 7,
                unit: RetentionUnit::Forever
            },
            now()
        )
        .is_none());
    }

    #[test]
    fn retention_cutoff_subtracts_by_unit() {
        let n = now();
        assert_eq!(
            retention_cutoff(
                &Retention {
                    value: 2,
                    unit: RetentionUnit::Hours
                },
                n
            ),
            Some(n - ChronoDuration::hours(2))
        );
        assert_eq!(
            retention_cutoff(
                &Retention {
                    value: 3,
                    unit: RetentionUnit::Days
                },
                n
            ),
            Some(n - ChronoDuration::days(3))
        );
        assert_eq!(
            retention_cutoff(
                &Retention {
                    value: 1,
                    unit: RetentionUnit::Weeks
                },
                n
            ),
            Some(n - ChronoDuration::weeks(1))
        );
        assert_eq!(
            retention_cutoff(
                &Retention {
                    value: 1,
                    unit: RetentionUnit::Months
                },
                n
            ),
            Some(n - ChronoDuration::days(30))
        );
    }
}

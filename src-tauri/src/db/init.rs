use anyhow::Context;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{ConnectOptions, SqlitePool};
use std::time::Duration;
use tauri::AppHandle;

use crate::core::Result;
use crate::db::{db_path, items};

/// 单 SQLite 文件 + WAL 下连接越多只会放大锁竞争与页缓存开销；
/// 5 已覆盖「监听入库 + 列表查询 + 设置写入」的常态并发。
const MAX_CONNECTIONS: u32 = 5;

pub async fn init(app: &AppHandle) -> Result<SqlitePool> {
    let path = db_path(app)?;

    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5))
        .disable_statement_logging();

    let pool = SqlitePoolOptions::new()
        .max_connections(MAX_CONNECTIONS)
        .connect_with(options)
        .await
        .with_context(|| format!("failed to open sqlite database at {path:?}"))?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("failed to run sqlite migrations")?;

    // 存量文本行的换行归一（一次性数据修复，幂等）。失败不阻断启动——旧行保持原样，
    // 只是与新采集口径的哈希对不上，同内容会再入一条新行。
    match items::normalize_legacy_text_breaks(&pool).await {
        Ok(0) => {}
        Ok(fixed) => log::info!("normalized line breaks in {fixed} legacy clipboard text items"),
        Err(err) => log::warn!("legacy text line-break normalization failed: {err:#}"),
    }

    log::info!("sqlite pool ready at {path:?}");
    Ok(pool)
}

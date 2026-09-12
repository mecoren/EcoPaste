//! file_type_icons 表：按文件类型缓存 icon，避免重复抽取系统 icon。
//!
//! cache_key 生成规则见 `clipboard::icon::get_icon_cache_key`。

use std::collections::HashMap;

use chrono::Utc;
use sqlx::SqlitePool;

use super::models::Platform;
use crate::core::Result;

/// 查询指定 cache_key 的 icon 文件名。
pub async fn get_icon(
    pool: &SqlitePool,
    cache_key: &str,
    platform: Platform,
) -> Result<Option<String>> {
    let row = sqlx::query_as::<_, (String,)>(
        "SELECT icon_file FROM file_type_icons WHERE cache_key = ? AND platform = ?",
    )
    .bind(cache_key)
    .bind(platform)
    .fetch_optional(pool)
    .await
    .map_err(|e| {
        log::error!("query file_type_icons failed: {e}");
        anyhow::anyhow!("{e}")
    })?;
    Ok(row.map(|r| r.0))
}

/// 批量查询一组 cache_key 的 icon 文件名，返回 `cache_key -> icon_file` 映射。
/// `cache_keys` 为空时不发查询。供列表 / 预览条目组装时一次取整页图标，
/// 消除逐路径单查的 N+1 往返。
pub async fn get_icons(
    pool: &SqlitePool,
    cache_keys: &[String],
    platform: Platform,
) -> Result<HashMap<String, String>> {
    if cache_keys.is_empty() {
        return Ok(HashMap::new());
    }

    let mut qb: sqlx::QueryBuilder<sqlx::Sqlite> = sqlx::QueryBuilder::new(
        "SELECT cache_key, icon_file FROM file_type_icons WHERE platform = ",
    );
    qb.push_bind(platform);
    qb.push(" AND cache_key IN (");
    let mut separated = qb.separated(", ");
    for key in cache_keys {
        separated.push_bind(key);
    }
    qb.push(")");

    let rows = qb
        .build_query_as::<(String, String)>()
        .fetch_all(pool)
        .await
        .map_err(|e| {
            log::error!("query file_type_icons batch failed: {e}");
            anyhow::anyhow!("{e}")
        })?;

    Ok(rows.into_iter().collect())
}

/// upsert：插入或更新 icon 记录。
pub async fn upsert_icon(
    pool: &SqlitePool,
    cache_key: &str,
    platform: Platform,
    icon_file: &str,
) -> Result<()> {
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO file_type_icons (cache_key, platform, icon_file, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT (cache_key, platform) DO UPDATE SET
             icon_file = excluded.icon_file,
             updated_at = excluded.updated_at",
    )
    .bind(cache_key)
    .bind(platform)
    .bind(icon_file)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| {
        log::error!("upsert file_type_icons failed: {e}");
        anyhow::anyhow!("{e}")
    })?;
    Ok(())
}

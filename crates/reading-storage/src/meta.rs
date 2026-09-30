//! `app_meta`（应用元信息）：schema 版本、内容索引版本、迁移历史。
//!
//! 8 张冻结表中 `app_meta` 是唯一允许存「运行时元信息」的表；迁移历史也复用
//! 本表的 `key/value` 行，避免引入第 9 张表（gate-0 §2.6 冻结为 8 张表）。

use rusqlite::{Connection, OptionalExtension};

use crate::connection;
use crate::error::{Result, StorageError};
use crate::validate::ensure_meta_key;

/// schema 版本键（与 `PRAGMA user_version` 双写，互为校验）。
pub const SCHEMA_VERSION_KEY: &str = "schema_version";
/// 内容索引版本键（reading-core `ContentUnit.content_index_version` 的持久化副本）。
pub const CONTENT_INDEX_VERSION_KEY: &str = "content_index_version";

/// 章节扫描器版本键（本地副本；与代码常量不一致即强制重扫）。
pub const CHAPTER_SCANNER_VERSION_KEY: &str = "chapter_scanner_version";

/// `app_meta` 表是否已建立（初始迁移前为 false）。
pub fn table_exists(conn: &Connection) -> Result<bool> {
    let found: Option<String> = conn
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'app_meta'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

/// 读取元信息键。
pub fn get(conn: &Connection, key: &str) -> Result<Option<String>> {
    ensure_meta_key(key)?;
    if !table_exists(conn)? {
        return Ok(None);
    }
    let value = conn
        .query_row("SELECT value FROM app_meta WHERE key = ?1", [key], |row| {
            row.get::<_, String>(0)
        })
        .optional()?;
    Ok(value)
}

/// 写入（upsert）元信息键。
pub fn set(conn: &Connection, key: &str, value: &str, now: i64) -> Result<()> {
    ensure_meta_key(key)?;
    connection::ensure_meta_value_len(value)?;
    conn.execute(
        "INSERT INTO app_meta (key, value, updated_at) VALUES (?1, ?2, ?3) \
         ON CONFLICT (key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        rusqlite::params![key, value, now],
    )?;
    Ok(())
}

/// 删除元信息键；返回是否确有删除。
pub fn remove(conn: &Connection, key: &str) -> Result<bool> {
    ensure_meta_key(key)?;
    if !table_exists(conn)? {
        return Ok(false);
    }
    let affected = conn.execute("DELETE FROM app_meta WHERE key = ?1", [key])?;
    Ok(affected > 0)
}

/// 元信息行数。
pub fn count(conn: &Connection) -> Result<i64> {
    if !table_exists(conn)? {
        return Ok(0);
    }
    connection::count_rows(conn, "app_meta")
}

/// 整数型元信息读取（缺失/非法均为 0，不猜测）。
pub fn get_i64(conn: &Connection, key: &str) -> Result<i64> {
    match get(conn, key)? {
        Some(raw) => raw
            .trim()
            .parse::<i64>()
            .map_err(|_| StorageError::internal(format!("app_meta.{key} 不是整数: `{raw}`"))),
        None => Ok(0),
    }
}

/// 当前 schema 版本（未迁移时为 0）。
pub fn schema_version(conn: &Connection) -> Result<i64> {
    get_i64(conn, SCHEMA_VERSION_KEY)
}

/// 写入 schema 版本（同时更新 `PRAGMA user_version`）。
pub fn set_schema_version(conn: &Connection, version: i64, now: i64) -> Result<()> {
    set(conn, SCHEMA_VERSION_KEY, &version.to_string(), now)?;
    connection::set_user_version(conn, version)?;
    Ok(())
}

/// 迁移历史键：`migration.<name>.applied_at`。
pub fn migration_history_key(name: &str) -> String {
    format!("migration.{name}.applied_at")
}

/// 当前内容索引版本。
pub fn content_index_version(conn: &Connection) -> Result<i64> {
    get_i64(conn, CONTENT_INDEX_VERSION_KEY)
}

/// 本地记录的章节扫描器版本（从未写入过时为 0，因而必然与代码常量不一致 ⇒ 首次升级会自动重扫）。
pub fn chapter_scanner_version(conn: &Connection) -> Result<i64> {
    get_i64(conn, CHAPTER_SCANNER_VERSION_KEY)
}

/// 记录当前章节扫描器版本。
pub fn set_chapter_scanner_version(conn: &Connection, version: i64, now: i64) -> Result<()> {
    set(conn, CHAPTER_SCANNER_VERSION_KEY, &version.to_string(), now)
}

/// 递增内容索引版本并返回新值（编码切换 / 文件指纹变化后调用，使可重建索引失效）。
pub fn bump_content_index_version(conn: &Connection, now: i64) -> Result<i64> {
    let next = content_index_version(conn)?.saturating_add(1);
    set(conn, CONTENT_INDEX_VERSION_KEY, &next.to_string(), now)?;
    Ok(next)
}

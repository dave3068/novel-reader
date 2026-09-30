//! 连接建立与连接级 PRAGMA。
//!
//! 约束：正文不入库、不联网、不依赖 UI。连接使用 WAL（读多写少、崩溃恢复友好）
//! 并强制 `foreign_keys = ON`（避免出现孤儿进度/书签）。

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use crate::error::{Result, StorageError};
use crate::validate::{ensure_identifier, ensure_max_len, MAX_META_VALUE_BYTES};

/// 忙等待上限（毫秒）：多入口串行写库时的退避上限。
pub const DEFAULT_BUSY_TIMEOUT_MS: u64 = 5_000;

/// 副作用：打开（或创建）文件数据库并应用连接级 PRAGMA。
pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path).map_err(|e| StorageError::io(path, e.to_string()))?;
    configure_file_connection(&conn)?;
    Ok(conn)
}

/// 副作用：打开内存数据库并应用连接级 PRAGMA（测试与迁移演练用）。
pub fn open_in_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    configure(&conn)?;
    Ok(conn)
}

fn configure_file_connection(conn: &Connection) -> Result<()> {
    // WAL 是数据库级别设置，且会返回一行结果，必须用 query_row 读取（pragma_update 会报错）。
    let mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") && !mode.eq_ignore_ascii_case("memory") {
        return Err(StorageError::internal(format!(
            "无法启用 WAL 日志模式：journal_mode = {mode}"
        )));
    }
    configure(conn)
}

/// 连接级 PRAGMA：外键、忙等待、同步级别。
pub fn configure(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(Duration::from_millis(DEFAULT_BUSY_TIMEOUT_MS))?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}

/// 当前 Unix 时间戳（秒）。所有仓储写操作显式接收时间戳，便于确定性断言。
pub fn now_unix() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(delta) => delta.as_secs() as i64,
        Err(_) => 0,
    }
}

/// `sqlite_master` 中的用户表名（排除 `sqlite_%` 内部表），已排序。
pub fn table_names(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut names = Vec::new();
    for row in rows {
        names.push(row?);
    }
    Ok(names)
}

/// `sqlite_master` 中的索引名（排除自动索引 `sqlite_autoindex_%`），已排序。
pub fn index_names(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'index' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut names = Vec::new();
    for row in rows {
        names.push(row?);
    }
    Ok(names)
}

/// 表的列名（按 `PRAGMA table_info` 顺序）。
pub fn column_names(conn: &Connection, table: &str) -> Result<Vec<String>> {
    ensure_identifier(table)?;
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let mut names = Vec::new();
    for row in rows {
        names.push(row?);
    }
    Ok(names)
}

/// `PRAGMA user_version`。
pub fn user_version(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("PRAGMA user_version", [], |row| row.get(0))?)
}

/// 设置 `PRAGMA user_version`。
pub fn set_user_version(conn: &Connection, version: i64) -> Result<()> {
    conn.pragma_update(None, "user_version", version)?;
    Ok(())
}

/// 外键是否启用（用于断言，不是产品行为）。
pub fn foreign_keys_enabled(conn: &Connection) -> Result<bool> {
    let enabled: i64 = conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
    Ok(enabled != 0)
}

/// `journal_mode` 文本。
pub fn journal_mode(conn: &Connection) -> Result<String> {
    Ok(conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?)
}

/// 单表行数（表名经白名单校验后拼接）。
pub fn count_rows(conn: &Connection, table: &str) -> Result<i64> {
    ensure_identifier(table)?;
    Ok(
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })?,
    )
}

/// 迁移前自检：数据库文件必须可写（把「备份失败」挡在改数据之前）。
pub fn ensure_file_writable(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.is_dir() {
            return Err(StorageError::io(
                parent,
                "数据库所在目录不存在，无法创建/写入数据库文件",
            ));
        }
    }
    Ok(())
}

/// `app_meta` 值长度的统一上限检查（供 `meta` 模块复用）。
pub fn ensure_meta_value_len(value: &str) -> Result<()> {
    ensure_max_len("app_meta.value", value, MAX_META_VALUE_BYTES)
}

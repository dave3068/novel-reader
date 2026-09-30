//! 迁移执行器：迁移前备份、事务化应用、失败可恢复。
//!
//! 规则（gate-0 §2.6 + 开发方案 §15.4）：
//! 1. 迁移脚本按 `migrations/0001_*.sql`、`0002_*.sql` 递增，**只新增版本，不覆盖用户数据**；
//! 2. 升级前自动备份数据库（SQLite 在线备份 API，WAL 安全）；
//! 3. 单条迁移在**一个事务**内完成：DDL + 版本号 + 迁移历史一起提交，失败整体回滚
//!    （异常中断后数据库仍停在迁移前的版本，不会留下半成品 schema）；
//! 4. 迁移失败返回 [`StorageError::MigrationFailed`]，携带备份路径供
//!    [`restore_from_backup`] 恢复；原始文件与用户元数据在任何路径上都不会被删除。

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension};

use crate::error::{Result, StorageError};
use crate::{connection, meta, validate};

/// 初始迁移脚本（编译期内嵌，与 `migrations/0001_init.sql` 逐字节一致，
/// 由集成测试 `embedded_migration_matches_on_disk_file` 守住）。
pub const INITIAL_MIGRATION_SQL: &str = include_str!("../../../migrations/0001_init.sql");

/// 初始迁移文件名。
pub const INITIAL_MIGRATION_NAME: &str = "0001_init";

/// 仓库内初始迁移路径（相对仓库根）。
pub const INITIAL_MIGRATION_RELATIVE_PATH: &str = "migrations/0001_init.sql";

/// V1 目标 schema 版本。
pub const TARGET_SCHEMA_VERSION: i64 = 1;

/// 备份文件后缀。
pub const BACKUP_SUFFIX: &str = "backup";

/// 一条迁移。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Migration {
    pub version: i64,
    pub name: &'static str,
    pub sql: &'static str,
}

/// V1 冻结迁移序列。
pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: INITIAL_MIGRATION_NAME,
    sql: INITIAL_MIGRATION_SQL,
}];

/// 待执行的迁移计划。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationPlan {
    pub from_version: i64,
    pub to_version: i64,
    pub pending: Vec<(i64, String)>,
}

/// 迁移结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationOutcome {
    /// 迁移前版本。
    pub from_version: i64,
    /// 迁移后版本。
    pub to_version: i64,
    /// 本次真实执行的迁移（版本, 名称）。
    pub applied: Vec<(i64, String)>,
    /// 迁移前备份路径（已是最新版本时为 `None`）。
    pub backup: Option<PathBuf>,
    /// 是否发生了 schema 变更。
    pub changed: bool,
}

impl MigrationOutcome {
    /// 是否为「无待执行迁移」的空转结果（重复执行初始迁移即为此情形）。
    pub fn is_noop(&self) -> bool {
        !self.changed && self.applied.is_empty()
    }
}

/// 迁移选项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrateOptions {
    /// 迁移前是否备份（默认开；关闭仅用于无文件的内存库或演练）。
    pub backup: bool,
    /// 备份目录（默认与数据库同目录）。
    pub backup_dir: Option<PathBuf>,
}

impl Default for MigrateOptions {
    fn default() -> Self {
        Self {
            backup: true,
            backup_dir: None,
        }
    }
}

impl MigrateOptions {
    pub fn with_backup() -> Self {
        Self::default()
    }

    pub fn without_backup() -> Self {
        Self {
            backup: false,
            backup_dir: None,
        }
    }

    pub fn with_backup_dir(dir: impl Into<PathBuf>) -> Self {
        Self {
            backup: true,
            backup_dir: Some(dir.into()),
        }
    }
}

/// 校验迁移序列：版本单调递增且为正、名称与 SQL 非空。
pub fn validate_migrations(migrations: &[Migration]) -> Result<()> {
    let mut previous = 0_i64;
    for migration in migrations {
        if migration.version <= previous {
            return Err(StorageError::invalid(format!(
                "迁移版本必须严格递增：{} 出现在 {} 之后",
                migration.version, previous
            )));
        }
        if migration.version < 1 {
            return Err(StorageError::invalid("迁移版本必须 >= 1"));
        }
        if migration.name.trim().is_empty() || migration.sql.trim().is_empty() {
            return Err(StorageError::invalid(format!(
                "迁移 {} 的名称或 SQL 为空",
                migration.version
            )));
        }
        previous = migration.version;
    }
    Ok(())
}

/// 当前 schema 版本（未迁移的库为 0）。
pub fn current_schema_version(conn: &Connection) -> Result<i64> {
    meta::schema_version(conn)
}

/// 计算待执行计划。
pub fn plan_with(conn: &Connection, migrations: &[Migration]) -> Result<MigrationPlan> {
    validate_migrations(migrations)?;
    let from_version = current_schema_version(conn)?;
    let to_version = migrations
        .iter()
        .map(|m| m.version)
        .max()
        .unwrap_or(from_version);
    let pending = migrations
        .iter()
        .filter(|m| m.version > from_version)
        .map(|m| (m.version, m.name.to_string()))
        .collect();
    Ok(MigrationPlan {
        from_version,
        to_version,
        pending,
    })
}

/// 冻结序列的迁移计划。
pub fn plan(conn: &Connection) -> Result<MigrationPlan> {
    plan_with(conn, MIGRATIONS)
}

/// 备份文件路径：`<db>.backup-v<from>-<unix>-<version>`（同秒重复调用也不会互相覆盖）。
pub fn backup_file_path(
    db_path: &Path,
    from_version: i64,
    now: i64,
    backup_dir: Option<&Path>,
) -> PathBuf {
    let file_name = db_path.file_name().map_or_else(
        || "novel-reader.db".to_string(),
        |n| n.to_string_lossy().into(),
    );
    let candidate =
        format!("{file_name}.{BACKUP_SUFFIX}-v{from_version}-{now}.{TARGET_SCHEMA_VERSION}");
    match backup_dir {
        Some(dir) => dir.join(candidate),
        None => db_path.with_file_name(candidate),
    }
}

/// 用 SQLite 在线备份 API 把 `conn` 的完整状态复制到 `target`（WAL 安全）。
pub fn backup_database(conn: &Connection, target: &Path) -> Result<()> {
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| StorageError::io(parent, format!("创建备份目录失败: {e}")))?;
        }
    }
    remove_if_exists(target)?;
    let mut destination =
        Connection::open(target).map_err(|e| StorageError::io(target, e.to_string()))?;
    {
        let backup = rusqlite::backup::Backup::new(conn, &mut destination)?;
        backup.run_to_completion(256, Duration::from_millis(0), None)?;
    }
    destination
        .close()
        .map_err(|(_, e)| StorageError::io(target, format!("关闭备份文件失败: {e}")))?;
    Ok(())
}

/// 从备份恢复到数据库路径。调用前**必须**先关闭指向该路径的连接。
pub fn restore_from_backup(backup: &Path, db_path: &Path) -> Result<()> {
    if !backup.is_file() {
        return Err(StorageError::invalid(format!(
            "备份文件不存在：{}",
            backup.display()
        )));
    }
    // WAL 边车文件必须一并清理，否则旧日志会覆盖恢复结果。
    remove_if_exists(db_path)?;
    remove_if_exists(&sidecar(db_path, "-wal"))?;
    remove_if_exists(&sidecar(db_path, "-shm"))?;
    let source = Connection::open(backup).map_err(|e| StorageError::io(backup, e.to_string()))?;
    {
        let mut target =
            Connection::open(db_path).map_err(|e| StorageError::io(db_path, e.to_string()))?;
        let restore = rusqlite::backup::Backup::new(&source, &mut target)?;
        restore.run_to_completion(256, Duration::from_millis(0), None)?;
    }
    Ok(())
}

/// SQLite 边车文件（`-wal` / `-shm`）路径。
pub fn sidecar(db_path: &Path, suffix: &str) -> PathBuf {
    let file_name = db_path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into());
    db_path.with_file_name(format!("{file_name}{suffix}"))
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(StorageError::io(path, format!("删除失败: {error}"))),
    }
}

/// 是否已有该版本迁移历史记录。
pub fn migration_applied(conn: &Connection, migration: &Migration) -> Result<bool> {
    Ok(meta::get(conn, &meta::migration_history_key(migration.name))?.is_some())
}

/// 用冻结序列迁移（带备份）。
pub fn migrate(conn: &mut Connection, db_path: Option<&Path>) -> Result<MigrationOutcome> {
    migrate_with(conn, db_path, MIGRATIONS, &MigrateOptions::default())
}

/// 迁移主流程：备份 → 逐条事务化应用 → 记录历史。
///
/// 失败语义：任一条迁移失败时返回 [`StorageError::MigrationFailed`]，其中
/// `backup` 指向迁移前备份；失败事务已回滚，数据库仍停在迁移前版本。
pub fn migrate_with(
    conn: &mut Connection,
    db_path: Option<&Path>,
    migrations: &[Migration],
    options: &MigrateOptions,
) -> Result<MigrationOutcome> {
    validate_migrations(migrations)?;
    let plan = plan_with(conn, migrations)?;
    if plan.pending.is_empty() {
        return Ok(MigrationOutcome {
            from_version: plan.from_version,
            to_version: plan.from_version,
            applied: Vec::new(),
            backup: None,
            changed: false,
        });
    }

    // 1) 迁移前备份：备份失败 ⇒ 不迁移（宁可停在旧版本，也不冒丢失风险）。
    let backup = match (options.backup, db_path) {
        (true, Some(path)) => {
            connection::ensure_file_writable(path)?;
            let target = backup_file_path(
                path,
                plan.from_version,
                connection::now_unix(),
                options.backup_dir.as_deref(),
            );
            backup_database(conn, &target)?;
            Some(target)
        }
        _ => None,
    };

    // 2) 逐条事务化应用。
    let mut applied: Vec<(i64, String)> = Vec::new();
    for version in plan.pending.iter().map(|(v, _)| *v) {
        let migration = migrations
            .iter()
            .find(|m| m.version == version)
            .ok_or_else(|| StorageError::internal(format!("迁移 {version} 不在序列中")))?;
        if let Err(detail) = apply_one(conn, migration) {
            return Err(StorageError::MigrationFailed {
                version: migration.version,
                name: migration.name.to_string(),
                backup,
                detail,
            });
        }
        applied.push((migration.version, migration.name.to_string()));
    }

    let to_version = applied.last().map_or(plan.from_version, |(v, _)| *v);
    Ok(MigrationOutcome {
        from_version: plan.from_version,
        to_version,
        applied,
        backup,
        changed: true,
    })
}

/// 单条迁移：DDL + schema 版本 + 迁移历史写在同一个事务里。
fn apply_one(conn: &mut Connection, migration: &Migration) -> std::result::Result<(), String> {
    let now = connection::now_unix();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute_batch(migration.sql).map_err(|e| e.to_string())?;
    meta::set_schema_version(&tx, migration.version, now).map_err(|e| e.to_string())?;
    meta::set(
        &tx,
        &meta::migration_history_key(migration.name),
        &now.to_string(),
        now,
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

/// `PRAGMA user_version` 与 `app_meta.schema_version` 是否一致（迁移一致性自检）。
pub fn version_is_consistent(conn: &Connection) -> Result<bool> {
    let from_meta = meta::schema_version(conn)?;
    let from_pragma = connection::user_version(conn)?;
    Ok(from_meta == from_pragma)
}

/// 8 张冻结表是否齐备（用于启动自检与测试）。
pub fn frozen_tables_present(conn: &Connection) -> Result<Vec<String>> {
    let existing = connection::table_names(conn)?;
    Ok(crate::FROZEN_TABLES
        .iter()
        .filter(|name| existing.iter().any(|e| e == *name))
        .map(|name| (*name).to_string())
        .collect())
}

/// 数据库文件是否存在（备份/恢复路径断言用）。
pub fn database_file_exists(db_path: &Path) -> bool {
    db_path.is_file()
}

/// 已应用迁移历史键集合（按字典序）。
pub fn applied_migration_keys(conn: &Connection) -> Result<Vec<String>> {
    if !meta::table_exists(conn)? {
        return Ok(Vec::new());
    }
    let mut stmt =
        conn.prepare("SELECT key FROM app_meta WHERE key LIKE 'migration.%' ORDER BY key")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut keys = Vec::new();
    for row in rows {
        keys.push(row?);
    }
    Ok(keys)
}

/// 迁移前「表尚未建立」的判定（用于迁移演练断言）。
pub fn table_absent(conn: &Connection, table: &str) -> Result<bool> {
    validate::ensure_identifier(table)?;
    let found: Option<String> = conn
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_none())
}

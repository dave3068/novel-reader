//! `reading-storage`：SQLite 连接、迁移与事务仓储。
//!
//! ## 职责
//!
//! - `migrations/0001_init.sql` 的 8 张表：`books`、`book_sources`、`chapters`、
//!   `reading_progress`、`bookmarks`、`reader_settings`、`layout_records`、`app_meta`；
//! - 事务化仓储：导入 upsert / 重复导入、进度幂等 upsert、书签 CRUD、章节索引与分页缓存写入；
//! - 迁移前自动备份、迁移失败可恢复（事务回滚 + 备份恢复）；
//! - 「清理索引/分页缓存」只删可重建数据（`chapters` / `layout_records`），
//!   **绝不**删除书籍、进度、书签、设置与元信息，**绝不**删除用户原始 TXT。
//!
//! ## 边界
//!
//! - 正文不入库：只保存元数据、索引、位置与短摘要（见 [`validate`] 里的长度上限）；
//! - 本 crate 位于 `reading-core` 下层，不依赖 `reading-core`（否则成环）；错误码用同数值的
//!   本地 [`StorageError`] 暴露 [`StorageError::code`]，数值与 gate-0 §2.2 冻结表一致；
//! - 不依赖 UI / Android / 网络；只使用本地 SQLite 文件（`rusqlite` bundled）。

pub mod background;
pub mod bookmarks;
pub mod books;
pub mod categories;
pub mod connection;
pub mod error;
pub mod history;
pub mod history_view;
pub mod index;
pub mod meta;
pub mod migrate;
pub mod progress;
pub mod settings;
pub mod theme;
pub mod theme_derive;
pub mod validate;

use std::path::{Path, PathBuf};

use rusqlite::Connection;

pub use error::{Result, StorageError};

/// V1 冻结的 8 张表（gate-0 §2.6；按字典序，便于逐项比对）。
///
/// **只含 0001 初始迁移建立的表**；V1.1（0002）新增的 4 张表见 [`V11_TABLES`]。
/// 两者分开登记，是为了让「V1 冻结集」与「当前 schema」能被分别核对。
pub const FROZEN_TABLES: &[&str] = &[
    "app_meta",
    "book_sources",
    "bookmarks",
    "books",
    "chapters",
    "layout_records",
    "reader_settings",
    "reading_progress",
];

/// V1.1（0002 迁移）新增的表（按字典序）。
pub const V11_TABLES: &[&str] = &[
    "book_categories",
    "reader_background",
    "reader_theme_custom",
    "reading_history",
];

/// 当前 schema 版本应含的全部表（V1 冻结 8 张 + V1.1 新增 4 张；按字典序）。
pub fn expected_tables() -> Vec<String> {
    let mut tables: Vec<String> = FROZEN_TABLES
        .iter()
        .chain(V11_TABLES.iter())
        .map(|name| (*name).to_string())
        .collect();
    tables.sort_unstable();
    tables
}

/// 本地存储句柄：连接 + 数据库文件路径（路径用于迁移前备份与恢复）。
pub struct Storage {
    conn: Connection,
    path: Option<PathBuf>,
}

impl Storage {
    /// 打开（或创建）文件数据库；自动创建父目录并应用连接级 PRAGMA。
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| StorageError::io(parent, format!("创建数据库目录失败: {e}")))?;
            }
        }
        let conn = connection::open(&path)?;
        Ok(Self {
            conn,
            path: Some(path),
        })
    }

    /// 打开内存数据库（测试与迁移演练；无备份路径）。
    pub fn open_in_memory() -> Result<Self> {
        Ok(Self {
            conn: connection::open_in_memory()?,
            path: None,
        })
    }

    /// 由既有连接构造（调用方自行保证 PRAGMA 已设置）。
    pub fn from_connection(conn: Connection, path: Option<PathBuf>) -> Self {
        Self { conn, path }
    }

    /// 数据库文件路径；内存库为 `None`。
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    pub fn into_connection(self) -> Connection {
        self.conn
    }

    /// 按冻结序列迁移（带迁移前备份）。
    pub fn migrate(&mut self) -> Result<migrate::MigrationOutcome> {
        migrate::migrate(&mut self.conn, self.path.as_deref())
    }

    /// 按给定序列/选项迁移（演练失败恢复用）。
    pub fn migrate_with(
        &mut self,
        migrations: &[migrate::Migration],
        options: &migrate::MigrateOptions,
    ) -> Result<migrate::MigrationOutcome> {
        migrate::migrate_with(&mut self.conn, self.path.as_deref(), migrations, options)
    }

    /// 当前 schema 版本（`app_meta.schema_version`）。
    pub fn schema_version(&self) -> Result<i64> {
        meta::schema_version(&self.conn)
    }

    /// `PRAGMA user_version`（与 `schema_version` 互为校验）。
    pub fn user_version(&self) -> Result<i64> {
        connection::user_version(&self.conn)
    }

    /// 迁移版本自检：`app_meta.schema_version` 与 `PRAGMA user_version` 是否一致。
    pub fn version_is_consistent(&self) -> Result<bool> {
        migrate::version_is_consistent(&self.conn)
    }

    /// 立即备份到指定路径（SQLite 在线备份，WAL 安全）。
    pub fn backup_to(&self, target: impl AsRef<Path>) -> Result<()> {
        migrate::backup_database(&self.conn, target.as_ref())
    }

    /// 从备份恢复：关闭当前连接 → 覆盖数据库文件 → 重新打开。
    pub fn restore_from_backup(&mut self, backup: impl AsRef<Path>) -> Result<()> {
        let path = self.path.clone().ok_or_else(|| {
            StorageError::invalid("内存数据库没有可恢复的文件路径，无法从备份恢复")
        })?;
        let placeholder = connection::open_in_memory()?;
        let previous = std::mem::replace(&mut self.conn, placeholder);
        drop(previous);
        migrate::restore_from_backup(backup.as_ref(), &path)?;
        self.conn = connection::open(&path)?;
        Ok(())
    }

    /// 清理可重建索引与分页缓存（设置页「清理索引/分页缓存」）。
    pub fn clear_rebuildable_index(&self) -> Result<index::IndexClearReport> {
        index::clear_rebuildable_index(&self.conn)
    }

    /// 表清单（`sqlite_master` 用户表，已排序）。
    pub fn table_names(&self) -> Result<Vec<String>> {
        connection::table_names(&self.conn)
    }

    /// 审计辅助：列出全部用户表格 + 索引名 + 行数。
    pub fn table_row_counts(&self) -> Result<Vec<(String, i64)>> {
        let mut counts = Vec::new();
        for table in connection::table_names(&self.conn)? {
            counts.push((table.clone(), connection::count_rows(&self.conn, &table)?));
        }
        Ok(counts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use error::*;

    /// gate-0 §2.2 的 12 项错误码数值（本 crate 侧同名常量必须逐项一致）。
    #[test]
    fn error_code_constants_match_gate0_frozen_table() {
        let expected = [
            (0_i32, ERR_OK, "Ok"),
            (1, ERR_INVALID_ARGUMENT, "InvalidArgument"),
            (2, ERR_OUT_OF_RANGE, "OutOfRange"),
            (3, ERR_UTF8_BOUNDARY, "Utf8Boundary"),
            (4, ERR_BOOK_NOT_FOUND, "BookNotFound"),
            (5, ERR_NO_OPEN_BOOK, "NoOpenBook"),
            (6, ERR_IO, "IoError"),
            (7, ERR_CANCELLED, "Cancelled"),
            (8, ERR_LAYOUT_NOT_FOUND, "LayoutNotFound"),
            (9, ERR_ENGINE_DESTROYED, "EngineDestroyed"),
            (10, ERR_INTERNAL, "Internal"),
            (11, ERR_CHAPTER_OUT_OF_RANGE, "ChapterOutOfRange"),
        ];
        for (value, actual, name) in expected {
            assert_eq!(value, actual, "错误码 {name} 与 gate-0 冻结值不一致");
        }
    }

    #[test]
    fn storage_error_variants_map_to_frozen_codes() {
        let invalid = StorageError::invalid("x");
        assert_eq!(invalid.code(), ERR_INVALID_ARGUMENT);
        let out_of_range = StorageError::out_of_range("x");
        assert_eq!(out_of_range.code(), ERR_OUT_OF_RANGE);
        let missing = StorageError::BookNotFound { book_id: 7 };
        assert_eq!(missing.code(), ERR_BOOK_NOT_FOUND);
        let io = StorageError::io("/tmp/x", "boom");
        assert_eq!(io.code(), ERR_IO);
        let internal = StorageError::internal("x");
        assert_eq!(internal.code(), ERR_INTERNAL);
        let migration = StorageError::MigrationFailed {
            version: 2,
            name: "0002_x".to_string(),
            backup: Some(PathBuf::from("/tmp/db.backup")),
            detail: "boom".to_string(),
        };
        assert_eq!(migration.code(), ERR_INTERNAL);
        assert!(migration.is_recoverable());
        assert!(migration.backup_path().is_some());
        assert!(!invalid.is_recoverable());
    }

    #[test]
    fn frozen_table_list_is_exactly_eight_unique_sorted_tables() {
        let mut tables = FROZEN_TABLES.to_vec();
        assert_eq!(tables.len(), 8, "V1 冻结表清单必须恰为 8 张（gate-0 §2.6）");
        tables.sort_unstable();
        tables.dedup();
        assert_eq!(tables, FROZEN_TABLES, "冻结表清单必须无重复且按字典序排列");
        for required in [
            "books",
            "book_sources",
            "chapters",
            "reading_progress",
            "bookmarks",
            "reader_settings",
            "layout_records",
            "app_meta",
        ] {
            assert!(
                FROZEN_TABLES.contains(&required),
                "冻结表清单缺少 {required}"
            );
        }
    }

    #[test]
    fn v11_table_list_is_four_unique_sorted_tables() {
        let mut tables = V11_TABLES.to_vec();
        assert_eq!(tables.len(), 4, "V1.1 新增表必须恰为 4 张");
        tables.sort_unstable();
        tables.dedup();
        assert_eq!(tables, V11_TABLES, "V1.1 表清单必须无重复且按字典序排列");
        for required in [
            "book_categories",
            "reading_history",
            "reader_theme_custom",
            "reader_background",
        ] {
            assert!(V11_TABLES.contains(&required), "V1.1 表清单缺少 {required}");
        }
        // 与 V1 冻结集不得重叠（V1.1 只新增，不改写既有表名）。
        for name in V11_TABLES {
            assert!(
                !FROZEN_TABLES.contains(name),
                "{name} 同时出现在 V1 冻结集与 V1.1 新增集"
            );
        }
    }

    #[test]
    fn expected_tables_merges_both_sets_without_duplicates() {
        let tables = expected_tables();
        assert_eq!(tables.len(), FROZEN_TABLES.len() + V11_TABLES.len());
        let mut sorted = tables.clone();
        sorted.sort_unstable();
        assert_eq!(tables, sorted, "expected_tables 必须按字典序返回");
    }

    #[test]
    fn in_memory_storage_migrates_and_keeps_version_consistent() {
        let mut storage = Storage::open_in_memory().expect("打开内存库");
        let outcome = storage.migrate().expect("初始迁移");
        assert!(outcome.changed);
        assert_eq!(outcome.from_version, 0);
        assert_eq!(outcome.to_version, migrate::TARGET_SCHEMA_VERSION);
        assert_eq!(storage.schema_version().expect("schema 版本"), 2);
        assert_eq!(storage.user_version().expect("user_version"), 2);
        assert!(storage.version_is_consistent().expect("版本一致性"));
        assert_eq!(storage.table_names().expect("表清单"), expected_tables());
    }

    #[test]
    fn migrating_an_existing_v1_database_upgrades_to_v11_without_losing_books() {
        use rusqlite::Connection;

        // 先只跑 0001，模拟既有 V1 库（含一本书）。
        let mut conn = Connection::open_in_memory().expect("内存库");
        migrate::migrate_with(
            &mut conn,
            None,
            &[migrate::Migration {
                version: 1,
                name: migrate::INITIAL_MIGRATION_NAME,
                sql: migrate::INITIAL_MIGRATION_SQL,
            }],
            &migrate::MigrateOptions::without_backup(),
        )
        .expect("V1 迁移");
        assert_eq!(meta::schema_version(&conn).expect("版本"), 1);
        conn.execute(
            "INSERT INTO books (id, display_name, original_name, format, source_kind,
                                source_locator, created_at, updated_at)
             VALUES (1, '甲', 'a.txt', 'txt', 'desktop_path', '/tmp/a.txt', 1, 1)",
            [],
        )
        .expect("插入书籍");
        let v1_tables = connection::table_names(&conn).expect("V1 表清单");
        assert_eq!(
            v1_tables,
            FROZEN_TABLES
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        );

        // 用完整序列升级。
        let mut storage = Storage::from_connection(conn, None);
        let outcome = storage.migrate().expect("升级到 V1.1");
        assert_eq!(outcome.from_version, 1);
        assert_eq!(outcome.to_version, 2);
        assert_eq!(
            outcome.applied,
            vec![(2, migrate::V11_MIGRATION_NAME.to_string())],
            "只应执行 0002 一条"
        );

        // 既有书籍仍在，且 category_id 为空（未分类）。
        let (name, category): (String, Option<i64>) = storage
            .conn()
            .query_row(
                "SELECT display_name, category_id FROM books WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("读回书籍");
        assert_eq!(name, "甲", "升级不得丢用户数据");
        assert_eq!(category, None, "存量书籍应落在未分类");
        assert_eq!(storage.table_names().expect("表清单"), expected_tables());

        // 幂等：再次迁移为空转。
        let again = storage.migrate().expect("重复迁移");
        assert!(again.is_noop(), "重复迁移必须为空转");
    }

    #[test]
    fn background_filename_constraint_rejects_paths_and_traversal() {
        let mut storage = Storage::open_in_memory().expect("内存库");
        storage.migrate().expect("迁移");
        let conn = storage.conn();
        for bad in ["../secret.png", "sub/dir/a.png", "sub\\dir\\a.png", ""] {
            let result = conn.execute(
                "INSERT INTO reader_background (id, file_name, file_size, updated_at)
                 VALUES (1, ?1, 1, 1)",
                [bad],
            );
            assert!(result.is_err(), "非法背景文件名 {bad:?} 应被数据库约束拒绝");
        }
        assert!(conn
            .execute(
                "INSERT INTO reader_background (id, file_name, file_size, updated_at)
                 VALUES (1, 'bg-1.png', 1, 1)",
                [],
            )
            .is_ok());
    }

    #[test]
    fn theme_custom_table_stores_only_base_colors_not_token_values() {
        let mut storage = Storage::open_in_memory().expect("内存库");
        storage.migrate().expect("迁移");
        let conn = storage.conn();
        assert!(conn
            .execute(
                "INSERT INTO reader_theme_custom
                    (id, enabled, base_foreground, base_background, base_primary, base_accent, updated_at)
                 VALUES (1, 1, '#222222', '#ffffff', '#1890ff', '#d4380d', 1)",
                [],
            )
            .is_ok());
        // 非 #RRGGBB 的基色被拒。
        assert!(conn
            .execute(
                "INSERT INTO reader_theme_custom
                    (id, enabled, base_foreground, updated_at)
                 VALUES (2, 1, 'red', 1)",
                [],
            )
            .is_err());
        // 列清单里不允许出现任何语义令牌列（40 令牌不落库，规格 R11）。
        let mut statement = conn
            .prepare("SELECT name FROM pragma_table_info('reader_theme_custom')")
            .expect("列清单");
        let columns: Vec<String> = statement
            .query_map([], |row| row.get(0))
            .expect("查询")
            .map(|row| row.expect("行"))
            .collect();
        assert_eq!(
            columns,
            vec![
                "id",
                "enabled",
                "base_foreground",
                "base_background",
                "base_primary",
                "base_accent",
                "updated_at"
            ]
        );
    }
}

//! RC6 迁移测试：初始迁移可重复执行、schema 冻结、迁移失败可恢复、异常中断回滚。
//!
//! 过滤器：`migration_recovery` 命中本文件的失败恢复用例（卡片必须验证命令之一）。

mod common;

use std::collections::HashMap;
use std::path::Path;

use common::{sample_import, sha256_hex, snapshot, write_sample_txt, TempDir};
use reading_storage::bookmarks::BookmarkInput;
use reading_storage::books::BookImport;
use reading_storage::error::StorageError;
use reading_storage::index::{ChapterInput, LayoutRecordInput};
use reading_storage::progress::ProgressInput;
use reading_storage::settings::{load_settings, save_settings, ReaderSettings};
use reading_storage::{bookmarks, books, connection, index, meta, migrate, progress, Storage};
use reading_storage::{validate, FROZEN_TABLES};

/// 故意失败的 v2 迁移：先建表并插入一行，再执行一条必然失败的语句。
/// 用于证明「整条迁移是一个事务：失败后 schema 与数据都回到迁移前状态」。
const BROKEN_V2_SQL: &str = "CREATE TABLE IF NOT EXISTS v2_probe (id INTEGER PRIMARY KEY);\n\
INSERT INTO v2_probe (id) VALUES (1);\n\
INSERT INTO table_that_does_not_exist_rc6 (id) VALUES (1);\n";

const BROKEN_V2: migrate::Migration = migrate::Migration {
    version: 2,
    name: "0002_broken",
    sql: BROKEN_V2_SQL,
};

fn frozen_column_expectations() -> HashMap<&'static str, Vec<&'static str>> {
    let mut expected = HashMap::new();
    expected.insert(
        "books",
        vec![
            "id",
            "display_name",
            "original_name",
            "format",
            "source_kind",
            "source_locator",
            "source_size",
            "source_modified_hint",
            "source_fingerprint",
            "encoding",
            "encoding_confidence",
            "encoding_user_override",
            "status",
            "created_at",
            "updated_at",
        ],
    );
    expected.insert(
        "book_sources",
        vec![
            "id",
            "book_id",
            "source_kind",
            "source_locator",
            "source_size",
            "source_modified_hint",
            "source_fingerprint",
            "encoding",
            "encoding_confidence",
            "encoding_user_override",
            "is_primary",
            "created_at",
            "updated_at",
        ],
    );
    expected.insert(
        "chapters",
        vec![
            "id",
            "book_id",
            "sequence",
            "title",
            "start_byte",
            "end_byte",
            "rule_id",
            "index_version",
            "created_at",
        ],
    );
    expected.insert(
        "reading_progress",
        vec![
            "book_id",
            "content_unit_id",
            "chapter_index",
            "byte_offset",
            "nearby_fingerprint",
            "last_page_hint",
            "index_version",
            "updated_at",
            "last_opened_at",
        ],
    );
    expected.insert(
        "bookmarks",
        vec![
            "id",
            "book_id",
            "content_unit_id",
            "chapter_index",
            "byte_offset",
            "nearby_fingerprint",
            "title",
            "created_at",
            "updated_at",
        ],
    );
    expected.insert(
        "reader_settings",
        vec![
            "id",
            "font_family",
            "font_weight",
            "font_size_milli",
            "line_spacing_milli",
            "paragraph_spacing_milli",
            "first_line_indent_milli",
            "margin_x_milli",
            "margin_y_milli",
            "theme",
            "operation_mode",
            "default_encoding",
            "updated_at",
        ],
    );
    expected.insert(
        "layout_records",
        vec![
            "id",
            "book_id",
            "layout_signature",
            "content_index_version",
            "generated_until_byte",
            "page_count",
            "status",
            "updated_at",
        ],
    );
    expected.insert("app_meta", vec!["key", "value", "updated_at"]);
    expected
}

fn seed_user_data(storage: &Storage, dir: &TempDir) -> i64 {
    let (path, bytes) = write_sample_txt(dir, "seed.txt");
    let locator = path.to_string_lossy().to_string();
    let import = sample_import(&locator, "样书-种子", &bytes);
    let book_id = books::import_book(storage.conn(), &import, 1_000)
        .expect("导入书籍")
        .book_id;

    let mut position = ProgressInput::at(book_id, 0, 32);
    position.nearby_fingerprint = Some("第一章 起始".to_string());
    position.last_page_hint = Some(1);
    progress::save_progress(storage.conn(), &position, 1_001).expect("保存进度");

    let bookmark = BookmarkInput::at(book_id, 0, 32, "开头");
    bookmarks::create_bookmark(storage.conn(), &bookmark, 1_002).expect("新增书签");

    let chapters = vec![
        ChapterInput {
            sequence: 0,
            title: "第一章 起始".to_string(),
            start_byte: 0,
            end_byte: 32,
            rule_id: "chapter-rule-1".to_string(),
        },
        ChapterInput {
            sequence: 1,
            title: "第二章 结束".to_string(),
            start_byte: 32,
            end_byte: bytes.len() as i64,
            rule_id: "chapter-rule-1".to_string(),
        },
    ];
    index::rebuild_chapters(storage.conn(), book_id, 1, &chapters, 1_003).expect("写入章节");
    index::upsert_layout_record(
        storage.conn(),
        &LayoutRecordInput::new(book_id, "nrlay1:seed", 1),
        1_004,
    )
    .expect("写入分页缓存");

    let settings = ReaderSettings {
        font_size_milli: 20_000,
        ..ReaderSettings::default()
    };
    save_settings(storage.conn(), &settings, 1_005).expect("保存设置");
    book_id
}

#[test]
fn initial_migration_is_repeatable_and_idempotent() {
    let dir = TempDir::new("migration-repeat");
    let db = dir.join("library.db");
    let mut storage = Storage::open(&db).expect("打开数据库");

    let first = storage.migrate().expect("首次迁移");
    assert!(first.changed, "首次迁移必须发生 schema 变更");
    assert_eq!(first.from_version, 0);
    assert_eq!(first.to_version, migrate::TARGET_SCHEMA_VERSION);
    assert_eq!(
        first.applied,
        vec![(1, migrate::INITIAL_MIGRATION_NAME.to_string())]
    );
    let first_backup = first.backup.clone().expect("首次迁移应留下备份");
    assert!(first_backup.is_file(), "备份文件必须真实存在");

    let book_id = seed_user_data(&storage, &dir);
    let before = snapshot(&storage);

    let second = storage.migrate().expect("重复迁移");
    assert!(second.is_noop(), "重复执行必须是空转");
    assert!(second.applied.is_empty());
    assert!(second.backup.is_none(), "空转不得再产生备份");
    assert_eq!(second.to_version, migrate::TARGET_SCHEMA_VERSION);

    let third = storage.migrate().expect("第三次迁移");
    assert!(third.is_noop(), "初始迁移必须可重复执行");

    assert_eq!(storage.schema_version().expect("schema 版本"), 1);
    assert_eq!(storage.user_version().expect("user_version"), 1);
    assert!(storage.version_is_consistent().expect("版本一致性"));
    assert_eq!(
        migrate::applied_migration_keys(storage.conn()).expect("迁移历史"),
        vec![meta::migration_history_key(migrate::INITIAL_MIGRATION_NAME)],
        "迁移历史不得重复写入"
    );

    let mut tables = storage.table_names().expect("表清单");
    tables.sort_unstable();
    assert_eq!(tables, FROZEN_TABLES.to_vec());

    let after = snapshot(&storage);
    after.assert_all_eq(&before, "重复迁移后");
    assert!(books::get_book(storage.conn(), book_id)
        .expect("读取书籍")
        .is_some());
}

#[test]
fn initial_migration_creates_frozen_schema_without_full_text_columns() {
    let dir = TempDir::new("migration-schema");
    let mut storage = Storage::open(dir.join("library.db")).expect("打开数据库");
    storage.migrate().expect("初始迁移");

    let mut tables = storage.table_names().expect("表清单");
    tables.sort_unstable();
    assert_eq!(
        tables,
        FROZEN_TABLES.to_vec(),
        "初始迁移必须恰好建立 8 张冻结表（不得多出第 9 张表）"
    );
    assert!(
        !tables.iter().any(|name| name == "sqlite_sequence"),
        "不得使用 AUTOINCREMENT（会引入 sqlite_sequence 表）"
    );

    for (table, expected) in frozen_column_expectations() {
        let actual = connection::column_names(storage.conn(), table).expect("列清单");
        assert_eq!(actual, expected, "{table} 列结构与冻结清单不一致");
        for column in &actual {
            let lowered = column.to_ascii_lowercase();
            assert!(
                !lowered.contains("content_text")
                    && !lowered.contains("body")
                    && !lowered.contains("fulltext")
                    && !lowered.contains("raw_text"),
                "{table}.{column} 出现疑似正文字段名"
            );
        }
    }

    let indexes = connection::index_names(storage.conn()).expect("索引清单");
    assert!(
        indexes.contains(&"ux_books_source".to_string()),
        "缺少来源唯一索引 ux_books_source（导入 upsert 依赖它）"
    );
    assert!(connection::foreign_keys_enabled(storage.conn()).expect("外键开关"));
    assert_eq!(
        connection::journal_mode(storage.conn()).expect("日志模式"),
        "wal"
    );
    assert_eq!(
        migrate::frozen_tables_present(storage.conn())
            .expect("冻结表检查")
            .len(),
        8
    );
}

#[test]
fn embedded_migration_matches_on_disk_file() {
    let on_disk = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations/0001_init.sql"),
    )
    .expect("读取 migrations/0001_init.sql");
    assert_eq!(
        on_disk,
        migrate::INITIAL_MIGRATION_SQL,
        "内嵌迁移脚本与 migrations/0001_init.sql 必须逐字节一致"
    );
    assert_eq!(
        migrate::INITIAL_MIGRATION_RELATIVE_PATH,
        "migrations/0001_init.sql"
    );
    assert_eq!(migrate::TARGET_SCHEMA_VERSION, 1);
    assert_eq!(migrate::MIGRATIONS.len(), 1);
}

#[test]
fn migration_recovery_restores_previous_state_after_failed_migration() {
    let dir = TempDir::new("migration-recovery");
    let db = dir.join("library.db");
    let mut storage = Storage::open(&db).expect("打开数据库");
    storage.migrate().expect("初始迁移");
    let book_id = seed_user_data(&storage, &dir);
    let before = snapshot(&storage);

    let sequence = [migrate::MIGRATIONS[0], BROKEN_V2];
    let error = storage
        .migrate_with(&sequence, &migrate::MigrateOptions::with_backup())
        .expect_err("失败的 v2 迁移必须返回错误");
    let (version, backup) = match &error {
        StorageError::MigrationFailed {
            version, backup, ..
        } => (*version, backup.clone()),
        other => panic!("错误类型必须是 MigrationFailed，实际 {other:?}"),
    };
    assert_eq!(version, 2);
    assert_eq!(error.code(), reading_storage::error::ERR_INTERNAL);
    assert!(error.is_recoverable(), "带备份的迁移失败必须是可恢复错误");
    let backup = backup.expect("必须留下迁移前备份");
    assert!(backup.is_file(), "备份文件必须真实存在");

    // 断言 1：失败迁移整体回滚，schema 与数据停在迁移前状态。
    assert_eq!(storage.schema_version().expect("schema 版本"), 1);
    assert_eq!(storage.user_version().expect("user_version"), 1);
    assert!(
        migrate::table_absent(storage.conn(), "v2_probe").expect("半成品表检查"),
        "失败迁移中的 DDL 必须被回滚，不得留下半成品表"
    );
    assert_eq!(
        migrate::applied_migration_keys(storage.conn()).expect("迁移历史"),
        vec![meta::migration_history_key(migrate::INITIAL_MIGRATION_NAME)],
        "失败迁移不得写入迁移历史"
    );
    let after_failure = snapshot(&storage);
    after_failure.assert_all_eq(&before, "迁移失败后");

    // 断言 2：备份是一个完整可用的 v1 数据库，用户数据齐备。
    let backup_storage = Storage::open(&backup).expect("打开备份数据库");
    assert_eq!(backup_storage.schema_version().expect("备份版本"), 1);
    assert!(backup_storage
        .version_is_consistent()
        .expect("备份版本一致性"));
    let backup_snapshot = snapshot(&backup_storage);
    backup_snapshot.assert_all_eq(&before, "备份内容");
    assert!(books::get_book(backup_storage.conn(), book_id)
        .expect("备份中的书籍")
        .is_some());
    assert_eq!(
        progress::count_progress(backup_storage.conn()).expect("备份进度行数"),
        1
    );
    assert_eq!(
        bookmarks::count_all_bookmarks(backup_storage.conn()).expect("备份书签行数"),
        1
    );
    drop(backup_storage);

    // 断言 3：可从备份恢复，恢复后用户数据与失败前完全一致。
    storage
        .restore_from_backup(&backup)
        .expect("从备份恢复数据库");
    assert_eq!(storage.schema_version().expect("恢复后版本"), 1);
    let after_restore = snapshot(&storage);
    after_restore.assert_all_eq(&before, "从备份恢复后");
    let restored = load_settings(storage.conn()).expect("恢复后设置");
    assert_eq!(restored.font_size_milli, 20_000);
}

#[test]
fn migration_recovery_rolls_back_interrupted_ddl_on_fresh_database() {
    let dir = TempDir::new("migration-interrupt");
    let db = dir.join("fresh.db");
    let mut storage = Storage::open(&db).expect("打开数据库");
    assert_eq!(
        migrate::current_schema_version(storage.conn()).expect("初始版本"),
        0
    );

    let error = storage
        .migrate_with(
            &[migrate::MIGRATIONS[0], BROKEN_V2],
            &migrate::MigrateOptions::with_backup(),
        )
        .expect_err("含失败迁移的序列必须返回错误");
    assert!(error.is_recoverable());

    // 0001 在独立事务中已提交（可继续使用），0002 的事务整体回滚。
    assert_eq!(storage.schema_version().expect("schema 版本"), 1);
    assert_eq!(storage.user_version().expect("user_version"), 1);
    assert!(migrate::table_absent(storage.conn(), "v2_probe").expect("半成品表检查"));
    let mut tables = storage.table_names().expect("表清单");
    tables.sort_unstable();
    assert_eq!(tables, FROZEN_TABLES.to_vec());
    assert_eq!(
        migrate::applied_migration_keys(storage.conn()).expect("迁移历史"),
        vec![meta::migration_history_key(migrate::INITIAL_MIGRATION_NAME)]
    );

    // 数据库仍可正常写入用户数据（未被半成品 schema 污染）。
    let (path, bytes) = write_sample_txt(&dir, "after-interrupt.txt");
    let import = sample_import(&path.to_string_lossy(), "中断后样书", &bytes);
    let outcome = books::import_book(storage.conn(), &import, 2_000).expect("中断后仍可导入");
    assert!(outcome.created);
}

#[test]
fn migration_options_can_disable_backup_and_migration_sequence_is_validated() {
    let dir = TempDir::new("migration-options");
    let db = dir.join("plain.db");
    let mut storage = Storage::open(&db).expect("打开数据库");
    let outcome = storage
        .migrate_with(
            migrate::MIGRATIONS,
            &migrate::MigrateOptions::without_backup(),
        )
        .expect("无备份迁移");
    assert!(outcome.changed);
    assert!(outcome.backup.is_none(), "关闭备份时不得产生备份文件");
    let leftovers: Vec<String> = std::fs::read_dir(dir.path())
        .expect("读取目录")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name.contains(".backup-v"))
        .collect();
    assert!(leftovers.is_empty(), "不应存在备份文件: {leftovers:?}");

    // 版本乱序 / 重复必须在执行前被拒绝。
    let mut duplicate = Vec::from(migrate::MIGRATIONS);
    duplicate.push(migrate::Migration {
        version: 1,
        name: "0001_duplicate",
        sql: "SELECT 1;",
    });
    let error = migrate::validate_migrations(&duplicate).expect_err("重复版本必须被拒绝");
    assert_eq!(error.code(), reading_storage::error::ERR_INVALID_ARGUMENT);
    assert_eq!(
        migrate::plan_with(storage.conn(), &duplicate)
            .expect_err("乱序序列不得生成计划")
            .code(),
        reading_storage::error::ERR_INVALID_ARGUMENT
    );
}

#[test]
fn migration_and_validation_helpers_guard_user_files() {
    let dir = TempDir::new("migration-helpers");
    let db = dir.join("guard.db");
    let mut storage = Storage::open(&db).expect("打开数据库");
    storage.migrate().expect("初始迁移");

    // 迁移前自检 / 备份路径 / 边车文件路径。
    let backup = migrate::backup_file_path(&db, 1, 42, None);
    assert!(backup.to_string_lossy().contains("backup-v1-42"));
    assert_eq!(
        migrate::sidecar(&db, "-wal"),
        db.with_file_name(format!("{}-wal", db.file_name().unwrap().to_string_lossy()))
    );
    assert!(migrate::database_file_exists(&db));
    assert_eq!(
        migrate::plan(storage.conn()).expect("计划").pending.len(),
        0
    );

    // 表名白名单：非法标识符必须被拒绝（防 SQL 拼接）。
    assert_eq!(
        validate::ensure_identifier("books; DROP TABLE books")
            .expect_err("非法标识符必须被拒绝")
            .code(),
        reading_storage::error::ERR_INVALID_ARGUMENT
    );

    // 导入请求中的超长短摘要（正文）必须被拒绝。
    let import = BookImport {
        display_name: "甲".repeat(validate::MAX_NAME_BYTES + 1),
        original_name: "x.txt".to_string(),
        source_kind: books::KIND_DESKTOP_PATH.to_string(),
        source_locator: "/tmp/x.txt".to_string(),
        source_size: 1,
        source_modified_hint: None,
        source_fingerprint: None,
        encoding: "utf-8".to_string(),
        encoding_confidence: 0,
        encoding_user_override: false,
    };
    assert_eq!(
        books::import_book(storage.conn(), &import, 1)
            .expect_err("超长显示名必须被拒绝")
            .code(),
        reading_storage::error::ERR_INVALID_ARGUMENT
    );

    // 文件哈希辅助函数本身的健全性（用于源文件不变断言）。
    assert_eq!(sha256_hex(b"").len(), 64);
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

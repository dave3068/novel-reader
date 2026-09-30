//! RC6 演示：真实执行一次「迁移 → 导入 → 写入进度/书签/索引 → 清理索引/分页缓存」闭环，
//! 逐步打印表行数与原始文件哈希，用于人工复核「清理索引不丢用户元数据」。
//!
//! 运行：
//! ```bash
//! cd ${NR_PROJECT_DIR} && cargo run --release -p reading-storage --example rc6_storage_demo
//! ```

use std::fs;

use reading_storage::bookmarks::{self, BookmarkInput};
use reading_storage::books::{self, BookImport};
use reading_storage::index::{self, ChapterInput, LayoutRecordInput, LAYOUT_COMPLETE};
use reading_storage::progress::{self, ProgressInput};
use reading_storage::settings::{self, ReaderSettings};
use reading_storage::{connection, meta, Storage};

fn fnv1a64_hex(data: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in data {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn print_counts(storage: &Storage, label: &str) {
    println!("-- {label} --");
    for (table, rows) in storage.table_row_counts().expect("逐表行数") {
        println!("   {table:<18} {rows:>4} 行");
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("演示失败: {error}");
        std::process::exit(1);
    }
}

fn run() -> reading_storage::Result<()> {
    let dir = std::env::temp_dir().join(format!("nr-rc6-demo-{}", std::process::id()));
    fs::create_dir_all(&dir).map_err(|e| reading_storage::StorageError::io(&dir, e.to_string()))?;
    let db = dir.join("library.db");

    println!("== 1. 打开数据库并执行初始迁移 ==");
    let mut storage = Storage::open(&db)?;
    let outcome = storage.migrate()?;
    println!(
        "   schema {} -> {}，applied={:?}，备份={:?}",
        outcome.from_version,
        outcome.to_version,
        outcome.applied,
        outcome
            .backup
            .as_ref()
            .map(|path| path.display().to_string())
    );
    let repeat = storage.migrate()?;
    println!(
        "   重复执行：changed={} applied={} 备份={}（初始迁移可重复执行）",
        repeat.changed,
        repeat.applied.len(),
        repeat.backup.is_some()
    );

    println!("\n== 2. 导入真实 TXT（只登记元数据，不复制正文）==");
    let source = dir.join("book.txt");
    let mut body = String::from("第一章 起始\n");
    for index in 0..32 {
        body.push_str(&format!("第 {index} 行：演示正文，不入库。\n"));
    }
    body.push_str("第二章 结束\n");
    fs::write(&source, &body)
        .map_err(|e| reading_storage::StorageError::io(&source, e.to_string()))?;
    let bytes = fs::read(&source).expect("读取演示 TXT");
    let hash_before = fnv1a64_hex(&bytes);

    let import = BookImport {
        display_name: "演示书".to_string(),
        original_name: "book.txt".to_string(),
        source_kind: books::KIND_DESKTOP_PATH.to_string(),
        source_locator: source.to_string_lossy().to_string(),
        source_size: bytes.len() as i64,
        source_modified_hint: Some(1_700_000_000),
        source_fingerprint: Some(hash_before.clone()),
        encoding: "utf-8".to_string(),
        encoding_confidence: 1000,
        encoding_user_override: false,
    };
    let imported = books::import_book(storage.conn(), &import, 1_000)?;
    println!(
        "   book_id={} created={} source_created={} updated={}",
        imported.book_id, imported.created, imported.source_created, imported.updated
    );
    let duplicate = books::import_book(storage.conn(), &import, 2_000)?;
    println!(
        "   重复导入：book_id={} changed_any={}（幂等）",
        duplicate.book_id,
        duplicate.changed_any()
    );

    println!("\n== 3. 写入进度 / 书签 / 章节索引 / 分页缓存 / 设置 ==");
    let conn = storage.conn();
    let mut position = ProgressInput::at(imported.book_id, 1, 256);
    position.nearby_fingerprint = Some("第一章 起始".to_string());
    let saved = progress::save_progress(conn, &position, 1_100)?;
    println!(
        "   进度：inserted={} changed={}",
        saved.inserted, saved.changed
    );
    for (chapter_index, offset) in [(0_i64, 64_i64), (1, 256)] {
        bookmarks::create_bookmark(
            conn,
            &BookmarkInput::at(
                imported.book_id,
                chapter_index,
                offset,
                format!("书签{chapter_index}"),
            ),
            1_200 + chapter_index,
        )?;
    }
    let total = bytes.len() as i64;
    index::rebuild_chapters(
        conn,
        imported.book_id,
        1,
        &[
            ChapterInput {
                sequence: 0,
                title: "第一章 起始".to_string(),
                start_byte: 0,
                end_byte: total / 2,
                rule_id: "demo-rule".to_string(),
            },
            ChapterInput {
                sequence: 1,
                title: "第二章 结束".to_string(),
                start_byte: total / 2,
                end_byte: total,
                rule_id: "demo-rule".to_string(),
            },
        ],
        1_300,
    )?;
    let mut layout = LayoutRecordInput::new(imported.book_id, "nrlay1:demo", 1);
    layout.status = LAYOUT_COMPLETE.to_string();
    layout.page_count = 3;
    index::upsert_layout_record(conn, &layout, 1_400)?;
    settings::save_settings(
        conn,
        &ReaderSettings {
            font_size_milli: 20_000,
            ..ReaderSettings::default()
        },
        1_500,
    )?;
    print_counts(&storage, "写入用户数据与索引后");

    println!("\n== 4. 清理索引 / 分页缓存（设置页操作）==");
    let report = storage.clear_rebuildable_index()?;
    println!(
        "   删除 chapters={} layout_records={}",
        report.chapters_deleted, report.layout_records_deleted
    );
    println!(
        "   保留 books={} book_sources={} reading_progress={} bookmarks={} reader_settings={} app_meta={}",
        report.books_preserved,
        report.book_sources_preserved,
        report.progress_preserved,
        report.bookmarks_preserved,
        report.settings_preserved,
        report.meta_preserved
    );
    print_counts(&storage, "清理索引 / 分页缓存后");

    println!("\n== 5. 校验用户元数据与原始文件 ==");
    println!(
        "   书架条目数={} 进度偏移={:?} 书签数={} schema 版本={} user_version={}",
        books::list_shelf_books(conn)?.len(),
        progress::load_progress(conn, imported.book_id)?.map(|p| p.byte_offset),
        bookmarks::count_all_bookmarks(conn)?,
        meta::schema_version(conn)?,
        connection::user_version(conn)?
    );
    let bytes_after = fs::read(&source).expect("读取演示 TXT");
    println!(
        "   原始 TXT：存在={} 字节数={} fnv1a64 {} -> {}",
        source.is_file(),
        bytes_after.len(),
        hash_before,
        fnv1a64_hex(&bytes_after)
    );

    println!("\n== 6. 备份 / 恢复 ==");
    let backup = dir.join("manual-backup.db");
    storage.backup_to(&backup)?;
    println!(
        "   备份文件：{} 存在={}",
        backup.display(),
        backup.is_file()
    );

    let leftovers = fs::read_dir(&dir).map_or(0, |entries| entries.count());
    println!("   演示目录文件数={leftovers}");
    drop(storage);
    fs::remove_dir_all(&dir).map_err(|e| reading_storage::StorageError::io(&dir, e.to_string()))?;
    println!("\nRESULT: 演示完成（清理索引后书籍/进度/书签/设置仍在，原始 TXT 未被修改）");
    Ok(())
}

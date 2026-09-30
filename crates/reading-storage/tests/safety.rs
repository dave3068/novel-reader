//! RC6 安全测试：清理索引/分页缓存不得丢用户元数据，任何操作都不得删除用户原始 TXT，
//! 数据库不得保存正文。
//!
//! 过滤器：`index_clear_keeps_user_metadata` 命中本文件的清理安全用例（卡片必须验证命令之一）。

mod common;

use std::path::PathBuf;

use common::{sample_import, seed_book, sha256_hex, snapshot, TempDir};
use reading_storage::bookmarks::{self, BookmarkInput};
use reading_storage::books;
use reading_storage::error::ERR_BOOK_NOT_FOUND;
use reading_storage::index::{self, ChapterInput, LayoutRecordInput, LAYOUT_COMPLETE};
use reading_storage::progress::{self, ProgressInput};
use reading_storage::settings;
use reading_storage::{connection, meta, migrate, Storage};

/// 构造一份「完整用户状态」：书籍 + 来源 + 进度 + 3 个书签 + 章节索引 + 分页缓存 + 设置。
fn seed_full_state(storage: &Storage, dir: &TempDir) -> (i64, PathBuf, Vec<u8>) {
    let (book_id, path, bytes) = seed_book(storage, dir, "来源.txt", "SentinelBook");
    let conn = storage.conn();

    let mut position = ProgressInput::at(book_id, 1, 512);
    position.nearby_fingerprint = Some("第一章 起始".to_string());
    position.last_page_hint = Some(3);
    progress::save_progress(conn, &position, 1_100).expect("保存进度");

    for (chapter_index, offset) in [(0_i64, 64_i64), (1, 256), (2, 512)] {
        let bookmark = BookmarkInput::at(
            book_id,
            chapter_index,
            offset,
            format!("书签 {chapter_index}"),
        );
        bookmarks::create_bookmark(conn, &bookmark, 1_200 + chapter_index).expect("新增书签");
    }

    let total = bytes.len() as i64;
    let chapters: Vec<ChapterInput> = (0..4)
        .map(|index| ChapterInput {
            sequence: index,
            title: format!("第 {} 章", index + 1),
            start_byte: index * (total / 4),
            end_byte: (index + 1) * (total / 4),
            rule_id: "chapter-rule-rc6".to_string(),
        })
        .collect();
    index::rebuild_chapters(conn, book_id, 1, &chapters, 1_300).expect("写入章节索引");

    let mut layout = LayoutRecordInput::new(book_id, "nrlay1:rc6safe", 1);
    layout.generated_until_byte = 4_096;
    layout.page_count = 12;
    layout.status = LAYOUT_COMPLETE.to_string();
    index::upsert_layout_record(conn, &layout, 1_400).expect("写入分页缓存");

    let mut value = settings::load_settings(conn).expect("读取设置");
    value.font_size_milli = 21_000;
    value.updated_at = 1_500;
    settings::save_settings(conn, &value, 1_500).expect("保存设置");

    (book_id, path, bytes)
}

fn find(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && needle.len() <= haystack.len()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

#[test]
fn index_clear_keeps_user_metadata_and_original_txt_intact() {
    let dir = TempDir::new("safety-index-clear");
    let db = dir.join("library.db");
    let mut storage = Storage::open(&db).expect("打开数据库");
    storage.migrate().expect("初始迁移");
    let (book_id, path, bytes) = seed_full_state(&storage, &dir);
    let conn = storage.conn();

    let source_hash_before = sha256_hex(&std::fs::read(&path).expect("读取原始 TXT"));
    let before = snapshot(&storage);
    assert!(
        !before.chapters.is_empty() && !before.layouts.is_empty(),
        "清理前必须存在可重建索引（防空检查）"
    );
    assert_eq!(before.progress.len(), 1);
    assert_eq!(before.bookmarks.len(), 3);

    let report = storage
        .clear_rebuildable_index()
        .expect("清理索引/分页缓存");
    assert_eq!(report.chapters_deleted, before.chapters.len() as i64);
    assert_eq!(report.layout_records_deleted, before.layouts.len() as i64);
    assert_eq!(report.books_preserved, 1);
    assert_eq!(report.book_sources_preserved, 1);
    assert_eq!(report.progress_preserved, 1);
    assert_eq!(report.bookmarks_preserved, 3);
    assert_eq!(report.settings_preserved, 1);

    // 独立复核（不采信报告）：可重建数据清零，用户元数据仍在。
    assert_eq!(index::count_chapters(conn, book_id).expect("章节数"), 0);
    assert_eq!(
        index::count_layout_records(conn, book_id).expect("缓存行数"),
        0
    );
    assert_eq!(connection::count_rows(conn, "books").expect("书籍行数"), 1);
    assert_eq!(
        connection::count_rows(conn, "book_sources").expect("来源行数"),
        1
    );
    assert_eq!(progress::count_progress(conn).expect("进度行数"), 1);
    assert_eq!(bookmarks::count_all_bookmarks(conn).expect("书签行数"), 3);
    assert_eq!(settings::settings_row_count(conn).expect("设置行数"), 1);
    assert_eq!(meta::schema_version(conn).expect("schema 版本"), 1);
    assert_eq!(connection::table_names(conn).expect("表清单").len(), 8);

    // 用户元数据逐字段一致（书籍、进度、书签、设置）。
    let after = snapshot(&storage);
    after.assert_user_metadata_eq(&before, "清理索引后");
    let progress_after = progress::load_progress(conn, book_id)
        .expect("读取进度")
        .expect("进度仍在");
    assert_eq!(progress_after.byte_offset, 512);
    assert_eq!(
        progress_after.nearby_fingerprint.as_deref(),
        Some("第一章 起始")
    );
    assert_eq!(progress_after.last_page_hint, Some(3));
    assert_eq!(
        bookmarks::list_bookmarks(conn, book_id)
            .expect("书签清单")
            .iter()
            .map(|bookmark| bookmark.title.clone())
            .collect::<Vec<_>>(),
        vec![
            "书签 0".to_string(),
            "书签 1".to_string(),
            "书签 2".to_string()
        ]
    );
    assert_eq!(books::list_shelf_books(conn).expect("书架").len(), 1);

    // 用户原始 TXT：未被删除、未被修改（逐字节 + sha256）。
    assert!(path.is_file(), "清理索引不得删除用户原始 TXT");
    let after_bytes = std::fs::read(&path).expect("重新读取原始 TXT");
    assert_eq!(after_bytes, bytes, "原始 TXT 字节内容不得变化");
    assert_eq!(
        sha256_hex(&after_bytes),
        source_hash_before,
        "原始 TXT 的 sha256 不得变化"
    );
}

#[test]
fn index_clear_keeps_user_metadata_for_removed_shelf_entries() {
    let dir = TempDir::new("safety-shelf");
    let db = dir.join("library.db");
    let mut storage = Storage::open(&db).expect("打开数据库");
    storage.migrate().expect("初始迁移");
    let (book_id, path, bytes) = seed_full_state(&storage, &dir);
    let conn = storage.conn();
    let locator = path.to_string_lossy().to_string();
    let source_hash_before = sha256_hex(&std::fs::read(&path).expect("读取原始 TXT"));
    let before = snapshot(&storage);

    // 移除书架记录（软移除）：只清可重建索引，进度/书签/来源保留。
    let removal = books::remove_book_from_shelf(conn, book_id, 2_000).expect("移除书架记录");
    assert_eq!(removal.status, books::STATUS_REMOVED);
    assert_eq!(removal.chapters_deleted, before.chapters.len() as i64);
    assert_eq!(removal.layout_records_deleted, before.layouts.len() as i64);
    assert!(removal.progress_preserved);
    assert_eq!(removal.bookmarks_preserved, 3);
    assert_eq!(removal.sources_preserved, 1);
    assert_eq!(removal.visible_shelf_rows, 0);
    assert!(books::list_shelf_books(conn).expect("书架").is_empty());
    assert_eq!(books::count_books(conn).expect("书籍行数"), 1);

    // 再执行一次全局「清理索引/分页缓存」：用户元数据仍然完整。
    let clear = storage.clear_rebuildable_index().expect("清理索引");
    assert_eq!(clear.books_preserved, 1);
    assert_eq!(clear.progress_preserved, 1);
    assert_eq!(clear.bookmarks_preserved, 3);
    assert!(
        progress::load_progress(conn, book_id)
            .expect("读取进度")
            .expect("进度仍在")
            .byte_offset
            == 512
    );
    assert_eq!(
        bookmarks::count_bookmarks(conn, book_id).expect("书签数"),
        3
    );
    let after = snapshot(&storage);
    // 软移除只改变书架状态：进度、书签、设置逐字段不变；书籍行本身仍在（status=removed）。
    assert_eq!(after.progress, before.progress, "软移除后进度必须保留");
    assert_eq!(after.bookmarks, before.bookmarks, "软移除后书签必须保留");
    assert_eq!(after.settings, before.settings, "软移除后设置必须保留");
    assert_eq!(after.books.len(), 1, "书籍记录不得被删除");
    let (kept, original) = (&after.books[0], &before.books[0]);
    assert_eq!(kept.id, original.id);
    assert_eq!(kept.source_locator, original.source_locator);
    assert_eq!(kept.source_fingerprint, original.source_fingerprint);
    assert_eq!(kept.source_size, original.source_size);
    assert_eq!(kept.status, books::STATUS_REMOVED);
    assert!(after.chapters.is_empty() && after.layouts.is_empty());
    assert_eq!(
        sha256_hex(&std::fs::read(&path).expect("读取原始 TXT")),
        source_hash_before
    );

    // 重新导入同一来源 ⇒ 回到书架，进度与书签自动恢复。
    let reimport = sample_import(&locator, "SentinelBook", &bytes);
    let outcome = books::import_book(conn, &reimport, 3_000).expect("重新导入");
    assert_eq!(outcome.book_id, book_id);
    assert!(!outcome.created, "重新导入不得新建条目");
    let restored = books::get_book(conn, book_id)
        .expect("读取书籍")
        .expect("书籍存在");
    assert_eq!(restored.status, books::STATUS_READY);
    assert!(restored.is_on_shelf());
    assert_eq!(books::list_shelf_books(conn).expect("书架").len(), 1);
    assert_eq!(
        bookmarks::count_bookmarks(conn, book_id).expect("书签数"),
        3
    );
    assert_eq!(
        progress::load_progress(conn, book_id)
            .expect("读取进度")
            .expect("进度仍在")
            .byte_offset,
        512
    );
}

#[test]
fn shelf_removal_and_purge_never_touch_original_txt() {
    let dir = TempDir::new("safety-purge");
    let db = dir.join("library.db");
    let mut storage = Storage::open(&db).expect("打开数据库");
    storage.migrate().expect("初始迁移");
    let (book_id, path, bytes) = seed_full_state(&storage, &dir);
    let conn = storage.conn();
    let locator = path.to_string_lossy().to_string();
    let hash_before = sha256_hex(&std::fs::read(&path).expect("读取原始 TXT"));

    books::remove_book_from_shelf(conn, book_id, 2_000).expect("移除书架记录");
    assert!(path.is_file(), "移除书架记录不得删除原始 TXT");
    assert_eq!(
        sha256_hex(&std::fs::read(&path).expect("读取原始 TXT")),
        hash_before
    );

    books::purge_book_record(conn, book_id).expect("硬删除数据库记录");
    assert!(path.is_file(), "硬删除数据库记录不得删除原始 TXT");
    assert_eq!(
        sha256_hex(&std::fs::read(&path).expect("读取原始 TXT")),
        hash_before
    );
    assert_eq!(std::fs::read(&path).expect("读取原始 TXT"), bytes);

    // 数据库侧：书籍、来源、章节、缓存、进度、书签随级联清理；全局设置与元信息保留。
    assert_eq!(books::count_books(conn).expect("书籍行数"), 0);
    assert_eq!(
        connection::count_rows(conn, "book_sources").expect("来源行数"),
        0
    );
    assert_eq!(
        connection::count_rows(conn, "chapters").expect("章节行数"),
        0
    );
    assert_eq!(
        connection::count_rows(conn, "layout_records").expect("缓存行数"),
        0
    );
    assert_eq!(progress::count_progress(conn).expect("进度行数"), 0);
    assert_eq!(bookmarks::count_all_bookmarks(conn).expect("书签行数"), 0);
    assert_eq!(settings::settings_row_count(conn).expect("设置行数"), 1);
    assert_eq!(meta::schema_version(conn).expect("schema 版本"), 1);
    assert_eq!(
        books::purge_book_record(conn, book_id)
            .expect_err("重复硬删除必须报错")
            .code(),
        ERR_BOOK_NOT_FOUND
    );

    // 原始文件仍可被重新导入（file still readable → 完整闭环）。
    let reimport = sample_import(&locator, "SentinelBook", &bytes);
    let outcome = books::import_book(conn, &reimport, 4_000).expect("重新导入原始文件");
    assert!(outcome.created, "硬删除后重新导入应新建条目");
    assert_eq!(books::count_books(conn).expect("书籍行数"), 1);
    assert_eq!(
        sha256_hex(&std::fs::read(&path).expect("读取原始 TXT")),
        hash_before
    );
}

#[test]
fn database_never_stores_full_text_of_source() {
    let dir = TempDir::new("safety-no-fulltext");
    let db = dir.join("library.db");
    let mut storage = Storage::open(&db).expect("打开数据库");
    storage.migrate().expect("初始迁移");
    let (_book_id, _path, bytes) = seed_full_state(&storage, &dir);

    let source_text = String::from_utf8(bytes).expect("夹具必须是 UTF-8");
    let sentinel = "正文不应写入数据库";
    assert!(
        source_text.contains(sentinel),
        "夹具正文必须包含哨兵串（防空检查）"
    );

    // 数据库 + WAL 边车文件的原始字节。
    let mut blob = std::fs::read(&db).expect("读取数据库文件");
    if let Ok(wal) = std::fs::read(migrate::sidecar(&db, "-wal")) {
        blob.extend_from_slice(&wal);
    }
    let mut shm = Vec::new();
    if let Ok(bytes) = std::fs::read(migrate::sidecar(&db, "-shm")) {
        shm = bytes;
    }

    // 正向对照：已落库的元数据（书架显示名）必须能在数据库字节中命中 ⇒ 证明扫描方法有效。
    assert!(
        find(&blob, b"SentinelBook"),
        "元数据未出现在数据库字节中，说明本检查不是有效检查"
    );

    // 正文哨兵与整行正文都不得出现在数据库（含 WAL）里。
    assert!(
        !find(&blob, sentinel.as_bytes()),
        "数据库中出现正文内容：{sentinel}"
    );
    for line in source_text
        .split('\n')
        .filter(|line| line.len() >= 24)
        .take(64)
    {
        assert!(!find(&blob, line.as_bytes()), "数据库中出现正文行: {line}");
    }
    assert!(
        !find(&shm, sentinel.as_bytes()),
        "WAL 共享内存文件中出现正文内容"
    );
}

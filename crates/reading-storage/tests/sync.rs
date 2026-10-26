//! 阶段 0 同步包：导出/导入/合并/幂等/原文件只读/路径安全。

mod common;

use std::fs;

use common::{seed_book, sha256_hex, TempDir};
use reading_storage::bookmarks::{self, BookmarkInput};
use reading_storage::progress::{self, ProgressInput};
use reading_storage::settings::{self, ReaderSettings};
use reading_storage::sync::{self, ExportOptions, ImportOptions};
use reading_storage::{books, Storage};

fn open_db(path: &std::path::Path) -> Storage {
    let mut storage = Storage::open(path).expect("打开数据库");
    storage.migrate().expect("迁移");
    storage
}

fn seed_device_a(root: &TempDir) -> (Storage, std::path::PathBuf, std::path::PathBuf, Vec<u8>) {
    let db = root.join("a/library.db");
    let storage = open_db(&db);
    let (book_id, source, bytes) = seed_book(&storage, root, "同步样书.txt", "同步样书");

    let mut position = ProgressInput::at(book_id, 1, 64);
    position.nearby_fingerprint = Some("第二章 起始".to_string());
    progress::save_progress(storage.conn(), &position, 2_000).expect("保存进度");

    let bookmark = BookmarkInput::at(book_id, 1, 64, "读到第二章");
    bookmarks::create_bookmark(storage.conn(), &bookmark, 2_001).expect("创建书签");

    let next_settings = ReaderSettings {
        font_size_milli: 21_000,
        theme: "dark".to_string(),
        ..ReaderSettings::default()
    };
    settings::save_settings(storage.conn(), &next_settings, 2_002).expect("保存设置");
    (storage, db, source, bytes)
}

#[test]
fn export_import_roundtrip_preserves_progress_bookmarks_settings_and_source() {
    let root = TempDir::new("sync-roundtrip");
    let (storage_a, db_a, source, source_bytes) = seed_device_a(&root);
    let source_hash_before = sha256_hex(&source_bytes);

    let bundle = root.join("bundle");
    let mut store_a = sync::SyncStore::open(Some(&db_a)).expect("打开 A 边车");
    let exported = sync::export_bundle(
        &mut store_a,
        storage_a.conn(),
        &bundle,
        &ExportOptions::default(),
    )
    .expect("导出");
    assert_eq!(exported.books_exported, 1);
    assert_eq!(exported.book_files_copied, 1);
    assert_eq!(exported.progress_exported, 1);
    assert_eq!(exported.bookmarks_exported, 1);
    assert!(bundle.join("state.json").is_file());
    let bundle_books: Vec<_> = fs::read_dir(bundle.join("books"))
        .expect("列同步包书籍")
        .collect::<Result<_, _>>()
        .expect("目录项");
    assert_eq!(bundle_books.len(), 1);
    let bundle_file = bundle_books[0].path();
    assert_eq!(
        sha256_hex(&fs::read(&bundle_file).expect("读书籍包")),
        source_hash_before
    );

    // 第二台设备：空库导入。
    let db_b = root.join("b/library.db");
    let storage_b = open_db(&db_b);
    let mut store_b = sync::SyncStore::open(Some(&db_b)).expect("打开 B 边车");
    let imported = sync::import_bundle(
        &mut store_b,
        storage_b.conn(),
        &bundle,
        &ImportOptions::default(),
    )
    .expect("导入");

    assert_eq!(imported.books_created, 1);
    assert_eq!(imported.progress_applied, 1);
    assert_eq!(imported.bookmarks_created, 1);
    assert!(imported.settings_applied);
    assert!(imported.conflicts.is_empty(), "{:?}", imported.conflicts);

    let shelf = books::list_shelf_books(storage_b.conn()).expect("B 书架");
    assert_eq!(shelf.len(), 1);
    assert_eq!(shelf[0].display_name, "同步样书");
    assert_eq!(shelf[0].source_kind, books::KIND_PRIVATE_COPY);
    assert_eq!(shelf[0].status, books::STATUS_READY);
    let imported_file = std::path::PathBuf::from(&shelf[0].source_locator);
    assert!(imported_file.is_file(), "导入的书籍副本必须真实存在");
    assert_eq!(
        sha256_hex(&fs::read(&imported_file).expect("读导入副本")),
        source_hash_before
    );

    let progress = progress::load_progress(storage_b.conn(), shelf[0].id)
        .expect("读进度")
        .expect("进度存在");
    assert_eq!(progress.byte_offset, 64);
    assert_eq!(progress.chapter_index, 1);
    assert_eq!(progress.nearby_fingerprint.as_deref(), Some("第二章 起始"));

    let marks = bookmarks::list_bookmarks(storage_b.conn(), shelf[0].id).expect("B 书签");
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].title, "读到第二章");

    let loaded = settings::load_settings(storage_b.conn()).expect("B 设置");
    assert_eq!(loaded.font_size_milli, 21_000);
    assert_eq!(loaded.theme, "dark");

    // 原始 TXT 必须逐字节未变。
    assert_eq!(
        sha256_hex(&fs::read(&source).expect("读原文件")),
        source_hash_before
    );

    // 重复导入必须幂等：不新增书/书签/进度行。
    let before_books = books::count_books(storage_b.conn()).expect("B 书籍数");
    let before_marks = bookmarks::count_all_bookmarks(storage_b.conn()).expect("B 书签数");
    let again = sync::import_bundle(
        &mut store_b,
        storage_b.conn(),
        &bundle,
        &ImportOptions::default(),
    )
    .expect("重复导入");
    assert_eq!(again.books_created, 0);
    assert_eq!(again.bookmarks_created, 0);
    assert_eq!(
        books::count_books(storage_b.conn()).expect("B 书籍数"),
        before_books
    );
    assert_eq!(
        bookmarks::count_all_bookmarks(storage_b.conn()).expect("B 书签数"),
        before_marks
    );
}

#[test]
fn removed_book_is_propagated_without_deleting_source_file() {
    let root = TempDir::new("sync-removed");
    let (storage_a, db_a, source, source_bytes) = seed_device_a(&root);
    let source_hash = sha256_hex(&source_bytes);
    let bundle = root.join("bundle");
    let mut store_a = sync::SyncStore::open(Some(&db_a)).expect("A 边车");
    sync::export_bundle(
        &mut store_a,
        storage_a.conn(),
        &bundle,
        &ExportOptions::default(),
    )
    .expect("首次导出");

    let db_b = root.join("b/library.db");
    let storage_b = open_db(&db_b);
    let mut store_b = sync::SyncStore::open(Some(&db_b)).expect("B 边车");
    sync::import_bundle(
        &mut store_b,
        storage_b.conn(),
        &bundle,
        &ImportOptions::default(),
    )
    .expect("首次导入");
    let b_book = books::list_shelf_books(storage_b.conn()).expect("B 书架")[0].clone();

    // A 移出书架（软移除），再导出。
    let book_a = books::list_shelf_books(storage_a.conn()).expect("A 书架")[0].clone();
    books::remove_book_from_shelf(storage_a.conn(), book_a.id, 3_000).expect("A 移出书架");
    sync::export_bundle(
        &mut store_a,
        storage_a.conn(),
        &bundle,
        &ExportOptions::default(),
    )
    .expect("移除后导出");

    sync::import_bundle(
        &mut store_b,
        storage_b.conn(),
        &bundle,
        &ImportOptions::default(),
    )
    .expect("移除后导入");
    let after = books::get_book(storage_b.conn(), b_book.id)
        .expect("读 B 书")
        .expect("B 书存在");
    assert_eq!(after.status, books::STATUS_REMOVED);
    assert!(books::list_shelf_books(storage_b.conn())
        .expect("B 书架")
        .is_empty());
    // 原文件不动；B 的副本也不因软移除被删（可恢复）。
    assert_eq!(
        sha256_hex(&fs::read(&source).expect("读原文件")),
        source_hash
    );
    assert!(std::path::PathBuf::from(&after.source_locator).is_file());
}

#[test]
fn metadata_only_export_creates_missing_placeholder_and_still_merges_progress() {
    let root = TempDir::new("sync-metadata");
    let (storage_a, db_a, _source, _bytes) = seed_device_a(&root);
    let bundle = root.join("bundle");
    let mut store_a = sync::SyncStore::open(Some(&db_a)).expect("A 边车");
    sync::export_bundle(
        &mut store_a,
        storage_a.conn(),
        &bundle,
        &ExportOptions {
            include_books: false,
        },
    )
    .expect("仅元数据导出");
    assert_eq!(
        fs::read_dir(bundle.join("books")).expect("列目录").count(),
        0,
        "include_books=false 不得复制书籍"
    );

    let db_b = root.join("b/library.db");
    let storage_b = open_db(&db_b);
    let mut store_b = sync::SyncStore::open(Some(&db_b)).expect("B 边车");
    let imported = sync::import_bundle(
        &mut store_b,
        storage_b.conn(),
        &bundle,
        &ImportOptions::default(),
    )
    .expect("导入");

    assert_eq!(imported.books_missing, 1);
    let book = books::list_all_books(storage_b.conn()).expect("B 全部书籍")[0].clone();
    assert_eq!(book.status, books::STATUS_MISSING);
    let progress = progress::load_progress(storage_b.conn(), book.id)
        .expect("读进度")
        .expect("占位书也必须能保留进度");
    assert_eq!(progress.byte_offset, 64);
}

#[test]
fn bundle_rejects_path_traversal_and_bad_hash() {
    let root = TempDir::new("sync-invalid");
    let db = root.join("library.db");
    let storage = open_db(&db);
    let mut store = sync::SyncStore::open(Some(&db)).expect("边车");
    let bundle = root.join("bundle");
    fs::create_dir_all(bundle.join("books")).expect("建目录");

    // sync_id 含路径分隔符：必须拒绝，避免占位副本落盘到目录外。
    let state = r#"{
      "magic":"novel-reader-sync",
      "schema":1,
      "device_id":"peer",
      "books":[{
        "sync_id":"../escape",
        "display_name":"坏书",
        "original_name":"坏书.txt"
      }]
    }"#;
    fs::write(bundle.join("state.json"), state).expect("写 state");
    let error = sync::import_bundle(
        &mut store,
        storage.conn(),
        &bundle,
        &ImportOptions::default(),
    )
    .expect_err("非法 sync_id 必须被拒");
    assert!(error.to_string().contains("sync_id"), "{error}");

    // content_hash 必须是 64 位十六进制；`..` 不能被当文件名。
    let state = r#"{
      "magic":"novel-reader-sync",
      "schema":1,
      "device_id":"peer",
      "books":[{
        "sync_id":"book-1",
        "content_hash":"../../escape",
        "display_name":"坏书",
        "original_name":"坏书.txt"
      }]
    }"#;
    fs::write(bundle.join("state.json"), state).expect("写 state");
    let error = sync::import_bundle(
        &mut store,
        storage.conn(),
        &bundle,
        &ImportOptions::default(),
    )
    .expect_err("非法 content_hash 必须被拒");
    assert!(error.to_string().contains("content_hash"), "{error}");
}

#[test]
fn export_is_stable_for_same_book_and_reuses_sync_id() {
    let root = TempDir::new("sync-stable");
    let (storage, db, _source, _bytes) = seed_device_a(&root);
    let mut store = sync::SyncStore::open(Some(&db)).expect("边车");
    let first = root.join("bundle-1");
    let second = root.join("bundle-2");
    sync::export_bundle(
        &mut store,
        storage.conn(),
        &first,
        &ExportOptions::default(),
    )
    .expect("首次导出");
    sync::export_bundle(
        &mut store,
        storage.conn(),
        &second,
        &ExportOptions::default(),
    )
    .expect("二次导出");

    let read = |dir: &std::path::Path| {
        let bytes = fs::read(dir.join("state.json")).expect("读 state");
        serde_json::from_slice::<sync::SyncBundle>(&bytes).expect("解析 state")
    };
    let a = read(&first);
    let b = read(&second);
    assert_eq!(a.books[0].sync_id, b.books[0].sync_id);
    assert_eq!(a.books[0].content_hash, b.books[0].content_hash);
    assert_eq!(b.seq, a.seq + 1);
}

#[test]
fn import_can_use_shared_books_dir_for_webdav_layout() {
    let root = TempDir::new("sync-shared-books");
    let (storage_a, db_a, _source, bytes) = seed_device_a(&root);
    let bundle = root.join("bundle");
    let mut store_a = sync::SyncStore::open(Some(&db_a)).expect("A 边车");
    sync::export_bundle(
        &mut store_a,
        storage_a.conn(),
        &bundle,
        &ExportOptions::default(),
    )
    .expect("导出");

    // 模拟 WebDAV：state.json 与书籍 blob 分开存放。
    let shared = root.join("shared-books");
    fs::create_dir_all(&shared).expect("建共享书籍目录");
    for entry in fs::read_dir(bundle.join("books")).expect("列包书籍") {
        let entry = entry.expect("目录项");
        fs::rename(entry.path(), shared.join(entry.file_name())).expect("移动 blob");
    }
    fs::remove_dir_all(bundle.join("books")).expect("移除空 books 目录");

    let db_b = root.join("b/library.db");
    let storage_b = open_db(&db_b);
    let mut store_b = sync::SyncStore::open(Some(&db_b)).expect("B 边车");
    let imported = sync::import_bundle(
        &mut store_b,
        storage_b.conn(),
        &bundle,
        &ImportOptions {
            library_dir: None,
            books_dir: Some(shared),
            progress_policy: sync::ProgressPolicy::KeepLocal,
            ..ImportOptions::default()
        },
    )
    .expect("从共享书籍目录导入");
    assert_eq!(imported.books_created, 1);
    assert_eq!(
        imported.books[0].status,
        reading_storage::books::STATUS_READY
    );
    let book = books::list_shelf_books(storage_b.conn()).expect("B 书架")[0].clone();
    assert!(std::path::PathBuf::from(&book.source_locator).is_file());
    assert_eq!(
        sha256_hex(&fs::read(book.source_locator).expect("读副本")),
        sha256_hex(&bytes)
    );
}

#[test]
fn progress_policy_keeps_local_or_uses_newer_remote() {
    let root = TempDir::new("sync-progress-policy");
    let (storage_a, db_a, _source, _bytes) = seed_device_a(&root);
    let bundle = root.join("bundle");
    let mut store_a = sync::SyncStore::open(Some(&db_a)).expect("A 边车");
    sync::export_bundle(
        &mut store_a,
        storage_a.conn(),
        &bundle,
        &ExportOptions::default(),
    )
    .expect("导出");

    let db_b = root.join("b/library.db");
    let storage_b = open_db(&db_b);
    let mut store_b = sync::SyncStore::open(Some(&db_b)).expect("B 边车");
    sync::import_bundle(
        &mut store_b,
        storage_b.conn(),
        &bundle,
        &ImportOptions::default(),
    )
    .expect("首次导入");
    let book = books::list_shelf_books(storage_b.conn()).expect("B 书架")[0].clone();

    // 把 B 的本地进度改成「更旧但不同位置」。
    progress::save_progress(storage_b.conn(), &ProgressInput::at(book.id, 0, 8), 1_000)
        .expect("写本地旧进度");

    let kept = sync::import_bundle(
        &mut store_b,
        storage_b.conn(),
        &bundle,
        &ImportOptions {
            progress_policy: sync::ProgressPolicy::KeepLocal,
            ..ImportOptions::default()
        },
    )
    .expect("保留本地导入");
    assert_eq!(kept.progress_applied, 0);
    assert_eq!(kept.progress_skipped_remote_newer, 1);
    let local = progress::load_progress(storage_b.conn(), book.id)
        .expect("读进度")
        .expect("进度存在");
    assert_eq!(local.byte_offset, 8);

    let used = sync::import_bundle(
        &mut store_b,
        storage_b.conn(),
        &bundle,
        &ImportOptions {
            progress_policy: sync::ProgressPolicy::RemoteIfNewer,
            ..ImportOptions::default()
        },
    )
    .expect("使用远端导入");
    assert_eq!(used.progress_applied, 1);
    let remote = progress::load_progress(storage_b.conn(), book.id)
        .expect("读进度")
        .expect("进度存在");
    assert_eq!(remote.byte_offset, 64);
}

#[test]
fn excluded_book_is_not_exported_or_imported() {
    let root = TempDir::new("sync-exclude-book");
    let (storage_a, db_a, _source, _bytes) = seed_device_a(&root);
    let mut store_a = sync::SyncStore::open(Some(&db_a)).expect("A 边车");
    let book = books::list_shelf_books(storage_a.conn()).expect("A 书架")[0].clone();
    sync::set_book_excluded(&mut store_a, storage_a.conn(), book.id, true).expect("标记仅本地");

    let bundle = root.join("bundle");
    let exported = sync::export_bundle(
        &mut store_a,
        storage_a.conn(),
        &bundle,
        &ExportOptions::default(),
    )
    .expect("导出");
    assert_eq!(exported.books_exported, 0);
    assert_eq!(exported.progress_exported, 0);
    assert_eq!(exported.bookmarks_exported, 0);

    let db_b = root.join("b/library.db");
    let storage_b = open_db(&db_b);
    let mut store_b = sync::SyncStore::open(Some(&db_b)).expect("B 边车");
    sync::import_bundle(
        &mut store_b,
        storage_b.conn(),
        &bundle,
        &ImportOptions::default(),
    )
    .expect("导入");
    assert!(books::list_shelf_books(storage_b.conn())
        .expect("B 书架")
        .is_empty());
}

#[test]
fn excluded_category_inherits_to_children_and_blocks_import() {
    let root = TempDir::new("sync-exclude-category");
    let (storage_a, db_a, _source, _bytes) = seed_device_a(&root);
    let parent = reading_storage::categories::create_category(storage_a.conn(), None, "仅本地", 1)
        .expect("建父分类");
    let child =
        reading_storage::categories::create_category(storage_a.conn(), Some(parent), "子分类", 2)
            .expect("建子分类");
    let book = books::list_shelf_books(storage_a.conn()).expect("A 书架")[0].clone();
    reading_storage::categories::move_book_at(storage_a.conn(), book.id, Some(child), 3)
        .expect("移入子分类");
    let mut store_a = sync::SyncStore::open(Some(&db_a)).expect("A 边车");
    sync::set_category_excluded(&mut store_a, storage_a.conn(), parent, true).expect("标记分类");

    let bundle = root.join("bundle");
    let exported = sync::export_bundle(
        &mut store_a,
        storage_a.conn(),
        &bundle,
        &ExportOptions::default(),
    )
    .expect("导出");
    assert_eq!(exported.books_exported, 0);
    assert_eq!(exported.categories_exported, 0);

    let db_b = root.join("b/library.db");
    let storage_b = open_db(&db_b);
    let mut store_b = sync::SyncStore::open(Some(&db_b)).expect("B 边车");
    sync::import_bundle(
        &mut store_b,
        storage_b.conn(),
        &bundle,
        &ImportOptions::default(),
    )
    .expect("导入");
    assert!(books::list_shelf_books(storage_b.conn())
        .expect("B 书架")
        .is_empty());
    assert!(
        reading_storage::categories::list_categories(storage_b.conn())
            .expect("B 分类")
            .is_empty()
    );
}

#[test]
fn locally_excluded_book_ignores_remote_progress() {
    let root = TempDir::new("sync-exclude-progress");
    let (storage_a, db_a, _source, _bytes) = seed_device_a(&root);
    let bundle = root.join("bundle");
    let mut store_a = sync::SyncStore::open(Some(&db_a)).expect("A 边车");
    sync::export_bundle(
        &mut store_a,
        storage_a.conn(),
        &bundle,
        &ExportOptions::default(),
    )
    .expect("导出");

    let db_b = root.join("b/library.db");
    let storage_b = open_db(&db_b);
    let mut store_b = sync::SyncStore::open(Some(&db_b)).expect("B 边车");
    sync::import_bundle(
        &mut store_b,
        storage_b.conn(),
        &bundle,
        &ImportOptions::default(),
    )
    .expect("首次导入");
    let book = books::list_shelf_books(storage_b.conn()).expect("B 书架")[0].clone();
    sync::set_book_excluded(&mut store_b, storage_b.conn(), book.id, true).expect("B 标记仅本地");
    progress::save_progress(storage_b.conn(), &ProgressInput::at(book.id, 0, 8), 1_000)
        .expect("B 本地进度");

    let imported = sync::import_bundle(
        &mut store_b,
        storage_b.conn(),
        &bundle,
        &ImportOptions {
            progress_policy: sync::ProgressPolicy::RemoteIfNewer,
            ..ImportOptions::default()
        },
    )
    .expect("再次导入");
    assert!(imported.books_skipped_excluded >= 1);
    let kept = progress::load_progress(storage_b.conn(), book.id)
        .expect("读进度")
        .expect("进度存在");
    assert_eq!(kept.byte_offset, 8);
}

#[test]
fn import_flags_allow_progress_only_or_books_only() {
    let root = TempDir::new("sync-import-flags");
    let (storage_a, db_a, _source, _bytes) = seed_device_a(&root);
    let bundle = root.join("bundle");
    let mut store_a = sync::SyncStore::open(Some(&db_a)).expect("A 边车");
    sync::export_bundle(
        &mut store_a,
        storage_a.conn(),
        &bundle,
        &ExportOptions::default(),
    )
    .expect("导出");

    // 空库 + 仅进度：不应创建书籍。
    let db_progress = root.join("progress/library.db");
    let storage_progress = open_db(&db_progress);
    let mut store_progress = sync::SyncStore::open(Some(&db_progress)).expect("进度边车");
    let progress_only = sync::import_bundle(
        &mut store_progress,
        storage_progress.conn(),
        &bundle,
        &ImportOptions {
            import_books: false,
            ..ImportOptions::default()
        },
    )
    .expect("仅进度导入");
    assert_eq!(progress_only.books_created, 0);
    assert!(books::list_shelf_books(storage_progress.conn())
        .expect("进度库书架")
        .is_empty());

    // 已有书 + 仅书籍：文件/元数据导入，但不应用远端进度。
    let db_books = root.join("books/library.db");
    let storage_books = open_db(&db_books);
    let mut store_books = sync::SyncStore::open(Some(&db_books)).expect("书籍边车");
    let books_only = sync::import_bundle(
        &mut store_books,
        storage_books.conn(),
        &bundle,
        &ImportOptions {
            apply_progress: false,
            apply_bookmarks: false,
            apply_settings: false,
            ..ImportOptions::default()
        },
    )
    .expect("仅书籍导入");
    assert_eq!(books_only.books_created, 1);
    assert_eq!(books_only.progress_applied, 0);
    let book = books::list_shelf_books(storage_books.conn()).expect("书籍库书架")[0].clone();
    assert!(progress::load_progress(storage_books.conn(), book.id)
        .expect("读进度")
        .is_none());
}

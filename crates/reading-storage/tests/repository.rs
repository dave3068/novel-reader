//! RC6 事务化仓储测试：导入 upsert、重复导入、进度幂等 upsert、书签 CRUD、索引写入与校验。

mod common;

use common::{sample_import, seed_book, write_sample_txt, TempDir};
use reading_storage::bookmarks::{self, BookmarkInput};
use reading_storage::books;
use reading_storage::error::{ERR_INVALID_ARGUMENT, ERR_OUT_OF_RANGE};
use reading_storage::index::{self, ChapterInput, LayoutRecordInput, LAYOUT_COMPLETE};
use reading_storage::progress::{self, ProgressInput};
use reading_storage::settings::{self, ReaderSettings, DEFAULT_FONT_FAMILY, THEME_DARK};
use reading_storage::{connection, meta, validate, Storage};

#[test]
fn import_book_upserts_and_duplicate_import_is_idempotent() {
    let dir = TempDir::new("repo-import");
    let mut storage = Storage::open(dir.join("library.db")).expect("打开数据库");
    storage.migrate().expect("初始迁移");
    let conn = storage.conn();

    let (path, bytes) = write_sample_txt(&dir, "import.txt");
    let locator = path.to_string_lossy().to_string();
    let import = sample_import(&locator, "样书A", &bytes);

    let first = books::import_book(conn, &import, 1_000).expect("首次导入");
    assert!(first.created && first.source_created && first.updated);
    assert_eq!(books::count_books(conn).expect("书籍数"), 1);
    let stored = books::get_book(conn, first.book_id)
        .expect("读取书籍")
        .expect("书籍存在");
    assert_eq!(stored.status, books::STATUS_READY);
    assert_eq!(stored.source_size, bytes.len() as i64);
    assert_eq!(stored.created_at, 1_000);
    assert_eq!(stored.updated_at, 1_000);
    assert_eq!(stored.format, books::FORMAT_TXT);
    assert!(stored.is_on_shelf());

    // 完全重复导入：同一 id、无新增行、时间戳不推进（幂等）。
    let duplicate = books::import_book(conn, &import, 2_000).expect("重复导入");
    assert_eq!(duplicate.book_id, first.book_id);
    assert!(!duplicate.created && !duplicate.source_created && !duplicate.updated);
    assert_eq!(books::count_books(conn).expect("书籍数"), 1);
    let reloaded = books::get_book(conn, first.book_id)
        .expect("读取书籍")
        .expect("书籍存在");
    assert_eq!(reloaded.updated_at, 1_000, "重复导入不得推进 updated_at");
    assert_eq!(
        books::list_book_sources(conn, first.book_id)
            .expect("来源清单")
            .len(),
        1
    );

    // 改名后再导入：同一行被更新，仍无重复。
    let mut renamed = import.clone();
    renamed.display_name = "样书A（改名）".to_string();
    let updated = books::import_book(conn, &renamed, 3_000).expect("改名导入");
    assert!(!updated.created && updated.updated);
    assert_eq!(books::count_books(conn).expect("书籍数"), 1);
    let after_rename = books::get_book(conn, first.book_id)
        .expect("读取书籍")
        .expect("书籍存在");
    assert_eq!(after_rename.display_name, "样书A（改名）");
    assert_eq!(after_rename.updated_at, 3_000);
}

#[test]
fn import_book_claims_same_content_from_another_locator() {
    let dir = TempDir::new("repo-claim");
    let mut storage = Storage::open(dir.join("library.db")).expect("打开数据库");
    storage.migrate().expect("初始迁移");
    let conn = storage.conn();

    let (book_id, path, bytes) = seed_book(&storage, &dir, "origin.txt", "样书-认领");
    let moved = dir.join("moved.txt");
    std::fs::write(&moved, &bytes).expect("写入移动后的副本");
    let reimport = sample_import(&moved.to_string_lossy(), "样书-认领", &bytes);

    let outcome = books::import_book(conn, &reimport, 2_000).expect("从新路径导入");
    assert_eq!(outcome.book_id, book_id, "同内容应复用同一条目");
    assert!(!outcome.created);
    assert!(outcome.source_created, "新路径应新增一条来源记录");
    assert_eq!(books::count_books(conn).expect("书籍数"), 1);

    let sources = books::list_book_sources(conn, book_id).expect("来源清单");
    assert_eq!(sources.len(), 2);
    assert_eq!(
        sources.iter().filter(|s| s.is_primary).count(),
        1,
        "主来源必须唯一"
    );
    let primary = sources.iter().find(|s| s.is_primary).expect("存在主来源");
    assert_eq!(primary.source_locator, moved.to_string_lossy());

    // 按来源身份（旧定位符经 book_sources 仍可命中）与指纹均可定位。
    assert_eq!(
        books::find_book_by_locator(conn, books::KIND_DESKTOP_PATH, &path.to_string_lossy())
            .expect("按旧来源查找")
            .map(|b| b.id),
        Some(book_id),
        "历史来源定位符必须仍指向同一条目"
    );
    assert_eq!(
        books::find_book_by_locator(conn, books::KIND_DESKTOP_PATH, &moved.to_string_lossy())
            .expect("按新来源查找")
            .map(|b| b.id),
        Some(book_id)
    );
    let fingerprint = common::fnv1a64_hex(&bytes);
    assert_eq!(
        books::find_book_by_fingerprint(conn, &fingerprint)
            .expect("按指纹查找")
            .map(|b| b.id),
        Some(book_id)
    );
}

#[test]
fn import_book_marks_needs_reindex_and_clears_only_rebuildable_index_on_content_change() {
    let dir = TempDir::new("repo-content-change");
    let mut storage = Storage::open(dir.join("library.db")).expect("打开数据库");
    storage.migrate().expect("初始迁移");
    let (book_id, path, bytes) = seed_book(&storage, &dir, "changed.txt", "样书-变更");
    let conn = storage.conn();

    // 先建立完整状态：进度、书签、章节索引、分页缓存。
    let mut position = ProgressInput::at(book_id, 0, 64);
    position.nearby_fingerprint = Some("第一章 起始".to_string());
    progress::save_progress(conn, &position, 1_100).expect("保存进度");
    bookmarks::create_bookmark(conn, &BookmarkInput::at(book_id, 0, 64, "锚点"), 1_200)
        .expect("新增书签");
    let chapters = vec![ChapterInput {
        sequence: 0,
        title: "第一章 起始".to_string(),
        start_byte: 0,
        end_byte: bytes.len() as i64,
        rule_id: "chapter-rule-1".to_string(),
    }];
    index::rebuild_chapters(conn, book_id, 1, &chapters, 1_300).expect("写入章节");
    index::upsert_layout_record(
        conn,
        &LayoutRecordInput::new(book_id, "nrlay1:old", 1),
        1_400,
    )
    .expect("写入分页缓存");
    assert_eq!(meta::content_index_version(conn).expect("索引版本"), 0);

    // 用户原始文件被外部改动（内容变化 ⇒ 指纹变化），并注册编码变化。
    let mut changed_bytes = bytes.clone();
    changed_bytes.extend_from_slice("第三章 新增内容\n".as_bytes());
    std::fs::write(&path, &changed_bytes).expect("模拟外部修改");
    let locator = path.to_string_lossy().to_string();
    let mut reimport = sample_import(&locator, "样书-变更", &changed_bytes);
    reimport.encoding = "gb18030".to_string();

    let outcome = books::import_book(conn, &reimport, 2_000).expect("内容变化后重新导入");
    assert_eq!(outcome.book_id, book_id, "内容变化不得新建条目");
    assert!(!outcome.created && outcome.updated);
    assert_eq!(books::count_books(conn).expect("书籍数"), 1);

    let stored = books::get_book(conn, book_id)
        .expect("读取书籍")
        .expect("书籍存在");
    assert_eq!(stored.status, books::STATUS_NEEDS_REINDEX);
    assert_eq!(
        stored.source_fingerprint.as_deref(),
        Some(common::fnv1a64_hex(&changed_bytes).as_str())
    );
    assert_eq!(stored.encoding, "gb18030");

    // 只丢失可重建数据；进度、书签保留；内容索引版本推进（旧分页缓存失效）。
    assert_eq!(index::count_chapters(conn, book_id).expect("章节数"), 0);
    assert_eq!(
        index::count_layout_records(conn, book_id).expect("缓存行数"),
        0
    );
    assert_eq!(meta::content_index_version(conn).expect("索引版本"), 1);
    assert_eq!(
        progress::load_progress(conn, book_id)
            .expect("读取进度")
            .expect("进度仍在")
            .byte_offset,
        64
    );
    assert_eq!(
        bookmarks::count_bookmarks(conn, book_id).expect("书签数"),
        1
    );

    // 再导入同一内容：字段无变化 ⇒ 幂等，且不得把 needs_reindex 悄悄改回 ready。
    let again = books::import_book(conn, &reimport, 3_000).expect("重复导入");
    assert!(!again.updated && !again.changed_any());
    let after = books::get_book(conn, book_id)
        .expect("读取书籍")
        .expect("书籍存在");
    assert_eq!(after.status, books::STATUS_NEEDS_REINDEX);
    assert_eq!(after.updated_at, 2_000, "幂等重复导入不得推进时间戳");

    // 原始文件仍然是用户改后的内容（存储层从不改写用户文件）。
    assert_eq!(std::fs::read(&path).expect("读取原始文件"), changed_bytes);
}

#[test]
fn progress_upsert_is_idempotent_and_updates_only_on_change() {
    let dir = TempDir::new("repo-progress");
    let mut storage = Storage::open(dir.join("library.db")).expect("打开数据库");
    storage.migrate().expect("初始迁移");
    let (book_id, _path, _bytes) = seed_book(&storage, &dir, "progress.txt", "样书-进度");
    let conn = storage.conn();

    let mut position = ProgressInput::at(book_id, 0, 128);
    position.nearby_fingerprint = Some("第一章 起始".to_string());
    position.last_page_hint = Some(1);

    let first = progress::save_progress(conn, &position, 10).expect("保存进度");
    assert!(first.inserted && first.changed);
    let stored = progress::load_progress(conn, book_id)
        .expect("读取进度")
        .expect("进度存在");
    assert_eq!(stored.byte_offset, 128);
    assert_eq!(stored.chapter_index, 0);
    assert_eq!(stored.content_unit_id, 0);
    assert_eq!(stored.updated_at, 10);
    assert_eq!(stored.last_opened_at, 10);

    // 幂等：同位置重复保存 ⇒ 不写库、不推进时间戳。
    let repeat = progress::save_progress(conn, &position, 20).expect("重复保存");
    assert!(!repeat.inserted && !repeat.changed);
    let after_repeat = progress::load_progress(conn, book_id)
        .expect("读取进度")
        .expect("进度存在");
    assert_eq!(after_repeat, stored, "幂等保存不得改变任何字段");
    assert_eq!(progress::count_progress(conn).expect("进度行数"), 1);

    // 位置变化 ⇒ 更新同一行（仍只有一行）。
    position.byte_offset = 256;
    position.last_page_hint = None;
    let moved = progress::save_progress(conn, &position, 30).expect("保存新位置");
    assert!(!moved.inserted && moved.changed);
    let moved_record = progress::load_progress(conn, book_id)
        .expect("读取进度")
        .expect("进度存在");
    assert_eq!(moved_record.byte_offset, 256);
    assert_eq!(moved_record.updated_at, 30);
    assert_eq!(progress::count_progress(conn).expect("进度行数"), 1);

    // 只更新最近打开时间。
    assert!(progress::touch_last_opened(conn, book_id, 40).expect("touch"));
    let touched = progress::load_progress(conn, book_id)
        .expect("读取进度")
        .expect("进度存在");
    assert_eq!(touched.last_opened_at, 40);
    assert_eq!(touched.updated_at, 30, "touch 不得改动位置更新时间");
    assert!(!progress::touch_last_opened(conn, 9_999, 40).expect("touch 未命中"));
}

#[test]
fn bookmark_crud_round_trip() {
    let dir = TempDir::new("repo-bookmark");
    let mut storage = Storage::open(dir.join("library.db")).expect("打开数据库");
    storage.migrate().expect("初始迁移");
    let (book_id, _path, _bytes) = seed_book(&storage, &dir, "bookmark.txt", "样书-书签");
    let conn = storage.conn();

    let first = bookmarks::create_bookmark(conn, &BookmarkInput::at(book_id, 0, 64, "甲"), 10)
        .expect("新增书签甲");
    let second = bookmarks::create_bookmark(conn, &BookmarkInput::at(book_id, 1, 32, "乙"), 11)
        .expect("新增书签乙");
    let third = bookmarks::create_bookmark(conn, &BookmarkInput::at(book_id, 2, 96, ""), 12)
        .expect("新增无名书签");
    assert_ne!(first, second);
    assert_ne!(second, third);

    let listed = bookmarks::list_bookmarks(conn, book_id).expect("书签清单");
    assert_eq!(listed.len(), 3);
    assert_eq!(
        listed.iter().map(|b| b.byte_offset).collect::<Vec<_>>(),
        vec![32, 64, 96],
        "书签按字节偏移排序（跳转顺序稳定）"
    );
    assert_eq!(
        bookmarks::count_bookmarks(conn, book_id).expect("书签数"),
        3
    );
    assert_eq!(bookmarks::count_all_bookmarks(conn).expect("书签总数"), 3);

    let fetched = bookmarks::get_bookmark(conn, second)
        .expect("读取书签")
        .expect("书签存在");
    assert_eq!(fetched.title, "乙");
    assert_eq!(fetched.created_at, 11);

    assert!(bookmarks::rename_bookmark(conn, second, "乙（改名）", 20).expect("重命名"));
    let renamed = bookmarks::get_bookmark(conn, second)
        .expect("读取书签")
        .expect("书签存在");
    assert_eq!(renamed.title, "乙（改名）");
    assert_eq!(renamed.updated_at, 20);
    assert_eq!(renamed.created_at, 11, "重命名不得改动创建时间");
    assert!(!bookmarks::rename_bookmark(conn, 9_999, "x", 21).expect("重命名未命中"));

    assert!(bookmarks::delete_bookmark(conn, first).expect("删除书签"));
    assert!(!bookmarks::delete_bookmark(conn, first).expect("重复删除"));
    assert_eq!(
        bookmarks::count_bookmarks(conn, book_id).expect("书签数"),
        2
    );
    assert_eq!(
        bookmarks::delete_bookmarks_for_book(conn, book_id).expect("清空书签"),
        2
    );
    assert_eq!(bookmarks::count_all_bookmarks(conn).expect("书签总数"), 0);
}

#[test]
fn chapters_rebuild_replaces_index_and_layout_records_are_keyed_by_signature() {
    let dir = TempDir::new("repo-index");
    let mut storage = Storage::open(dir.join("library.db")).expect("打开数据库");
    storage.migrate().expect("初始迁移");
    let (book_id, _path, bytes) = seed_book(&storage, &dir, "index.txt", "样书-索引");
    let conn = storage.conn();

    let chapters: Vec<ChapterInput> = (0..5)
        .map(|index| ChapterInput {
            sequence: index,
            title: format!("第 {} 章", index + 1),
            start_byte: index * 10,
            end_byte: (index + 1) * 10,
            rule_id: "chapter-rule-a".to_string(),
        })
        .collect();
    assert_eq!(
        index::rebuild_chapters(conn, book_id, 1, &chapters, 10).expect("写入章节"),
        5
    );
    assert_eq!(index::count_chapters(conn, book_id).expect("章节数"), 5);
    let listed = index::list_chapters(conn, book_id).expect("章节清单");
    assert_eq!(listed.len(), 5);
    assert_eq!(listed[0].index_version, 1);
    assert_eq!(listed[4].end_byte, 50);

    // 新索引版本重建 ⇒ 旧行被替换（不累积）。
    let rebuilt: Vec<ChapterInput> = (0..3)
        .map(|index| ChapterInput {
            sequence: index,
            title: format!("重排章 {}", index + 1),
            start_byte: index * 20,
            end_byte: (index + 1) * 20,
            rule_id: "chapter-rule-b".to_string(),
        })
        .collect();
    assert_eq!(
        index::rebuild_chapters(conn, book_id, 2, &rebuilt, 11).expect("重建章节"),
        3
    );
    assert_eq!(index::count_chapters(conn, book_id).expect("章节数"), 3);
    assert!(index::list_chapters(conn, book_id)
        .expect("章节清单")
        .iter()
        .all(|chapter| chapter.index_version == 2));

    // 非法章节（end < start / 回退）被拒绝，且既有索引不被破坏。
    let broken = [ChapterInput {
        sequence: 0,
        title: "坏章".to_string(),
        start_byte: 30,
        end_byte: 10,
        rule_id: "r".to_string(),
    }];
    assert_eq!(
        index::rebuild_chapters(conn, book_id, 3, &broken, 12)
            .expect_err("end < start 必须被拒绝")
            .code(),
        ERR_OUT_OF_RANGE
    );
    assert_eq!(index::count_chapters(conn, book_id).expect("章节数"), 3);

    // 分页缓存：键 = book_id + layout_signature + content_index_version。
    let signature_a = "nrlay1:aaaaaaaa";
    let signature_b = "nrlay1:bbbbbbbb";
    let mut input = LayoutRecordInput::new(book_id, signature_a, 2);
    input.generated_until_byte = 2_048;
    input.page_count = 8;
    input.status = LAYOUT_COMPLETE.to_string();
    let id_first = index::upsert_layout_record(conn, &input, 10).expect("写入分页缓存");
    let id_again = index::upsert_layout_record(conn, &input, 20).expect("重复写入分页缓存");
    assert_eq!(id_first, id_again, "同键 upsert 不得新增行");
    assert_eq!(
        index::count_layout_records(conn, book_id).expect("缓存行数"),
        1
    );
    let record = index::get_layout_record(conn, book_id, signature_a, 2)
        .expect("读取缓存")
        .expect("缓存存在");
    assert_eq!(record.page_count, 8);
    assert_eq!(record.generated_until_byte, 2_048);
    assert_eq!(record.updated_at, 20);
    assert!(index::get_layout_record(conn, book_id, signature_a, 1)
        .expect("读取缓存")
        .is_none());

    index::upsert_layout_record(conn, &LayoutRecordInput::new(book_id, signature_b, 2), 30)
        .expect("写入第二种签名");
    assert_eq!(
        index::count_layout_records(conn, book_id).expect("缓存行数"),
        2
    );

    // 签名变化 ⇒ 旧签名边界全部失效。
    assert_eq!(
        index::drop_layout_records_for_signature(conn, book_id, signature_a).expect("按签名失效"),
        1
    );
    // 内容索引版本变化 ⇒ 旧版本缓存全部失效。
    index::upsert_layout_record(conn, &LayoutRecordInput::new(book_id, signature_b, 7), 40)
        .expect("写入过期版本");
    assert_eq!(
        index::count_layout_records(conn, book_id).expect("缓存行数"),
        2
    );
    assert_eq!(
        index::drop_stale_layout_records(conn, book_id, 2).expect("丢弃过期版本"),
        1
    );
    assert_eq!(
        index::count_layout_records(conn, book_id).expect("缓存行数"),
        1
    );

    // 非法状态被拒绝。
    let mut invalid = LayoutRecordInput::new(book_id, "nrlay1:cccc", 2);
    invalid.status = "unknown".to_string();
    assert_eq!(
        index::upsert_layout_record(conn, &invalid, 50)
            .expect_err("非法状态必须被拒绝")
            .code(),
        ERR_INVALID_ARGUMENT
    );
    assert_eq!(
        index::rebuild_chapters(conn, book_id, 4, &[], 51).expect("清空章节"),
        0
    );
    assert_eq!(index::count_chapters(conn, book_id).expect("章节数"), 0);
    assert!(bytes.len() > 50);
}

#[test]
fn settings_singleton_upsert_and_app_meta_helpers() {
    let dir = TempDir::new("repo-settings");
    let mut storage = Storage::open(dir.join("library.db")).expect("打开数据库");
    storage.migrate().expect("初始迁移");
    let conn = storage.conn();

    // 无行时返回默认设置，且不写库。
    let defaults = settings::load_settings(conn).expect("读取默认设置");
    assert_eq!(defaults.font_family, DEFAULT_FONT_FAMILY);
    assert_eq!(defaults.font_size_milli, settings::DEFAULT_FONT_SIZE_MILLI);
    assert_eq!(settings::settings_row_count(conn).expect("设置行数"), 0);

    let mut changed = defaults.clone();
    changed.theme = THEME_DARK.to_string();
    changed.font_weight = 700;
    changed.updated_at = 11;
    settings::save_settings(conn, &changed, 11).expect("保存设置");
    assert_eq!(settings::settings_row_count(conn).expect("设置行数"), 1);
    assert_eq!(settings::load_settings(conn).expect("读取设置"), changed);

    // 单行 upsert：再次保存仍只有一行。
    let mut second = changed.clone();
    second.font_size_milli = 22_000;
    second.updated_at = 12;
    settings::save_settings(conn, &second, 12).expect("再次保存设置");
    assert_eq!(settings::settings_row_count(conn).expect("设置行数"), 1);
    assert_eq!(settings::load_settings(conn).expect("读取设置"), second);

    // 非法枚举 / 负尺寸被拒绝。
    let mut invalid = second.clone();
    invalid.theme = "neon".to_string();
    assert_eq!(
        settings::save_settings(conn, &invalid, 13)
            .expect_err("非法主题必须被拒绝")
            .code(),
        ERR_INVALID_ARGUMENT
    );
    let mut negative = second.clone();
    negative.margin_x_milli = -1;
    assert_eq!(
        settings::save_settings(conn, &negative, 13)
            .expect_err("负边距必须被拒绝")
            .code(),
        ERR_OUT_OF_RANGE
    );
    assert!(settings::reset_settings(conn).expect("重置设置"));
    assert_eq!(settings::settings_row_count(conn).expect("设置行数"), 0);

    // app_meta：读写、删除、内容索引版本递增。
    assert_eq!(meta::get(conn, "unknown_key").expect("读取缺失键"), None);
    meta::set(conn, "ui.last_shelf_tab", "recent", 20).expect("写入元信息");
    assert_eq!(
        meta::get(conn, "ui.last_shelf_tab").expect("读取元信息"),
        Some("recent".to_string())
    );
    let before = meta::count(conn).expect("元信息行数");
    meta::set(conn, "ui.last_shelf_tab", "all", 21).expect("覆盖元信息");
    assert_eq!(meta::count(conn).expect("元信息行数"), before);
    assert!(meta::remove(conn, "ui.last_shelf_tab").expect("删除元信息"));
    assert!(!meta::remove(conn, "ui.last_shelf_tab").expect("重复删除"));
    assert_eq!(meta::get(conn, "ui.last_shelf_tab").expect("读取"), None);

    assert_eq!(meta::content_index_version(conn).expect("索引版本"), 0);
    assert_eq!(
        meta::bump_content_index_version(conn, 30).expect("递增索引版本"),
        1
    );
    assert_eq!(
        meta::bump_content_index_version(conn, 31).expect("递增索引版本"),
        2
    );
    assert_eq!(meta::content_index_version(conn).expect("索引版本"), 2);
    assert_eq!(
        meta::schema_version(conn).expect("schema 版本"),
        reading_storage::migrate::TARGET_SCHEMA_VERSION
    );
    assert_eq!(meta::SCHEMA_VERSION_KEY, "schema_version");

    // 非法元信息键 / 超长值被拒绝。
    assert_eq!(
        meta::set(conn, "bad key with spaces", "x", 32)
            .expect_err("非法键必须被拒绝")
            .code(),
        ERR_INVALID_ARGUMENT
    );
    assert_eq!(
        meta::set(
            conn,
            "ok_key",
            &"甲".repeat(validate::MAX_META_VALUE_BYTES),
            33
        )
        .expect_err("超长值必须被拒绝")
        .code(),
        ERR_INVALID_ARGUMENT
    );
}

#[test]
fn repository_rejects_full_text_and_out_of_range_values() {
    let dir = TempDir::new("repo-validate");
    let mut storage = Storage::open(dir.join("library.db")).expect("打开数据库");
    storage.migrate().expect("初始迁移");
    let (book_id, path, bytes) = seed_book(&storage, &dir, "validate.txt", "样书-校验");
    let conn = storage.conn();

    // 指纹冒充正文（超长短摘要）⇒ 拒绝，不落库。
    let mut oversized = sample_import(&path.to_string_lossy(), "样书-校验", &bytes);
    oversized.source_fingerprint = Some("甲".repeat(validate::MAX_FINGERPRINT_BYTES));
    assert_eq!(
        books::import_book(conn, &oversized, 1_000)
            .expect_err("超长指纹必须被拒绝")
            .code(),
        ERR_INVALID_ARGUMENT
    );
    assert_eq!(books::count_books(conn).expect("书籍数"), 1);

    // 未知来源种类 / 负长度被拒绝。
    let mut unknown_kind = sample_import("/tmp/x.txt", "x", b"abc");
    unknown_kind.source_kind = "usb_stick".to_string();
    assert_eq!(
        books::import_book(conn, &unknown_kind, 1_000)
            .expect_err("未知来源种类必须被拒绝")
            .code(),
        ERR_INVALID_ARGUMENT
    );
    let mut negative_size = sample_import("/tmp/y.txt", "y", b"abc");
    negative_size.source_size = -1;
    assert_eq!(
        books::import_book(conn, &negative_size, 1_000)
            .expect_err("负长度必须被拒绝")
            .code(),
        ERR_OUT_OF_RANGE
    );

    // 进度负偏移 / 超长指纹被拒绝。
    assert_eq!(
        progress::save_progress(conn, &ProgressInput::at(book_id, 0, -1), 1_000)
            .expect_err("负偏移必须被拒绝")
            .code(),
        ERR_OUT_OF_RANGE
    );
    let mut traced = ProgressInput::at(book_id, 0, 16);
    traced.nearby_fingerprint = Some("甲".repeat(validate::MAX_FINGERPRINT_BYTES));
    assert_eq!(
        progress::save_progress(conn, &traced, 1_000)
            .expect_err("超长进度指纹必须被拒绝")
            .code(),
        ERR_INVALID_ARGUMENT
    );
    assert_eq!(progress::count_progress(conn).expect("进度行数"), 0);

    // 书签超长标题被拒绝。
    let long_title = BookmarkInput::at(book_id, 0, 16, "甲".repeat(validate::MAX_TITLE_BYTES + 1));
    assert_eq!(
        bookmarks::create_bookmark(conn, &long_title, 1_000)
            .expect_err("超长书签标题必须被拒绝")
            .code(),
        ERR_INVALID_ARGUMENT
    );
    assert_eq!(bookmarks::count_all_bookmarks(conn).expect("书签数"), 0);

    // 越界防护：偏移不得超过文件长度。
    assert_eq!(
        progress::ensure_offset_within(2_048, 1_024)
            .expect_err("越界偏移必须被拒绝")
            .code(),
        ERR_OUT_OF_RANGE
    );
    assert!(progress::ensure_offset_within(1_024, 1_024).is_ok());

    // 不存在的书籍：读取返回 None，写状态返回 BookNotFound。
    assert!(books::get_book(conn, 4_242)
        .expect("读取不存在的书")
        .is_none());
    assert_eq!(
        books::set_book_status(conn, 4_242, books::STATUS_MISSING, 1_000)
            .expect_err("不存在的书籍必须报错")
            .code(),
        reading_storage::error::ERR_BOOK_NOT_FOUND
    );

    // 未迁移的库：schema 版本为 0，app_meta 尚不存在。
    let mut fresh = Storage::open(dir.join("fresh.db")).expect("打开空库");
    assert_eq!(fresh.schema_version().expect("schema 版本"), 0);
    assert!(!meta::table_exists(fresh.conn()).expect("app_meta 表检查"));
    assert_eq!(
        meta::get(fresh.conn(), "any").expect("未迁移时读取元信息"),
        None
    );
    assert_eq!(meta::count(fresh.conn()).expect("元信息行数"), 0);
    assert!(!meta::remove(fresh.conn(), "any").expect("未迁移时删除元信息"));
    assert_eq!(
        connection::table_names(fresh.conn()).expect("表清单").len(),
        0
    );
    assert!(fresh.migrate().expect("空库迁移").changed);
    assert!(meta::table_exists(fresh.conn()).expect("迁移后 app_meta 存在"));
    assert_eq!(fresh.schema_version().expect("迁移后版本"), 1);

    // 状态枚举受控。
    assert_eq!(
        books::set_book_status(conn, book_id, "deleted_forever", 1_000)
            .expect_err("未知状态必须被拒绝")
            .code(),
        ERR_INVALID_ARGUMENT
    );
    let settings_default = ReaderSettings {
        operation_mode: "chaos".to_string(),
        ..ReaderSettings::default()
    };
    assert_eq!(
        settings::save_settings(conn, &settings_default, 1_000)
            .expect_err("未知操作模式必须被拒绝")
            .code(),
        ERR_INVALID_ARGUMENT
    );
}

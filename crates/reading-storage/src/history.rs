//! `reading_history`：阅读历史流水（V1.1，规格 R8/R9）。
//!
//! 口径（主人裁定 2026-10-05）：
//!
//! - **每次打开书籍记一条流水**：时间、当时章节与位置、进入方式；
//! - **不统计阅读时长**（V1 已明确排除「精确阅读时长统计」，本期不引入心跳/生命周期钩子）；
//! - 记录的是**打开时刻**的位置，不随后续阅读变化（设计稿 H1）；
//! - 书籍被硬删除 → 流水级联删除；被软移除（移出书架）→ 流水保留。
//!
//! 排序与聚合全部在存储层完成（规格 R18 / 门禁 C34：算法不得进 UI）。

use rusqlite::{params, Connection};

use crate::error::{Result, StorageError};

/// 进入方式（`reading_history.entry_kind` 的取值口径）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// 从书架行打开。
    FromShelf = 0,
    /// 阅读页内重开同一本。
    ReopenInReader = 1,
    /// 按上次保存的进度恢复。
    ResumeProgress = 2,
    /// 从书签跳转。
    BookmarkJump = 3,
    /// 从章节目录跳转。
    ChapterJump = 4,
}

impl EntryKind {
    /// 数据库存储值。
    pub fn as_i64(self) -> i64 {
        self as i64
    }

    /// 从数据库值还原；未知值按「书架打开」处理（向前兼容，不报错）。
    pub fn from_i64(value: i64) -> Self {
        match value {
            1 => EntryKind::ReopenInReader,
            2 => EntryKind::ResumeProgress,
            3 => EntryKind::BookmarkJump,
            4 => EntryKind::ChapterJump,
            _ => EntryKind::FromShelf,
        }
    }

    /// 用户可见文案（设计稿 H1 的进入方式 tag）。
    pub fn label(self) -> &'static str {
        match self {
            EntryKind::FromShelf => "书架打开",
            EntryKind::ReopenInReader => "阅读页打开",
            EntryKind::ResumeProgress => "进度恢复",
            EntryKind::BookmarkJump => "书签跳转",
            EntryKind::ChapterJump => "章节跳转",
        }
    }
}

/// 一条流水。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub id: i64,
    pub book_id: i64,
    /// 冗余书名快照（书籍改名后仍可读）。
    pub book_title: String,
    pub opened_at: i64,
    pub chapter_index: i64,
    pub byte_offset: i64,
    pub entry_kind: EntryKind,
    pub source_kind: String,
    pub source_locator: String,
    pub content_index_version: i64,
}

/// 写入一条流水的输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryInput {
    pub book_id: i64,
    pub book_title: String,
    pub opened_at: i64,
    pub chapter_index: i64,
    pub byte_offset: i64,
    pub entry_kind: EntryKind,
    pub source_kind: String,
    pub source_locator: String,
    pub content_index_version: i64,
}

impl HistoryInput {
    /// 从书籍记录构造（读取路径的常见入口）。
    ///
    /// 「当时的位置」由调用方给出（打开瞬间的进度），因此这里只填书名与来源。
    pub fn from_book(
        book: &crate::books::BookRecord,
        opened_at: i64,
        chapter_index: i64,
        byte_offset: i64,
        entry_kind: EntryKind,
        content_index_version: i64,
    ) -> Self {
        Self {
            book_id: book.id,
            book_title: book.display_name.clone(),
            opened_at,
            chapter_index,
            byte_offset,
            entry_kind,
            source_kind: book.source_kind.clone(),
            source_locator: book.source_locator.clone(),
            content_index_version,
        }
    }
}

/// 记一条流水。
pub fn record(conn: &Connection, input: &HistoryInput) -> Result<i64> {
    if input.book_id <= 0 {
        return Err(StorageError::invalid("书籍 id 必须为正整数"));
    }
    if input.chapter_index < 0 || input.byte_offset < 0 {
        return Err(StorageError::out_of_range("章节序号与字节偏移不得为负"));
    }
    conn.execute(
        "INSERT INTO reading_history
            (book_id, book_title, opened_at, chapter_index, byte_offset,
             entry_kind, source_kind, source_locator, content_index_version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            input.book_id,
            input.book_title,
            input.opened_at,
            input.chapter_index,
            input.byte_offset,
            input.entry_kind.as_i64(),
            input.source_kind,
            input.source_locator,
            input.content_index_version,
        ],
    )
    .map_err(StorageError::from)?;
    Ok(conn.last_insert_rowid())
}

/// 列出流水（时间倒序；同秒用 id 兜底保证稳定）。
pub fn list(conn: &Connection, limit: Option<i64>) -> Result<Vec<HistoryEntry>> {
    // 带 `limit` 时 SQL 加 `LIMIT ?1`（**占位符必须与参数数目一致** ——
    // 早前版本传了 1 个参数却没有占位符，触发
    // `Wrong number of parameters`（V1.1 FFI 接入时实测踩中））。
    //
    // 两条分支的 SQL 不同，故必须各自持有 `statement`；用 `Box` 让它活到
    // 遍历结束（`Rows` 借用它，返回时不能提前 drop）。
    let base = "SELECT id, book_id, book_title, opened_at, chapter_index, byte_offset,
                       entry_kind, source_kind, source_locator, content_index_version
                FROM reading_history
                ORDER BY opened_at DESC, id DESC";
    let mut entries = Vec::new();
    match limit {
        Some(limit) => {
            let sql = format!("{base} LIMIT ?1");
            let mut statement = conn.prepare(&sql).map_err(StorageError::from)?;
            let mut rows = statement
                .query(params![limit.limit_positive()])
                .map_err(StorageError::from)?;
            while let Some(row) = rows.next().map_err(StorageError::from)? {
                entries.push(map_entry(row)?);
            }
        }
        None => {
            let mut statement = conn.prepare(base).map_err(StorageError::from)?;
            let mut rows = statement.query([]).map_err(StorageError::from)?;
            while let Some(row) = rows.next().map_err(StorageError::from)? {
                entries.push(map_entry(row)?);
            }
        }
    }
    Ok(entries)
}

/// 列出某本书的流水（时间倒序）。
pub fn list_by_book(
    conn: &Connection,
    book_id: i64,
    limit: Option<i64>,
) -> Result<Vec<HistoryEntry>> {
    // 带 `limit` 时 SQL 加 `LIMIT ?2` —— 占位符必须与参数数目一致
    // （与 `list` 同类缺陷：早前传了 2 个参数却只有 `?1`）。
    let base = "SELECT id, book_id, book_title, opened_at, chapter_index, byte_offset,
                       entry_kind, source_kind, source_locator, content_index_version
                FROM reading_history
                WHERE book_id = ?1
                ORDER BY opened_at DESC, id DESC";
    let mut entries = Vec::new();
    match limit {
        Some(limit) => {
            let sql = format!("{base} LIMIT ?2");
            let mut statement = conn.prepare(&sql).map_err(StorageError::from)?;
            let mut rows = statement
                .query(params![book_id, limit.limit_positive()])
                .map_err(StorageError::from)?;
            while let Some(row) = rows.next().map_err(StorageError::from)? {
                entries.push(map_entry(row)?);
            }
        }
        None => {
            let mut statement = conn.prepare(base).map_err(StorageError::from)?;
            let mut rows = statement
                .query(params![book_id])
                .map_err(StorageError::from)?;
            while let Some(row) = rows.next().map_err(StorageError::from)? {
                entries.push(map_entry(row)?);
            }
        }
    }
    Ok(entries)
}

/// 按书聚合的一组流水（设计稿 H2）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryGroup {
    pub book_id: i64,
    pub book_title: String,
    /// 该书的流水条数。
    pub count: i64,
    /// 最近一次打开时间（组头排序键）。
    pub last_opened_at: i64,
    /// 该书的全部流水（时间倒序）。
    pub entries: Vec<HistoryEntry>,
}

/// 聚合视图：组按最近时间倒序，组内同样倒序（排序在存储层完成）。
pub fn list_grouped(conn: &Connection) -> Result<Vec<HistoryGroup>> {
    let all = list(conn, None)?;
    let mut order: Vec<i64> = Vec::new();
    let mut buckets: std::collections::HashMap<i64, (String, Vec<HistoryEntry>)> =
        std::collections::HashMap::new();
    // `all` 已按时间倒序，因此每本书首次出现即为最近时间，顺序天然正确。
    for entry in all {
        let bucket = buckets.entry(entry.book_id).or_insert_with(|| {
            order.push(entry.book_id);
            (entry.book_title.clone(), Vec::new())
        });
        bucket.1.push(entry);
    }
    let mut groups = Vec::with_capacity(order.len());
    for book_id in order {
        if let Some((title, entries)) = buckets.get(&book_id) {
            groups.push(HistoryGroup {
                book_id,
                book_title: title.clone(),
                count: entries.len() as i64,
                last_opened_at: entries.first().map(|item| item.opened_at).unwrap_or(0),
                entries: entries.clone(),
            });
        }
    }
    Ok(groups)
}

/// 清除单条流水。返回是否命中。
pub fn remove_one(conn: &Connection, id: i64) -> Result<bool> {
    let changed = conn
        .execute("DELETE FROM reading_history WHERE id = ?1", params![id])
        .map_err(StorageError::from)?;
    Ok(changed > 0)
}

/// 清除某本书的全部流水（设计稿 H2 组头「清除该书的全部流水」）。
pub fn remove_by_book(conn: &Connection, book_id: i64) -> Result<usize> {
    conn.execute(
        "DELETE FROM reading_history WHERE book_id = ?1",
        params![book_id],
    )
    .map_err(StorageError::from)
}

/// 清空全部流水（**只删流水**：不动书籍、进度、书签；规格 R8）。
pub fn clear_all(conn: &Connection) -> Result<usize> {
    conn.execute("DELETE FROM reading_history", [])
        .map_err(StorageError::from)
}

/// 流水总数（清空确认文案的事实来源；设计稿 H3）。
pub fn count(conn: &Connection) -> Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM reading_history", [], |row| row.get(0))
        .map_err(StorageError::from)
}

/// 某本书最近一条流水（书架「继续阅读」与历史入口的默认目标）。
pub fn latest_for_book(conn: &Connection, book_id: i64) -> Result<Option<HistoryEntry>> {
    // 口径：SQL 只含 `?1`（`book_id`），limit 由常量传入而非占位符 ——
    // 早前版本误传了两个参数，触发 `Wrong number of parameters`（实测踩中）。
    let sql = "SELECT id, book_id, book_title, opened_at, chapter_index, byte_offset,
                      entry_kind, source_kind, source_locator, content_index_version
               FROM reading_history
               WHERE book_id = ?1
               ORDER BY opened_at DESC, id DESC
               LIMIT 1";
    // 用 `query_map` 而非 `query_row`：本 crate 的 `Result` 与 `rusqlite::Result`
    // 不同，直接把 `map_entry` 当行映射传会类型不匹配。
    // `statement` 必须绑定为具名局部：`prepare(...)?.query(...)` 的行借用着它，
    // 写进同一个 `let` 会触发 E0716（临时值提前释放）。
    let mut statement = conn.prepare(sql).map_err(StorageError::from)?;
    let mut rows = statement
        .query(params![book_id])
        .map_err(StorageError::from)?;
    match rows.next().map_err(StorageError::from)? {
        Some(row) => Ok(Some(map_entry(row)?)),
        None => Ok(None),
    }
}

fn map_entry(row: &rusqlite::Row<'_>) -> Result<HistoryEntry> {
    Ok(HistoryEntry {
        id: row.get(0)?,
        book_id: row.get(1)?,
        book_title: row.get(2)?,
        opened_at: row.get(3)?,
        chapter_index: row.get(4)?,
        byte_offset: row.get(5)?,
        entry_kind: EntryKind::from_i64(row.get(6)?),
        source_kind: row.get(7)?,
        source_locator: row.get(8)?,
        content_index_version: row.get(9)?,
    })
}

/// 非正上限按「不限」处理（避免 `LIMIT 0` 返回空这种反直觉结果）。
trait LimitPositive {
    fn limit_positive(self) -> i64;
}

impl LimitPositive for i64 {
    fn limit_positive(self) -> i64 {
        if self > 0 {
            self
        } else {
            -1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::books::{
        import_book, BookImport, KIND_DESKTOP_PATH, STATUS_MISSING, STATUS_REMOVED,
    };
    use crate::progress::{save_progress, ProgressInput};
    use crate::Storage;

    fn storage() -> Storage {
        let mut storage = Storage::open_in_memory().expect("内存库");
        storage.migrate().expect("迁移");
        storage
    }

    fn seed_book(conn: &Connection, locator: &str, now: i64) -> i64 {
        import_book(
            conn,
            &BookImport {
                display_name: locator.to_string(),
                original_name: "book.txt".to_string(),
                source_kind: KIND_DESKTOP_PATH.to_string(),
                source_locator: locator.to_string(),
                source_size: 256,
                source_modified_hint: None,
                source_fingerprint: None,
                encoding: "utf-8".to_string(),
                encoding_confidence: 1000,
                encoding_user_override: false,
            },
            now,
        )
        .expect("导入")
        .book_id
    }

    fn input(book_id: i64, title: &str, opened_at: i64) -> HistoryInput {
        HistoryInput {
            book_id,
            book_title: title.to_string(),
            opened_at,
            chapter_index: 2,
            byte_offset: 4096,
            entry_kind: EntryKind::FromShelf,
            source_kind: KIND_DESKTOP_PATH.to_string(),
            source_locator: "/tmp/book.txt".to_string(),
            content_index_version: 1,
        }
    }

    #[test]
    fn three_opens_produce_three_rows_in_descending_time_order() {
        let storage = storage();
        let conn = storage.conn();
        let first = seed_book(conn, "/tmp/1.txt", 1);
        let second = seed_book(conn, "/tmp/2.txt", 2);
        let third = seed_book(conn, "/tmp/3.txt", 3);

        record(conn, &input(first, "甲", 100)).expect("记 1");
        record(conn, &input(second, "乙", 200)).expect("记 2");
        record(conn, &input(third, "丙", 300)).expect("记 3");

        let entries = list(conn, None).expect("列表");
        assert_eq!(entries.len(), 3);
        assert_eq!(
            entries
                .iter()
                .map(|item| item.book_title.as_str())
                .collect::<Vec<_>>(),
            vec!["丙", "乙", "甲"]
        );
        assert_eq!(count(conn).expect("计数"), 3);
    }

    #[test]
    fn entry_records_position_and_kind_faithfully() {
        let storage = storage();
        let conn = storage.conn();
        let book = seed_book(conn, "/tmp/1.txt", 1);
        let mut payload = input(book, "甲", 100);
        payload.entry_kind = EntryKind::BookmarkJump;
        payload.chapter_index = 7;
        payload.byte_offset = 65536;
        record(conn, &payload).expect("记");

        let entry = list(conn, None).expect("列表").remove(0);
        assert_eq!(entry.entry_kind, EntryKind::BookmarkJump);
        assert_eq!(entry.entry_kind.label(), "书签跳转");
        assert_eq!(entry.chapter_index, 7);
        assert_eq!(entry.byte_offset, 65536);
        assert_eq!(entry.content_index_version, 1);
    }

    #[test]
    fn all_entry_kinds_round_trip() {
        for kind in [
            EntryKind::FromShelf,
            EntryKind::ReopenInReader,
            EntryKind::ResumeProgress,
            EntryKind::BookmarkJump,
            EntryKind::ChapterJump,
        ] {
            assert_eq!(EntryKind::from_i64(kind.as_i64()), kind);
            assert!(!kind.label().is_empty());
        }
        // 未知值向前兼容。
        assert_eq!(EntryKind::from_i64(99), EntryKind::FromShelf);
    }

    #[test]
    fn soft_removal_keeps_history_and_purge_cascades_it() {
        let storage = storage();
        let conn = storage.conn();
        let kept = seed_book(conn, "/tmp/kept.txt", 1);
        let purged = seed_book(conn, "/tmp/purged.txt", 2);
        record(conn, &input(kept, "保留", 100)).expect("记");
        record(conn, &input(purged, "删除", 200)).expect("记");

        // 软移除：状态改 removed，历史仍在。
        crate::books::set_book_status(conn, kept, STATUS_REMOVED, 3).expect("软移除");
        assert_eq!(count(conn).expect("计数"), 2, "软移除不应删历史");

        // 硬删除：级联删历史。
        crate::books::purge_book_record(conn, purged).expect("硬删除");
        let entries = list(conn, None).expect("列表");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].book_id, kept);
    }

    #[test]
    fn clearing_history_leaves_books_progress_and_bookmarks_untouched() {
        let storage = storage();
        let conn = storage.conn();
        let book = seed_book(conn, "/tmp/1.txt", 1);
        save_progress(conn, &ProgressInput::at(book, 1, 128), 2).expect("写进度");
        conn.execute(
            "INSERT INTO bookmarks (book_id, chapter_index, byte_offset, title, created_at, updated_at)
             VALUES (?1, 0, 0, '标记', 3, 3)",
            params![book],
        )
        .expect("写书签");
        record(conn, &input(book, "甲", 100)).expect("记");
        record(conn, &input(book, "甲", 200)).expect("记");

        let before = snapshot(conn);
        let cleared = clear_all(conn).expect("清空");
        assert_eq!(cleared, 2);
        assert_eq!(count(conn).expect("计数"), 0);

        let after = snapshot(conn);
        assert_eq!(before, after, "清空流水不得影响书籍/进度/书签");
    }

    #[test]
    fn remove_one_and_remove_by_book_only_touch_history() {
        let storage = storage();
        let conn = storage.conn();
        let first = seed_book(conn, "/tmp/1.txt", 1);
        let second = seed_book(conn, "/tmp/2.txt", 2);
        let one = record(conn, &input(first, "甲", 100)).expect("记");
        record(conn, &input(first, "甲", 200)).expect("记");
        record(conn, &input(second, "乙", 300)).expect("记");

        assert!(remove_one(conn, one).expect("删单条"));
        assert!(!remove_one(conn, one).expect("重复删应返回 false"));
        assert_eq!(count(conn).expect("计数"), 2);

        assert_eq!(remove_by_book(conn, first).expect("按书删"), 1);
        assert_eq!(count(conn).expect("计数"), 1);
    }

    #[test]
    fn grouped_view_orders_groups_and_entries_by_recency() {
        let storage = storage();
        let conn = storage.conn();
        let first = seed_book(conn, "/tmp/1.txt", 1);
        let second = seed_book(conn, "/tmp/2.txt", 2);
        record(conn, &input(first, "甲", 100)).expect("记");
        record(conn, &input(second, "乙", 300)).expect("记");
        record(conn, &input(first, "甲", 200)).expect("记");

        let groups = list_grouped(conn).expect("聚合");
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].book_id, second, "最近的组排前面");
        assert_eq!(groups[1].book_id, first);
        assert_eq!(groups[1].count, 2);
        assert_eq!(groups[1].last_opened_at, 200);
        assert_eq!(
            groups[1]
                .entries
                .iter()
                .map(|item| item.opened_at)
                .collect::<Vec<_>>(),
            vec![200, 100]
        );
    }

    #[test]
    fn latest_for_book_returns_most_recent_entry() {
        let storage = storage();
        let conn = storage.conn();
        let book = seed_book(conn, "/tmp/1.txt", 1);
        assert!(latest_for_book(conn, book).expect("查").is_none());
        record(conn, &input(book, "甲", 100)).expect("记");
        record(conn, &input(book, "甲", 900)).expect("记");
        let latest = latest_for_book(conn, book).expect("查").expect("有记录");
        assert_eq!(latest.opened_at, 900);
    }

    #[test]
    fn rejects_negative_position_and_invalid_book_id() {
        let storage = storage();
        let conn = storage.conn();
        let book = seed_book(conn, "/tmp/1.txt", 1);
        let mut bad = input(book, "甲", 100);
        bad.byte_offset = -1;
        assert!(record(conn, &bad).is_err(), "负偏移应被拒");
        let mut bad = input(book, "甲", 100);
        bad.book_id = 0;
        assert!(record(conn, &bad).is_err(), "非法书籍 id 应被拒");
    }

    /// 书籍 / 进度 / 书签三表行数快照（清空流水不得改变它们）。
    fn snapshot(conn: &Connection) -> (i64, i64, i64) {
        let books: i64 = conn
            .query_row("SELECT COUNT(*) FROM books", [], |row| row.get(0))
            .expect("books");
        let progress: i64 = conn
            .query_row("SELECT COUNT(*) FROM reading_progress", [], |row| {
                row.get(0)
            })
            .expect("progress");
        let bookmarks: i64 = conn
            .query_row("SELECT COUNT(*) FROM bookmarks", [], |row| row.get(0))
            .expect("bookmarks");
        (books, progress, bookmarks)
    }

    #[test]
    fn entry_kind_labels_cover_design_doc_tags() {
        // 设计稿 H1：阅读页打开（b）/ 进度恢复（g）/ 书签跳转（g）/ 章节跳转（g）。
        assert_eq!(EntryKind::FromShelf.label(), "书架打开");
        assert_eq!(EntryKind::ReopenInReader.label(), "阅读页打开");
        assert_eq!(EntryKind::ResumeProgress.label(), "进度恢复");
        assert_eq!(EntryKind::BookmarkJump.label(), "书签跳转");
        assert_eq!(EntryKind::ChapterJump.label(), "章节跳转");
    }

    #[test]
    fn status_missing_books_keep_history_intact() {
        let storage = storage();
        let conn = storage.conn();
        let book = seed_book(conn, "/tmp/1.txt", 1);
        record(conn, &input(book, "甲", 100)).expect("记");
        crate::books::set_book_status(conn, book, STATUS_MISSING, 2).expect("置失效");
        assert_eq!(count(conn).expect("计数"), 1, "源失效不影响历史");
    }
}

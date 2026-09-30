//! `books` / `book_sources` 仓储：导入 upsert、重复导入、书架状态、移除与清除。
//!
//! 语义裁定（本卡冻结，详见 `results/rc6-storage.md`）：
//! - **导入 upsert**：以 `(source_kind, source_locator)` 为第一身份；找不到时按
//!   `source_fingerprint` 认领（同一文件从不同路径再次导入 ⇒ 复用同一书籍条目，
//!   并在 `book_sources` 增加一条来源），完全找不到才新建；
//! - **移除书架记录**（[`remove_book_from_shelf`]）是**软移除**：只把 `books.status`
//!   置为 `removed` 并清掉该书可重建索引；**进度、书签、来源记录全部保留**
//!   （用户重新导入同一文件即可恢复阅读位置），且永不触碰用户原始 TXT；
//! - **硬删除**（[`purge_book_record`]）只在显式调用时发生，仍**不删除原始文件**。

use rusqlite::{Connection, OptionalExtension};

use crate::error::{Result, StorageError};
use crate::index;
use crate::validate::{
    ensure_max_len, ensure_non_negative, ensure_one_of, ensure_short_text, MAX_ENCODING_BYTES,
    MAX_FINGERPRINT_BYTES, MAX_LOCATOR_BYTES, MAX_NAME_BYTES,
};

/// 来源种类：桌面路径。
pub const KIND_DESKTOP_PATH: &str = "desktop_path";
/// 来源种类：Android SAF URI。
pub const KIND_ANDROID_URI: &str = "android_uri";
/// 来源种类：Android 私有副本（Provider 不支持可靠 seek 时的落盘副本）。
pub const KIND_PRIVATE_COPY: &str = "private_copy";
/// 允许的来源种类。
pub const SOURCE_KINDS: &[&str] = &[KIND_DESKTOP_PATH, KIND_ANDROID_URI, KIND_PRIVATE_COPY];

/// 书架状态：正常可读。
pub const STATUS_READY: &str = "ready";
/// 书架状态：原文件失效（保留记录、进度和书签，允许重新关联）。
pub const STATUS_MISSING: &str = "missing";
/// 书架状态：索引过期，需要重建。
pub const STATUS_NEEDS_REINDEX: &str = "needs_reindex";
/// 书架状态：导入失败。
pub const STATUS_IMPORT_FAILED: &str = "import_failed";
/// 书架状态：已从书架移除（软移除；用户元数据保留）。
pub const STATUS_REMOVED: &str = "removed";
/// 允许的书架状态。
pub const BOOK_STATUSES: &[&str] = &[
    STATUS_READY,
    STATUS_MISSING,
    STATUS_NEEDS_REINDEX,
    STATUS_IMPORT_FAILED,
    STATUS_REMOVED,
];

/// V1 固定为 txt。
pub const FORMAT_TXT: &str = "txt";
/// 编码置信度上限（千分比，与 reading-source 编码识别口径一致）。
pub const MAX_ENCODING_CONFIDENCE: i32 = 1000;

const BOOK_COLUMNS: &str = "id, display_name, original_name, format, source_kind, source_locator, \
     source_size, source_modified_hint, source_fingerprint, encoding, encoding_confidence, \
     encoding_user_override, status, created_at, updated_at";

/// 导入请求（一次「添加 TXT」的完整事实）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookImport {
    pub display_name: String,
    pub original_name: String,
    pub source_kind: String,
    pub source_locator: String,
    pub source_size: i64,
    pub source_modified_hint: Option<i64>,
    pub source_fingerprint: Option<String>,
    pub encoding: String,
    pub encoding_confidence: i32,
    pub encoding_user_override: bool,
}

/// `books` 行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookRecord {
    pub id: i64,
    pub display_name: String,
    pub original_name: String,
    pub format: String,
    pub source_kind: String,
    pub source_locator: String,
    pub source_size: i64,
    pub source_modified_hint: Option<i64>,
    pub source_fingerprint: Option<String>,
    pub encoding: String,
    pub encoding_confidence: i32,
    pub encoding_user_override: bool,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl BookRecord {
    /// 是否在书架可见（软移除的条目不出现在书架列表）。
    pub fn is_on_shelf(&self) -> bool {
        self.status != STATUS_REMOVED
    }
}

/// `book_sources` 行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookSourceRecord {
    pub id: i64,
    pub book_id: i64,
    pub source_kind: String,
    pub source_locator: String,
    pub source_size: i64,
    pub source_fingerprint: Option<String>,
    pub is_primary: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 导入结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportOutcome {
    pub book_id: i64,
    /// 是否新建书籍条目。
    pub created: bool,
    /// 是否新增来源记录（同一书从新路径再次导入时为 true）。
    pub source_created: bool,
    /// 书籍条目是否发生变化（完全重复导入为 false ⇒ 幂等可断言）。
    pub updated: bool,
}

impl ImportOutcome {
    /// 是否发生了任何数据库变更（完全重复导入返回 false）。
    pub fn changed_any(&self) -> bool {
        self.created || self.updated || self.source_created
    }
}

/// 软移除报告。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShelfRemovalReport {
    pub book_id: i64,
    pub status: String,
    pub chapters_deleted: i64,
    pub layout_records_deleted: i64,
    pub progress_preserved: bool,
    pub bookmarks_preserved: i64,
    pub sources_preserved: i64,
    pub visible_shelf_rows: i64,
}

fn validate_import(import: &BookImport) -> Result<()> {
    ensure_short_text("books.display_name", &import.display_name, MAX_NAME_BYTES)?;
    ensure_short_text("books.original_name", &import.original_name, MAX_NAME_BYTES)?;
    ensure_one_of("books.source_kind", &import.source_kind, SOURCE_KINDS)?;
    ensure_short_text(
        "books.source_locator",
        &import.source_locator,
        MAX_LOCATOR_BYTES,
    )?;
    ensure_max_len("books.encoding", &import.encoding, MAX_ENCODING_BYTES)?;
    ensure_non_negative("books.source_size", import.source_size)?;
    if import.encoding_confidence < 0 || import.encoding_confidence > MAX_ENCODING_CONFIDENCE {
        return Err(StorageError::invalid(format!(
            "books.encoding_confidence 必须在 0..={MAX_ENCODING_CONFIDENCE}，实际 {}",
            import.encoding_confidence
        )));
    }
    if let Some(hint) = import.source_modified_hint {
        ensure_non_negative("books.source_modified_hint", hint)?;
    }
    if let Some(fingerprint) = &import.source_fingerprint {
        ensure_max_len(
            "books.source_fingerprint",
            fingerprint,
            MAX_FINGERPRINT_BYTES,
        )?;
    }
    Ok(())
}

fn book_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<BookRecord> {
    Ok(BookRecord {
        id: row.get(0)?,
        display_name: row.get(1)?,
        original_name: row.get(2)?,
        format: row.get(3)?,
        source_kind: row.get(4)?,
        source_locator: row.get(5)?,
        source_size: row.get(6)?,
        source_modified_hint: row.get(7)?,
        source_fingerprint: row.get(8)?,
        encoding: row.get(9)?,
        encoding_confidence: row.get(10)?,
        encoding_user_override: row.get::<_, i64>(11)? != 0,
        status: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}

/// 按主键读取书籍（不存在返回 `None`）。
pub fn get_book(conn: &Connection, book_id: i64) -> Result<Option<BookRecord>> {
    ensure_non_negative("book_id", book_id)?;
    let sql = format!("SELECT {BOOK_COLUMNS} FROM books WHERE id = ?1");
    let book = conn.query_row(&sql, [book_id], book_from_row).optional()?;
    Ok(book)
}

/// 按来源身份查找书籍。
pub fn find_book_by_locator(
    conn: &Connection,
    source_kind: &str,
    source_locator: &str,
) -> Result<Option<BookRecord>> {
    ensure_one_of("source_kind", source_kind, SOURCE_KINDS)?;
    ensure_short_text("source_locator", source_locator, MAX_LOCATOR_BYTES)?;
    // 同时查主来源（books）与历史来源（book_sources）：重新关联 / 换路径后
    // 旧定位符仍然认得同一条目，避免用户再次导入同一文件时产生重复书籍。
    let sql = format!(
        "SELECT {BOOK_COLUMNS} FROM books \
         WHERE (source_kind = ?1 AND source_locator = ?2) \
         OR EXISTS (SELECT 1 FROM book_sources s WHERE s.book_id = books.id \
                    AND s.source_kind = ?1 AND s.source_locator = ?2) \
         ORDER BY id LIMIT 1"
    );
    let book = conn
        .query_row(&sql, [source_kind, source_locator], book_from_row)
        .optional()?;
    Ok(book)
}

/// 按文件指纹查找书籍（内容相同、路径不同的重复导入）。
pub fn find_book_by_fingerprint(
    conn: &Connection,
    fingerprint: &str,
) -> Result<Option<BookRecord>> {
    ensure_short_text("source_fingerprint", fingerprint, MAX_FINGERPRINT_BYTES)?;
    let sql = format!(
        "SELECT {BOOK_COLUMNS} FROM books WHERE source_fingerprint = ?1 ORDER BY id LIMIT 1"
    );
    let book = conn
        .query_row(&sql, [fingerprint], book_from_row)
        .optional()?;
    Ok(book)
}

/// 书架可见条目（`status != 'removed'`），按最近打开时间倒序（无进度时用创建时间）。
pub fn list_shelf_books(conn: &Connection) -> Result<Vec<BookRecord>> {
    let sql = format!(
        "SELECT {BOOK_COLUMNS} FROM books WHERE status <> ?1 \
         ORDER BY COALESCE((SELECT last_opened_at FROM reading_progress WHERE book_id = books.id), created_at) DESC, id DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([STATUS_REMOVED], book_from_row)?;
    let mut books = Vec::new();
    for row in rows {
        books.push(row?);
    }
    Ok(books)
}

/// 全部条目（含软移除），按 id 升序。
pub fn list_all_books(conn: &Connection) -> Result<Vec<BookRecord>> {
    let sql = format!("SELECT {BOOK_COLUMNS} FROM books ORDER BY id");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], book_from_row)?;
    let mut books = Vec::new();
    for row in rows {
        books.push(row?);
    }
    Ok(books)
}

/// 书籍条目总数。
pub fn count_books(conn: &Connection) -> Result<i64> {
    crate::connection::count_rows(conn, "books")
}

/// 指定书籍的来源记录。
pub fn list_book_sources(conn: &Connection, book_id: i64) -> Result<Vec<BookSourceRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, book_id, source_kind, source_locator, source_size, source_fingerprint, \
         is_primary, created_at, updated_at FROM book_sources WHERE book_id = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map([book_id], |row| {
        Ok(BookSourceRecord {
            id: row.get(0)?,
            book_id: row.get(1)?,
            source_kind: row.get(2)?,
            source_locator: row.get(3)?,
            source_size: row.get(4)?,
            source_fingerprint: row.get(5)?,
            is_primary: row.get::<_, i64>(6)? != 0,
            created_at: row.get(7)?,
            updated_at: row.get(8)?,
        })
    })?;
    let mut sources = Vec::new();
    for row in rows {
        sources.push(row?);
    }
    Ok(sources)
}

fn source_exists(conn: &Connection, book_id: i64, import: &BookImport) -> Result<bool> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT id FROM book_sources WHERE book_id = ?1 AND source_kind = ?2 AND source_locator = ?3",
            rusqlite::params![book_id, import.source_kind, import.source_locator],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

fn upsert_source(tx: &Connection, book_id: i64, import: &BookImport, now: i64) -> Result<()> {
    tx.execute(
        "INSERT INTO book_sources (book_id, source_kind, source_locator, source_size, \
         source_modified_hint, source_fingerprint, encoding, encoding_confidence, \
         encoding_user_override, is_primary, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, ?10, ?10) \
         ON CONFLICT (book_id, source_kind, source_locator) DO UPDATE SET \
         source_size = excluded.source_size, source_modified_hint = excluded.source_modified_hint, \
         source_fingerprint = excluded.source_fingerprint, encoding = excluded.encoding, \
         encoding_confidence = excluded.encoding_confidence, \
         encoding_user_override = excluded.encoding_user_override, updated_at = excluded.updated_at",
        rusqlite::params![
            book_id,
            import.source_kind,
            import.source_locator,
            import.source_size,
            import.source_modified_hint,
            import.source_fingerprint,
            import.encoding,
            import.encoding_confidence,
            i64::from(import.encoding_user_override),
            now
        ],
    )?;
    // 最近一次导入的来源为主来源。
    tx.execute(
        "UPDATE book_sources SET is_primary = CASE WHEN source_kind = ?2 AND source_locator = ?3 \
         THEN 1 ELSE 0 END WHERE book_id = ?1",
        rusqlite::params![book_id, import.source_kind, import.source_locator],
    )?;
    Ok(())
}

/// 导入 upsert：幂等、不产生重复条目、不触碰用户文件。
pub fn import_book(conn: &Connection, import: &BookImport, now: i64) -> Result<ImportOutcome> {
    validate_import(import)?;
    let tx = conn.unchecked_transaction()?;

    let by_locator = find_book_by_locator(&tx, &import.source_kind, &import.source_locator)?;
    let existing = match by_locator {
        Some(book) => Some(book),
        None => match import.source_fingerprint.as_deref() {
            Some(fingerprint) if !fingerprint.is_empty() => {
                find_book_by_fingerprint(&tx, fingerprint)?
            }
            _ => None,
        },
    };

    let outcome = match existing {
        Some(book) => {
            // 内容（指纹）或编码口径变化 ⇒ 可重建索引失效（章节区间与分页边界都不再可信），
            // 但书籍、进度、书签一律保留（design.md「编码或原始文件指纹变化只使可重建索引失效」）。
            let index_invalidated = book.source_fingerprint != import.source_fingerprint
                || book.encoding != import.encoding;
            let restored_from_removal = book.status == STATUS_REMOVED;
            let next_status = if restored_from_removal {
                STATUS_READY.to_string()
            } else if index_invalidated {
                STATUS_NEEDS_REINDEX.to_string()
            } else {
                book.status.clone()
            };
            let fields_unchanged = book.display_name == import.display_name
                && book.source_kind == import.source_kind
                && book.source_locator == import.source_locator
                && book.source_size == import.source_size
                && book.source_modified_hint == import.source_modified_hint
                && book.source_fingerprint == import.source_fingerprint
                && book.encoding == import.encoding
                && book.encoding_confidence == import.encoding_confidence
                && book.encoding_user_override == import.encoding_user_override;
            let unchanged = fields_unchanged && next_status == book.status;
            let had_source = source_exists(&tx, book.id, import)?;
            if !unchanged {
                tx.execute(
                    "UPDATE books SET display_name = ?2, source_kind = ?3, source_locator = ?4, \
                     source_size = ?5, source_modified_hint = ?6, source_fingerprint = ?7, \
                     encoding = ?8, encoding_confidence = ?9, encoding_user_override = ?10, \
                     status = ?11, updated_at = ?12 WHERE id = ?1",
                    rusqlite::params![
                        book.id,
                        import.display_name,
                        import.source_kind,
                        import.source_locator,
                        import.source_size,
                        import.source_modified_hint,
                        import.source_fingerprint,
                        import.encoding,
                        import.encoding_confidence,
                        i64::from(import.encoding_user_override),
                        next_status,
                        now
                    ],
                )?;
            }
            if index_invalidated && !restored_from_removal {
                // 只删可重建数据，并推进内容索引版本（旧分页缓存随之失效）。
                crate::index::clear_chapters_for_book(&tx, book.id)?;
                crate::index::clear_layout_records_for_book(&tx, book.id)?;
                crate::meta::bump_content_index_version(&tx, now)?;
            }
            upsert_source(&tx, book.id, import, now)?;
            ImportOutcome {
                book_id: book.id,
                created: false,
                source_created: !had_source,
                updated: !unchanged,
            }
        }
        None => {
            tx.execute(
                "INSERT INTO books (display_name, original_name, format, source_kind, \
                 source_locator, source_size, source_modified_hint, source_fingerprint, encoding, \
                 encoding_confidence, encoding_user_override, status, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)",
                rusqlite::params![
                    import.display_name,
                    import.original_name,
                    FORMAT_TXT,
                    import.source_kind,
                    import.source_locator,
                    import.source_size,
                    import.source_modified_hint,
                    import.source_fingerprint,
                    import.encoding,
                    import.encoding_confidence,
                    i64::from(import.encoding_user_override),
                    STATUS_READY,
                    now
                ],
            )?;
            let book_id = tx.last_insert_rowid();
            upsert_source(&tx, book_id, import, now)?;
            ImportOutcome {
                book_id,
                created: true,
                source_created: true,
                updated: true,
            }
        }
    };

    tx.commit()?;
    Ok(outcome)
}

/// 覆盖一本书的内容编码（**用户手动选择**）。
///
/// 语义与契约：
/// - 只改变「如何解释文件字节」，**永不写回原始文件**（与核心 `EncodingState::override_encoding` 同一口径）；
/// - `encoding` 取 `TextEncoding` 的展示名（`UTF-8` / `UTF-8-BOM` / `UTF-16LE` / `UTF-16BE` / `GB18030` / `GBK` / `Big5`）；
/// - 写入 `encoding_user_override = 1`，以便界面区分「识别」与「用户指定」；
/// - 调用方**必须**同时递增内容索引版本（`meta::bump_content_index_version`），
///   否则目录缓存与分页边界仍按旧编码口径复用 —— 那正是「改了编码但正文没变」的成因。
pub fn set_encoding(
    conn: &Connection,
    book_id: i64,
    encoding: &str,
    confidence: i32,
    now: i64,
) -> Result<()> {
    ensure_max_len("books.encoding", encoding, MAX_ENCODING_BYTES)?;
    if !(0..=MAX_ENCODING_CONFIDENCE).contains(&confidence) {
        return Err(StorageError::invalid(format!(
            "books.encoding_confidence 必须在 0..={MAX_ENCODING_CONFIDENCE}，实际 {confidence}"
        )));
    }
    let affected = conn.execute(
        "UPDATE books SET encoding = ?2, encoding_confidence = ?3, encoding_user_override = 1, \
         updated_at = ?4 WHERE id = ?1",
        rusqlite::params![book_id, encoding, confidence, now],
    )?;
    if affected == 0 {
        return Err(StorageError::BookNotFound { book_id });
    }
    Ok(())
}

/// 设置书架状态。
pub fn set_book_status(conn: &Connection, book_id: i64, status: &str, now: i64) -> Result<()> {
    ensure_one_of("books.status", status, BOOK_STATUSES)?;
    let affected = conn.execute(
        "UPDATE books SET status = ?2, updated_at = ?3 WHERE id = ?1",
        rusqlite::params![book_id, status, now],
    )?;
    if affected == 0 {
        return Err(StorageError::BookNotFound { book_id });
    }
    Ok(())
}

/// 重命名书架显示名（不触碰原始文件）。
pub fn rename_book(conn: &Connection, book_id: i64, display_name: &str, now: i64) -> Result<()> {
    ensure_short_text("books.display_name", display_name, MAX_NAME_BYTES)?;
    let affected = conn.execute(
        "UPDATE books SET display_name = ?2, updated_at = ?3 WHERE id = ?1",
        rusqlite::params![book_id, display_name, now],
    )?;
    if affected == 0 {
        return Err(StorageError::BookNotFound { book_id });
    }
    Ok(())
}

/// 从书架移除（软移除）：保留进度、书签、来源记录；清掉可重建索引；不删除原始文件。
pub fn remove_book_from_shelf(
    conn: &Connection,
    book_id: i64,
    now: i64,
) -> Result<ShelfRemovalReport> {
    if get_book(conn, book_id)?.is_none() {
        return Err(StorageError::BookNotFound { book_id });
    }
    let tx = conn.unchecked_transaction()?;
    let chapters_deleted = index::clear_chapters_for_book(&tx, book_id)?;
    let layout_records_deleted = index::clear_layout_records_for_book(&tx, book_id)?;
    tx.execute(
        "UPDATE books SET status = ?2, updated_at = ?3 WHERE id = ?1",
        rusqlite::params![book_id, STATUS_REMOVED, now],
    )?;
    let progress_preserved = crate::progress::load_progress(&tx, book_id)?.is_some();
    let bookmarks_preserved = crate::bookmarks::count_bookmarks(&tx, book_id)?;
    let sources_preserved = list_book_sources(&tx, book_id)?.len() as i64;
    tx.commit()?;
    Ok(ShelfRemovalReport {
        book_id,
        status: STATUS_REMOVED.to_string(),
        chapters_deleted,
        layout_records_deleted,
        progress_preserved,
        bookmarks_preserved,
        sources_preserved,
        visible_shelf_rows: count_books(conn)? - removed_book_count(conn)?,
    })
}

/// 软移除条目数。
pub fn removed_book_count(conn: &Connection) -> Result<i64> {
    let mut stmt = conn.prepare("SELECT COUNT(*) FROM books WHERE status = ?1")?;
    let count: i64 = stmt.query_row([STATUS_REMOVED], |row| row.get(0))?;
    Ok(count)
}

/// 硬删除数据库记录（级联 `book_sources` / `chapters` / `reading_progress` /
/// `bookmarks` / `layout_records`）。**不删除用户原始文件**。
pub fn purge_book_record(conn: &Connection, book_id: i64) -> Result<()> {
    if get_book(conn, book_id)?.is_none() {
        return Err(StorageError::BookNotFound { book_id });
    }
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM books WHERE id = ?1", [book_id])?;
    tx.commit()?;
    Ok(())
}

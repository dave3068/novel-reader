//! `bookmarks` 仓储：以内容位置为准的书签 CRUD。
//!
//! 书签以内容位置为准，不绑定动态页码（gate-0 §2.4 冻结规则）；
//! `title` 只允许短摘要（名称），V1 不支持正文批注。

use rusqlite::{Connection, OptionalExtension};

use crate::error::Result;
use crate::validate::{
    ensure_max_len, ensure_non_negative, ensure_short_text, MAX_FINGERPRINT_BYTES, MAX_TITLE_BYTES,
};

/// 书签输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookmarkInput {
    pub book_id: i64,
    pub content_unit_id: i64,
    pub chapter_index: i64,
    pub byte_offset: i64,
    pub nearby_fingerprint: Option<String>,
    pub title: String,
}

impl BookmarkInput {
    /// 便捷构造：content_unit_id=0、无指纹。
    pub fn at(
        book_id: i64,
        chapter_index: i64,
        byte_offset: i64,
        title: impl Into<String>,
    ) -> Self {
        Self {
            book_id,
            content_unit_id: 0,
            chapter_index,
            byte_offset,
            nearby_fingerprint: None,
            title: title.into(),
        }
    }
}

/// `bookmarks` 行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookmarkRecord {
    pub id: i64,
    pub book_id: i64,
    pub content_unit_id: i64,
    pub chapter_index: i64,
    pub byte_offset: i64,
    pub nearby_fingerprint: Option<String>,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
}

const BOOKMARK_COLUMNS: &str = "id, book_id, content_unit_id, chapter_index, byte_offset, \
     nearby_fingerprint, title, created_at, updated_at";

fn bookmark_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<BookmarkRecord> {
    Ok(BookmarkRecord {
        id: row.get(0)?,
        book_id: row.get(1)?,
        content_unit_id: row.get(2)?,
        chapter_index: row.get(3)?,
        byte_offset: row.get(4)?,
        nearby_fingerprint: row.get(5)?,
        title: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn validate(input: &BookmarkInput) -> Result<()> {
    ensure_non_negative("bookmarks.book_id", input.book_id)?;
    ensure_non_negative("bookmarks.content_unit_id", input.content_unit_id)?;
    ensure_non_negative("bookmarks.chapter_index", input.chapter_index)?;
    ensure_non_negative("bookmarks.byte_offset", input.byte_offset)?;
    // 书签名称允许为空（默认按章节/位置命名），但不得超长（不得藏正文）。
    ensure_max_len("bookmarks.title", &input.title, MAX_TITLE_BYTES)?;
    if let Some(fingerprint) = &input.nearby_fingerprint {
        ensure_max_len(
            "bookmarks.nearby_fingerprint",
            fingerprint,
            MAX_FINGERPRINT_BYTES,
        )?;
    }
    Ok(())
}

/// 新增书签，返回新书签 id。
pub fn create_bookmark(conn: &Connection, input: &BookmarkInput, now: i64) -> Result<i64> {
    validate(input)?;
    conn.execute(
        "INSERT INTO bookmarks (book_id, content_unit_id, chapter_index, byte_offset, \
         nearby_fingerprint, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        rusqlite::params![
            input.book_id,
            input.content_unit_id,
            input.chapter_index,
            input.byte_offset,
            input.nearby_fingerprint,
            input.title,
            now
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// 读取书签。
pub fn get_bookmark(conn: &Connection, bookmark_id: i64) -> Result<Option<BookmarkRecord>> {
    let sql = format!("SELECT {BOOKMARK_COLUMNS} FROM bookmarks WHERE id = ?1");
    let record = conn
        .query_row(&sql, [bookmark_id], bookmark_from_row)
        .optional()?;
    Ok(record)
}

/// 某书全部书签（按字节偏移升序）。
pub fn list_bookmarks(conn: &Connection, book_id: i64) -> Result<Vec<BookmarkRecord>> {
    let sql = format!(
        "SELECT {BOOKMARK_COLUMNS} FROM bookmarks WHERE book_id = ?1 ORDER BY byte_offset, id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([book_id], bookmark_from_row)?;
    let mut bookmarks = Vec::new();
    for row in rows {
        bookmarks.push(row?);
    }
    Ok(bookmarks)
}

/// 某书书签数。
pub fn count_bookmarks(conn: &Connection, book_id: i64) -> Result<i64> {
    let mut stmt = conn.prepare("SELECT COUNT(*) FROM bookmarks WHERE book_id = ?1")?;
    let count: i64 = stmt.query_row([book_id], |row| row.get(0))?;
    Ok(count)
}

/// 书签总数。
pub fn count_all_bookmarks(conn: &Connection) -> Result<i64> {
    crate::connection::count_rows(conn, "bookmarks")
}

/// 重命名书签（更新 `updated_at`）；返回是否命中。
pub fn rename_bookmark(conn: &Connection, bookmark_id: i64, title: &str, now: i64) -> Result<bool> {
    ensure_max_len("bookmarks.title", title, MAX_TITLE_BYTES)?;
    let affected = conn.execute(
        "UPDATE bookmarks SET title = ?2, updated_at = ?3 WHERE id = ?1",
        rusqlite::params![bookmark_id, title, now],
    )?;
    Ok(affected > 0)
}

/// 删除书签；返回是否确有删除。
pub fn delete_bookmark(conn: &Connection, bookmark_id: i64) -> Result<bool> {
    let affected = conn.execute("DELETE FROM bookmarks WHERE id = ?1", [bookmark_id])?;
    Ok(affected > 0)
}

/// 删除某书全部书签（显式用户操作）。
pub fn delete_bookmarks_for_book(conn: &Connection, book_id: i64) -> Result<i64> {
    let affected = conn.execute("DELETE FROM bookmarks WHERE book_id = ?1", [book_id])?;
    Ok(affected as i64)
}

/// 书签名称非空校验（UI 需要展示名称时使用）。
pub fn ensure_non_empty_title(title: &str) -> Result<()> {
    ensure_short_text("bookmarks.title", title, MAX_TITLE_BYTES)
}

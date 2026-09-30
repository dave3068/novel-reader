//! 可重建索引仓储：`chapters`（章节区间）与 `layout_records`（分页边界缓存）。
//!
//! 这两张表都是**可重建数据**：清理它们不得影响书籍、进度、书签、设置与元信息
//! （gate-0 §2.6 冻结规则、开发方案 §9.2）。

use rusqlite::{Connection, OptionalExtension};

use crate::connection;
use crate::error::{Result, StorageError};
use crate::validate::{
    ensure_max_len, ensure_non_negative, ensure_one_of, ensure_short_text, MAX_SIGNATURE_BYTES,
    MAX_TITLE_BYTES,
};

/// 章节切分规则标识上限（字节）。
pub const MAX_RULE_ID_BYTES: usize = 64;

/// `layout_records.status`：部分生成。
pub const LAYOUT_PARTIAL: &str = "partial";
/// `layout_records.status`：已生成完毕。
pub const LAYOUT_COMPLETE: &str = "complete";
/// `layout_records.status`：签名已过期。
pub const LAYOUT_STALE: &str = "stale";
/// `layout_records.status`：生成失败。
pub const LAYOUT_FAILED: &str = "failed";
/// 允许的布局记录状态。
pub const LAYOUT_STATUSES: &[&str] =
    &[LAYOUT_PARTIAL, LAYOUT_COMPLETE, LAYOUT_STALE, LAYOUT_FAILED];

/// 章节输入（章节名是短摘要，不是正文）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterInput {
    pub sequence: i64,
    pub title: String,
    pub start_byte: i64,
    pub end_byte: i64,
    pub rule_id: String,
}

/// `chapters` 行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterRecord {
    pub id: i64,
    pub book_id: i64,
    pub sequence: i64,
    pub title: String,
    pub start_byte: i64,
    pub end_byte: i64,
    pub rule_id: String,
    pub index_version: i64,
    pub created_at: i64,
}

/// 分页边界记录输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutRecordInput {
    pub book_id: i64,
    /// reading-core `LayoutSignature` 的稳定文本编码（本 crate 只当不透明键使用）。
    pub layout_signature: String,
    pub content_index_version: i64,
    pub generated_until_byte: i64,
    pub page_count: i64,
    pub status: String,
}

impl LayoutRecordInput {
    /// 便捷构造：`partial` 状态、页数 0。
    pub fn new(
        book_id: i64,
        layout_signature: impl Into<String>,
        content_index_version: i64,
    ) -> Self {
        Self {
            book_id,
            layout_signature: layout_signature.into(),
            content_index_version,
            generated_until_byte: 0,
            page_count: 0,
            status: LAYOUT_PARTIAL.to_string(),
        }
    }
}

/// `layout_records` 行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutRecord {
    pub id: i64,
    pub book_id: i64,
    pub layout_signature: String,
    pub content_index_version: i64,
    pub generated_until_byte: i64,
    pub page_count: i64,
    pub status: String,
    pub updated_at: i64,
}

/// 索引清理报告（每种被保护的用户数据给出保留行数）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexClearReport {
    pub chapters_deleted: i64,
    pub layout_records_deleted: i64,
    pub books_preserved: i64,
    pub book_sources_preserved: i64,
    pub progress_preserved: i64,
    pub bookmarks_preserved: i64,
    pub settings_preserved: i64,
    pub meta_preserved: i64,
}

fn chapter_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChapterRecord> {
    Ok(ChapterRecord {
        id: row.get(0)?,
        book_id: row.get(1)?,
        sequence: row.get(2)?,
        title: row.get(3)?,
        start_byte: row.get(4)?,
        end_byte: row.get(5)?,
        rule_id: row.get(6)?,
        index_version: row.get(7)?,
        created_at: row.get(8)?,
    })
}

fn layout_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LayoutRecord> {
    Ok(LayoutRecord {
        id: row.get(0)?,
        book_id: row.get(1)?,
        layout_signature: row.get(2)?,
        content_index_version: row.get(3)?,
        generated_until_byte: row.get(4)?,
        page_count: row.get(5)?,
        status: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn validate_chapters(chapters: &[ChapterInput]) -> Result<()> {
    let mut previous_end = -1_i64;
    for chapter in chapters {
        ensure_non_negative("chapters.sequence", chapter.sequence)?;
        ensure_short_text("chapters.title", &chapter.title, MAX_TITLE_BYTES)?;
        ensure_non_negative("chapters.start_byte", chapter.start_byte)?;
        ensure_non_negative("chapters.end_byte", chapter.end_byte)?;
        if chapter.end_byte < chapter.start_byte {
            return Err(StorageError::out_of_range(format!(
                "章节 {} 的 end_byte {} 小于 start_byte {}",
                chapter.sequence, chapter.end_byte, chapter.start_byte
            )));
        }
        if chapter.start_byte < previous_end {
            return Err(StorageError::out_of_range(format!(
                "章节 {} 的 start_byte {} 回退到前一章 end_byte {} 之前",
                chapter.sequence, chapter.start_byte, previous_end
            )));
        }
        previous_end = chapter.end_byte;
        ensure_short_text("chapters.rule_id", &chapter.rule_id, MAX_RULE_ID_BYTES)?;
    }
    Ok(())
}

/// 重建某书的章节索引（事务内先删该书全部章节，再按 `index_version` 插入）。
pub fn rebuild_chapters(
    conn: &Connection,
    book_id: i64,
    index_version: i64,
    chapters: &[ChapterInput],
    now: i64,
) -> Result<usize> {
    ensure_non_negative("book_id", book_id)?;
    ensure_non_negative("index_version", index_version)?;
    validate_chapters(chapters)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM chapters WHERE book_id = ?1", [book_id])?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO chapters (book_id, sequence, title, start_byte, end_byte, rule_id, \
             index_version, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        for chapter in chapters {
            stmt.execute(rusqlite::params![
                book_id,
                chapter.sequence,
                chapter.title,
                chapter.start_byte,
                chapter.end_byte,
                chapter.rule_id,
                index_version,
                now
            ])?;
        }
    }
    tx.commit()?;
    Ok(chapters.len())
}

/// 某书的章节索引（按 `sequence` 升序）。
pub fn list_chapters(conn: &Connection, book_id: i64) -> Result<Vec<ChapterRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, book_id, sequence, title, start_byte, end_byte, rule_id, index_version, \
         created_at FROM chapters WHERE book_id = ?1 ORDER BY sequence",
    )?;
    let rows = stmt.query_map([book_id], chapter_from_row)?;
    let mut chapters = Vec::new();
    for row in rows {
        chapters.push(row?);
    }
    Ok(chapters)
}

/// 某书章节数。
pub fn count_chapters(conn: &Connection, book_id: i64) -> Result<i64> {
    let mut stmt = conn.prepare("SELECT COUNT(*) FROM chapters WHERE book_id = ?1")?;
    let count: i64 = stmt.query_row([book_id], |row| row.get(0))?;
    Ok(count)
}

/// 删除某书全部章节，返回删除行数。
pub fn clear_chapters_for_book(conn: &Connection, book_id: i64) -> Result<i64> {
    let affected = conn.execute("DELETE FROM chapters WHERE book_id = ?1", [book_id])?;
    Ok(affected as i64)
}

/// 写入（upsert）分页边界记录：键 = `book_id + layout_signature + content_index_version`。
pub fn upsert_layout_record(conn: &Connection, input: &LayoutRecordInput, now: i64) -> Result<i64> {
    ensure_non_negative("layout_records.book_id", input.book_id)?;
    ensure_short_text(
        "layout_records.layout_signature",
        &input.layout_signature,
        MAX_SIGNATURE_BYTES,
    )?;
    ensure_non_negative(
        "layout_records.content_index_version",
        input.content_index_version,
    )?;
    ensure_non_negative(
        "layout_records.generated_until_byte",
        input.generated_until_byte,
    )?;
    ensure_non_negative("layout_records.page_count", input.page_count)?;
    ensure_one_of("layout_records.status", &input.status, LAYOUT_STATUSES)?;
    conn.execute(
        "INSERT INTO layout_records (book_id, layout_signature, content_index_version, \
         generated_until_byte, page_count, status, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
         ON CONFLICT (book_id, layout_signature, content_index_version) DO UPDATE SET \
         generated_until_byte = excluded.generated_until_byte, page_count = excluded.page_count, \
         status = excluded.status, updated_at = excluded.updated_at",
        rusqlite::params![
            input.book_id,
            input.layout_signature,
            input.content_index_version,
            input.generated_until_byte,
            input.page_count,
            input.status,
            now
        ],
    )?;
    let id: i64 = conn.query_row(
        "SELECT id FROM layout_records WHERE book_id = ?1 AND layout_signature = ?2 \
         AND content_index_version = ?3",
        rusqlite::params![
            input.book_id,
            input.layout_signature,
            input.content_index_version
        ],
        |row| row.get(0),
    )?;
    Ok(id)
}

/// 读取单条分页边界记录。
pub fn get_layout_record(
    conn: &Connection,
    book_id: i64,
    layout_signature: &str,
    content_index_version: i64,
) -> Result<Option<LayoutRecord>> {
    let record = conn
        .query_row(
            "SELECT id, book_id, layout_signature, content_index_version, generated_until_byte, \
             page_count, status, updated_at FROM layout_records \
             WHERE book_id = ?1 AND layout_signature = ?2 AND content_index_version = ?3",
            rusqlite::params![book_id, layout_signature, content_index_version],
            layout_from_row,
        )
        .optional()?;
    Ok(record)
}

/// 某书全部分页边界记录（按签名、索引版本排序）。
pub fn list_layout_records(conn: &Connection, book_id: i64) -> Result<Vec<LayoutRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, book_id, layout_signature, content_index_version, generated_until_byte, \
         page_count, status, updated_at FROM layout_records WHERE book_id = ?1 \
         ORDER BY layout_signature, content_index_version",
    )?;
    let rows = stmt.query_map([book_id], layout_from_row)?;
    let mut records = Vec::new();
    for row in rows {
        records.push(row?);
    }
    Ok(records)
}

/// 某书分页边界记录数。
pub fn count_layout_records(conn: &Connection, book_id: i64) -> Result<i64> {
    let mut stmt = conn.prepare("SELECT COUNT(*) FROM layout_records WHERE book_id = ?1")?;
    let count: i64 = stmt.query_row([book_id], |row| row.get(0))?;
    Ok(count)
}

/// 失效某个签名下的全部边界记录（签名变化 ⇒ 旧边界不可用）。
pub fn drop_layout_records_for_signature(
    conn: &Connection,
    book_id: i64,
    layout_signature: &str,
) -> Result<i64> {
    let affected = conn.execute(
        "DELETE FROM layout_records WHERE book_id = ?1 AND layout_signature = ?2",
        rusqlite::params![book_id, layout_signature],
    )?;
    Ok(affected as i64)
}

/// 丢弃内容索引版本不匹配的边界记录（内容索引重建后旧缓存全部失效）。
pub fn drop_stale_layout_records(
    conn: &Connection,
    book_id: i64,
    keep_content_index_version: i64,
) -> Result<i64> {
    let affected = conn.execute(
        "DELETE FROM layout_records WHERE book_id = ?1 AND content_index_version <> ?2",
        rusqlite::params![book_id, keep_content_index_version],
    )?;
    Ok(affected as i64)
}

/// 删除某书全部分页边界记录，返回删除行数。
pub fn clear_layout_records_for_book(conn: &Connection, book_id: i64) -> Result<i64> {
    let affected = conn.execute("DELETE FROM layout_records WHERE book_id = ?1", [book_id])?;
    Ok(affected as i64)
}

fn preservation_counts(conn: &Connection) -> Result<(i64, i64, i64, i64, i64, i64)> {
    Ok((
        connection::count_rows(conn, "books")?,
        connection::count_rows(conn, "book_sources")?,
        connection::count_rows(conn, "reading_progress")?,
        connection::count_rows(conn, "bookmarks")?,
        connection::count_rows(conn, "reader_settings")?,
        connection::count_rows(conn, "app_meta")?,
    ))
}

/// 清理**全部**可重建索引与分页缓存（设置页「清理索引/分页缓存」）。
///
/// 只允许删除 `chapters` 与 `layout_records` 两张可重建表的内容；
/// 书籍、来源、进度、书签、设置与元信息一律保留（返回报告中给出保留行数）。
pub fn clear_rebuildable_index(conn: &Connection) -> Result<IndexClearReport> {
    let tx = conn.unchecked_transaction()?;
    let chapters_deleted = tx.execute("DELETE FROM chapters", [])? as i64;
    let layout_records_deleted = tx.execute("DELETE FROM layout_records", [])? as i64;
    let (
        books_preserved,
        book_sources_preserved,
        progress_preserved,
        bookmarks_preserved,
        settings_preserved,
        meta_preserved,
    ) = preservation_counts(&tx)?;
    tx.commit()?;
    Ok(IndexClearReport {
        chapters_deleted,
        layout_records_deleted,
        books_preserved,
        book_sources_preserved,
        progress_preserved,
        bookmarks_preserved,
        settings_preserved,
        meta_preserved,
    })
}

/// 只清理某本书的可重建索引（重建索引、软移除书架时使用）。
pub fn clear_rebuildable_index_for_book(
    conn: &Connection,
    book_id: i64,
) -> Result<IndexClearReport> {
    ensure_non_negative("book_id", book_id)?;
    let tx = conn.unchecked_transaction()?;
    let chapters_deleted = clear_chapters_for_book(&tx, book_id)?;
    let layout_records_deleted = clear_layout_records_for_book(&tx, book_id)?;
    let (
        books_preserved,
        book_sources_preserved,
        progress_preserved,
        bookmarks_preserved,
        settings_preserved,
        meta_preserved,
    ) = preservation_counts(&tx)?;
    tx.commit()?;
    Ok(IndexClearReport {
        chapters_deleted,
        layout_records_deleted,
        books_preserved,
        book_sources_preserved,
        progress_preserved,
        bookmarks_preserved,
        settings_preserved,
        meta_preserved,
    })
}

/// 布局签名文本长度上限（供调用方在写库前自检）。
pub fn max_layout_signature_bytes() -> usize {
    MAX_SIGNATURE_BYTES
}

/// 章节名长度上限。
pub fn max_chapter_title_bytes() -> usize {
    MAX_TITLE_BYTES
}

/// 章节标题长度校验（供上层在扫描阶段提前拦截）。
pub fn validate_chapter_title(title: &str) -> Result<()> {
    ensure_max_len("chapters.title", title, MAX_TITLE_BYTES)
}

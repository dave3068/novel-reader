//! `reading_progress` 仓储：以 `ContentPosition` 为准的**幂等** upsert。
//!
//! 幂等语义（开发方案 §9.2「写入进度采用幂等 upsert，应用进入后台和阅读页退出时各尝试保存一次」）：
//! 位置五元组（content_unit_id / chapter_index / byte_offset / nearby_fingerprint /
//! index_version）与既有行完全一致时，**不写库、不更新时间戳**，返回 `changed = false`。

use rusqlite::{Connection, OptionalExtension};

use crate::error::{Result, StorageError};
use crate::validate::{ensure_max_len, ensure_non_negative, MAX_FINGERPRINT_BYTES};

/// 阅读进度输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressInput {
    pub book_id: i64,
    pub content_unit_id: i64,
    pub chapter_index: i64,
    pub byte_offset: i64,
    pub nearby_fingerprint: Option<String>,
    pub last_page_hint: Option<i64>,
    pub index_version: i64,
}

impl ProgressInput {
    /// 便捷构造：默认 content_unit_id=0、无指纹、无页码提示、index_version=0。
    pub fn at(book_id: i64, chapter_index: i64, byte_offset: i64) -> Self {
        Self {
            book_id,
            content_unit_id: 0,
            chapter_index,
            byte_offset,
            nearby_fingerprint: None,
            last_page_hint: None,
            index_version: 0,
        }
    }
}

/// `reading_progress` 行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressRecord {
    pub book_id: i64,
    pub content_unit_id: i64,
    pub chapter_index: i64,
    pub byte_offset: i64,
    pub nearby_fingerprint: Option<String>,
    pub last_page_hint: Option<i64>,
    pub index_version: i64,
    pub updated_at: i64,
    pub last_opened_at: i64,
}

/// 保存结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveProgressOutcome {
    /// 是否新建了进度行。
    pub inserted: bool,
    /// 位置是否发生变化（false ⇒ 幂等空写，未触碰时间戳）。
    pub changed: bool,
}

/// 位置五元组是否一致（幂等判定）。
fn same_position(existing: &ProgressRecord, input: &ProgressInput) -> bool {
    existing.content_unit_id == input.content_unit_id
        && existing.chapter_index == input.chapter_index
        && existing.byte_offset == input.byte_offset
        && existing.nearby_fingerprint == input.nearby_fingerprint
        && existing.index_version == input.index_version
}

fn validate(input: &ProgressInput) -> Result<()> {
    ensure_non_negative("reading_progress.book_id", input.book_id)?;
    ensure_non_negative("reading_progress.content_unit_id", input.content_unit_id)?;
    ensure_non_negative("reading_progress.chapter_index", input.chapter_index)?;
    ensure_non_negative("reading_progress.byte_offset", input.byte_offset)?;
    ensure_non_negative("reading_progress.index_version", input.index_version)?;
    if let Some(hint) = input.last_page_hint {
        ensure_non_negative("reading_progress.last_page_hint", hint)?;
    }
    if let Some(fingerprint) = &input.nearby_fingerprint {
        ensure_max_len(
            "reading_progress.nearby_fingerprint",
            fingerprint,
            MAX_FINGERPRINT_BYTES,
        )?;
    }
    Ok(())
}

fn progress_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProgressRecord> {
    Ok(ProgressRecord {
        book_id: row.get(0)?,
        content_unit_id: row.get(1)?,
        chapter_index: row.get(2)?,
        byte_offset: row.get(3)?,
        nearby_fingerprint: row.get(4)?,
        last_page_hint: row.get(5)?,
        index_version: row.get(6)?,
        updated_at: row.get(7)?,
        last_opened_at: row.get(8)?,
    })
}

/// 读取进度。
pub fn load_progress(conn: &Connection, book_id: i64) -> Result<Option<ProgressRecord>> {
    let record = conn
        .query_row(
            "SELECT book_id, content_unit_id, chapter_index, byte_offset, nearby_fingerprint, \
             last_page_hint, index_version, updated_at, last_opened_at \
             FROM reading_progress WHERE book_id = ?1",
            [book_id],
            progress_from_row,
        )
        .optional()?;
    Ok(record)
}

/// 幂等保存进度。
pub fn save_progress(
    conn: &Connection,
    input: &ProgressInput,
    now: i64,
) -> Result<SaveProgressOutcome> {
    validate(input)?;
    if let Some(existing) = load_progress(conn, input.book_id)? {
        if same_position(&existing, input) && existing.last_page_hint == input.last_page_hint {
            return Ok(SaveProgressOutcome {
                inserted: false,
                changed: false,
            });
        }
        conn.execute(
            "UPDATE reading_progress SET content_unit_id = ?2, chapter_index = ?3, \
             byte_offset = ?4, nearby_fingerprint = ?5, last_page_hint = ?6, index_version = ?7, \
             updated_at = ?8, last_opened_at = ?8 WHERE book_id = ?1",
            rusqlite::params![
                input.book_id,
                input.content_unit_id,
                input.chapter_index,
                input.byte_offset,
                input.nearby_fingerprint,
                input.last_page_hint,
                input.index_version,
                now
            ],
        )?;
        return Ok(SaveProgressOutcome {
            inserted: false,
            changed: true,
        });
    }

    conn.execute(
        "INSERT INTO reading_progress (book_id, content_unit_id, chapter_index, byte_offset, \
         nearby_fingerprint, last_page_hint, index_version, updated_at, last_opened_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        rusqlite::params![
            input.book_id,
            input.content_unit_id,
            input.chapter_index,
            input.byte_offset,
            input.nearby_fingerprint,
            input.last_page_hint,
            input.index_version,
            now
        ],
    )?;
    Ok(SaveProgressOutcome {
        inserted: true,
        changed: true,
    })
}

/// 只更新最近打开时间（不改变阅读位置）；返回是否命中既有行。
pub fn touch_last_opened(conn: &Connection, book_id: i64, now: i64) -> Result<bool> {
    let affected = conn.execute(
        "UPDATE reading_progress SET last_opened_at = ?2 WHERE book_id = ?1",
        rusqlite::params![book_id, now],
    )?;
    Ok(affected > 0)
}

/// 清空某书进度（显式用户操作；不在任何索引清理路径上调用）。
pub fn delete_progress(conn: &Connection, book_id: i64) -> Result<bool> {
    let affected = conn.execute("DELETE FROM reading_progress WHERE book_id = ?1", [book_id])?;
    Ok(affected > 0)
}

/// 进度行数。
pub fn count_progress(conn: &Connection) -> Result<i64> {
    crate::connection::count_rows(conn, "reading_progress")
}

/// 越界防护：byte_offset 不得超出给定文件长度。
pub fn ensure_offset_within(byte_offset: i64, source_size: i64) -> Result<()> {
    ensure_non_negative("source_size", source_size)?;
    if byte_offset > source_size {
        return Err(StorageError::out_of_range(format!(
            "byte_offset {byte_offset} 超出文件长度 {source_size}"
        )));
    }
    Ok(())
}

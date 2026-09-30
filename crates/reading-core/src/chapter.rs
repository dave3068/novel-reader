//! 章节索引与锚点恢复的**上层编排**（reading-core 侧）。
//!
//! 本模块不重复实现规则或匹配算法，而是把 [`reading_index`] 的能力接到
//! [`reading_source::TextSource`] 与 reading-core 的 [`ContentPosition`] 上：
//!
//! - [`scan_chapters_from_source`]：按有限窗口、可取消地扫描章节，无命中时回退到
//!   单一虚拟全文章节（`spec.md` CR3）；
//! - [`generate_anchor`]：在给定字节偏移处采样 `nearby_fingerprint`；
//! - [`restore_position`]：用**有界半径**锚点就近恢复位置，失败时回退到最近可确认的
//!   章节开头（`docs/novel-reader-v1-development-plan.md` §7.1 第 3–5 步）。
//!
//! 全程不整本读取、不把正文留在内存里（章节扫描只保留标题行与区间）。

use reading_index::{scan_chapters_with_encoding, anchor::{locate_anchor_bounded, make_anchor_from_source, AnchorSample}};
use reading_index::chapter::{scan_chapters, ChapterScanSession, ScanStats};
use reading_source::{SourceError, TextEncoding, TextSource};

use crate::error::{ErrorCode, NrError, NrResult};
use crate::position::{chapter_index_for_offset, ContentPosition};

pub use reading_index::anchor::{
    AnchorRecovery, AnchorSearch, AnchorStatus, ANCHOR_MAX_QUOTE_BYTES, ANCHOR_QUOTE_CHARS,
    DEFAULT_ANCHOR_RADIUS_BYTES, MAX_ANCHOR_RADIUS_BYTES,
};
pub use reading_index::chapter::{
    detect_chapter_title, ChapterIndex, ChapterMatch, ChapterRange, ChapterRule, ScanStep,
    CHAPTER_INDEX_VERSION, MIN_STANDALONE_NUMERIC_SECTIONS, VIRTUAL_CHAPTER_TITLE,
};

/// 把数据源错误映射为统一错误码（数值与 `NR_ERR_*` 一致，不重新定义数值）。
pub fn map_source_error(error: SourceError) -> NrError {
    let code = ErrorCode::from_code(error.code()).unwrap_or(ErrorCode::Internal);
    NrError::new(code, format!("数据源错误: {}", error.as_str()))
}

/// 按有限窗口、可取消地扫描章节索引。
///
/// `cancel` 每个窗口被查询一次；返回的 [`ChapterIndex::cancelled`] 为 `true` 时，
/// 结果是**部分结果**（调用方可稍后从 `scanned_bytes` 续扫）。
pub fn scan_chapters_from_source<S: TextSource + ?Sized>(
    source: &S,
    cancel: &dyn Fn() -> bool,
) -> NrResult<(ChapterIndex, ScanStats)> {
    scan_chapters_from_source_with_encoding(source, cancel, TextEncoding::Utf8)
}

/// 按内容编码扫描章节（CR2：非 UTF-8 文档必须也能识别章节目录）。
pub fn scan_chapters_from_source_with_encoding<S: TextSource + ?Sized>(
    source: &S,
    cancel: &dyn Fn() -> bool,
    encoding: TextEncoding,
) -> NrResult<(ChapterIndex, ScanStats)> {
    scan_chapters_with_encoding(source, cancel, encoding).map_err(map_source_error)
}

/// 无章节命中时的强制回退：单一「全文」虚拟章节。
pub fn virtual_chapter_index(total_bytes: u64) -> ChapterIndex {
    ChapterIndex::virtual_full_text(total_bytes)
}

/// 打开一个可增量推进的章节扫描会话（调用方逐 [`ScanStep`] 推进，可在步间让路）。
pub fn chapter_scan_session<S: TextSource + ?Sized>(source: &S) -> ChapterScanSession<'_, S> {
    ChapterScanSession::new(source)
}

/// 在指定字节偏移处采样锚点指纹（只读一个窗口）。
pub fn generate_anchor<S: TextSource + ?Sized>(
    source: &S,
    byte_offset: u64,
) -> NrResult<AnchorSample> {
    make_anchor_from_source(source, byte_offset).map_err(map_source_error)
}

/// 由字节偏移构造一个可持久化的 [`ContentPosition`]（章节序号由索引推断）。
pub fn position_at_offset<S: TextSource + ?Sized>(
    source: &S,
    book_id: u64,
    content_unit_id: u64,
    index: &ChapterIndex,
    byte_offset: u64,
) -> NrResult<ContentPosition> {
    let sample = generate_anchor(source, byte_offset)?;
    let chapter_index = chapter_index_for_offset(index, sample.byte_offset);
    Ok(ContentPosition::new(
        book_id,
        content_unit_id,
        chapter_index,
        sample.byte_offset,
        sample.fingerprint,
    ))
}

/// 位置恢复结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoredPosition {
    /// 恢复后的位置（字节偏移与章节序号已按实际命中/回退修正）。
    pub position: ContentPosition,
    /// 锚点恢复细节（含读取量有界证据）。
    pub recovery: AnchorRecovery,
    /// 是否回退到了章节开头（锚点为空或半径内未命中）。
    pub fallback_to_chapter_start: bool,
}

/// 用有界半径锚点就近恢复位置；失败时回退到最近可确认的章节开头。
///
/// 调用方必须提供当前 [`ChapterIndex`]（编码/内容索引变化后应使用新索引重建）。
pub fn restore_position<S: TextSource + ?Sized>(
    source: &S,
    index: &ChapterIndex,
    position: &ContentPosition,
    search: &AnchorSearch,
) -> NrResult<RestoredPosition> {
    let clamped = position.clamped_for(index);
    let chapter_start = index
        .chapter_start(clamped.chapter_index)
        .unwrap_or(clamped.original_byte_offset);

    if clamped.nearby_fingerprint.is_empty() {
        let recovery = AnchorRecovery {
            status: AnchorStatus::EmptyQuote,
            byte_offset: chapter_start,
            distance_bytes: 0,
            candidates: 0,
            windows_read: 0,
            bytes_read: 0,
            search_start: chapter_start,
            search_end: chapter_start,
            radius_bytes: search.effective_radius(),
        };
        return Ok(RestoredPosition {
            position: ContentPosition::new(
                clamped.book_id,
                clamped.content_unit_id,
                clamped.chapter_index,
                chapter_start,
                clamped.nearby_fingerprint,
            ),
            recovery,
            fallback_to_chapter_start: true,
        });
    }

    let recovery = locate_anchor_bounded(
        source,
        clamped.original_byte_offset,
        &clamped.nearby_fingerprint,
        search,
    )
    .map_err(map_source_error)?;

    if recovery.is_found() {
        let chapter_index = chapter_index_for_offset(index, recovery.byte_offset);
        Ok(RestoredPosition {
            position: ContentPosition::new(
                clamped.book_id,
                clamped.content_unit_id,
                chapter_index,
                recovery.byte_offset,
                clamped.nearby_fingerprint,
            ),
            recovery,
            fallback_to_chapter_start: false,
        })
    } else {
        // 恢复失败：回到最近可确认的章节开头（§7.1 第 5 步）。
        Ok(RestoredPosition {
            position: ContentPosition::new(
                clamped.book_id,
                clamped.content_unit_id,
                clamped.chapter_index,
                chapter_start,
                clamped.nearby_fingerprint,
            ),
            recovery,
            fallback_to_chapter_start: true,
        })
    }
}

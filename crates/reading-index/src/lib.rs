//! `reading-index`：章节区间、稀疏索引、分页边界与锚点。
//!
//! 本 crate 只依赖 [`reading_source`] 的字节坐标与有限窗口能力，不依赖 UI/Android/网络，
//! 也不依赖 `reading-core`（避免依赖环：`reading-core` 在它**上层**编排阅读会话）。
//!
//! # 本卡（RC4）已落地
//!
//! - [`chapter`]：章节规则（`第一章`/`第1章`/`第001章`/`第一卷`/`卷一`/`楔子`/`序章`/`前言`/
//!   `后记`/`尾声`/`Chapter N`/独立数字标题）、**增量可取消**扫描（[`ChapterScanSession`]）、
//!   区间索引 [`ChapterIndex`]，以及无章节命中时的**单一虚拟全文章节**回退；
//! - [`anchor`]：`nearby_fingerprint` 锚点生成与**有界半径就近恢复**（[`locate_anchor_bounded`]）。
//!
//! 尚未落地（后续卡片）：SQLite 章节索引落地（RC6，`reading-storage`）与 C ABI/JNI 传输（RC7）。

pub mod anchor;
pub mod boundary;
pub mod chapter;

pub use anchor::{
    make_anchor, make_anchor_from_source, AnchorRecovery, AnchorSample, AnchorSearch, AnchorStatus,
    NearbyFingerprint, ANCHOR_MAX_QUOTE_BYTES, ANCHOR_QUOTE_CHARS, DEFAULT_ANCHOR_RADIUS_BYTES,
    MAX_ANCHOR_RADIUS_BYTES,
};
pub use boundary::{
    check_strict_advance, is_char_boundary_at, is_char_boundary_in_window, validate_interval,
    validate_page_boundary, BoundaryError, BoundaryRejection, PageBoundary, PageBoundaryChain,
    PAGE_BOUNDARY_ENCODING_PREFIX,
};
pub use chapter::{
    detect_chapter_title, normalize_title, parse_chinese_numeral, scan_chapters, ChapterIndex,
    ChapterMatch, ChapterRange, ChapterRule, ChapterScanSession, ChapterScanner, ScanStats,
    ScanStep, CHAPTER_INDEX_VERSION, MAX_LINE_BYTES, MAX_STANDALONE_NUMERIC_DIGITS,
    MAX_TITLE_LINE_BYTES, MAX_TITLE_LINE_CHARS, MAX_TITLE_TAIL_CHARS,
    MIN_STANDALONE_NUMERIC_SECTIONS, VIRTUAL_CHAPTER_TITLE,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chapter_rules_expose_stable_ids_and_virtual_fallback_is_readable() {
        // 稳定数值用于跨 ABI / 落库；数值变化必须递增 CHAPTER_INDEX_VERSION。
        assert_eq!(ChapterRule::VirtualFullText.id(), 0);
        assert_eq!(ChapterRule::ChapterNumber.id(), 1);
        assert_eq!(ChapterRule::VolumeNumber.id(), 2);
        assert_eq!(ChapterRule::Keyword.id(), 3);
        assert_eq!(ChapterRule::English.id(), 4);
        assert_eq!(ChapterRule::StandaloneNumeric.id(), 5);
        for rule in [
            ChapterRule::VirtualFullText,
            ChapterRule::ChapterNumber,
            ChapterRule::VolumeNumber,
            ChapterRule::Keyword,
            ChapterRule::English,
            ChapterRule::StandaloneNumeric,
        ] {
            assert_eq!(ChapterRule::from_id(rule.id()), Some(rule));
            assert!(!rule.as_str().is_empty());
        }
        assert_eq!(ChapterRule::from_id(99), None);

        let index = ChapterIndex::virtual_full_text(4096);
        assert!(index.is_virtual, "无章节命中必须回退到虚拟全文章节");
        assert_eq!(index.len(), 1);
        assert_eq!(index.chapter_for_offset(0), Some(0));
        assert_eq!(index.chapter_for_offset(4095), Some(0));
        assert_eq!(index.index_version, CHAPTER_INDEX_VERSION);
        assert_eq!(
            index.chapter(0).map(|c| c.title.as_str()),
            Some(VIRTUAL_CHAPTER_TITLE)
        );
    }

    #[test]
    fn anchor_constants_match_frozen_window_contract() {
        // 半径硬上限必须 ≤ 窗口硬上限，避免锚点恢复放大单次读取量（gate-0 §2.3）。
        let max_radius: u64 = MAX_ANCHOR_RADIUS_BYTES;
        let default_radius: u64 = DEFAULT_ANCHOR_RADIUS_BYTES;
        assert_eq!(max_radius, 262_144);
        assert!(default_radius < max_radius);
        assert_eq!(ANCHOR_QUOTE_CHARS * 4, ANCHOR_MAX_QUOTE_BYTES);
        assert_eq!(AnchorSearch::new(u64::MAX).effective_radius(), max_radius);
    }
}
pub use crate::chapter::{scan_chapters_with_encoding, CHAPTER_SCANNER_VERSION};

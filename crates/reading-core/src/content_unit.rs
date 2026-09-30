//! 内容单元（content unit）：位置语义的坐标系所有者。
//!
//! 依据 `docs/novel-reader-v1-development-plan.md` §6 与 `design.md` §5：
//!
//! > 普通 TXT 是一个 `content_unit`；章节只是区间。布局缓存由
//! > `layout_signature + content_index_version` 标识。
//!
//! # V1 约定
//!
//! - 一个普通 TXT 文件**固定为一个内容单元**（[`SINGLE_CONTENT_UNIT_ID`]）；
//! - 内容单元只保存**元数据**（id、book_id、字节长度、来源标识、内容索引版本），
//!   **不保存正文**——正文永远经过 [`reading_source::TextSource`] 的有限窗口读取；
//! - 编码切换或来源指纹变化只递增 `content_index_version` 并使可重建索引失效，
//!   不删除书籍、进度和书签（`IndexInvalidation`，见 `reading_source::encoding`）。

use reading_source::TextSource;

use crate::position::{ContentPosition, NearbyFingerprint};

/// V1 中单个 TXT 内容单元的固定标识。
pub const SINGLE_CONTENT_UNIT_ID: u64 = 1;

/// 内容单元 schema 版本。
pub const CONTENT_UNIT_VERSION: u32 = 1;

/// 内容单元元数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentUnit {
    /// 内容单元标识（V1 固定为 [`SINGLE_CONTENT_UNIT_ID`]）。
    pub id: u64,
    /// 所属书籍标识。
    pub book_id: u64,
    /// 原始文件字节长度（坐标空间上界）。
    pub byte_len: u64,
    /// 来源标识（桌面为路径；Android 为持久化 URI）。
    pub source_identity: String,
    /// 内容索引版本（编码/指纹变化时递增，用于使章节与分页索引失效）。
    pub content_index_version: u32,
}

impl ContentUnit {
    /// 构造 V1 的单 TXT 内容单元。
    pub fn single_txt(book_id: u64, byte_len: u64, source_identity: impl Into<String>) -> Self {
        Self {
            id: SINGLE_CONTENT_UNIT_ID,
            book_id,
            byte_len,
            source_identity: source_identity.into(),
            content_index_version: 0,
        }
    }

    /// 由数据源元信息构造（只读取元信息，不读正文）。
    pub fn from_source<S: TextSource + ?Sized>(source: &S, book_id: u64) -> Self {
        let metadata = source.metadata();
        Self::single_txt(book_id, metadata.size, metadata.source_identity)
    }

    /// 是否是 V1 的单内容单元。
    pub fn is_single_unit(&self) -> bool {
        self.id == SINGLE_CONTENT_UNIT_ID
    }

    /// 偏移是否落在内容单元内。
    pub fn contains_offset(&self, offset: u64) -> bool {
        offset < self.byte_len
    }

    /// 把偏移夹紧到 `[0, byte_len)`；空文件返回 0。
    pub fn clamp_offset(&self, offset: u64) -> u64 {
        if self.byte_len == 0 {
            0
        } else {
            offset.min(self.byte_len - 1)
        }
    }

    /// 递增内容索引版本（编码切换 / 来源指纹变化时调用）。
    pub fn bump_content_index_version(&mut self) -> u32 {
        self.content_index_version = self.content_index_version.saturating_add(1);
        self.content_index_version
    }

    /// 在内容单元内构造位置（偏移先夹紧，保证位置可读）。
    pub fn position(
        &self,
        chapter_index: u32,
        original_byte_offset: u64,
        nearby_fingerprint: NearbyFingerprint,
    ) -> ContentPosition {
        ContentPosition::new(
            self.book_id,
            self.id,
            chapter_index,
            self.clamp_offset(original_byte_offset),
            nearby_fingerprint,
        )
    }

    /// 在内容单元内由附近文本构造位置。
    pub fn position_with_quote(
        &self,
        chapter_index: u32,
        original_byte_offset: u64,
        nearby_text: &str,
    ) -> ContentPosition {
        self.position(
            chapter_index,
            original_byte_offset,
            NearbyFingerprint::from_str_prefix(nearby_text),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_unit_is_single_for_v1_txt_and_never_holds_body() {
        let mut unit = ContentUnit::single_txt(42, 8192, "/home/user/book.txt");
        assert_eq!(unit.id, SINGLE_CONTENT_UNIT_ID);
        assert!(unit.is_single_unit());
        assert_eq!(unit.book_id, 42);
        assert_eq!(unit.byte_len, 8192);
        assert!(unit.contains_offset(0));
        assert!(unit.contains_offset(8191));
        assert!(!unit.contains_offset(8192));
        assert_eq!(unit.clamp_offset(u64::MAX), 8191);
        assert_eq!(unit.content_index_version, 0);
        assert_eq!(unit.bump_content_index_version(), 1);
        assert_eq!(unit.bump_content_index_version(), 2);

        let position = unit.position_with_quote(0, 9000, "第一章 初见");
        assert_eq!(position.content_unit_id, unit.id);
        assert_eq!(position.book_id, 42);
        assert_eq!(
            position.original_byte_offset, 8191,
            "偏移必须落在内容单元内"
        );
        assert!(position.nearby_fingerprint.is_consistent());
    }

    #[test]
    fn content_unit_of_empty_file_clamps_to_zero() {
        let unit = ContentUnit::single_txt(1, 0, "empty.txt");
        assert!(!unit.contains_offset(0));
        assert_eq!(unit.clamp_offset(100), 0);
        assert_eq!(unit.position_with_quote(0, 100, "").original_byte_offset, 0);
    }
}

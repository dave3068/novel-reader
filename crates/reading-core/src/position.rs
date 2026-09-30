//! `ContentPosition`：以内容单元 + 章节序号 + **原始字节偏移**表达阅读位置。
//!
//! 依据 `results/gates/gate-0-baseline.md` §2.4（字段冻结）与 `reading-core/design.md`
//! §位置规则：
//!
//! ```text
//! ContentPosition = book_id + content_unit_id + chapter_index
//!                 + original_byte_offset + nearby_fingerprint
//! ```
//!
//! # 冻结规则
//!
//! - **动态页码不是位置身份**：页码只属于某个 layout signature；本类型没有页码字段，
//!   也不允许把页码编码进位置；
//! - V1 一个普通 TXT 只有一个 content unit（见 [`crate::content_unit`]）；
//! - `original_byte_offset` 始终是**原始文件字节偏移**（不是解码后字符序号）；
//! - 恢复依赖 `nearby_fingerprint`（[`reading_index::NearbyFingerprint`]）在**有界半径**内
//!   就近命中（[`crate::chapter::restore_position`]）。
//!
//! # 编码往返
//!
//! [`ContentPosition::encode`] / [`ContentPosition::decode`] 提供一个无依赖、无浮点、
//! 无 JSON 的稳定文本形式，用于跨语言边界（必要时按字符串传输 64 位值）与本地持久化。
//! 往返必须**逐字段相等**（含 `u64::MAX` 这类极值）。

use reading_index::chapter::ChapterIndex;

pub use reading_index::anchor::NearbyFingerprint;

use crate::error::{ErrorCode, NrError, NrResult};

/// 位置编码前缀（版本化；不兼容变更必须换前缀）。
pub const POSITION_ENCODING_PREFIX: &str = "nrpos1";

/// 位置编码的字段数量（**不含**前缀）：book_id / content_unit_id / chapter_index /
/// original_byte_offset / nearby_fingerprint。刻意没有页码字段。
pub const POSITION_ENCODED_FIELDS: usize = 5;

/// 阅读位置（V1 契约冻结项）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentPosition {
    /// 书籍标识（64 位）。
    pub book_id: u64,
    /// 内容单元标识（V1 单个 TXT 固定为一个内容单元，64 位）。
    pub content_unit_id: u64,
    /// 章节序号（无章节时为虚拟全文章节，`index = 0`）。
    pub chapter_index: u32,
    /// 原始文件字节偏移。
    pub original_byte_offset: u64,
    /// 附近文本指纹，用于锚点就近恢复。
    pub nearby_fingerprint: NearbyFingerprint,
}

impl ContentPosition {
    /// 按冻结字段构造位置。
    pub fn new(
        book_id: u64,
        content_unit_id: u64,
        chapter_index: u32,
        original_byte_offset: u64,
        nearby_fingerprint: NearbyFingerprint,
    ) -> Self {
        Self {
            book_id,
            content_unit_id,
            chapter_index,
            original_byte_offset,
            nearby_fingerprint,
        }
    }

    /// 由一段附近文本构造位置（内部取前 [`reading_index::ANCHOR_QUOTE_CHARS`] 个字符）。
    pub fn at(
        book_id: u64,
        content_unit_id: u64,
        chapter_index: u32,
        original_byte_offset: u64,
        nearby_text: &str,
    ) -> Self {
        Self::new(
            book_id,
            content_unit_id,
            chapter_index,
            original_byte_offset,
            NearbyFingerprint::from_str_prefix(nearby_text),
        )
    }

    /// 章节序号是否落在给定索引范围内。
    pub fn chapter_index_in_range(&self, index: &ChapterIndex) -> bool {
        (self.chapter_index as usize) < index.len()
    }

    /// 所属章节（用于向 UI 展示“当前章节”）。
    pub fn chapter<'a>(&self, index: &'a ChapterIndex) -> Option<&'a reading_index::ChapterRange> {
        index.chapter(self.chapter_index)
    }

    /// 偏移是否落在文件范围内。
    pub fn is_inside(&self, total_bytes: u64) -> bool {
        self.original_byte_offset < total_bytes
    }

    /// 把位置夹紧到给定章节索引与文件长度内（章节越界夹到最近章节，偏移越界夹到章节起点）。
    ///
    /// 用于索引重建/编码切换后的恢复兜底：返回值一定满足 `chapter_index_in_range`。
    pub fn clamped_for(&self, index: &ChapterIndex) -> Self {
        let chapter_count = index.len();
        if chapter_count == 0 {
            return self.clone();
        }
        let chapter_index = (self.chapter_index as usize).min(chapter_count - 1) as u32;
        let Some(chapter) = index.chapter(chapter_index) else {
            return self.clone();
        };
        let max_offset = chapter.end_byte.saturating_sub(1).max(chapter.start_byte);
        let offset = self
            .original_byte_offset
            .clamp(chapter.start_byte, max_offset);
        Self {
            chapter_index,
            original_byte_offset: offset,
            ..self.clone()
        }
    }

    /// 编码为稳定文本形式（`nrpos1:<book>:<unit>:<chapter>:<offset>:<hex(quote)>`）。
    pub fn encode(&self) -> String {
        format!(
            "{}:{}:{}:{}:{}:{}",
            POSITION_ENCODING_PREFIX,
            self.book_id,
            self.content_unit_id,
            self.chapter_index,
            self.original_byte_offset,
            hex_encode(self.nearby_fingerprint.quote.as_bytes()),
        )
    }

    /// 解析 [`ContentPosition::encode`] 的输出；字段缺失、数值越界或非十六进制均报
    /// [`ErrorCode::InvalidArgument`]。
    pub fn decode(text: &str) -> NrResult<Self> {
        let mut parts = text.split(':');
        let prefix = parts.next().unwrap_or_default();
        if prefix != POSITION_ENCODING_PREFIX {
            return Err(invalid(format!(
                "位置编码前缀必须是 {POSITION_ENCODING_PREFIX}，实际 {prefix:?}"
            )));
        }
        let book_id = parse_u64(parts.next(), "book_id")?;
        let content_unit_id = parse_u64(parts.next(), "content_unit_id")?;
        let chapter_index = parse_u32(parts.next(), "chapter_index")?;
        let original_byte_offset = parse_u64(parts.next(), "original_byte_offset")?;
        let quote_part = parts
            .next()
            .ok_or_else(|| invalid("位置编码缺少 nearby_fingerprint 字段".to_string()))?;
        if parts.next().is_some() {
            return Err(invalid(
                "位置编码字段数量多于冻结的 5 项（位置身份不得包含动态页码等字段）".to_string(),
            ));
        }
        let quote_bytes = hex_decode(quote_part)
            .ok_or_else(|| invalid("nearby_fingerprint 不是合法十六进制".to_string()))?;
        let quote = String::from_utf8(quote_bytes)
            .map_err(|_| invalid("nearby_fingerprint 不是合法 UTF-8".to_string()))?;
        Ok(Self::new(
            book_id,
            content_unit_id,
            chapter_index,
            original_byte_offset,
            NearbyFingerprint::from_str_prefix(&quote),
        ))
    }
}

/// 按偏移求章节序号（越界偏移夹到最后一个章节；空索引返回 0）。
pub fn chapter_index_for_offset(index: &ChapterIndex, offset: u64) -> u32 {
    index.chapter_for_offset(offset).unwrap_or(0)
}

/// 章节起始字节（越界返回 `None`）。
pub fn chapter_start_offset(index: &ChapterIndex, chapter_index: u32) -> Option<u64> {
    index.chapter_start(chapter_index)
}

fn invalid(message: String) -> NrError {
    NrError::new(ErrorCode::InvalidArgument, message)
}

fn parse_u64(part: Option<&str>, field: &str) -> NrResult<u64> {
    part.ok_or_else(|| invalid(format!("位置编码缺少 {field} 字段")))?
        .parse::<u64>()
        .map_err(|_| invalid(format!("{field} 不是合法的 64 位无符号整数")))
}

fn parse_u32(part: Option<&str>, field: &str) -> NrResult<u32> {
    part.ok_or_else(|| invalid(format!("位置编码缺少 {field} 字段")))?
        .parse::<u32>()
        .map_err(|_| invalid(format!("{field} 不是合法的 32 位无符号整数")))
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        out.push(char::from_digit(u32::from(byte & 0x0F), 16).unwrap_or('0'));
    }
    out
}

fn hex_decode(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(text.len() / 2);
    let mut index = 0usize;
    while index + 1 < bytes.len() {
        let high = char::from(bytes[index]).to_digit(16)?;
        let low = char::from(bytes[index + 1]).to_digit(16)?;
        out.push((high * 16 + low) as u8);
        index += 2;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_fields_match_frozen_contract() {
        assert_eq!(POSITION_ENCODED_FIELDS, 5);
        assert_eq!(POSITION_ENCODING_PREFIX, "nrpos1");
        let position = ContentPosition::new(7, 1, 3, 4096, NearbyFingerprint::new("第一章 初见"));
        assert_eq!(position.book_id, 7);
        assert_eq!(position.content_unit_id, 1);
        assert_eq!(position.chapter_index, 3);
        assert_eq!(position.original_byte_offset, 4096);
        assert!(position.nearby_fingerprint.is_consistent());
    }

    #[test]
    fn position_encoding_rejects_extra_page_like_field() {
        // 位置身份不得包含动态页码：多出的字段必须被拒绝。
        let encoded = "nrpos1:1:1:0:0:414243:99";
        let error = ContentPosition::decode(encoded).expect_err("多余字段必须拒绝");
        assert_eq!(error.code(), ErrorCode::InvalidArgument);
        let bad_prefix =
            ContentPosition::decode("nrpos2:1:1:0:0:414243").expect_err("前缀必须校验");
        assert_eq!(bad_prefix.code(), ErrorCode::InvalidArgument);
        let bad_hex = ContentPosition::decode("nrpos1:1:1:0:0:xyz").expect_err("十六进制必须校验");
        assert_eq!(bad_hex.code(), ErrorCode::InvalidArgument);
    }

    #[test]
    fn position_hex_round_trip_is_exact() {
        assert_eq!(hex_encode(b""), "");
        assert_eq!(hex_encode(b"AB"), "4142");
        assert_eq!(hex_decode("4142"), Some(b"AB".to_vec()));
        assert_eq!(hex_decode("414"), None);
        assert_eq!(hex_decode("zz"), None);
    }
}

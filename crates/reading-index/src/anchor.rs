//! `nearby_fingerprint` 锚点：附近文本指纹的生成与**有界半径**就近恢复。
//!
//! 依据 `docs/novel-reader-v1-development-plan.md` §6（`ContentPosition` 的
//! `nearby_fingerprint`）、§7.1（编码切换后用原始字节位置附近的锚点尝试恢复）与
//! `reading-core/design.md` §位置规则。
//!
//! # 硬约束（卡片 RC4 禁止事项）
//!
//! - 锚点只保存**附近**一段 UTF-8 片段（默认 [`ANCHOR_QUOTE_CHARS`] 个字符），
//!   不保存正文、不保存整章/整本内容；
//! - 恢复只在 `[target - radius, target + radius + quote_len)` 的**有界区间**内进行，
//!   每次读取仍走 [`TextSource::read_at`] 的有限窗口（单次物化 ≤ 窗口硬上限），
//!   因此不会因锚点恢复而整本读取；
//! - 半径有硬上限 [`MAX_ANCHOR_RADIUS_BYTES`]，超过会被夹紧而不是放大读取。
//!
//! # 为什么“就近”而不是“第一个命中”
//!
//! 网络小说正文里重复段落（对话、信件、卷首语）非常常见，同一个片段可能在全文出现
//! 几十次。锚点恢复必须回到**距离原位置最近**的那一次出现，否则用户会跳到书的另一端。

use reading_source::{is_utf8_boundary, SourceError, TextSource};

/// 锚点引用文本的字符数上限。
pub const ANCHOR_QUOTE_CHARS: usize = 24;

/// 锚点引用文本的字节数上限（24 个字符 × 最多 4 字节）。
pub const ANCHOR_MAX_QUOTE_BYTES: usize = ANCHOR_QUOTE_CHARS * 4;

/// 默认就近搜索半径：目标位置前后各 32 KiB。
pub const DEFAULT_ANCHOR_RADIUS_BYTES: u64 = 32 * 1024;

/// 搜索半径硬上限：256 KiB（与窗口硬上限一致，避免锚点恢复放大读取量）。
pub const MAX_ANCHOR_RADIUS_BYTES: u64 = 256 * 1024;

/// 附近文本指纹：一段 UTF-8 引用文本 + 长度（`ContentPosition.nearby_fingerprint` 的载体）。
///
/// 同时记录**字符数**与**字节数**：字符数用于跨排版/跨端的内容比对，字节数用于在
/// 原始字节坐标空间里划定搜索长度（`gate-0-baseline.md` §2.4：「UTF-8 + 长度」）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NearbyFingerprint {
    /// 引用文本（UTF-8，最多 [`ANCHOR_QUOTE_CHARS`] 个字符）。
    pub quote: String,
    /// 引用文本的字符数。
    pub char_len: u32,
    /// 引用文本的字节数。
    pub byte_len: u32,
}

impl NearbyFingerprint {
    /// 由一段文本构造指纹（只取前 [`ANCHOR_QUOTE_CHARS`] 个字符）。
    pub fn new(quote: impl Into<String>) -> Self {
        let quote = quote.into();
        Self::from_str_prefix(&quote)
    }

    /// 由字符串前缀构造指纹。
    pub fn from_str_prefix(text: &str) -> Self {
        let quote: String = text.chars().take(ANCHOR_QUOTE_CHARS).collect();
        let char_len = quote.chars().count();
        let byte_len = quote.len();
        Self {
            quote,
            char_len: char_len as u32,
            byte_len: byte_len as u32,
        }
    }

    /// 空指纹（没有可用的引用文本）。
    pub fn empty() -> Self {
        Self::default()
    }

    /// 是否没有引用文本。
    pub fn is_empty(&self) -> bool {
        self.quote.is_empty()
    }

    /// 字节长度（`usize` 视图）。
    pub fn byte_len_usize(&self) -> usize {
        self.quote.len()
    }

    /// 指纹自洽性：声明的长度与实际内容一致，且不超过上限。
    pub fn is_consistent(&self) -> bool {
        self.char_len as usize == self.quote.chars().count()
            && self.byte_len as usize == self.quote.len()
            && self.quote.len() <= ANCHOR_MAX_QUOTE_BYTES
            && self.char_len as usize <= ANCHOR_QUOTE_CHARS
    }

    /// 引用文本是否出现在给定文本中（跨端一致性检查用）。
    pub fn matches(&self, text: &str) -> bool {
        !self.quote.is_empty() && text.contains(self.quote.as_str())
    }
}

impl std::fmt::Display for NearbyFingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "\"{}\"({}字/{}字节)",
            self.quote, self.char_len, self.byte_len
        )
    }
}

/// 一次锚点采样：指纹 + 实际锚定的字节偏移 + 本次采样读取的字节数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorSample {
    /// 生成的指纹。
    pub fingerprint: NearbyFingerprint,
    /// 锚点所在字节偏移（已对齐到合法 UTF-8 字符边界）。
    pub byte_offset: u64,
    /// 本次采样从数据源物化的原始字节数（≤ 窗口硬上限）。
    pub bytes_read: u64,
}

/// 由已解码文本构造锚点：把 `byte_offset` 向前对齐到字符边界后取前缀字符。
///
/// `byte_offset` 超出文本长度时退化为文本末尾。
pub fn make_anchor(text: &str, byte_offset: u64) -> NearbyFingerprint {
    let mut offset = (byte_offset.min(text.len() as u64)) as usize;
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    NearbyFingerprint::from_str_prefix(&text[offset..])
}

/// 从数据源在 `byte_offset` 处读取一个有限窗口并生成锚点。
///
/// 只读取一个窗口（默认 64 KiB，硬上限 256 KiB），不整本读取。
pub fn make_anchor_from_source<S: TextSource + ?Sized>(
    source: &S,
    byte_offset: u64,
) -> Result<AnchorSample, SourceError> {
    let len = source.len();
    if len == 0 {
        return Ok(AnchorSample {
            fingerprint: NearbyFingerprint::empty(),
            byte_offset: 0,
            bytes_read: 0,
        });
    }
    let offset = byte_offset.min(len - 1);
    let window = source.read_at(offset, source.default_window_bytes())?;
    let bytes_read = window.raw_bytes() as u64;
    let fingerprint = match std::str::from_utf8(&window.data) {
        Ok(text) => NearbyFingerprint::from_str_prefix(text),
        // 非 UTF-8 数据（如尚未解码的传统编码）：此时不生成锚点，交由上层回退到章节开头。
        Err(_) => NearbyFingerprint::empty(),
    };
    Ok(AnchorSample {
        fingerprint,
        byte_offset: window.start(),
        bytes_read,
    })
}

/// 就近搜索参数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnchorSearch {
    /// 目标位置前后各搜索多少字节。
    pub radius_bytes: u64,
}

impl AnchorSearch {
    /// 默认搜索参数（前后各 32 KiB）。
    pub const DEFAULT: Self = Self {
        radius_bytes: DEFAULT_ANCHOR_RADIUS_BYTES,
    };

    /// 指定半径（超过硬上限时夹紧，不放大读取）。
    pub fn new(radius_bytes: u64) -> Self {
        Self { radius_bytes }
    }

    /// 夹紧后的有效半径。
    pub fn effective_radius(&self) -> u64 {
        self.radius_bytes.min(MAX_ANCHOR_RADIUS_BYTES)
    }
}

impl Default for AnchorSearch {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 锚点恢复结果状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorStatus {
    /// 在半径内找到了引用文本（就近命中）。
    Found,
    /// 半径内没有找到引用文本（上层应回退到章节开头）。
    NotFound,
    /// 指纹为空（无可恢复信息）。
    EmptyQuote,
}

impl AnchorStatus {
    /// 稳定短名（日志/证据用）。
    pub const fn as_str(self) -> &'static str {
        match self {
            AnchorStatus::Found => "Found",
            AnchorStatus::NotFound => "NotFound",
            AnchorStatus::EmptyQuote => "EmptyQuote",
        }
    }
}

/// 锚点恢复结果（含“读取量有界”的可断言证据）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorRecovery {
    /// 恢复状态。
    pub status: AnchorStatus,
    /// 恢复出的字节偏移（`NotFound`/`EmptyQuote` 时为夹紧后的目标偏移）。
    pub byte_offset: u64,
    /// 命中位置与目标位置的距离（字节）。
    pub distance_bytes: u64,
    /// 半径内命中的候选数量（重复段落会 > 1）。
    pub candidates: u32,
    /// 本次恢复读取的窗口次数。
    pub windows_read: u64,
    /// 本次恢复从数据源物化的原始字节数。
    pub bytes_read: u64,
    /// 搜索区间起点（含）。
    pub search_start: u64,
    /// 搜索区间终点（不含）。
    pub search_end: u64,
    /// 本次使用的有效半径。
    pub radius_bytes: u64,
}

impl AnchorRecovery {
    /// 是否命中。
    pub fn is_found(&self) -> bool {
        self.status == AnchorStatus::Found
    }
}

/// 在**有界半径**内就近定位锚点，返回距离 `target_offset` 最近的命中位置。
///
/// - 只在 `[target - radius, target + radius + quote_len)` 内搜索；
/// - 即使区间大于单窗口，也只会按窗口顺序读取（每次 ≤ 窗口硬上限），不整本读取；
/// - 多个命中时取距离最近者；距离相同则取偏移较小者（确定性）；
/// - 半径内没有命中时返回 [`AnchorStatus::NotFound`]，由上层回退到章节开头。
pub fn locate_anchor_bounded<S: TextSource + ?Sized>(
    source: &S,
    target_offset: u64,
    fingerprint: &NearbyFingerprint,
    search: &AnchorSearch,
) -> Result<AnchorRecovery, SourceError> {
    let len = source.len();
    let target = target_offset.min(len);
    let radius = search.effective_radius();
    let quote = fingerprint.quote.as_bytes();

    if quote.is_empty() {
        return Ok(AnchorRecovery {
            status: AnchorStatus::EmptyQuote,
            byte_offset: target,
            distance_bytes: 0,
            candidates: 0,
            windows_read: 0,
            bytes_read: 0,
            search_start: target,
            search_end: target,
            radius_bytes: radius,
        });
    }

    let search_start = target.saturating_sub(radius);
    let search_end = len.min(
        target
            .saturating_add(radius)
            .saturating_add(quote.len() as u64),
    );

    let quote_len = quote.len();
    let overlap_keep = quote_len.saturating_sub(1);
    let max_window = source.max_window_bytes().max(1);
    let default_window = source.default_window_bytes().clamp(1, max_window);

    let mut best: Option<(u64, u64)> = None; // (distance, offset)
    let mut candidates: u32 = 0;
    let mut windows_read: u64 = 0;
    let mut bytes_read: u64 = 0;
    let mut overlap: Vec<u8> = Vec::new();
    let mut overlap_start = search_start;
    let mut previous_buf_end: u64 = 0;
    let mut offset = search_start;

    while offset < search_end && search_end > search_start {
        let remaining = search_end - offset;
        let want = (remaining + overlap_keep as u64).max(1);
        let want = want.min(default_window as u64) as usize;
        let window = source.read_at(offset, want)?;
        windows_read += 1;
        bytes_read += window.raw_bytes() as u64;

        let mut buf: Vec<u8> = Vec::with_capacity(overlap.len() + window.data.len());
        buf.extend_from_slice(&overlap);
        buf.extend_from_slice(&window.data);
        let buf_start = if overlap.is_empty() {
            window.start()
        } else {
            overlap_start
        };
        let buf_end = buf_start + buf.len() as u64;

        if buf.len() >= quote_len {
            for (index, slice) in buf.windows(quote_len).enumerate() {
                if slice != quote {
                    continue;
                }
                let at = buf_start + index as u64;
                // 去重：完全落在上一轮缓冲区内的命中已统计过。
                if previous_buf_end > 0 && at + quote_len as u64 <= previous_buf_end {
                    continue;
                }
                if at < search_start || !is_utf8_boundary(&buf, index) {
                    continue;
                }
                candidates += 1;
                let distance = at.abs_diff(target);
                if best.is_none_or(|(best_distance, _)| distance < best_distance) {
                    best = Some((distance, at));
                }
            }
        }

        previous_buf_end = buf_end;
        let keep = overlap_keep.min(window.data.len());
        overlap = window.data[window.data.len() - keep..].to_vec();
        overlap_start = window.end() - keep as u64;

        let next = window.end();
        if window.eof || window.data.is_empty() || next <= offset {
            break;
        }
        offset = next;
    }

    let (status, byte_offset, distance_bytes) = match best {
        Some((distance, at)) => (AnchorStatus::Found, at, distance),
        None => (AnchorStatus::NotFound, target, 0),
    };

    Ok(AnchorRecovery {
        status,
        byte_offset,
        distance_bytes,
        candidates,
        windows_read,
        bytes_read,
        search_start,
        search_end,
        radius_bytes: radius,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_fingerprint_records_utf8_and_lengths() {
        let fp = NearbyFingerprint::new("第一章 中文测试🙂end");
        assert_eq!(fp.quote, "第一章 中文测试🙂end");
        assert_eq!(fp.char_len as usize, fp.quote.chars().count());
        assert_eq!(fp.byte_len as usize, fp.quote.len());
        assert!(fp.byte_len > fp.char_len, "中文引用的字节数必须大于字符数");
        assert!(fp.is_consistent());
        assert!(fp.matches("前言第一章 中文测试🙂end后记"));
        assert!(!fp.matches("完全不相关的文本"));

        let long = NearbyFingerprint::new("甲".repeat(100));
        assert_eq!(long.char_len as usize, ANCHOR_QUOTE_CHARS);
        assert_eq!(long.byte_len_usize(), ANCHOR_QUOTE_CHARS * 3);
        assert!(long.is_consistent());
        assert!(NearbyFingerprint::empty().is_empty());
        assert!(NearbyFingerprint::empty().is_consistent());
    }

    #[test]
    fn anchor_is_aligned_to_character_boundaries() {
        let text = "第一章 中文测试";
        let mid_char = 1u64; // 「第」的第 2 个字节
        let fp = make_anchor(text, mid_char);
        assert_eq!(fp.quote, "第一章 中文测试", "必须向前对齐到字符起点");
        let tail = make_anchor(text, text.len() as u64 + 100);
        assert!(tail.is_empty(), "超出末尾的锚点为空");
        let exact = make_anchor(text, 0);
        assert_eq!(exact.quote, text);
    }
}

//! 内容位置与分页锚点：跨 UI 页边界恢复同一原文附近位置（S-R3 / H-R4）。

use crate::pagination::ceil_char_boundary;

/// 锚点引用文本的字符数。
pub const ANCHOR_QUOTE_CHARS: usize = 24;

/// 阅读位置（共享语义，两端必须一致）。
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ContentPosition {
    pub book_id: u64,
    pub chapter_index: u32,
    pub byte_offset: u64,
}

/// 页面边界登记项。
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PageBoundary {
    pub chapter_index: u32,
    pub start_byte: u64,
    pub end_byte: u64,
    /// 产生该边界的排版标识（字体/尺寸/间距/边距的组合）。
    pub layout_id: u64,
    pub line_count: u32,
}

/// 分页锚点：即使两端页边界不同，也能恢复到同一原文附近位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageAnchor {
    pub chapter_index: u32,
    pub start_byte: u64,
    pub end_byte: u64,
    /// 页首引用文本（最多 `ANCHOR_QUOTE_CHARS` 个字符）。
    pub quote: String,
    /// 页首在原文中的字符序号（辅助定位）。
    pub char_index: u64,
}

/// 由页范围构造锚点。
pub fn make_anchor(text: &str, chapter_index: u32, start_byte: u64, end_byte: u64) -> PageAnchor {
    let start = crate::pagination::floor_char_boundary(text, (start_byte as usize).min(text.len()));
    let quote: String = text[start..].chars().take(ANCHOR_QUOTE_CHARS).collect();
    let char_index = text[..start].chars().count() as u64;
    PageAnchor {
        chapter_index,
        start_byte: start as u64,
        end_byte,
        quote,
        char_index,
    }
}

/// 在原文中定位锚点，返回最接近原始页首的字节偏移。
///
/// 若引用文本找不到（例如排版导致引用被裁断），退化为使用 `char_index`
/// 换算的字节偏移；再不可用则返回 `anchor.start_byte`。
pub fn locate_anchor(text: &str, anchor: &PageAnchor) -> u64 {
    if anchor.quote.is_empty() {
        return anchor.start_byte.min(text.len() as u64);
    }
    let mut best: Option<(u64, u64)> = None; // (distance, offset)
    let target = anchor.start_byte.min(text.len() as u64);
    let mut from = 0usize;
    while let Some(rel) = text[from..].find(&anchor.quote) {
        let at = (from + rel) as u64;
        let dist = at.abs_diff(target);
        if best.map_or(true, |(bd, _)| dist < bd) {
            best = Some((dist, at));
        }
        if at as usize >= text.len() {
            break;
        }
        from = at as usize + anchor.quote.chars().next().map_or(1, |c| c.len_utf8());
        if from >= text.len() {
            break;
        }
    }
    if let Some((_, at)) = best {
        return at;
    }
    // 退化路径：按字符序号换算。
    let mut count = 0u64;
    for (off, _) in text.char_indices() {
        if count == anchor.char_index {
            return off as u64;
        }
        count += 1;
    }
    text.len() as u64
}

/// 计算 `pos` 之前最近的合法 UTF-8 边界（用于位置校正）。
pub fn normalize_position(text: &str, pos: u64) -> u64 {
    if pos as usize >= text.len() {
        return text.len() as u64;
    }
    if text.is_char_boundary(pos as usize) {
        return pos;
    }
    ceil_char_boundary(text, pos as usize) as u64
}

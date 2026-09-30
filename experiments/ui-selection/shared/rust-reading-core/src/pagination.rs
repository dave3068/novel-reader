//! 中英文分页尖峰：由稳定公共 API 返回合法 UTF-8 页末位置。
//!
//! 该实现属于共享业务（S-R1/S-R3），桌面端通过直接 Rust API 调用，
//! 跨语言候选通过 `nr_layout_page` 调用；两端语义等价（PR1）。
//! 排版度量是确定性模型（不做字形光栅），因此可跨端复现与断言。

use std::time::Instant;

/// 排版配置。两端可使用不同取值，因此页边界允许不同（H-R4）。
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutConfig {
    /// 可用宽度（逻辑像素）。
    pub width: f32,
    /// 可用高度（逻辑像素）。
    pub height: f32,
    /// 字体族（仅记录，用于布局标识）。
    pub font_family: String,
    /// 字号（逻辑像素）。
    pub font_size: f32,
    /// 行距倍数。
    pub line_spacing: f32,
    /// 段间距（逻辑像素）。
    pub paragraph_spacing: f32,
    /// 首行缩进（逻辑像素）。
    pub first_line_indent: f32,
    /// 左右页边距。
    pub margin_x: f32,
    /// 上下页边距。
    pub margin_y: f32,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            width: 800.0,
            height: 600.0,
            font_family: "Noto Sans CJK SC".to_string(),
            font_size: 18.0,
            line_spacing: 1.4,
            paragraph_spacing: 8.0,
            first_line_indent: 36.0,
            margin_x: 24.0,
            margin_y: 24.0,
        }
    }
}

impl LayoutConfig {
    /// 桌面 Slint 端默认排版。
    pub fn desktop_default() -> Self {
        Self {
            width: 800.0,
            height: 560.0,
            font_family: "Noto Sans CJK SC".into(),
            ..Self::default()
        }
    }

    /// Android 端默认排版（尺寸/密度差异导致页边界不同，属预期）。
    pub fn android_default() -> Self {
        Self {
            width: 411.0,
            height: 731.0,
            font_family: "Noto Sans CJK SC".into(),
            font_size: 20.0,
            line_spacing: 1.5,
            paragraph_spacing: 12.0,
            first_line_indent: 40.0,
            margin_x: 16.0,
            margin_y: 20.0,
        }
    }
}

/// 单页排版结果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageLayout {
    /// 本页消费的 UTF-8 字节数（从 `start_byte` 起算）。
    pub consumed_utf8_bytes: usize,
    /// 可见行数。
    pub visible_line_count: u32,
    /// 末行实际宽度。
    pub last_line_width: f32,
    /// 行高。
    pub last_line_height: f32,
    /// 布局耗时（微秒）。
    pub layout_time_us: u64,
}

impl PageLayout {
    /// EOF（无可消费内容）。
    pub fn eof() -> Self {
        Self {
            consumed_utf8_bytes: 0,
            visible_line_count: 0,
            last_line_width: 0.0,
            last_line_height: 0.0,
            layout_time_us: 0,
        }
    }
}

/// 行首禁则字符（不可出现在行首）。
const NO_BREAK_BEFORE: [char; 22] = [
    '，', '。', '！', '？', '；', '：', '、', '）', '】', '」', '』', '》', '〉', '·', '%', '℃',
    '”', '’', '…', '〕', '］', '｝',
];
/// 行尾禁则字符（不可出现在行尾）。
const NO_BREAK_AFTER: [char; 11] = [
    '（', '【', '「', '『', '《', '〈', '“', '‘', '〔', '［', '｛',
];

/// 单字符前进宽度（确定性度量模型）。
///
/// - 组合符/零宽字符：0
/// - 全角（CJK/Kana/全角标点/谚文）：字号
/// - Emoji：字号
/// - 其余半角：0.55 × 字号
pub fn char_advance(c: char, font_size: f32) -> f32 {
    match c as u32 {
        0x0300..=0x036F | 0x200B..=0x200F | 0xFE00..=0xFE0F | 0xFE20..=0xFE2F => 0.0,
        0x1100..=0x115F
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6 => font_size,
        0x1F000..=0x1FAFF | 0x2600..=0x27BF => font_size,
        _ => font_size * 0.55,
    }
}

/// 向下取整到合法 UTF-8 边界。
pub fn floor_char_boundary(text: &str, idx: usize) -> usize {
    if idx >= text.len() {
        return text.len();
    }
    if text.is_char_boundary(idx) {
        return idx;
    }
    let mut i = idx;
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// 向上取整到合法 UTF-8 边界。
pub fn ceil_char_boundary(text: &str, idx: usize) -> usize {
    if idx >= text.len() {
        return text.len();
    }
    if text.is_char_boundary(idx) {
        return idx;
    }
    let mut i = idx;
    while i < text.len() && !text.is_char_boundary(i) {
        i += 1;
    }
    i
}

fn is_no_break_before(c: char) -> bool {
    NO_BREAK_BEFORE.contains(&c)
}

fn is_no_break_after(c: char) -> bool {
    NO_BREAK_AFTER.contains(&c)
}

/// 计算从 `start` 开始的一行，返回（行尾字节偏移，行宽）。
///
/// 保证返回值严格大于 `start`（除 `start >= text.len()` 外），即不会死循环。
fn break_line(
    text: &str,
    start: usize,
    width: f32,
    cfg: &LayoutConfig,
    indent: f32,
) -> (usize, f32) {
    let width = width.max(cfg.font_size);
    let indent = indent.clamp(0.0, width * 0.5);
    let mut x = indent;
    let mut candidate: Option<(usize, f32)> = None;

    for (off, c) in text[start..].char_indices() {
        let byte_idx = start + off;
        if c == '\n' {
            return (byte_idx + c.len_utf8(), x);
        }
        let adv = char_advance(c, cfg.font_size);
        if x + adv > width {
            if byte_idx > start {
                if let Some((bp, bw)) = candidate {
                    if bp > start && bp <= byte_idx {
                        return (bp, bw);
                    }
                }
                return (byte_idx, x);
            }
            // 行首单字符超宽：至少消费一个字符，保证前进。
            return (byte_idx + c.len_utf8(), adv);
        }
        x += adv;
        let next_c = text[byte_idx + c.len_utf8()..].chars().next();
        let can_break_after =
            !is_no_break_after(c) && next_c.map_or(true, |n| !is_no_break_before(n));
        if can_break_after {
            candidate = Some((byte_idx + c.len_utf8(), x));
        }
    }
    (text.len(), x)
}

/// 对 `text[start_byte..]` 排版一页。
///
/// 返回的 `consumed_utf8_bytes` 一定落在 UTF-8 合法边界；若 `start_byte`
/// 不在边界上会向下取整；EOF 时返回 0。
pub fn layout_page(text: &str, start_byte: usize, cfg: &LayoutConfig) -> PageLayout {
    let t0 = Instant::now();
    let start = floor_char_boundary(text, start_byte.min(text.len()));
    if start >= text.len() {
        return PageLayout::eof();
    }

    let line_height = (cfg.font_size * cfg.line_spacing).max(1.0);
    let avail_w = (cfg.width - 2.0 * cfg.margin_x).max(cfg.font_size);
    let avail_h = (cfg.height - 2.0 * cfg.margin_y).max(line_height);
    let max_lines = ((avail_h / line_height).floor() as usize).max(1);

    let mut pos = start;
    let mut lines: u32 = 0;
    let mut y = 0.0f32;
    let mut last_width = 0.0f32;

    while (lines as usize) < max_lines && pos < text.len() {
        let prev = text[..pos].chars().next_back();
        let is_para_start = pos == start || prev == Some('\n');
        let indent = if is_para_start {
            cfg.first_line_indent
        } else {
            0.0
        };
        let (end, w) = break_line(text, pos, avail_w, cfg, indent);
        let end = if end <= pos {
            ceil_char_boundary(text, pos + 1)
        } else {
            end
        };
        let ends_with_newline = text[pos..end].ends_with('\n');
        let block_h = line_height
            + if ends_with_newline {
                cfg.paragraph_spacing
            } else {
                0.0
            };
        if lines > 0 && y + block_h > avail_h {
            break;
        }
        y += block_h;
        lines += 1;
        last_width = w;
        pos = end;
    }

    let consumed = pos - start;
    PageLayout {
        consumed_utf8_bytes: consumed,
        visible_line_count: lines,
        last_line_width: last_width,
        last_line_height: line_height,
        layout_time_us: t0.elapsed().as_micros() as u64,
    }
}

/// 排版标识：同一配置产生同一 id，用于页面边界登记与失效。
pub fn layout_id(cfg: &LayoutConfig) -> u64 {
    // FNV-1a 64 位，字段按固定顺序写入，保证跨端一致。
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let feed = |bytes: &[u8], hash: &mut u64| {
        for b in bytes {
            *hash ^= *b as u64;
            *hash = hash.wrapping_mul(0x100_0000_01b3);
        }
    };
    feed(cfg.font_family.as_bytes(), &mut hash);
    feed(&cfg.width.to_bits().to_le_bytes(), &mut hash);
    feed(&cfg.height.to_bits().to_le_bytes(), &mut hash);
    feed(&cfg.font_size.to_bits().to_le_bytes(), &mut hash);
    feed(&cfg.line_spacing.to_bits().to_le_bytes(), &mut hash);
    feed(&cfg.paragraph_spacing.to_bits().to_le_bytes(), &mut hash);
    feed(&cfg.first_line_indent.to_bits().to_le_bytes(), &mut hash);
    feed(&cfg.margin_x.to_bits().to_le_bytes(), &mut hash);
    feed(&cfg.margin_y.to_bits().to_le_bytes(), &mut hash);
    hash
}

/// 连续翻页：从 `start_byte` 起连续产生 `page_count` 页的（起点, 消费字节）序列。
///
/// 用于「正向 / 反向 / 重排」检查的公共入口。
pub fn paginate(
    text: &str,
    start_byte: usize,
    cfg: &LayoutConfig,
    page_count: usize,
) -> Vec<(usize, usize)> {
    let mut out = Vec::with_capacity(page_count.min(1_000_000));
    let mut pos = floor_char_boundary(text, start_byte);
    for _ in 0..page_count {
        if pos >= text.len() {
            break;
        }
        let page = layout_page(text, pos, cfg);
        if page.consumed_utf8_bytes == 0 {
            break;
        }
        out.push((pos, page.consumed_utf8_bytes));
        pos += page.consumed_utf8_bytes;
    }
    out
}

//! 桌面分页测量与页边界提交（DT3 / DR3）。
//!
//! # 职责划分
//!
//! - **本模块**：按 [`crate::reflow::Typography`]（窗口映射后的正文区域尺寸 + 真实渲染参数）
//!   把有限窗口文本断成整页的行；每次测量都带 `layout_id` / `request_id`，并提交给冻结核心
//!   [`reading_core::PaginationCache`] 做「区间合法 / UTF-8 边界 / 世代 / 请求号」校验。
//! - **冻结核心**：页面边界语义、字符边界校验、布局签名失效与缓存键——不在此重复实现。
//! - **UI（Slint）**：只渲染本模块给出的逐行结果，不做任何分页推断。
//!
//! # 同口径契约（`docs/novel-reader-v1-development-plan.md` §8.3）
//!
//! 测量与渲染使用同一份参数：字号、行盒高度、字距（0）、段距（0）、首行缩进（0）、
//! 正文区域宽高。渲染端按 [`PagedPage::lines`] 逐行定位（`y = i × 行盒`），因此
//! **页面高度由测量侧唯一决定**：一页的行数乘行盒必须 ≤ 正文区高度，且再多一行就会溢出——
//! 「无滚动条 / 无半字裁切 / 底部不留大片空白」三件事都由这条不变量保证。
//!
//! # 前进宽度模型
//!
//! Slint 1.17 不提供公开的文本测量 API（`TextMetrics` 之类属私有实现），因此断行使用
//! **确定性的整数前进宽度模型**（[`char_advance_milli`]）：CJK/全角 = 1.0 em、ASCII ≈ 0.62 em、
//! 窄字符 0.34 em、宽字符 0.92 em。取值刻意**不小于**真实字形前进宽度（保守），
//! 于是「测量出的行宽 ≥ 渲染出的行宽」恒成立：行不会因为渲染侧更宽而被挤成两行，
//! 也不会出现半字裁切或横向溢出。代价只是每行偶尔少放一两个字（§8.3 第 7 条允许
//! 「宁可少排一行」）。

use reading_core::{
    BoundaryCommitStatus, BoundaryRejection, ContentUnit, LayoutSignature, NrError, PageBoundary,
    PageBoundaryRequest, PaginationCache,
};
use reading_source::{SourceError, TextEncoding, TextSource, MAX_WINDOW_BYTES};

use crate::encoded_source::EncodedFileSource;

use std::path::{Path, PathBuf};

use crate::layout::LayoutMetrics;
use crate::page_cache::{CacheStats, PageCache, DEFAULT_CACHE_CAPACITY, MAX_PREFETCH_BATCH};
use crate::reflow::{
    MappedViewport, PlatformFontMetrics, Typography, TypographyError, TypographySettings,
};

/// 单页测量的起始窗口（字节）：页尾不足时按倍数放大。
pub const INITIAL_PAGE_WINDOW_BYTES: usize = 8 * 1024;
/// 单页测量的窗口上限（等于核心的窗口硬上限，绝不整本读取）。
pub const MAX_PAGE_WINDOW_BYTES: usize = MAX_WINDOW_BYTES;
/// 反向翻页在缺少已登记边界时，最多重扫的页数（防止退化成无界扫描）。
pub const MAX_BACKWARD_RESCAN_PAGES: usize = 4_096;

/// 行首禁则字符（不得出现在行首）。
const NO_BREAK_BEFORE: [char; 22] = [
    '，', '。', '！', '？', '；', '：', '、', '）', '】', '」', '』', '》', '〉', '·', '%', '℃',
    '”', '’', '…', '〕', '］', '｝',
];
/// 行尾禁则字符（不得出现在行尾）。
const NO_BREAK_AFTER: [char; 11] = [
    '（', '【', '「', '『', '《', '〈', '“', '‘', '〔', '［', '｛',
];

/// 单字符前进宽度（毫像素，按字号千分比）。
///
/// 返回值为**保守上界**（≥ 真实字形前进宽度）；组合符与零宽字符为 0。
pub fn char_advance_milli(c: char, font_size_milli: u32) -> u32 {
    let ratio: u32 = match c {
        // 组合符 / 零宽 / 变体选择符
        '\u{0300}'..='\u{036F}'
        | '\u{200B}'..='\u{200F}'
        | '\u{FE00}'..='\u{FE0F}'
        | '\u{FE20}'..='\u{FE2F}' => 0,
        // 全角：CJK / 假名 / 全角标点 / 谚文
        '\u{1100}'..='\u{115F}'
        | '\u{2E80}'..='\u{303E}'
        | '\u{3041}'..='\u{33FF}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{A000}'..='\u{A4CF}'
        | '\u{AC00}'..='\u{D7A3}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{FE30}'..='\u{FE4F}'
        | '\u{FF00}'..='\u{FF60}'
        | '\u{FFE0}'..='\u{FFE6}' => 1_000,
        // Emoji 与常见符号
        '\u{1F000}'..='\u{1FAFF}' | '\u{2600}'..='\u{27BF}' => 1_000,
        ' ' => 300,
        '\t' => 1_200,
        'i' | 'l' | 'j' | 'I' | '.' | ',' | ':' | ';' | '!' | '\'' | '"' | '|' | '(' | ')'
        | '[' | ']' | '{' | '}' | '-' | '/' | '\\' => 340,
        'm' | 'w' | 'M' | 'W' | '@' | '%' | '&' | 'Q' => 920,
        _ => 620,
    };
    (font_size_milli as u64 * ratio as u64 / 1_000) as u32
}

fn is_no_break_before(c: char) -> bool {
    NO_BREAK_BEFORE.contains(&c)
}

fn is_no_break_after(c: char) -> bool {
    NO_BREAK_AFTER.contains(&c)
}

/// 断出一行：返回（行尾字节偏移，行宽毫像素）。
///
/// 保证返回值严格大于 `start`（除非 `start >= text.len()`），因此断行循环不会死循环；
/// 行首单字符超宽时至少消费一个字符（宁可单字成行，也不产生空行死循环）。
pub fn break_line(
    text: &str,
    start: usize,
    typography: &Typography,
    indent_milli: u32,
) -> (usize, u32) {
    if start >= text.len() {
        return (text.len(), 0);
    }
    let width = typography.line_width_milli.max(typography.font_size_milli);
    let indent = indent_milli.min(width / 2);
    let mut x = indent;
    let mut candidate: Option<(usize, u32)> = None;

    for (offset, c) in text[start..].char_indices() {
        let byte_index = start + offset;
        if c == '\n' {
            return (byte_index + c.len_utf8(), x);
        }
        let advance = char_advance_milli(c, typography.font_size_milli);
        if x + advance > width {
            if byte_index > start {
                if let Some((break_at, break_width)) = candidate {
                    if break_at > start && break_at <= byte_index {
                        return (break_at, break_width);
                    }
                }
                return (byte_index, x);
            }
            // 行首单字符超宽：至少消费一个字符。
            return (byte_index + c.len_utf8(), advance);
        }
        x += advance;
        let next = text[byte_index + c.len_utf8()..].chars().next();
        let can_break_after = !is_no_break_after(c) && next.is_none_or(|n| !is_no_break_before(n));
        if can_break_after {
            candidate = Some((byte_index + c.len_utf8(), x));
        }
    }
    (text.len(), x)
}

/// 去掉行尾换行符与所有回车符（CRLF 文本的 `\r` 不得进入渲染文本）。
fn clean_line(raw: &str) -> String {
    let mut line = raw;
    line = line.trim_end_matches('\n');
    line = line.trim_end_matches('\r');
    if line.contains('\r') {
        line.replace('\r', "")
    } else {
        line.to_string()
    }
}

/// 把一段文本断成至多 `max_lines` 行：返回（消费字节数，行文本）。
fn layout_lines(text: &str, typography: &Typography, max_lines: u32) -> (usize, Vec<String>) {
    let mut lines: Vec<String> = Vec::new();
    let mut offset = 0usize;
    while (lines.len() as u32) < max_lines.max(1) && offset < text.len() {
        // 段距：桌面端渲染不叠加段距，因此测量侧不插入额外空行（同口径契约）。
        let indent = if lines.is_empty() {
            typography.first_line_indent_milli
        } else {
            0
        };
        let (end, _width) = break_line(text, offset, typography, indent);
        if end <= offset {
            break;
        }
        lines.push(clean_line(&text[offset..end]));
        offset = end;
    }
    (offset, lines)
}

/// 一页的测量结果（尚未提交核心；发布时若已过期会被丢弃）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasuredPage {
    /// 页面起始字节（已对齐到合法字符边界）。
    pub start_byte: u64,
    /// 页面结束字节（不含）。
    pub end_byte: u64,
    /// 逐行文本（渲染端按同一份断行结果渲染）。
    pub lines: Vec<String>,
    /// 本页是否已到内容末尾。
    pub eof: bool,
}

impl MeasuredPage {
    /// 行数。
    pub fn line_count(&self) -> u32 {
        self.lines.len() as u32
    }

    /// 字节长度。
    pub fn byte_len(&self) -> u64 {
        self.end_byte.saturating_sub(self.start_byte)
    }

    /// 渲染文本（行以 `\n` 连接）。
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// 是否为空页（到达内容末尾且没有内容）。
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// 测量失败原因（不静默截断、不产生半页）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageError {
    /// 数据源读取失败。
    Source(SourceError),
    /// 请求的内容长度与数据源实际长度不一致。
    TotalLengthMismatch {
        /// 请求里声明的长度。
        expected: u64,
        /// 数据源实际长度。
        actual: u64,
    },
    /// 窗口数据不是合法 UTF-8（编码识别应先于分页完成）。
    WindowNotUtf8 {
        /// 窗口起始偏移。
        offset: u64,
    },
    /// 窗口数据在当前内容编码下无法解码（编码判定与实际字节不符）。
    WindowUndecodable {
        /// 窗口起始偏移。
        offset: u64,
    },
    /// 窗口已到硬上限仍未排满一页（拒绝输出「半页」，避免看起来像页面坏掉）。
    WindowExhausted {
        /// 起始偏移。
        start_byte: u64,
        /// 已使用的窗口字节数。
        window_bytes: usize,
    },
    /// 断行没有前进（防御性检查；正常路径不可能触发）。
    NoProgress {
        /// 起始偏移。
        start_byte: u64,
    },
}

impl PageError {
    /// 可读说明（状态栏/日志用，不作为业务分支）。
    pub fn message(&self) -> String {
        match self {
            PageError::Source(error) => format!("读取正文失败（{}）", error.as_str()),
            PageError::TotalLengthMismatch { expected, actual } => {
                format!("内容长度不一致：声明 {expected}，实际 {actual}")
            }
            PageError::WindowNotUtf8 { offset } => {
                format!("偏移 {offset} 处的窗口不是合法 UTF-8")
            }
            PageError::WindowUndecodable { offset } => {
                format!("偏移 {offset} 处的窗口无法按当前内容编码解码")
            }
            PageError::WindowExhausted {
                start_byte,
                window_bytes,
            } => format!("偏移 {start_byte} 处一页超过窗口硬上限（{window_bytes} 字节）"),
            PageError::NoProgress { start_byte } => format!("偏移 {start_byte} 处断行未前进"),
        }
    }
}

/// 一次页测量请求（两阶段：先测量，再回 UI 线程发布）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageMeasurementRequest {
    /// 本次排版世代（由核心缓存给出）。
    pub layout_id: u64,
    /// 本次请求标识（单调递增；更新的请求会让旧结果过期）。
    pub request_id: u64,
    /// 内容索引版本（必须与缓存键一致）。
    pub content_index_version: u32,
    /// 内容总字节数。
    pub total_bytes: u64,
    /// 页面起始字节。
    pub start_byte: u64,
}

/// 已发布的一页（缓存与渲染的单位）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagedPage {
    /// 排版世代。
    pub layout_id: u64,
    /// 请求标识。
    pub request_id: u64,
    /// 页面起始字节。
    pub start_byte: u64,
    /// 页面结束字节（不含）。
    pub end_byte: u64,
    /// 逐行文本。
    pub lines: Vec<String>,
    /// 本页是否已到内容末尾（EOF 有明确状态，不产生空白死循环页）。
    pub eof: bool,
    /// 本页在已登记边界中的精确序号（由核心给出，不是按字节比例估算）。
    pub page_index: Option<usize>,
}

impl PagedPage {
    /// 构造一页（发布路径与缓存测试共用）。
    pub fn new(
        layout_id: u64,
        request_id: u64,
        start_byte: u64,
        end_byte: u64,
        lines: Vec<String>,
        eof: bool,
        page_index: Option<usize>,
    ) -> Self {
        Self {
            layout_id,
            request_id,
            start_byte,
            end_byte,
            lines,
            eof,
            page_index,
        }
    }

    /// 行数。
    pub fn line_count(&self) -> u32 {
        self.lines.len() as u32
    }

    /// 渲染文本（行以 `\n` 连接）。
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// 页码标签（"第 N 页"，有精确序号时才给序号）。
    pub fn label(&self) -> String {
        match self.page_index {
            Some(index) => format!("第 {} 页", index + 1),
            None => "第 ? 页".to_string(),
        }
    }
}

/// 一次翻页/定位的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageOutcome {
    /// 已发布一页（唯一的成功态）。
    Ready(PagedPage),
    /// 已到内容末尾（明确 EOF，不产生空白页）。
    EndOfBook {
        /// 排版世代。
        layout_id: u64,
        /// 游标（内容长度）。
        cursor: u64,
        /// 内容总字节数。
        total_bytes: u64,
    },
    /// 已经在第一页（不能再往前）。
    AtStart {
        /// 排版世代。
        layout_id: u64,
        /// 游标（0）。
        cursor: u64,
    },
    /// 结果已过期被丢弃（旧结果不得覆盖当前页面）。
    Stale {
        /// 被丢弃的排版世代。
        stale_layout_id: u64,
        /// 被丢弃的请求号。
        stale_request_id: u64,
    },
    /// 边界被核心拒绝（丢字/重字/半字/非法区间在核心侧被挡住）。
    Rejected(BoundaryRejection),
    /// 测量失败。
    Failed(PageError),
}

impl PageOutcome {
    /// 成功态页面。
    pub fn page(&self) -> Option<&PagedPage> {
        match self {
            PageOutcome::Ready(page) => Some(page),
            _ => None,
        }
    }

    /// 是否成功。
    pub fn is_ready(&self) -> bool {
        matches!(self, PageOutcome::Ready(_))
    }

    /// 是否已到末尾。
    pub fn is_end_of_book(&self) -> bool {
        matches!(self, PageOutcome::EndOfBook { .. })
    }

    /// 游标（下一页的起点）。
    pub fn cursor(&self) -> Option<u64> {
        match self {
            PageOutcome::Ready(page) => Some(page.end_byte),
            PageOutcome::EndOfBook { cursor, .. } | PageOutcome::AtStart { cursor, .. } => {
                Some(*cursor)
            }
            _ => None,
        }
    }

    /// 一句话说明（状态栏/日志用）。
    pub fn describe(&self) -> String {
        match self {
            PageOutcome::Ready(page) => format!(
                "第 {} 行 · 字节 {}..{}",
                page.line_count(),
                page.start_byte,
                page.end_byte
            ),
            PageOutcome::EndOfBook { total_bytes, .. } => {
                format!("已到末尾（共 {total_bytes} 字节）")
            }
            PageOutcome::AtStart { .. } => "已是第一页".to_string(),
            PageOutcome::Stale { .. } => "结果已过期，已丢弃（不覆盖当前页面）".to_string(),
            PageOutcome::Rejected(rejection) => {
                format!("页面边界被拒绝（{}）", rejection.as_str())
            }
            PageOutcome::Failed(error) => error.message(),
        }
    }
}

/// 会话建立失败原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    /// 排版签名非法（核心校验失败）。
    Signature(NrError),
    /// 排版参数不可用（窗口尚未映射 / 一行都排不下）。
    Typography(TypographyError),
}

impl SessionError {
    /// 可读说明。
    pub fn message(&self) -> String {
        match self {
            SessionError::Signature(error) => format!("排版签名非法：{error}"),
            SessionError::Typography(error) => error.message(),
        }
    }
}

/// 一次重排的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflowOutcome {
    /// 新排版世代。
    pub layout_id: u64,
    /// 签名是否真的变化（未变化时复用已有页面，不清空缓存）。
    pub changed: bool,
    /// 因签名变化被作废的边界数量。
    pub removed_boundaries: usize,
    /// 重排后重新分页的起始位置（当前页起点/锚点）。
    pub start_byte: u64,
    /// 重排后的一页结果。
    pub outcome: PageOutcome,
}

/// 分页会话：一个内容单元 + 一个排版世代下的分页状态。
///
/// 说明：会话**不**保存整本正文；正文始终经 [`TextSource`] 的有限窗口读取。
/// 页边界登记在冻结核心的 [`PaginationCache`] 里，页面文本只保留
/// [`crate::page_cache`] 的有界窗口（当前页 + 前一页 + 后三页）。
pub struct PageSession<S: TextSource> {
    source: S,
    book_id: u64,
    /// 内容编码（分页测量解码、页边界校验与章节扫描共用同一口径）。
    encoding: TextEncoding,
    content_index_version: u32,
    signature: LayoutSignature,
    typography: Typography,
    boundaries: PaginationCache,
    pages: PageCache,
    history: Vec<u64>,
    current: Option<u64>,
    measurements: u64,
    discarded: u64,
}

impl<S: TextSource> PageSession<S> {
    /// 建立会话。
    ///
    /// `signature` 必须由 [`crate::reflow::layout_signature`] 在**窗口映射完成后**派生；
    /// 未映射/过小的窗口会在派生阶段就被拒绝（这里再次由核心校验兜底）。
    pub fn open(
        source: S,
        book_id: u64,
        encoding: TextEncoding,
        signature: LayoutSignature,
        font: PlatformFontMetrics,
        cache_capacity: usize,
    ) -> Result<Self, SessionError> {
        signature.validate().map_err(SessionError::Signature)?;
        let typography = Typography::derive(&signature, font).map_err(SessionError::Typography)?;
        let unit = ContentUnit::from_source(&source, book_id);
        let boundaries = PaginationCache::new(signature.clone(), unit.content_index_version)
            .map_err(SessionError::Signature)?;
        let layout_id = boundaries.layout_id();
        Ok(Self {
            source,
            book_id,
            encoding,
            content_index_version: unit.content_index_version,
            signature,
            typography,
            boundaries,
            pages: PageCache::new(layout_id, cache_capacity),
            history: Vec::new(),
            current: None,
            measurements: 0,
            discarded: 0,
        })
    }

    /// 建立会话（默认缓存容量）。
    pub fn open_default(
        source: S,
        book_id: u64,
        signature: LayoutSignature,
        font: PlatformFontMetrics,
    ) -> Result<Self, SessionError> {
        Self::open(
            source,
            book_id,
            TextEncoding::Utf8,
            signature,
            font,
            DEFAULT_CACHE_CAPACITY,
        )
    }

    /// 建立会话（**显式内容编码**）：桌面阅读器按书目记录的编码调用，非 UTF-8 文档由此贯通。
    pub fn open_with_encoding(
        source: S,
        book_id: u64,
        encoding: TextEncoding,
        signature: LayoutSignature,
        font: PlatformFontMetrics,
    ) -> Result<Self, SessionError> {
        Self::open(
            source,
            book_id,
            encoding,
            signature,
            font,
            DEFAULT_CACHE_CAPACITY,
        )
    }

    /// 内容编码（只读）。
    pub fn encoding(&self) -> TextEncoding {
        self.encoding
    }

    /// 数据源（只读）。
    pub fn source(&self) -> &S {
        &self.source
    }

    /// 书籍标识。
    pub fn book_id(&self) -> u64 {
        self.book_id
    }

    /// 当前排版签名。
    pub fn signature(&self) -> &LayoutSignature {
        &self.signature
    }

    /// 当前派生排版参数（测量与渲染共用）。
    pub fn typography(&self) -> &Typography {
        &self.typography
    }

    /// 内容总字节数。
    pub fn total_bytes(&self) -> u64 {
        self.source.len()
    }

    /// 当前排版世代。
    pub fn layout_id(&self) -> u64 {
        self.boundaries.layout_id()
    }

    /// 最近分配的请求号。
    pub fn latest_request_id(&self) -> u64 {
        self.boundaries.latest_request_id()
    }

    /// 内容索引版本。
    pub fn content_index_version(&self) -> u32 {
        self.content_index_version
    }

    /// （测量次数，被丢弃的过期结果数）。
    pub fn counters(&self) -> (u64, u64) {
        (self.measurements, self.discarded)
    }

    /// 页面缓存统计。
    pub fn cache_stats(&self) -> CacheStats {
        self.pages.stats()
    }

    /// 当前页。
    pub fn current_page(&self) -> Option<PagedPage> {
        let start = self.current?;
        self.pages.peek(start).cloned()
    }

    /// 当前游标（下一页起点）。
    pub fn cursor(&self) -> u64 {
        self.current_page()
            .map(|page| page.end_byte)
            .unwrap_or_else(|| self.history.last().copied().unwrap_or(0))
    }

    /// 已登记的页面边界（升序；证据与测试用）。
    pub fn boundaries(&self) -> Vec<PageBoundary> {
        self.boundaries.ordered_boundaries()
    }

    /// 已缓存页面的起点（升序）。
    pub fn cached_starts(&self) -> Vec<u64> {
        self.pages.starts()
    }

    /// 分配一次测量请求（请求号单调递增：更新的请求会让旧结果过期）。
    pub fn begin_measurement(&mut self, start_byte: u64) -> PageMeasurementRequest {
        let request_id = self.boundaries.begin_request();
        PageMeasurementRequest {
            layout_id: self.boundaries.layout_id(),
            request_id,
            content_index_version: self.content_index_version,
            total_bytes: self.source.len(),
            start_byte,
        }
    }

    /// 执行测量（只读数据源，不改变会话状态；可在后台线程调用）。
    pub fn measure(&self, request: &PageMeasurementRequest) -> Result<MeasuredPage, PageError> {
        measure_page_with_encoding(&self.source, request, &self.typography, self.encoding)
    }

    /// 提交并发布一次测量结果：世代/请求号过期或边界非法时**不覆盖当前页面**。
    pub fn publish(
        &mut self,
        request: &PageMeasurementRequest,
        measured: MeasuredPage,
    ) -> PageOutcome {
        if request.layout_id != self.boundaries.layout_id() {
            self.discarded += 1;
            return PageOutcome::Stale {
                stale_layout_id: request.layout_id,
                stale_request_id: request.request_id,
            };
        }
        if request.request_id < self.boundaries.latest_request_id() {
            self.discarded += 1;
            return PageOutcome::Stale {
                stale_layout_id: request.layout_id,
                stale_request_id: request.request_id,
            };
        }
        let commit = self.boundaries.commit_page_boundary(
            &self.source,
            &PageBoundaryRequest {
                layout_id: request.layout_id,
                request_id: request.request_id,
                content_index_version: request.content_index_version,
                total_bytes: request.total_bytes,
                start_byte: measured.start_byte,
                end_byte: measured.end_byte,
            },
            // 边界校验按**内容实际编码**（核心自 2026-09-29 起支持 7 种编码的边界判定；
            // UTF-8 为默认）。桌面侧的编码来源 = 书目记录（导入时识别的结果，可手动覆盖）。
            self.encoding,
        );
        match commit.status {
            BoundaryCommitStatus::Committed | BoundaryCommitStatus::DuplicateIdentical => {
                let page = PagedPage {
                    layout_id: commit.current_layout_id,
                    request_id: request.request_id,
                    start_byte: measured.start_byte,
                    end_byte: measured.end_byte,
                    lines: measured.lines,
                    eof: measured.eof,
                    page_index: self.boundaries.page_index_for_offset(measured.start_byte),
                };
                self.pages.insert(page.clone());
                PageOutcome::Ready(page)
            }
            BoundaryCommitStatus::Rejected => {
                PageOutcome::Rejected(commit.rejection.unwrap_or(BoundaryRejection::EmptyInterval))
            }
            _ => {
                self.discarded += 1;
                PageOutcome::Stale {
                    stale_layout_id: request.layout_id,
                    stale_request_id: request.request_id,
                }
            }
        }
    }

    /// 定位到某一页起点（优先使用已缓存/已登记的结果，不重复测量）。
    pub fn goto(&mut self, start_byte: u64) -> PageOutcome {
        let total = self.source.len();
        if start_byte >= total {
            return PageOutcome::EndOfBook {
                layout_id: self.boundaries.layout_id(),
                cursor: total,
                total_bytes: total,
            };
        }
        let outcome = self.load_page(start_byte);
        if let PageOutcome::Ready(page) = &outcome {
            if self.current != Some(page.start_byte) {
                if let Some(previous) = self.current {
                    self.push_history(previous);
                }
            }
            self.current = Some(page.start_byte);
        }
        outcome
    }

    /// 下一页。
    pub fn next_page(&mut self) -> PageOutcome {
        let Some(page) = self.current_page() else {
            return self.goto(0);
        };
        if page.eof || page.end_byte >= self.source.len() {
            return PageOutcome::EndOfBook {
                layout_id: self.boundaries.layout_id(),
                cursor: self.source.len(),
                total_bytes: self.source.len(),
            };
        }
        self.push_history(page.start_byte);
        self.goto(page.end_byte)
    }

    /// 上一页（优先使用核心已登记的边界；必要时从内容起点重扫）。
    pub fn prev_page(&mut self) -> PageOutcome {
        let Some(current) = self.current else {
            return PageOutcome::AtStart {
                layout_id: self.boundaries.layout_id(),
                cursor: 0,
            };
        };
        if current == 0 {
            return PageOutcome::AtStart {
                layout_id: self.boundaries.layout_id(),
                cursor: 0,
            };
        }
        let target = match self.known_previous_boundary(current) {
            Some(previous) => previous,
            None => self.rescan_previous(current),
        };
        self.history.pop();
        self.goto(target)
    }

    /// 预读：当前页 + 前一页 + 后 `forward` 页（有上限，重复调用不会重复填充）。
    ///
    /// 返回本次新填充的页数；预读**不改变当前页**。
    pub fn prefetch(&mut self, forward: usize) -> usize {
        let Some(current) = self.current_page() else {
            return 0;
        };
        let mut added = 0usize;
        let mut cursor = current.end_byte;
        for _ in 0..forward.min(MAX_PREFETCH_BATCH) {
            if cursor >= self.source.len() {
                break;
            }
            if let Some(page) = self.pages.peek(cursor).cloned() {
                cursor = page.end_byte;
                continue;
            }
            match self.load_page(cursor) {
                PageOutcome::Ready(page) => {
                    cursor = page.end_byte;
                    added += 1;
                }
                _ => break,
            }
        }
        if let Some(previous) = self.known_previous_boundary(current.start_byte) {
            if !self.pages.contains(previous) && self.load_page(previous).is_ready() {
                added += 1;
            }
        }
        added
    }

    /// 重排：切换到新签名并以当前位置（或锚点）重新分页。
    ///
    /// 合并连续设置变更与串行化（同一把锁）由 DT3B 负责；本方法只保证
    /// **一次重排的语义正确**：旧边界全部失效、旧页面缓存整批丢弃、
    /// 新页面从当前位置开始测量、在途的旧结果随后一律被丢弃。
    pub fn reflow(
        &mut self,
        signature: LayoutSignature,
        font: PlatformFontMetrics,
    ) -> Result<ReflowOutcome, SessionError> {
        signature.validate().map_err(SessionError::Signature)?;
        let typography = Typography::derive(&signature, font).map_err(SessionError::Typography)?;
        let start_byte = self
            .current_page()
            .map(|page| page.start_byte)
            .unwrap_or_else(|| self.history.last().copied().unwrap_or(0));
        let change = self
            .boundaries
            .set_layout(signature.clone(), self.content_index_version)
            .map_err(SessionError::Signature)?;
        self.signature = signature;
        self.typography = typography;
        if change.changed {
            self.pages.set_layout(self.boundaries.layout_id());
            self.history.clear();
            self.current = None;
        }
        let outcome = self.goto(start_byte);
        Ok(ReflowOutcome {
            layout_id: self.boundaries.layout_id(),
            changed: change.changed,
            removed_boundaries: change.removed,
            start_byte,
            outcome,
        })
    }

    /// 用默认版式常量建立签名并重排（桌面窗口/设置变化后的入口）。
    pub fn reflow_with_viewport(
        &mut self,
        metrics: &LayoutMetrics,
        viewport: &crate::reflow::MappedViewport,
        settings: &crate::reflow::TypographySettings,
        font: PlatformFontMetrics,
        menu_open: bool,
    ) -> Result<ReflowOutcome, SessionError> {
        let plan = crate::reflow::plan_reflow(
            Some(&self.signature),
            viewport,
            metrics,
            settings,
            font,
            menu_open,
            self.current
                .or_else(|| self.history.last().copied())
                .unwrap_or(0),
        )
        .map_err(SessionError::Typography)?;
        if !plan.changed {
            let outcome = self.goto(plan.start_byte);
            return Ok(ReflowOutcome {
                layout_id: self.boundaries.layout_id(),
                changed: false,
                removed_boundaries: 0,
                start_byte: plan.start_byte,
                outcome,
            });
        }
        self.reflow(plan.signature, font)
    }

    /// 载入一页：命中缓存直接返回，否则测量并提交核心（不改变当前页）。
    fn load_page(&mut self, start_byte: u64) -> PageOutcome {
        if let Some(page) = self.pages.get(start_byte) {
            return PageOutcome::Ready(page);
        }
        let request = self.begin_measurement(start_byte);
        self.measurements += 1;
        match self.measure(&request) {
            Ok(measured) => self.publish(&request, measured),
            Err(error) => PageOutcome::Failed(error),
        }
    }

    fn push_history(&mut self, start_byte: u64) {
        if self.history.last() != Some(&start_byte) {
            self.history.push(start_byte);
        }
    }

    /// 已登记边界中「结束于 `start_byte`」的页起点（首尾相接，不重复测量）。
    fn known_previous_boundary(&self, start_byte: u64) -> Option<u64> {
        self.boundaries
            .ordered_boundaries()
            .into_iter()
            .find(|boundary| boundary.end_byte == start_byte)
            .map(|boundary| boundary.start_byte)
    }

    /// 缺少已登记边界时（例如重排后反向翻页）：从内容起点确定性重扫到目标之前的一页。
    fn rescan_previous(&mut self, target_start: u64) -> u64 {
        let mut cursor = 0u64;
        let mut previous = 0u64;
        let mut steps = 0usize;
        while cursor < target_start && steps < MAX_BACKWARD_RESCAN_PAGES {
            let outcome = self.load_page(cursor);
            let PageOutcome::Ready(page) = outcome else {
                break;
            };
            previous = page.start_byte;
            if page.end_byte <= cursor {
                break;
            }
            cursor = page.end_byte;
            steps += 1;
        }
        previous
    }
}

/// 一个字符在给定编码下的字节长度（与核心 `is_boundary` 文档化的长度规则一致；
/// 核心的实现不对外公开，此处按同一口径重写）。
fn char_len_at(slice: &[u8], encoding: TextEncoding) -> usize {
    let Some(&lead) = slice.first() else {
        return 0;
    };
    match encoding {
        // UTF-16 必须**先于** ASCII 早退判断：汉字码元的低字节常 < 0x80（如「一」= 00 4E）。
        TextEncoding::Utf16Le | TextEncoding::Utf16Be => {
            let unit = if matches!(encoding, TextEncoding::Utf16Le) {
                u16::from_le_bytes([lead, slice.get(1).copied().unwrap_or(0)])
            } else {
                u16::from_be_bytes([lead, slice.get(1).copied().unwrap_or(0)])
            };
            if slice.len() >= 4 && (0xD800..=0xDBFF).contains(&unit) {
                4
            } else {
                2
            }
        }
        _ if lead < 0x80 => 1,
        TextEncoding::Utf8 | TextEncoding::Utf8Bom => {
            if lead >= 0xF0 {
                4
            } else if lead >= 0xE0 {
                3
            } else if lead >= 0xC0 {
                2
            } else {
                1
            }
        }
        TextEncoding::Gb18030 => {
            if slice.len() >= 4
                && (0x81..=0xFE).contains(&lead)
                && (0x30..=0x39).contains(&slice[1])
                && (0x81..=0xFE).contains(&slice[2])
                && (0x30..=0x39).contains(&slice[3])
            {
                4
            } else if (0x81..=0xFE).contains(&lead) {
                2
            } else {
                1
            }
        }
        TextEncoding::Gbk | TextEncoding::Big5 => {
            if (0x81..=0xFE).contains(&lead) {
                2
            } else {
                1
            }
        }
    }
}

/// 按内容编码解码窗口，并给出「第 i 个字符结束处的**源字节**偏移」表（长度 = 字符数 + 1）。
///
/// 该表是「断行结果（字符/字符串字节）→ 核心坐标空间（文件字节）」的唯一换算依据：
/// 核心的页边界必须落在**源文件字节**的合法字符边界上。
fn decode_window(
    data: &[u8],
    encoding: TextEncoding,
    base: u64,
) -> Result<(String, Vec<usize>), PageError> {
    if matches!(encoding, TextEncoding::Utf8 | TextEncoding::Utf8Bom) {
        // UTF-8：保持既有行为与既有错误（窗口非 UTF-8 → WindowNotUtf8）。
        let text =
            std::str::from_utf8(data).map_err(|_| PageError::WindowNotUtf8 { offset: base })?;
        let mut offsets: Vec<usize> = text.char_indices().map(|(index, _)| index).collect();
        offsets.push(text.len());
        return Ok((text.to_string(), offsets));
    }
    let decoded = reading_source::decode_line(data, encoding)
        .ok_or(PageError::WindowUndecodable { offset: base })?;
    // BOM：`encoding_rs` 的 `decode()` 会按编码标准**剥离** UTF-16 的 BOM（UTF-8 走零拷贝借用，
    // 不剥离），而字节长度表必须与「实际解码出的字符数」逐一对齐，因此此处显式跳过 BOM 字节。
    let skip = match reading_source::detect_bom(data) {
        Some((bom_encoding, len)) if bom_encoding == encoding => match encoding {
            TextEncoding::Utf16Le | TextEncoding::Utf16Be => len,
            _ => 0,
        },
        _ => 0,
    };
    let mut offsets: Vec<usize> = Vec::with_capacity(decoded.chars().count() + 1);
    offsets.push(0);
    let mut cursor = skip.min(data.len());
    while cursor < data.len() {
        let step = char_len_at(&data[cursor..], encoding);
        if step == 0 {
            break;
        }
        cursor = (cursor + step).min(data.len());
        offsets.push(cursor);
    }
    if offsets.len() != decoded.chars().count() + 1 {
        // 长度规则与实际解码不一致（坏数据 / 编码判定与字节不符）：如实报错，
        // 绝不「近似映射」——错位的页边界会被核心拒绝，或更糟：静默丢字。
        return Err(PageError::WindowUndecodable { offset: base });
    }
    Ok((decoded.into_owned(), offsets))
}

/// 断行消耗的部分（以 UTF-8 字符串字节表示）换算回**源文件字节**。
fn consumed_source_bytes(text: &str, consumed: usize, offsets: &[usize]) -> usize {
    let consumed = consumed.min(text.len());
    let chars = text[..consumed].chars().count();
    offsets
        .get(chars)
        .copied()
        .unwrap_or_else(|| offsets.last().copied().unwrap_or(0))
}

/// 测量一页（UTF-8 口径的兼容入口；其他编码见 [`measure_page_with_encoding`]）。
pub fn measure_page<S: TextSource + ?Sized>(
    source: &S,
    request: &PageMeasurementRequest,
    typography: &Typography,
) -> Result<MeasuredPage, PageError> {
    measure_page_with_encoding(source, request, typography, TextEncoding::Utf8)
}

/// 测量一页（**按内容编码**）：窗口按编码对齐、按编码解码、页尾偏移按编码的字符长度
/// 换算回**源文件字节**（核心的坐标空间正是文件字节，两侧口径必须一致）。
pub fn measure_page_with_encoding<S: TextSource + ?Sized>(
    source: &S,
    request: &PageMeasurementRequest,
    typography: &Typography,
    encoding: TextEncoding,
) -> Result<MeasuredPage, PageError> {
    let total = source.len();
    if request.total_bytes != total {
        return Err(PageError::TotalLengthMismatch {
            expected: request.total_bytes,
            actual: total,
        });
    }
    if request.start_byte >= total {
        return Ok(MeasuredPage {
            start_byte: request.start_byte,
            end_byte: request.start_byte,
            lines: Vec::new(),
            eof: true,
        });
    }
    let max_lines = typography.lines_per_page();
    let window_cap = source.max_window_bytes().min(MAX_PAGE_WINDOW_BYTES);
    let mut window_bytes = INITIAL_PAGE_WINDOW_BYTES.min(window_cap).max(1);

    loop {
        let window = source
            .read_at(request.start_byte, window_bytes)
            .map_err(PageError::Source)?;
        if window.is_empty() {
            return Ok(MeasuredPage {
                start_byte: request.start_byte,
                end_byte: request.start_byte,
                lines: Vec::new(),
                eof: true,
            });
        }
        let base = window.start();
        let (text, offsets) = decode_window(&window.data, encoding, base)?;
        let (consumed, lines) = layout_lines(&text, typography, max_lines);
        let filled = lines.len() as u32 >= max_lines;
        let exhausted = consumed >= text.len();
        if exhausted && !window.eof && !filled {
            if window_bytes >= window_cap {
                return Err(PageError::WindowExhausted {
                    start_byte: base,
                    window_bytes,
                });
            }
            window_bytes = (window_bytes * 2).min(window_cap);
            continue;
        }
        if consumed == 0 {
            return Err(PageError::NoProgress { start_byte: base });
        }
        let end_byte = base + consumed_source_bytes(&text, consumed, &offsets) as u64;
        return Ok(MeasuredPage {
            start_byte: base,
            end_byte,
            lines,
            eof: window.eof && exhausted,
        });
    }
}

/// 打开的书（阅读运行时的输入）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedBook {
    /// 书籍标识。
    pub book_id: i64,
    /// 显示名（书名）。
    pub title: String,
    /// 原始文件路径（桌面来源定位符）。
    pub path: PathBuf,
    /// 核心存储层给出的来源状态（`ready` / `missing` / `needs_reindex` …）。
    pub status: String,
    /// 内容编码（导入时识别、可由用户手动覆盖）：分页解码与页边界校验共用同一口径。
    pub encoding: TextEncoding,
    /// 编码是否由用户手动指定（`true` = 界面「编码」面板选用；`false` = 导入识别）。
    pub encoding_user_override: bool,
}

/// 书目记录里的编码名（`TextEncoding` 的展示名，导入/覆盖时写入）→ 编码枚举。
///
/// 未知或空值退回 UTF-8（不抛错）；界面仍显示记录里的原始名，避免「悄悄换了口径」。
pub fn parse_encoding(name: &str) -> TextEncoding {
    match name.trim().to_ascii_uppercase().as_str() {
        "UTF-8-BOM" | "UTF8-BOM" | "UTF-8 BOM" => TextEncoding::Utf8Bom,
        "UTF-16LE" | "UTF16LE" => TextEncoding::Utf16Le,
        "UTF-16BE" | "UTF16BE" => TextEncoding::Utf16Be,
        "GB18030" => TextEncoding::Gb18030,
        "GBK" | "CP936" => TextEncoding::Gbk,
        "BIG5" => TextEncoding::Big5,
        _ => TextEncoding::Utf8,
    }
}

impl OpenedBook {
    /// 来源是否可读（只有 `ready` 才允许分页；失效来源走 DT2 的重新关联入口）。
    pub fn is_readable(&self) -> bool {
        self.status == reading_storage::books::STATUS_READY
    }
}

/// 选择并打开阅读目标。
///
/// `selection` 为空或 `first` 时取书架第一本；否则按书籍标识解析。
/// 只读取已有记录，不修改任何文件（导入/重新关联属于 DT2）。
pub fn open_book(db_path: &Path, selection: Option<&str>) -> Result<OpenedBook, String> {
    let mut storage = reading_storage::Storage::open(db_path)
        .map_err(|error| format!("无法打开书架数据库：{error}"))?;
    storage
        .migrate()
        .map_err(|error| format!("书架数据库迁移失败：{error}"))?;
    let records = reading_storage::books::list_shelf_books(storage.conn())
        .map_err(|error| format!("读取书架失败：{error}"))?;
    let selected = match selection.map(str::trim) {
        None | Some("") | Some("first") => records.first().cloned(),
        Some(text) => {
            let book_id = text
                .parse::<i64>()
                .map_err(|_| format!("书籍标识非法：{text}"))?;
            records.iter().find(|record| record.id == book_id).cloned()
        }
    };
    let record = selected.ok_or_else(|| "书架为空：请先在书架导入 TXT".to_string())?;
    Ok(OpenedBook {
        book_id: record.id,
        title: record.display_name.clone(),
        path: PathBuf::from(record.source_locator.clone()),
        status: record.status.clone(),
        encoding: parse_encoding(&record.encoding),
        encoding_user_override: record.encoding_user_override,
    })
}

/// 编码覆盖（[`ReaderRuntime::reopen_with_encoding`]）的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReopenOutcome {
    /// 生效的新编码。
    pub encoding: TextEncoding,
    /// 覆盖前请求保留的字节偏移。
    pub requested_offset: u64,
    /// 该偏移在新编码下是否被精确保留（`false` = 已如实回退到内容起点）。
    pub offset_retained: bool,
    /// 重建后的首屏结果。
    pub outcome: PageOutcome,
}

/// 千分比 → 进度文案（`42.3%`；与 Android 端同一展示口径）。
pub fn format_progress_percent(permille: u32) -> String {
    let permille = permille.min(1_000);
    format!("{}.{}%", permille / 10, permille % 10)
}

/// 阅读运行时建立失败的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReaderRuntimeError {
    /// 书架/来源层错误（可读信息）。
    Storage(String),
    /// 原始文件不可读。
    Source(SourceError),
    /// 分页会话建立失败（窗口未映射 / 签名非法）。
    Session(SessionError),
}

impl ReaderRuntimeError {
    /// 可读说明（状态栏用）。
    pub fn message(&self) -> String {
        match self {
            ReaderRuntimeError::Storage(message) => message.clone(),
            ReaderRuntimeError::Source(error) => {
                format!("正文来源不可读（{}）", error.as_str())
            }
            ReaderRuntimeError::Session(error) => error.message(),
        }
    }
}

/// 桌面阅读运行时：把「打开的书 + 分页会话 + 排版状态 + 视口」绑在一起。
///
/// UI 只做状态投影：本类型持有分页状态，UI 不复制任何分页语义。
/// 串行化与重排请求合并（同一把锁、只保留最终值）由 DT3B 负责；此处保证单次操作语义正确。
pub struct ReaderRuntime {
    session: PageSession<EncodedFileSource>,
    book: OpenedBook,
    metrics: LayoutMetrics,
    viewport: MappedViewport,
    settings: TypographySettings,
    font: PlatformFontMetrics,
    menu_open: bool,
    /// 沉浸阅读（顶栏 / 底部状态栏 / 菜单条不再占位；正文区使用整个窗口）。
    immersive: bool,
    turns: u64,
    reflows: u64,
}

impl ReaderRuntime {
    /// 打开一本书并完成首次分页（**必须**在窗口映射后调用）。
    pub fn open(
        book: OpenedBook,
        metrics: LayoutMetrics,
        viewport: MappedViewport,
        settings: TypographySettings,
        font: PlatformFontMetrics,
        menu_open: bool,
    ) -> Result<Self, ReaderRuntimeError> {
        let plan =
            crate::reflow::plan_reflow(None, &viewport, &metrics, &settings, font, menu_open, 0)
                .map_err(|error| ReaderRuntimeError::Session(SessionError::Typography(error)))?;
        let source = EncodedFileSource::open(&book.path, book.encoding)
            .map_err(ReaderRuntimeError::Source)?;
        let mut session = PageSession::open_with_encoding(
            source,
            book.book_id.max(0) as u64,
            book.encoding,
            plan.signature,
            font,
        )
        .map_err(ReaderRuntimeError::Session)?;
        let _ = session.goto(0);
        let _ = session.prefetch(crate::page_cache::PREFETCH_FORWARD);
        Ok(Self {
            session,
            book,
            metrics,
            viewport,
            settings,
            font,
            menu_open,
            immersive: false,
            turns: 0,
            reflows: 0,
        })
    }

    /// 当前书。
    pub fn book(&self) -> &OpenedBook {
        &self.book
    }

    /// 当前页。
    pub fn page(&self) -> Option<PagedPage> {
        self.session.current_page()
    }

    /// 当前内容编码（只读）。
    pub fn encoding(&self) -> TextEncoding {
        self.session.encoding()
    }

    /// 沉浸状态（只读）。
    pub fn immersive(&self) -> bool {
        self.immersive
    }

    /// 内容总字节数（进度条跳转的换算基准）。
    pub fn total_bytes(&self) -> u64 {
        self.session.total_bytes()
    }

    /// 当前生效的版式常量（沉浸态为派生值；渲染几何与测量同源）。
    pub fn layout_metrics(&self) -> LayoutMetrics {
        if self.immersive {
            self.metrics.immersive()
        } else {
            self.metrics
        }
    }

    /// 当前视口（窗口逻辑尺寸；渲染几何与测量同源）。
    pub fn viewport(&self) -> MappedViewport {
        self.viewport
    }

    /// 阅读工具条（菜单）展开态。
    pub fn menu_open(&self) -> bool {
        self.menu_open
    }

    /// 阅读进度（按**字节位置**的千分比）。
    ///
    /// 页码只属于某个排版签名（切章/重开/改字号后必然失真），因此进度读数一律用字节位置：
    /// 任何时刻都可算、且不会退回 0（与 Android 端同一口径）。
    pub fn progress_permille(&self) -> u32 {
        let total = self.session.total_bytes();
        if total == 0 {
            return 0;
        }
        let start = self
            .session
            .current_page()
            .map(|page| page.start_byte)
            .unwrap_or(0);
        ((start.saturating_mul(1_000)) / total).min(1_000) as u32
    }

    /// 进入 / 退出沉浸阅读：按**沉浸派生**的版式重排（菜单条不再占位），位置按字节保持。
    ///
    /// 与菜单切换共用同一重排入口（[`ReaderRuntime::reflow_now`]）：版式变化必须重排，
    /// 位置保持由核心的边界/锚点语义保证；界面只做显隐。
    pub fn set_immersive(&mut self, immersive: bool) -> Result<ReflowOutcome, SessionError> {
        self.immersive = immersive;
        self.reflow_now()
    }

    /// 当前生效的版式常量：沉浸阅读时使用派生版式（正文区 = 整个窗口）。
    fn active_metrics(&self) -> LayoutMetrics {
        if self.immersive {
            self.metrics.immersive()
        } else {
            self.metrics
        }
    }

    /// 切换内容编码并**保留当前字节位置**重建会话（用户手动覆盖编码的入口）。
    ///
    /// 语义：
    /// - 数据源与会话都按新编码重建（窗口对齐、解码、字节计数、边界校验口径同步切换）；
    /// - 位置按**字节偏移**保持：该偏移在新编码下若是合法边界则就地恢复，
    ///   否则回退到内容起点，并如实回报 `offset_retained=false`（绝不伪造位置）；
    /// - 章节目录不在本方法内处理：调用方须递增内容索引版本并重建目录后再调用。
    pub fn reopen_with_encoding(
        &mut self,
        encoding: TextEncoding,
    ) -> Result<ReopenOutcome, ReaderRuntimeError> {
        let target = self
            .session
            .current_page()
            .map(|page| page.start_byte)
            .unwrap_or(0);
        self.book.encoding = encoding;
        let source = EncodedFileSource::open(&self.book.path, encoding)
            .map_err(ReaderRuntimeError::Source)?;
        let mut session = PageSession::open_with_encoding(
            source,
            self.book.book_id.max(0) as u64,
            encoding,
            self.session.signature().clone(),
            self.font,
        )
        .map_err(ReaderRuntimeError::Session)?;
        let mut outcome = session.goto(target);
        let offset_retained = outcome.is_ready();
        if !offset_retained {
            outcome = session.goto(0);
        }
        self.session = session;
        self.turns = 0;
        self.reflows += 1;
        Ok(ReopenOutcome {
            encoding,
            requested_offset: target,
            offset_retained,
            outcome,
        })
    }

    /// 编码覆盖后的重建结果 → 结构化证据行（`NR-ENCODING`）。
    pub fn reopen_state_log(&self, result: &ReopenOutcome) -> String {
        format!(
            "NR-ENCODING override=1 encoding={} requested_offset={} retained={} layout={} start={} end={}",
            result.encoding,
            result.requested_offset,
            u8::from(result.offset_retained),
            self.session.layout_id(),
            self.session
                .current_page()
                .map(|page| page.start_byte)
                .unwrap_or(0),
            self.session
                .current_page()
                .map(|page| page.end_byte)
                .unwrap_or(0),
        )
    }

    /// 当前派生排版参数（测量与渲染共用）。
    pub fn typography(&self) -> &Typography {
        self.session.typography()
    }

    /// 当前排版世代。
    pub fn layout_id(&self) -> u64 {
        self.session.layout_id()
    }

    /// 当前签名。
    pub fn signature(&self) -> &LayoutSignature {
        self.session.signature()
    }

    /// 翻页计数与重排计数。
    pub fn counters(&self) -> (u64, u64) {
        (self.turns, self.reflows)
    }

    /// 下一页。
    pub fn next_page(&mut self) -> PageOutcome {
        let outcome = self.session.next_page();
        if outcome.is_ready() {
            self.turns += 1;
            let _ = self.session.prefetch(crate::page_cache::PREFETCH_FORWARD);
        }
        outcome
    }

    /// 上一页。
    pub fn prev_page(&mut self) -> PageOutcome {
        let outcome = self.session.prev_page();
        if outcome.is_ready() {
            self.turns += 1;
            let _ = self.session.prefetch(crate::page_cache::PREFETCH_FORWARD);
        }
        outcome
    }

    /// 连续翻页（正数向后、负数向前；程序化验证用）。
    pub fn turn_pages(&mut self, turns: i32) -> PageOutcome {
        let mut outcome = self
            .session
            .current_page()
            .map(PageOutcome::Ready)
            .unwrap_or_else(|| self.session.goto(0));
        for _ in 0..turns.abs() {
            outcome = if turns >= 0 {
                self.session.next_page()
            } else {
                self.session.prev_page()
            };
            if !outcome.is_ready() {
                break;
            }
            self.turns += 1;
        }
        let _ = self.session.prefetch(crate::page_cache::PREFETCH_FORWARD);
        outcome
    }

    /// 跳转到指定字节偏移（DT4：章节跳转 / 书签跳转 / 重启恢复共用入口）。
    ///
    /// 只做「以内容位置重新测量」：偏移来自核心位置语义（章节起点或 `ContentPosition`），
    /// 不来自页码；越界偏移由核心回给 `EndOfBook`，不产生空白页。
    pub fn goto_byte(&mut self, start_byte: u64) -> PageOutcome {
        let outcome = self.session.goto(start_byte);
        if outcome.is_ready() {
            let _ = self.session.prefetch(crate::page_cache::PREFETCH_FORWARD);
        }
        outcome
    }

    /// 窗口 / DPI 变化：以当前位置重新分页（签名未变化时幂等复用）。
    pub fn apply_viewport(
        &mut self,
        viewport: MappedViewport,
    ) -> Result<ReflowOutcome, SessionError> {
        if viewport == self.viewport {
            let start = self
                .session
                .current_page()
                .map(|page| page.start_byte)
                .unwrap_or(0);
            return Ok(ReflowOutcome {
                layout_id: self.session.layout_id(),
                changed: false,
                removed_boundaries: 0,
                start_byte: start,
                outcome: self.session.goto(start),
            });
        }
        let outcome = self.session.reflow_with_viewport(
            &self.active_metrics(),
            &viewport,
            &self.settings,
            self.font,
            self.menu_open,
        )?;
        self.viewport = viewport;
        if outcome.changed {
            self.reflows += 1;
        }
        Ok(outcome)
    }

    /// 字号变化（像素）：以当前位置重新分页。
    pub fn set_font_size_px(&mut self, font_size_px: u32) -> Result<ReflowOutcome, SessionError> {
        let mut settings = self.settings.clone();
        settings.font_size_milli = font_size_px.saturating_mul(1_000);
        self.settings = settings;
        self.reflow_now()
    }

    /// 展开/收起阅读菜单（正文区域高度变化 → 重排）。
    pub fn toggle_menu(&mut self) -> Result<ReflowOutcome, SessionError> {
        self.menu_open = !self.menu_open;
        self.reflow_now()
    }

    /// 平台字体度量变化（窗口映射后取得）：必要时重排。
    pub fn set_font_metrics(
        &mut self,
        font: PlatformFontMetrics,
    ) -> Result<ReflowOutcome, SessionError> {
        if font == self.font {
            let start = self
                .session
                .current_page()
                .map(|page| page.start_byte)
                .unwrap_or(0);
            return Ok(ReflowOutcome {
                layout_id: self.session.layout_id(),
                changed: false,
                removed_boundaries: 0,
                start_byte: start,
                outcome: self.session.goto(start),
            });
        }
        self.font = font;
        self.reflow_now()
    }

    /// 排版设置整体变化（DT4 接入设置表后由 UI 传入）。
    pub fn set_typography_settings(
        &mut self,
        settings: TypographySettings,
    ) -> Result<ReflowOutcome, SessionError> {
        self.settings = settings;
        self.reflow_now()
    }

    /// 一行分页状态（`NR-PAGE` 日志与状态栏证据）。
    pub fn state_log(&self) -> String {
        let stats = self.session.cache_stats();
        let typography = self.session.typography();
        let (dropped_measurements, discarded) = self.session.counters();
        // 正文区（含留白）= 窗口减去装饰的几何事实：沉浸态下等于整窗。
        let metrics = self.layout_metrics();
        let area = metrics.content_area(self.viewport.width, self.viewport.height, self.menu_open);
        let pad_h = metrics.content_padding_h_for(self.viewport.width);
        let pad_v = metrics.content_padding_v_for(self.viewport.width);
        let region = (area.width + 2 * pad_h, area.height + 2 * pad_v);
        match self.session.current_page() {
            Some(page) => format!(
                "NR-PAGE book={} layout={} req={} page_index={} start={} end={} lines={} \
                 lines_per_page={} pitch={} font={} window={}x{} viewport={}x{} region={}x{} menu={} immersive={} eof={} turns={} \
                 reflows={} measured={} stale={} cached={} total={}",
                self.book.book_id,
                page.layout_id,
                page.request_id,
                page.page_index.map(|index| index as i64).unwrap_or(-1),
                page.start_byte,
                page.end_byte,
                page.line_count(),
                typography.lines_per_page(),
                typography.line_pitch_px(),
                typography.font_size_px(),
                self.viewport.width,
                self.viewport.height,
                typography.line_width_px(),
                typography.content_height_milli / 1_000,
                region.0,
                region.1,
                u8::from(self.menu_open),
                u8::from(self.immersive),
                u8::from(page.eof),
                self.turns,
                self.reflows,
                dropped_measurements,
                discarded,
                stats.len,
                self.session.total_bytes(),
            ),
            None => format!(
                "NR-PAGE book={} layout={} req=0 page_index=-1 start=0 end=0 lines=0 \
                 lines_per_page={} pitch={} font={} window={}x{} viewport={}x{} region={}x{} menu={} eof=0 turns={} \
                 reflows={} measured={} stale={} cached={} total={}",
                self.book.book_id,
                self.session.layout_id(),
                typography.lines_per_page(),
                typography.line_pitch_px(),
                typography.font_size_px(),
                self.viewport.width,
                self.viewport.height,
                typography.line_width_px(),
                typography.content_height_milli / 1_000,
                region.0,
                region.1,
                u8::from(self.menu_open),
                self.turns,
                self.reflows,
                dropped_measurements,
                discarded,
                stats.len,
                self.session.total_bytes(),
            ),
        }
    }

    fn reflow_now(&mut self) -> Result<ReflowOutcome, SessionError> {
        let start = self
            .session
            .current_page()
            .map(|page| page.start_byte)
            .unwrap_or(0);
        let plan = crate::reflow::plan_reflow(
            Some(self.session.signature()),
            &self.viewport,
            &self.active_metrics(),
            &self.settings,
            self.font,
            self.menu_open,
            start,
        )
        .map_err(SessionError::Typography)?;
        if !plan.changed {
            return Ok(ReflowOutcome {
                layout_id: self.session.layout_id(),
                changed: false,
                removed_boundaries: 0,
                start_byte: start,
                outcome: self.session.goto(start),
            });
        }
        let outcome = self.session.reflow(plan.signature, self.font)?;
        if outcome.changed {
            self.reflows += 1;
        }
        let _ = self.session.prefetch(crate::page_cache::PREFETCH_FORWARD);
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use reading_source::FileTextSource;

    use super::*;
    use crate::reflow::{layout_signature, MappedViewport, TypographySettings};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempGuard {
        dir: PathBuf,
    }

    impl Drop for TempGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn temp_source(label: &str, content: &[u8]) -> (TempGuard, FileTextSource) {
        let dir = std::env::temp_dir().join(format!(
            "nr-dt3-pagination-{}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
            label
        ));
        std::fs::create_dir_all(&dir).expect("创建临时目录");
        let path = dir.join("book.txt");
        std::fs::write(&path, content).expect("写入临时文件");
        let source = FileTextSource::open(&path).expect("打开临时文件");
        (TempGuard { dir }, source)
    }

    fn font() -> PlatformFontMetrics {
        PlatformFontMetrics::from_px(22, 5)
    }

    fn signature_for(width: u32, height: u32) -> LayoutSignature {
        layout_signature(
            &MappedViewport::new(width, height),
            &LayoutMetrics::DESKTOP,
            &TypographySettings::default(),
            font(),
            false,
        )
        .expect("窗口映射后可派生签名")
    }

    fn session_for(content: &[u8], label: &str) -> (TempGuard, PageSession<FileTextSource>) {
        let (guard, source) = temp_source(label, content);
        let session = PageSession::open_default(source, 1, signature_for(1_080, 720), font())
            .expect("建立分页会话");
        (guard, session)
    }

    /// 长篇中文夹具：每段 5 行、约 190 字，用于 200 页正反向压力分页。
    fn novel_bytes(paragraphs: usize) -> Vec<u8> {
        let mut text = String::new();
        for index in 1..=paragraphs {
            text.push_str(&format!("第{index}章 分页验证\n"));
            for line in 0..4 {
                text.push_str(&format!(
                    "这是第{line}行用于整页分页验证的中文正文，包含标点：句号、逗号、顿号与引号“示例”。\n"
                ));
            }
            text.push('\n');
        }
        text.into_bytes()
    }

    #[test]
    fn pagination_200_pages_forward_are_contiguous() {
        let content = novel_bytes(1_500);
        let (guard, mut session) = session_for(&content, "forward200");
        // 释放文件句柄前统计物化字节：单次读取必须 ≤ 窗口硬上限（不得整本复制）。
        let mut pages: Vec<PagedPage> = Vec::new();
        let outcome = session.goto(0);
        let PageOutcome::Ready(first) = outcome else {
            panic!("首页必须可读：{outcome:?}");
        };
        assert_eq!(first.start_byte, 0, "首页必须从 0 开始");
        pages.push(first);

        for _ in 1..200 {
            match session.next_page() {
                PageOutcome::Ready(page) => pages.push(page),
                other => panic!("第 {} 页不可读：{other:?}", pages.len() + 1),
            }
        }
        assert_eq!(pages.len(), 200);
        assert!(session.boundaries().len() >= 200);

        let expected_lines = session.typography().lines_per_page();
        let mut previous_end = 0u64;
        let mut expected_text = String::new();
        for (index, page) in pages.iter().enumerate() {
            assert_eq!(
                page.start_byte,
                previous_end,
                "第 {} 页必须与上一页首尾相接（不丢字、不重字）",
                index + 1
            );
            assert!(page.end_byte > page.start_byte, "页面必须严格前进");
            // 每页（除末页）都必须用满行数：底部不留整行空白。
            if !page.eof {
                assert_eq!(
                    page.line_count(),
                    expected_lines,
                    "第 {} 页必须排满 {expected_lines} 行",
                    index + 1
                );
            }
            let slice =
                std::str::from_utf8(&content[page.start_byte as usize..page.end_byte as usize])
                    .expect("夹具是合法 UTF-8");
            let joined: String = page.lines.join("");
            let normalized: String = slice.chars().filter(|c| *c != '\r' && *c != '\n').collect();
            assert_eq!(
                joined,
                normalized,
                "第 {} 页的渲染文本必须与源切片逐字一致（无丢字/重字）",
                index + 1
            );
            expected_text.push_str(slice);
            previous_end = page.end_byte;
        }
        // 核心的连续性校验：每条相邻边界都必须首尾相接且严格前进。
        let boundaries = session.boundaries();
        for window in boundaries.windows(2) {
            reading_core::check_strict_advance(&window[0], &window[1]).expect("页面必须连续");
        }
        assert!(expected_text.len() > 200 * 1_000, "夹具必须足够长");
        let stats = session.source().stats();
        assert!(
            stats.max_materialized_bytes <= MAX_WINDOW_BYTES as u64,
            "单次读取不得越过窗口硬上限：{}",
            stats.max_materialized_bytes
        );
        drop(guard);
    }

    #[test]
    fn backward_100_pages_match_forward_measurements() {
        let content = novel_bytes(1_500);
        let (guard, mut session) = session_for(&content, "backward100");
        let mut forward: Vec<PagedPage> = Vec::new();
        match session.goto(0) {
            PageOutcome::Ready(page) => forward.push(page),
            other => panic!("首页必须可读：{other:?}"),
        }
        for _ in 1..200 {
            match session.next_page() {
                PageOutcome::Ready(page) => forward.push(page),
                other => panic!("正向翻页失败：{other:?}"),
            }
        }

        // 反向 100 页：每一页都必须与正向测量逐字节、逐行一致。
        for step in 1..=100usize {
            let expected = &forward[forward.len() - 1 - step];
            match session.prev_page() {
                PageOutcome::Ready(page) => {
                    assert_eq!(page.start_byte, expected.start_byte, "反向第 {step} 页起点");
                    assert_eq!(page.end_byte, expected.end_byte, "反向第 {step} 页终点");
                    assert_eq!(page.lines, expected.lines, "反向第 {step} 页内容");
                    assert_eq!(page.page_index, expected.page_index);
                }
                other => panic!("反向第 {step} 页失败：{other:?}"),
            }
        }
        // 继续反向到第一页后必须给出明确的起始状态（不产生空白页）。
        for _ in 0..100 {
            session.prev_page();
        }
        for _ in 0..200 {
            match session.prev_page() {
                PageOutcome::AtStart { cursor, .. } => {
                    assert_eq!(cursor, 0);
                    break;
                }
                PageOutcome::Ready(_) => continue,
                other => panic!("反向翻页异常：{other:?}"),
            }
        }
        match session.prev_page() {
            PageOutcome::AtStart { cursor, .. } => assert_eq!(cursor, 0),
            other => panic!("首页之前必须给出 AtStart：{other:?}"),
        }
        drop(guard);
    }

    #[test]
    fn pagination_reaches_end_of_book_without_blank_pages() {
        let content = "短篇：一页多一点点的正文，用来验证 EOF。\n第二段。\n"
            .repeat(30)
            .into_bytes();
        let (guard, mut session) = session_for(&content, "eof");
        let mut seen = 0usize;
        let mut outcome = session.goto(0);
        loop {
            match outcome {
                PageOutcome::Ready(page) => {
                    seen += 1;
                    assert!(!page.lines.is_empty(), "不得出现空白页");
                    outcome = session.next_page();
                    if page.eof {
                        // 末页必须带明确 EOF 状态；再翻页只能是 EndOfBook。
                        match session.next_page() {
                            PageOutcome::EndOfBook {
                                cursor,
                                total_bytes,
                                ..
                            } => {
                                assert_eq!(cursor, total_bytes);
                                assert_eq!(cursor, content.len() as u64);
                                break;
                            }
                            other => panic!("末页之后必须给出 EndOfBook：{other:?}"),
                        }
                    }
                }
                PageOutcome::EndOfBook { .. } => break,
                other => panic!("翻页异常：{other:?}"),
            }
        }
        assert!(seen >= 2, "夹具应当至少两页：{seen}");
        drop(guard);
    }

    #[test]
    fn utf8_boundary_pages_never_split_characters() {
        // 混入 4 字节字符（扩展 CJK）与 emoji：任何页面边界都必须落在字符边界上。
        let mut text = String::new();
        for index in 0..400 {
            text.push_str(&format!(
                "第{index}段：\u{20000}\u{20001}𠀀测试🧪𝄞多字节边界与标点。"
            ));
            text.push('\n');
        }
        let content = text.clone().into_bytes();
        let (guard, mut session) = session_for(&content, "utf8");
        let mut boundaries: Vec<(u64, u64)> = Vec::new();
        let mut outcome = session.goto(0);
        while let PageOutcome::Ready(page) = outcome {
            assert!(
                text.is_char_boundary(page.start_byte as usize),
                "页面起点 {} 必须位于字符边界",
                page.start_byte
            );
            assert!(
                text.is_char_boundary(page.end_byte as usize),
                "页面终点 {} 必须位于字符边界（不得半字裁切）",
                page.end_byte
            );
            boundaries.push((page.start_byte, page.end_byte));
            // 语义复核：两侧原本都调用 `session.next_page()`，`eof` 未参与分支结果。
            // `next_page()` 内部已按 `page.eof || page.end_byte >= source.len()` 判定 EndOfBook
            // （见本文件 PageSession::next_page），因此 `eof` 真假不可能改变走向——
            // 并非「漏写分支体」，等价合并；循环终止仍由 next_page 返回非 Ready 保证。
            outcome = session.next_page();
            if boundaries.len() > 4_000 {
                panic!("分页未收敛（疑似死循环）");
            }
        }
        assert!(!boundaries.is_empty());
        // 全量覆盖：相邻页首尾相接，最后一页抵达内容末尾。
        let mut cursor = 0u64;
        for (start, end) in &boundaries {
            assert_eq!(*start, cursor, "页面之间不得有缝隙或重叠");
            cursor = *end;
        }
        assert_eq!(cursor, content.len() as u64, "分页必须覆盖全文");

        // 起点落在多字节字符中间时（防御路径）：必须被对齐到字符边界且仍严格前进。
        let mid = (1..text.len()).find(|offset| !text.is_char_boundary(*offset));
        if let Some(mid) = mid {
            match session.goto(mid as u64) {
                PageOutcome::Ready(page) => {
                    assert!(page.start_byte >= mid as u64);
                    assert!(text.is_char_boundary(page.start_byte as usize));
                    assert!(page.start_byte <= page.end_byte);
                }
                other => panic!("非字符边界起点必须被对齐而不是失败：{other:?}"),
            }
        }
        drop(guard);
    }

    #[test]
    fn pagination_long_single_line_advances_without_looping() {
        // 超长单行（没有换行）+ 极窄正文区：必须按宽度断行并严格前进。
        let content = "长".repeat(5_000).into_bytes();
        let (guard, source) = temp_source("longline", &content);
        let narrow = signature_for(200, 400);
        let mut session = PageSession::open_default(source, 1, narrow, font()).expect("建立会话");
        let (mut cursor, mut pages) = (0u64, 0usize);
        loop {
            match session.goto(cursor) {
                PageOutcome::Ready(page) => {
                    assert!(page.end_byte > page.start_byte, "必须严格前进");
                    pages += 1;
                    cursor = page.end_byte;
                    if page.eof {
                        break;
                    }
                }
                PageOutcome::EndOfBook { .. } => break,
                other => panic!("窄窗口分页异常：{other:?}"),
            }
            assert!(pages < 5_000, "分页未收敛");
        }
        assert!(pages > 1);
        assert_eq!(cursor, content.len() as u64);
        drop(guard);
    }

    #[test]
    fn pagination_stale_layout_result_is_discarded() {
        let content = novel_bytes(20);
        let (guard, mut session) = session_for(&content, "stale");
        let first = session.goto(0);
        let PageOutcome::Ready(first_page) = first else {
            panic!("首页必须可读");
        };

        // 在途测量：分配请求 → 期间排版世代变化 → 发布时必须丢弃。
        let request = session.begin_measurement(first_page.end_byte);
        let measured = session.measure(&request).expect("测量成功");
        let settings = TypographySettings {
            font_size_milli: 24_000,
            ..Default::default()
        };
        let plan = crate::reflow::plan_reflow(
            Some(session.signature()),
            &MappedViewport::new(1_080, 720),
            &LayoutMetrics::DESKTOP,
            &settings,
            font(),
            false,
            first_page.start_byte,
        )
        .expect("重排计划");
        let reflowed = session.reflow(plan.signature, font()).expect("重排成功");
        assert!(reflowed.changed);
        assert_ne!(reflowed.layout_id, first_page.layout_id);

        let outcome = session.publish(&request, measured);
        assert!(
            matches!(outcome, PageOutcome::Stale { .. }),
            "过期结果必须被丢弃：{outcome:?}"
        );
        assert_eq!(session.counters().1, 1);
        // 当前页面仍然是重排后测量出来的那一页，没有被过期结果覆盖。
        let current = session.current_page().expect("重排后有当前页");
        assert_eq!(current.layout_id, reflowed.layout_id);
        assert_eq!(current.start_byte, reflowed.start_byte);
        drop(guard);
    }

    #[test]
    fn pagination_reflow_keeps_position_and_invalidates_old_pages() {
        let content = novel_bytes(60);
        let (guard, mut session) = session_for(&content, "reflow");
        let mut forward = Vec::new();
        match session.goto(0) {
            PageOutcome::Ready(page) => forward.push(page),
            other => panic!("首页必须可读：{other:?}"),
        }
        for _ in 1..5 {
            match session.next_page() {
                PageOutcome::Ready(page) => forward.push(page),
                other => panic!("翻页失败：{other:?}"),
            }
        }
        let old_layout = session.layout_id();
        let position = forward.last().expect("有当前页").start_byte;
        assert!(session.boundaries().len() >= 5);
        assert!(!session.cached_starts().is_empty());

        // 字号变化 → 新签名 → 旧边界与旧页面缓存整批失效，并从当前位置重新分页。
        let settings = TypographySettings {
            font_size_milli: 26_000,
            ..Default::default()
        };
        let plan = crate::reflow::plan_reflow(
            Some(session.signature()),
            &MappedViewport::new(1_080, 720),
            &LayoutMetrics::DESKTOP,
            &settings,
            font(),
            false,
            position,
        )
        .expect("重排计划");
        assert!(plan.changed);
        let outcome = session
            .reflow(plan.signature.clone(), font())
            .expect("重排");
        assert!(outcome.changed);
        assert_ne!(outcome.layout_id, old_layout);
        assert_eq!(outcome.start_byte, position, "重排必须保留内容位置");
        let page = outcome.outcome.page().expect("重排后立即可读");
        assert_eq!(page.start_byte, position, "新页面必须从当前位置开始");
        assert_eq!(page.layout_id, outcome.layout_id);
        assert_eq!(session.signature(), &plan.signature);
        assert_eq!(session.typography().font_size_milli, 26_000);
        assert!(
            session
                .boundaries()
                .iter()
                .all(|boundary| boundary.start_byte >= position),
            "旧边界必须全部失效（不得残留旧世代边界）"
        );
        assert!(
            session
                .cached_starts()
                .iter()
                .all(|start| session.cached_starts().contains(&page.start_byte)
                    || *start >= position),
            "旧页面缓存必须整批丢弃"
        );

        // 同签名重复重排是幂等的：不换世代、不清缓存。
        let again = session.reflow(plan.signature, font()).expect("幂等重排");
        assert!(!again.changed, "签名未变化时不得清空分页缓存");
        assert_eq!(again.layout_id, outcome.layout_id);
        drop(guard);
    }

    #[test]
    fn pagination_prefetch_fills_bounded_window_without_duplicates() {
        let content = novel_bytes(80);
        let (guard, mut session) = session_for(&content, "prefetch");
        assert!(session.goto(0).is_ready());
        let added = session.prefetch(3);
        assert_eq!(added, 3, "预读后三页（首页没有前一页）");
        let stats = session.cache_stats();
        assert_eq!(stats.inserts, 4, "当前页 + 后三页");
        assert!(stats.len <= stats.capacity, "缓存必须保持有界");

        // 同一页重复预读：不得重复测量/重复填充。
        let before_inserts = session.cache_stats().inserts;
        let before_measures = session.counters().0;
        assert_eq!(session.prefetch(3), 0, "已缓存的页不得重复填充");
        assert_eq!(session.cache_stats().inserts, before_inserts);
        assert_eq!(session.counters().0, before_measures);

        // 翻到第二页后预读：只补缺失页，且缓存保持有界。
        assert!(session.next_page().is_ready());
        let second = session.current_page().expect("第 2 页");
        let added = session.prefetch(3);
        assert!(added <= 3, "一次预读最多补齐 3 页：{added}");
        let stats = session.cache_stats();
        assert!(stats.len <= stats.capacity, "缓存必须保持有界");
        assert!(
            session.cached_starts().contains(&second.start_byte),
            "当前页必须留在缓存里"
        );
        drop(guard);
    }

    #[test]
    fn pagination_page_fills_content_height_with_explicit_line_pitch() {
        let (guard, session) = session_for(b"x\n".repeat(10).as_slice(), "fill");
        let typography = session.typography();
        assert!(typography.line_pitch_px().saturating_mul(1_000) >= typography.line_pitch_milli);
        // 渲染端逐行定位：行数 × 行盒 ≤ 正文区高度，且再加一行会溢出。
        let used = typography
            .lines_per_page()
            .saturating_mul(typography.line_pitch_milli);
        assert!(used <= typography.content_height_milli);
        assert!(used + typography.line_pitch_milli > typography.content_height_milli);
        drop(guard);
    }
}

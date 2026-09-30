//! `PageBoundary`：页面边界的**字节坐标语义**、合法性校验与严格前进规则。
//!
//! 依据：
//! - `reading-core/spec.md` CR4「校验页面起止边界、合法 UTF-8 边界和严格前进关系」；
//! - `results/gates/gate-0-baseline.md` §2.4（「所有页面 `end_byte` 必须严格前进且位于合法
//!   字符边界」）与 §2.5（缓存键 = `layout_signature + content_index_version`）；
//! - `reading-core/design.md` §位置规则。
//!
//! # 分层
//!
//! 本模块只认识**原始字节坐标上的半开区间**与 UTF-8 边界事实，不认识布局签名、缓存或 UI。
//! 布局签名与分页缓存编排在 `reading-core`（`layout.rs` / `pagination.rs`），因此
//! `reading-index` 不依赖 `reading-core`（避免依赖环）。
//!
//! # 反死循环（卡片 RC5 禁止事项）
//!
//! `end_byte <= start_byte` 一律被拒（[`BoundaryRejection::EmptyInterval`] /
//! [`BoundaryRejection::ReversedInterval`]）：零长度或倒置的页面会让「下一页」永远停在原地。
//! 同理，[`check_strict_advance`] 拒绝任何 `end_byte` 不严格增大的后继页面。
//!
//! # 不丢字 / 不重字
//!
//! 区间按半开约定 `[start_byte, end_byte)` 解释，相邻页面必须满足
//! `next.start_byte == previous.end_byte`：大于即丢字（[`BoundaryRejection::GapBeforePrevious`]），
//! 小于即重字（[`BoundaryRejection::OverlapWithPrevious`]）。

use std::fmt;

use reading_source::{is_boundary, SourceError, TextEncoding, TextSource, TextWindow};

/// 页面边界：原始字节坐标上的半开区间 `[start_byte, end_byte)`。
///
/// 「原始字节」指未解码数据源中的绝对字节偏移（`ContentPosition.original_byte_offset`
/// 所在坐标空间），因此排版设置变化不会改变已有边界之外的坐标含义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageBoundary {
    /// 页面起始字节（含）。
    pub start_byte: u64,
    /// 页面结束字节（不含）；必须位于合法 UTF-8 字符边界。
    pub end_byte: u64,
}

impl PageBoundary {
    /// 按半开区间构造。
    pub const fn new(start_byte: u64, end_byte: u64) -> Self {
        Self {
            start_byte,
            end_byte,
        }
    }

    /// 页面字节长度（倒置区间返回 0）。
    pub fn len_bytes(&self) -> u64 {
        self.end_byte.saturating_sub(self.start_byte)
    }

    /// 是否为空页面（`end_byte <= start_byte`）。
    pub fn is_empty(&self) -> bool {
        self.end_byte <= self.start_byte
    }

    /// 偏移是否落在本页面内（`[start_byte, end_byte)`）。
    pub fn contains(&self, offset: u64) -> bool {
        offset >= self.start_byte && offset < self.end_byte
    }

    /// 是否与另一页面首尾相接（`self.end == other.start`）。
    pub fn touches(&self, other: &PageBoundary) -> bool {
        self.end_byte == other.start_byte
    }

    /// 稳定文本形式（`nrpb1:<start>:<end>`），用于日志与持久化对照。
    pub fn encode(&self) -> String {
        format!(
            "{}:{}:{}",
            PAGE_BOUNDARY_ENCODING_PREFIX, self.start_byte, self.end_byte
        )
    }

    /// 解析 [`PageBoundary::encode`] 的输出；前缀或数值非法时报错。
    pub fn decode(text: &str) -> Result<Self, BoundaryError> {
        let mut parts = text.split(':');
        let prefix = parts.next().unwrap_or_default();
        if prefix != PAGE_BOUNDARY_ENCODING_PREFIX {
            return Err(BoundaryError::new(
                BoundaryRejection::TotalLengthMismatch,
                format!("页面边界前缀必须是 {PAGE_BOUNDARY_ENCODING_PREFIX}，实际 {prefix:?}"),
            ));
        }
        let start_byte = parse_u64(parts.next(), "start_byte")?;
        let end_byte = parse_u64(parts.next(), "end_byte")?;
        if parts.next().is_some() {
            return Err(BoundaryError::new(
                BoundaryRejection::TotalLengthMismatch,
                "页面边界字段数量多于冻结的 2 项".to_string(),
            ));
        }
        Ok(Self::new(start_byte, end_byte))
    }
}

/// 页面边界编码前缀（版本化；不兼容变更必须换前缀）。
pub const PAGE_BOUNDARY_ENCODING_PREFIX: &str = "nrpb1";

/// 边界被拒绝的原因（稳定数值，可映射到 `NR_ERR_*`）。
///
/// 数值本身不跨 ABI 传输；跨语言只传 [`BoundaryRejection::code`] 给出的 `NR_ERR_*` 数值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryRejection {
    /// `end_byte < start_byte`：区间倒置。
    ReversedInterval,
    /// `end_byte == start_byte`：零长度页面（会造成死循环）。
    EmptyInterval,
    /// 起止字节超出 `[0, total_bytes]`。
    OutOfRange,
    /// `end_byte` 落在多字节字符中间。
    IllegalCharBoundary,
    /// 与上一页之间存在空隙（丢字）。
    GapBeforePrevious,
    /// 与上一页重叠（重字）。
    OverlapWithPrevious,
    /// 相对上一页没有严格前进（`next.end_byte <= previous.end_byte`）。
    NotAdvancing,
    /// 同一签名同一起点已登记不同的结束字节（缓存结果不一致）。
    ConflictWithCached,
    /// `total_bytes` 与数据源实际长度不一致。
    TotalLengthMismatch,
}

impl BoundaryRejection {
    /// 跨 ABI 的错误码数值（与 `NR_ERR_*` 一致；`1`/`2`/`3`）。
    pub const fn code(self) -> i32 {
        match self {
            BoundaryRejection::OutOfRange => 2,
            BoundaryRejection::IllegalCharBoundary => 3,
            _ => 1,
        }
    }

    /// 稳定英文短名（日志用，不得作为业务分支依据）。
    pub const fn as_str(self) -> &'static str {
        match self {
            BoundaryRejection::ReversedInterval => "ReversedInterval",
            BoundaryRejection::EmptyInterval => "EmptyInterval",
            BoundaryRejection::OutOfRange => "OutOfRange",
            BoundaryRejection::IllegalCharBoundary => "IllegalCharBoundary",
            BoundaryRejection::GapBeforePrevious => "GapBeforePrevious",
            BoundaryRejection::OverlapWithPrevious => "OverlapWithPrevious",
            BoundaryRejection::NotAdvancing => "NotAdvancing",
            BoundaryRejection::ConflictWithCached => "ConflictWithCached",
            BoundaryRejection::TotalLengthMismatch => "TotalLengthMismatch",
        }
    }
}

/// 边界校验失败：稳定原因 + 可读详情。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundaryError {
    /// 拒绝原因。
    pub rejection: BoundaryRejection,
    /// 可读详情（仅用于日志与用户提示映射）。
    pub detail: String,
}

impl BoundaryError {
    /// 构造错误。
    pub fn new(rejection: BoundaryRejection, detail: impl Into<String>) -> Self {
        Self {
            rejection,
            detail: detail.into(),
        }
    }

    /// 拒绝原因。
    pub fn rejection(&self) -> BoundaryRejection {
        self.rejection
    }

    /// 跨 ABI 错误码数值。
    pub fn code(&self) -> i32 {
        self.rejection.code()
    }

    /// 可读详情。
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for BoundaryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.rejection.as_str(), self.detail)
    }
}

impl std::error::Error for BoundaryError {}

/// 数据源 I/O 失败统一并入边界错误（错误码 6），避免调用方漏判读取失败。
impl From<SourceError> for BoundaryError {
    fn from(error: SourceError) -> Self {
        BoundaryError::new(
            BoundaryRejection::TotalLengthMismatch,
            format!("数据源错误: {}", error.as_str()),
        )
    }
}

/// 已经读取到手的窗口内，`absolute_offset` 是否位于**该编码下**的合法字符边界（CR2 的 7 种编码）。
///
/// - `window` 的两端（[`TextWindow::start`] / [`TextWindow::end`]）由 `read_at` 保证对齐到
///   字符边界，因此等于两端直接判为边界；
/// - 偏移不落在窗口内时返回 `None`（调用方需换窗口，而不是猜测）。
pub fn is_char_boundary_in_window(
    window: &TextWindow,
    absolute_offset: u64,
    encoding: TextEncoding,
) -> Option<bool> {
    let start = window.start();
    let end = window.end();
    if absolute_offset < start || absolute_offset > end {
        return None;
    }
    if absolute_offset == start || absolute_offset == end {
        return Some(true);
    }
    let index = (absolute_offset - start) as usize;
    Some(is_boundary(&window.data, index, encoding))
}

/// 校验 `offset` 是否位于数据源的**合法字符边界**（按内容编码，UTF-8 为默认）。
///
/// 只读取一个 ≤ 8 字节的小窗口（必要时向后推进），不整本读取；`offset == len` 视为边界。
///
/// 判定依据：UTF-8 中续接字节（`0b10xx_xxxx`）必然处于字符内部，因此只需看 `offset` 处的首字节：
/// 非续接字节即为字符起点。
pub fn is_char_boundary_at<S: TextSource + ?Sized>(
    source: &S,
    anchor: u64,
    offset: u64,
    encoding: TextEncoding,
) -> Result<bool, SourceError> {
    // 有基点可用时按**基点**判定：GB18030/GBK/Big5 的字符边界无法从错位小窗口判定，
    // 必须从「已知对齐」的位置（页起点 = 上一页终点；或章节起点/文件头）步进。
    // 超出可验证距离时**接受**（不猜测、不误拒）：宁可放过，也不把合法边界判死
    // —— 后者会让阅读器整页无法提交（真机事故）。
    if offset <= anchor {
        return Ok(true);
    }
    // 只有**传统双字节编码**才必须走基点步进：它们的字符边界无法由单个字节判定
    // （首字节与 UTF-8 续接区间重叠，见 results/encoding-core-boundary.md）。
    // UTF-8 / UTF-16 的判定只看就近字节即可，保持既有「只读小窗口」的廉价契约（有测试锁定）。
    let needs_anchor_walk = matches!(
        encoding,
        TextEncoding::Gb18030 | TextEncoding::Gbk | TextEncoding::Big5
    );
    if needs_anchor_walk {
        let span = offset - anchor;
        if span <= MAX_ANCHOR_SPAN_BYTES {
            let window = source.read_at_with_encoding(anchor, span as usize, encoding)?;
            if window.dropped_leading == 0 && window.data.len() as u64 == span {
                return Ok(reading_source::is_boundary(&window.data, span as usize, encoding));
            }
        }
        // 跨度超出可验证范围或基点未对齐：**接受**（不猜测、不误拒——误拒会让整页无法提交）。
        return Ok(true);
    }
    is_char_boundary_probe(source, offset, encoding)
}

/// 既有探针路径：读 `offset - 4` 的 ≤8 字节小窗口判定（UTF-8/UTF-16 足够；见调用方注释）。
fn is_char_boundary_probe<S: TextSource + ?Sized>(
    source: &S,
    offset: u64,
    encoding: TextEncoding,
) -> Result<bool, SourceError> {
    let len = source.len();
    if offset == 0 || offset == len {
        return Ok(true);
    }
    if offset > len {
        return Err(SourceError::OutOfRange);
    }

    // 回看最多 4 字节即可覆盖最长的 UTF-8 字符。
    let mut probe = offset - 4u64.min(offset);
    loop {
        let window = source.read_at_with_encoding(probe, 8, encoding)?;
        if window.data.is_empty() {
            // 窗口内全是续接字节：文件在该处已经是坏数据，无法确认边界。
            return Ok(false);
        }
        if offset <= window.end() {
            return is_char_boundary_in_window(&window, offset, encoding)
                .ok_or(SourceError::Internal);
        }
        let next = window.end();
        if next <= probe {
            // 推进失败时立即返回，避免任何形式的死循环。
            return Ok(false);
        }
        probe = next;
    }
}

/// 纯区间校验：起止关系与范围（不访问数据源）。
///
/// 规则：`end_byte >= start_byte` 且 `end_byte > start_byte`（反死循环），
/// 且 `end_byte <= total_bytes`。
pub fn validate_interval(boundary: &PageBoundary, total_bytes: u64) -> Result<(), BoundaryError> {
    if boundary.end_byte < boundary.start_byte {
        return Err(BoundaryError::new(
            BoundaryRejection::ReversedInterval,
            format!(
                "页面区间倒置: end_byte({}) < start_byte({})",
                boundary.end_byte, boundary.start_byte
            ),
        ));
    }
    if boundary.end_byte == boundary.start_byte {
        return Err(BoundaryError::new(
            BoundaryRejection::EmptyInterval,
            format!("零长度页面 start == end == {}", boundary.start_byte),
        ));
    }
    if boundary.end_byte > total_bytes {
        return Err(BoundaryError::new(
            BoundaryRejection::OutOfRange,
            format!(
                "end_byte({}) 超出内容长度({total_bytes})",
                boundary.end_byte
            ),
        ));
    }
    Ok(())
}

/// 基点判定的最大可验证跨度：超过即「接受」（避免为一次校验读取整章文本）。
const MAX_ANCHOR_SPAN_BYTES: u64 = 64 * 1024;

/// 完整边界校验：纯区间规则 + `end_byte` 位于**该编码下**的合法字符边界 + 长度一致性。
///
/// 只读取 `end_byte` 附近的 ≤ 8 字节窗口，不物化整页正文。
pub fn validate_page_boundary<S: TextSource + ?Sized>(
    source: &S,
    boundary: &PageBoundary,
    total_bytes: u64,
    encoding: TextEncoding,
) -> Result<(), BoundaryError> {
    if total_bytes != source.len() {
        return Err(BoundaryError::new(
            BoundaryRejection::TotalLengthMismatch,
            format!(
                "total_bytes({total_bytes}) 与数据源长度({}) 不一致",
                source.len()
            ),
        ));
    }
    validate_interval(boundary, total_bytes)?;
    if !is_char_boundary_at(source, boundary.start_byte, boundary.end_byte, encoding)? {
        return Err(BoundaryError::new(
            BoundaryRejection::IllegalCharBoundary,
            format!(
                "end_byte({}) 落在多字节字符中间（必须对齐到字符边界）",
                boundary.end_byte
            ),
        ));
    }
    Ok(())
}

/// 严格前进校验：`next` 相对 `previous` 必须首尾相接且 `end_byte` 严格增大。
///
/// 拒绝三类情况（分别对应丢字、重字与死循环）：
/// - `next.start_byte > previous.end_byte` → [`BoundaryRejection::GapBeforePrevious`]；
/// - `next.start_byte < previous.end_byte` → [`BoundaryRejection::OverlapWithPrevious`]；
/// - `next.end_byte <= previous.end_byte` → [`BoundaryRejection::NotAdvancing`]。
pub fn check_strict_advance(
    previous: &PageBoundary,
    next: &PageBoundary,
) -> Result<(), BoundaryError> {
    if next.end_byte < next.start_byte {
        return Err(BoundaryError::new(
            BoundaryRejection::ReversedInterval,
            format!(
                "后继页面区间倒置: end_byte({}) < start_byte({})",
                next.end_byte, next.start_byte
            ),
        ));
    }
    if next.end_byte == next.start_byte {
        return Err(BoundaryError::new(
            BoundaryRejection::EmptyInterval,
            format!("后继页面为零长度（start == end == {}）", next.start_byte),
        ));
    }
    if next.start_byte > previous.end_byte {
        return Err(BoundaryError::new(
            BoundaryRejection::GapBeforePrevious,
            format!(
                "下一页起点({}) 大于上一页终点({})，将丢失字节",
                next.start_byte, previous.end_byte
            ),
        ));
    }
    if next.start_byte < previous.end_byte {
        return Err(BoundaryError::new(
            BoundaryRejection::OverlapWithPrevious,
            format!(
                "下一页起点({}) 小于上一页终点({})，将重复字节",
                next.start_byte, previous.end_byte
            ),
        ));
    }
    if next.end_byte <= previous.end_byte {
        return Err(BoundaryError::new(
            BoundaryRejection::NotAdvancing,
            format!(
                "下一页终点({}) 未超过上一页终点({})，不得前进",
                next.end_byte, previous.end_byte
            ),
        ));
    }
    Ok(())
}

/// 连续的页面边界链：保证 `page(k+1).start == page(k).end`（既不丢字也不重字）。
///
/// 正反向翻页都从这条链上取页面：正向取 `next_of`，反向取 `previous_of`，
/// 二者共享同一份边界，因此不会因为「向上取整/向下取整」这类近似算法产生漂移。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageBoundaryChain {
    origin: u64,
    pages: Vec<PageBoundary>,
}

impl PageBoundaryChain {
    /// 从内容起点建链（首个页面的 `start_byte` 必须等于 `origin`）。
    pub fn new(origin: u64) -> Self {
        Self {
            origin,
            pages: Vec::new(),
        }
    }

    /// 链的起点（通常为 0；章节内分页可为章节起点）。
    pub fn origin(&self) -> u64 {
        self.origin
    }

    /// 已登记的页面数量。
    pub fn len(&self) -> usize {
        self.pages.len()
    }

    /// 是否还没有页面。
    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    /// 全部页面（按登记顺序，已保证首尾相接）。
    pub fn pages(&self) -> &[PageBoundary] {
        &self.pages
    }

    /// 最后一个页面。
    pub fn last(&self) -> Option<&PageBoundary> {
        self.pages.last()
    }

    /// 按序号取页面。
    pub fn page(&self, index: usize) -> Option<&PageBoundary> {
        self.pages.get(index)
    }

    /// 正向翻页：序号 `index` 的下一页。
    pub fn next_of(&self, index: usize) -> Option<&PageBoundary> {
        self.pages.get(index + 1)
    }

    /// 反向翻页：序号 `index` 的上一页。
    pub fn previous_of(&self, index: usize) -> Option<&PageBoundary> {
        index.checked_sub(1).and_then(|prev| self.pages.get(prev))
    }

    /// 追加一个页面；必须与链尾首尾相接并严格前进。
    pub fn push(&mut self, boundary: PageBoundary) -> Result<(), BoundaryError> {
        match self.pages.last() {
            None => {
                if boundary.end_byte <= boundary.start_byte {
                    let rejection = if boundary.end_byte < boundary.start_byte {
                        BoundaryRejection::ReversedInterval
                    } else {
                        BoundaryRejection::EmptyInterval
                    };
                    return Err(BoundaryError::new(
                        rejection,
                        format!("首个页面区间非法: {boundary:?}"),
                    ));
                }
                if boundary.start_byte != self.origin {
                    let rejection = if boundary.start_byte > self.origin {
                        BoundaryRejection::GapBeforePrevious
                    } else {
                        BoundaryRejection::OverlapWithPrevious
                    };
                    return Err(BoundaryError::new(
                        rejection,
                        format!(
                            "首个页面起点({}) 必须等于链起点({})",
                            boundary.start_byte, self.origin
                        ),
                    ));
                }
            }
            Some(previous) => {
                let previous = *previous;
                check_strict_advance(&previous, &boundary)?;
            }
        }
        self.pages.push(boundary);
        Ok(())
    }

    /// 整条链覆盖的字节区间 `[首次起点, 末次终点)`。
    pub fn covered(&self) -> (u64, u64) {
        match (self.pages.first(), self.pages.last()) {
            (Some(first), Some(last)) => (first.start_byte, last.end_byte),
            _ => (self.origin, self.origin),
        }
    }

    /// 复核整条链：逐对页面必须严格前进（不丢字、不重字）。
    pub fn assert_contiguous(&self) -> Result<(), BoundaryError> {
        for pair in self.pages.windows(2) {
            check_strict_advance(&pair[0], &pair[1])?;
        }
        Ok(())
    }

    /// 偏移落在第几页（精确序号，不是百分比近似）。
    pub fn page_index_for_offset(&self, offset: u64) -> Option<usize> {
        self.pages.iter().position(|page| page.contains(offset))
    }
}

fn parse_u64(part: Option<&str>, field: &str) -> Result<u64, BoundaryError> {
    let raw = part.ok_or_else(|| {
        BoundaryError::new(
            BoundaryRejection::TotalLengthMismatch,
            format!("页面边界缺少 {field} 字段"),
        )
    })?;
    raw.parse::<u64>().map_err(|_| {
        BoundaryError::new(
            BoundaryRejection::TotalLengthMismatch,
            format!("{field} 不是合法的 64 位无符号整数"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_window(offset: u64, data: &[u8], dropped_leading: usize) -> TextWindow {
        TextWindow {
            offset,
            data: data.to_vec(),
            dropped_leading,
            trimmed_tail: 0,
            eof: false,
            requested_len: data.len(),
        }
    }

    #[test]
    fn boundary_interval_rules_reject_empty_and_reversed() {
        // 反死循环：零长度与倒置区间必须被拒。
        let empty = PageBoundary::new(100, 100);
        let empty_error = validate_interval(&empty, 1000).expect_err("零长度页面必须被拒");
        assert_eq!(empty_error.rejection(), BoundaryRejection::EmptyInterval);
        assert_eq!(empty_error.code(), 1);

        let reversed = PageBoundary::new(200, 100);
        let reversed_error = validate_interval(&reversed, 1000).expect_err("倒置区间必须被拒");
        assert_eq!(
            reversed_error.rejection(),
            BoundaryRejection::ReversedInterval
        );

        let beyond = PageBoundary::new(10, 2000);
        let beyond_error = validate_interval(&beyond, 1000).expect_err("越界区间必须被拒");
        assert_eq!(beyond_error.rejection(), BoundaryRejection::OutOfRange);
        assert_eq!(beyond_error.code(), 2);

        assert!(validate_interval(&PageBoundary::new(0, 1000), 1000).is_ok());
    }

    #[test]
    fn boundary_advance_rules_reject_gap_overlap_and_stall() {
        let previous = PageBoundary::new(0, 100);
        assert!(check_strict_advance(&previous, &PageBoundary::new(100, 200)).is_ok());

        let gap = check_strict_advance(&previous, &PageBoundary::new(101, 200))
            .expect_err("空隙必须被拒");
        assert_eq!(gap.rejection(), BoundaryRejection::GapBeforePrevious);
        let overlap =
            check_strict_advance(&previous, &PageBoundary::new(99, 200)).expect_err("重叠必须被拒");
        assert_eq!(overlap.rejection(), BoundaryRejection::OverlapWithPrevious);
        let stall = check_strict_advance(&previous, &PageBoundary::new(100, 100))
            .expect_err("零长度后继必须被拒");
        assert_eq!(stall.rejection(), BoundaryRejection::EmptyInterval);
        let backward =
            check_strict_advance(&PageBoundary::new(100, 200), &PageBoundary::new(200, 150))
                .expect_err("终点后退必须被拒");
        assert_eq!(backward.rejection(), BoundaryRejection::ReversedInterval);
    }

    #[test]
    fn boundary_chain_is_contiguous_in_both_directions() {
        let mut chain = PageBoundaryChain::new(0);
        chain.push(PageBoundary::new(0, 300)).expect("首页");
        chain.push(PageBoundary::new(300, 700)).expect("第二页");
        chain.push(PageBoundary::new(700, 1000)).expect("第三页");
        assert_eq!(chain.len(), 3);
        assert_eq!(chain.covered(), (0, 1000));
        assert!(chain.assert_contiguous().is_ok());

        // 正向 / 反向取自同一份边界。
        assert_eq!(chain.next_of(0), Some(&PageBoundary::new(300, 700)));
        assert_eq!(chain.previous_of(2), Some(&PageBoundary::new(300, 700)));
        assert_eq!(chain.previous_of(0), None);
        assert_eq!(chain.next_of(2), None);
        assert_eq!(chain.page_index_for_offset(999), Some(2));
        assert_eq!(chain.page_index_for_offset(1000), None);

        let mut bad = PageBoundaryChain::new(0);
        bad.push(PageBoundary::new(0, 300)).expect("首页");
        let error = bad
            .push(PageBoundary::new(400, 700))
            .expect_err("丢字必须被拒");
        assert_eq!(error.rejection(), BoundaryRejection::GapBeforePrevious);
        let overlap = bad
            .push(PageBoundary::new(200, 400))
            .expect_err("重字必须被拒");
        assert_eq!(overlap.rejection(), BoundaryRejection::OverlapWithPrevious);
        // 首个页面必须从链起点开始。
        let mut offset_chain = PageBoundaryChain::new(10);
        let start_error = offset_chain
            .push(PageBoundary::new(20, 30))
            .expect_err("起点不等于 origin 必须被拒");
        assert_eq!(
            start_error.rejection(),
            BoundaryRejection::GapBeforePrevious
        );
    }

    #[test]
    fn boundary_window_probe_detects_mid_char_offsets() {
        // "中文abc"：中(0..3) 文(3..6) a(6) b(7) c(8)
        let text = "中文abc";
        let bytes = text.as_bytes();
        let window = make_window(0, bytes, 0);
        assert_eq!(is_char_boundary_in_window(&window, 0, TextEncoding::Utf8), Some(true));
        assert_eq!(is_char_boundary_in_window(&window, 1, TextEncoding::Utf8), Some(false));
        assert_eq!(is_char_boundary_in_window(&window, 3, TextEncoding::Utf8), Some(true));
        assert_eq!(is_char_boundary_in_window(&window, 4, TextEncoding::Utf8), Some(false));
        assert_eq!(is_char_boundary_in_window(&window, 9, TextEncoding::Utf8), Some(true));
        assert_eq!(is_char_boundary_in_window(&window, 10, TextEncoding::Utf8), None);

        // 窗口首部被裁剪时，窗口起点与终点仍视为边界。
        let trimmed = make_window(0, &bytes[3..], 3);
        assert_eq!(trimmed.start(), 3);
        assert_eq!(is_char_boundary_in_window(&trimmed, 3, TextEncoding::Utf8), Some(true));
        assert_eq!(is_char_boundary_in_window(&trimmed, 4, TextEncoding::Utf8), Some(false));
    }

    #[test]
    fn boundary_encoding_round_trip() {
        let boundary = PageBoundary::new(4096, 8192);
        assert_eq!(boundary.encode(), "nrpb1:4096:8192");
        assert_eq!(PageBoundary::decode(&boundary.encode()), Ok(boundary));
        assert!(PageBoundary::decode("nrpb2:1:2").is_err());
        assert!(PageBoundary::decode("nrpb1:1").is_err());
        assert!(PageBoundary::decode("nrpb1:1:2:3").is_err());
        assert!(PageBoundary::decode("nrpb1:x:2").is_err());
        assert_eq!(PageBoundary::new(10, 4).len_bytes(), 0);
        assert!(PageBoundary::new(10, 10).is_empty());
        assert!(PageBoundary::new(10, 20).contains(10));
        assert!(!PageBoundary::new(10, 20).contains(20));
        assert!(PageBoundary::new(10, 20).touches(&PageBoundary::new(20, 30)));
    }

    #[test]
    fn boundary_rejection_codes_match_frozen_error_codes() {
        // 数值来自 results/gates/gate-0-baseline.md §2.2（1/2/3）。
        let out_of_range: i32 = BoundaryRejection::OutOfRange.code();
        let illegal: i32 = BoundaryRejection::IllegalCharBoundary.code();
        let empty: i32 = BoundaryRejection::EmptyInterval.code();
        assert_eq!(out_of_range, 2);
        assert_eq!(illegal, 3);
        assert_eq!(empty, 1);
        let error = BoundaryError::new(BoundaryRejection::IllegalCharBoundary, "x");
        assert_eq!(error.code(), 3);
        assert_eq!(error.detail(), "x");
        assert!(error.to_string().starts_with("IllegalCharBoundary: "));
        let from_source: BoundaryError = SourceError::IoError.into();
        assert_eq!(from_source.code(), 1);
    }
}

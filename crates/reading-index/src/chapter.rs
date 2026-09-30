//! 章节规则、增量扫描与「虚拟全文章节」回退。
//!
//! 依据 `docs/novel-reader-v1-development-plan.md` §7.2（章节规则与章节记录）与
//! `reading-core/spec.md` CR3。
//!
//! # 契约
//!
//! - 章节是**内容单元中的字节区间**（`[start_byte, end_byte)`），不把章节编号当作唯一恢复位置；
//! - 识别不到任何章节时，必须生成单一「全文」虚拟章节（`index = 0`，覆盖 `[0, len)`），
//!   保证文件仍然可读；
//! - 扫描**可增量、可取消**：调用方按窗口推进（[`ChapterScanSession::step`]），
//!   或自带取消回调（[`scan_chapters`]）；单步读取量不超过数据源的默认窗口；
//! - 扫描**不保存正文**：只保留标题行（≤ [`MAX_TITLE_LINE_BYTES`]）与区间元数据，
//!   未完成行缓冲上限为 [`MAX_LINE_BYTES`]，超长行会被截断并计数。
//!
//! # 规则集（V1）
//!
//! | 规则 | 例子 |
//! |---|---|
//! | [`ChapterRule::ChapterNumber`] | `第一章`、`第1章`、`第001章`、`第 12 章`、`第十二章 初见` |
//! | [`ChapterRule::VolumeNumber`] | `第一卷`、`卷一` |
//! | [`ChapterRule::Keyword`] | `楔子`、`序章`、`前言`、`后记`、`尾声` |
//! | [`ChapterRule::English`] | `Chapter 1`、`chapter12` |
//! | [`ChapterRule::StandaloneNumeric`] | 独立的 `1` / `12` 行（需满足长度、上下文、相邻章节条件） |
//!
//! 章节识别是启发式结果，本模块只做“可测试且诚实”的判断：宁可漏识别（回退到虚拟章节），
//! 也不把正文句子误当标题。

use reading_source::{decode_line, SourceError, TextEncoding, TextSource};

/// 章节索引版本（章节规则或区间语义变化时必须递增，用于索引失效）。
pub const CHAPTER_INDEX_VERSION: u32 = 1;

/// 标题行最大字节数（更长的行一律不是标题）。
pub const MAX_TITLE_LINE_BYTES: usize = 256;

/// 标题行最大字符数（中文标题不会超过这个长度）。
pub const MAX_TITLE_LINE_CHARS: usize = 64;

/// 未完成行的缓冲上限（超长行截断，避免长期占用内存或把整章读进内存）。
pub const MAX_LINE_BYTES: usize = 8 * 1024;

/// 标题尾部最大字符数（无分隔符时，`第一章初见` 这类紧凑写法仍可接受，但不能整句正文）。
pub const MAX_TITLE_TAIL_CHARS: usize = 20;

/// 独立数字标题的最大位数。
pub const MAX_STANDALONE_NUMERIC_DIGITS: usize = 4;

/// 独立数字标题被采纳所需的最少出现次数（「相邻章节条件」的一部分）。
pub const MIN_STANDALONE_NUMERIC_SECTIONS: usize = 2;

/// 虚拟全文章节的标题。
pub const VIRTUAL_CHAPTER_TITLE: &str = "全文";

/// 章节扫描器版本：**扫描规则或行解码行为变更时必须递增**。
///
/// 索引缓存的键只有「内容索引版本」（存储侧计数器），不含代码行为版本 ⇒ 升级应用后
/// 既有书籍会继续复用旧索引（真机现象：更新应用无效、须卸载重装才重新扫描）。
/// 引擎在打开时比对本常量与本地存储的副本，不一致即**强制重扫**。
///
/// - v1：仅 UTF-8 行解码、无编码感知；
/// - v2：按内容编码解码每行（GBK/GB18030/Big5/UTF-16 文档可扫描出真实章节）；
/// - v3：章节单位新增 `集/节/回`；排除以句末标点结尾的正文句子（「第二章到！」）。
pub const CHAPTER_SCANNER_VERSION: i64 = 3;

/// 章节规则标识（稳定数值，可跨 ABI / 落库；规则语义变化必须递增 [`CHAPTER_INDEX_VERSION`]）。
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ChapterRule {
    /// 虚拟全文章节（无任何章节命中时的回退）。
    VirtualFullText = 0,
    /// `第…章`。
    ChapterNumber = 1,
    /// `第…卷` / `卷…`。
    VolumeNumber = 2,
    /// `楔子` / `序章` / `前言` / `后记` / `尾声`。
    Keyword = 3,
    /// `Chapter N`。
    English = 4,
    /// 独立数字标题。
    StandaloneNumeric = 5,
}

impl ChapterRule {
    /// 稳定数值。
    pub const fn id(self) -> u32 {
        self as u32
    }

    /// 稳定短名（日志/证据用，不得作为业务分支依据）。
    pub const fn as_str(self) -> &'static str {
        match self {
            ChapterRule::VirtualFullText => "VirtualFullText",
            ChapterRule::ChapterNumber => "ChapterNumber",
            ChapterRule::VolumeNumber => "VolumeNumber",
            ChapterRule::Keyword => "Keyword",
            ChapterRule::English => "English",
            ChapterRule::StandaloneNumeric => "StandaloneNumeric",
        }
    }

    /// 由稳定数值还原规则。
    pub const fn from_id(id: u32) -> Option<Self> {
        match id {
            0 => Some(ChapterRule::VirtualFullText),
            1 => Some(ChapterRule::ChapterNumber),
            2 => Some(ChapterRule::VolumeNumber),
            3 => Some(ChapterRule::Keyword),
            4 => Some(ChapterRule::English),
            5 => Some(ChapterRule::StandaloneNumeric),
            _ => None,
        }
    }

    /// 是否是需要「相邻章节条件」的弱规则。
    pub const fn is_weak(self) -> bool {
        matches!(self, ChapterRule::StandaloneNumeric)
    }
}

/// 单行标题识别结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterMatch {
    /// 命中的规则。
    pub rule: ChapterRule,
    /// 解析出的章节序号（`楔子` 等无序号标题为 `None`）。
    pub number: Option<u64>,
    /// 原始标题（已 trim）。
    pub title: String,
    /// 归一化标题（去空白、全角转半角、ASCII 小写）。
    pub normalized_title: String,
}

/// 章节区间记录（对应开发方案 §7.2 的章节记录字段）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterRange {
    /// 章节序号（从 0 起，连续）。
    pub index: u32,
    /// 原始标题。
    pub title: String,
    /// 归一化标题（查找/去重用）。
    pub normalized_title: String,
    /// 章节起始字节（标题行行首）。
    pub start_byte: u64,
    /// 章节结束字节（下一章节起始，或文件末尾；不含）。
    pub end_byte: u64,
    /// 命中的规则。
    pub rule: ChapterRule,
    /// 章节序号（解析所得，可能为 `None`）。
    pub number: Option<u64>,
}

impl ChapterRange {
    /// 章节字节长度。
    pub fn len_bytes(&self) -> u64 {
        self.end_byte.saturating_sub(self.start_byte)
    }

    /// 偏移是否落在本区间内（`[start_byte, end_byte)`）。
    pub fn contains(&self, offset: u64) -> bool {
        offset >= self.start_byte && offset < self.end_byte
    }
}

/// 章节索引：按字节偏移排序、彼此不重叠、覆盖 `[0, total_bytes)`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterIndex {
    /// 章节区间（index 与下标一致）。
    pub chapters: Vec<ChapterRange>,
    /// 是否为「无章节命中」的虚拟全文章节。
    pub is_virtual: bool,
    /// 章节索引版本。
    pub index_version: u32,
    /// 文件总字节数。
    pub total_bytes: u64,
    /// 扫描过的字节数。
    pub scanned_bytes: u64,
    /// 扫描过的行数。
    pub lines_scanned: u64,
    /// 无法按 UTF-8 解码的行数（例如尚未解码的传统编码）。
    pub undecodable_lines: u64,
    /// 被截断的超长行数。
    pub truncated_lines: u64,
    /// 独立数字标题候选数（用于解释为何采纳/丢弃）。
    pub numeric_candidates: u32,
    /// 本次扫描是否被取消（取消时结果是部分结果）。
    pub cancelled: bool,
}

impl ChapterIndex {
    /// 生成单一「全文」虚拟章节（无章节命中时的强制回退）。
    pub fn virtual_full_text(total_bytes: u64) -> Self {
        Self {
            chapters: vec![ChapterRange {
                index: 0,
                title: VIRTUAL_CHAPTER_TITLE.to_string(),
                normalized_title: normalize_title(VIRTUAL_CHAPTER_TITLE),
                start_byte: 0,
                end_byte: total_bytes,
                rule: ChapterRule::VirtualFullText,
                number: None,
            }],
            is_virtual: true,
            index_version: CHAPTER_INDEX_VERSION,
            total_bytes,
            scanned_bytes: 0,
            lines_scanned: 0,
            undecodable_lines: 0,
            truncated_lines: 0,
            numeric_candidates: 0,
            cancelled: false,
        }
    }

    /// 章节数量。
    pub fn len(&self) -> usize {
        self.chapters.len()
    }

    /// 是否没有任何章节（正常构造下恒为 false：至少有一个虚拟章节）。
    pub fn is_empty(&self) -> bool {
        self.chapters.is_empty()
    }

    /// 按序号取章节。
    pub fn chapter(&self, index: u32) -> Option<&ChapterRange> {
        self.chapters.get(index as usize)
    }

    /// 章节起始字节。
    pub fn chapter_start(&self, index: u32) -> Option<u64> {
        self.chapter(index).map(|chapter| chapter.start_byte)
    }

    /// 偏移落在哪个章节；越界偏移夹紧到最后一个章节（不在索引内时返回 `None`）。
    pub fn chapter_for_offset(&self, offset: u64) -> Option<u32> {
        if self.chapters.is_empty() {
            return None;
        }
        let found = self.chapters.binary_search_by(|candidate| {
            if offset < candidate.start_byte {
                std::cmp::Ordering::Greater
            } else if offset >= candidate.end_byte {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        });
        match found {
            Ok(index) => Some(index as u32),
            // 只有“超出文件末尾”会走到这里（区间本身连续覆盖 [0, total)）：夹紧到最后一章。
            Err(_) => Some((self.chapters.len() - 1) as u32),
        }
    }
}

/// 独立数字标题候选（延迟决策，用于施加「相邻章节条件」）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct NumericCandidate {
    start_byte: u64,
    title: String,
    normalized_title: String,
    number: u64,
    prev_blank: bool,
    next_blank: bool,
}

/// 增量章节扫描器：逐段喂入**原始文件字节**，按行识别章节。
///
/// 调用方负责按有限窗口读取数据源并把窗口字节按真实偏移喂进来；本类型自己不持有
/// 数据源，因此可以自由地被上层调度（前台让路、后台低优先级、可取消）。
#[derive(Debug)]
pub struct ChapterScanner {
    chapters: Vec<ChapterRange>,
    numeric_candidates: Vec<NumericCandidate>,
    pending_numeric: Option<NumericCandidate>,
    carry: Vec<u8>,
    line_start: u64,
    next_offset: u64,
    line_buffer_full: bool,
    last_line_blank: bool,
    line_index: u64,
    scanned_bytes: u64,
    undecodable_lines: u64,
    /// 内容编码：解码每行后再匹配章节规则（非 UTF-8 文档必需）。
    encoding: TextEncoding,
    truncated_lines: u64,
    cancelled: bool,
}

impl Default for ChapterScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl ChapterScanner {
    /// 新建扫描器。
    pub fn new() -> Self {
        Self {
            chapters: Vec::new(),
            numeric_candidates: Vec::new(),
            pending_numeric: None,
            carry: Vec::new(),
            line_start: 0,
            next_offset: 0,
            line_buffer_full: false,
            // 文件开头视为“空行上下文”，与「前一行是空行」等价。
            last_line_blank: true,
            line_index: 0,
            scanned_bytes: 0,
            undecodable_lines: 0,
        encoding: TextEncoding::Utf8,
            truncated_lines: 0,
            cancelled: false,
        }
    }

    /// 请求取消：扫描器会停止处理后续输入，结果标记为部分结果。
    pub fn cancel(&mut self) {
        self.cancelled = true;
    }

    /// 是否已取消。
    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    /// 当前未完成行的已缓冲字节数（用于断言「不长期保存正文」）。
    pub fn buffered_line_bytes(&self) -> usize {
        self.carry.len()
    }

    /// 已处理的字节数。
    pub fn scanned_bytes(&self) -> u64 {
        self.scanned_bytes
    }

    /// 已识别的章节数（可能后续被合并/重排）。
    pub fn accepted_chapters(&self) -> usize {
        self.chapters.len()
    }

    /// 喂入一段连续文件字节。`start_offset` 是该段在文件中的真实起始偏移。
    ///
    /// 若与上一段不连续（底层窗口裁剪造成缝隙），当前未完成行会被当作行尾处理，
    /// 缝隙字节不会被计入任何标题——这是保守且可解释的行为。
    pub fn push_bytes(&mut self, start_offset: u64, bytes: &[u8]) {
        if self.cancelled || bytes.is_empty() {
            return;
        }
        if start_offset != self.next_offset {
            self.finish_current_line();
        }
        let base = start_offset;
        for (index, byte) in bytes.iter().enumerate() {
            if self.line_buffer_full {
                if *byte == b'\n' {
                    self.truncated_lines += 1;
                    self.line_index += 1;
                    self.last_line_blank = false;
                    self.pending_numeric = None;
                    self.line_buffer_full = false;
                    self.carry.clear();
                    self.line_start = base + index as u64 + 1;
                }
                continue;
            }
            if *byte == b'\n' {
                let line = std::mem::take(&mut self.carry);
                self.process_line(&line, self.line_start, self.encoding);
                self.line_start = base + index as u64 + 1;
                continue;
            }
            self.carry.push(*byte);
            if self.carry.len() >= MAX_LINE_BYTES {
                self.line_buffer_full = true;
            }
        }
        self.next_offset = base + bytes.len() as u64;
        self.scanned_bytes += bytes.len() as u64;
    }

    /// 结束扫描并生成章节索引（`total_bytes` 为文件总长度）。
    pub fn finish(mut self, total_bytes: u64) -> ChapterIndex {
        if self.line_buffer_full {
            self.truncated_lines += 1;
            self.pending_numeric = None;
        } else {
            self.finish_current_line();
        }

        // 结算最后一行上的独立数字标题候选：文件末尾等价于「后一行为空行」。
        if let Some(mut pending) = self.pending_numeric.take() {
            pending.next_blank = true;
            self.numeric_candidates.push(pending);
        }

        let mut chapters = self.chapters;
        let numeric_candidates = self.numeric_candidates.len() as u32;

        // 独立数字标题：长度（规则内保证）+ 上下文（前后空行）+ 相邻章节条件
        // （出现次数 ≥ MIN_STANDALONE_NUMERIC_SECTIONS 且序号严格递增）。
        if self.numeric_candidates.len() >= MIN_STANDALONE_NUMERIC_SECTIONS {
            let mut last_number = 0u64;
            for candidate in &self.numeric_candidates {
                if !(candidate.prev_blank && candidate.next_blank) {
                    continue;
                }
                if candidate.number <= last_number {
                    continue;
                }
                last_number = candidate.number;
                chapters.push(ChapterRange {
                    index: 0,
                    title: candidate.title.clone(),
                    normalized_title: candidate.normalized_title.clone(),
                    start_byte: candidate.start_byte,
                    end_byte: candidate.start_byte,
                    rule: ChapterRule::StandaloneNumeric,
                    number: Some(candidate.number),
                });
            }
        }

        let is_virtual = chapters.is_empty();
        if is_virtual {
            chapters.push(ChapterRange {
                index: 0,
                title: VIRTUAL_CHAPTER_TITLE.to_string(),
                normalized_title: normalize_title(VIRTUAL_CHAPTER_TITLE),
                start_byte: 0,
                end_byte: total_bytes,
                rule: ChapterRule::VirtualFullText,
                number: None,
            });
        } else {
            chapters.sort_by_key(|chapter| chapter.start_byte);
            chapters.dedup_by_key(|chapter| chapter.start_byte);
            for (index, chapter) in chapters.iter_mut().enumerate() {
                chapter.index = index as u32;
            }
            for position in 0..chapters.len() {
                let end = chapters
                    .get(position + 1)
                    .map_or(total_bytes, |next| next.start_byte);
                chapters[position].end_byte = end.max(chapters[position].start_byte);
            }
            // 首个章节必须从文件起点开始，否则文件开头会没有归属章节。
            if let Some(first) = chapters.first_mut() {
                first.start_byte = 0;
            }
        }

        ChapterIndex {
            chapters,
            is_virtual,
            index_version: CHAPTER_INDEX_VERSION,
            total_bytes,
            scanned_bytes: self.scanned_bytes,
            lines_scanned: self.line_index,
            undecodable_lines: self.undecodable_lines,
            truncated_lines: self.truncated_lines,
            numeric_candidates,
            cancelled: self.cancelled,
        }
    }

    /// 把当前未完成行当作一行处理（用于缝隙与文本末尾）。
    fn finish_current_line(&mut self) {
        if self.line_buffer_full {
            self.truncated_lines += 1;
            self.line_buffer_full = false;
            self.carry.clear();
            self.pending_numeric = None;
            return;
        }
        if self.carry.is_empty() {
            return;
        }
        let line = std::mem::take(&mut self.carry);
        self.process_line(&line, self.line_start, self.encoding);
    }

    /// 处理一行（不含换行符）。
    /// 指定内容编码（builder；默认 UTF-8）。
    fn with_encoding(mut self, encoding: TextEncoding) -> Self {
        self.encoding = encoding;
        self
    }

    fn process_line(&mut self, line: &[u8], start_byte: u64, encoding: TextEncoding) {
        self.line_index += 1;

        // 先结算上一行的独立数字标题候选：本行是否为空行决定它的「后一行是空行」。
        let current_blank = is_blank_line(line);
        if let Some(mut pending) = self.pending_numeric.take() {
            pending.next_blank = current_blank;
            self.numeric_candidates.push(pending);
        }

        if current_blank {
            self.last_line_blank = true;
            return;
        }

        let trimmed_line = trim_ascii_whitespace(strip_trailing_cr(line));
        // 必须按**内容编码**解码：曾用 `from_utf8`，非 UTF-8 文档每一行都失败 ⇒ 目录只剩「全文」。
        let Some(text) = decode_line(trimmed_line, encoding) else {
            self.undecodable_lines += 1;
            self.last_line_blank = false;
            return;
        };

        let prev_blank = self.last_line_blank || self.line_index == 1;
        self.last_line_blank = false;

        let Some(found) = detect_chapter_title(text.as_ref()) else {
            return;
        };

        if found.rule.is_weak() {
            self.pending_numeric = Some(NumericCandidate {
                start_byte,
                title: found.title,
                normalized_title: found.normalized_title,
                number: found.number.unwrap_or(0),
                prev_blank,
                next_blank: false,
            });
            return;
        }

        self.chapters.push(ChapterRange {
            index: 0,
            title: found.title,
            normalized_title: found.normalized_title,
            start_byte,
            end_byte: start_byte,
            rule: found.rule,
            number: found.number,
        });
    }
}

/// 单步扫描结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanStep {
    /// 又推进了一个窗口。
    Progress {
        /// 下一个待读偏移。
        offset: u64,
        /// 已读窗口数。
        windows: u64,
        /// 已识别章节数（部分结果）。
        chapters: usize,
    },
    /// 已扫描到文件末尾。
    Done,
    /// 已被取消（结果是部分结果）。
    Cancelled,
}

/// 扫描统计（用于断言读取量有界）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanStats {
    /// 读取的窗口次数。
    pub windows_read: u64,
    /// 从数据源物化的原始字节总数。
    pub bytes_read: u64,
    /// 单次物化字节峰值。
    pub max_window_bytes: u64,
}

impl ScanStats {
    fn record(&mut self, raw_bytes: u64) {
        self.windows_read += 1;
        self.bytes_read += raw_bytes;
        self.max_window_bytes = self.max_window_bytes.max(raw_bytes);
    }
}

/// 可增量推进的章节扫描会话：每个 [`ChapterScanSession::step`] 只读一个窗口。
///
/// 会话持有数据源引用与扫描器；调用方可以在两次 `step` 之间做其它工作
/// （例如让路给前台翻页），或直接 [`ChapterScanSession::cancel`]。
pub struct ChapterScanSession<'a, S: TextSource + ?Sized> {
    source: &'a S,
    scanner: ChapterScanner,
    offset: u64,
    total_bytes: u64,
    stats: ScanStats,
    done: bool,
    /// 内容编码（章节标记必须先按它解码再匹配；非 UTF-8 文档曾因此扫不出章节）。
    encoding: TextEncoding,
}

impl<'a, S: TextSource + ?Sized> ChapterScanSession<'a, S> {
    /// 新建会话（读取数据源长度，不读正文）。
    pub fn new(source: &'a S) -> Self {
        Self::new_with_encoding(source, TextEncoding::Utf8)
    }

    /// 按给定内容编码新建扫描会话（CR2：UTF-8/BOM、UTF-16 LE/BE、GB18030、GBK、Big5）。
    pub fn new_with_encoding(source: &'a S, encoding: TextEncoding) -> Self {
        Self {
            source,
            scanner: ChapterScanner::new().with_encoding(encoding),
            offset: 0,
            total_bytes: source.len(),
            stats: ScanStats::default(),
            done: false,
            encoding,
        }
    }

    /// 当前进度（下一个待读偏移）。
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// 已识别章节数（部分结果）。
    pub fn chapters_found(&self) -> usize {
        self.scanner.accepted_chapters()
    }

    /// 读取统计。
    pub fn stats(&self) -> ScanStats {
        self.stats
    }

    /// 读取一个窗口并推进。每次调用最多读取 `source.default_window_bytes()` 字节。
    pub fn step(&mut self) -> Result<ScanStep, SourceError> {
        if self.scanner.is_cancelled() {
            return Ok(ScanStep::Cancelled);
        }
        if self.done || self.offset >= self.total_bytes {
            self.done = true;
            return Ok(ScanStep::Done);
        }
        let window = self
            .source
            .read_at(self.offset, self.source.default_window_bytes())?;
        self.stats.record(window.raw_bytes() as u64);
        if !window.data.is_empty() {
            self.scanner.push_bytes(window.start(), &window.data);
        }
        let next = window.end();
        if window.eof || window.data.is_empty() || next <= self.offset {
            self.done = true;
            return Ok(ScanStep::Done);
        }
        self.offset = next;
        Ok(ScanStep::Progress {
            offset: self.offset,
            windows: self.stats.windows_read,
            chapters: self.scanner.accepted_chapters(),
        })
    }

    /// 请求取消（下一次 `step` 返回 [`ScanStep::Cancelled`]）。
    pub fn cancel(&mut self) {
        self.scanner.cancel();
    }

    /// 结束扫描并返回章节索引 + 统计。
    pub fn finish(self) -> (ChapterIndex, ScanStats) {
        (self.scanner.finish(self.total_bytes), self.stats)
    }
}

/// 从头到尾扫描章节（每读一个窗口检查一次取消回调）。
///
/// 这是便利入口；需要真正“让路/分批”的调用方请使用 [`ChapterScanSession`]。
pub fn scan_chapters<S: TextSource + ?Sized>(
    source: &S,
    cancel: &dyn Fn() -> bool,
) -> Result<(ChapterIndex, ScanStats), SourceError> {
    scan_chapters_with_encoding(source, cancel, TextEncoding::Utf8)
}

/// 与 [`scan_chapters`] 相同，但按给定**内容编码**解码每行再匹配章节规则。
///
/// 真机缺陷背景：GBK/GB18030 文档（中文 TXT 的常见编码）曾因 `from_utf8` 逐行失败
/// 而扫不出任何章节，目录只剩虚拟章节「全文」。
pub fn scan_chapters_with_encoding<S: TextSource + ?Sized>(
    source: &S,
    cancel: &dyn Fn() -> bool,
    encoding: TextEncoding,
) -> Result<(ChapterIndex, ScanStats), SourceError> {
    let mut session = ChapterScanSession::new_with_encoding(source, encoding);
    loop {
        if cancel() {
            session.cancel();
        }
        match session.step()? {
            ScanStep::Progress { .. } => continue,
            ScanStep::Done | ScanStep::Cancelled => break,
        }
    }
    Ok(session.finish())
}

/// 判断一行（不含换行符）是否为空行。
fn is_blank_line(line: &[u8]) -> bool {
    trim_ascii_whitespace(strip_trailing_cr(line)).is_empty()
}

/// 去掉行尾的 `\r`（CRLF）。
fn strip_trailing_cr(line: &[u8]) -> &[u8] {
    match line.split_last() {
        Some((b'\r', rest)) => rest,
        _ => line,
    }
}

/// 去掉两端的 ASCII 空白（全角空白由 `str::trim` 处理）。
fn trim_ascii_whitespace(trimmed: &[u8]) -> &[u8] {
    let mut start = 0usize;
    let mut end = trimmed.len();
    while start < end && trimmed[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && trimmed[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &trimmed[start..end]
}

/// 标题后可接受的分隔符（用于 `楔子：少年` 这类写法，避免匹配正文句子）。
fn is_title_separator(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            '：' | ':'
                | '、'
                | '·'
                | '，'
                | ','
                | '.'
                | '—'
                | '-'
                | '～'
                | '~'
                | '「'
                | '（'
                | '('
                | '【'
                | '['
                | '"'
                | '\''
                | '“'
                | '《'
        )
}

/// 关键字标题（无序号）。
const KEYWORD_TITLES: [&str; 5] = ["楔子", "序章", "前言", "后记", "尾声"];

/// 章节单位字符。
/// 章节单位：`第<数字><单位>`。
///
/// `卷/集` 是「大层」（卷级），`章/节/回` 是正文层；`集` 是用户真实样本要求
/// （`盘龙` 的标题形如「第一集 盘龙之戒 第一章 小镇的早晨」）。
const CHAPTER_UNITS: [char; 5] = ['章', '卷', '集', '节', '回'];

/// 句末标点：紧贴标记且以此结尾的尾巴更像**正文句子**（真机实测：
/// `盘龙` 的「第二章到！」「第四章到！」被误判为章节标题，导致目录只出现这两条）。
const SENTENCE_TERMINATORS: [char; 8] = ['。', '！', '？', '!', '?', '；', ';', '…'];

/// 识别一行文本是否是章节标题。
///
/// 返回 `None` 表示「不是标题」；长度/字符数超限的行一律不是标题。
pub fn detect_chapter_title(line: &str) -> Option<ChapterMatch> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.len() > MAX_TITLE_LINE_BYTES || trimmed.chars().count() > MAX_TITLE_LINE_CHARS {
        return None;
    }

    let (rule, number) = detect_rule(trimmed)?;
    Some(ChapterMatch {
        rule,
        number,
        title: trimmed.to_string(),
        normalized_title: normalize_title(trimmed),
    })
}

/// 按优先级判定规则并解析序号。
fn detect_rule(trimmed: &str) -> Option<(ChapterRule, Option<u64>)> {
    if let Some((number, unit)) = parse_di_head(trimmed) {
        let rule = if unit == '卷' {
            ChapterRule::VolumeNumber
        } else {
            ChapterRule::ChapterNumber
        };
        return Some((rule, Some(number)));
    }
    if let Some(number) = parse_volume_head(trimmed) {
        return Some((ChapterRule::VolumeNumber, Some(number)));
    }
    if is_keyword_title(trimmed) {
        return Some((ChapterRule::Keyword, None));
    }
    if let Some(number) = parse_english_head(trimmed) {
        return Some((ChapterRule::English, Some(number)));
    }
    if let Some(number) = parse_standalone_numeric(trimmed) {
        return Some((ChapterRule::StandaloneNumeric, Some(number)));
    }
    None
}

/// `第<数字><章|卷>`：数字可以是阿拉伯数字或中文数字，允许 `第 12 章`、`第001章`。
fn parse_di_head(trimmed: &str) -> Option<(u64, char)> {
    let rest = trimmed.strip_prefix('第')?;
    let rest = rest.trim_start();
    let (number, after_number) = parse_leading_number(rest)?;
    let after_number = after_number.trim_start();
    let unit = after_number.chars().next()?;
    if !CHAPTER_UNITS.contains(&unit) {
        return None;
    }
    let tail = &after_number[unit.len_utf8()..];
    if !is_acceptable_title_tail(tail) {
        return None;
    }
    Some((number, unit))
}

/// `卷<数字>`（无「第」）。
fn parse_volume_head(trimmed: &str) -> Option<u64> {
    let rest = trimmed.strip_prefix('卷')?;
    let (number, after_number) = parse_leading_number(rest)?;
    if !is_acceptable_title_tail(after_number) {
        return None;
    }
    Some(number)
}

/// 标题尾部是否可接受：空、以分隔符开头，或是不超过 [`MAX_TITLE_TAIL_CHARS`] 的短副标题。
///
/// 该约束把 `第一章 初见` / `第一章初见` 与整句正文区分开，避免把叙述句误判为标题。
fn is_acceptable_title_tail(tail: &str) -> bool {
    match tail.chars().next() {
        None => true,
        // 以分隔符开头（空格/：/·/「 等）→ 视为标题，不限制长度之外的形态。
        Some(ch) if is_title_separator(ch) => true,
        Some(_) => {
            // 紧贴标记、没有分隔符的尾巴：短且**不以句末标点结尾**才算标题。
            // 例如「第二章到！」被排除，而「第1章旧土」这类无空格写法仍然接受。
            tail.chars().count() <= MAX_TITLE_TAIL_CHARS && !ends_with_sentence_terminator(tail)
        }
    }
}

/// 尾巴是否以句末标点结尾（用于排除「第二章到！」这类正文句子）。
fn ends_with_sentence_terminator(tail: &str) -> bool {
    matches!(tail.chars().last(), Some(ch) if SENTENCE_TERMINATORS.contains(&ch))
}

/// 解析开头的数字（阿拉伯数字或中文数字），返回 `(数值, 剩余)`。
fn parse_leading_number(text: &str) -> Option<(u64, &str)> {
    let digits: String = text.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if !digits.is_empty() {
        let value = digits.parse::<u64>().ok()?;
        return Some((value, &text[digits.len()..]));
    }
    let numerals: String = text
        .chars()
        .take_while(|ch| is_chinese_numeral_char(*ch))
        .collect();
    if numerals.is_empty() {
        return None;
    }
    let value = parse_chinese_numeral(&numerals)?;
    Some((value, &text[numerals.len()..]))
}

/// 是否是中文数字字符。
fn is_chinese_numeral_char(ch: char) -> bool {
    matches!(
        ch,
        '零' | '〇'
            | '一'
            | '二'
            | '两'
            | '三'
            | '四'
            | '五'
            | '六'
            | '七'
            | '八'
            | '九'
            | '十'
            | '百'
            | '千'
            | '万'
    )
}

/// 解析中文数字（支持 万 以内，章节序号远小于该范围）。
///
/// `十二` → 12、`二十` → 20、`一百零三` → 103、`三百二十一` → 321。
pub fn parse_chinese_numeral(text: &str) -> Option<u64> {
    if text.is_empty() {
        return None;
    }
    let mut total: u64 = 0;
    let mut section: u64 = 0;
    let mut pending: u64 = 0;
    let mut saw_symbol = false;
    for ch in text.chars() {
        match ch {
            '零' | '〇' => {
                pending = 0;
                saw_symbol = true;
            }
            '一' => pending = 1,
            '二' | '两' => pending = 2,
            '三' => pending = 3,
            '四' => pending = 4,
            '五' => pending = 5,
            '六' => pending = 6,
            '七' => pending = 7,
            '八' => pending = 8,
            '九' => pending = 9,
            '十' | '百' | '千' => {
                let multiplier = match ch {
                    '十' => 10,
                    '百' => 100,
                    _ => 1000,
                };
                section += pending.max(1) * multiplier;
                pending = 0;
                saw_symbol = true;
            }
            '万' => {
                section += pending;
                total += section.max(1) * 10_000;
                section = 0;
                pending = 0;
                saw_symbol = true;
            }
            _ => return None,
        }
    }
    if !saw_symbol && pending == 0 {
        return None;
    }
    Some(total + section + pending)
}

/// 关键字标题：整行等于关键字，或关键字后紧跟分隔符（`楔子：少年`）。
fn is_keyword_title(trimmed: &str) -> bool {
    KEYWORD_TITLES.iter().any(|keyword| {
        let Some(rest) = trimmed.strip_prefix(keyword) else {
            return false;
        };
        rest.is_empty() || rest.chars().next().is_some_and(is_title_separator)
    })
}

/// `Chapter <N>`（大小写不敏感，允许 `chapter12`）。
fn parse_english_head(trimmed: &str) -> Option<u64> {
    const PREFIX: &[u8; 7] = b"chapter";
    let bytes = trimmed.as_bytes();
    if bytes.len() <= PREFIX.len() {
        return None;
    }
    if !bytes[..PREFIX.len()].eq_ignore_ascii_case(PREFIX) {
        return None;
    }
    // 前 7 字节是 ASCII，索引 7 必然是字符边界。
    let rest = trimmed[PREFIX.len()..].trim_start();
    let (number, tail) = parse_leading_number(rest)?;
    if !is_acceptable_title_tail(tail.trim_start()) {
        return None;
    }
    Some(number)
}

/// 独立数字标题：整行只有 1..=4 位 ASCII 数字。
fn parse_standalone_numeric(trimmed: &str) -> Option<u64> {
    if trimmed.is_empty() || trimmed.len() > MAX_STANDALONE_NUMERIC_DIGITS {
        return None;
    }
    if !trimmed.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    trimmed.parse::<u64>().ok()
}

/// 归一化标题：去掉所有空白、全角转半角、ASCII 转小写。
///
/// 用于查找与去重；原始标题始终保留在 [`ChapterRange::title`]。
pub fn normalize_title(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    for ch in title.chars() {
        if ch.is_whitespace() {
            continue;
        }
        let mapped = if ('\u{FF01}'..='\u{FF5E}').contains(&ch) {
            char::from_u32(ch as u32 - 0xFEE0).unwrap_or(ch)
        } else {
            ch
        };
        if mapped.is_ascii_uppercase() {
            out.push(mapped.to_ascii_lowercase());
        } else {
            out.push(mapped);
        }
    }
    out
}

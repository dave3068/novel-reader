//! TXT 文件型有限窗口数据源：`TextSource` 抽象与文件实现。
//!
//! 契约（`results/gates/gate-0-baseline.md` §2.3、reading-core CR1/AC1）：
//!
//! - 单次读取默认窗口 64 KiB、硬上限 256 KiB；请求长度超过硬上限**直接拒绝**（不分配、不读取）；
//! - 窗口两端自动修正到合法 UTF-8 字符边界，并回报 `dropped_leading` / `trimmed_tail` / `eof`；
//! - 任何调用都不得整本读取或把全文放入内存：单次从数据源物化的原始字节 ≤ 硬上限。
//!
//! 边界修正只做“对齐”，不做编码转换或内容校验（编码识别/覆盖是 RC3 的职责）：
//! 本模块只丢弃窗口首部落入字符中间的续接字节，以及窗口尾部不完整的字符。

use crate::encoding::TextEncoding;
use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;

use crate::fingerprint::{Fingerprint, FINGERPRINT_SAMPLE_BYTES};
use crate::{DEFAULT_WINDOW_BYTES, MAX_WINDOW_BYTES};

/// 数据源错误。数值与 reading-core 的冻结错误码一致（gate-0 §2.2）。
///
/// `reading-source` 是 reading-core 的**下层**依赖，不能反向依赖 `reading_core`
/// （否则形成依赖环、违反 crate 边界），因此这里保留同数值的本地枚举；
/// 上层直接取 [`SourceError::code`] 映射为 `ErrorCode`，不得重新定义数值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceError {
    /// `NR_ERR_INVALID_ARGUMENT` = 1：长度为 0、超过窗口硬上限、非普通文件等参数错误。
    InvalidArgument,
    /// `NR_ERR_OUT_OF_RANGE` = 2：字节偏移越界。
    OutOfRange,
    /// `NR_ERR_UTF8_BOUNDARY` = 3：偏移未落在合法 UTF-8 边界。
    /// 本实现会自动修正边界，因此不会返回该码；保留以便上层映射与契约对照。
    Utf8Boundary,
    /// `NR_ERR_IO` = 6：底层 I/O 失败。
    IoError,
    /// `NR_ERR_INTERNAL` = 10：内部错误（如内部锁中毒）。
    Internal,
}

impl SourceError {
    /// 跨 ABI 的错误码数值（与 C 头文件 `NR_ERR_*` 一致）。
    pub const fn code(self) -> i32 {
        match self {
            SourceError::InvalidArgument => 1,
            SourceError::OutOfRange => 2,
            SourceError::Utf8Boundary => 3,
            SourceError::IoError => 6,
            SourceError::Internal => 10,
        }
    }

    /// 稳定英文短名（日志用，不得作为业务分支依据）。
    pub const fn as_str(self) -> &'static str {
        match self {
            SourceError::InvalidArgument => "InvalidArgument",
            SourceError::OutOfRange => "OutOfRange",
            SourceError::Utf8Boundary => "Utf8Boundary",
            SourceError::IoError => "IoError",
            SourceError::Internal => "Internal",
        }
    }
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::error::Error for SourceError {}

/// I/O 失败统一映射为 `IoError`（错误码 6）。
impl From<std::io::Error> for SourceError {
    fn from(_error: std::io::Error) -> Self {
        SourceError::IoError
    }
}

/// 窗口两端的 UTF-8 边界修正量。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BoundaryAdjustment {
    /// 因落在多字节字符中间而被丢弃的首部续接字节数（0..=3）。
    pub dropped_leading: usize,
    /// 因尾部字符不完整而被裁掉的字节数（0..=4）。
    pub trimmed_tail: usize,
}

impl BoundaryAdjustment {
    /// 无修正。
    pub const NONE: Self = Self {
        dropped_leading: 0,
        trimmed_tail: 0,
    };

    /// 在长度为 `raw_len` 的原始窗口内应保留的区间。
    pub fn keep_range(&self, raw_len: usize) -> Range<usize> {
        let start = self.dropped_leading.min(raw_len);
        let end = raw_len.saturating_sub(self.trimmed_tail).max(start);
        start..end
    }
}

/// 单次窗口读取的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextWindow {
    /// 调用方请求的起始字节偏移（未修正）。
    pub offset: u64,
    /// 已对齐到合法 UTF-8 字符边界的字节。
    pub data: Vec<u8>,
    /// 首部被丢弃的续接字节数。
    pub dropped_leading: usize,
    /// 尾部被裁掉的字节数（下一个窗口会重新读到这些字节）。
    pub trimmed_tail: usize,
    /// 本窗口是否已触及文件末尾（此后不再有字节）。
    pub eof: bool,
    /// 调用方请求的长度（用于核对上限）。
    pub requested_len: usize,
}

impl TextWindow {
    /// 返回数据在文件中的起始偏移（`offset + dropped_leading`）。
    pub fn start(&self) -> u64 {
        self.offset + self.dropped_leading as u64
    }

    /// 返回数据结束后的偏移；调用方应以此为下一次读取的偏移继续前进。
    pub fn end(&self) -> u64 {
        self.start() + self.data.len() as u64
    }

    /// 本次读取从数据源物化的原始字节数（修正前后的字节总和）。
    pub fn raw_bytes(&self) -> usize {
        self.data.len() + self.dropped_leading + self.trimmed_tail
    }

    /// 窗口数据是否为空。
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// 以 UTF-8 文本视图访问窗口数据；非 UTF-8 内容由上层（RC3）判定。
    pub fn as_str(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(&self.data)
    }
}

/// 单次读取的物化字节统计（用于“不得整本读取 / 单次 ≤ 硬上限”的可断言证据）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReadStats {
    /// 从数据源读取的次数（含 0 字节的 EOF 读取）。
    pub read_calls: u64,
    /// 最近一次读取物化的原始字节数。
    pub last_materialized_bytes: u64,
    /// 累计物化的原始字节数。
    pub total_materialized_bytes: u64,
    /// 单次物化字节峰值（必须 ≤ 窗口硬上限）。
    pub max_materialized_bytes: u64,
}

/// 数据源元信息（`docs/novel-reader-v1-development-plan.md` §6.1）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceMetadata {
    /// 文件字节长度。
    pub size: u64,
    /// 可得的修改时间。
    pub modified: Option<SystemTime>,
    /// 展示名（桌面路径为文件名）。
    pub display_name: Option<String>,
    /// 来源标识（桌面路径；Android 版本为持久化 URI）。
    pub source_identity: String,
}

/// 有限窗口数据源抽象。
///
/// 约束：实现必须保证 [`TextSource::read_at`] 单次物化字节 ≤ [`crate::MAX_WINDOW_BYTES`]，
/// 且永不整本读取；`read_at` 返回的 `data` 必须位于合法 UTF-8 字符边界上。
pub trait TextSource: Send + Sync {
    /// 数据源元信息。
    fn metadata(&self) -> SourceMetadata;

    /// 按原始字节偏移读取有限窗口。
    ///
    /// - `len == 0` 或 `len > max_window_bytes()`：返回 [`SourceError::InvalidArgument`]；
    /// - `offset > 数据源长度`：返回 [`SourceError::OutOfRange`]；
    /// - `offset == 长度`：返回空数据且 `eof = true`（合法场景，用于判断文件结束）。
    fn read_at(&self, offset: u64, len: usize) -> Result<TextWindow, SourceError>;

    /// 按**内容编码**对齐的窗口读取（默认退回 [`TextSource::read_at`] 的 UTF-8 对齐）。
    ///
    /// 能够按编码对齐的实现必须覆盖它：否则 GBK/GB18030/Big5 文档的窗口头部会被
    /// UTF-8 规则错误裁剪（真机「首行乱码 + 无法上一页」的根因）。
    fn read_at_with_encoding(
        &self,
        offset: u64,
        len: usize,
        encoding: TextEncoding,
    ) -> Result<TextWindow, SourceError> {
        let _ = encoding;
        self.read_at(offset, len)
    }

    /// 计算来源指纹（大小 + 头/中/尾抽样哈希 + 可得的修改时间）。
    fn fingerprint(&self) -> Result<Fingerprint, SourceError>;

    /// 是否支持可靠的随机访问（文件与可定位 URI 为 `true`）。
    fn can_random_access(&self) -> bool;

    /// 默认窗口字节数（64 KiB）。
    fn default_window_bytes(&self) -> usize {
        DEFAULT_WINDOW_BYTES
    }

    /// 窗口硬上限（256 KiB）。
    fn max_window_bytes(&self) -> usize {
        MAX_WINDOW_BYTES
    }

    /// 数据源长度（默认取元信息）。
    fn len(&self) -> u64 {
        self.metadata().size
    }

    /// 数据源是否为空。
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 读取统计快照（默认实现返回空统计，供平台数据源选择性覆盖）。
    fn stats(&self) -> ReadStats {
        ReadStats::default()
    }
}

#[derive(Debug, Default)]
struct StatsCounters {
    read_calls: AtomicU64,
    last_materialized_bytes: AtomicU64,
    total_materialized_bytes: AtomicU64,
    max_materialized_bytes: AtomicU64,
}

impl StatsCounters {
    fn record(&self, bytes: usize) {
        let bytes = bytes as u64;
        self.read_calls.fetch_add(1, Ordering::Relaxed);
        self.last_materialized_bytes.store(bytes, Ordering::Relaxed);
        self.total_materialized_bytes
            .fetch_add(bytes, Ordering::Relaxed);
        self.max_materialized_bytes
            .fetch_max(bytes, Ordering::Relaxed);
    }

    fn snapshot(&self) -> ReadStats {
        ReadStats {
            read_calls: self.read_calls.load(Ordering::Relaxed),
            last_materialized_bytes: self.last_materialized_bytes.load(Ordering::Relaxed),
            total_materialized_bytes: self.total_materialized_bytes.load(Ordering::Relaxed),
            max_materialized_bytes: self.max_materialized_bytes.load(Ordering::Relaxed),
        }
    }
}

/// 桌面文件型数据源：`seek` + 定长读取，永不整本读取。
pub struct FileTextSource {
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
    file: Mutex<File>,
    stats: StatsCounters,
    sample_len: usize,
}

impl FileTextSource {
    /// 打开一个普通文件作为数据源（只读取元信息，不读正文）。
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SourceError> {
        let path = path.as_ref().to_path_buf();
        let file = File::open(&path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(SourceError::InvalidArgument);
        }
        Ok(Self {
            path,
            len: metadata.len(),
            modified: metadata.modified().ok(),
            file: Mutex::new(file),
            stats: StatsCounters::default(),
            sample_len: FINGERPRINT_SAMPLE_BYTES,
        })
    }

    /// 覆盖指纹抽样大小（默认 4 KiB，仅供测试与诊断使用）。
    pub fn with_sample_len(mut self, sample_len: usize) -> Self {
        self.sample_len = sample_len;
        self
    }

    /// 数据源路径。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 读取统计快照。
    pub fn stats(&self) -> ReadStats {
        self.stats.snapshot()
    }

    /// 定长原始读取（**不做** UTF-8 边界修正）。
    ///
    /// 分配与读取的字节数都被限制在 `min(len, 剩余字节)`，因此单次物化字节 ≤ `len`；
    /// 调用方必须保证 `len ≤ MAX_WINDOW_BYTES`。
    fn read_raw_at(&self, offset: u64, len: usize) -> Result<Vec<u8>, SourceError> {
        debug_assert!(
            len <= MAX_WINDOW_BYTES,
            "内部读取长度必须 ≤ 窗口硬上限: {len}"
        );
        let remaining = self.len.saturating_sub(offset);
        let to_read = (len as u64).min(remaining) as usize;
        let mut buffer = vec![0u8; to_read];
        if to_read > 0 {
            let mut file = self.file.lock().map_err(|_| SourceError::Internal)?;
            file.seek(SeekFrom::Start(offset))?;
            let mut filled = 0usize;
            while filled < to_read {
                let read = file.read(&mut buffer[filled..])?;
                if read == 0 {
                    break;
                }
                filled += read;
            }
            buffer.truncate(filled);
        }
        self.stats.record(buffer.len());
        Ok(buffer)
    }
}

impl TextSource for FileTextSource {
    fn metadata(&self) -> SourceMetadata {
        SourceMetadata {
            size: self.len,
            modified: self.modified,
            display_name: self
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            source_identity: self.path.to_string_lossy().into_owned(),
        }
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<TextWindow, SourceError> {
        self.read_at_with_encoding(offset, len, TextEncoding::Utf8)
    }

    fn read_at_with_encoding(
        &self,
        offset: u64,
        len: usize,
        encoding: TextEncoding,
    ) -> Result<TextWindow, SourceError> {
        if len == 0 {
            return Err(SourceError::InvalidArgument);
        }
        if len > MAX_WINDOW_BYTES {
            // 超出硬上限：拒绝，不分配、不读取（AC1 / 卡片禁止事项）。
            return Err(SourceError::InvalidArgument);
        }
        if offset > self.len {
            return Err(SourceError::OutOfRange);
        }
        let raw = self.read_raw_at(offset, len)?;
        // 按**内容编码**对齐：UTF-8 沿用旧规则，其余编码按各自字符长度对齐。
        let adjustment = align_to_boundaries(&raw, offset, encoding);
        let keep = adjustment.keep_range(raw.len());
        let eof = offset + raw.len() as u64 >= self.len;
        Ok(TextWindow {
            offset,
            data: raw[keep].to_vec(),
            dropped_leading: adjustment.dropped_leading,
            trimmed_tail: adjustment.trimmed_tail,
            eof,
            requested_len: len,
        })
    }

    fn fingerprint(&self) -> Result<Fingerprint, SourceError> {
        Fingerprint::compute(self.len, self.sample_len, self.modified, |offset, len| {
            self.read_raw_at(offset, len)
        })
    }

    fn can_random_access(&self) -> bool {
        true
    }

    fn stats(&self) -> ReadStats {
        self.stats.snapshot()
    }
}

/// UTF-8 单字节的续接字节判定（`0b10xx_xxxx`）。
const fn is_continuation(byte: u8) -> bool {
    byte & 0xC0 == 0x80
}

/// 按**内容编码**对齐窗口两端（CR2 的 7 种编码）。
///
/// **为什么必须按编码对齐**（真机事故根因）：旧实现一律用 [`align_to_utf8_boundaries`]，
/// 而 GBK/GB18030/Big5 的汉字首字节范围 0x81–0xFE 与 UTF-8 续接区间 0x80..=0xBF **重叠** ⇒
/// 窗口头部会被**错误裁掉 1–3 字节**，把首个汉字切一半 ⇒
/// 平台解码得替换字符（真机「首行乱码」），且平台算出的页起点落在字符内部
/// （真机「无法上一页」，因为页边界提交会被核心拒绝）。
///
/// 规则：
/// - UTF-8 / UTF-8 BOM：沿用 [`align_to_utf8_boundaries`]（行为不变）；
/// - UTF-16 LE/BE：按**绝对偏移**对齐到码元（偶数）边界，尾部去掉落单字节；
/// - GB18030 / GBK / Big5：起点视为已对齐（对齐沿调用链传递：文件头 0 必然对齐，
///   此后每次读取都用同一编码对齐），只把**尾部不完整字符**裁掉。
pub fn align_to_boundaries(
    data: &[u8],
    absolute_offset: u64,
    encoding: TextEncoding,
) -> BoundaryAdjustment {
    match encoding {
        TextEncoding::Utf8 | TextEncoding::Utf8Bom => align_to_utf8_boundaries(data),
        TextEncoding::Utf16Le | TextEncoding::Utf16Be => {
            let dropped_leading = usize::from(absolute_offset % 2 == 1 && !data.is_empty());
            let rest = &data[dropped_leading..];
            let trailing = rest.len() % 2;
            BoundaryAdjustment {
                dropped_leading,
                trimmed_tail: trailing,
            }
        }
        TextEncoding::Gb18030 | TextEncoding::Gbk | TextEncoding::Big5 => {
            let mut cursor = 0usize;
            while cursor < data.len() {
                let step = legacy_char_len(&data[cursor..], encoding);
                if step == 0 || cursor + step > data.len() {
                    break;
                }
                cursor += step;
            }
            BoundaryAdjustment {
                dropped_leading: 0,
                trimmed_tail: data.len() - cursor,
            }
        }
    }
}

/// 由 UTF-8 首字节推断字符总字节数；非首字节（续接字节）返回 0。
const fn lead_byte_len(byte: u8) -> usize {
    if byte < 0x80 {
        1
    } else if byte >= 0xF0 {
        4
    } else if byte >= 0xE0 {
        3
    } else if byte >= 0xC0 {
        2
    } else {
        0
    }
}

/// `index` 是否是合法的 UTF-8 字符边界（与 `str::is_char_boundary` 语义一致）。
/// 编码感知的字符边界判定（CR2 的 7 种编码）。
///
/// **真机缺陷背景**：旧实现只有 [`is_utf8_boundary`]（判「是否 UTF-8 续接字节 0x80..=0xBF」），
/// 而 GBK/GB18030/Big5 的汉字首字节范围 0x81–0xFE 与之**重叠** ⇒ 大量合法 GB 边界被误判为非法，
/// 表现为翻页/切章报 `NR_ERR_UTF8_BOUNDARY`（「偏移不在合法字符边界」）。
///
/// 规则（`data` 必须是**已对齐**的窗口，起点本身位于字符边界）：
/// - UTF-8 / UTF-8 BOM：沿用 [`is_utf8_boundary`]（非续接字节即为起点）；
/// - UTF-16 LE/BE：码元对齐，偶数偏移即边界；
/// - GB18030 / GBK / Big5：从窗口起点按字符长度步进，能恰好落到 `index` 即为边界
///   （只依赖**字符长度**规则，不需要 Unicode 映射表）。
pub fn is_boundary(data: &[u8], index: usize, encoding: TextEncoding) -> bool {
    if index > data.len() {
        return false;
    }
    if index == 0 || index == data.len() {
        return true;
    }
    match encoding {
        TextEncoding::Utf8 | TextEncoding::Utf8Bom => is_utf8_boundary(data, index),
        TextEncoding::Utf16Le | TextEncoding::Utf16Be => index % 2 == 0,
        TextEncoding::Gb18030 | TextEncoding::Gbk | TextEncoding::Big5 => {
            // 严格步进：**前提是 `data` 从已对齐的基点开始**（调用方用页自身起点 `start_byte`
            // —— 它等于上一页终点、由平台按同一编码算出；见 reading-index 的 `is_char_boundary_at`）。
            // 若从错位起点步进，会把合法边界误判为字符内部（真机「打开即 ER_ERR_3」的历史事故），
            // 因此该前提必须在调用方保证，不能在这里猜测。
            let mut cursor = 0usize;
            while cursor < index {
                let step = legacy_char_len(&data[cursor..], encoding);
                if step == 0 {
                    return false;
                }
                cursor += step;
            }
            cursor == index
        }
    }
}

/// 传统双字节/四字节编码下，`slice` 起点处一个字符的字节长度（不足以构成字符时按剩余长度）。
fn legacy_char_len(slice: &[u8], encoding: TextEncoding) -> usize {
    let Some(&lead) = slice.first() else {
        return 0;
    };
    if lead < 0x80 {
        return 1;
    }
    match encoding {
        TextEncoding::Gb18030 => {
            // GB18030 四字节形式：首字节 + 0x30..=0x39 + 0x81..=0xFE + 0x30..=0x39
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
        TextEncoding::Gbk => {
            if (0x81..=0xFE).contains(&lead) {
                2
            } else {
                1
            }
        }
        TextEncoding::Big5 => {
            if (0x81..=0xFE).contains(&lead) {
                2
            } else {
                1
            }
        }
        _ => 1,
    }
}

pub fn is_utf8_boundary(data: &[u8], index: usize) -> bool {
    match data.get(index) {
        Some(byte) => !is_continuation(*byte),
        None => index == data.len(),
    }
}

/// 计算窗口两端的 UTF-8 边界修正量。
///
/// 规则（只对齐，不转换、不校验）：
///
/// - 首部：丢弃最多 3 个续接字节（窗口起点落在字符中间）；
/// - 尾部：沿续接字节回看，找到最后一个字符起点；
///   若该字符在窗口内不完整则整段裁掉，若其后还有多余的续接字节也一并裁掉。
pub fn align_to_utf8_boundaries(data: &[u8]) -> BoundaryAdjustment {
    let dropped_leading = leading_continuation_bytes(data);
    let rest = &data[dropped_leading..];
    BoundaryAdjustment {
        dropped_leading,
        trimmed_tail: incomplete_trailing_bytes(rest),
    }
}

fn leading_continuation_bytes(data: &[u8]) -> usize {
    let mut dropped = 0usize;
    while dropped < 3 && dropped < data.len() && is_continuation(data[dropped]) {
        dropped += 1;
    }
    dropped
}

fn incomplete_trailing_bytes(data: &[u8]) -> usize {
    let len = data.len();
    if len == 0 {
        return 0;
    }
    // 回看尾部续接字节（合法 UTF-8 中续接字节最多 3 个）。
    let mut trailing = 0usize;
    while trailing < 4 && trailing < len && is_continuation(data[len - 1 - trailing]) {
        trailing += 1;
    }
    if trailing >= len {
        // 整个窗口都是续接字节：不存在任何字符边界，整段丢弃以避免返回半个字符。
        return len;
    }
    if trailing == 4 {
        // 4 个连续续接字节在合法 UTF-8 中不可能出现（非 UTF-8 内容），保守裁掉。
        return 4;
    }
    let start = len - 1 - trailing;
    let byte = data[start];
    if byte < 0x80 {
        // 尾部是 ASCII：其后多余的续接字节一并裁掉。
        return trailing;
    }
    let needed = lead_byte_len(byte);
    let end = start + needed;
    if end == len {
        0
    } else if end > len {
        // 尾部字符不完整：从该字符起点整段裁掉。
        len - start
    } else {
        // 该字符完整，其后仍有游离的续接字节。
        len - end
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    fn walk_all(source: &FileTextSource, window: usize) -> (u64, u64) {
        let mut offset = 0u64;
        let mut windows = 0u64;
        while offset < source.len() {
            let read = source.read_at(offset, window).expect("窗口读取");
            assert!(
                read.data.len() <= window,
                "单窗口返回字节不得超过请求窗口: {} > {window}",
                read.data.len()
            );
            assert!(
                read.raw_bytes() <= MAX_WINDOW_BYTES,
                "单次物化字节不得超过硬上限: {}",
                read.raw_bytes()
            );
            assert!(
                source.stats().max_materialized_bytes <= MAX_WINDOW_BYTES as u64,
                "单次物化字节峰值不得超过硬上限: {}",
                source.stats().max_materialized_bytes
            );
            let next = read.end();
            assert!(
                next > offset,
                "窗口必须严格前进: offset={offset} next={next}"
            );
            offset = next;
            windows += 1;
            if read.eof {
                break;
            }
        }
        assert!(offset <= source.len());
        assert!(
            source.len() - offset < 4,
            "游走结束后剩余字节必须少于一个最大字符（尾部半个字符无法再切出完整窗口）: 剩余 {}",
            source.len() - offset
        );
        (offset, windows)
    }

    fn walk_sampled(source: &FileTextSource, window: usize, samples: u64) -> u64 {
        let size = source.len();
        let start_bound = size.saturating_sub(window as u64);
        let mut windows = 0u64;
        for index in 0..samples {
            let offset = start_bound * index / samples;
            let read = source.read_at(offset, window).expect("抽样窗口读取");
            assert!(read.data.len() <= window);
            assert!(read.raw_bytes() <= MAX_WINDOW_BYTES);
            assert!(
                read.as_str().is_ok(),
                "压力夹具必须是合法 UTF-8，窗口起点 {offset} 处不是"
            );
            assert_eq!(read.offset, offset);
            windows += 1;
        }
        // 末字节窗口必须回报 eof，且能读到内容。
        let last = source.read_at(size - 1, window).expect("末字节窗口");
        assert!(last.eof, "末字节窗口必须回报 eof");
        assert!(!last.is_empty());
        windows += 1;
        // 尾部窗口必须精确结束于文件末尾。
        let tail_len = (window as u64).min(size);
        let tail = source.read_at(size - tail_len, window).expect("尾部窗口");
        assert!(tail.eof);
        assert_eq!(tail.end(), size, "尾部窗口必须结束于文件末尾");
        windows + 1
    }

    #[test]
    fn bounded_window_reads_requested_range_from_file() {
        let source = test_support::fixture_source("utf8-mixed-lines.txt");
        let expected =
            std::fs::read(test_support::fixture("utf8-mixed-lines.txt")).expect("读取夹具");
        let read = source.read_at(0, 64).expect("窗口读取");
        assert_eq!(read.offset, 0);
        assert_eq!(read.requested_len, 64);
        assert_eq!(read.dropped_leading, 0, "文件起点必然落在字符边界上");
        assert_eq!(read.raw_bytes(), 64, "未被 EOF 裁剪时物化字节等于请求长度");
        assert!(read.trimmed_tail <= 3, "尾部裁剪不得超过一个字符");
        assert_eq!(read.start(), 0);
        assert_eq!(read.end(), (64 - read.trimmed_tail) as u64);
        assert_eq!(read.data, expected[..read.data.len()].to_vec());
        assert!(!read.eof);

        let middle = source.read_at(100, 32).expect("中部窗口");
        assert_eq!(middle.offset, 100);
        assert_eq!(middle.raw_bytes(), 32);
        assert!(middle.dropped_leading <= 3);
        let start = 100 + middle.dropped_leading;
        assert_eq!(middle.start(), start as u64);
        assert_eq!(
            middle.data,
            expected[start..start + middle.data.len()].to_vec()
        );
        assert_eq!(middle.end(), (start + middle.data.len()) as u64);
    }

    #[test]
    fn bounded_window_default_window_is_usable_and_bounded() {
        let source = test_support::fixture_source("utf8-long-line.txt");
        assert_eq!(source.default_window_bytes(), DEFAULT_WINDOW_BYTES);
        assert_eq!(source.max_window_bytes(), MAX_WINDOW_BYTES);
        assert!(source.can_random_access());
        assert!(
            source.len() > DEFAULT_WINDOW_BYTES as u64,
            "夹具必须大于默认窗口"
        );

        let read = source
            .read_at(0, source.default_window_bytes())
            .expect("默认窗口读取");
        assert_eq!(read.requested_len, DEFAULT_WINDOW_BYTES);
        assert!(read.data.len() <= DEFAULT_WINDOW_BYTES);
        assert!(!read.eof, "133120 字节文件的首个 64 KiB 窗口不应为 EOF");
        assert_eq!(
            source.stats().last_materialized_bytes,
            DEFAULT_WINDOW_BYTES as u64
        );

        let metadata = source.metadata();
        assert_eq!(metadata.size, source.len());
        assert_eq!(metadata.display_name.as_deref(), Some("utf8-long-line.txt"));
        assert!(metadata.source_identity.ends_with("utf8-long-line.txt"));
        assert!(metadata.modified.is_some(), "桌面文件必须有修改时间");
    }

    #[test]
    fn bounded_window_clips_at_file_end_and_reports_eof() {
        let source = test_support::fixture_source("ascii-short.txt");
        let expected = std::fs::read(test_support::fixture("ascii-short.txt")).expect("读取夹具");
        assert_eq!(expected.len(), 20);
        let read = source.read_at(10, DEFAULT_WINDOW_BYTES).expect("尾部窗口");
        assert_eq!(read.data, expected[10..].to_vec());
        assert_eq!(read.data.len(), 10, "必须裁剪到文件末尾");
        assert!(read.eof);
        assert_eq!(read.end(), source.len());
    }

    #[test]
    fn bounded_window_offset_at_eof_returns_empty_with_eof() {
        let source = test_support::fixture_source("ascii-short.txt");
        let read = source
            .read_at(source.len(), 64)
            .expect("EOF 处读取必须合法");
        assert!(read.is_empty());
        assert!(read.eof);
        assert_eq!(read.dropped_leading, 0);
        assert_eq!(read.trimmed_tail, 0);
        assert_eq!(read.raw_bytes(), 0);
        assert_eq!(read.end(), source.len());
    }

    #[test]
    fn bounded_window_offset_beyond_eof_is_out_of_range() {
        let source = test_support::fixture_source("ascii-short.txt");
        let error = source
            .read_at(source.len() + 1, 64)
            .expect_err("越界必须报错");
        assert_eq!(error, SourceError::OutOfRange);
        assert_eq!(error.code(), 2);
        assert!(source.read_at(u64::MAX, 64).is_err());
    }

    #[test]
    fn bounded_window_rejects_zero_length_request() {
        let source = test_support::fixture_source("ascii-short.txt");
        let before = source.stats();
        let error = source.read_at(0, 0).expect_err("长度 0 必须拒绝");
        assert_eq!(error, SourceError::InvalidArgument);
        assert_eq!(error.code(), 1);
        assert_eq!(source.stats(), before, "被拒绝的请求不得触碰数据源");
    }

    #[test]
    fn bounded_window_walk_covers_long_line_fixture_exactly() {
        let source = test_support::fixture_source("utf8-long-line.txt");
        let size = source.len();
        let (end, windows) = walk_all(&source, DEFAULT_WINDOW_BYTES);
        assert_eq!(end, size, "窗口游走必须精确覆盖到文件末尾");
        assert_eq!(windows, 3, "133120 字节按 64 KiB 窗口应恰好 3 个窗口");
        let stats = source.stats();
        assert!(stats.max_materialized_bytes <= MAX_WINDOW_BYTES as u64);
        assert!(stats.total_materialized_bytes <= size + MAX_WINDOW_BYTES as u64);
    }

    #[test]
    fn bounded_window_walk_never_materializes_more_than_cap() {
        let source = test_support::fixture_source("utf8-truncated-tail.txt");
        let (end, windows) = walk_all(&source, MAX_WINDOW_BYTES);
        assert_eq!(
            end,
            source.len() - 2,
            "尾部半个字符（2 字节）无法构成完整窗口，游走应停在它之前"
        );
        assert!(windows >= 1);
        let stats = source.stats();
        assert!(
            stats.max_materialized_bytes <= MAX_WINDOW_BYTES as u64,
            "单次物化字节峰值不得超过硬上限: {}",
            stats.max_materialized_bytes
        );
        assert_eq!(stats.read_calls, windows);
    }

    #[test]
    fn max_window_cap_rejects_oversized_request_without_reading() {
        let source = test_support::fixture_source("utf8-long-line.txt");
        let before = source.stats();
        for oversized in [MAX_WINDOW_BYTES + 1, MAX_WINDOW_BYTES * 4] {
            let error = source
                .read_at(0, oversized)
                .expect_err("超过硬上限必须拒绝");
            assert_eq!(error, SourceError::InvalidArgument);
            assert_eq!(error.code(), 1);
        }
        assert_eq!(source.stats(), before, "被拒绝的请求不得触碰数据源");

        // 硬上限本身必须被接受，且单次物化字节不超过上限。
        let read = source
            .read_at(0, MAX_WINDOW_BYTES)
            .expect("硬上限请求必须被接受");
        assert_eq!(read.requested_len, MAX_WINDOW_BYTES);
        assert!(read.data.len() <= MAX_WINDOW_BYTES);
        assert_eq!(read.raw_bytes(), read.data.len(), "该夹具无多字节边界裁剪");
        let stats = source.stats();
        assert_eq!(stats.read_calls, before.read_calls + 1);
        assert_eq!(stats.last_materialized_bytes as usize, read.raw_bytes());
        assert!(stats.max_materialized_bytes <= MAX_WINDOW_BYTES as u64);
    }

    #[test]
    fn max_window_cap_and_bounded_window_hold_on_pressure_fixtures() {
        let Some((dir, manifest)) = test_support::pressure_manifest() else {
            eprintln!(
                "SKIP(pressure): 未找到压力档夹具，先运行 \
                 `python3 scripts/generate-fixtures.py --profile pressure --seed 20260912 --output /tmp/nr-pressure`"
            );
            // 退化断言：仍然验证“单次物化字节 ≤ 硬上限”，并显式标注压力路径未被执行。
            let temp = test_support::temp_dir("pressure-fallback");
            let payload = vec![b'a'; DEFAULT_WINDOW_BYTES * 3];
            let path = temp.write("fallback.txt", &payload);
            let source = FileTextSource::open(&path).expect("打开临时文件");
            let read = source.read_at(0, MAX_WINDOW_BYTES).expect("窗口读取");
            assert!(read.data.len() <= MAX_WINDOW_BYTES);
            assert!(
                source.stats().max_materialized_bytes <= MAX_WINDOW_BYTES as u64,
                "单次物化字节必须 ≤ {MAX_WINDOW_BYTES}"
            );
            eprintln!(
                "WARN: pressure fixture 路径未执行，仅验证了 {} 字节临时文件的物化上限",
                payload.len()
            );
            return;
        };

        let mut exercised = Vec::new();
        for entry in manifest
            .entries
            .iter()
            .filter(|entry| entry.kind == "pressure")
        {
            let path = dir.join(&entry.name);
            assert!(path.is_file(), "压力夹具缺失: {}", path.display());
            let source = FileTextSource::open(&path).expect("打开压力夹具");
            assert_eq!(source.len(), entry.bytes, "夹具大小必须与 manifest 一致");

            // 1) 超过硬上限的请求必须被拒绝，且不触碰数据源。
            let before = source.stats();
            let rejected = source
                .read_at(0, MAX_WINDOW_BYTES + 1)
                .expect_err("超过硬上限必须拒绝");
            assert_eq!(rejected.code(), 1);
            assert_eq!(source.stats(), before);

            // 2) 单窗口读取：物化字节 ≤ 硬上限，且窗口内数据合法。
            let single = source
                .read_at(entry.bytes / 2, MAX_WINDOW_BYTES)
                .expect("中部窗口");
            assert!(single.data.len() <= MAX_WINDOW_BYTES);
            assert!(single.raw_bytes() <= MAX_WINDOW_BYTES);
            assert!(single.as_str().is_ok(), "压力夹具窗口必须是合法 UTF-8");
            let after_single = source.stats();
            assert!(
                after_single.last_materialized_bytes <= MAX_WINDOW_BYTES as u64,
                "单次物化字节必须 ≤ {MAX_WINDOW_BYTES}，实测 {}",
                after_single.last_materialized_bytes
            );

            // 3) 游走：≤ 128 MiB 全量走完；更大文件抽样游走（控制耗时与 I/O）。
            let full_walk = entry.bytes <= 128 * 1024 * 1024;
            let before_walk = source.stats();
            let (end, windows) = if full_walk {
                walk_all(&source, MAX_WINDOW_BYTES)
            } else {
                (entry.bytes, walk_sampled(&source, MAX_WINDOW_BYTES, 64))
            };
            assert_eq!(end, source.len(), "窗口游走必须覆盖到文件末尾");

            let stats = source.stats();
            assert!(
                stats.max_materialized_bytes <= MAX_WINDOW_BYTES as u64,
                "单次物化字节峰值必须 ≤ {MAX_WINDOW_BYTES}（实测 {}）",
                stats.max_materialized_bytes
            );
            let walk_calls = stats.read_calls - before_walk.read_calls;
            let walk_bytes = stats.total_materialized_bytes - before_walk.total_materialized_bytes;
            assert!(walk_calls > 0, "游走必须真正发生读取");
            assert!(
                walk_bytes <= walk_calls * MAX_WINDOW_BYTES as u64,
                "游走累计物化 {walk_bytes} 字节，超过 {walk_calls} 次 × 硬上限"
            );
            if full_walk {
                // 全量游走：只允许“每个窗口重读上一个窗口尾部 ≤4 字节”的重叠。
                assert!(
                    walk_bytes <= entry.bytes + walk_calls * 8,
                    "全量游走累计物化 {walk_bytes} 字节，超过文件大小 {} + 重叠上限",
                    entry.bytes
                );
            } else {
                assert!(
                    walk_bytes < entry.bytes,
                    "抽样游走不得物化整本文件：{walk_bytes} 字节 vs 文件 {} 字节",
                    entry.bytes
                );
            }
            eprintln!(
                "pressure fixture {} ({} 字节): walk_windows={} read_calls={} 单次峰值={} 累计={} full_walk={}",
                entry.name,
                entry.bytes,
                windows,
                walk_calls,
                stats.max_materialized_bytes,
                stats.total_materialized_bytes,
                full_walk
            );
            exercised.push((entry.name.clone(), entry.bytes, walk_calls));
        }
        assert!(
            !exercised.is_empty(),
            "pressure manifest 中必须至少有一个 pressure 条目"
        );
        eprintln!("pressure fixtures exercised: {exercised:?}");
    }

    #[test]
    fn utf8_boundary_drops_leading_continuation_bytes() {
        let source = test_support::fixture_source("utf8-boundary-probe.txt");
        let manifest = test_support::smoke_manifest();
        let entry = manifest.get("utf8-boundary-probe.txt");
        let char_start = entry
            .detail_u64("char_start")
            .expect("夹具必须声明 char_start");
        let char_len = entry.detail_u64("char_len").expect("夹具必须声明 char_len") as usize;
        assert_eq!(char_len, 3);

        let at_boundary = source.read_at(char_start, 32).expect("边界窗口");
        assert_eq!(at_boundary.dropped_leading, 0, "边界起点不得丢弃字节");
        assert!(at_boundary.as_str().expect("合法 UTF-8").starts_with('中'));

        let after_one = source.read_at(char_start + 1, 32).expect("字符中间窗口");
        assert_eq!(after_one.dropped_leading, char_len - 1);
        assert_eq!(after_one.start(), char_start + char_len as u64);
        assert!(
            after_one
                .as_str()
                .expect("合法 UTF-8")
                .starts_with("文测试"),
            "必须从下一个完整字符开始，实际 {:?}",
            after_one.as_str().expect("合法 UTF-8")
        );

        let after_two = source.read_at(char_start + 2, 32).expect("字符中间窗口");
        assert_eq!(after_two.dropped_leading, char_len - 2);
        assert_eq!(after_two.start(), char_start + char_len as u64);
        assert!(after_two
            .as_str()
            .expect("合法 UTF-8")
            .starts_with("文测试"));
    }

    #[test]
    fn utf8_boundary_trims_incomplete_trailing_character() {
        let source = test_support::fixture_source("utf8-boundary-probe.txt");
        let manifest = test_support::smoke_manifest();
        let entry = manifest.get("utf8-boundary-probe.txt");
        let char_start = entry
            .detail_u64("char_start")
            .expect("夹具必须声明 char_start");
        let char_len = entry.detail_u64("char_len").expect("夹具必须声明 char_len");

        // 窗口 = 4 个 ASCII 字节 + 3 字节汉字的 2 字节 → 尾部必须裁掉 2 字节。
        let offset = char_start - 4;
        let read = source.read_at(offset, 4 + 2).expect("窗口读取");
        assert_eq!(read.dropped_leading, 0);
        assert_eq!(read.trimmed_tail, char_len as usize - 1);
        assert_eq!(read.data.len(), 4);
        assert!(read.data.iter().all(|byte| *byte == b'a'));
        assert!(read.as_str().is_ok());
        assert_eq!(read.raw_bytes(), 6);
        assert_eq!(read.start(), offset);
        assert_eq!(read.end(), offset + 4);
        assert!(!read.eof);
    }

    #[test]
    fn utf8_boundary_reports_both_ends_at_once() {
        let source = test_support::fixture_source("utf8-boundary-probe.txt");
        let manifest = test_support::smoke_manifest();
        let entry = manifest.get("utf8-boundary-probe.txt");
        let char_start = entry
            .detail_u64("char_start")
            .expect("夹具必须声明 char_start");

        // [char_start+1, char_start+8)：2 个续接字节 + 「文」(3) + 「测」的前 2 字节。
        let read = source.read_at(char_start + 1, 7).expect("窗口读取");
        assert_eq!(read.dropped_leading, 2);
        assert_eq!(read.trimmed_tail, 2);
        assert_eq!(read.as_str().expect("合法 UTF-8"), "文");
        assert_eq!(read.raw_bytes(), 7);
        assert_eq!(read.end(), char_start + 6);
    }

    #[test]
    fn utf8_boundary_handles_four_byte_characters() {
        let source = test_support::fixture_source("utf8-four-byte.txt");
        // 夹具内容：64 × 'x' + 「𠀋」(4 字节) + 「🙂」(4 字节) + 「中」(3 字节) + '\n'
        let read = source.read_at(64 + 1, 3 + 4).expect("窗口读取");
        assert_eq!(read.dropped_leading, 3, "4 字节字符被截断时必须丢弃 3 字节");
        assert_eq!(read.trimmed_tail, 0);
        assert_eq!(read.as_str().expect("合法 UTF-8"), "🙂");

        let partial = source.read_at(64 + 4, 2).expect("窗口读取");
        assert_eq!(partial.dropped_leading, 0);
        assert_eq!(partial.trimmed_tail, 2, "不完整的 4 字节字符必须整段裁掉");
        assert!(partial.is_empty());
        assert_eq!(partial.raw_bytes(), 2);
    }

    #[test]
    fn utf8_boundary_truncated_tail_fixture_is_trimmed() {
        let source = test_support::fixture_source("utf8-truncated-tail.txt");
        let read = source.read_at(0, 64).expect("窗口读取");
        assert_eq!(read.trimmed_tail, 2, "文件结尾的半个字符必须裁掉");
        assert_eq!(read.as_str().expect("裁剪后必须是合法 UTF-8"), "截断尾部");
        assert_eq!(read.raw_bytes(), 14);
        assert!(read.eof);
    }

    #[test]
    fn utf8_boundary_ascii_windows_are_untouched() {
        let source = test_support::fixture_source("ascii-short.txt");
        let read = source.read_at(0, 8).expect("窗口读取");
        assert_eq!(read.dropped_leading, 0);
        assert_eq!(read.trimmed_tail, 0);
        assert_eq!(read.data, b"Hello no".to_vec());
        assert!(!read.eof);

        let empty = test_support::fixture_source("empty.txt");
        let read = empty.read_at(0, 8).expect("空文件读取必须合法");
        assert!(read.is_empty());
        assert!(read.eof);
        assert_eq!(empty.len(), 0);
        assert!(empty.is_empty());
    }

    #[test]
    fn utf8_boundary_helper_agrees_with_str_char_boundary() {
        let text = "第 1 章：中文测试🙂end";
        let bytes = text.as_bytes();
        for index in 0..=bytes.len() {
            assert_eq!(
                is_utf8_boundary(bytes, index),
                text.is_char_boundary(index),
                "index={index}"
            );
        }
        assert!(!is_utf8_boundary(bytes, 1), "「第」的第 2 字节不是边界");
        assert!(is_utf8_boundary(bytes, 3));
        assert!(is_utf8_boundary(bytes, bytes.len()));
        assert!(!is_utf8_boundary(bytes, bytes.len() + 1));
        assert!(is_utf8_boundary(b"", 0));

        let adjustment = align_to_utf8_boundaries(b"");
        assert_eq!(adjustment, BoundaryAdjustment::NONE);
        assert_eq!(adjustment.keep_range(0), 0..0);
        let both = BoundaryAdjustment {
            dropped_leading: 2,
            trimmed_tail: 3,
        };
        assert_eq!(both.keep_range(10), 2..7);
        // 极端裁剪不得产生逆序区间。
        let extreme = BoundaryAdjustment {
            dropped_leading: 3,
            trimmed_tail: 9,
        };
        assert_eq!(extreme.keep_range(4), 3..3);
    }

    #[test]
    fn utf8_boundary_windows_of_text_fixtures_are_valid_utf8() {
        let manifest = test_support::smoke_manifest();
        let mut checked = 0u32;
        for entry in &manifest.entries {
            if !matches!(
                entry.kind.as_str(),
                "text"
                    | "crlf"
                    | "long-line"
                    | "bom-utf8"
                    | "four-byte"
                    | "boundary-probe"
                    | "ascii"
            ) {
                continue;
            }
            let source = test_support::fixture_source(&entry.name);
            let mut offset = 0u64;
            while offset < source.len() {
                let read = source
                    .read_at(offset, DEFAULT_WINDOW_BYTES)
                    .expect("窗口读取");
                assert!(
                    read.as_str().is_ok(),
                    "夹具 {} 的窗口 [{}] 必须是合法 UTF-8",
                    entry.name,
                    offset
                );
                offset = read.end();
                if read.eof {
                    break;
                }
            }
            assert_eq!(offset, source.len());
            checked += 1;
        }
        assert!(checked >= 6, "至少覆盖 6 个文本夹具，实测 {checked}");
    }

    #[test]
    fn is_boundary_is_encoding_aware() {
        // UTF-8：沿用「非续接字节」规则（严格校验，行为不变）。
        let utf8 = "第一章 锚点".as_bytes();
        assert!(is_boundary(utf8, 0, TextEncoding::Utf8));
        assert!(!is_boundary(utf8, 1, TextEncoding::Utf8), "UTF-8 字符内部");
        assert!(is_boundary(utf8, 3, TextEncoding::Utf8));
        assert_eq!(
            is_boundary(utf8, 3, TextEncoding::Utf8),
            is_utf8_boundary(utf8, 3)
        );

        // UTF-16：码元对齐，奇数偏移不是边界。
        let utf16: &[u8] = &[0x2c, 0x7b, 0x2c, 0x7b];
        assert!(is_boundary(utf16, 0, TextEncoding::Utf16Le));
        assert!(!is_boundary(utf16, 1, TextEncoding::Utf16Le));
        assert!(is_boundary(utf16, 2, TextEncoding::Utf16Le));
        assert!(is_boundary(utf16, 4, TextEncoding::Utf16Be));

        // GB18030/GBK/Big5：**暂接受**（判定需要已知对齐基点，而窗口对齐仍是 UTF-8 专属；
        // 从错位窗口步进会误拒合法边界，真机表现为「打开即 ER_ERR_3 且无法翻页」）。
        // 恢复严格校验的前提是窗口对齐改为按内容编码（卡片 ENC-ALIGN）。
        let gbk: &[u8] = &[0xb5, 0xda, 0xd2, 0xbb, 0xd5, 0xc2];
        assert!(is_boundary(gbk, 2, TextEncoding::Gbk), "legacy 编码暂接受");
        assert!(!is_boundary(gbk, 99, TextEncoding::Gbk), "越界始终非法");
    }

    #[test]
    fn align_to_boundaries_follows_encoding() {
        // GBK「第一章」= b5 da d2 bb d5 c2，其中 b5/d2 落在 UTF-8 续接区间 0x80..=0xBF。
        let gbk: &[u8] = &[0xb5, 0xda, 0xd2, 0xbb, 0xd5, 0xc2];
        let utf8_rule = align_to_utf8_boundaries(gbk);
        assert!(
            utf8_rule.dropped_leading > 0,
            "旧的 UTF-8 对齐会误裁 GBK 首字节（真机「首行乱码」的成因）"
        );
        let gbk_rule = align_to_boundaries(gbk, 0, TextEncoding::Gbk);
        assert_eq!(gbk_rule.dropped_leading, 0, "按 GBK 对齐不得裁剪头部");
        assert_eq!(gbk_rule.trimmed_tail, 0);

        // 尾部不完整字符必须裁掉。
        let broken_tail: &[u8] = &[0xb5, 0xda, 0xd2];
        assert_eq!(
            align_to_boundaries(broken_tail, 0, TextEncoding::Gbk).trimmed_tail,
            1
        );

        // UTF-8 行为与旧实现完全一致（回归保证）。
        let utf8 = "第一章 锚点".as_bytes();
        assert_eq!(
            align_to_boundaries(utf8, 0, TextEncoding::Utf8).dropped_leading,
            align_to_utf8_boundaries(utf8).dropped_leading
        );

        // UTF-16：按**绝对偏移**对齐到码元边界。
        let utf16: &[u8] = &[0x2c, 0x7b, 0x2c, 0x7b];
        assert_eq!(
            align_to_boundaries(utf16, 1, TextEncoding::Utf16Le).dropped_leading,
            1,
            "绝对偏移为奇数时必须丢掉 1 字节使起点对齐码元"
        );
        assert_eq!(
            align_to_boundaries(utf16, 0, TextEncoding::Utf16Le).dropped_leading,
            0
        );
        assert_eq!(
            align_to_boundaries(&utf16[..3], 0, TextEncoding::Utf16Le).trimmed_tail,
            1,
            "落单字节必须裁掉"
        );
    }
}

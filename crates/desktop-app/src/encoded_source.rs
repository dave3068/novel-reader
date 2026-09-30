//! 编码感知的桌面文件数据源。
//!
//! ## 为什么需要它
//!
//! 冻结核心的 `FileTextSource::read_at` 一律按 **UTF-8** 对齐窗口
//! （`align_to_utf8_boundaries`：丢弃首部续接字节、裁掉尾部不完整字符）。
//! 对 GBK / GB18030 / Big5 这类编码，汉字首字节范围 `0x81..=0xFE` 与 UTF-8 续接字节
//! `0x80..=0xBF` **重叠** —— 于是「合法的 GB 字符首字节」会被当成续接字节**丢掉**，
//! 窗口起点随之前移，页起始偏移与核心的坐标空间不再一致（与真机同源的缺陷，
//! 见 `results/encoding-core-boundary.md` §1）。
//!
//! 本模块因此实现一个**按内容编码对齐**的桌面数据源：
//!
//! - UTF-8 / UTF-8 BOM：沿用核心的 [`align_to_utf8_boundaries`]（行为与既有一致）；
//! - UTF-16 LE/BE：按码元（2 字节）对齐；
//! - GB18030 / GBK / Big5：按**字符长度**步进（含 GB18030 四字节形式）裁掉尾部不完整字符；
//!   页起点由分页会话保证为合法边界，故首部不丢字节（避免误丢合法首字节）。
//!
//! 语义与 [`reading_source::TextSource`] 契约一致：有限窗口、`dropped_leading` /
//! `trimmed_tail` / `eof` 如实回报，绝不整本读取（单次物化 ≤ 窗口硬上限 256 KiB）。

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use reading_source::{
    align_to_utf8_boundaries, Fingerprint, ReadStats, SourceError, SourceMetadata, TextEncoding,
    TextSource, TextWindow, FINGERPRINT_SAMPLE_BYTES, MAX_WINDOW_BYTES,
};

/// 桌面端文件数据源（按内容编码对齐窗口）。
#[derive(Debug)]
pub struct EncodedFileSource {
    path: PathBuf,
    file: Mutex<File>,
    size: u64,
    modified: Option<std::time::SystemTime>,
    encoding: TextEncoding,
    read_calls: std::sync::atomic::AtomicU64,
    last_materialized_bytes: std::sync::atomic::AtomicU64,
    total_materialized_bytes: std::sync::atomic::AtomicU64,
    max_materialized_bytes: std::sync::atomic::AtomicU64,
}

impl EncodedFileSource {
    /// 打开文件并按给定内容编码对齐窗口。
    pub fn open(path: impl AsRef<Path>, encoding: TextEncoding) -> Result<Self, SourceError> {
        let path = path.as_ref().to_path_buf();
        let file = File::open(&path).map_err(|_| SourceError::IoError)?;
        let meta = file.metadata().map_err(|_| SourceError::IoError)?;
        Ok(Self {
            path,
            file: Mutex::new(file),
            size: meta.len(),
            modified: meta.modified().ok(),
            encoding,
            read_calls: std::sync::atomic::AtomicU64::new(0),
            last_materialized_bytes: std::sync::atomic::AtomicU64::new(0),
            total_materialized_bytes: std::sync::atomic::AtomicU64::new(0),
            max_materialized_bytes: std::sync::atomic::AtomicU64::new(0),
        })
    }

    /// 当前内容编码。
    pub fn encoding(&self) -> TextEncoding {
        self.encoding
    }

    /// 受限原始读取（不做边界对齐；调用方负责按编码对齐）。
    fn read_raw(&self, offset: u64, len: usize) -> Result<Vec<u8>, SourceError> {
        if len == 0 || len > MAX_WINDOW_BYTES {
            return Err(SourceError::InvalidArgument);
        }
        if offset > self.size {
            return Err(SourceError::OutOfRange);
        }
        let remain = self.size.saturating_sub(offset);
        let want = len.min(remain as usize);
        let mut buffer = vec![0u8; want];
        if want > 0 {
            let mut file = self.file.lock().map_err(|_| SourceError::Internal)?;
            file.seek(SeekFrom::Start(offset))
                .map_err(|_| SourceError::IoError)?;
            file.read_exact(&mut buffer)
                .map_err(|_| SourceError::IoError)?;
        }
        self.read_calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.last_materialized_bytes
            .store(want as u64, std::sync::atomic::Ordering::Relaxed);
        self.total_materialized_bytes
            .fetch_add(want as u64, std::sync::atomic::Ordering::Relaxed);
        self.max_materialized_bytes
            .fetch_max(want as u64, std::sync::atomic::Ordering::Relaxed);
        Ok(buffer)
    }
}

/// 一个字符在给定编码下的字节长度（与本仓核心 `legacy_char_len` 同一规则；
/// 核心实现不对外公开，此处按 [`reading_source::is_boundary`] 文档化的长度规则重写，口径一致）。
fn char_len_at(slice: &[u8], encoding: TextEncoding) -> usize {
    let Some(&lead) = slice.first() else {
        return 0;
    };
    match encoding {
        // UTF-16 必须先于 ASCII 早退判断（汉字码元低字节常 < 0x80）。
        TextEncoding::Utf16Le | TextEncoding::Utf16Be => {
            if slice.len() >= 4 && is_high_surrogate(lead, slice[1], encoding) {
                4
            } else {
                2
            }
        }
        _ if lead < 0x80 => 1,
        TextEncoding::Utf8 | TextEncoding::Utf8Bom => utf8_char_len(lead),
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

fn utf8_char_len(lead: u8) -> usize {
    if lead < 0x80 {
        1
    } else if lead >= 0xF0 {
        4
    } else if lead >= 0xE0 {
        3
    } else if lead >= 0xC0 {
        2
    } else {
        1
    }
}

fn is_high_surrogate(first: u8, second: u8, encoding: TextEncoding) -> bool {
    let unit = match encoding {
        TextEncoding::Utf16Le => u16::from_le_bytes([first, second]),
        _ => u16::from_be_bytes([first, second]),
    };
    (0xD800..=0xDBFF).contains(&unit)
}

/// 按内容编码计算窗口尾部应裁掉的字节数（尾部字符不完整时整段裁掉，下次读取会重新读到）。
fn trailing_incomplete_bytes(data: &[u8], encoding: TextEncoding) -> usize {
    let mut cursor = 0usize;
    while cursor < data.len() {
        let step = char_len_at(&data[cursor..], encoding);
        if step == 0 {
            break;
        }
        if cursor + step > data.len() {
            return data.len() - cursor;
        }
        cursor += step;
    }
    0
}

impl TextSource for EncodedFileSource {
    fn metadata(&self) -> SourceMetadata {
        SourceMetadata {
            size: self.size,
            modified: self.modified,
            display_name: self
                .path
                .file_name()
                .map(|name| name.to_string_lossy().to_string()),
            source_identity: self.path.to_string_lossy().to_string(),
        }
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<TextWindow, SourceError> {
        if len == 0 || len > MAX_WINDOW_BYTES {
            return Err(SourceError::InvalidArgument);
        }
        if offset > self.size {
            return Err(SourceError::OutOfRange);
        }
        let raw = self.read_raw(offset, len)?;
        let eof = offset + raw.len() as u64 >= self.size;
        let (dropped_leading, trimmed_tail) = match self.encoding {
            TextEncoding::Utf8 | TextEncoding::Utf8Bom => {
                let adjustment = align_to_utf8_boundaries(&raw);
                (adjustment.dropped_leading, adjustment.trimmed_tail)
            }
            TextEncoding::Utf16Le | TextEncoding::Utf16Be => {
                let dropped = if offset % 2 == 1 {
                    1usize.min(raw.len())
                } else {
                    0
                };
                let trailing = raw.len().saturating_sub(dropped) % 2;
                (dropped, trailing)
            }
            TextEncoding::Gb18030 | TextEncoding::Gbk | TextEncoding::Big5 => {
                (0, trailing_incomplete_bytes(&raw, self.encoding))
            }
        };
        let start = dropped_leading.min(raw.len());
        let end = raw.len().saturating_sub(trimmed_tail).max(start);
        Ok(TextWindow {
            offset,
            data: raw[start..end].to_vec(),
            dropped_leading,
            trimmed_tail,
            eof,
            requested_len: len,
        })
    }

    fn fingerprint(&self) -> Result<Fingerprint, SourceError> {
        let size = self.size;
        Fingerprint::compute(
            size,
            FINGERPRINT_SAMPLE_BYTES,
            self.modified,
            |offset, len| self.read_raw(offset, len),
        )
    }

    fn can_random_access(&self) -> bool {
        true
    }

    fn max_window_bytes(&self) -> usize {
        MAX_WINDOW_BYTES
    }

    fn stats(&self) -> ReadStats {
        use std::sync::atomic::Ordering::Relaxed;
        ReadStats {
            read_calls: self.read_calls.load(Relaxed),
            last_materialized_bytes: self.last_materialized_bytes.load(Relaxed),
            total_materialized_bytes: self.total_materialized_bytes.load(Relaxed),
            max_materialized_bytes: self.max_materialized_bytes.load(Relaxed),
        }
    }
}

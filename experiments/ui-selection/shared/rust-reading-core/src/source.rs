//! 有限窗口文本源：按原始字节偏移读取，绝不整体载入大文件（S-R2 / PNFR3）。

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::error::{io_error, out_of_range, NrResult};

/// 只读文本源。
pub trait TextSource {
    /// 总字节数。
    fn total_bytes(&self) -> u64;

    /// 读取 `[offset, offset+len)` 的原始字节（可能因到达 EOF 而不足）。
    fn read_at(&self, offset: u64, len: usize) -> NrResult<Vec<u8>>;

    /// 供结果记录的人类可读描述。
    fn describe(&self) -> String;
}

/// 已物化字节计数（用于证明内存不随文件规模线性增长）。
#[derive(Debug, Default)]
pub struct MaterializationGauge {
    bytes: AtomicU64,
    reads: AtomicU64,
}

impl MaterializationGauge {
    pub fn add(&self, bytes: usize) {
        self.bytes.fetch_add(bytes as u64, Ordering::Relaxed);
        self.reads.fetch_add(1, Ordering::Relaxed);
    }
    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }
    pub fn reads(&self) -> u64 {
        self.reads.load(Ordering::Relaxed)
    }
}

/// 外部内容（例如 Android SAF 选中的文件）的占位来源。
///
/// 共享库**不持有**这类内容的字节：只登记坐标空间长度，实际字节由平台侧
/// 按有限窗口提供（Android 用 SAF 读取后交给排版）。因此本地 `read_at`
/// 返回空字节而不是伪造数据，避免出现「看似读到了、其实不是原文」的假象。
#[derive(Debug, Clone)]
pub struct ExternalTextSource {
    total: u64,
    label: String,
}

impl ExternalTextSource {
    /// `label` 仅用于日志/结果记录。
    pub fn new(total_bytes: u64, label: impl Into<String>) -> Self {
        Self {
            total: total_bytes,
            label: label.into(),
        }
    }
}

impl TextSource for ExternalTextSource {
    fn total_bytes(&self) -> u64 {
        self.total
    }

    fn read_at(&self, offset: u64, _len: usize) -> NrResult<Vec<u8>> {
        if offset > self.total {
            return Err(out_of_range(format!(
                "偏移 {offset} 超出外部内容长度 {}",
                self.total
            )));
        }
        Ok(Vec::new())
    }

    fn describe(&self) -> String {
        format!("external:{}:{}bytes", self.label, self.total)
    }
}

/// 真实文件文本源（seek + 定长读取）。
#[derive(Debug)]
pub struct FileTextSource {
    file: std::sync::Mutex<File>,
    len: u64,
    gauge: Arc<MaterializationGauge>,
    path: String,
}

impl FileTextSource {
    pub fn open(path: impl AsRef<Path>) -> NrResult<Self> {
        let path_ref = path.as_ref();
        let file =
            File::open(path_ref).map_err(|e| io_error(format!("打开失败 {path_ref:?}: {e}")))?;
        let len = file
            .metadata()
            .map_err(|e| io_error(format!("读取元数据失败 {path_ref:?}: {e}")))?
            .len();
        Ok(Self {
            file: std::sync::Mutex::new(file),
            len,
            gauge: Arc::new(MaterializationGauge::default()),
            path: path_ref.display().to_string(),
        })
    }

    pub fn gauge(&self) -> Arc<MaterializationGauge> {
        Arc::clone(&self.gauge)
    }
}

impl TextSource for FileTextSource {
    fn total_bytes(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, len: usize) -> NrResult<Vec<u8>> {
        if offset > self.len {
            return Err(out_of_range(format!(
                "偏移 {offset} 超出文件长度 {}",
                self.len
            )));
        }
        let want = (len as u64).min(self.len - offset) as usize;
        if want == 0 {
            return Ok(Vec::new());
        }
        let mut file = self.file.lock().expect("文本源锁中毒");
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| io_error(format!("seek 失败: {e}")))?;
        let mut buf = vec![0u8; want];
        let read = file
            .read(&mut buf)
            .map_err(|e| io_error(format!("读取失败: {e}")))?;
        buf.truncate(read);
        self.gauge.add(buf.len());
        Ok(buf)
    }

    fn describe(&self) -> String {
        format!("file:{}", self.path)
    }
}

/// 合成文本源：由种子确定性地按偏移生成内容，1GB 级数据也不占内存。
///
/// 文件被切分为固定大小的块（默认 4KiB），块边界落在换行符上，
/// 因此任意偏移的读取只需生成覆盖该范围的块。
#[derive(Debug, Clone)]
pub struct SyntheticTextSource {
    total: u64,
    seed: u64,
    gauge: Arc<MaterializationGauge>,
}

impl SyntheticTextSource {
    /// 块大小：块边界必然落在 `\n` 上。
    pub const BLOCK: usize = 4096;

    pub fn new(total_bytes: u64, seed: u64) -> Self {
        Self {
            total: total_bytes,
            seed,
            gauge: Arc::new(MaterializationGauge::default()),
        }
    }

    pub fn gauge(&self) -> Arc<MaterializationGauge> {
        Arc::clone(&self.gauge)
    }

    fn block_count(&self) -> u64 {
        self.total.div_ceil(Self::BLOCK as u64)
    }

    /// 生成第 `index` 块（长度固定为 `BLOCK`，以 `\n` 补齐）。
    fn block(&self, index: u64) -> Vec<u8> {
        let mut rng = crate::rng::Rng::new(self.seed ^ index.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut out: Vec<u8> = Vec::with_capacity(Self::BLOCK);
        while out.len() < Self::BLOCK {
            let mut sentence = *rng.pick(&PARAGRAPH_SENTENCES);
            if out.len() + sentence.len() + 1 > Self::BLOCK {
                // 放不下长句时退化为最短句，尽量填满块内空间
                sentence = PARAGRAPH_SENTENCES[PARAGRAPH_SENTENCES.len() - 1];
                if out.len() + sentence.len() + 1 > Self::BLOCK {
                    break;
                }
            }
            out.extend_from_slice(sentence.as_bytes());
            out.push(b'\n');
        }
        while out.len() < Self::BLOCK {
            out.push(b'\n');
        }
        out
    }
}

/// 合成正文的句子池：使用连贯的公有领域中文语句，避免真机上出现「随机字符拼接」的观感。
const PARAGRAPH_SENTENCES: [&str; 12] = [
    "床前明月光，疑是地上霜。举头望明月，低头思故乡。",
    "春眠不觉晓，处处闻啼鸟。夜来风雨声，花落知多少。",
    "白日依山尽，黄河入海流。欲穷千里目，更上一层楼。",
    "红豆生南国，春来发几枝。愿君多采撷，此物最相思。",
    "空山不见人，但闻人语响。返景入深林，复照青苔上。",
    "独在异乡为异客，每逢佳节倍思亲。遥知兄弟登高处，遍插茱萸少一人。",
    "海上生明月，天涯共此时。情人怨遥夜，竟夕起相思。",
    "山重水复疑无路，柳暗花明又一村。",
    "会当凌绝顶，一览众山小。",
    "大漠孤烟直，长河落日圆。",
    "读书破万卷，下笔如有神。",
    "千里之行，始于足下。",
];

impl TextSource for SyntheticTextSource {
    fn total_bytes(&self) -> u64 {
        self.total
    }

    fn read_at(&self, offset: u64, len: usize) -> NrResult<Vec<u8>> {
        if offset > self.total {
            return Err(out_of_range(format!(
                "偏移 {offset} 超出合成源长度 {}",
                self.total
            )));
        }
        if len == 0 || offset == self.total {
            return Ok(Vec::new());
        }
        let end = (offset + len as u64).min(self.total);
        let first = offset / Self::BLOCK as u64;
        let last = (end - 1) / Self::BLOCK as u64;
        let mut acc: Vec<u8> = Vec::with_capacity(((last - first + 1) as usize) * Self::BLOCK);
        for b in first..=last {
            acc.extend_from_slice(&self.block(b));
        }
        self.gauge.add(acc.len());
        let base = first * Self::BLOCK as u64;
        let from = (offset - base) as usize;
        let to = (end - base) as usize;
        let to = to.min(acc.len());
        if from >= to {
            return Ok(Vec::new());
        }
        Ok(acc[from..to].to_vec())
    }

    fn describe(&self) -> String {
        format!(
            "synthetic:{}bytes:seed{}:blocks{}",
            self.total,
            self.seed,
            self.block_count()
        )
    }
}

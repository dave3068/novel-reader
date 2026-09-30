//! 文件指纹：大小 + 头/中/尾抽样哈希 + 可得的修改时间。
//!
//! 依据 `docs/novel-reader-v1-development-plan.md` §6.3：来源指纹 = 文件大小 +
//! 头部/中部/尾部有限抽样哈希 + 来源标识 + 可获得的修改时间提示。指纹只用于提示
//! “可能是原书”，**不得**当作安全身份认证。
//!
//! 本模块的硬约束（卡片 rc2 禁止事项）：
//!
//! - 抽样读取的总字节数有上限（默认 3 × 4 KiB），任何情况都不整本读取、不整本哈希；
//! - 哈希作用于**原始字节**，不做编码转换或校验（编码识别/覆盖是 RC3 的职责），
//!   因此指纹与文件编码无关；
//! - 哈希算法为 FNV-1a 64（非加密哈希，仅用于变更检测）；算法标识写入指纹结果，
//!   便于将来替换算法时识别历史值。

use std::time::{SystemTime, UNIX_EPOCH};

use crate::source::SourceError;

/// 单次抽样字节数（头部、中部、尾部各一次）。
pub const FINGERPRINT_SAMPLE_BYTES: usize = 4096;

/// 抽样次数：头部、中部、尾部。
pub const FINGERPRINT_SAMPLE_COUNT: usize = 3;

/// 指纹哈希算法标识。
pub const FINGERPRINT_ALGO: &str = "fnv1a64";

/// FNV-1a 64 位初始值。
pub const FNV1A64_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a 64 位乘法因子。
pub const FNV1A64_PRIME: u64 = 0x0000_0100_0000_01b3;

/// 用 FNV-1a 64 哈希字节序列（默认种子）。
pub fn hash_bytes(data: &[u8]) -> u64 {
    hash_bytes_seeded(FNV1A64_OFFSET_BASIS, data)
}

/// 用 FNV-1a 64 哈希字节序列，允许指定种子（便于同一算法下生成多路摘要）。
pub fn hash_bytes_seeded(seed: u64, data: &[u8]) -> u64 {
    let mut hash = seed;
    for byte in data {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV1A64_PRIME);
    }
    hash
}

/// 来源指纹快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    /// 文件字节长度。
    pub size: u64,
    /// 头部抽样哈希。
    pub head_hash: u64,
    /// 中部抽样哈希。
    pub mid_hash: u64,
    /// 尾部抽样哈希。
    pub tail_hash: u64,
    /// 单次抽样字节数。
    pub sample_len: usize,
    /// 本次指纹实际读取的字节总数（≤ 3 × `sample_len`）。
    pub sampled_bytes: u64,
    /// 可得的修改时间（不可得时为 `None`，例如平台不提供或权限受限）。
    pub modified: Option<SystemTime>,
    /// 哈希算法标识。
    pub algo: &'static str,
}

impl Fingerprint {
    /// 计算头/中/尾抽样区间 `(offset, len)`。
    ///
    /// - 空文件或 `sample_len == 0`：三个区间均为 `(0, 0)`；
    /// - 文件不大于单次抽样：三个区间都退化为整个文件（不放大读取量）；
    /// - 大文件：头部 `[0, sample)`、中部以文件中心为中心、尾部 `[size - sample, size)`，
    ///   三者均在文件范围内，且不重复放大总读取量。
    pub fn sampled_ranges(
        size: u64,
        sample_len: usize,
    ) -> [(u64, usize); FINGERPRINT_SAMPLE_COUNT] {
        if size == 0 || sample_len == 0 {
            return [(0, 0); FINGERPRINT_SAMPLE_COUNT];
        }
        let sample = sample_len as u64;
        let edge = sample.min(size);
        let head = (0u64, edge as usize);
        let tail = (size - edge, edge as usize);
        let mid = if size <= sample {
            (0u64, size as usize)
        } else {
            let start = (size / 2).saturating_sub(sample / 2);
            let available = size - start;
            (start, sample.min(available) as usize)
        };
        [head, mid, tail]
    }

    /// 抽样读取字节数的硬上限。
    pub fn sample_bytes_bound(sample_len: usize) -> u64 {
        (sample_len as u64) * (FINGERPRINT_SAMPLE_COUNT as u64)
    }

    /// 通过受控读取回调计算指纹。
    ///
    /// `read` 必须按 `(offset, len)` 返回**至多** `len` 字节的原始数据；本函数不会请求
    /// 超过 `sample_len` 的区间，因此底层数据源不会因指纹计算而整本读取。
    pub fn compute<F>(
        size: u64,
        sample_len: usize,
        modified: Option<SystemTime>,
        mut read: F,
    ) -> Result<Self, SourceError>
    where
        F: FnMut(u64, usize) -> Result<Vec<u8>, SourceError>,
    {
        let ranges = Self::sampled_ranges(size, sample_len);
        let mut hashes = [FNV1A64_OFFSET_BASIS; FINGERPRINT_SAMPLE_COUNT];
        let mut sampled_bytes = 0u64;
        for (index, (offset, len)) in ranges.iter().enumerate() {
            if *len == 0 {
                continue;
            }
            let bytes = read(*offset, *len)?;
            if bytes.len() > *len {
                return Err(SourceError::Internal);
            }
            sampled_bytes += bytes.len() as u64;
            hashes[index] = hash_bytes(&bytes);
        }
        let [head_hash, mid_hash, tail_hash] = hashes;
        Ok(Self {
            size,
            head_hash,
            mid_hash,
            tail_hash,
            sample_len,
            sampled_bytes,
            modified,
            algo: FINGERPRINT_ALGO,
        })
    }

    /// 只包含内容事实的指纹签名（不含修改时间）。
    ///
    /// 修改时间可能因复制、同步而改变但内容不变，因此“是否同一份内容”只比较该签名。
    pub fn content_signature(&self) -> (u64, u64, u64, u64) {
        (self.size, self.head_hash, self.mid_hash, self.tail_hash)
    }

    /// 内容签名是否一致（忽略修改时间）。
    pub fn same_content(&self, other: &Self) -> bool {
        self.content_signature() == other.content_signature()
    }

    /// 修改时间（Unix 毫秒）；不可得或早于纪元时为 `None`。
    pub fn modified_unix_ms(&self) -> Option<u128> {
        self.modified?
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_millis())
    }

    /// 稳定的文本表示，便于日志与证据记录。
    pub fn describe(&self) -> String {
        format!(
            "{} size={} head={:016x} mid={:016x} tail={:016x} sampled={} sample_len={} mtime_ms={:?}",
            self.algo,
            self.size,
            self.head_hash,
            self.mid_hash,
            self.tail_hash,
            self.sampled_bytes,
            self.sample_len,
            self.modified_unix_ms(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;
    use crate::{FileTextSource, TextSource, MAX_WINDOW_BYTES};

    #[test]
    fn fingerprint_hash_is_deterministic_and_seedable() {
        // 期望值与独立实现（Python，见 results/rc2-source.md）逐位一致，属跨语言对照。
        assert_eq!(hash_bytes(b""), FNV1A64_OFFSET_BASIS);
        assert_eq!(hash_bytes(b"abc"), 0xe71f_a219_0541_574b);
        assert_eq!(hash_bytes(b"novel-reader"), 0x4802_8973_1953_9f03);
        assert_eq!(hash_bytes("中".as_bytes()), 0x1e59_811b_678a_02d4);
        assert_ne!(hash_bytes(b"abc"), hash_bytes(b"abd"));
        assert_eq!(hash_bytes_seeded(7, b"abc"), hash_bytes_seeded(7, b"abc"));
        assert_ne!(hash_bytes_seeded(7, b"abc"), hash_bytes_seeded(8, b"abc"));
    }

    #[test]
    fn fingerprint_is_stable_for_unchanged_file() {
        let source = test_support::fixture_source("utf8-mixed-lines.txt");
        let first = source.fingerprint().expect("计算指纹");
        let second = source.fingerprint().expect("再次计算指纹");
        assert_eq!(first, second, "未变更文件的指纹必须稳定");
        assert_eq!(first.describe(), second.describe());
        assert_eq!(first.size, source.len());
        assert_eq!(first.algo, FINGERPRINT_ALGO);
        assert_eq!(first.sample_len, FINGERPRINT_SAMPLE_BYTES);
        assert!(first.same_content(&second));
        assert!(first.modified.is_some(), "桌面路径必须能取到修改时间");
    }

    #[test]
    fn fingerprint_detects_same_size_content_change() {
        let temp = test_support::temp_dir("fingerprint-same-size-edit");
        let payload = vec![b'a'; 64 * 1024 + 7];
        let path = temp.write("book.txt", &payload);
        let before = FileTextSource::open(&path)
            .expect("打开临时文件")
            .fingerprint()
            .expect("计算指纹");

        let mut edited = payload.clone();
        let middle = edited.len() / 2;
        edited[0] = b'b';
        edited[middle] = b'b';
        if let Some(last) = edited.last_mut() {
            *last = b'b';
        }
        std::fs::write(&path, &edited).expect("重写同尺寸内容");
        let after = FileTextSource::open(&path)
            .expect("重新打开")
            .fingerprint()
            .expect("计算指纹");

        assert_eq!(before.size, after.size, "本用例尺寸不变");
        assert_ne!(before.head_hash, after.head_hash, "头部变化必须被检出");
        assert_ne!(before.mid_hash, after.mid_hash, "中部变化必须被检出");
        assert_ne!(before.tail_hash, after.tail_hash, "尾部变化必须被检出");
        assert!(!before.same_content(&after));
    }

    #[test]
    fn fingerprint_detects_size_change() {
        let temp = test_support::temp_dir("fingerprint-size-change");
        let path = temp.write("book.txt", b"hello novel reader\n");
        assert!(path.starts_with(temp.path()), "写入路径必须位于临时目录内");
        let before = FileTextSource::open(&path)
            .expect("打开临时文件")
            .fingerprint()
            .expect("计算指纹");
        std::fs::write(&path, b"hello novel reader\nappended\n").expect("追加内容");
        let after = FileTextSource::open(&path)
            .expect("重新打开")
            .fingerprint()
            .expect("计算指纹");
        assert_ne!(before.size, after.size);
        assert!(!before.same_content(&after));
    }

    #[test]
    fn fingerprint_of_empty_file_is_defined() {
        let temp = test_support::temp_dir("fingerprint-empty");
        let path = temp.write("empty.txt", b"");
        let fingerprint = FileTextSource::open(&path)
            .expect("打开空文件")
            .fingerprint()
            .expect("计算指纹");
        assert_eq!(fingerprint.size, 0);
        assert_eq!(fingerprint.sampled_bytes, 0);
        assert_eq!(fingerprint.head_hash, FNV1A64_OFFSET_BASIS);
        assert_eq!(fingerprint.mid_hash, FNV1A64_OFFSET_BASIS);
        assert_eq!(fingerprint.tail_hash, FNV1A64_OFFSET_BASIS);
    }

    #[test]
    fn fingerprint_covers_head_mid_tail_ranges() {
        let size = 1u64 << 20;
        let sample = FINGERPRINT_SAMPLE_BYTES as u64;
        let ranges = Fingerprint::sampled_ranges(size, FINGERPRINT_SAMPLE_BYTES);
        assert_eq!(ranges[0], (0, FINGERPRINT_SAMPLE_BYTES), "头部抽样");
        assert_eq!(
            ranges[2],
            (size - sample, FINGERPRINT_SAMPLE_BYTES),
            "尾部抽样"
        );
        assert_eq!(
            ranges[1],
            (size / 2 - sample / 2, FINGERPRINT_SAMPLE_BYTES),
            "中部抽样"
        );
        for (offset, len) in ranges {
            assert!(
                offset + len as u64 <= size,
                "抽样区间必须落在文件内: offset={offset} len={len}"
            );
        }

        let small = Fingerprint::sampled_ranges(10, FINGERPRINT_SAMPLE_BYTES);
        assert_eq!(
            small,
            [(0, 10); FINGERPRINT_SAMPLE_COUNT],
            "小文件退化为整文件"
        );
        let empty = Fingerprint::sampled_ranges(0, FINGERPRINT_SAMPLE_BYTES);
        assert_eq!(empty, [(0, 0); FINGERPRINT_SAMPLE_COUNT], "空文件不读取");
    }

    #[test]
    fn fingerprint_sampling_is_bounded() {
        let source = test_support::fixture_source("utf8-long-line.txt");
        let before = source.stats();
        let fingerprint = source.fingerprint().expect("计算指纹");
        let stats = source.stats();
        let bound = Fingerprint::sample_bytes_bound(FINGERPRINT_SAMPLE_BYTES);

        assert_eq!(fingerprint.size, source.len());
        assert!(
            stats.last_materialized_bytes <= FINGERPRINT_SAMPLE_BYTES as u64,
            "单次抽样物化字节必须 ≤ {}，实测 {}",
            FINGERPRINT_SAMPLE_BYTES,
            stats.last_materialized_bytes
        );
        assert!(stats.max_materialized_bytes <= MAX_WINDOW_BYTES as u64);
        assert_eq!(
            fingerprint.sampled_bytes,
            stats.total_materialized_bytes - before.total_materialized_bytes,
            "指纹抽样字节必须与实际读取字节一致"
        );
        assert!(
            fingerprint.sampled_bytes <= bound,
            "指纹抽样总量必须 ≤ {bound}，实测 {}",
            fingerprint.sampled_bytes
        );
        assert_eq!(
            stats.read_calls,
            before.read_calls + FINGERPRINT_SAMPLE_COUNT as u64
        );

        // 若存在压力档夹具，额外验证 GB 级文件上的抽样仍然有界。
        if let Some((dir, manifest)) = test_support::pressure_manifest() {
            let big = manifest
                .entries
                .iter()
                .filter(|entry| entry.kind == "pressure" && entry.bytes >= 1 << 30)
                .map(|entry| (dir.join(&entry.name), entry))
                .find(|(path, _)| path.is_file());
            if let Some((path, entry)) = big {
                let source = FileTextSource::open(&path).expect("打开 1GB 压力夹具");
                let before = source.stats();
                let fingerprint = source.fingerprint().expect("计算指纹");
                let stats = source.stats();
                assert_eq!(fingerprint.size, entry.bytes);
                assert!(
                    fingerprint.sampled_bytes <= bound,
                    "GB 级文件指纹抽样必须 ≤ {bound}，实测 {}",
                    fingerprint.sampled_bytes
                );
                assert_eq!(
                    stats.total_materialized_bytes - before.total_materialized_bytes,
                    fingerprint.sampled_bytes
                );
                println!(
                    "1GB 夹具指纹抽样字节 = {}（文件 {} 字节）",
                    fingerprint.sampled_bytes, entry.bytes
                );
            }
        }
    }

    #[test]
    fn fingerprint_matches_fixture_manifest_sizes() {
        let manifest = test_support::smoke_manifest();
        assert!(manifest.entries.len() >= 8, "smoke 夹具数量不足");
        let mut checked = 0u32;
        for entry in &manifest.entries {
            let source = test_support::fixture_source(&entry.name);
            assert_eq!(
                source.len(),
                entry.bytes,
                "夹具 {} 大小必须与 manifest 一致",
                entry.name
            );
            let fingerprint = source.fingerprint().expect("计算指纹");
            assert_eq!(fingerprint.size, entry.bytes);
            checked += 1;
        }
        assert_eq!(checked as usize, manifest.entries.len());
    }

    #[test]
    fn fingerprint_includes_modified_time_when_available() {
        let path = test_support::fixture("ascii-short.txt");
        let fingerprint = test_support::fixture_source("ascii-short.txt")
            .fingerprint()
            .expect("计算指纹");
        let metadata = std::fs::metadata(&path).expect("读取元数据");
        assert_eq!(fingerprint.modified, metadata.modified().ok());
        let expected_ms = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis());
        assert!(fingerprint.modified_unix_ms().is_some());
        assert_eq!(fingerprint.modified_unix_ms(), expected_ms);
        assert!(fingerprint.describe().contains("fnv1a64"));
    }
}

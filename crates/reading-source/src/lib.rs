//! `reading-source`：TXT 有限窗口数据源、编码识别与文件指纹。
//!
//! 本 crate 是 reading-core 的**下层**依赖：只提供“按原始字节偏移读取有限窗口”和
//! “来源指纹”能力，不认识章节、位置、分页或 UI 语义，也不依赖 reading-core
//! （避免依赖环；错误码以同数值的本地枚举 [`SourceError`] 暴露）。
//!
//! # 边界（见 `scripts/verify-core-scope.py`）
//!
//! - 不依赖 UI/Android/网络/窗口系统；
//! - 不引用 `experiments/` 下的验证探针（合成文本源、模拟书库、空 External 数据源）；
//! - 依赖精确锁定版本。
//!
//! # 本卡（rc2）已落地
//!
//! - [`TextSource`] 抽象与 [`FileTextSource`] 文件实现：`read_at(offset, len)` 定长读取，
//!   默认窗口 [`DEFAULT_WINDOW_BYTES`]（64 KiB）、硬上限 [`MAX_WINDOW_BYTES`]（256 KiB），
//!   超过硬上限直接拒绝，单次物化字节不超过硬上限；
//! - 窗口两端自动修正到合法 UTF-8 字符边界（[`align_to_utf8_boundaries`]），
//!   并回报 `dropped_leading` / `trimmed_tail` / `eof`；
//! - 文件指纹 [`Fingerprint`]：大小 + 头/中/尾抽样哈希 + 可得的修改时间；
//! - 夹具生成器 `scripts/generate-fixtures.py`（`smoke` / `pressure` 两档，确定性 seed，
//!   输出 `manifest.json` / `manifest.tsv`）。
//!
//! 尚未落地（后续卡片）：编码识别与手动覆盖（RC3）、章节/位置/分页（RC4）、SQLite（RC5）、
//! C ABI/JNI（RC7）。

pub mod encoding;
pub mod fingerprint;
pub mod source;

pub use encoding::{
    char_is_text_plausible, decodes_equivalently, detect_bom, detect_encoding,
    detect_encoding_from_samples, score_legacy_candidates, ConfidenceLevel, EncodingCandidate,
    EncodingDetection, EncodingFamily, EncodingState, IndexInvalidation, TextEncoding,
    DETECT_HEAD_BYTES, DETECT_MAX_SAMPLED_BYTES, DETECT_SAMPLE_BYTES, ENCODING_DETECTOR_VERSION,
    LEGACY_MARGIN_HIGH, LEGACY_MARGIN_MEDIUM, LEGACY_SCORE_HIGH_MIN,
    LEGACY_STRUCTURAL_MIN_PERMILLE, UTF16_SUSPICIOUS_MAX_PERMILLE, UTF8_HIGH_MIN_SAMPLED_BYTES,
    UTF8_LOW_MAX_SAMPLED_BYTES, UTF8_SUSPICIOUS_MAX_PERMILLE,
};
pub use fingerprint::{
    hash_bytes, hash_bytes_seeded, Fingerprint, FINGERPRINT_ALGO, FINGERPRINT_SAMPLE_BYTES,
    FINGERPRINT_SAMPLE_COUNT, FNV1A64_OFFSET_BASIS, FNV1A64_PRIME,
};
pub use source::{
    align_to_utf8_boundaries, is_utf8_boundary, BoundaryAdjustment, FileTextSource, ReadStats,
    SourceError, SourceMetadata, TextSource, TextWindow,
};

/// 单次读取的默认窗口：64 KiB（gate-0 §2.3 冻结值）。
pub const DEFAULT_WINDOW_BYTES: usize = 64 * 1024;

/// 单次读取的硬上限：256 KiB（gate-0 §2.3 冻结值）。
///
/// 任一次读取实际物化字节都必须 ≤ 该值；请求长度超过该值直接拒绝。
pub const MAX_WINDOW_BYTES: usize = 256 * 1024;

/// 仅供测试使用的夹具与 manifest 辅助（无第三方 JSON 依赖，直接解析 `manifest.tsv`）。
#[cfg(test)]
pub(crate) mod test_support {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::FileTextSource;

    /// `manifest.tsv` 中的一行夹具记录。
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ManifestEntry {
        pub name: String,
        pub bytes: u64,
        pub sha256: String,
        pub kind: String,
        pub details: String,
    }

    impl ManifestEntry {
        /// 读取 `key=value` 形式的细节字段。
        pub fn detail(&self, key: &str) -> Option<&str> {
            self.details.split(';').find_map(|pair| {
                let (candidate, value) = pair.split_once('=')?;
                (candidate == key).then_some(value)
            })
        }

        /// 读取数字型细节字段。
        pub fn detail_u64(&self, key: &str) -> Option<u64> {
            self.detail(key).and_then(|value| value.parse().ok())
        }
    }

    /// 解析后的 manifest：元信息 + 夹具记录。
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct Manifest {
        pub meta: Vec<(String, String)>,
        pub entries: Vec<ManifestEntry>,
    }

    impl Manifest {
        /// 读取元信息键值。
        pub fn meta_get(&self, key: &str) -> Option<&str> {
            self.meta
                .iter()
                .find(|(candidate, _)| candidate == key)
                .map(|(_, value)| value.as_str())
        }

        /// 读取数字型元信息。
        pub fn meta_u64(&self, key: &str) -> Option<u64> {
            self.meta_get(key).and_then(|value| value.parse().ok())
        }

        /// 按名称取夹具记录（缺失时给出清晰断言信息）。
        pub fn get(&self, name: &str) -> &ManifestEntry {
            self.entries
                .iter()
                .find(|entry| entry.name == name)
                .unwrap_or_else(|| panic!("manifest 中缺少夹具 {name}"))
        }
    }

    /// 仓库内小型夹具目录：`tests/fixtures/`。
    pub fn fixtures_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("tests")
            .join("fixtures")
    }

    /// 夹具绝对路径。
    pub fn fixture(name: &str) -> PathBuf {
        fixtures_dir().join(name)
    }

    /// 以 [`FileTextSource`] 打开仓库夹具。
    pub fn fixture_source(name: &str) -> FileTextSource {
        FileTextSource::open(fixture(name))
            .unwrap_or_else(|error| panic!("打开夹具 {name} 失败: {error}"))
    }

    /// 解析目录下的 `manifest.tsv`。
    pub fn read_manifest(dir: &Path) -> Result<Manifest, String> {
        let path = dir.join("manifest.tsv");
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("读取 {} 失败: {error}", path.display()))?;
        let mut manifest = Manifest::default();
        for line in text.lines() {
            if let Some(comment) = line.strip_prefix("# ") {
                for token in comment.split_whitespace() {
                    if let Some((key, value)) = token.split_once('=') {
                        manifest.meta.push((key.to_string(), value.to_string()));
                    }
                }
                continue;
            }
            if line.trim().is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() < 5 {
                return Err(format!("manifest 行字段不足 5 列: {line:?}"));
            }
            let bytes = fields[1]
                .parse::<u64>()
                .map_err(|error| format!("解析字节数失败 {line:?}: {error}"))?;
            manifest.entries.push(ManifestEntry {
                name: fields[0].to_string(),
                bytes,
                sha256: fields[2].to_string(),
                kind: fields[3].to_string(),
                details: fields[4].to_string(),
            });
        }
        Ok(manifest)
    }

    /// 仓库内 smoke 夹具 manifest（由 `scripts/generate-fixtures.py` 生成）。
    pub fn smoke_manifest() -> Manifest {
        read_manifest(&fixtures_dir()).unwrap_or_else(|error| {
            panic!("tests/fixtures/manifest.tsv 不可用（请先运行夹具生成器）: {error}")
        })
    }

    /// 压力档夹具候选目录（环境变量优先，其次常用临时目录）。
    pub fn pressure_dirs() -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        for key in ["NR_PRESSURE_DIR", "NR_FIXTURE_DIR"] {
            if let Some(value) = std::env::var_os(key) {
                dirs.push(PathBuf::from(value));
            }
        }
        for candidate in ["/tmp/nr-pressure", "/tmp/nr-core-fixtures"] {
            dirs.push(PathBuf::from(candidate));
        }
        dirs.push(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("tests")
                .join("generated"),
        );
        dirs.push(fixtures_dir().join("pressure"));
        dirs
    }

    /// 找到可用的压力档夹具目录（manifest 中至少有一个存在的 `pressure` 条目）。
    pub fn pressure_manifest() -> Option<(PathBuf, Manifest)> {
        for dir in pressure_dirs() {
            let Ok(manifest) = read_manifest(&dir) else {
                continue;
            };
            let available = manifest.entries.iter().any(|entry| {
                entry.kind == "pressure"
                    && !entry.name.contains('/')
                    && dir.join(&entry.name).is_file()
            });
            if available {
                return Some((dir, manifest));
            }
        }
        None
    }

    /// 自动清理的临时目录。
    #[derive(Debug)]
    pub struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        /// 目录路径。
        pub fn path(&self) -> &Path {
            &self.path
        }

        /// 在目录内写入文件并返回路径。
        pub fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.path.join(name);
            fs::write(&path, bytes)
                .unwrap_or_else(|error| panic!("写入 {} 失败: {error}", path.display()));
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// 创建本次测试专属的临时目录（测试结束自动清理，不长期驻留大文件）。
    pub fn temp_dir(label: &str) -> TempDir {
        let unique = format!(
            "nr-reading-source-{}-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed),
            label
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir_all(&path)
            .unwrap_or_else(|error| panic!("创建 {} 失败: {error}", path.display()));
        TempDir { path }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_window_contract_matches_frozen_baseline() {
        // 冻结依据：results/gates/gate-0-baseline.md §2.3（父级 NFR1 / reading-core CR1 / AC1）。
        let default_window: usize = DEFAULT_WINDOW_BYTES;
        let max_window: usize = MAX_WINDOW_BYTES;
        assert_eq!(default_window, 65_536);
        assert_eq!(max_window, 262_144);
        assert_eq!(default_window, 64 * 1024);
        assert_eq!(max_window, 256 * 1024);
        assert!(default_window < max_window, "默认窗口必须小于硬上限");
        assert_eq!(
            max_window % default_window,
            0,
            "硬上限必须是默认窗口的整数倍"
        );
    }

    #[test]
    fn bounded_window_contract_matches_fixture_manifest() {
        // 跨语言对照：夹具生成脚本写入 manifest 的常量必须与 Rust 冻结常量一致。
        let manifest = test_support::smoke_manifest();
        let default_window = manifest
            .meta_u64("default_window_bytes")
            .expect("manifest 必须声明 default_window_bytes");
        let max_window = manifest
            .meta_u64("max_window_bytes")
            .expect("manifest 必须声明 max_window_bytes");
        let sample_bytes = manifest
            .meta_u64("fingerprint_sample_bytes")
            .expect("manifest 必须声明 fingerprint_sample_bytes");
        assert_eq!(default_window, DEFAULT_WINDOW_BYTES as u64);
        assert_eq!(max_window, MAX_WINDOW_BYTES as u64);
        assert_eq!(sample_bytes, FINGERPRINT_SAMPLE_BYTES as u64);
        assert_eq!(manifest.meta_get("profile"), Some("smoke"));
        assert_eq!(manifest.meta_get("seed"), Some("20260912"));
        assert!(!manifest.entries.is_empty());
    }

    #[test]
    fn bounded_window_error_codes_match_reading_core_contract() {
        // 数值来自 results/gates/gate-0-baseline.md §2.2（1/2/3/6/10）。
        assert_eq!(SourceError::InvalidArgument.code(), 1);
        assert_eq!(SourceError::OutOfRange.code(), 2);
        assert_eq!(SourceError::Utf8Boundary.code(), 3);
        assert_eq!(SourceError::IoError.code(), 6);
        assert_eq!(SourceError::Internal.code(), 10);
        assert_eq!(SourceError::InvalidArgument.as_str(), "InvalidArgument");
        assert_eq!(SourceError::IoError.to_string(), "IoError");

        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        assert_eq!(SourceError::from(io_error), SourceError::IoError);
        let as_std_error: &dyn std::error::Error = &SourceError::OutOfRange;
        assert_eq!(as_std_error.to_string(), "OutOfRange");
    }

    #[test]
    fn bounded_window_boundary_helpers_are_exported() {
        // 供后续卡片（RC4 分页边界）复用的 UTF-8 边界工具。
        let text = "中文abc";
        let bytes = text.as_bytes();
        assert!(is_utf8_boundary(bytes, 0));
        assert!(is_utf8_boundary(bytes, 3));
        assert!(!is_utf8_boundary(bytes, 1));
        let adjustment = align_to_utf8_boundaries(bytes);
        assert_eq!(adjustment, BoundaryAdjustment::NONE);
        assert_eq!(adjustment.keep_range(bytes.len()), 0..bytes.len());
        let window = TextWindow {
            offset: 0,
            data: b"abc".to_vec(),
            dropped_leading: 0,
            trimmed_tail: 0,
            eof: true,
            requested_len: 3,
        };
        assert_eq!(window.as_str().expect("合法 UTF-8"), "abc");
        assert_eq!(window.end(), 3);
        assert_eq!(window.raw_bytes(), 3);
        assert_eq!(ReadStats::default().read_calls, 0);
    }
}
pub use crate::encoding::decode_line;
pub use crate::source::align_to_boundaries;
pub use crate::source::is_boundary;

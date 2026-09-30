//! RC3 编码识别与手动覆盖的集成测试（夹具驱动）。
//!
//! 夹具与期望值来自 `tests/fixtures/encoding/manifest.tsv`，该文件由
//! `crates/reading-source/tests/generate-encoding-fixtures.py` 生成；Rust 侧独立复算
//! 每份夹具的 sha256（本文件内的纯 Rust SHA-256，带标准已知向量自检）并与 manifest
//! 中 Python `hashlib` 的结果对照，形成跨语言证据链。
//!
//! 覆盖范围：七类编码（UTF-8 / UTF-8-BOM / UTF-16LE / UTF-16BE / GB18030 / GBK / Big5）
//! 各有断言；低可信度场景可覆盖；切换编码前后原文件 sha256 不变；检测只读有限窗口。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use reading_source::{
    detect_encoding, ConfidenceLevel, EncodingState, FileTextSource, TextEncoding, TextSource,
    DETECT_HEAD_BYTES, DETECT_MAX_SAMPLED_BYTES, DETECT_SAMPLE_BYTES, ENCODING_DETECTOR_VERSION,
};

// --------------------------------------------------------------------------------------
// SHA-256（纯 Rust，仅测试使用；带已知向量自检，并与 Python hashlib 的 manifest 对照）
// --------------------------------------------------------------------------------------

mod sha256 {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    const H0: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    /// 计算 SHA-256 摘要（标准 FIPS 180-4 实现）。
    pub fn digest(data: &[u8]) -> [u8; 32] {
        let mut h = H0;
        let bit_len = (data.len() as u64).wrapping_mul(8);
        let mut padded = Vec::with_capacity(data.len() + 72);
        padded.extend_from_slice(data);
        padded.push(0x80);
        while padded.len() % 64 != 56 {
            padded.push(0);
        }
        padded.extend_from_slice(&bit_len.to_be_bytes());

        let mut w = [0u32; 64];
        for chunk in padded.as_chunks::<64>().0 {
            for (index, slot) in w.iter_mut().take(16).enumerate() {
                let start = index * 4;
                *slot = u32::from_be_bytes([
                    chunk[start],
                    chunk[start + 1],
                    chunk[start + 2],
                    chunk[start + 3],
                ]);
            }
            for index in 16..64 {
                let s0 = w[index - 15].rotate_right(7)
                    ^ w[index - 15].rotate_right(18)
                    ^ (w[index - 15] >> 3);
                let s1 = w[index - 2].rotate_right(17)
                    ^ w[index - 2].rotate_right(19)
                    ^ (w[index - 2] >> 10);
                w[index] = w[index - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[index - 7])
                    .wrapping_add(s1);
            }
            let mut a = h[0];
            let mut b = h[1];
            let mut c = h[2];
            let mut d = h[3];
            let mut e = h[4];
            let mut f = h[5];
            let mut g = h[6];
            let mut hh = h[7];
            for index in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ ((!e) & g);
                let temp1 = hh
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[index])
                    .wrapping_add(w[index]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let temp2 = s0.wrapping_add(maj);
                hh = g;
                g = f;
                f = e;
                e = d.wrapping_add(temp1);
                d = c;
                c = b;
                b = a;
                a = temp1.wrapping_add(temp2);
            }
            h[0] = h[0].wrapping_add(a);
            h[1] = h[1].wrapping_add(b);
            h[2] = h[2].wrapping_add(c);
            h[3] = h[3].wrapping_add(d);
            h[4] = h[4].wrapping_add(e);
            h[5] = h[5].wrapping_add(f);
            h[6] = h[6].wrapping_add(g);
            h[7] = h[7].wrapping_add(hh);
        }

        let mut out = [0u8; 32];
        for (index, value) in h.iter().enumerate() {
            out[index * 4..index * 4 + 4].copy_from_slice(&value.to_be_bytes());
        }
        out
    }

    /// 十六进制小写摘要。
    pub fn hex(data: &[u8]) -> String {
        digest(data)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

// --------------------------------------------------------------------------------------
// 夹具与 manifest
// --------------------------------------------------------------------------------------

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("encoding")
}

struct FixtureEntry {
    name: String,
    bytes: u64,
    sha256: String,
    expected_encoding: String,
    expected_confidence: String,
    expects_confirmation: bool,
    details: String,
}

impl FixtureEntry {
    fn path(&self) -> PathBuf {
        fixtures_dir().join(&self.name)
    }

    fn detail(&self, key: &str) -> Option<&str> {
        self.details.split(';').find_map(|pair| {
            let (candidate, value) = pair.split_once('=')?;
            (candidate == key).then_some(value)
        })
    }
}

fn load_manifest() -> Vec<FixtureEntry> {
    let path = fixtures_dir().join("manifest.tsv");
    let text = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "读取 {} 失败: {error}（请先运行夹具生成器）",
            path.display()
        )
    });
    let mut entries = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(fields.len(), 7, "manifest 行必须 7 列: {line:?}");
        entries.push(FixtureEntry {
            name: fields[0].to_string(),
            bytes: fields[1].parse().expect("字节数必须是整数"),
            sha256: fields[2].to_string(),
            expected_encoding: fields[3].to_string(),
            expected_confidence: fields[4].to_string(),
            expects_confirmation: fields[5] == "true",
            details: fields[6].to_string(),
        });
    }
    assert!(
        entries.len() >= 7,
        "编码夹具至少 7 份（七类编码），实测 {}",
        entries.len()
    );
    entries
}

fn encoding_from_id(id: &str) -> TextEncoding {
    TextEncoding::ALL
        .iter()
        .copied()
        .find(|encoding| encoding.as_str() == id)
        .unwrap_or_else(|| panic!("manifest 中的编码标识 {id} 不是 TextEncoding 的稳定标识"))
}

fn confidence_from_id(id: &str) -> ConfidenceLevel {
    match id {
        "low" => ConfidenceLevel::Low,
        "medium" => ConfidenceLevel::Medium,
        "high" => ConfidenceLevel::High,
        other => panic!("未知可信度标识 {other}"),
    }
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "nr-encoding-{}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
            label
        ));
        fs::create_dir_all(&path).expect("创建临时目录");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn detect_fixture(entry: &FixtureEntry) -> reading_source::EncodingDetection {
    let source = FileTextSource::open(entry.path())
        .unwrap_or_else(|error| panic!("打开夹具 {} 失败: {error}", entry.name));
    detect_encoding(&source).unwrap_or_else(|error| panic!("检测夹具 {} 失败: {error}", entry.name))
}

// --------------------------------------------------------------------------------------
// SHA-256 自检与跨语言对照
// --------------------------------------------------------------------------------------

#[test]
fn encoding_sha256_matches_known_vectors_and_python_manifest() {
    // 标准已知向量（FIPS 180-4 附录 B）。
    assert_eq!(
        sha256::hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256::hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        sha256::hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    // 跨语言对照：Python hashlib 写入 manifest 的 sha256 必须与 Rust 实现一致。
    let mut checked = 0usize;
    for entry in load_manifest() {
        let raw = fs::read(entry.path())
            .unwrap_or_else(|error| panic!("读取夹具 {} 失败: {error}", entry.name));
        assert_eq!(
            raw.len() as u64,
            entry.bytes,
            "夹具 {} 字节数不一致",
            entry.name
        );
        assert_eq!(
            sha256::hex(&raw),
            entry.sha256,
            "夹具 {} 的 Rust sha256 与 manifest（Python hashlib）不一致",
            entry.name
        );
        checked += 1;
    }
    assert!(checked >= 7, "至少对照 7 份夹具，实测 {checked}");
}

// --------------------------------------------------------------------------------------
// 七类编码夹具断言
// --------------------------------------------------------------------------------------

#[test]
fn encoding_fixtures_match_expected_detection() {
    let entries = load_manifest();
    let mut covered: Vec<TextEncoding> = Vec::new();
    for entry in &entries {
        let detection = detect_fixture(entry);
        let expected = encoding_from_id(&entry.expected_encoding);
        let confidence = confidence_from_id(&entry.expected_confidence);
        assert_eq!(
            detection.encoding, expected,
            "夹具 {} 期望 {} 实际 {}（候选明细 {:#?}）",
            entry.name, expected, detection.encoding, detection.legacy_candidates
        );
        assert_eq!(
            detection.confidence,
            confidence,
            "夹具 {} 期望可信度 {} 实际 {}（分数 {}）",
            entry.name,
            confidence.as_str(),
            detection.confidence.as_str(),
            detection.score
        );
        assert_eq!(
            detection.needs_confirmation(),
            entry.expects_confirmation,
            "夹具 {} 的“需要用户确认”状态不符（置信度 {} / 采样 {} 字节）",
            entry.name,
            detection.confidence.as_str(),
            detection.sampled_bytes
        );
        assert_eq!(
            detection.detector_version, ENCODING_DETECTOR_VERSION,
            "夹具 {} 的检测器版本必须冻结为 {ENCODING_DETECTOR_VERSION}",
            entry.name
        );
        assert_declared_bom_len(entry, expected, detection.bom_len);
        assert_eq!(
            detection.source_bytes, entry.bytes,
            "数据源总长必须等于夹具大小"
        );
        if entry.bytes == 0 {
            assert_eq!(detection.sampled_bytes, 0, "空文件不读字节");
        } else {
            assert!(detection.sampled_bytes > 0, "非空夹具必须读到字节");
            assert!(
                detection.sampled_bytes <= DETECT_MAX_SAMPLED_BYTES as u64,
                "夹具 {} 采样 {} 字节，超过硬上限 {DETECT_MAX_SAMPLED_BYTES}",
                entry.name,
                detection.sampled_bytes
            );
        }
        assert_eq!(
            detection.encoding.bom_len(),
            detection.bom_len,
            "夹具 {} 的 bom_len 必须与编码常量一致",
            entry.name
        );
        if !covered.contains(&expected) {
            covered.push(expected);
        }
    }
    for required in [
        TextEncoding::Utf8,
        TextEncoding::Utf8Bom,
        TextEncoding::Utf16Le,
        TextEncoding::Utf16Be,
        TextEncoding::Gb18030,
        TextEncoding::Gbk,
        TextEncoding::Big5,
    ] {
        assert!(
            covered.contains(&required),
            "七类编码必须各有断言，缺少 {required}（已覆盖 {covered:?}）"
        );
    }
    assert_eq!(covered.len(), 7, "覆盖的编码类别应恰为 7 类: {covered:?}");
}

fn assert_declared_bom_len(entry: &FixtureEntry, expected: TextEncoding, actual: usize) {
    assert_eq!(
        actual,
        expected.bom_len(),
        "夹具 {} 的 BOM 长度必须与编码 {} 一致",
        entry.name,
        expected
    );
    if let Some(declared) = entry.detail("bom_len") {
        assert_eq!(
            declared.parse::<usize>().expect("bom_len 必须是整数"),
            actual,
            "夹具 {} 的 manifest bom_len 与检测结果不一致",
            entry.name
        );
    }
}

#[test]
fn encoding_legacy_candidates_expose_honest_scores() {
    let entries = load_manifest();
    // GBK 夹具：GBK 领先 Big5，且把字节等价的 GB18030 作为替代告知用户。
    let gbk = entries
        .iter()
        .find(|e| e.name == "gbk.txt")
        .expect("gbk 夹具");
    let detection = detect_fixture(gbk);
    assert_eq!(detection.encoding, TextEncoding::Gbk);
    assert_eq!(detection.confidence, ConfidenceLevel::High);
    assert!(
        detection.alternatives.contains(&TextEncoding::Gb18030),
        "GBK 结果必须把两字节区间字节相同的 GB18030 列为替代: {:?}",
        detection.alternatives
    );
    let gbk_candidate = detection.candidate(TextEncoding::Gbk).expect("GBK 评分");
    let big5_candidate = detection.candidate(TextEncoding::Big5).expect("Big5 评分");
    assert!(gbk_candidate.is_structurally_valid());
    assert!(
        gbk_candidate.score > big5_candidate.score,
        "GBK 命中率应高于 Big5 误判: gbk={} big5={}",
        gbk_candidate.score,
        big5_candidate.score
    );
    assert_eq!(
        gbk_candidate.four_byte_sequences, 0,
        "GBK 夹具不含四字节序列"
    );
    assert!(
        big5_candidate.hit_permille < gbk_candidate.hit_permille,
        "Big5 误判命中率必须更低: {} vs {}",
        big5_candidate.hit_permille,
        gbk_candidate.hit_permille
    );

    // GB18030 夹具：出现四字节序列后 GBK 结构上不成立，判 GB18030。
    let gb18030 = entries
        .iter()
        .find(|e| e.name == "gb18030.txt")
        .expect("gb18030 夹具");
    let detection = detect_fixture(gb18030);
    assert_eq!(detection.encoding, TextEncoding::Gb18030);
    let candidate = detection
        .candidate(TextEncoding::Gb18030)
        .expect("GB18030 评分");
    let gbk_candidate = detection.candidate(TextEncoding::Gbk).expect("GBK 评分");
    assert!(
        candidate.four_byte_sequences >= 1,
        "GB18030 夹具必须真的包含四字节序列"
    );
    assert!(
        candidate.structural_ok_permille > gbk_candidate.structural_ok_permille,
        "四字节序列让 GBK 结构不成立: gb18030={} gbk={}",
        candidate.structural_ok_permille,
        gbk_candidate.structural_ok_permille
    );

    // Big5 夹具：Big5 领先两个 GB 系候选。
    let big5 = entries
        .iter()
        .find(|e| e.name == "big5.txt")
        .expect("big5 夹具");
    let detection = detect_fixture(big5);
    assert_eq!(detection.encoding, TextEncoding::Big5);
    assert_eq!(detection.confidence, ConfidenceLevel::High);
    let big5_candidate = detection.candidate(TextEncoding::Big5).expect("Big5 评分");
    let gbk_candidate = detection.candidate(TextEncoding::Gbk).expect("GBK 评分");
    assert!(
        big5_candidate.score > gbk_candidate.score,
        "Big5 命中率应高于 GBK 误判: big5={} gbk={}",
        big5_candidate.score,
        gbk_candidate.score
    );
}

#[test]
fn encoding_ambiguous_fixture_reports_low_confidence_and_overridable_state() {
    let entries = load_manifest();
    let entry = entries
        .iter()
        .find(|e| e.name == "ambiguous-gbk-big5.bin")
        .expect("模糊夹具");
    let detection = detect_fixture(entry);

    // 低可信度 + 允许覆盖：评分必须是“真的接近”，不是人为压低。
    assert_eq!(detection.confidence, ConfidenceLevel::Low);
    assert!(detection.needs_confirmation(), "低可信度必须提示用户确认");
    let gbk_candidate = detection.candidate(TextEncoding::Gbk).expect("GBK 评分");
    let big5_candidate = detection.candidate(TextEncoding::Big5).expect("Big5 评分");
    assert!(gbk_candidate.is_structurally_valid());
    assert!(big5_candidate.is_structurally_valid());
    let gap = gbk_candidate.score.abs_diff(big5_candidate.score);
    assert!(
        gap <= reading_source::LEGACY_MARGIN_HIGH,
        "模糊夹具的 GBK/Big5 分数差 {gap} 必须落在“并列”阈值内"
    );
    assert!(
        detection.alternatives.contains(&TextEncoding::Big5),
        "并列时 Big5 必须出现在替代列表: {:?}",
        detection.alternatives
    );

    // 覆盖语义：内容/章节/分页索引全部失效，版本递增，识别结果本身保留。
    let mut state = EncodingState::new(detection.clone());
    assert!(state.needs_confirmation());
    assert_eq!(state.effective_encoding(), TextEncoding::Gbk);
    assert_eq!(state.effective_confidence(), ConfidenceLevel::Low);
    let version_before = state.content_index_version();

    let invalidation = state.override_encoding(TextEncoding::Big5);
    assert!(invalidation.content_index_invalidated);
    assert!(invalidation.chapter_index_invalidated);
    assert!(invalidation.pagination_invalidated);
    assert_eq!(invalidation.previous_content_index_version, version_before);
    assert_eq!(
        invalidation.content_index_version,
        version_before + 1,
        "覆盖编码必须递增 content_index_version"
    );
    assert_eq!(state.user_override(), Some(TextEncoding::Big5));
    assert_eq!(state.effective_encoding(), TextEncoding::Big5);
    assert_eq!(state.effective_confidence(), ConfidenceLevel::High);
    assert!(!state.needs_confirmation(), "用户指定后不再需要确认");
    assert_eq!(state.detector_version(), ENCODING_DETECTOR_VERSION);
    assert_eq!(
        state.detection().encoding,
        TextEncoding::Gbk,
        "覆盖不得篡改原始识别结果"
    );

    // 重复覆盖成同一编码：无变更、不失效。
    let noop = state.override_encoding(TextEncoding::Big5);
    assert!(!noop.invalidated());
    assert_eq!(noop.content_index_version, version_before + 1);

    // 清除覆盖：回到识别结果，同样要重建索引。
    let cleared = state
        .clear_override()
        .expect("存在覆盖，清除必须返回失效报告");
    assert!(cleared.invalidated());
    assert_eq!(cleared.content_index_version, version_before + 2);
    assert_eq!(state.user_override(), None);
    assert_eq!(state.effective_encoding(), TextEncoding::Gbk);
    assert!(state.needs_confirmation());
    assert!(state.clear_override().is_none(), "无覆盖时清除返回 None");

    // 两字节区间内 GBK 与 GB18030 字节完全相同：覆盖视为等价，不得制造假失效。
    let mut equivalent_state = EncodingState::new(detection);
    let version = equivalent_state.content_index_version();
    let equivalent = equivalent_state.override_encoding(TextEncoding::Gb18030);
    assert!(
        !equivalent.invalidated(),
        "GBK↔GB18030 在采样字节上解码等价，不得制造假失效"
    );
    assert_eq!(equivalent.content_index_version, version);
    assert_eq!(equivalent_state.user_override(), None);
    assert_eq!(equivalent_state.effective_encoding(), TextEncoding::Gbk);
    // 但 UTF-16LE 与 UTF-16BE 只是同族、并不解码等价，必须失效。
    let swapped = equivalent_state.override_encoding(TextEncoding::Utf16Be);
    assert!(swapped.invalidated());
    assert_eq!(swapped.content_index_version, version + 1);
}

// --------------------------------------------------------------------------------------
// 覆盖不修改原文件
// --------------------------------------------------------------------------------------

#[test]
fn encoding_override_keeps_file_untouched() {
    let entries = load_manifest();
    let entry = entries
        .iter()
        .find(|e| e.name == "gbk.txt")
        .expect("gbk 夹具");
    let source_bytes = fs::read(entry.path()).expect("读取夹具原始字节");

    let temp = TempDir::new("override");
    let copy_path = temp.path().join("gbk-copy.txt");
    fs::write(&copy_path, &source_bytes).expect("写入临时副本");

    let sha_before = sha256::hex(&source_bytes);
    let len_before = fs::metadata(&copy_path).expect("元信息").len();
    let modified_before = fs::metadata(&copy_path).expect("元信息").modified().ok();

    // 1) 自动识别。
    let source = FileTextSource::open(&copy_path).expect("打开临时副本");
    let detection = detect_encoding(&source).expect("编码检测");
    assert_eq!(detection.encoding, TextEncoding::Gbk);
    let mut state = EncodingState::new(detection);

    // 2) 用户手动覆盖成每一种其它编码，并断言索引失效语义。
    let mut total_invalidations = 0u32;
    let mut expected_version = state.content_index_version();
    for target in [
        TextEncoding::Big5,
        TextEncoding::Utf8,
        TextEncoding::Utf16Be,
    ] {
        let invalidation = state.override_encoding(target);
        assert!(invalidation.invalidated(), "切换到 {target} 必须让索引失效");
        expected_version += 1;
        assert_eq!(invalidation.content_index_version, expected_version);
        total_invalidations += 1;
        assert_eq!(state.effective_encoding(), target);
    }
    assert_eq!(total_invalidations, 3);

    // 3) 原文件字节、长度与修改时间都必须保持不变。
    let after_bytes = fs::read(&copy_path).expect("重新读取临时副本");
    let sha_after = sha256::hex(&after_bytes);
    assert_eq!(
        sha_after, sha_before,
        "切换编码前后原文件 sha256 必须一致（覆盖不得写回原文件）"
    );
    assert_eq!(after_bytes, source_bytes, "文件字节必须逐字节保持一致");
    assert_eq!(
        fs::metadata(&copy_path).expect("元信息").len(),
        len_before,
        "文件长度不得变化"
    );
    assert_eq!(
        fs::metadata(&copy_path).expect("元信息").modified().ok(),
        modified_before,
        "文件修改时间不得变化"
    );
    assert_eq!(
        sha_after, entry.sha256,
        "临时副本 sha256 必须与 manifest 中 Python hashlib 的结果一致"
    );
    assert_eq!(
        sha_before, entry.sha256,
        "覆盖前 sha256 也必须与 manifest 一致"
    );

    // 4) 仓库内夹具本身也不得被改动。
    assert_eq!(
        fs::read(entry.path()).expect("读取仓库夹具").len() as u64,
        entry.bytes
    );
    assert_eq!(
        sha256::hex(&fs::read(entry.path()).expect("读取仓库夹具")),
        entry.sha256
    );

    // 5) 全部编码切换完成后仍可继续切换（索引版本单调递增）。
    assert_eq!(
        state.content_index_version(),
        EncodingState::INITIAL_CONTENT_INDEX_VERSION + 3
    );
}

// --------------------------------------------------------------------------------------
// 有限窗口
// --------------------------------------------------------------------------------------

#[test]
fn encoding_detection_reads_bounded_windows_only() {
    let entries = load_manifest();
    let entry = entries
        .iter()
        .find(|e| e.name == "gbk.txt")
        .expect("gbk 夹具");
    let unit = fs::read(entry.path()).expect("读取夹具原始字节");

    // 约 8 MiB 的 GBK 文本（夹具内容重复拼接：周期为偶数字节，2 字节对齐不被破坏）。
    let repeat = 3000usize;
    let mut payload = Vec::with_capacity(unit.len() * repeat);
    for _ in 0..repeat {
        payload.extend_from_slice(&unit);
    }
    let temp = TempDir::new("bounded");
    let big_path = temp.path().join("big-gbk.txt");
    fs::write(&big_path, &payload).expect("写入大文件");
    let big_len = payload.len() as u64;
    drop(payload);

    let source = FileTextSource::open(&big_path).expect("打开大文件");
    assert_eq!(source.len(), big_len);
    let detection = detect_encoding(&source).expect("大文件编码检测");

    assert_eq!(
        detection.encoding,
        TextEncoding::Gbk,
        "大文件仍应识别为 GBK"
    );
    assert_eq!(detection.confidence, ConfidenceLevel::High);
    assert_eq!(detection.source_bytes, big_len);
    assert!(
        detection.windows_read <= 3,
        "检测最多读 3 个窗口，实测 {}",
        detection.windows_read
    );
    assert!(
        detection.sampled_bytes <= DETECT_MAX_SAMPLED_BYTES as u64,
        "检测采样 {} 字节，超过硬上限 {DETECT_MAX_SAMPLED_BYTES}",
        detection.sampled_bytes
    );
    assert!(
        detection.sampled_bytes * 100 < big_len,
        "检测采样量必须与文件大小无关：采样 {} 字节 / 文件 {} 字节",
        detection.sampled_bytes,
        big_len
    );
    assert!(
        !detection.sampled_whole_source(),
        "大文件不得整本读入（采样 {} < 总长 {}）",
        detection.sampled_bytes,
        big_len
    );

    // 数据源的读取计数与物化上限也必须成立（证明走的是有限窗口路径）。
    let stats = source.stats();
    assert_eq!(stats.read_calls as u32, detection.windows_read);
    assert!(stats.max_materialized_bytes <= DETECT_SAMPLE_BYTES.max(DETECT_HEAD_BYTES) as u64);
    assert!(
        stats.total_materialized_bytes <= DETECT_MAX_SAMPLED_BYTES as u64,
        "累计物化 {} 字节，超过硬上限 {DETECT_MAX_SAMPLED_BYTES}",
        stats.total_materialized_bytes
    );
    assert!(
        stats.total_materialized_bytes >= detection.sampled_bytes,
        "采样字节数不可能多于物理读取字节数"
    );
    // 差值只是每个窗口两端的边界对齐裁剪（≤4 字节/窗口）。
    assert!(
        stats.total_materialized_bytes - detection.sampled_bytes
            <= 4 * detection.windows_read as u64,
        "物化 {} 与采样 {} 的差额必须仅来自窗口边界对齐",
        stats.total_materialized_bytes,
        detection.sampled_bytes
    );

    // 小文件（夹具本身）允许读完全部内容，但同样受硬上限约束。
    let small = FileTextSource::open(entry.path()).expect("打开夹具");
    let small_detection = detect_encoding(&small).expect("夹具检测");
    assert!(small_detection.sampled_whole_source());
    assert!(small_detection.sampled_bytes <= DETECT_MAX_SAMPLED_BYTES as u64);
    assert_eq!(small_detection.windows_read, 1);
}

#[test]
fn encoding_detection_prints_fixture_report() {
    // 诊断用：`cargo test -p reading-source --release -- --nocapture encoding_detection_prints`
    for entry in load_manifest() {
        let detection = detect_fixture(&entry);
        println!(
            "{}: encoding={} confidence={} score={} bom={} sampled={}/{} windows={} confirm={} alternatives={:?}",
            entry.name,
            detection.encoding,
            detection.confidence.as_str(),
            detection.score,
            detection.bom_len,
            detection.sampled_bytes,
            detection.source_bytes,
            detection.windows_read,
            detection.needs_confirmation(),
            detection.alternatives
        );
        for candidate in &detection.legacy_candidates {
            println!(
                "    {} score={} structural={}‰ hits={}‰ seq={} table_hits={} four_byte={} invalid={} total={}",
                candidate.encoding,
                candidate.score,
                candidate.structural_ok_permille,
                candidate.hit_permille,
                candidate.sequences,
                candidate.table_hits,
                candidate.four_byte_sequences,
                candidate.invalid_bytes,
                candidate.total_bytes
            );
        }
    }
}

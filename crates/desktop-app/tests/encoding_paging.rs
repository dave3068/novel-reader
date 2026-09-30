//! 编码贯通契约（桌面端）：非 UTF-8 文档必须能排版、页边界必须被核心接受、原文件只读。
//!
//! 背景：核心自 2026-09-29 起支持 7 种编码的边界判定与章节扫描
//! （见 `results/encoding-core-boundary.md`）；桌面侧由
//! [`desktop_app::encoded_source::EncodedFileSource`] + `PageSession::open_with_encoding`
//! 贯通「识别 → 解码 → 字节计数 → 边界提交」。本文件把这条链路固定成契约。
//!
//! 夹具一律以**只读**方式打开，并在用例内核对指纹（大小 + 修改时间 + inode）不变。

use std::path::{Path, PathBuf};

use desktop_app::encoded_source::EncodedFileSource;
use desktop_app::layout::LayoutMetrics;
use desktop_app::pagination::{measure_page_with_encoding, PageError, PageMeasurementRequest};
use desktop_app::reflow::{
    layout_signature, MappedViewport, PlatformFontMetrics, Typography, TypographySettings,
};
use reading_core::{BoundaryCommitStatus, ContentUnit, PageBoundaryRequest, PaginationCache};
use reading_source::{is_boundary, TextEncoding, TextSource};

/// 仓库夹具路径（`crates/desktop-app/../../tests/fixtures`）。
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

/// 夹具指纹（大小 + 修改时间 + inode），用于断言只读。
fn fingerprint_of(path: &Path) -> (u64, std::time::SystemTime, u64) {
    let meta = std::fs::metadata(path).expect("夹具元信息");
    #[cfg(unix)]
    let inode = {
        use std::os::unix::fs::MetadataExt;
        meta.ino()
    };
    #[cfg(not(unix))]
    let inode = 0u64;
    (meta.len(), meta.modified().expect("mtime"), inode)
}

fn font() -> PlatformFontMetrics {
    PlatformFontMetrics::from_px(14, 5)
}

fn signature() -> reading_core::LayoutSignature {
    layout_signature(
        &MappedViewport::new(1080, 720),
        &LayoutMetrics::DESKTOP,
        &TypographySettings::default(),
        font(),
        false,
    )
    .expect("窗口映射后可派生签名")
}

fn typography() -> Typography {
    Typography::derive(&signature(), font()).expect("排版参数")
}

fn request_for(
    source: &EncodedFileSource,
    start_byte: u64,
    layout_id: u64,
    request_id: u64,
) -> PageMeasurementRequest {
    PageMeasurementRequest {
        layout_id,
        request_id,
        content_index_version: ContentUnit::from_source(source, 1).content_index_version,
        total_bytes: source.len(),
        start_byte,
    }
}

/// 顺序排版并把每页边界提交核心：返回（被接受页数、被拒绝页数、页尾序列）。
fn submit_all(path: &Path, encoding: TextEncoding) -> (usize, usize, Vec<u64>) {
    let source = EncodedFileSource::open(path, encoding).expect("打开夹具");
    let total = source.len();
    let version = ContentUnit::from_source(&source, 1).content_index_version;
    let mut cache = PaginationCache::new(signature(), version).expect("分页缓存");
    let typography = typography();
    let mut start = 0u64;
    let mut accepted = 0usize;
    let mut rejected = 0usize;
    let mut ends = Vec::new();
    while start < total {
        let mut request = request_for(
            &source,
            start,
            cache.layout_id(),
            cache.latest_request_id() + 1,
        );
        request.request_id = cache.latest_request_id() + 1;
        let measured =
            measure_page_with_encoding(&source, &request, &typography, encoding).expect("测量一页");
        let commit = cache.commit_page_boundary(
            &source,
            &PageBoundaryRequest {
                layout_id: request.layout_id,
                request_id: request.request_id,
                content_index_version: request.content_index_version,
                total_bytes: total,
                start_byte: measured.start_byte,
                end_byte: measured.end_byte,
            },
            encoding,
        );
        match commit.status {
            BoundaryCommitStatus::Committed | BoundaryCommitStatus::DuplicateIdentical => {
                accepted += 1
            }
            _ => rejected += 1,
        }
        assert!(measured.end_byte > start, "页必须严格前进（不丢字/不重字）");
        // 页尾必须是**该编码**下的合法字符边界（核心的判断口径）。
        let window = source.read_at(0, 64.min(total as usize)).expect("窗口");
        if (measured.end_byte as usize) < window.data.len() {
            assert!(
                is_boundary(&window.data, measured.end_byte as usize, encoding),
                "页尾 {} 必须是 {encoding:?} 下的合法字符边界",
                measured.end_byte
            );
        }
        ends.push(measured.end_byte);
        start = measured.end_byte;
    }
    (accepted, rejected, ends)
}

#[test]
fn gbk_pages_are_measured_and_accepted_by_the_core() {
    let path = fixture("encoding/gbk.txt");
    let before = fingerprint_of(&path);
    let (accepted, rejected, ends) = submit_all(&path, TextEncoding::Gbk);
    assert!(
        accepted >= 2,
        "GBK 夹具应排出至少两页（accepted={accepted}）"
    );
    assert_eq!(rejected, 0, "GBK 页边界不得被核心拒绝");
    assert_eq!(ends.last().copied(), Some(2864), "最后一页必须到文件末尾");

    let source = EncodedFileSource::open(&path, TextEncoding::Gbk).expect("打开");
    let request = request_for(&source, 0, 1, 1);
    let page = measure_page_with_encoding(&source, &request, &typography(), TextEncoding::Gbk)
        .expect("GBK 首屏必须可排版");
    assert_eq!(page.lines.len(), 20, "首屏应排满 20 行");
    let text = page.lines.join("");
    assert!(
        text.contains("第一章") || text.contains("第1章"),
        "GBK 正文必须解码为可读中文：{:?}",
        &text.chars().take(40).collect::<String>()
    );
    assert_eq!(fingerprint_of(&path), before, "夹具必须只读");
}

#[test]
fn utf16le_bom_document_renders() {
    let path = fixture("encoding/utf16le-bom.txt");
    let before = fingerprint_of(&path);
    let source = EncodedFileSource::open(&path, TextEncoding::Utf16Le).expect("打开");
    let request = request_for(&source, 0, 1, 1);
    let page = measure_page_with_encoding(&source, &request, &typography(), TextEncoding::Utf16Le)
        .expect("UTF-16LE 首屏必须可排版");
    assert!(!page.lines.is_empty(), "UTF-16LE 首屏必须有行");
    let text = page.lines.join("");
    assert!(
        !text.trim().is_empty() && !text.contains('\u{FFFD}'),
        "UTF-16LE 正文不得为空或替换字符：{:?}",
        &text.chars().take(40).collect::<String>()
    );
    assert!(page.end_byte % 2 == 0, "UTF-16 页尾必须落在码元边界");
    assert_eq!(fingerprint_of(&path), before, "夹具必须只读");
}

#[test]
fn utf8_behavior_is_unchanged() {
    let path = fixture("smoke.txt");
    let before = fingerprint_of(&path);
    let source = EncodedFileSource::open(&path, TextEncoding::Utf8).expect("打开");
    assert_eq!(source.len(), 18415);
    let request = request_for(&source, 0, 1, 1);
    let page = measure_page_with_encoding(&source, &request, &typography(), TextEncoding::Utf8)
        .expect("UTF-8 首屏");
    assert_eq!(
        (page.start_byte, page.end_byte, page.lines.len()),
        (0, 1623, 20),
        "UTF-8 口径不得回归"
    );
    assert_eq!(fingerprint_of(&path), before, "夹具必须只读");
}

#[test]
fn utf16_walk_matches_decoder_counts() {
    // 契约：按码元步进的字节长度表必须与解码器实际产出的字符数**逐一对应**
    // （否则页边界会错位；「ASCII 首字节 → 1 字节」的早退是曾经的实现陷阱）。
    let path = fixture("encoding/utf16le-bom.txt");
    let data = std::fs::read(&path).expect("读夹具");
    assert_eq!(
        reading_source::detect_bom(&data),
        Some((TextEncoding::Utf16Le, 2))
    );
    let decoded = reading_source::decode_line(&data, TextEncoding::Utf16Le).expect("解码");
    let mut steps = 0usize;
    let mut cursor = 2usize;
    while cursor < data.len() {
        let unit = u16::from_le_bytes([data[cursor], data[cursor + 1]]);
        let step = if data.len() - cursor >= 4 && (0xD800..=0xDBFF).contains(&unit) {
            4
        } else {
            2
        };
        cursor += step;
        steps += 1;
    }
    assert_eq!(cursor, data.len(), "步进必须恰好覆盖窗口");
    assert_eq!(steps, decoded.chars().count(), "步进数与解码字符数必须一致");
}

#[test]
fn boundary_inside_a_gbk_character_is_rejected() {
    // 负向对照：把页首落在 GBK 字符内部必须被拒绝（否则「贯通」退化为「一律放行」）。
    let path = fixture("encoding/gbk.txt");
    let source = EncodedFileSource::open(&path, TextEncoding::Gbk).expect("打开");
    let data = source.read_at(0, 64).expect("窗口").data;
    let mut inside = None;
    for index in 1..data.len() {
        if !is_boundary(&data, index, TextEncoding::Gbk)
            && is_boundary(&data, index, TextEncoding::Utf8)
        {
            inside = Some(index);
            break;
        }
    }
    let Some(index) = inside else {
        panic!("夹具里应存在「GBK 字符内部、但 UTF-8 判定为边界」的偏移用于负向对照");
    };
    assert!(
        !is_boundary(&data, index, TextEncoding::Gbk),
        "偏移 {index} 在 GBK 语义下位于字符内部，必须判为非边界"
    );
    let request = request_for(&source, index as u64, 1, 1);
    match measure_page_with_encoding(&source, &request, &typography(), TextEncoding::Gbk) {
        Err(PageError::WindowUndecodable { .. }) | Err(PageError::NoProgress { .. }) => {}
        other => panic!("字符内部起步必须被拒绝，实际：{other:?}"),
    }
}

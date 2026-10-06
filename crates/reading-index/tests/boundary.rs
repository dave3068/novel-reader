//! RC5 集成测试（reading-index）：页面边界的字节语义、字符边界探测与连续链。
//!
//! 覆盖 `reading-index` 侧的 DoD 子集：
//! - `is_char_boundary_at` 在多字节文本上与 `str::is_char_boundary` 的事实**逐字节一致**
//!   （只读 ≤ 8 字节小窗口，不整本读取）；
//! - `validate_page_boundary` 拒绝字符中间、零长度、倒置与越界；
//! - `PageBoundaryChain` 保证正反向翻页共享同一份边界且不丢字/不重字。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use reading_index::boundary::{
    check_strict_advance, is_char_boundary_at, is_char_boundary_in_window, validate_interval,
    validate_page_boundary, BoundaryRejection, PageBoundary, PageBoundaryChain,
};
use reading_source::TextEncoding;
use reading_source::{FileTextSource, SourceError, TextSource};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempGuard {
    dir: PathBuf,
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn temp_source(label: &str, content: &str) -> (TempGuard, FileTextSource) {
    let dir = std::env::temp_dir().join(format!(
        "nr-rc5-index-{}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
        label
    ));
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    let path = dir.join("book.txt");
    std::fs::write(&path, content.as_bytes()).expect("写入临时文件");
    let source = FileTextSource::open(&path).expect("打开临时文件");
    (TempGuard { dir }, source)
}

/// 中英混排 + 全角标点 + 4 字节 emoji：覆盖 2/3/4 字节字符的每一个内部偏移。
fn mixed_text() -> String {
    "第一章 墨色深处\n“你到底还是来了？”他低声说道。🙂\n中文abc，标点齐全。\n".repeat(12)
}

#[test]
fn boundary_char_probe_matches_str_facts_byte_for_byte() {
    let text = mixed_text();
    let (_guard, source) = temp_source("probe", &text);
    let total = source.len();

    for offset in 0..=total {
        let expected = text.is_char_boundary(offset as usize);
        let actual =
            is_char_boundary_at(&source, 0, offset, TextEncoding::Utf8).expect("探测必须成功");
        assert_eq!(
            actual, expected,
            "偏移 {offset} 的字符边界判定必须与 str::is_char_boundary 一致"
        );
    }
    // 越界偏移报 OutOfRange，而不是猜测。
    assert_eq!(
        is_char_boundary_at(&source, 0, total + 1, TextEncoding::Utf8),
        Err(SourceError::OutOfRange)
    );

    // 只读小窗口：探测 3 个不同偏移不物化整本。
    let before = source.stats().total_materialized_bytes;
    let _ = is_char_boundary_at(&source, 0, 7, TextEncoding::Utf8).expect("探测");
    let _ = is_char_boundary_at(&source, 0, total / 2, TextEncoding::Utf8).expect("探测");
    let _ = is_char_boundary_at(&source, 0, total - 1, TextEncoding::Utf8).expect("探测");
    let read = source.stats().total_materialized_bytes - before;
    assert!(read <= 64, "字符边界探测必须只读小窗口（实际 {read} 字节）");

    // 手工窗口探针：不在窗口内返回 None，窗口两端恒为边界。
    let window = source.read_at(3, 8).expect("窗口读取");
    assert_eq!(
        is_char_boundary_in_window(&window, window.start(), TextEncoding::Utf8),
        Some(true)
    );
    assert_eq!(
        is_char_boundary_in_window(&window, window.end(), TextEncoding::Utf8),
        Some(true)
    );
    assert_eq!(
        is_char_boundary_in_window(&window, window.start() - 1, TextEncoding::Utf8),
        None
    );
}

#[test]
fn boundary_validation_rejects_mid_char_empty_and_out_of_range() {
    let text = mixed_text();
    let (_guard, source) = temp_source("validate", &text);
    let total = source.len();

    // 合法：覆盖全文。
    validate_page_boundary(
        &source,
        &PageBoundary::new(0, total),
        total,
        TextEncoding::Utf8,
    )
    .expect("全文边界合法");

    // 非法：end 落在字符内部（多字节字符的每个内部偏移都必须被拒）。
    let mut rejected_internal = 0usize;
    for offset in 0..total {
        if text.is_char_boundary(offset as usize) {
            continue;
        }
        let error = validate_page_boundary(
            &source,
            &PageBoundary::new(0, offset),
            total,
            TextEncoding::Utf8,
        )
        .expect_err("字符中间的 end 必须被拒");
        assert_eq!(error.rejection(), BoundaryRejection::IllegalCharBoundary);
        assert_eq!(error.code(), 3);
        rejected_internal += 1;
    }
    assert!(
        rejected_internal > 0,
        "夹具必须真的含有字符内部偏移（否则该断言是空检查）"
    );

    // 非法：零长度与倒置。
    let empty = validate_interval(&PageBoundary::new(12, 12), total).expect_err("零长度必须被拒");
    assert_eq!(empty.rejection(), BoundaryRejection::EmptyInterval);
    assert_eq!(empty.code(), 1);
    let reversed = validate_interval(&PageBoundary::new(30, 12), total).expect_err("倒置必须被拒");
    assert_eq!(reversed.rejection(), BoundaryRejection::ReversedInterval);

    // 非法：越界与长度不一致。
    let beyond = validate_page_boundary(
        &source,
        &PageBoundary::new(0, total + 1),
        total,
        TextEncoding::Utf8,
    )
    .expect_err("越界必须被拒");
    assert_eq!(beyond.rejection(), BoundaryRejection::OutOfRange);
    assert_eq!(beyond.code(), 2);
    let mismatch = validate_page_boundary(
        &source,
        &PageBoundary::new(0, 12),
        total + 5,
        TextEncoding::Utf8,
    )
    .expect_err("长度不一致必须被拒");
    assert_eq!(mismatch.rejection(), BoundaryRejection::TotalLengthMismatch);

    // 严格前进：空隙/重叠/停滞分别有独立原因。
    let previous = PageBoundary::new(0, 30);
    assert!(check_strict_advance(&previous, &PageBoundary::new(30, 60)).is_ok());
    assert_eq!(
        check_strict_advance(&previous, &PageBoundary::new(33, 60))
            .expect_err("空隙必须被拒")
            .rejection(),
        BoundaryRejection::GapBeforePrevious
    );
    assert_eq!(
        check_strict_advance(&previous, &PageBoundary::new(27, 60))
            .expect_err("重叠必须被拒")
            .rejection(),
        BoundaryRejection::OverlapWithPrevious
    );
}

#[test]
fn boundary_chain_partitions_text_in_both_directions() {
    let text = mixed_text().repeat(4);
    let (_guard, source) = temp_source("chain", &text);
    let total = source.len();
    let bytes = text.as_bytes();

    // 按每页约 700 字节测量并向前对齐字符边界。
    let mut chain = PageBoundaryChain::new(0);
    let mut start = 0usize;
    while start < text.len() {
        let mut end = (start + 700).min(text.len());
        while end < text.len() && !text.is_char_boundary(end) {
            end += 1;
        }
        let boundary = PageBoundary::new(start as u64, end as u64);
        validate_page_boundary(&source, &boundary, total, TextEncoding::Utf8)
            .expect("测量结果必须合法");
        chain.push(boundary).expect("页面必须严格前进");
        start = end;
    }
    assert!(chain.len() >= 3);
    assert_eq!(chain.covered(), (0, total));
    chain.assert_contiguous().expect("整链必须首尾相接");

    // 逐字节重建：拼接所有页面必须与原文件完全一致。
    let mut rebuilt = Vec::with_capacity(total as usize);
    for page in chain.pages() {
        rebuilt.extend_from_slice(&bytes[page.start_byte as usize..page.end_byte as usize]);
    }
    assert_eq!(rebuilt, bytes);

    // 反向遍历回到同一批页面。
    let mut backward: Vec<PageBoundary> = Vec::new();
    for index in (0..chain.len()).rev() {
        backward.push(*chain.page(index).expect("页面存在"));
    }
    for index in 1..chain.len() {
        assert_eq!(
            chain.next_of(index - 1),
            chain.page(index),
            "正向翻页必须与边界链一致"
        );
    }
    backward.reverse();
    assert_eq!(backward, chain.pages());

    // 再次追加同一页面会被拒（重复登记不是前进）。
    let last = *chain.last().expect("非空");
    let duplicate = chain.push(PageBoundary::new(last.start_byte, last.end_byte));
    assert!(duplicate.is_err(), "重复登记同一页面必须被拒");
}

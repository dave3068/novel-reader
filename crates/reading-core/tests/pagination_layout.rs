//! RC5 集成测试（reading-core）：分页边界、布局签名与缓存失效的端到端契约。
//!
//! 覆盖本卡 DoD：
//! - 正向 / 反向分页连续、无丢字、无重字（[`pagination_forward_and_backward_pages_are_contiguous_without_gap_or_overlap`]）；
//! - 非法边界被拒（`end == start`、倒置、越界、落在字符中间）且不产生任何缓存副作用
//!   （[`page_boundary_rejects_illegal_intervals_and_mid_char_end`]）；
//! - 签名变化使全部旧边界失效（[`layout_signature_change_invalidates_all_cached_boundaries`]）；
//! - 按签名失效只影响匹配签名，并让在途结果过期（[`invalidate_by_signature_bumps_generation_and_drops_inflight`]）；
//! - 过期排版结果按 `layout_id` / `request_id` / `content_index_version` 丢弃
//!   （[`stale_layout_result_rejected_by_layout_id_and_request_id`]）；
//! - 错误码与冻结契约一致、页码精确（不是百分比）
//!   （[`pagination_boundary_codes_match_frozen_contract_not_percentage`]）。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use reading_core::{
    is_char_boundary_at, validate_page_boundary, BoundaryCommitStatus, BoundaryRejection,
    ContentPosition, ErrorCode, LayoutSignature, PageBoundary, PageBoundaryChain, PaginationCache,
    TextDirection, LAYOUT_ALGORITHM_VERSION, LAYOUT_SIGNATURE_FIELDS, PAGINATION_VERSION,
};
use reading_source::{FileTextSource, TextEncoding, TextSource, DEFAULT_WINDOW_BYTES};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// 临时文件守卫（测试结束即删，不长期驻留夹具）。
struct TempGuard {
    dir: PathBuf,
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn temp_source(label: &str, content: &[u8]) -> (TempGuard, FileTextSource) {
    let dir = std::env::temp_dir().join(format!(
        "nr-rc5-core-{}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
        label
    ));
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    let path = dir.join("book.txt");
    std::fs::write(&path, content).expect("写入临时文件");
    let source = FileTextSource::open(&path).expect("打开临时文件");
    (TempGuard { dir }, source)
}

/// 构造含中文、全角标点与多字节字符的正文（每字符 3 字节，边界陷阱密集）。
fn chinese_book(repeats: usize) -> String {
    "第一章 墨色深处\n“你到底还是来了。”他低声说道，声音里带着三年未曾散去的寒意。\n窗外雨声渐密，灯影在墙上摇晃，字迹被雨水洇开。\n\n".repeat(repeats)
}

/// 按每页约 `target_bytes` 字节测量页面，并**向前对齐到合法字符边界**，覆盖 `[0, len)`。
///
/// 这正是平台适配层必须遵守的流程：测量结果回来先对齐字符边界，再由核心校验。
fn measured_pages(text: &str, target_bytes: usize) -> Vec<PageBoundary> {
    let mut pages = Vec::new();
    let mut start = 0usize;
    while start < text.len() {
        let mut end = (start + target_bytes).min(text.len());
        while end < text.len() && !text.is_char_boundary(end) {
            end += 1;
        }
        pages.push(PageBoundary::new(start as u64, end as u64));
        start = end;
    }
    pages
}

fn commit(
    cache: &mut PaginationCache,
    source: &FileTextSource,
    boundary: PageBoundary,
) -> reading_core::BoundaryCommitOutcome {
    let request_id = cache.begin_request();
    let request = reading_core::PageBoundaryRequest {
        layout_id: cache.layout_id(),
        request_id,
        content_index_version: cache.layout_key().content_index_version,
        total_bytes: source.len(),
        start_byte: boundary.start_byte,
        end_byte: boundary.end_byte,
    };
    cache.commit_page_boundary(source, &request, TextEncoding::Utf8)
}

#[test]
fn pagination_forward_and_backward_pages_are_contiguous_without_gap_or_overlap() {
    let text = chinese_book(160);
    let (_guard, source) = temp_source("forward-backward", text.as_bytes());
    let total = source.len();
    let mut cache = PaginationCache::new(LayoutSignature::default(), 1).expect("建立分页缓存");

    let expected = measured_pages(&text, 1_024);
    assert!(expected.len() >= 4, "必须有多页才谈得上正反向翻页");

    // 正向：逐页提交（模拟平台文本测量结果回来后的提交）。
    for boundary in &expected {
        let outcome = commit(&mut cache, &source, *boundary);
        assert_eq!(
            outcome.status,
            BoundaryCommitStatus::Committed,
            "{outcome:?}"
        );
        assert_eq!(outcome.error_code, 0);
        // 每个 end 都必须真的落在字符边界（重新从数据源核对，而不是相信构造器）。
        assert!(is_char_boundary_at(&source, 0, boundary.end_byte, TextEncoding::Utf8).expect("边界探测"));
        validate_page_boundary(&source, boundary, total, TextEncoding::Utf8).expect("提交的边界必须合法");
    }

    // 连续性：相邻页面首尾相接，既不丢字也不重字。
    let chain = cache.page_chain().expect("页面必须首尾相接");
    assert_eq!(chain.covered(), (0, total), "页面必须完整覆盖内容");
    chain.assert_contiguous().expect("整链必须严格前进");
    assert_eq!(chain.len(), expected.len());

    // 逐字节重建：所有页面字节区间拼接后必须与原文件**逐字节相等**（最强的无丢字/无重字断言）。
    let bytes = text.as_bytes();
    let mut rebuilt = Vec::with_capacity(total as usize);
    for boundary in chain.pages() {
        rebuilt.extend_from_slice(&bytes[boundary.start_byte as usize..boundary.end_byte as usize]);
    }
    assert_eq!(rebuilt, bytes, "页面拼接必须与原文逐字节一致");
    let covered_bytes: u64 = chain.pages().iter().map(|page| page.len_bytes()).sum();
    assert_eq!(covered_bytes, total, "页面长度之和必须等于文件长度");

    // 反向：从最后一页往回翻，必须回到同一批边界（不会出现漂移或重复）。
    let mut backward = Vec::new();
    for index in (0..chain.len()).rev() {
        let page = chain.page(index).expect("页面存在");
        backward.push(*page);
        assert_eq!(cache.page_index_for_offset(page.start_byte), Some(index));
    }
    backward.reverse();
    assert_eq!(backward, expected, "反向翻页必须与正向边界完全一致");
    assert_eq!(chain.next_of(0), Some(&expected[1]));
    assert_eq!(chain.previous_of(2), Some(&expected[1]));
    assert_eq!(chain.previous_of(0), None);
    assert_eq!(chain.next_of(chain.len() - 1), None);
}

#[test]
fn page_boundary_rejects_illegal_intervals_and_mid_char_end() {
    // 每个汉字 3 字节：偏移 1、2 落在字符内部。
    let text = "中文正文内容，标点齐全。".repeat(40);
    let (_guard, source) = temp_source("illegal-boundary", text.as_bytes());
    let total = source.len();
    let mut cache = PaginationCache::new(LayoutSignature::default(), 1).expect("建立分页缓存");

    // 1) end 落在字符中间 → Utf8Boundary(3)。
    let mid_char = commit(&mut cache, &source, PageBoundary::new(0, 1));
    assert!(mid_char.is_rejected());
    assert_eq!(
        mid_char.rejection,
        Some(BoundaryRejection::IllegalCharBoundary)
    );
    assert_eq!(
        mid_char.error_code,
        ErrorCode::Utf8Boundary.as_c(),
        "字符中间必须映射为 NR_ERR_UTF8_BOUNDARY"
    );
    let mid_tail = commit(&mut cache, &source, PageBoundary::new(0, total - 1));
    assert_eq!(
        mid_tail.rejection,
        Some(BoundaryRejection::IllegalCharBoundary),
        "文件末尾前一字节同样可能落在字符中间"
    );

    // 2) end == start（死循环隐患）→ ReversedInterval/EmptyInterval，必须拒绝。
    let zero = commit(&mut cache, &source, PageBoundary::new(9, 9));
    assert_eq!(zero.status, BoundaryCommitStatus::Rejected);
    assert_eq!(zero.rejection, Some(BoundaryRejection::EmptyInterval));
    let reversed = commit(&mut cache, &source, PageBoundary::new(30, 12));
    assert_eq!(
        reversed.rejection,
        Some(BoundaryRejection::ReversedInterval)
    );

    // 3) 越界 → OutOfRange(2)。
    let beyond = commit(&mut cache, &source, PageBoundary::new(0, total + 3));
    assert_eq!(beyond.rejection, Some(BoundaryRejection::OutOfRange));
    assert_eq!(beyond.error_code, ErrorCode::OutOfRange.as_c());

    // 4) 反复提交零长度页面不会推进任何状态（不得死循环、不得污染缓存）。
    for _ in 0..64 {
        let zero_length = commit(&mut cache, &source, PageBoundary::new(0, 0));
        assert_eq!(zero_length.status, BoundaryCommitStatus::Rejected);
    }
    assert!(cache.is_empty(), "被拒的边界不得写入缓存");

    // 5) 链也拒绝零长度与空隙/重叠：状态只会在合法边界上报增。
    let mut chain = PageBoundaryChain::new(0);
    chain.push(PageBoundary::new(0, 9)).expect("首页合法");
    let stall = chain
        .push(PageBoundary::new(9, 9))
        .expect_err("零长度必须被拒");
    assert_eq!(stall.rejection(), BoundaryRejection::EmptyInterval);
    assert_eq!(chain.len(), 1, "非法页面不得进入链");
    let gap = chain
        .push(PageBoundary::new(12, 21))
        .expect_err("空隙必须被拒");
    assert_eq!(gap.rejection(), BoundaryRejection::GapBeforePrevious);
    let overlap = chain
        .push(PageBoundary::new(3, 15))
        .expect_err("重叠必须被拒");
    assert_eq!(overlap.rejection(), BoundaryRejection::OverlapWithPrevious);
    assert_eq!(chain.len(), 1);
}

#[test]
fn layout_signature_change_invalidates_all_cached_boundaries() {
    let text = chinese_book(60);
    let (_guard, source) = temp_source("layout-change", text.as_bytes());
    let mut cache = PaginationCache::new(LayoutSignature::default(), 1).expect("建立分页缓存");

    let pages = measured_pages(&text, 512);
    for boundary in &pages {
        assert!(commit(&mut cache, &source, *boundary).is_committed());
    }
    assert_eq!(cache.len(), pages.len());
    let old_layout_id = cache.layout_id();

    // 幂等：同一签名 + 同一内容索引版本的通知不得清空缓存。
    let unchanged = cache
        .set_layout(cache.signature().clone(), 1)
        .expect("同名重设");
    assert!(!unchanged.changed);
    assert_eq!(unchanged.removed, 0);
    assert_eq!(cache.len(), pages.len());

    // 字号变化 → 旧布局边界全部失效。
    let bigger = LayoutSignature {
        font_size_milli: 32_000,
        ..cache.signature().clone()
    };
    let change = cache.set_layout(bigger.clone(), 1).expect("字号变化");
    assert!(change.changed);
    assert_eq!(change.removed, pages.len(), "签名变化必须清空全部旧边界");
    assert_eq!(change.layout_id, old_layout_id + 1);
    assert!(cache.is_empty());
    for boundary in &pages {
        assert_eq!(
            cache.query_page_boundary(old_layout_id, boundary.start_byte),
            None,
            "旧世代边界必须全部不可查"
        );
    }

    // 视口变化（窗口尺寸改变）同样失效；方向变化亦然。
    for mutated in [
        LayoutSignature {
            viewport_width: 800,
            ..bigger.clone()
        },
        LayoutSignature {
            viewport_height: 1280,
            ..bigger.clone()
        },
        LayoutSignature {
            direction: TextDirection::Vertical,
            ..bigger.clone()
        },
        LayoutSignature {
            font_family: "Noto Serif CJK SC".to_string(),
            ..bigger.clone()
        },
        LayoutSignature {
            line_spacing_milli: 12_000,
            ..bigger.clone()
        },
        LayoutSignature {
            paragraph_spacing_milli: 16_000,
            ..bigger.clone()
        },
        LayoutSignature {
            first_line_indent_milli: 0,
            ..bigger.clone()
        },
        LayoutSignature {
            margin_horizontal_milli: 40_000,
            ..bigger.clone()
        },
        LayoutSignature {
            margin_vertical_milli: 40_000,
            ..bigger.clone()
        },
        LayoutSignature {
            algorithm_version: LAYOUT_ALGORITHM_VERSION + 1,
            ..bigger.clone()
        },
        LayoutSignature {
            platform_font_fingerprint: 0xABCD,
            ..bigger.clone()
        },
    ] {
        cache.set_layout(bigger.clone(), 1).expect("先回到基准签名");
        assert!(commit(&mut cache, &source, pages[0]).is_committed());
        let outcome = cache.set_layout(mutated.clone(), 1).expect("签名变化");
        assert!(outcome.changed, "该字段变化必须被识别: {mutated:?}");
        assert_eq!(outcome.removed, 1);
        assert!(cache.is_empty());
    }

    // 内容索引版本变化（章节索引重建）同样使缓存键失效，但不动用户位置。
    cache.set_layout(bigger.clone(), 1).expect("回到基准签名");
    assert!(commit(&mut cache, &source, pages[0]).is_committed());
    let version_change = cache.set_layout(bigger, 2).expect("内容索引版本变化");
    assert!(version_change.changed);
    assert_eq!(version_change.removed, 1);

    // 位置身份与页码无关：重建索引不影响 ContentPosition 的字段。
    let head: String = text.chars().take(12).collect();
    let position = ContentPosition::at(9, 1, 0, pages[0].start_byte, &head);
    let decoded = ContentPosition::decode(&position.encode()).expect("位置往返");
    assert_eq!(decoded, position);
}

#[test]
fn invalidate_by_signature_bumps_generation_and_drops_inflight() {
    let text = chinese_book(40);
    let (_guard, source) = temp_source("invalidate-by-signature", text.as_bytes());
    let mut cache = PaginationCache::new(LayoutSignature::default(), 1).expect("建立分页缓存");
    let signature = cache.signature().clone();

    let pages = measured_pages(&text, 300);
    for boundary in &pages {
        assert!(commit(&mut cache, &source, *boundary).is_committed());
    }
    let previous_layout_id = cache.layout_id();
    assert_eq!(cache.len(), pages.len());

    // 按签名失效：删除全部匹配边界，并递增排版世代（在途结果随之过期）。
    let invalidation = cache.invalidate(&signature);
    assert_eq!(invalidation.removed, pages.len());
    assert!(invalidation.generation_bumped);
    assert_eq!(invalidation.invalidated_layout_id, Some(previous_layout_id));
    assert_ne!(invalidation.layout_id, previous_layout_id);
    assert!(cache.is_empty());
    assert_eq!(invalidation.layout_id, cache.layout_id());
    assert!(cache.latest_request_id() >= 1);

    // 不匹配签名的失效：既不删边界也不递增世代。
    let other = LayoutSignature {
        font_size_milli: 26_000,
        ..signature
    };
    let request_id = cache.begin_request();
    let current = reading_core::PageBoundaryRequest {
        layout_id: cache.layout_id(),
        request_id,
        content_index_version: cache.layout_key().content_index_version,
        total_bytes: source.len(),
        start_byte: pages[0].start_byte,
        end_byte: pages[0].end_byte,
    };
    assert!(cache.commit_page_boundary(&source, &current, TextEncoding::Utf8).is_committed());
    let noop_same_generation = cache.invalidate(&other);
    assert_eq!(noop_same_generation.removed, 0);
    assert!(!noop_same_generation.generation_bumped);
    assert_eq!(noop_same_generation.layout_id, cache.layout_id());
    assert_eq!(cache.len(), 1, "不匹配签名不得删掉任何边界");

    // 在途结果：携带失效前的 layout_id 提交 → 丢弃，且不写入缓存。
    let in_flight = reading_core::PageBoundaryRequest {
        layout_id: previous_layout_id,
        request_id: cache.begin_request(),
        content_index_version: cache.layout_key().content_index_version,
        total_bytes: source.len(),
        start_byte: pages[1].start_byte,
        end_byte: pages[1].end_byte,
    };
    let dropped = cache.commit_page_boundary(&source, &in_flight, TextEncoding::Utf8);
    assert_eq!(dropped.status, BoundaryCommitStatus::StaleLayoutId);
    assert!(dropped.is_stale());
    assert_eq!(dropped.stale_layout_id, Some(previous_layout_id));
    assert_eq!(dropped.boundary, None);
    assert_eq!(cache.len(), 1, "过期结果不得改变缓存");

    // invalidate_all 也会递增世代并清空。
    let all = cache.invalidate_all();
    assert_eq!(all.removed, 1);
    assert!(all.generation_bumped);
    assert!(cache.is_empty());
}

#[test]
fn stale_layout_result_rejected_by_layout_id_and_request_id() {
    let text = chinese_book(50);
    let (_guard, source) = temp_source("stale-layout", text.as_bytes());
    let mut cache = PaginationCache::new(LayoutSignature::default(), 1).expect("建立分页缓存");
    let pages = measured_pages(&text, 512);

    // 1) request_id 过期：更新的请求已发出，旧请求的结果必须被丢弃。
    let stale_request_id = cache.begin_request();
    let newest_request_id = cache.begin_request();
    assert!(newest_request_id > stale_request_id);
    let stale_old = reading_core::PageBoundaryRequest {
        layout_id: cache.layout_id(),
        request_id: stale_request_id,
        content_index_version: cache.layout_key().content_index_version,
        total_bytes: source.len(),
        start_byte: pages[0].start_byte,
        end_byte: pages[0].end_byte,
    };
    let rejected = cache.commit_page_boundary(&source, &stale_old, TextEncoding::Utf8);
    assert_eq!(rejected.status, BoundaryCommitStatus::StaleRequestId);
    assert!(rejected.is_stale());
    assert_eq!(rejected.stale_request_id, Some(stale_request_id));
    assert_eq!(rejected.error_code, ErrorCode::Cancelled.as_c());
    assert!(cache.is_empty(), "过期请求的结果不得写入缓存");

    let newest = reading_core::PageBoundaryRequest {
        request_id: newest_request_id,
        ..stale_old.clone()
    };
    assert!(cache.commit_page_boundary(&source, &newest, TextEncoding::Utf8).is_committed());

    // 2) layout_id 过期：重排后旧世代的结果必须被丢弃。
    let previous_layout_id = cache.layout_id();
    let reflowed = cache
        .set_layout(
            LayoutSignature {
                font_size_milli: 28_000,
                ..cache.signature().clone()
            },
            1,
        )
        .expect("重排");
    assert!(reflowed.changed);
    let stale_layout = reading_core::PageBoundaryRequest {
        layout_id: previous_layout_id,
        request_id: cache.begin_request(),
        content_index_version: cache.layout_key().content_index_version,
        total_bytes: source.len(),
        start_byte: pages[0].start_byte,
        end_byte: pages[0].end_byte,
    };
    let dropped = cache.commit_page_boundary(&source, &stale_layout, TextEncoding::Utf8);
    assert_eq!(dropped.status, BoundaryCommitStatus::StaleLayoutId);
    assert_eq!(dropped.error_code, ErrorCode::LayoutNotFound.as_c());
    assert_eq!(dropped.current_layout_id, reflowed.layout_id);
    assert!(cache.is_empty());

    // 3) content_index_version 过期：索引重建后的结果同样必须被丢弃。
    let stale_index = reading_core::PageBoundaryRequest {
        layout_id: cache.layout_id(),
        request_id: cache.begin_request(),
        content_index_version: 99,
        ..stale_layout.clone()
    };
    let dropped_index = cache.commit_page_boundary(&source, &stale_index, TextEncoding::Utf8);
    assert_eq!(
        dropped_index.status,
        BoundaryCommitStatus::StaleContentIndexVersion
    );
    assert!(dropped_index.is_stale());
    assert!(cache.is_empty());

    // 4) 在新世代下用正确参数提交仍然成功（丢弃只针对过期结果）。
    let fresh = reading_core::PageBoundaryRequest {
        layout_id: cache.layout_id(),
        content_index_version: cache.layout_key().content_index_version,
        request_id: cache.begin_request(),
        total_bytes: source.len(),
        start_byte: pages[0].start_byte,
        end_byte: pages[0].end_byte,
    };
    let committed = cache.commit_page_boundary(&source, &fresh, TextEncoding::Utf8);
    assert!(committed.is_committed(), "{committed:?}");
    assert_eq!(cache.len(), 1);
    assert_eq!(
        cache.query_page_boundary(cache.layout_id(), fresh.start_byte),
        Some(PageBoundary::new(fresh.start_byte, fresh.end_byte))
    );
}

#[test]
fn pagination_boundary_codes_match_frozen_contract_not_percentage() {
    // 版本与冻结字段数（gate-0 契约基线）。
    let pagination_version: u32 = PAGINATION_VERSION;
    let layout_fields: usize = LAYOUT_SIGNATURE_FIELDS;
    let algorithm_version: u32 = LAYOUT_ALGORITHM_VERSION;
    assert_eq!(pagination_version, 1);
    assert_eq!(layout_fields, 13);
    assert_eq!(algorithm_version, 1);

    // 边界拒绝原因 → NR_ERR_* 数值。
    let illegal: i32 = BoundaryRejection::IllegalCharBoundary.code();
    let out_of_range: i32 = BoundaryRejection::OutOfRange.code();
    let empty: i32 = BoundaryRejection::EmptyInterval.code();
    let gap: i32 = BoundaryRejection::GapBeforePrevious.code();
    assert_eq!(illegal, ErrorCode::Utf8Boundary.as_c());
    assert_eq!(out_of_range, ErrorCode::OutOfRange.as_c());
    assert_eq!(empty, ErrorCode::InvalidArgument.as_c());
    assert_eq!(gap, ErrorCode::InvalidArgument.as_c());

    // 提交状态 → NR_ERR_* 数值。
    let committed: i32 = BoundaryCommitStatus::Committed.code();
    let duplicate: i32 = BoundaryCommitStatus::DuplicateIdentical.code();
    let stale_layout: i32 = BoundaryCommitStatus::StaleLayoutId.code();
    let stale_request: i32 = BoundaryCommitStatus::StaleRequestId.code();
    assert_eq!(committed, ErrorCode::Ok.as_c());
    assert_eq!(duplicate, ErrorCode::Ok.as_c());
    assert_eq!(stale_layout, ErrorCode::LayoutNotFound.as_c());
    assert_eq!(stale_request, ErrorCode::Cancelled.as_c());
    assert!(BoundaryCommitStatus::Committed.is_committed());
    assert!(BoundaryCommitStatus::DuplicateIdentical.is_committed());
    assert!(BoundaryCommitStatus::StaleLayoutId.is_stale());
    assert!(BoundaryCommitStatus::StaleContentIndexVersion.is_stale());
    assert!(!BoundaryCommitStatus::Rejected.is_stale());

    // 页码精确：切换点就是边界起点，绝不是按字节百分比的估算。
    let text = "The quick brown fox jumps over the lazy dog.\n".repeat(2_000);
    let (_guard, source) = temp_source("exact-index", text.as_bytes());
    let total = source.len();
    let mut cache = PaginationCache::new(LayoutSignature::default(), 1).expect("建立分页缓存");
    // 不等长页面，故意让「等分近似」与真实边界错开。
    let sizes = [200u64, 400, 800, 1_600, total - 3_000];
    let mut start = 0u64;
    for size in sizes {
        let end = start + size;
        assert!(commit(&mut cache, &source, PageBoundary::new(start, end)).is_committed());
        start = end;
    }
    assert_eq!(start, total);
    let pages = cache.len();
    let equal_share = total / pages as u64;
    assert_ne!(
        cache.ordered_boundaries()[1].start_byte,
        equal_share,
        "精确切换点必须等于真实边界，而不是等分/百分比位置"
    );
    assert_eq!(cache.ordered_boundaries()[1].start_byte, 200);
    assert_eq!(cache.page_index_for_offset(199), Some(0));
    assert_eq!(cache.page_index_for_offset(200), Some(1));
    assert_eq!(cache.page_index_for_offset(total - 1), Some(pages - 1));
    assert_eq!(cache.page_index_for_offset(total), None);

    // 单窗口语义未被本卡放大：一次最多物化 64 KiB（默认窗口），远小于整本。
    let window: usize = DEFAULT_WINDOW_BYTES;
    assert_eq!(window, 65_536);
    assert!(total > window as u64, "内容应大于一个窗口以体现有限读取");
    let window_read = source.read_at(0, window).expect("窗口读取");
    assert!(window_read.raw_bytes() <= window);
}

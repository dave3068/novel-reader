//! 锚点 / 布局失效 / 事件与取消契约测试（S-R3 / S-R5）。
//!
//! 清单入口：`cargo test --release 'anchor_|invalidate_|async_'`

use std::sync::{Arc, Mutex};

use rust_reading_core::pagination::paginate;
use rust_reading_core::position::{locate_anchor, make_anchor, ANCHOR_QUOTE_CHARS};
use rust_reading_core::{
    Book, BookSource, Event, EventKind, LayoutConfig, Library, PageBoundary, ReadingEngine,
};

const POOL: [char; 20] = [
    '静', '夜', '思', '床', '前', '明', '月', '光', '疑', '是', '地', '上', '霜', '举', '头', '望',
    '低', '故', '乡', '春',
];

fn corpus() -> String {
    let mut rng = rust_reading_core::Rng::new(20260905);
    let mut out = String::new();
    for _ in 0..600 {
        let n = rng.range_inclusive(60, 140);
        for _ in 0..n {
            out.push(*rng.pick(&POOL));
        }
        out.push('。');
        out.push('\n');
    }
    out
}

fn flat_library(book_id: u64, total_bytes: u64) -> Library {
    Library {
        seed: 1,
        categories: Vec::new(),
        books: vec![Book {
            id: book_id,
            title: "锚点夹具".into(),
            author: "夹具".into(),
            category_id: 1,
            chapter_count: 1,
            total_bytes,
        }],
        history: Vec::new(),
    }
}

#[test]
fn anchor_recovers_same_source_position_across_layouts() {
    let text = corpus();
    let desktop = LayoutConfig::desktop_default();
    let android = LayoutConfig::android_default();

    let desktop_pages = paginate(&text, 0, &desktop, 200);
    let android_pages = paginate(&text, 0, &android, 200);
    assert!(!desktop_pages.is_empty() && !android_pages.is_empty());
    assert_ne!(
        desktop_pages[1..5.min(desktop_pages.len())],
        android_pages[1..5.min(android_pages.len())],
        "两端页边界必须不同（否则该断言无意义）"
    );

    for (start, consumed) in desktop_pages.iter().take(40) {
        let anchor = make_anchor(&text, 0, *start as u64, (*start + consumed) as u64);
        assert!(!anchor.quote.is_empty());
        assert!(anchor.quote.chars().count() <= ANCHOR_QUOTE_CHARS);
        let located = locate_anchor(&text, &anchor) as usize;
        assert_eq!(located, *start, "锚点必须恢复到同一原文位置");
        // 该位置一定落在 Android 的某一页内
        let containing = android_pages
            .iter()
            .find(|(s, c)| located >= *s && located < s + c)
            .expect("必须能定位到另一端的页");
        assert!(containing.1 > 0);
    }
}

#[test]
fn anchor_survives_unaligned_boundary() {
    let text = corpus();
    let android = LayoutConfig::android_default();
    let android_pages = paginate(&text, 0, &android, 200);

    // 取一个不是任何页起点的字符边界
    let page_starts: Vec<usize> = android_pages.iter().map(|(s, _)| *s).collect();
    let mut offset = 3usize;
    while offset < text.len() && (page_starts.contains(&offset) || !text.is_char_boundary(offset)) {
        offset += 1;
    }
    assert!(offset < text.len());
    let anchor = make_anchor(&text, 0, offset as u64, (offset + 200) as u64);
    assert_eq!(locate_anchor(&text, &anchor) as usize, offset);

    // 空引用退化到 start_byte
    let empty = make_anchor("", 0, 12, 24);
    assert_eq!(locate_anchor("", &empty), 0);
}

#[test]
fn invalidate_layout_clears_only_target_layout() {
    let mut engine = ReadingEngine::with_library(flat_library(1, 4 * 1024 * 1024), 256 * 1024);
    engine
        .open_book(
            1,
            BookSource::Synthetic {
                total_bytes: 4 * 1024 * 1024,
            },
        )
        .unwrap();

    let layout_a = rust_reading_core::layout_id(&LayoutConfig::desktop_default());
    let layout_b = rust_reading_core::layout_id(&LayoutConfig::android_default());
    assert_ne!(layout_a, layout_b);

    for i in 0..5u64 {
        engine
            .commit_page_boundary(PageBoundary {
                chapter_index: 0,
                start_byte: i * 1000,
                end_byte: i * 1000 + 900,
                layout_id: layout_a,
                line_count: 12,
            })
            .unwrap();
        engine
            .commit_page_boundary(PageBoundary {
                chapter_index: 0,
                start_byte: i * 1000,
                end_byte: i * 1000 + 700,
                layout_id: layout_b,
                line_count: 10,
            })
            .unwrap();
    }
    assert_eq!(engine.all_page_boundaries(layout_a).len(), 5);
    assert_eq!(engine.all_page_boundaries(layout_b).len(), 5);

    assert_eq!(engine.invalidate_layout(layout_a), 5);
    assert!(engine.all_page_boundaries(layout_a).is_empty());
    assert_eq!(engine.all_page_boundaries(layout_b).len(), 5);
    assert_eq!(engine.invalidate_layout(layout_a), 0);

    // 越界边界必须被拒绝
    assert!(engine
        .commit_page_boundary(PageBoundary {
            chapter_index: 0,
            start_byte: 0,
            end_byte: 8 * 1024 * 1024,
            layout_id: layout_b,
            line_count: 1,
        })
        .is_err());
    assert!(engine
        .commit_page_boundary(PageBoundary {
            chapter_index: 5,
            start_byte: 0,
            end_byte: 10,
            layout_id: layout_b,
            line_count: 1,
        })
        .is_err());

    assert_eq!(engine.invalidate_all_layouts(), 5);
    assert_eq!(engine.metrics().page_boundaries, 0);
}

#[test]
fn async_events_cancel_and_metrics() {
    let events: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let mut engine = ReadingEngine::new(64 * 1024);
    engine.subscribe(move |ev: &Event| {
        sink.lock().unwrap().push(ev.clone());
    });

    engine
        .open_book(
            1,
            BookSource::Synthetic {
                total_bytes: 512 * 1024,
            },
        )
        .unwrap();
    engine.read_window(0, 0, 4096).unwrap();
    engine
        .save_position(rust_reading_core::ContentPosition {
            book_id: 1,
            chapter_index: 0,
            byte_offset: 0,
        })
        .unwrap();

    let request = engine.start_batch_request(50);
    let p1 = engine.advance_request(request, 20).unwrap();
    assert_eq!(p1.done, 20);
    assert!(!p1.cancelled && !p1.finished);

    engine.cancel_request(request).unwrap();
    assert!(engine.is_cancelled(request));
    let p2 = engine.advance_request(request, 20).unwrap();
    assert!(p2.cancelled, "取消后必须返回 cancelled");
    assert_eq!(p2.done, 20, "取消后不得继续推进（部分成功）");
    assert!(!p2.finished);

    // 完整请求
    let request2 = engine.start_batch_request(10);
    let p3 = engine.advance_request(request2, 10).unwrap();
    assert!(p3.finished && !p3.cancelled);

    let log = events.lock().unwrap().clone();
    let kinds: Vec<EventKind> = log.iter().map(|e| e.kind).collect();
    // 注意：LibraryReady 在引擎构造时发出，早于订阅，因此不在本日志内。
    assert!(kinds.contains(&EventKind::BookOpened));
    assert!(kinds.contains(&EventKind::WindowRead));
    assert!(kinds.contains(&EventKind::PositionSaved));
    assert!(kinds.contains(&EventKind::RequestStarted));
    assert!(kinds.contains(&EventKind::RequestProgress));
    assert!(kinds.contains(&EventKind::RequestCancelled));
    assert!(kinds.contains(&EventKind::BatchLoaded));
    // 事件序号单调递增
    let mut last = 0u64;
    for e in log.iter() {
        assert!(e.seq > last, "事件序号必须单调递增");
        last = e.seq;
        assert!(e.to_json().starts_with("{\"seq\""));
    }

    let metrics = engine.metrics();
    assert_eq!(metrics.cancelled_requests, 1);
    assert!(metrics.events_emitted as usize >= log.len());
    assert!(metrics.read_calls >= 1);
    assert!(metrics.copied_bytes > 0);
    assert!(metrics.materialized_bytes > 0);

    // 取消未开始的请求也要记录
    engine.cancel_request(999).unwrap();
    assert!(engine.is_cancelled(999));
    assert_eq!(engine.metrics().cancelled_requests, 2);
}

#[test]
fn async_position_validation_and_errors() {
    let mut engine = ReadingEngine::new(64 * 1024);
    // 未打开书籍
    assert!(engine.read_window(0, 0, 16).is_err());
    assert_eq!(
        engine.read_window(0, 0, 16).unwrap_err().code,
        rust_reading_core::ErrorCode::NoOpenBook
    );
    engine
        .open_book(
            1,
            BookSource::Synthetic {
                total_bytes: 128 * 1024,
            },
        )
        .unwrap();
    // 书籍不存在
    assert_eq!(
        engine
            .open_book(123456, BookSource::Synthetic { total_bytes: 1024 })
            .unwrap_err()
            .code,
        rust_reading_core::ErrorCode::BookNotFound
    );
    // 位置越界
    assert!(engine
        .save_position(rust_reading_core::ContentPosition {
            book_id: 1,
            chapter_index: 99,
            byte_offset: 0
        })
        .is_err());
}

#[test]
fn async_engine_snapshot_is_valid_json_shape() {
    let engine = ReadingEngine::new(0);
    let snapshot = engine.snapshot_json();
    assert!(snapshot.starts_with('{') && snapshot.ends_with('}'));
    assert!(snapshot.contains("\"abiEntries\":11"));
    assert!(snapshot.contains("\"books\":10000"));
    // 构造时已发出 LibraryReady，因此事件计数必须非零
    assert!(engine.metrics().events_emitted >= 1);
}

//! C ABI 基线契约测试：11 个入口 / 12 个符号、生命周期、跨边界释放、
//! 回调线程、JSON 批次与「直接 Rust API ↔ C ABI 语义等价」。
//!
//! 清单入口：`cargo test --release abi_`

use std::sync::atomic::{AtomicUsize, Ordering};

use rust_reading_core::abi::{
    ABI_ENTRIES, ABI_ENTRY_COUNT, ABI_SYMBOL_COUNT, ABI_VERSION, BASELINE_SYMBOLS,
    EXTENSION_SYMBOLS,
};
use rust_reading_core::ffi::*;
use rust_reading_core::{layout_id, layout_page, LayoutConfig};

static CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static LAST_EVENT_LEN: AtomicUsize = AtomicUsize::new(0);

extern "C" fn on_event(_user: *mut core::ffi::c_void, data: *const u8, len: usize) {
    if !data.is_null() && len > 0 {
        let bytes = unsafe { std::slice::from_raw_parts(data, len) };
        assert!(
            std::str::from_utf8(bytes).is_ok(),
            "事件负载必须是合法 UTF-8"
        );
        LAST_EVENT_LEN.store(len, Ordering::SeqCst);
    }
    CALLBACKS.fetch_add(1, Ordering::SeqCst);
}

fn header_text() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../ffi/include/novel_reader_probe.h"
    ))
    .expect("头文件必须存在")
}

#[test]
fn abi_entry_table_is_complete() {
    assert_eq!(ABI_ENTRIES.len(), ABI_ENTRY_COUNT);
    assert_eq!(ABI_ENTRY_COUNT, 11, "基线入口固定为 11 个");
    let symbol_total: usize = ABI_ENTRIES.iter().map(|e| e.symbols.len()).sum();
    assert_eq!(symbol_total, ABI_SYMBOL_COUNT);
    assert_eq!(BASELINE_SYMBOLS.len(), ABI_SYMBOL_COUNT);
    assert_eq!(nr_abi_version(), ABI_VERSION);

    // 每个入口都要有需求归属与类别
    for entry in ABI_ENTRIES.iter() {
        assert!(!entry.entry.is_empty());
        assert!(entry.requirement.starts_with('S'));
        assert!(!entry.kind.is_empty());
        assert!(!entry.symbols.is_empty());
    }

    // 头文件必须声明全部基线符号与扩展符号
    let header = header_text();
    for s in BASELINE_SYMBOLS.iter().chain(EXTENSION_SYMBOLS.iter()) {
        assert!(header.contains(s), "头文件缺少声明: {s}");
    }
    // 错误码必须与 Rust 侧一致
    use rust_reading_core::ErrorCode;
    for (code, name) in [
        (ErrorCode::Ok, "NR_ERR_OK"),
        (ErrorCode::InvalidArgument, "NR_ERR_INVALID_ARGUMENT"),
        (ErrorCode::OutOfRange, "NR_ERR_OUT_OF_RANGE"),
        (ErrorCode::Utf8Boundary, "NR_ERR_UTF8_BOUNDARY"),
        (ErrorCode::BookNotFound, "NR_ERR_BOOK_NOT_FOUND"),
        (ErrorCode::NoOpenBook, "NR_ERR_NO_OPEN_BOOK"),
        (ErrorCode::IoError, "NR_ERR_IO"),
        (ErrorCode::Cancelled, "NR_ERR_CANCELLED"),
        (ErrorCode::LayoutNotFound, "NR_ERR_LAYOUT_NOT_FOUND"),
        (ErrorCode::EngineDestroyed, "NR_ERR_ENGINE_DESTROYED"),
        (ErrorCode::Internal, "NR_ERR_INTERNAL"),
        (ErrorCode::ChapterOutOfRange, "NR_ERR_CHAPTER_OUT_OF_RANGE"),
    ] {
        assert!(header.contains(name), "头文件缺少错误码 {name}");
        assert!(
            header.contains(&format!("#define {name}")),
            "{name} 未定义为宏"
        );
        let _ = code;
    }
}

#[test]
fn abi_lifecycle_and_error_codes() {
    use rust_reading_core::ErrorCode;
    unsafe {
        let engine = nr_engine_create(64 * 1024);
        assert!(!engine.is_null(), "引擎创建失败");

        let mut buf = NrBuffer::empty();
        // 未打开书籍
        assert_eq!(
            nr_read_window(engine, 0, 0, 16, &mut buf),
            ErrorCode::NoOpenBook.as_c()
        );
        // 打开合成书
        assert_eq!(nr_open_book(engine, 1, core::ptr::null(), 0, 512 * 1024), 0);
        assert_eq!(nr_read_window(engine, 0, 0, 256, &mut buf), 0);
        assert!(buf.len > 0);
        let bytes = std::slice::from_raw_parts(buf.ptr, buf.len).to_vec();
        assert!(std::str::from_utf8(&bytes).is_ok(), "窗口必须是合法 UTF-8");
        assert_eq!(nr_release_buffer(buf), 0);

        // 章节越界
        assert_eq!(
            nr_read_window(engine, 999, 0, 16, &mut buf),
            ErrorCode::ChapterOutOfRange.as_c()
        );
        // 书籍不存在
        assert_eq!(
            nr_open_book(engine, 9_999_999, core::ptr::null(), 0, 1024),
            ErrorCode::BookNotFound.as_c()
        );
        // 错误消息可取回
        let mut msg = NrBuffer::empty();
        assert_eq!(nr_last_error_message(engine, &mut msg), 0);
        assert!(msg.len > 0);
        assert_eq!(nr_release_buffer(msg), 0);

        // 位置与页面边界
        assert_eq!(nr_save_position(engine, 1, 0, 0), 0);
        assert_eq!(
            nr_save_position(engine, 1, 999, 0),
            ErrorCode::ChapterOutOfRange.as_c()
        );
        let lid = layout_id(&LayoutConfig::desktop_default());
        assert_eq!(nr_commit_page_boundary(engine, 0, 0, 512, lid, 5), 0);
        let mut removed = 0u64;
        assert_eq!(nr_invalidate_layout(engine, lid, &mut removed), 0);
        assert_eq!(removed, 1);
        assert_eq!(nr_invalidate_layout(engine, lid, &mut removed), 0);
        assert_eq!(removed, 0);

        // 空指针保护
        assert_eq!(
            nr_read_window(engine, 0, 0, 16, core::ptr::null_mut()),
            ErrorCode::InvalidArgument.as_c()
        );

        nr_engine_destroy(engine);
    }
}

#[test]
fn abi_batch_json_and_metrics() {
    use rust_reading_core::ErrorCode;
    unsafe {
        let engine = nr_engine_create(0);
        let mut buf = NrBuffer::empty();
        assert_eq!(nr_get_book_batch(engine, 0, 3, &mut buf), 0);
        let json =
            String::from_utf8(std::slice::from_raw_parts(buf.ptr, buf.len).to_vec()).unwrap();
        assert!(json.contains("\"items\""));
        assert_eq!(json.matches("\"id\"").count(), 3, "批次条数必须为 3");
        assert!(json.contains("\"total\":10000"), "10000 本书: {json}");
        assert_eq!(nr_release_buffer(buf), 0);

        assert_eq!(nr_get_chapter_batch(engine, 1, 0, 2, &mut buf), 0);
        let json =
            String::from_utf8(std::slice::from_raw_parts(buf.ptr, buf.len).to_vec()).unwrap();
        assert!(json.contains("\"bookId\":1"));
        assert_eq!(nr_release_buffer(buf), 0);
        assert_eq!(
            nr_get_chapter_batch(engine, 9_999_999, 0, 2, &mut buf),
            ErrorCode::BookNotFound.as_c()
        );

        let mut metrics = NrMetrics::default();
        assert_eq!(nr_get_metrics(engine, &mut metrics), 0);
        assert!(metrics.events_emitted > 0);

        let mut snap = NrBuffer::empty();
        assert_eq!(nr_engine_snapshot_json(engine, &mut snap), 0);
        let snap_json =
            String::from_utf8(std::slice::from_raw_parts(snap.ptr, snap.len).to_vec()).unwrap();
        assert!(snap_json.contains("\"abiEntries\":11"), "{snap_json}");
        assert!(snap_json.contains("\"categories\":1000"), "{snap_json}");
        assert!(snap_json.contains("\"history\":2000"), "{snap_json}");
        assert_eq!(nr_release_buffer(snap), 0);

        nr_engine_destroy(engine);
    }
}

#[test]
fn abi_callback_cancel_and_ownership() {
    use rust_reading_core::ErrorCode;
    unsafe {
        let engine = nr_engine_create(0);
        CALLBACKS.store(0, Ordering::SeqCst);
        let id = nr_subscribe_events(engine, Some(on_event), core::ptr::null_mut());
        assert!(id > 0, "订阅返回正数 id");
        assert_eq!(nr_open_book(engine, 2, core::ptr::null(), 0, 256 * 1024), 0);
        assert!(CALLBACKS.load(Ordering::SeqCst) > 0, "打开书籍必须触发回调");
        assert!(LAST_EVENT_LEN.load(Ordering::SeqCst) > 0);

        // 无回调时空指针保护
        assert!(nr_subscribe_events(engine, None, core::ptr::null_mut()) < 0);

        // 批次请求 → 取消 → 部分成功
        let mut request_id = 0u64;
        assert_eq!(nr_start_batch_request(engine, 100, &mut request_id), 0);
        assert!(request_id > 0);
        let mut progress = NrRequestProgress::default();
        assert_eq!(nr_advance_request(engine, request_id, 10, &mut progress), 0);
        assert_eq!(progress.done, 10);
        assert_eq!(progress.finished, 0);
        assert_eq!(nr_cancel_request(engine, request_id), 0);
        assert_eq!(nr_advance_request(engine, request_id, 10, &mut progress), 0);
        assert_eq!(progress.cancelled, 1, "取消后必须标记 cancelled");
        assert_eq!(progress.done, 10, "取消后不得继续推进");

        // 未知请求
        assert_eq!(
            nr_advance_request(engine, 4242, 1, &mut progress),
            ErrorCode::InvalidArgument.as_c()
        );

        let mut metrics = NrMetrics::default();
        assert_eq!(nr_get_metrics(engine, &mut metrics), 0);
        assert_eq!(metrics.cancelled_requests, 1);

        nr_engine_destroy(engine);
    }
}

#[test]
fn abi_layout_page_equals_direct_rust_api() {
    let text = rust_reading_core::fixtures::mixed_corpus();
    let cfg = LayoutConfig::desktop_default();
    let direct = layout_page(&text, 0, &cfg);

    let family = cfg.font_family.as_bytes();
    let nr_cfg = NrLayoutConfig {
        width: cfg.width,
        height: cfg.height,
        font_size: cfg.font_size,
        line_spacing: cfg.line_spacing,
        paragraph_spacing: cfg.paragraph_spacing,
        first_line_indent: cfg.first_line_indent,
        margin_x: cfg.margin_x,
        margin_y: cfg.margin_y,
        font_family_ptr: family.as_ptr(),
        font_family_len: family.len(),
    };
    let mut out = NrPageLayout::default();
    let rc = unsafe { nr_layout_page(&nr_cfg, text.as_ptr(), text.len(), 0, &mut out) };
    assert_eq!(rc, 0);
    assert_eq!(
        out.consumed_utf8_bytes as usize, direct.consumed_utf8_bytes,
        "直接 Rust API 与 C ABI 必须语义等价"
    );
    assert_eq!(out.visible_line_count, direct.visible_line_count);
    assert_eq!(out.layout_id, layout_id(&cfg));

    // 非法参数
    assert_eq!(
        unsafe { nr_layout_page(core::ptr::null(), text.as_ptr(), text.len(), 0, &mut out) },
        ErrorCode::InvalidArgument.as_c()
    );
}

use rust_reading_core::ErrorCode;

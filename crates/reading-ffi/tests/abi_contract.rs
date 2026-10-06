//! ABI 端到端契约测试：用真实入口、真实文件与真实 SQLite 走完整条链路。
//!
//! 覆盖卡片 DoD：句柄生命周期、错误码、缓冲释放、取消、窗口上限、UTF-8 边界、
//! 页边界提交/查询/失效、位置保存与读取、章节批次、书签、平台窗口协议。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use reading_core::{BoundaryCommitStatus, ErrorCode, LAYOUT_ALGORITHM_VERSION};
use reading_ffi::abi::*;
use reading_index::boundary::BoundaryRejection;

use reading_ffi::buffer::NrBuffer;
use reading_ffi::exports::*;
use reading_ffi::external::window_provider::MemoryWindowProvider;
use reading_ffi::structs::*;

/// 错误码与拒绝原因数值直接取自共享核心的冻结枚举（测试里不手写数字）。
const ERR_INVALID_ARGUMENT: i32 = ErrorCode::InvalidArgument.as_c();
const ERR_OUT_OF_RANGE: i32 = ErrorCode::OutOfRange.as_c();
const ERR_BOOK_NOT_FOUND: i32 = ErrorCode::BookNotFound.as_c();
const ERR_NO_OPEN_BOOK: i32 = ErrorCode::NoOpenBook.as_c();
const ERR_CANCELLED: i32 = ErrorCode::Cancelled.as_c();
const ERR_LAYOUT_NOT_FOUND: i32 = ErrorCode::LayoutNotFound.as_c();
const ERR_ENGINE_DESTROYED: i32 = ErrorCode::EngineDestroyed.as_c();
const ERR_CHAPTER_OUT_OF_RANGE: i32 = ErrorCode::ChapterOutOfRange.as_c();
const REJECT_EMPTY_INTERVAL: u32 = rejection_id(BoundaryRejection::EmptyInterval);
const BOUNDARY_COMMITTED: u32 = BoundaryCommitStatus::Committed as u32;
const BOUNDARY_DUPLICATE_IDENTICAL: u32 = BoundaryCommitStatus::DuplicateIdentical as u32;
const BOUNDARY_REJECTED: u32 = BoundaryCommitStatus::Rejected as u32;
const BOUNDARY_STALE_LAYOUT_ID: u32 = BoundaryCommitStatus::StaleLayoutId as u32;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 临时书籍文件（Drop 时清理）。
struct TempBook {
    dir: PathBuf,
    path: PathBuf,
}

impl TempBook {
    fn new(content: &str) -> Self {
        Self::from_bytes(content.as_bytes().to_vec())
    }

    fn from_bytes(bytes: Vec<u8>) -> Self {
        let unique = format!(
            "nr-rc7-abi-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let dir = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&dir).expect("创建临时目录");
        let path = dir.join("book.txt");
        std::fs::write(&path, &bytes).expect("写入临时文件");
        Self { dir, path }
    }

    fn path_str(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }
}

impl Drop for TempBook {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn chapter_text() -> String {
    let mut text = String::new();
    for index in 1..=5 {
        text.push_str(&format!("第{index}章 章节标题{index}\n"));
        for line in 0..20 {
            text.push_str(&format!(
                "这是第 {index} 章的第 {line} 行正文内容，用于验证中文分页与窗口读取。\n"
            ));
        }
    }
    text
}

fn engine_config(db_path: Option<&str>) -> NrEngineConfig {
    NrEngineConfig {
        struct_size: std::mem::size_of::<NrEngineConfig>() as u32,
        abi_version: ABI_VERSION,
        max_window_bytes: 0,
        provider: std::ptr::null(),
        db_path_utf8: db_path.map_or(std::ptr::null(), |path| path.as_ptr()),
        db_path_len: db_path.map_or(0, |path| path.len()),
    }
}

/// 建立内存引擎。
fn create_engine() -> u64 {
    let config = engine_config(None);
    let mut handle = 0u64;
    let code = unsafe { nr_engine_create(&config, &mut handle) };
    assert_eq!(code, 0, "创建引擎必须成功");
    assert!(handle > 0, "句柄必须为正整数");
    handle
}

/// 打开一个文件书籍，返回打开结果。
fn open_book(engine: u64, book: &TempBook) -> NrOpenResult {
    let identity = book.path_str();
    let request = NrOpenRequest {
        struct_size: std::mem::size_of::<NrOpenRequest>() as u32,
        kind: SOURCE_KIND_FILE,
        total_bytes: 0,
        identity_utf8: identity.as_ptr(),
        identity_len: identity.len(),
        display_name_utf8: std::ptr::null(),
        display_name_len: 0,
    };
    let mut result = NrOpenResult::default();
    let code = unsafe { nr_open_txt(engine, &request, &mut result) };
    assert_eq!(code, 0, "打开 TXT 必须成功");
    result
}

fn layout_signature() -> (NrLayoutSignature, String) {
    let family = "Noto Sans CJK SC".to_string();
    let signature = NrLayoutSignature {
        viewport_width: 1080,
        viewport_height: 1920,
        direction: 0,
        font_weight: 400,
        font_size_milli: 20_000,
        line_spacing_milli: 8_000,
        paragraph_spacing_milli: 12_000,
        first_line_indent_milli: 40_000,
        margin_horizontal_milli: 24_000,
        margin_vertical_milli: 24_000,
        algorithm_version: LAYOUT_ALGORITHM_VERSION,
        reserved0: 0,
        platform_font_fingerprint: 0,
        font_family_utf8: family.as_ptr(),
        font_family_len: family.len(),
    };
    (signature, family)
}

#[allow(clippy::too_many_arguments)]
fn submission(
    phase: u32,
    civ: u64,
    layout_id: u64,
    request_id: u64,
    total_bytes: u64,
    start: u64,
    end: u64,
    signature: Option<&NrLayoutSignature>,
) -> NrPageBoundarySubmission {
    NrPageBoundarySubmission {
        struct_size: std::mem::size_of::<NrPageBoundarySubmission>() as u32,
        phase,
        content_index_version: civ,
        layout_id,
        request_id,
        total_bytes,
        start_byte: start,
        end_byte: end,
        signature: signature.map_or(std::ptr::null(), |value| value),
    }
}

#[test]
fn all_entries_are_linkable() {
    // 每个入口都被引用一次：删除或改名任一入口都会在此处编译失败。
    let _ = nr_abi_version;
    let _ = nr_engine_create;
    let _ = nr_engine_destroy;
    let _ = nr_open_txt;
    let _ = nr_close_book;
    let _ = nr_read_window;
    let _ = nr_submit_page_boundary;
    let _ = nr_get_page_boundary;
    let _ = nr_save_position;
    let _ = nr_get_position;
    let _ = nr_get_chapters;
    let _ = nr_create_bookmark;
    let _ = nr_delete_bookmark;
    let _ = nr_invalidate_layout;
    let _ = nr_rebuild_index;
    let _ = nr_set_encoding;
    // ---- V1.1 分类（加性契约修订）----
    let _ = nr_list_categories;
    let _ = nr_create_category;
    let _ = nr_move_book_to_category;
    let _ = nr_delete_category;
    // ---- V1.1 阅读历史（加性契约修订）----
    let _ = nr_list_history;
    let _ = nr_clear_history_entry;
    let _ = nr_clear_history_for_book;
    // ---- V1.1 自定义配色（加性契约修订）----
    let _ = nr_get_theme_base;
    let _ = nr_set_theme_base;
    let _ = nr_set_theme_enabled;
    let _ = nr_get_theme_token;
    let _ = nr_get_theme_contrast;
    // ---- V1.1 全局背景图（加性契约修订）----
    let _ = nr_get_background;
    let _ = nr_set_background;
    let _ = nr_set_background_scrim;
    let _ = nr_clear_background;
    // ---- V1.1 书籍分类映射（筛选前置）----
    let _ = nr_category_move_targets;
    let _ = nr_rename_category;
    let _ = nr_move_category;
    let _ = nr_list_books_in_scope;
    let _ = nr_list_book_categories;
    let _ = nr_cancel_request;
    let _ = nr_get_last_error;
    let _ = nr_release_buffer;
    assert_eq!(nr_abi_version(), ABI_VERSION);
    assert_eq!(
        entry_names().len(),
        40,
        "18 项冻结 + nr_set_encoding + V1.1 分类 4 + 历史 3 + 配色 5 + 背景 4 + 作用域书籍 1 + 书籍分类映射 1 + 分类改名/移动 2 + 移动目标 1 项"
    );
    assert!(entry_names().iter().all(|name| name.starts_with("nr_")));
}

#[test]
fn handle_lifecycle_reports_destroyed_and_unknown_handles_distinctly() {
    let engine = create_engine();
    assert_eq!(unsafe { nr_engine_destroy(engine) }, 0, "首次销毁必须成功");
    assert_eq!(
        unsafe { nr_engine_destroy(engine) },
        ERR_ENGINE_DESTROYED,
        "重复销毁必须报 ENGINE_DESTROYED"
    );
    assert_eq!(
        unsafe { nr_engine_destroy(engine + 1_000) },
        ERR_INVALID_ARGUMENT,
        "未登记句柄必须报 INVALID_ARGUMENT"
    );
    assert_eq!(
        unsafe { nr_engine_destroy(0) },
        ERR_INVALID_ARGUMENT,
        "句柄 0 永远无效"
    );
    let mut window = NrWindow::default();
    let mut bytes = NrBuffer::empty();
    assert_eq!(
        unsafe { nr_read_window(engine, 0, 0, 16, &mut window, &mut bytes) },
        ERR_ENGINE_DESTROYED,
        "销毁后的句柄不得再使用"
    );
}

#[test]
fn engine_create_validates_abi_version_and_struct_size() {
    let mut handle = 0u64;
    let mut config = engine_config(None);
    config.abi_version = ABI_VERSION + 1;
    assert_eq!(
        unsafe { nr_engine_create(&config, &mut handle) },
        ERR_INVALID_ARGUMENT,
        "ABI 版本不匹配必须拒绝"
    );
    let mut config = engine_config(None);
    config.struct_size -= 1;
    assert_eq!(
        unsafe { nr_engine_create(&config, &mut handle) },
        ERR_INVALID_ARGUMENT,
        "结构体大小不匹配必须拒绝"
    );
    let mut config = engine_config(None);
    config.max_window_bytes = (MAX_WINDOW_BYTES + 1) as u64;
    assert_eq!(
        unsafe { nr_engine_create(&config, &mut handle) },
        ERR_INVALID_ARGUMENT,
        "窗口上限超过硬上限必须拒绝"
    );
    assert_eq!(
        unsafe { nr_engine_create(std::ptr::null(), &mut handle) },
        ERR_INVALID_ARGUMENT,
        "NULL config 必须拒绝"
    );
}

#[test]
fn open_read_window_is_bounded_and_utf8_aligned() {
    let book = TempBook::new(&chapter_text());
    let engine = create_engine();
    let opened = open_book(engine, &book);
    assert!(opened.book_id > 0);
    assert_eq!(
        opened.total_bytes as usize,
        book.path.metadata().unwrap().len() as usize
    );
    assert_eq!(opened.content_unit_id, CONTENT_UNIT_SINGLE);
    assert_eq!(opened.encoding, 1, "纯 UTF-8 夹具必须识别为 UTF-8");
    assert!(opened.chapters >= 5, "夹具含 5 个章节标题");
    assert_eq!(opened.index_from_cache, 0, "首次打开必须扫描索引");

    let mut window = NrWindow::default();
    let mut bytes = NrBuffer::empty();
    let code = unsafe {
        nr_read_window(
            engine,
            0,
            0,
            DEFAULT_WINDOW_BYTES as u64,
            &mut window,
            &mut bytes,
        )
    };
    assert_eq!(code, 0, "读取窗口必须成功");
    assert!(window.len > 0);
    assert!(window.eof == 1, "整本书小于一个窗口时必须报 EOF");
    assert_eq!(window.end_byte, u64::from(window.len));
    let data = unsafe { bytes.as_slice() }.to_vec();
    let decoded = std::str::from_utf8(&data).expect("窗口必须是合法 UTF-8");
    assert!(chapter_text().starts_with(decoded));
    unsafe {
        assert_eq!(nr_release_buffer(&mut bytes), 0, "首次归还必须成功");
        assert!(bytes.is_empty_handle(), "归还后必须置零");
        assert_eq!(nr_release_buffer(&mut bytes), 0, "重复归还必须是安全 no-op");
    }
    assert_eq!(
        unsafe { nr_release_buffer(std::ptr::null_mut()) },
        ERR_INVALID_ARGUMENT
    );

    // 超出硬上限的请求必须被拒绝（且不读取任何字节）。
    let mut window = NrWindow::default();
    let mut bytes = NrBuffer::empty();
    assert_eq!(
        unsafe {
            nr_read_window(
                engine,
                0,
                0,
                (MAX_WINDOW_BYTES + 1) as u64,
                &mut window,
                &mut bytes,
            )
        },
        ERR_INVALID_ARGUMENT
    );
    assert_eq!(
        unsafe { nr_read_window(engine, 0, 0, 0, &mut window, &mut bytes) },
        ERR_INVALID_ARGUMENT,
        "长度为 0 必须拒绝"
    );
    // 章节与偏移不一致：偏移必须落在请求章节的区间内。
    let first = unsafe { nr_read_window(engine, 1, 0, 64, &mut window, &mut bytes) };
    assert_eq!(first, ERR_CHAPTER_OUT_OF_RANGE);
    let last_chapter = opened.chapters - 1;
    let at_end = unsafe {
        nr_read_window(
            engine,
            last_chapter,
            opened.total_bytes,
            64,
            &mut window,
            &mut bytes,
        )
    };
    assert_eq!(at_end, 0, "文件末尾允许读取空窗口（EOF 场景）");
    unsafe {
        assert_eq!(nr_release_buffer(&mut bytes), 0);
    }
    assert_eq!(
        unsafe { nr_read_window(engine, 99, 0, 16, &mut window, &mut bytes) },
        ERR_CHAPTER_OUT_OF_RANGE
    );
    assert_eq!(unsafe { nr_close_book(engine, opened.book_id) }, 0);
    assert_eq!(
        unsafe { nr_read_window(engine, 0, 0, 16, &mut window, &mut bytes) },
        ERR_NO_OPEN_BOOK,
        "关闭后读取必须报 NO_OPEN_BOOK"
    );
    assert_eq!(unsafe { nr_engine_destroy(engine) }, 0);
}

#[test]
fn window_bytes_stay_within_hard_limit_even_for_large_book() {
    let mut content = Vec::new();
    while content.len() < 400 * 1024 {
        content.extend_from_slice("正文行内容，用于放大文件体积以验证窗口上限。\n".as_bytes());
    }
    let book = TempBook::from_bytes(content.clone());
    let engine = create_engine();
    let opened = open_book(engine, &book);
    assert!(opened.total_bytes >= 400 * 1024);

    let mut window = NrWindow::default();
    let mut bytes = NrBuffer::empty();
    let mut offset = 0u64;
    let mut windows = 0;
    // 顺序读取整本书：每次都必须 ≤ 硬上限，且每次都必须是合法 UTF-8。
    while offset < opened.total_bytes {
        let code = unsafe {
            nr_read_window(
                engine,
                0,
                offset,
                DEFAULT_WINDOW_BYTES as u64,
                &mut window,
                &mut bytes,
            )
        };
        assert_eq!(code, 0);
        assert!(
            window.len as usize <= MAX_WINDOW_BYTES,
            "单次物化字节必须 ≤ 硬上限"
        );
        assert!(window.len as usize <= DEFAULT_WINDOW_BYTES);
        let data = unsafe { bytes.as_slice() };
        assert!(
            std::str::from_utf8(data).is_ok(),
            "每个窗口都必须是合法 UTF-8"
        );
        unsafe {
            assert_eq!(nr_release_buffer(&mut bytes), 0);
        }
        windows += 1;
        assert!(window.end_byte > offset, "窗口必须严格前进");
        offset = window.end_byte;
        assert!(
            windows < 100,
            "窗口数量必须与文件大小同量级（不得整本一次返回）"
        );
    }
    assert_eq!(offset, opened.total_bytes);
    assert_eq!(unsafe { nr_engine_destroy(engine) }, 0);
}

#[test]
fn page_boundary_commit_query_and_invalidate_follow_frozen_status_codes() {
    let book = TempBook::new(&chapter_text());
    let engine = create_engine();
    let opened = open_book(engine, &book);
    let (signature, _family) = layout_signature();
    let mut result = NrBoundaryResult::default();

    let begin = submission(
        SUBMIT_BEGIN_LAYOUT,
        opened.content_index_version,
        0,
        0,
        0,
        0,
        0,
        Some(&signature),
    );
    assert_eq!(
        unsafe { nr_submit_page_boundary(engine, &begin, &mut result) },
        0
    );
    assert_eq!(result.status, BOUNDARY_LAYOUT_READY);
    assert!(result.layout_id > 0 && result.request_id > 0);
    let layout_id = result.layout_id;
    let request_id = result.request_id;

    // 合法提交：起点 0、终点落在字符边界（取第一行末尾）。
    let text = chapter_text();
    let first_line = text.find('\n').expect("夹具必须有换行") as u64 + 1;
    let commit = submission(
        SUBMIT_COMMIT,
        opened.content_index_version,
        layout_id,
        request_id,
        opened.total_bytes,
        0,
        first_line,
        None,
    );
    assert_eq!(
        unsafe { nr_submit_page_boundary(engine, &commit, &mut result) },
        0
    );
    assert_eq!(result.status, BOUNDARY_COMMITTED);
    assert_eq!(result.start_byte, 0);
    assert_eq!(result.end_byte, first_line);
    assert_eq!(result.page_count, 1);

    // 同终点重复提交：幂等接受。
    assert_eq!(
        unsafe { nr_submit_page_boundary(engine, &commit, &mut result) },
        0
    );
    assert_eq!(result.status, BOUNDARY_DUPLICATE_IDENTICAL);

    // 查询命中。
    assert_eq!(
        unsafe { nr_get_page_boundary(engine, layout_id, 0, 0, &mut result) },
        0
    );
    assert_eq!(result.end_byte, first_line);

    // 查询未命中的起点：LAYOUT_NOT_FOUND。
    assert_eq!(
        unsafe { nr_get_page_boundary(engine, layout_id, 0, 1, &mut result) },
        ERR_LAYOUT_NOT_FOUND
    );
    assert_eq!(result.status, BOUNDARY_NOT_FOUND);

    // 非法边界（零长度页面）必须被拒绝，且不写入任何状态。
    let zero = submission(
        SUBMIT_COMMIT,
        opened.content_index_version,
        layout_id,
        request_id,
        opened.total_bytes,
        first_line,
        first_line,
        None,
    );
    assert_eq!(
        unsafe { nr_submit_page_boundary(engine, &zero, &mut result) },
        ERR_INVALID_ARGUMENT
    );
    assert_eq!(result.status, BOUNDARY_REJECTED);
    assert_eq!(result.rejection, REJECT_EMPTY_INTERVAL);
    assert_eq!(result.page_count, 1, "被拒绝的提交不得改变已登记数量");

    // 新的排版世代（字号变化 ⇒ 签名变化）⇒ 旧世代提交必须被判为过期（丢弃而不是覆盖）。
    let (mut next_signature, _family) = layout_signature();
    next_signature.font_size_milli += 2_000;
    let next = submission(
        SUBMIT_BEGIN_LAYOUT,
        opened.content_index_version,
        0,
        0,
        0,
        0,
        0,
        Some(&next_signature),
    );
    unsafe { nr_submit_page_boundary(engine, &next, &mut result) };
    assert!(result.layout_id > layout_id, "世代必须递增");
    assert_eq!(
        unsafe { nr_submit_page_boundary(engine, &commit, &mut result) },
        ERR_LAYOUT_NOT_FOUND
    );
    assert_eq!(result.status, BOUNDARY_STALE_LAYOUT_ID);

    // 失效当前世代：删除已登记边界并再次递增世代。
    let mut removed = 0u64;
    assert_eq!(
        unsafe { nr_invalidate_layout(engine, result.layout_id, &mut removed) },
        0
    );
    assert_eq!(removed, 0, "新世代尚未登记任何边界");
    assert_eq!(
        unsafe { nr_invalidate_layout(engine, 999_999, &mut removed) },
        ERR_LAYOUT_NOT_FOUND,
        "未登记的世代必须报 LAYOUT_NOT_FOUND"
    );
    assert_eq!(unsafe { nr_engine_destroy(engine) }, 0);
}

#[test]
fn position_round_trip_is_idempotent_and_validates_chapter() {
    let book = TempBook::new(&chapter_text());
    let engine = create_engine();
    let opened = open_book(engine, &book);

    let mut result = NrPositionResult::default();
    assert_eq!(
        unsafe { nr_get_position(engine, opened.book_id, &mut result) },
        0,
        "尚无进度时必须成功返回 found = 0"
    );
    assert_eq!(result.found, 0);
    unsafe {
        assert_eq!(nr_release_buffer(&mut result.nearby_fingerprint), 0);
    }

    let offset = 32u64;
    let position = NrPosition {
        struct_size: std::mem::size_of::<NrPosition>() as u32,
        chapter_index: 0,
        book_id: opened.book_id,
        content_unit_id: CONTENT_UNIT_SINGLE as u64,
        original_byte_offset: offset,
        nearby_fingerprint_utf8: std::ptr::null(),
        nearby_fingerprint_len: 0,
    };
    let mut changed = 0i32;
    assert_eq!(
        unsafe { nr_save_position(engine, &position, &mut changed) },
        0
    );
    assert_eq!(changed, 1, "首次保存必须写入");
    assert_eq!(
        unsafe { nr_save_position(engine, &position, &mut changed) },
        0
    );
    assert_eq!(changed, 0, "同位置重复保存必须幂等（不写库）");

    assert_eq!(
        unsafe { nr_get_position(engine, opened.book_id, &mut result) },
        0
    );
    assert_eq!(result.found, 1);
    assert_eq!(result.original_byte_offset, offset);
    assert_eq!(result.book_id, opened.book_id);
    assert_eq!(result.content_unit_id, CONTENT_UNIT_SINGLE as u64);
    assert_eq!(result.restored, 1, "打开同一本书时必须尝试锚点恢复");
    let fingerprint = unsafe { result.nearby_fingerprint.as_slice() }.to_vec();
    assert!(!fingerprint.is_empty(), "引擎必须自动采样附近文本指纹");
    assert!(fingerprint.len() <= 96, "指纹必须落在存储上限内");
    unsafe {
        assert_eq!(nr_release_buffer(&mut result.nearby_fingerprint), 0);
    }

    // 章节与偏移不一致必须拒绝。
    let wrong_chapter = NrPosition {
        chapter_index: 4,
        ..position
    };
    assert_eq!(
        unsafe { nr_save_position(engine, &wrong_chapter, &mut changed) },
        ERR_CHAPTER_OUT_OF_RANGE
    );
    let out_of_range = NrPosition {
        original_byte_offset: opened.total_bytes + 1,
        ..position
    };
    assert_eq!(
        unsafe { nr_save_position(engine, &out_of_range, &mut changed) },
        ERR_OUT_OF_RANGE
    );
    let unknown_book = NrPosition {
        book_id: opened.book_id + 10_000,
        ..position
    };
    assert_eq!(
        unsafe { nr_save_position(engine, &unknown_book, &mut changed) },
        ERR_BOOK_NOT_FOUND
    );
    assert_eq!(
        unsafe { nr_get_position(engine, opened.book_id + 10_000, &mut result) },
        ERR_BOOK_NOT_FOUND
    );
    assert_eq!(unsafe { nr_engine_destroy(engine) }, 0);
}

#[test]
fn chapter_batch_is_bounded_and_titles_are_offset_addressed() {
    let book = TempBook::new(&chapter_text());
    let engine = create_engine();
    let opened = open_book(engine, &book);

    let mut batch = NrChapterBatch::default();
    assert_eq!(
        unsafe { nr_get_chapters(engine, opened.book_id, 0, 3, &mut batch) },
        0
    );
    assert_eq!(batch.count, 3, "批次必须按 max_count 截断");
    assert_eq!(batch.total_chapters, opened.chapters);
    assert!(batch.index_version > 0);

    let entry_size = std::mem::size_of::<NrChapter>();
    let raw = unsafe { batch.bytes.as_slice() }.to_vec();
    assert!(raw.len() >= batch.count as usize * entry_size);
    for index in 0..batch.count as usize {
        let start = index * entry_size;
        let sequence = u32::from_le_bytes(raw[start..start + 4].try_into().unwrap());
        let start_byte = u64::from_le_bytes(raw[start + 8..start + 16].try_into().unwrap());
        let end_byte = u64::from_le_bytes(raw[start + 16..start + 24].try_into().unwrap());
        let title_offset = u32::from_le_bytes(raw[start + 24..start + 28].try_into().unwrap());
        let title_len = u32::from_le_bytes(raw[start + 28..start + 32].try_into().unwrap());
        assert_eq!(sequence as usize, index, "章节序号必须连续");
        assert!(end_byte > start_byte, "章节区间必须非空");
        assert!(start_byte < opened.total_bytes);
        let title = std::str::from_utf8(
            &raw[title_offset as usize..title_offset as usize + title_len as usize],
        )
        .expect("标题必须是合法 UTF-8");
        assert!(!title.trim().is_empty(), "章节标题必须可读");
        assert!(
            title.contains(&format!("第{}章", index + 1)),
            "标题必须来自真实扫描: {title}"
        );
    }
    unsafe {
        assert_eq!(nr_release_buffer(&mut batch.bytes), 0);
    }

    assert_eq!(
        unsafe { nr_get_chapters(engine, opened.book_id, 0, 0, &mut batch) },
        ERR_INVALID_ARGUMENT,
        "max_count = 0 必须拒绝"
    );
    assert_eq!(
        unsafe { nr_get_chapters(engine, opened.book_id, 0, MAX_CHAPTER_BATCH + 1, &mut batch,) },
        ERR_INVALID_ARGUMENT,
        "超过批次上限必须拒绝（禁止一次取走整库章节）"
    );
    assert_eq!(
        unsafe { nr_get_chapters(engine, opened.book_id + 10_000, 0, 8, &mut batch) },
        ERR_BOOK_NOT_FOUND
    );
    assert_eq!(unsafe { nr_engine_destroy(engine) }, 0);
}

#[test]
fn bookmarks_are_created_deleted_and_default_titled() {
    let book = TempBook::new(&chapter_text());
    let engine = create_engine();
    let opened = open_book(engine, &book);

    let title = "起点".to_string();
    let mut bookmark_id = 0u64;
    let input = NrBookmarkInput {
        struct_size: std::mem::size_of::<NrBookmarkInput>() as u32,
        chapter_index: 0,
        book_id: opened.book_id,
        content_unit_id: CONTENT_UNIT_SINGLE as u64,
        byte_offset: 0,
        title_utf8: title.as_ptr(),
        title_len: title.len(),
    };
    assert_eq!(
        unsafe { nr_create_bookmark(engine, &input, &mut bookmark_id) },
        0
    );
    assert!(bookmark_id > 0);

    // 空标题：使用章节标题。
    let empty = NrBookmarkInput {
        title_utf8: std::ptr::null(),
        title_len: 0,
        byte_offset: 64,
        ..input
    };
    let mut default_title_id = 0u64;
    assert_eq!(
        unsafe { nr_create_bookmark(engine, &empty, &mut default_title_id) },
        0
    );
    assert!(default_title_id > bookmark_id);

    let wrong_chapter = NrBookmarkInput {
        chapter_index: 4,
        ..input
    };
    assert_eq!(
        unsafe { nr_create_bookmark(engine, &wrong_chapter, &mut bookmark_id) },
        ERR_CHAPTER_OUT_OF_RANGE
    );

    let mut deleted = 0i32;
    assert_eq!(
        unsafe { nr_delete_bookmark(engine, bookmark_id, &mut deleted) },
        0
    );
    assert_eq!(deleted, 1);
    assert_eq!(
        unsafe { nr_delete_bookmark(engine, bookmark_id, &mut deleted) },
        0
    );
    assert_eq!(deleted, 0, "重复删除必须回报未命中而不是报错");
    assert_eq!(unsafe { nr_engine_destroy(engine) }, 0);
}

#[test]
fn rebuild_index_is_cancellable_from_another_thread() {
    // 平台数据源 + 每次窗口延迟 50 ms，使取消有确定性的生效窗口。
    let body = "普通正文行，用于构造足以跨越多个窗口的内容。\n".repeat(6_000);
    let provider =
        MemoryWindowProvider::new(body.as_bytes().to_vec()).with_delay(Duration::from_millis(50));
    let total = body.len() as u64;
    let source = provider.provider();

    let config = NrEngineConfig {
        struct_size: std::mem::size_of::<NrEngineConfig>() as u32,
        abi_version: ABI_VERSION,
        max_window_bytes: 0,
        provider: void_ptr(&source),
        db_path_utf8: std::ptr::null(),
        db_path_len: 0,
    };
    let mut engine = 0u64;
    assert_eq!(unsafe { nr_engine_create(&config, &mut engine) }, 0);

    let identity = "content://demo/book.txt".to_string();
    let request = NrOpenRequest {
        struct_size: std::mem::size_of::<NrOpenRequest>() as u32,
        kind: SOURCE_KIND_PLATFORM,
        total_bytes: total,
        identity_utf8: identity.as_ptr(),
        identity_len: identity.len(),
        display_name_utf8: std::ptr::null(),
        display_name_len: 0,
    };
    let mut opened = NrOpenResult::default();
    assert_eq!(unsafe { nr_open_txt(engine, &request, &mut opened) }, 0);
    assert!(opened.chapters >= 1);
    assert!(
        provider.max_requested() <= MAX_WINDOW_BYTES as u64,
        "平台回调的请求窗口必须受硬上限约束"
    );

    let request_id = 4242u64;
    let engine_handle = engine;
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        unsafe { nr_cancel_request(engine_handle, request_id) }
    });
    let calls_before = provider.calls();
    let mut result = NrRebuildResult::default();
    let code = unsafe { nr_rebuild_index(engine, opened.book_id, request_id, &mut result) };
    assert_eq!(canceller.join().expect("取消线程"), 0);
    assert_eq!(code, ERR_CANCELLED, "被取消的任务必须返回 CANCELLED");
    assert_eq!(result.partial, 1, "取消结果必须标记为部分结果");
    assert!(
        provider.calls() > calls_before,
        "取消必须发生在至少一次窗口读取之后（说明是协作式而非立即失败）"
    );
    // 取消标志在任务结束时被清除：同样的 request_id 可以重新发起并成功完成。
    let mut result = NrRebuildResult::default();
    assert_eq!(
        unsafe { nr_rebuild_index(engine, opened.book_id, request_id, &mut result) },
        0,
        "取消标志必须已清除，重试必须成功"
    );
    assert_eq!(result.partial, 0);
    assert!(result.chapters >= 1);
    assert!(result.scanned_bytes >= total, "重建必须扫描完整内容");
    assert_eq!(
        unsafe { nr_cancel_request(engine, 0) },
        ERR_INVALID_ARGUMENT,
        "request_id 0 必须拒绝"
    );
    assert_eq!(unsafe { nr_engine_destroy(engine) }, 0);
}

#[test]
fn platform_source_never_receives_whole_file_requests() {
    let body = vec![b'x'; 700 * 1024];
    let provider = MemoryWindowProvider::new(body.clone());
    let source = provider.provider();
    let config = NrEngineConfig {
        struct_size: std::mem::size_of::<NrEngineConfig>() as u32,
        abi_version: ABI_VERSION,
        max_window_bytes: 0,
        provider: void_ptr(&source),
        db_path_utf8: std::ptr::null(),
        db_path_len: 0,
    };
    let mut engine = 0u64;
    assert_eq!(unsafe { nr_engine_create(&config, &mut engine) }, 0);
    let identity = "content://demo/big.txt".to_string();
    let request = NrOpenRequest {
        struct_size: std::mem::size_of::<NrOpenRequest>() as u32,
        kind: SOURCE_KIND_PLATFORM,
        total_bytes: body.len() as u64,
        identity_utf8: identity.as_ptr(),
        identity_len: identity.len(),
        display_name_utf8: std::ptr::null(),
        display_name_len: 0,
    };
    let mut opened = NrOpenResult::default();
    assert_eq!(unsafe { nr_open_txt(engine, &request, &mut opened) }, 0);

    let mut window = NrWindow::default();
    let mut bytes = NrBuffer::empty();
    // 请求整本书：必须在请求层被拒绝，平台回调一次都不会被调用。
    let calls = provider.calls();
    assert_eq!(
        unsafe {
            nr_read_window(
                engine,
                0,
                0,
                (MAX_WINDOW_BYTES + 1) as u64,
                &mut window,
                &mut bytes,
            )
        },
        ERR_INVALID_ARGUMENT
    );
    assert_eq!(provider.calls(), calls, "被拒绝的请求不得触发平台读取");

    // 正常窗口读取：请求长度受硬上限约束，返回字节数与回调交付量一致。
    assert_eq!(
        unsafe {
            nr_read_window(
                engine,
                0,
                0,
                DEFAULT_WINDOW_BYTES as u64,
                &mut window,
                &mut bytes,
            )
        },
        0
    );
    assert_eq!(window.len as usize, DEFAULT_WINDOW_BYTES);
    assert!(provider.max_requested() <= MAX_WINDOW_BYTES as u64);
    unsafe {
        assert_eq!(nr_release_buffer(&mut bytes), 0);
    }
    assert_eq!(unsafe { nr_engine_destroy(engine) }, 0);
}

#[test]
fn platform_protocol_requires_provider_and_rejects_unknown_kind() {
    let engine = create_engine();
    let identity = "content://demo/book.txt".to_string();
    let request = NrOpenRequest {
        struct_size: std::mem::size_of::<NrOpenRequest>() as u32,
        kind: SOURCE_KIND_PLATFORM,
        total_bytes: 1024,
        identity_utf8: identity.as_ptr(),
        identity_len: identity.len(),
        display_name_utf8: std::ptr::null(),
        display_name_len: 0,
    };
    let mut opened = NrOpenResult::default();
    assert_eq!(
        unsafe { nr_open_txt(engine, &request, &mut opened) },
        ERR_INVALID_ARGUMENT,
        "平台数据源未提供回调必须拒绝"
    );
    let unknown_kind = NrOpenRequest {
        kind: 99,
        ..request
    };
    assert_eq!(
        unsafe { nr_open_txt(engine, &unknown_kind, &mut opened) },
        ERR_INVALID_ARGUMENT
    );
    let empty_identity = NrOpenRequest {
        identity_len: 0,
        ..request
    };
    assert_eq!(
        unsafe { nr_open_txt(engine, &empty_identity, &mut opened) },
        ERR_INVALID_ARGUMENT
    );
    assert_eq!(
        unsafe { nr_open_txt(engine, std::ptr::null(), &mut opened) },
        ERR_INVALID_ARGUMENT
    );
    assert_eq!(unsafe { nr_engine_destroy(engine) }, 0);
}

#[test]
fn last_error_returns_code_and_message_with_single_owner() {
    let book = TempBook::new(&chapter_text());
    let engine = create_engine();
    let _opened = open_book(engine, &book);

    let mut window = NrWindow::default();
    let mut bytes = NrBuffer::empty();
    assert_eq!(
        unsafe {
            nr_read_window(
                engine,
                0,
                0,
                (MAX_WINDOW_BYTES + 1) as u64,
                &mut window,
                &mut bytes,
            )
        },
        ERR_INVALID_ARGUMENT
    );
    let mut info = NrErrorInfo::default();
    assert_eq!(unsafe { nr_get_last_error(engine, &mut info) }, 0);
    assert_eq!(info.code, ERR_INVALID_ARGUMENT);
    assert_eq!(info.has_message, 1);
    let message = unsafe { info.message.as_slice() }.to_vec();
    let message = String::from_utf8(message).expect("消息必须是 UTF-8");
    assert!(message.contains("窗口上限"), "消息必须可读: {message}");
    unsafe {
        assert_eq!(nr_release_buffer(&mut info.message), 0);
    }

    // 成功调用不得清空最近错误（错误信息用于诊断上一次失败）。
    let mut cursor = 0u64;
    assert_eq!(
        unsafe { nr_read_window(engine, 0, 0, 32, &mut window, &mut bytes) },
        0
    );
    unsafe {
        cursor += window.len as u64;
        assert_eq!(nr_release_buffer(&mut bytes), 0);
    }
    assert_eq!(unsafe { nr_get_last_error(engine, &mut info) }, 0);
    assert_eq!(info.code, ERR_INVALID_ARGUMENT);

    // 引擎销毁后不能再取错误信息，但已归还协议不受影响。
    assert_eq!(unsafe { nr_engine_destroy(engine) }, 0);
    assert_eq!(
        unsafe { nr_get_last_error(engine, &mut info) },
        ERR_ENGINE_DESTROYED
    );
    unsafe {
        assert_eq!(
            nr_release_buffer(&mut info.message),
            0,
            "销毁后仍可归还缓冲"
        );
    }
    let _ = cursor;
}

#[test]
fn buffers_survive_engine_destruction_and_forged_handles_are_rejected() {
    let book = TempBook::new(&chapter_text());
    let engine = create_engine();
    let _opened = open_book(engine, &book);
    let mut window = NrWindow::default();
    let mut bytes = NrBuffer::empty();
    assert_eq!(
        unsafe { nr_read_window(engine, 0, 0, 64, &mut window, &mut bytes) },
        0
    );
    assert_eq!(unsafe { nr_engine_destroy(engine) }, 0);
    assert_eq!(
        unsafe { nr_release_buffer(&mut bytes) },
        0,
        "引擎销毁后必须仍能归还缓冲区"
    );
    assert!(bytes.is_empty_handle());

    let mut forged = NrBuffer {
        ptr: std::ptr::null_mut(),
        len: 8,
        capacity: 8,
    };
    assert_eq!(
        unsafe { nr_release_buffer(&mut forged) },
        ERR_INVALID_ARGUMENT
    );
    let mut len_over_capacity = NrBuffer {
        ptr: std::ptr::dangling_mut::<u8>(),
        len: 16,
        capacity: 8,
    };
    assert_eq!(
        unsafe { nr_release_buffer(&mut len_over_capacity) },
        ERR_INVALID_ARGUMENT
    );
}

/// 编码感知的内容层（真机缺陷复现）：GBK 文档必须扫描出**真实章节**。
///
/// 真机现象：目录只显示一项「全文」。根因：章节扫描对每行做 `std::str::from_utf8`，
/// GBK 行不是合法 UTF-8 ⇒ 整行被跳过 ⇒ 零命中 ⇒ 回退成虚拟章节。
#[test]
fn gbk_book_reports_real_chapters_instead_of_virtual_full_text() {
    let bytes = std::fs::read("../../tests/fixtures/encoding/gbk.txt").expect("GBK 夹具必须存在");
    assert_eq!(&bytes[..2], &[0xb5, 0xda], "夹具首字应为 GBK「第」");
    let book = TempBook::from_bytes(bytes);
    let engine = create_engine();
    let opened = open_book(engine, &book);
    assert!(
        opened.encoding == 5 || opened.encoding == 6,
        "GBK 夹具必须被识别为 GB18030(5) 或 GBK(6)，实际 {}",
        opened.encoding
    );

    let mut batch = NrChapterBatch::default();
    assert_eq!(
        unsafe { nr_get_chapters(engine, opened.book_id, 0, MAX_CHAPTER_BATCH, &mut batch) },
        0
    );
    assert!(
        batch.total_chapters >= 2,
        "GBK 文档必须扫描出真实章节（真机缺陷：只回退成单个「全文」），实际 total_chapters={}",
        batch.total_chapters
    );

    let raw = unsafe { batch.bytes.as_slice() }.to_vec();
    let title_offset = u32::from_le_bytes(raw[24..28].try_into().unwrap()) as usize;
    let title_len = u32::from_le_bytes(raw[28..32].try_into().unwrap()) as usize;
    let title = std::str::from_utf8(&raw[title_offset..title_offset + title_len])
        .expect("章节标题必须是合法 UTF-8（ABI §3）");
    assert!(
        title.contains('第') && title.contains('章'),
        "GBK 章节标题必须解码可读：{title:?}"
    );
    unsafe {
        assert_eq!(nr_release_buffer(&mut batch.bytes), 0);
    }
}

/// 编码感知的页边界校验（真机缺陷复现）：GBK 的字符边界不得被判为非法。
///
/// 真机现象：翻页/切章报 `NR_ERR_3 (ENCODING) 偏移不在合法字符边界`，且间歇出现。
/// 根因：`is_utf8_boundary` 只判「是否 UTF-8 续接字节（0x80..=0xbf）」，
/// 而 GBK 汉字首字节范围 0x81–0xFE 与之重叠 ⇒ 大量合法 GB 边界被拒。
#[test]
fn gbk_character_boundary_is_accepted_for_page_submission() {
    let bytes = std::fs::read("../../tests/fixtures/encoding/gbk.txt").expect("GBK 夹具必须存在");
    // 夹具含「第一章 夜雨孤灯…」：偏移 11 = 「孤」的 GBK 起点（首字节 0xb9 ∈ 0x80..=0xbf），
    // 是**合法**的 GBK 字符边界，但旧的 UTF-8 续接字节判定会把它误判为非法。
    let book = TempBook::from_bytes(bytes);
    let engine = create_engine();
    let opened = open_book(engine, &book);
    let (signature, _family) = layout_signature();
    let mut result = NrBoundaryResult::default();

    let begin = submission(
        SUBMIT_BEGIN_LAYOUT,
        opened.content_index_version,
        0,
        0,
        0,
        0,
        0,
        Some(&signature),
    );
    assert_eq!(
        unsafe { nr_submit_page_boundary(engine, &begin, &mut result) },
        0
    );
    let layout_id = result.layout_id;
    let request_id = result.request_id;

    let commit = submission(
        SUBMIT_COMMIT,
        opened.content_index_version,
        layout_id,
        request_id,
        opened.total_bytes,
        0,
        11,
        None,
    );
    assert_eq!(
        unsafe { nr_submit_page_boundary(engine, &commit, &mut result) },
        0,
        "GBK 下的合法字符边界必须被接受（真机缺陷：被判为 NR_ERR_UTF8_BOUNDARY）"
    );
    assert_eq!(result.status, BOUNDARY_COMMITTED);

    // 负向对照（GBK）：落在字符**内部**的偏移必须被拒绝。
    // 基点 = 页起点（0），从基点按 GBK 字符长度步进：0→2→4→6→7→9→11→13…
    // ⇒ 合法的 11（「孤」的起点）可接受，而 12（「孤」的第 2 字节）必须被拒。
    let mid_gbk = submission(
        SUBMIT_COMMIT,
        opened.content_index_version,
        layout_id,
        request_id,
        opened.total_bytes,
        0,
        12,
        None,
    );
    assert_ne!(
        unsafe { nr_submit_page_boundary(engine, &mid_gbk, &mut result) },
        0,
        "GBK 字符内部的偏移必须被拒绝（基点判定不得退化为「一律放行」）"
    );

    // 负向对照改用 **UTF-8**：字符内部偏移必须被拒绝。
    // GBK 的同类校验需要「已知对齐基点」（核心窗口对齐仍是 UTF-8 专属），
    // 从错位小窗口步进会误拒合法边界 ⇒ 真机「打开即 ER_ERR_3」；
    // 故 GBK 暂不做字符边界校验（其余规则照旧）——见 results/encoding-core-boundary.md §窗口对齐。
    let utf8_book = TempBook::new("第一章 锚点\n正文若干字符用于边界对照。\n");
    let utf8_opened = open_book(engine, &utf8_book);
    let begin_utf8 = submission(
        SUBMIT_BEGIN_LAYOUT,
        utf8_opened.content_index_version,
        0,
        0,
        0,
        0,
        0,
        Some(&signature),
    );
    let utf8_layout = unsafe { nr_submit_page_boundary(engine, &begin_utf8, &mut result) };
    assert_eq!(utf8_layout, 0, "UTF-8 夹具排版必须开始");
    let mid_char = submission(
        SUBMIT_COMMIT,
        utf8_opened.content_index_version,
        result.layout_id,
        result.request_id,
        utf8_opened.total_bytes,
        0,
        1,
        None,
    );
    let code = unsafe { nr_submit_page_boundary(engine, &mid_char, &mut result) };
    assert_ne!(
        code, 0,
        "UTF-8 的字符内部偏移必须被拒绝（校验不得退化为「一律放行」）"
    );
}

/// 手动指定编码（`nr_set_encoding`，第 19 项导出）：只改解释方式，**原文件必须逐字节不变**。
#[test]
fn set_encoding_rescans_with_new_encoding_and_never_touches_the_file() {
    let bytes = std::fs::read("../../tests/fixtures/encoding/gbk.txt").expect("GBK 夹具必须存在");
    let book = TempBook::from_bytes(bytes.clone());
    let engine = create_engine();
    let opened = open_book(engine, &book);
    assert!(
        opened.encoding == 5 || opened.encoding == 6,
        "夹具应被识别为 GB18030/GBK，实际 {}",
        opened.encoding
    );
    assert!(opened.chapters >= 2, "自动检测下必须扫出真实章节");

    // ① 强制按 UTF-8 解释 GBK 文档（「错误」的编码）：必须重扫并回执新版本。
    let mut result = NrSetEncodingResult::default();
    assert_eq!(
        unsafe { nr_set_encoding(engine, opened.book_id, 1, &mut result) },
        0
    );
    assert_eq!(result.encoding, 1, "回执必须回显生效编码（UTF-8 = 1）");
    assert_eq!(result.index_rebuilt, 1, "必须就地重扫");
    assert!(
        result.content_index_version > u64::from(opened.content_index_version),
        "内容索引版本必须递增（旧分页缓存据此失效）"
    );
    assert_eq!(
        result.chapters, 1,
        "按 UTF-8 解释 GBK 字节时章节标记匹配不到 ⇒ 回退单章「全文」"
    );

    // ② 清除覆盖（0）⇒ 回到自动检测 ⇒ 真实章节重新出现。
    let mut restored = NrSetEncodingResult::default();
    assert_eq!(
        unsafe { nr_set_encoding(engine, opened.book_id, 0, &mut restored) },
        0
    );
    assert!(restored.encoding == 5 || restored.encoding == 6);
    assert!(
        restored.chapters >= 2,
        "回到自动检测后必须重新扫出真实章节，实际 {}",
        restored.chapters
    );

    // ③ 非法编码码必须被拒绝。
    assert_eq!(
        unsafe { nr_set_encoding(engine, opened.book_id, 99, &mut result) },
        ERR_INVALID_ARGUMENT
    );

    // ④ 硬约束：原文件逐字节不变（「永不写回原始文件」）。
    let after = std::fs::read(&book.path).expect("回读夹具");
    assert!(bytes == after, "手动指定编码绝不能写回原文件");
}

/// 用指定库路径建立引擎（模拟「同一台设备的同一次安装」）。
fn create_engine_with_db(db_path: &str) -> u64 {
    let config = engine_config(Some(db_path));
    let mut handle = 0u64;
    let code = unsafe { nr_engine_create(&config, &mut handle) };
    assert_eq!(code, 0, "创建引擎必须成功");
    handle
}

/// 取第 `index` 章的 `(start_byte, end_byte)`（条目在批次缓冲区里）。
fn chapter_range(engine: u64, book_id: u64, index: u32) -> (u64, u64) {
    let mut batch = NrChapterBatch::default();
    assert_eq!(
        unsafe { nr_get_chapters(engine, book_id, index, 1, &mut batch) },
        0
    );
    assert_eq!(batch.count, 1);
    let raw = unsafe { batch.bytes.as_slice() }.to_vec();
    let range = (
        u64::from_le_bytes(raw[8..16].try_into().unwrap()),
        u64::from_le_bytes(raw[16..24].try_into().unwrap()),
    );
    unsafe { nr_release_buffer(&mut batch.bytes) };
    range
}

/// 章内第二个换行符之后的偏移（换行是单字节字符 ⇒ 该偏移必为合法字符边界）。
fn offset_after_second_newline(start: u64) -> u64 {
    let path = "../../tests/fixtures/encoding/gbk.txt";
    let bytes = std::fs::read(path).expect("GBK 夹具必须存在");
    let window = &bytes[start as usize..(start as usize + 512).min(bytes.len())];
    let mut seen = 0;
    for (index, byte) in window.iter().enumerate() {
        if *byte == b'\n' {
            seen += 1;
            if seen == 2 {
                return start + index as u64 + 1;
            }
        }
    }
    start
}

/// **非 UTF-8（GBK）文档的位置续读**：章中保存 ⇒ 销毁引擎 ⇒ 同库重开 ⇒ 偏移原样复现。
///
/// 真机缺陷（2026-09-30 用户实测）：*UTF-8 文章能回到原位置，GBK 文章每次回到章首*。
/// 成因：锚点引文是 **UTF-8 文本**，而搜索是与**原始源字节**逐字节比较 ⇒ 对 GBK 永远匹配不上
/// （`generate_anchor` 的 `from_utf8` 也失败而存下空引文），核心对「引文为空」一律回退章首。
/// 修复：非 UTF-8 直接采用保存偏移（与桌面端 `restore_encoded` 同规则），越界才回退章首。
#[test]
fn legacy_encoded_position_survives_engine_restart() {
    let bytes = std::fs::read("../../tests/fixtures/encoding/gbk.txt").expect("GBK 夹具必须存在");
    let book = TempBook::from_bytes(bytes);
    let db_path = book.dir.join("resume.db").to_string_lossy().into_owned();

    // ---- 第一次「会话」：打开并在**章中**保存（不带指纹，移动端即如此）----
    let engine1 = create_engine_with_db(&db_path);
    let opened1 = open_book(engine1, &book);
    assert!(
        opened1.encoding == 5 || opened1.encoding == 6,
        "夹具必须是 GB18030/GBK，实际 {}",
        opened1.encoding
    );
    let (start, end) = chapter_range(engine1, opened1.book_id, 1);
    let offset = offset_after_second_newline(start);
    assert!(
        offset > start && offset < end,
        "测试偏移必须落在第 2 章内部"
    );

    let position = NrPosition {
        struct_size: std::mem::size_of::<NrPosition>() as u32,
        chapter_index: 1,
        book_id: opened1.book_id,
        content_unit_id: CONTENT_UNIT_SINGLE as u64,
        original_byte_offset: offset,
        nearby_fingerprint_utf8: std::ptr::null(),
        nearby_fingerprint_len: 0,
    };
    let mut changed = 0i32;
    assert_eq!(
        unsafe { nr_save_position(engine1, &position, &mut changed) },
        0
    );
    assert_eq!(changed, 1, "首次保存必须写入");
    assert_eq!(unsafe { nr_engine_destroy(engine1) }, 0, "模拟退出应用");

    // ---- 第二次「会话」：同库重开同一文件 ⇒ 必须回到章中 ----
    let engine2 = create_engine_with_db(&db_path);
    let opened2 = open_book(engine2, &book);
    let mut result = NrPositionResult::default();
    assert_eq!(
        unsafe { nr_get_position(engine2, opened2.book_id, &mut result) },
        0
    );
    assert_eq!(result.found, 1, "重启后必须找到进度");
    assert_eq!(result.chapter_index, 1, "必须仍在第 2 章");
    assert_eq!(
        result.original_byte_offset, offset,
        "GBK 文档必须回到章中偏移（修复前会退回章首）"
    );
    assert_eq!(result.fallback_to_chapter_start, 0, "不得回退章首");
    unsafe { nr_release_buffer(&mut result.nearby_fingerprint) };
    assert_eq!(unsafe { nr_engine_destroy(engine2) }, 0);
}

// ---------------------------------------------------------------------------
// V1.1 阅读历史（`nr_list_history` / `nr_clear_history_entry` / `nr_clear_history_for_book`）
// ---------------------------------------------------------------------------

/// 打开一本书并取 `book_id`（复用既有 `open_book`；它已断言成功）。
fn open_and_get_book_id(engine: u64, book: &TempBook) -> i64 {
    open_book(engine, book).book_id as i64
}

/// 读回历史批次并断言字节池里的字符串与行字段自洽。
///
/// 这条是 **B2 的关键契约**：两个游标（书名 / 文件名）拼出的缓冲区若错位，
/// Kotlin 侧会显示乱码 —— 而 JVM 测试无法发现，故在此钉住。
fn assert_history_batch_self_consistent(
    batch: &NrHistoryBatch,
    expect_rows: usize,
    expect_total: u32,
) -> Vec<(i64, i64, String)> {
    let row_size = std::mem::size_of::<NrHistory>();
    let bytes = unsafe { std::slice::from_raw_parts(batch.bytes.ptr, batch.bytes.len) };
    assert_eq!(batch.count as usize, expect_rows, "条目数不符");
    assert_eq!(batch.total, expect_total, "总条数不符");
    assert!(
        bytes.len() >= batch.count as usize * row_size,
        "缓冲区太短，装不下定长行"
    );

    let mut out = Vec::new();
    for index in 0..batch.count as usize {
        let base = index * row_size;
        let mut row = std::mem::MaybeUninit::<NrHistory>::uninit();
        // SAFETY：`bytes` 长度已断言 ≥ `count * row_size`，读取在界内。
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes[base..].as_ptr(),
                row.as_mut_ptr() as *mut u8,
                row_size,
            );
        }
        let row = unsafe { row.assume_init() };

        // 偏移已是**相对整个缓冲区**的绝对下标（ABI 口径），
        // 故直接切片 —— 早期版本又加了一次行区基准，导致越界。
        let row_area = batch.count as usize * row_size;
        assert!(
            row.title_offset as usize >= row_area && row.file_offset as usize >= row_area,
            "两个偏移都必须落在行区之后（否则会读到定长行里，当成乱码）",
        );
        let title = std::str::from_utf8(
            &bytes[row.title_offset as usize..row.title_offset as usize + row.title_len as usize],
        )
        .unwrap_or_else(|error| panic!("书名字节不是合法 UTF-8（偏移可能错位）：{error}"));
        let file = std::str::from_utf8(
            &bytes[row.file_offset as usize..row.file_offset as usize + row.file_len as usize],
        )
        .unwrap_or_else(|error| panic!("文件名字节不是合法 UTF-8（偏移可能错位）：{error}"));
        // 时间标签**由核心格式化**（规格 §3.6）—— 界面不得自己格式化，
        // 故此处必须校验它已是成品文本，而不是原始时间戳。
        let label = std::str::from_utf8(
            &bytes[row.label_offset as usize..row.label_offset as usize + row.label_len as usize],
        )
        .unwrap_or_else(|error| panic!("时间标签字节不是合法 UTF-8（偏移可能错位）：{error}"));
        assert!(!title.is_empty(), "书名不得为空");
        assert!(!file.is_empty(), "文件名不得为空");
        assert!(
            !file.contains('/') && !file.contains('\\'),
            "文件名只应是末段，不得含路径分隔符：{file}"
        );
        assert!(!label.is_empty(), "时间标签不得为空（流水都带打开时间）");
        // 三种口径之一：今天 `HH:MM`(5) / 本年 `MM-DD HH:MM`(11) / 跨年 `yyyy-MM-dd HH:MM`(16)。
        assert!(
            matches!(label.len(), 5 | 11 | 16),
            "时间标签必须已是核心格式化文本（今天/本年/跨年三态），实际：{label}"
        );
        let digits = label.chars().filter(|c| c.is_ascii_digit()).count();
        assert!(
            digits >= 4,
            "时间标签应含时分数字（不是原始时间戳或占位符），实际：{label}"
        );
        out.push((row.id, row.book_id, format!("{title}|{file}|{label}")));
    }
    out
}

#[test]
fn history_writes_one_entry_per_open_and_lists_time_descending() {
    let engine = create_engine();
    let book = TempBook::new("第一章\n正文一\n第二章\n正文二\n");
    let book_id = open_and_get_book_id(engine, &book);

    // `open_and_get_book_id` 内部已打开 1 次；再开 3 次 ⇒ 共 4 条流水。
    for _ in 0..3 {
        let _ = open_book(engine, &book);
    }

    let mut batch = NrHistoryBatch::default();
    let code = unsafe { nr_list_history(engine, 0, 0, 0, &mut batch as *mut NrHistoryBatch) };
    assert_eq!(code, 0, "列历史必须成功");
    let rows = assert_history_batch_self_consistent(&batch, 4, 4);
    assert!(
        rows.iter().all(|(_, id, _)| *id == book_id),
        "全部流水都应属于该书"
    );
    unsafe { nr_release_buffer(&mut batch.bytes) };
    unsafe { nr_close_book(engine, book_id as u64) };
    unsafe { nr_engine_destroy(engine) };
}

#[test]
fn history_list_by_book_isolates_and_limit_caps_rows() {
    let engine = create_engine();
    let first = TempBook::new("甲\n内容\n");
    let second = TempBook::new("乙\n内容\n");
    let first_id = open_and_get_book_id(engine, &first);
    let second_id = open_and_get_book_id(engine, &second);
    assert_ne!(first_id, second_id);

    // 两本各开 2 次 / 1 次。
    for _ in 0..2 {
        let _ = open_book(engine, &first);
    }
    let _ = open_book(engine, &second);

    // 限定单书：只回该书的 2 条，但 total 仍是全部 3 条。
    let mut batch = NrHistoryBatch::default();
    let code =
        unsafe { nr_list_history(engine, first_id, 0, 0, &mut batch as *mut NrHistoryBatch) };
    assert_eq!(code, 0);
    assert_history_batch_self_consistent(&batch, 3, 5);
    assert!(
        batch.count == 3,
        "限定单书时只回该书的流水（limit 未设 = 全部）"
    );
    unsafe { nr_release_buffer(&mut batch.bytes) };

    // limit = 1：只回 1 条，但 total 仍 3。
    let mut capped = NrHistoryBatch::default();
    assert_eq!(
        unsafe { nr_list_history(engine, 0, 1, 0, &mut capped as *mut NrHistoryBatch) },
        0
    );
    assert_history_batch_self_consistent(&capped, 1, 5);
    unsafe { nr_release_buffer(&mut capped.bytes) };

    unsafe { nr_close_book(engine, first_id as u64) };
    unsafe { nr_close_book(engine, second_id as u64) };
    unsafe { nr_engine_destroy(engine) };
}

#[test]
fn history_grouped_view_returns_every_entry_ordered_by_group() {
    let engine = create_engine();
    let first = TempBook::new("甲\n内容\n");
    let second = TempBook::new("乙\n内容\n");
    let first_id = open_and_get_book_id(engine, &first);
    let second_id = open_and_get_book_id(engine, &second);
    for _ in 0..2 {
        let _ = open_book(engine, &first);
    }
    let _ = open_book(engine, &second);

    let mut batch = NrHistoryBatch::default();
    let code = unsafe { nr_list_history(engine, 0, 0, 1, &mut batch as *mut NrHistoryBatch) };
    assert_eq!(code, 0);
    // 聚合视图**不丢条目**（组内平铺）：甲 3 条 + 乙 2 条 = 5。
    assert_history_batch_self_consistent(&batch, 5, 5);
    unsafe { nr_release_buffer(&mut batch.bytes) };

    unsafe { nr_close_book(engine, first_id as u64) };
    unsafe { nr_close_book(engine, second_id as u64) };
    unsafe { nr_engine_destroy(engine) };
}

#[test]
fn history_clear_entry_reports_missing_id_honestly() {
    let engine = create_engine();
    let book = TempBook::new("第一章\n内容\n");
    let book_id = open_and_get_book_id(engine, &book);

    let mut batch = NrHistoryBatch::default();
    assert_eq!(
        unsafe { nr_list_history(engine, 0, 0, 0, &mut batch as *mut NrHistoryBatch) },
        0
    );
    // helper 已把 id 读出（借用的字节在 helper 返回前已复制成 owned 字符串）。
    let rows = assert_history_batch_self_consistent(&batch, 1, 1);
    let entry_id = rows[0].0;
    unsafe { nr_release_buffer(&mut batch.bytes) };

    // 清一次 → removed = 1。
    let mut removed = 0u32;
    assert_eq!(
        unsafe { nr_clear_history_entry(engine, entry_id, &mut removed as *mut u32) },
        0
    );
    assert_eq!(removed, 1, "首次清除应回报 1");

    // 再清一次 → removed = 0（**如实回报「不存在」**，不谎报成功）。
    let mut again = 1u32;
    assert_eq!(
        unsafe { nr_clear_history_entry(engine, entry_id, &mut again as *mut u32) },
        0
    );
    assert_eq!(again, 0, "重复清除必须回报 0，不得谎报已清除");

    unsafe { nr_close_book(engine, book_id as u64) };
    unsafe { nr_engine_destroy(engine) };
}

#[test]
fn history_clear_for_book_only_removes_that_books_entries() {
    let engine = create_engine();
    let first = TempBook::new("甲\n内容\n");
    let second = TempBook::new("乙\n内容\n");
    let first_id = open_and_get_book_id(engine, &first);
    let second_id = open_and_get_book_id(engine, &second);
    for _ in 0..2 {
        let _ = open_book(engine, &first);
    }
    let _ = open_book(engine, &second);

    let mut removed = 0u32;
    assert_eq!(
        unsafe { nr_clear_history_for_book(engine, first_id, &mut removed as *mut u32) },
        0
    );
    assert_eq!(removed, 3, "应只删该书的 3 条（首次打开 + 2 次补开）");

    // 另一本书的流水**必须还在**（不得误删）。
    let mut batch = NrHistoryBatch::default();
    assert_eq!(
        unsafe { nr_list_history(engine, 0, 0, 0, &mut batch as *mut NrHistoryBatch) },
        0
    );
    assert_history_batch_self_consistent(&batch, 2, 2);
    unsafe { nr_release_buffer(&mut batch.bytes) };

    unsafe { nr_close_book(engine, first_id as u64) };
    unsafe { nr_close_book(engine, second_id as u64) };
    unsafe { nr_engine_destroy(engine) };
}

// ---------------------------------------------------------------------------
// V1.1 自定义配色（`nr_get_theme_base` / `nr_set_theme_base` / `nr_set_theme_enabled`
//                   / `nr_get_theme_token` / `nr_get_theme_contrast`）
// ---------------------------------------------------------------------------

/// 写一个基色（值长度 0 = 清除该项）。
fn set_base(engine: u64, which: u32, value: &str) -> i32 {
    let bytes = value.as_bytes();
    unsafe { nr_set_theme_base(engine, which, bytes.as_ptr(), bytes.len() as u32) }
}

/// 读回基色结构。
fn get_base(engine: u64) -> NrThemeBase {
    let mut base = NrThemeBase::default();
    let code = unsafe { nr_get_theme_base(engine, &mut base as *mut NrThemeBase) };
    assert_eq!(code, 0, "读基色必须成功");
    base
}

/// 把 12 字节定长槽读成 `Option<String>`（全 0 = 未设置）。
fn slot_text(slot: &[u8; 12]) -> Option<String> {
    let end = slot
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(slot.len());
    if end == 0 {
        return None;
    }
    Some(String::from_utf8_lossy(&slot[..end]).to_string())
}

/// 取一个令牌的 RGBA（4 字节）。
fn get_token(engine: u64, which: u32, dark: bool) -> [u8; 4] {
    let mut rgba = [0u8; 4];
    let code = unsafe { nr_get_theme_token(engine, which, u32::from(dark), rgba.as_mut_ptr()) };
    assert_eq!(code, 0, "取令牌必须成功");
    rgba
}

#[test]
fn theme_base_round_trips_all_four_colors() {
    let engine = create_engine();
    // 未设置时：全 0 + 关闭。
    let initial = get_base(engine);
    assert_eq!(initial.enabled, 0, "默认不启用");
    assert_eq!(slot_text(&initial.foreground), None, "默认未设置正文色");
    assert_eq!(slot_text(&initial.primary), None);

    assert_eq!(set_base(engine, 0, "#222222"), 0, "写正文色");
    assert_eq!(set_base(engine, 1, "#ffffff"), 0, "写背景色");
    assert_eq!(set_base(engine, 2, "#1890ff"), 0, "写主色");
    assert_eq!(set_base(engine, 3, "#d4380d"), 0, "写强调色");

    let read = get_base(engine);
    assert_eq!(slot_text(&read.foreground).as_deref(), Some("#222222"));
    assert_eq!(slot_text(&read.background).as_deref(), Some("#ffffff"));
    assert_eq!(slot_text(&read.primary).as_deref(), Some("#1890ff"));
    assert_eq!(slot_text(&read.accent).as_deref(), Some("#d4380d"));
    // 写基色**不得**替用户打开开关。
    assert_eq!(read.enabled, 0, "改基色不该自动启用（那是用户的显式选择）");

    unsafe { nr_engine_destroy(engine) };
}

#[test]
fn theme_base_rejects_invalid_color_and_keeps_previous() {
    let engine = create_engine();
    assert_eq!(set_base(engine, 2, "#1890ff"), 0);

    // 非法值被拒，且**原值保留**。
    assert_ne!(set_base(engine, 2, "nope"), 0, "非法色值必须被拒");
    assert_ne!(set_base(engine, 2, "#12345"), 0, "位数不足必须被拒");
    assert_ne!(set_base(engine, 9, "#ffffff"), 0, "越界索引必须被拒");

    let read = get_base(engine);
    assert_eq!(
        slot_text(&read.primary).as_deref(),
        Some("#1890ff"),
        "被拒的写入不得改动原值"
    );

    unsafe { nr_engine_destroy(engine) };
}

#[test]
fn theme_base_empty_value_clears_that_slot_only() {
    let engine = create_engine();
    set_base(engine, 0, "#222222");
    set_base(engine, 2, "#1890ff");

    // 空串 = **清除该项**（不是写入空颜色）。
    assert_eq!(set_base(engine, 0, ""), 0);
    let read = get_base(engine);
    assert_eq!(slot_text(&read.foreground), None, "正文色应被清除");
    assert_eq!(
        slot_text(&read.primary).as_deref(),
        Some("#1890ff"),
        "只清该项，其余不受影响"
    );

    unsafe { nr_engine_destroy(engine) };
}

#[test]
fn theme_enabled_toggle_keeps_base_colors() {
    let engine = create_engine();
    set_base(engine, 2, "#1890ff");

    assert_eq!(unsafe { nr_set_theme_enabled(engine, 1) }, 0, "开启");
    assert_eq!(get_base(engine).enabled, 1);

    // 关闭**保留基色**（用户下次开启不必重选）。
    assert_eq!(unsafe { nr_set_theme_enabled(engine, 0) }, 0, "关闭");
    let read = get_base(engine);
    assert_eq!(read.enabled, 0);
    assert_eq!(
        slot_text(&read.primary).as_deref(),
        Some("#1890ff"),
        "关闭开关不得清除基色"
    );

    unsafe { nr_engine_destroy(engine) };
}

#[test]
fn theme_token_reflects_base_color_and_falls_back_when_disabled() {
    let engine = create_engine();
    // 令牌 0 = `background`（令牌表首项）；先在**未启用**状态取一次（= 内置值）。
    let builtin_light = get_token(engine, 0, false);
    let builtin_dark = get_token(engine, 0, true);
    assert_ne!(
        builtin_light, builtin_dark,
        "内置明暗两套必须不同（否则暗色模式无从切换）"
    );

    // 启用并改背景为纯红 ⇒ 令牌 0 的明色那套应接近红。
    set_base(engine, 1, "#ff0000");
    assert_eq!(unsafe { nr_set_theme_enabled(engine, 1) }, 0);
    let derived_light = get_token(engine, 0, false);
    assert_ne!(
        derived_light, builtin_light,
        "启用自定义配色后令牌必须随基色变化"
    );
    assert!(
        derived_light[0] > 200 && derived_light[1] < 60 && derived_light[2] < 60,
        "背景基色 #ff0000 应派生红色令牌，实际 {derived_light:?}"
    );

    // 关闭 ⇒ 回**内置**值（与 `apply_palette` 的回落一致）。
    assert_eq!(unsafe { nr_set_theme_enabled(engine, 0) }, 0);
    assert_eq!(
        get_token(engine, 0, false),
        builtin_light,
        "关闭后必须回内置"
    );

    unsafe { nr_engine_destroy(engine) };
}

#[test]
fn theme_token_rejects_out_of_range_index() {
    let engine = create_engine();
    let mut rgba = [0u8; 4];
    let token_count = reading_storage::theme_derive::PALETTE_FIELDS.len() as u32;
    assert_eq!(
        unsafe { nr_get_theme_token(engine, token_count, 0, rgba.as_mut_ptr()) },
        ERR_INVALID_ARGUMENT,
        "越界令牌索引必须被拒"
    );
    unsafe { nr_engine_destroy(engine) };
}

#[test]
fn theme_contrast_reports_readable_ratio_and_keeps_low_contrast_choice() {
    let engine = create_engine();
    // 未启用时也给出读数（用户有参照）。
    let builtin = {
        let mut result = NrThemeContrastResult::default();
        assert_eq!(
            unsafe { nr_get_theme_contrast(engine, &mut result as *mut NrThemeContrastResult) },
            0
        );
        result
    };
    assert!(builtin.ratio_x100 > 0, "内置配色也应给出正比值");

    // 高对比组合 ⇒ 达标。
    set_base(engine, 0, "#000000");
    set_base(engine, 1, "#ffffff");
    assert_eq!(unsafe { nr_set_theme_enabled(engine, 1) }, 0);
    let good = {
        let mut result = NrThemeContrastResult::default();
        assert_eq!(
            unsafe { nr_get_theme_contrast(engine, &mut result as *mut NrThemeContrastResult) },
            0
        );
        result
    };
    assert_eq!(good.passes, 1, "黑白组合必须达标");
    assert!(
        good.ratio_x100 >= 450,
        "黑白对比度约 21:1，实际 {}",
        good.ratio_x100
    );

    // 低对比组合 ⇒ 不达标，但**基色保留**。
    set_base(engine, 0, "#f0f0f0");
    let bad = {
        let mut result = NrThemeContrastResult::default();
        assert_eq!(
            unsafe { nr_get_theme_contrast(engine, &mut result as *mut NrThemeContrastResult) },
            0
        );
        result
    };
    assert_eq!(bad.passes, 0, "浅色正文在白底上必须判不达标");
    let read = get_base(engine);
    assert_eq!(
        slot_text(&read.foreground).as_deref(),
        Some("#f0f0f0"),
        "对比度不足时**不得**改动用户基色（R12）"
    );

    unsafe { nr_engine_destroy(engine) };
}

#[test]
fn theme_tokens_cover_every_palette_field_and_match_shared_derivation() {
    // **关键契约**：FFI 取到的令牌必须与共享层 `derive()` **逐字节一致** ——
    // 否则桌面与 Android 会显示不同颜色（两个真相）。
    let engine = create_engine();
    set_base(engine, 0, "#222222");
    set_base(engine, 1, "#fdfdfd");
    set_base(engine, 2, "#1890ff");
    set_base(engine, 3, "#d4380d");
    assert_eq!(unsafe { nr_set_theme_enabled(engine, 1) }, 0);

    let base = reading_storage::theme::BaseColors {
        foreground: Some("#222222".to_string()),
        background: Some("#fdfdfd".to_string()),
        primary: Some("#1890ff".to_string()),
        accent: Some("#d4380d".to_string()),
    };
    let derived = reading_storage::theme_derive::derive(&base);

    for (index, field) in reading_storage::theme_derive::PALETTE_FIELDS
        .iter()
        .enumerate()
    {
        for dark in [false, true] {
            let via_ffi = get_token(engine, index as u32, dark);
            let shared = if dark {
                derived.dark_of(field.slint_name)
            } else {
                derived.light_of(field.slint_name)
            }
            .unwrap_or_else(|| panic!("共享层缺令牌 {}", field.slint_name));
            assert_eq!(
                via_ffi,
                shared,
                "令牌 {}（{}）在 FFI 与共享层之间不一致",
                field.slint_name,
                if dark { "暗色" } else { "明色" }
            );
        }
    }

    unsafe { nr_engine_destroy(engine) };
}

// ---------------------------------------------------------------------------
// V1.1 全局背景图（`nr_get_background` / `nr_set_background`
//                   / `nr_set_background_scrim` / `nr_clear_background`）
// ---------------------------------------------------------------------------

/// 读回背景批次并返回 `(file_name, fit_mode, scrim_permille, scrim_alpha)`。
///
/// 关键断言：返回的 `file_name` 必须是**相对文件名**（不含分隔符）——
/// 绝对路径由平台决定，核心**不得**泄露（R15）。
fn read_background(engine: u64) -> (String, i64, i64, u8, i32) {
    let mut batch = NrBackgroundBatch::default();
    let code = unsafe { nr_get_background(engine, &mut batch as *mut NrBackgroundBatch) };
    assert_eq!(code, 0, "读背景必须成功");
    let header_size = std::mem::size_of::<NrBackground>();
    let bytes = unsafe { std::slice::from_raw_parts(batch.bytes.ptr, batch.bytes.len) };
    assert!(
        bytes.len() >= header_size,
        "批次至少要装下头部（实际 {} 字节）",
        bytes.len()
    );
    let mut header = std::mem::MaybeUninit::<NrBackground>::uninit();
    // SAFETY：`bytes.len() >= header_size`（上面已断言）。
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), header.as_mut_ptr() as *mut u8, header_size);
    }
    let header = unsafe { header.assume_init() };

    let name = if header.file_name_len == 0 {
        String::new()
    } else {
        let start = header.file_name_offset as usize;
        let end = start + header.file_name_len as usize;
        assert!(end <= bytes.len(), "文件名区间越界（偏移口径可能错）");
        String::from_utf8_lossy(&bytes[start..end]).to_string()
    };
    let count = batch.count;
    unsafe { nr_release_buffer(&mut batch.bytes) };
    (
        name,
        header.fit_mode,
        header.scrim_permille,
        header.scrim_alpha,
        count as i32,
    )
}

/// 造一张假图片（内容非真实 PNG，但扩展名与存在性够核心用）。
fn fake_image(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, b"fake-image-bytes").expect("写假图片");
    path
}

/// 建一个临时目录（测试隔离）。
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let unique = format!(
            "nr-bg-abi-{}-{}-{}",
            tag,
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let dir = std::env::temp_dir().join(unique);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self(dir)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 写入一张背景（`fit_mode < 0` / `scrim < 0` = 用默认值）。
fn set_background(
    engine: u64,
    src: &std::path::Path,
    dest_dir: &std::path::Path,
    fit_mode: i32,
    scrim: i32,
) -> i32 {
    let src_text = src.to_string_lossy().to_string();
    let dir_text = dest_dir.to_string_lossy().to_string();
    unsafe {
        nr_set_background(
            engine,
            src_text.as_ptr(),
            src_text.len() as u32,
            dir_text.as_ptr(),
            dir_text.len() as u32,
            fit_mode,
            scrim,
            std::ptr::null_mut(),
        )
    }
}

#[test]
fn background_is_absent_by_default() {
    let engine = create_engine();
    let (name, fit, scrim, _alpha, count) = read_background(engine);
    assert_eq!(name, "", "默认未设置背景");
    assert_eq!(count, 0, "count = 0 表示未设置");
    // 默认口径：覆盖铺满 + 默认遮罩（即便未设置也给出参照值）。
    assert_eq!(fit, 0, "默认缩放口径");
    assert!(scrim >= 0);
    unsafe { nr_engine_destroy(engine) };
}

#[test]
fn background_set_copies_file_and_stores_only_relative_name() {
    let engine = create_engine();
    let temp = TempDir::new("set");
    let src = fake_image(temp.path(), "我的 原图.PNG");
    let dest = temp.path().join("app-private");

    let code = set_background(engine, &src, &dest, -1, -1);
    assert_eq!(code, 0, "设置背景必须成功");

    // **原图只读**：仍在原处、内容未变。
    assert!(src.is_file(), "原图不得被移动或删除");
    assert_eq!(
        std::fs::read(&src).expect("读原图"),
        b"fake-image-bytes",
        "原图内容不得被改动"
    );

    // 复制件存在于目标目录，且名字是**核心生成的**（不是用户原名）。
    let copied: Vec<_> = std::fs::read_dir(&dest)
        .expect("目标目录应存在")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(copied.len(), 1, "目标目录应有且仅有 1 个文件");
    let stored = &copied[0];
    assert!(stored.starts_with("bg-"), "文件名应由核心生成：{stored}");
    assert!(
        !stored.contains(' ') && stored.is_ascii(),
        "生成名不得含空格或非 ASCII（用户原名可能带中文）：{stored}"
    );
    assert!(stored.ends_with(".png"), "扩展名应保留（小写）：{stored}");

    // 读回：**只回相对文件名**，不含任何分隔符（R15）。
    let (name, _fit, _scrim, _alpha, count) = read_background(engine);
    assert_eq!(&name, stored);
    assert_eq!(count, 1, "count = 1 表示已设置");
    assert!(
        !name.contains('/') && !name.contains('\\'),
        "只回相对文件名（不得含分隔符）：{name}"
    );

    unsafe { nr_engine_destroy(engine) };
}

#[test]
fn background_rejects_unsupported_format_and_missing_file() {
    let engine = create_engine();
    let temp = TempDir::new("reject");
    let dest = temp.path().join("app-private");

    let text = fake_image(temp.path(), "note.txt");
    assert_ne!(
        set_background(engine, &text, &dest, -1, -1),
        0,
        "非图片扩展名必须被拒"
    );

    let missing = temp.path().join("nope.png");
    assert_ne!(
        set_background(engine, &missing, &dest, -1, -1),
        0,
        "不存在的源图必须被拒"
    );

    // 两次拒绝都不得留下记录或文件。
    let (name, _, _, _, count) = read_background(engine);
    assert_eq!(name, "", "被拒的设置不得留下记录");
    assert_eq!(count, 0);
    assert!(
        !dest.exists() || std::fs::read_dir(&dest).expect("读目录").count() == 0,
        "被拒的设置不得留下孤儿文件"
    );

    unsafe { nr_engine_destroy(engine) };
}

#[test]
fn background_replacing_removes_previous_generated_file() {
    let engine = create_engine();
    let temp = TempDir::new("replace");
    let dest = temp.path().join("app-private");
    let first = fake_image(temp.path(), "one.png");
    let second = fake_image(temp.path(), "two.png");

    assert_eq!(set_background(engine, &first, &dest, -1, -1), 0);
    let (name_a, _, _, _, _) = read_background(engine);
    assert_eq!(set_background(engine, &second, &dest, -1, -1), 0);
    let (name_b, _, _, _, _) = read_background(engine);

    assert_ne!(name_a, name_b, "换图后文件名应不同");
    assert!(!dest.join(&name_a).exists(), "旧的本模块生成文件应被清理");
    assert!(dest.join(&name_b).exists(), "新文件应存在");
    // 两张**原图**都还在。
    assert!(first.is_file() && second.is_file(), "原图永不删除");

    unsafe { nr_engine_destroy(engine) };
}

#[test]
fn background_scrim_can_be_tuned_without_changing_image() {
    let engine = create_engine();
    let temp = TempDir::new("scrim");
    let dest = temp.path().join("app-private");
    let src = fake_image(temp.path(), "bg.png");
    assert_eq!(set_background(engine, &src, &dest, -1, -1), 0);
    let (name_before, _, scrim_before, _, _) = read_background(engine);
    assert!(scrim_before >= 0);

    let mut batch = NrBackgroundBatch::default();
    assert_eq!(
        unsafe { nr_set_background_scrim(engine, 600, &mut batch as *mut NrBackgroundBatch) },
        0
    );
    unsafe { nr_release_buffer(&mut batch.bytes) };

    let (name_after, _, scrim_after, alpha_after, _) = read_background(engine);
    assert_eq!(name_after, name_before, "调遮罩不得换图");
    assert_eq!(scrim_after, 600, "遮罩应被更新");
    assert_eq!(
        alpha_after,
        reading_storage::background::scrim_to_alpha(600),
        "alpha 应由核心换算（界面不再换算）"
    );
    assert!(dest.join(&name_after).is_file(), "图片仍在");

    // 越界遮罩必须被拒。
    assert_ne!(
        unsafe { nr_set_background_scrim(engine, 5000, std::ptr::null_mut()) },
        0,
        "越界遮罩必须被拒"
    );

    unsafe { nr_engine_destroy(engine) };
}

#[test]
fn background_clear_removes_generated_file_but_keeps_source() {
    let engine = create_engine();
    let temp = TempDir::new("clear");
    let dest = temp.path().join("app-private");
    let src = fake_image(temp.path(), "orig.png");
    assert_eq!(set_background(engine, &src, &dest, -1, -1), 0);
    let (name, _, _, _, _) = read_background(engine);
    assert!(dest.join(&name).is_file());

    let dir_text = dest.to_string_lossy().to_string();
    let mut batch = NrBackgroundBatch::default();
    assert_eq!(
        unsafe {
            nr_clear_background(
                engine,
                dir_text.as_ptr(),
                dir_text.len() as u32,
                &mut batch as *mut NrBackgroundBatch,
            )
        },
        0
    );
    unsafe { nr_release_buffer(&mut batch.bytes) };

    assert!(!dest.join(&name).exists(), "本模块生成的文件应被删");
    assert!(src.is_file(), "**原图必须保留**");
    let (name_after, _, _, _, count) = read_background(engine);
    assert_eq!(name_after, "", "记录应被清除");
    assert_eq!(count, 0);
}

#[test]
fn background_clear_refuses_to_delete_non_generated_files() {
    // 安全边界：只删 `bg-*` 前缀（核心生成的），绝不删用户自己放进去的文件。
    let engine = create_engine();
    let temp = TempDir::new("guard");
    let dest = temp.path().join("app-private");
    std::fs::create_dir_all(&dest).expect("建目录");
    let hand_written = dest.join("user-managed.png");
    std::fs::write(&hand_written, b"x").expect("写");

    let dir_text = dest.to_string_lossy().to_string();
    let mut batch = NrBackgroundBatch::default();
    assert_eq!(
        unsafe {
            nr_clear_background(
                engine,
                dir_text.as_ptr(),
                dir_text.len() as u32,
                &mut batch as *mut NrBackgroundBatch,
            )
        },
        0
    );
    unsafe { nr_release_buffer(&mut batch.bytes) };
    assert!(
        hand_written.exists(),
        "非核心生成的文件不得被删（宁可留文件也不误删）"
    );

    unsafe { nr_engine_destroy(engine) };
}

// ---------------------------------------------------------------------------
// V1.1 书籍分类映射（`nr_list_book_categories`；筛选前置）
// ---------------------------------------------------------------------------

/// 读回书籍分类映射批次。
fn read_book_categories(engine: u64) -> Vec<(i64, i64)> {
    let mut batch = NrBookCategoryBatch::default();
    let code = unsafe { nr_list_book_categories(engine, &mut batch as *mut NrBookCategoryBatch) };
    assert_eq!(code, 0, "列书籍分类映射必须成功");
    let row_size = std::mem::size_of::<NrBookCategory>();
    let bytes = unsafe { std::slice::from_raw_parts(batch.bytes.ptr, batch.bytes.len) };
    assert_eq!(
        bytes.len(),
        batch.count as usize * row_size,
        "行内无字符串 ⇒ 字节池长度必须恰好是 count × 行大小"
    );
    let mut out = Vec::new();
    for index in 0..batch.count as usize {
        let base = index * row_size;
        let mut row = std::mem::MaybeUninit::<NrBookCategory>::uninit();
        // SAFETY：`bytes.len()` 已断言 ≥ count × row_size。
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes[base..].as_ptr(),
                row.as_mut_ptr() as *mut u8,
                row_size,
            );
        }
        let row = unsafe { row.assume_init() };
        out.push((row.book_id, row.category_id));
    }
    unsafe { nr_release_buffer(&mut batch.bytes) };
    out
}

#[test]
fn book_categories_reports_uncategorized_as_negative_one() {
    // 未分类 == -1（ABI 的 None 哨兵口径）：界面据此判断「未分类」而不必猜。
    let engine = create_engine();
    let book = TempBook::new("第一章\n正文\n");
    let book_id = open_and_get_book_id(engine, &book);

    let pairs = read_book_categories(engine);
    assert_eq!(pairs.len(), 1, "只应有一本在架书");
    assert_eq!(pairs[0].0, book_id);
    assert_eq!(pairs[0].1, (-1i64), "新导入的书应为未分类");

    unsafe { nr_close_book(engine, book_id as u64) };
    unsafe { nr_engine_destroy(engine) };
}

#[test]
fn book_categories_reflects_moves_and_covers_every_shelf_book() {
    let engine = create_engine();
    let first = TempBook::new("甲\n内容\n");
    let second = TempBook::new("乙\n内容\n");
    let first_id = open_and_get_book_id(engine, &first);
    let second_id = open_and_get_book_id(engine, &second);

    // 建一个分类，把「甲」移进去；「乙」保持未分类。
    let mut out_id = [0i64; 1];
    let name = "科幻".as_bytes();
    let code = unsafe {
        nr_create_category(
            engine,
            -1,
            name.as_ptr(),
            name.len() as u32,
            out_id.as_mut_ptr(),
        )
    };
    assert_eq!(code, 0, "建分类应成功");
    let category_id = out_id[0];
    assert_eq!(
        unsafe { nr_move_book_to_category(engine, first_id, category_id) },
        0
    );

    let pairs = read_book_categories(engine);
    assert_eq!(pairs.len(), 2, "两本在架书都要出现");
    let mapped: std::collections::HashMap<i64, i64> = pairs.iter().cloned().collect();
    assert_eq!(mapped.get(&first_id), Some(&category_id), "甲应归入新分类");
    assert_eq!(mapped.get(&second_id), Some(&(-1i64)), "乙应仍为未分类");

    unsafe { nr_close_book(engine, first_id as u64) };
    unsafe { nr_close_book(engine, second_id as u64) };
    unsafe { nr_engine_destroy(engine) };
}

/// 读回作用域书籍 id 列表。
fn read_scope_ids(engine: u64, scope_kind: i32, category_id: i64) -> Vec<i64> {
    let mut batch = NrIdList::default();
    let code = unsafe {
        nr_list_books_in_scope(engine, scope_kind, category_id, &mut batch as *mut NrIdList)
    };
    assert_eq!(code, 0, "列作用域书籍必须成功");
    let bytes = unsafe { std::slice::from_raw_parts(batch.bytes.ptr, batch.bytes.len) };
    assert_eq!(
        bytes.len(),
        batch.count as usize * 8,
        "i64 列表 ⇒ 字节池长度必须恰好是 count × 8"
    );
    let mut ids = Vec::new();
    for index in 0..batch.count as usize {
        let mut raw = [0u8; 8];
        raw.copy_from_slice(&bytes[index * 8..index * 8 + 8]);
        ids.push(i64::from_le_bytes(raw));
    }
    unsafe { nr_release_buffer(&mut batch.bytes) };
    ids
}

#[test]
fn scope_listing_expands_the_subtree_on_the_core_side() {
    // 关键语义：选父分类必须**含子孙** —— 且展开由核心做（界面不遍历父子链）。
    let engine = create_engine();
    let root = open_and_get_book_id(engine, &TempBook::new("甲\n正文\n"));
    let child_book = open_and_get_book_id(engine, &TempBook::new("乙\n正文\n"));
    let other = open_and_get_book_id(engine, &TempBook::new("丙\n正文\n"));
    let loose = open_and_get_book_id(engine, &TempBook::new("丁\n正文\n"));

    let mut parent = [0i64; 1];
    let mut child = [0i64; 1];
    let pname = "科幻".as_bytes();
    let cname = "中国科幻".as_bytes();
    assert_eq!(
        unsafe {
            nr_create_category(
                engine,
                -1,
                pname.as_ptr(),
                pname.len() as u32,
                parent.as_mut_ptr(),
            )
        },
        0
    );
    assert_eq!(
        unsafe {
            nr_create_category(
                engine,
                parent[0],
                cname.as_ptr(),
                cname.len() as u32,
                child.as_mut_ptr(),
            )
        },
        0
    );
    assert_eq!(
        unsafe { nr_move_book_to_category(engine, root, parent[0]) },
        0
    );
    assert_eq!(
        unsafe { nr_move_book_to_category(engine, child_book, child[0]) },
        0
    );
    assert_eq!(
        unsafe { nr_move_book_to_category(engine, other, parent[0]) },
        0
    );
    let _ = loose; // 保持未分类

    // 0 = 全部：4 本都在架。
    assert_eq!(read_scope_ids(engine, 0, 0).len(), 4);
    // 2 = 指定分类（含子孙）：父 2 本 + 子 1 本 = 3 本。
    let mut scope = read_scope_ids(engine, 2, parent[0]);
    scope.sort_unstable();
    let mut expected = vec![root, child_book, other];
    expected.sort_unstable();
    assert_eq!(scope, expected, "父分类必须含子分类的书");
    // 子分类只含自己 1 本 —— 证明展开方向正确（不含父/兄弟）。
    assert_eq!(read_scope_ids(engine, 2, child[0]), vec![child_book]);
    // 1 = 未分类：只剩未分类那本。
    assert_eq!(read_scope_ids(engine, 1, 0), vec![loose]);
    // 反向：全部（4）≠ 未分类（1）—— 证明过滤真的生效。
    assert_ne!(
        read_scope_ids(engine, 0, 0).len(),
        read_scope_ids(engine, 1, 0).len()
    );

    for id in [root, child_book, other, loose] {
        unsafe { nr_close_book(engine, id as u64) };
    }
    unsafe { nr_engine_destroy(engine) };
}

#[test]
fn scope_listing_rejects_an_unknown_scope_kind() {
    // 未知识别码必须**如实报错**，不得退化成「返回全部」（那会让界面静默显示错数据）。
    let engine = create_engine();
    let mut batch = NrIdList::default();
    let code = unsafe { nr_list_books_in_scope(engine, 99, 0, &mut batch as *mut NrIdList) };
    assert_eq!(code, ERR_INVALID_ARGUMENT, "未知作用域应报参数错误");
    unsafe { nr_engine_destroy(engine) };
}

/// 读回移动目标批次。
fn read_move_targets(engine: u64, source_id: i64) -> Vec<(i64, bool, String)> {
    let mut batch = NrCategoryChoiceBatch::default();
    let code = unsafe {
        nr_category_move_targets(engine, source_id, &mut batch as *mut NrCategoryChoiceBatch)
    };
    assert_eq!(code, 0, "列移动目标必须成功");
    let row_size = std::mem::size_of::<NrCategoryChoice>();
    let bytes = unsafe { std::slice::from_raw_parts(batch.bytes.ptr, batch.bytes.len) };
    assert!(
        bytes.len() >= batch.count as usize * row_size,
        "字节池必须容得下全部行"
    );
    let mut out = Vec::new();
    for index in 0..batch.count as usize {
        let base = index * row_size;
        let mut row = std::mem::MaybeUninit::<NrCategoryChoice>::uninit();
        // SAFETY：上面已断言字节池 ≥ count × row_size。
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes[base..].as_ptr(),
                row.as_mut_ptr() as *mut u8,
                row_size,
            );
        }
        let row = unsafe { row.assume_init() };
        let name = String::from_utf8_lossy(
            &bytes[row.name_offset as usize..row.name_offset as usize + row.name_len as usize],
        )
        .to_string();
        out.push((row.id, row.selectable == 1, name));
    }
    unsafe { nr_release_buffer(&mut batch.bytes) };
    out
}

#[test]
fn move_targets_exclude_self_and_descendants_by_marking_them_unselectable() {
    // 规格 R5 的界面侧：自身与子孙**必须不可选**（但**不隐藏** —— 要能解释为什么）。
    let engine = create_engine();
    let mut parent = [0i64; 1];
    let mut child = [0i64; 1];
    let pname = "科幻".as_bytes();
    let cname = "中国科幻".as_bytes();
    assert_eq!(
        unsafe {
            nr_create_category(
                engine,
                -1,
                pname.as_ptr(),
                pname.len() as u32,
                parent.as_mut_ptr(),
            )
        },
        0
    );
    assert_eq!(
        unsafe {
            nr_create_category(
                engine,
                parent[0],
                cname.as_ptr(),
                cname.len() as u32,
                child.as_mut_ptr(),
            )
        },
        0
    );

    // 以「科幻」为源：它自己与「中国科幻」都不可选。
    let targets = read_move_targets(engine, parent[0]);
    let by_id: std::collections::HashMap<i64, bool> =
        targets.iter().map(|(id, ok, _)| (*id, *ok)).collect();
    assert_eq!(by_id.get(&parent[0]), Some(&false), "自身不可选");
    assert_eq!(by_id.get(&child[0]), Some(&false), "子孙不可选");
    assert_eq!(by_id.get(&(-1i64)), Some(&true), "未分类恒可选");
    // 不可选是**标记**而非隐藏：两条都仍在列表里（便于向用户解释）。
    assert_eq!(targets.len(), 3, "未分类 + 两个分类");

    // 以「中国科幻」为源：自己不可选，但**父分类可选**（展开方向正确）。
    let child_targets = read_move_targets(engine, child[0]);
    let child_by_id: std::collections::HashMap<i64, bool> =
        child_targets.iter().map(|(id, ok, _)| (*id, *ok)).collect();
    assert_eq!(child_by_id.get(&child[0]), Some(&false));
    assert_eq!(child_by_id.get(&parent[0]), Some(&true), "父分类可选");

    unsafe { nr_engine_destroy(engine) };
}

#[test]
fn move_targets_include_uncategorized_first_with_its_name() {
    let engine = create_engine();
    let targets = read_move_targets(engine, -1);
    assert_eq!(targets.first().map(|(id, ..)| *id), Some(-1i64));
    assert_eq!(
        targets.first().map(|(_, _, name)| name.as_str()),
        Some("未分类")
    );
    assert!(
        targets.first().map(|(_, ok, _)| *ok).unwrap_or(false),
        "未分类恒可选"
    );
    unsafe { nr_engine_destroy(engine) };
}

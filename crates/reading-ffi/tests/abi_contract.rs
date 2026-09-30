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
    let _ = nr_cancel_request;
    let _ = nr_get_last_error;
    let _ = nr_release_buffer;
    assert_eq!(nr_abi_version(), ABI_VERSION);
    assert_eq!(entry_names().len(), 19, "18 项冻结 + nr_set_encoding");
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
    let bytes =
        std::fs::read("../../tests/fixtures/encoding/gbk.txt").expect("GBK 夹具必须存在");
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
    let bytes =
        std::fs::read("../../tests/fixtures/encoding/gbk.txt").expect("GBK 夹具必须存在");
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

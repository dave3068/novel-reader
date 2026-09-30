//! 跨语言 C ABI：11 个基线入口 + 7 个扩展入口。
//!
//! 约定：
//! - 字符串一律为 `UTF-8 指针 + 长度`，不传语言对象；
//! - 输出缓冲由 Rust 分配，调用方读取后必须调用 [`nr_release_buffer`] 归还；
//! - 所有函数返回 `NR_ERR_*` 错误码；`panic` 被拦截为 `NR_ERR_INTERNAL`；
//! - 回调在触发线程同步执行，数据指针仅在回调期间有效。

use core::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::slice;

use crate::abi::ABI_VERSION;
use crate::engine::{BookSource, Metrics, NrEventCallback, ReadingEngine, DEFAULT_MAX_WINDOW};
use crate::error::{invalid_argument, ErrorCode, NrError};
use crate::pagination::{layout_id, layout_page, LayoutConfig};
use crate::position::{ContentPosition, PageBoundary};

/// 由 Rust 分配、调用方归还的字节缓冲。
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NrBuffer {
    pub ptr: *mut u8,
    pub len: usize,
    pub capacity: usize,
}

impl NrBuffer {
    /// 空缓冲。
    pub const fn empty() -> Self {
        Self {
            ptr: core::ptr::null_mut(),
            len: 0,
            capacity: 0,
        }
    }
}

impl Default for NrBuffer {
    fn default() -> Self {
        Self::empty()
    }
}

/// 测量指标（与 Rust `Metrics` 对应）。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NrMetrics {
    pub read_calls: u64,
    pub copied_bytes: u64,
    pub materialized_bytes: u64,
    pub last_layout_us: u64,
    pub events_emitted: u64,
    pub cancelled_requests: u64,
    pub page_boundaries: u64,
}

/// 排版配置（`font_family` 为 UTF-8 指针 + 长度）。
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NrLayoutConfig {
    pub width: f32,
    pub height: f32,
    pub font_size: f32,
    pub line_spacing: f32,
    pub paragraph_spacing: f32,
    pub first_line_indent: f32,
    pub margin_x: f32,
    pub margin_y: f32,
    pub font_family_ptr: *const u8,
    pub font_family_len: usize,
}

/// 单页排版结果。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NrPageLayout {
    pub consumed_utf8_bytes: u64,
    pub visible_line_count: u32,
    pub last_line_width: f32,
    pub last_line_height: f32,
    pub layout_time_us: u64,
    pub layout_id: u64,
}

/// 页面边界（`repr(C)` 与 `PageBoundary` 布局一致）。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NrPageBoundary {
    pub chapter_index: u32,
    pub start_byte: u64,
    pub end_byte: u64,
    pub layout_id: u64,
    pub line_count: u32,
}

/// 阅读位置（`repr(C)` 与 `ContentPosition` 布局一致）。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NrContentPosition {
    pub book_id: u64,
    pub chapter_index: u32,
    pub byte_offset: u64,
}

/// 请求进度。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NrRequestProgress {
    pub done: u32,
    pub total: u32,
    pub cancelled: u32,
    pub finished: u32,
}

/// 不透明引擎句柄。
pub struct NrEngine {
    inner: ReadingEngine,
    last_error: Option<Vec<u8>>,
}

impl NrEngine {
    fn new(max_window_bytes: u64) -> Self {
        let max = if max_window_bytes == 0 {
            DEFAULT_MAX_WINDOW
        } else {
            max_window_bytes as usize
        };
        Self {
            inner: ReadingEngine::new(max),
            last_error: None,
        }
    }

    fn set_error(&mut self, err: &NrError) {
        self.last_error = Some(format!("{}|{}", err.code.as_str(), err.message).into_bytes());
    }
}

fn ok() -> i32 {
    ErrorCode::Ok.as_c()
}

fn guard<F: FnOnce() -> i32>(f: F) -> i32 {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| ErrorCode::Internal.as_c())
}

fn make_buffer(mut v: Vec<u8>) -> NrBuffer {
    let buf = NrBuffer {
        ptr: v.as_mut_ptr(),
        len: v.len(),
        capacity: v.capacity(),
    };
    std::mem::forget(v);
    buf
}

unsafe fn write_buffer(out: *mut NrBuffer, v: Vec<u8>) {
    if !out.is_null() {
        *out = make_buffer(v);
    }
}

unsafe fn slice_opt<'a>(ptr: *const u8, len: usize) -> Option<&'a [u8]> {
    if len == 0 {
        return Some(&[]);
    }
    if ptr.is_null() {
        return None;
    }
    Some(slice::from_raw_parts(ptr, len))
}

unsafe fn engine_mut<'a>(e: *mut NrEngine) -> Result<&'a mut NrEngine, NrError> {
    if e.is_null() {
        Err(invalid_argument("引擎句柄为空"))
    } else {
        Ok(&mut *e)
    }
}

unsafe fn layout_config_from(cfg: *const NrLayoutConfig) -> Result<LayoutConfig, NrError> {
    if cfg.is_null() {
        return Err(invalid_argument("排版配置为空"));
    }
    let c = &*cfg;
    let family_bytes = slice_opt(c.font_family_ptr, c.font_family_len)
        .ok_or_else(|| invalid_argument("字体族指针为空"))?;
    let font_family = std::str::from_utf8(family_bytes)
        .map_err(|_| invalid_argument("字体族不是合法 UTF-8"))?
        .to_string();
    Ok(LayoutConfig {
        width: c.width,
        height: c.height,
        font_family,
        font_size: c.font_size,
        line_spacing: c.line_spacing,
        paragraph_spacing: c.paragraph_spacing,
        first_line_indent: c.first_line_indent,
        margin_x: c.margin_x,
        margin_y: c.margin_y,
    })
}

// ---------------------------------------------------------------------------
// 基线入口
// ---------------------------------------------------------------------------

/// 创建引擎；`max_window_bytes == 0` 时使用默认 256 KiB。
#[no_mangle]
pub extern "C" fn nr_engine_create(max_window_bytes: u64) -> *mut NrEngine {
    catch_unwind(AssertUnwindSafe(|| {
        Box::into_raw(Box::new(NrEngine::new(max_window_bytes)))
    }))
    .unwrap_or(core::ptr::null_mut())
}

/// 销毁引擎。
#[no_mangle]
pub unsafe extern "C" fn nr_engine_destroy(e: *mut NrEngine) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if !e.is_null() {
            drop(Box::from_raw(e));
        }
    }));
}

/// 打开书籍。`path_len == 0` 时使用合成文本源（`total_bytes` 为正文规模）。
#[no_mangle]
pub unsafe extern "C" fn nr_open_book(
    e: *mut NrEngine,
    book_id: u64,
    path_ptr: *const u8,
    path_len: usize,
    total_bytes: u64,
) -> i32 {
    guard(|| {
        let engine = match engine_mut(e) {
            Ok(x) => x,
            Err(err) => return err.code.as_c(),
        };
        let source = if path_len == 0 {
            BookSource::Synthetic { total_bytes }
        } else {
            let bytes = match slice_opt(path_ptr, path_len) {
                Some(b) => b,
                None => return ErrorCode::InvalidArgument.as_c(),
            };
            match std::str::from_utf8(bytes) {
                Ok(p) => BookSource::File(std::path::PathBuf::from(p)),
                Err(_) => return ErrorCode::InvalidArgument.as_c(),
            }
        };
        match engine.inner.open_book(book_id, source) {
            Ok(()) => ok(),
            Err(err) => {
                engine.set_error(&err);
                err.code.as_c()
            }
        }
    })
}

/// 读取有限窗口（章节内偏移），输出合法 UTF-8 字节。
#[no_mangle]
pub unsafe extern "C" fn nr_read_window(
    e: *mut NrEngine,
    chapter_index: u32,
    offset: u64,
    len: usize,
    out: *mut NrBuffer,
) -> i32 {
    guard(|| {
        let engine = match engine_mut(e) {
            Ok(x) => x,
            Err(err) => return err.code.as_c(),
        };
        if out.is_null() {
            return ErrorCode::InvalidArgument.as_c();
        }
        match engine.inner.read_window(chapter_index, offset, len) {
            Ok(w) => {
                write_buffer(out, w.bytes);
                ok()
            }
            Err(err) => {
                engine.set_error(&err);
                err.code.as_c()
            }
        }
    })
}

/// 归还缓冲。
#[no_mangle]
pub unsafe extern "C" fn nr_release_buffer(buf: NrBuffer) -> i32 {
    guard(|| {
        if buf.ptr.is_null() {
            return ok();
        }
        drop(Vec::from_raw_parts(buf.ptr, buf.len, buf.capacity));
        ok()
    })
}

/// 书籍批次（UTF-8 JSON）。
#[no_mangle]
pub unsafe extern "C" fn nr_get_book_batch(
    e: *mut NrEngine,
    start: u64,
    count: u32,
    out: *mut NrBuffer,
) -> i32 {
    guard(|| {
        let engine = match engine_mut(e) {
            Ok(x) => x,
            Err(err) => return err.code.as_c(),
        };
        if out.is_null() {
            return ErrorCode::InvalidArgument.as_c();
        }
        let json = engine
            .inner
            .library()
            .books_batch_json(start as usize, count as usize);
        write_buffer(out, json.into_bytes());
        ok()
    })
}

/// 章节批次（UTF-8 JSON）。
#[no_mangle]
pub unsafe extern "C" fn nr_get_chapter_batch(
    e: *mut NrEngine,
    book_id: u64,
    start: u32,
    count: u32,
    out: *mut NrBuffer,
) -> i32 {
    guard(|| {
        let engine = match engine_mut(e) {
            Ok(x) => x,
            Err(err) => return err.code.as_c(),
        };
        if out.is_null() {
            return ErrorCode::InvalidArgument.as_c();
        }
        match engine
            .inner
            .library()
            .chapters_batch_json(book_id, start, count)
        {
            Some(json) => {
                write_buffer(out, json.into_bytes());
                ok()
            }
            None => ErrorCode::BookNotFound.as_c(),
        }
    })
}

/// 保存阅读位置。
#[no_mangle]
pub unsafe extern "C" fn nr_save_position(
    e: *mut NrEngine,
    book_id: u64,
    chapter_index: u32,
    byte_offset: u64,
) -> i32 {
    guard(|| {
        let engine = match engine_mut(e) {
            Ok(x) => x,
            Err(err) => return err.code.as_c(),
        };
        let pos = ContentPosition {
            book_id,
            chapter_index,
            byte_offset,
        };
        match engine.inner.save_position(pos) {
            Ok(()) => ok(),
            Err(err) => {
                engine.set_error(&err);
                err.code.as_c()
            }
        }
    })
}

/// 登记页面边界。
#[no_mangle]
pub unsafe extern "C" fn nr_commit_page_boundary(
    e: *mut NrEngine,
    chapter_index: u32,
    start_byte: u64,
    end_byte: u64,
    layout: u64,
    line_count: u32,
) -> i32 {
    guard(|| {
        let engine = match engine_mut(e) {
            Ok(x) => x,
            Err(err) => return err.code.as_c(),
        };
        let b = PageBoundary {
            chapter_index,
            start_byte,
            end_byte,
            layout_id: layout,
            line_count,
        };
        match engine.inner.commit_page_boundary(b) {
            Ok(()) => ok(),
            Err(err) => {
                engine.set_error(&err);
                err.code.as_c()
            }
        }
    })
}

/// 失效排版缓存；`out_removed` 可为空。
#[no_mangle]
pub unsafe extern "C" fn nr_invalidate_layout(
    e: *mut NrEngine,
    layout: u64,
    out_removed: *mut u64,
) -> i32 {
    guard(|| {
        let engine = match engine_mut(e) {
            Ok(x) => x,
            Err(err) => return err.code.as_c(),
        };
        let removed = engine.inner.invalidate_layout(layout);
        if !out_removed.is_null() {
            *out_removed = removed as u64;
        }
        ok()
    })
}

/// 订阅事件，返回订阅 id（负数表示错误码）。
#[no_mangle]
pub unsafe extern "C" fn nr_subscribe_events(
    e: *mut NrEngine,
    cb: Option<NrEventCallback>,
    user: *mut c_void,
) -> i64 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let engine = match engine_mut(e) {
            Ok(x) => x,
            Err(err) => return -(err.code.as_c() as i64),
        };
        match cb {
            Some(cb) => engine.inner.subscribe_cabi(cb, user) as i64,
            None => -(ErrorCode::InvalidArgument.as_c() as i64),
        }
    }));
    result.unwrap_or(-(ErrorCode::Internal.as_c() as i64))
}

/// 取消请求（协作式，部分成功语义）。
#[no_mangle]
pub unsafe extern "C" fn nr_cancel_request(e: *mut NrEngine, request_id: u64) -> i32 {
    guard(|| {
        let engine = match engine_mut(e) {
            Ok(x) => x,
            Err(err) => return err.code.as_c(),
        };
        match engine.inner.cancel_request(request_id) {
            Ok(()) => ok(),
            Err(err) => {
                engine.set_error(&err);
                err.code.as_c()
            }
        }
    })
}

// ---------------------------------------------------------------------------
// 扩展入口（非基线）
// ---------------------------------------------------------------------------

/// ABI 版本。
#[no_mangle]
pub extern "C" fn nr_abi_version() -> u32 {
    ABI_VERSION
}

/// 共享排版（与桌面端直接 Rust API 语义等价）。
#[no_mangle]
pub unsafe extern "C" fn nr_layout_page(
    cfg: *const NrLayoutConfig,
    text_ptr: *const u8,
    text_len: usize,
    start_byte: u64,
    out: *mut NrPageLayout,
) -> i32 {
    guard(|| {
        if out.is_null() {
            return ErrorCode::InvalidArgument.as_c();
        }
        let config = match layout_config_from(cfg) {
            Ok(c) => c,
            Err(err) => return err.code.as_c(),
        };
        let bytes = match slice_opt(text_ptr, text_len) {
            Some(b) => b,
            None => return ErrorCode::InvalidArgument.as_c(),
        };
        let text = match std::str::from_utf8(bytes) {
            Ok(t) => t,
            Err(_) => return ErrorCode::InvalidArgument.as_c(),
        };
        let page = layout_page(text, start_byte as usize, &config);
        *out = NrPageLayout {
            consumed_utf8_bytes: page.consumed_utf8_bytes as u64,
            visible_line_count: page.visible_line_count,
            last_line_width: page.last_line_width,
            last_line_height: page.last_line_height,
            layout_time_us: page.layout_time_us,
            layout_id: layout_id(&config),
        };
        ok()
    })
}

/// 指标快照。
#[no_mangle]
pub unsafe extern "C" fn nr_get_metrics(e: *mut NrEngine, out: *mut NrMetrics) -> i32 {
    guard(|| {
        let engine = match engine_mut(e) {
            Ok(x) => x,
            Err(err) => return err.code.as_c(),
        };
        if out.is_null() {
            return ErrorCode::InvalidArgument.as_c();
        }
        let m: Metrics = engine.inner.metrics();
        *out = NrMetrics {
            read_calls: m.read_calls,
            copied_bytes: m.copied_bytes,
            materialized_bytes: m.materialized_bytes,
            last_layout_us: m.last_layout_us,
            events_emitted: m.events_emitted,
            cancelled_requests: m.cancelled_requests,
            page_boundaries: m.page_boundaries,
        };
        ok()
    })
}

/// 最近一次错误消息（`CODE|message`）。
#[no_mangle]
pub unsafe extern "C" fn nr_last_error_message(e: *mut NrEngine, out: *mut NrBuffer) -> i32 {
    guard(|| {
        let engine = match engine_mut(e) {
            Ok(x) => x,
            Err(err) => return err.code.as_c(),
        };
        if out.is_null() {
            return ErrorCode::InvalidArgument.as_c();
        }
        let bytes = engine.last_error.clone().unwrap_or_default();
        write_buffer(out, bytes);
        ok()
    })
}

/// 发起模拟批次请求。
#[no_mangle]
pub unsafe extern "C" fn nr_start_batch_request(
    e: *mut NrEngine,
    count: u32,
    out_request_id: *mut u64,
) -> i32 {
    guard(|| {
        let engine = match engine_mut(e) {
            Ok(x) => x,
            Err(err) => return err.code.as_c(),
        };
        let id = engine.inner.start_batch_request(count);
        if !out_request_id.is_null() {
            *out_request_id = id;
        }
        ok()
    })
}

/// 推进模拟批次请求。
#[no_mangle]
pub unsafe extern "C" fn nr_advance_request(
    e: *mut NrEngine,
    request_id: u64,
    steps: u32,
    out: *mut NrRequestProgress,
) -> i32 {
    guard(|| {
        let engine = match engine_mut(e) {
            Ok(x) => x,
            Err(err) => return err.code.as_c(),
        };
        match engine.inner.advance_request(request_id, steps) {
            Ok(p) => {
                if !out.is_null() {
                    *out = NrRequestProgress {
                        done: p.done,
                        total: p.total,
                        cancelled: if p.cancelled { 1 } else { 0 },
                        finished: if p.finished { 1 } else { 0 },
                    };
                }
                ok()
            }
            Err(err) => {
                engine.set_error(&err);
                err.code.as_c()
            }
        }
    })
}

/// 打开外部内容（单章，坐标空间长度 = `total_bytes`；字节由平台侧按窗口提供）。
#[no_mangle]
pub unsafe extern "C" fn nr_open_external_book(
    e: *mut NrEngine,
    book_id: u64,
    total_bytes: u64,
) -> i32 {
    guard(|| {
        let engine = match engine_mut(e) {
            Ok(x) => x,
            Err(err) => return err.code.as_c(),
        };
        match engine.inner.open_external(book_id, total_bytes) {
            Ok(()) => ok(),
            Err(err) => {
                engine.set_error(&err);
                err.code.as_c()
            }
        }
    })
}

/// 引擎快照（UTF-8 JSON，供 ready.json/结果记录引用）。
#[no_mangle]
pub unsafe extern "C" fn nr_engine_snapshot_json(e: *mut NrEngine, out: *mut NrBuffer) -> i32 {
    guard(|| {
        let engine = match engine_mut(e) {
            Ok(x) => x,
            Err(err) => return err.code.as_c(),
        };
        if out.is_null() {
            return ErrorCode::InvalidArgument.as_c();
        }
        write_buffer(out, engine.inner.snapshot_json().into_bytes());
        ok()
    })
}

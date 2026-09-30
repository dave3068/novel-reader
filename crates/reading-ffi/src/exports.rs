//! 冻结的 18 项 C ABI 导出 + `nr_set_encoding`（向后兼容新增，共 19 项）。
//!
//! # 通用调用约定
//!
//! - 每个入口返回 `int32_t` 错误码（`0` = 成功），错误码数值与 `NR_ERR_*` 一致；
//!   唯一例外是 [`nr_abi_version`]（直接返回 `uint32_t` 版本号）。
//! - **永不把 panic 泄漏到 C 边界**：每个入口都用 `catch_unwind` 包裹，
//!   panic 被转换为 `NR_ERR_INTERNAL`（10）而不是未定义行为；
//! - **句柄** `uint64_t` 为不透明整数，`0` 永远无效；销毁后的句柄报
//!   `NR_ERR_ENGINE_DESTROYED`（9），未登记句柄报 `NR_ERR_INVALID_ARGUMENT`（1）；
//! - **字符串**一律为 UTF-8 指针 + 长度，不依赖 NUL 结尾；
//! - **缓冲区**由 Rust 分配（`nr_read_window` / `nr_get_chapters` / `nr_get_position` /
//!   `nr_get_last_error`），调用方必须用 [`nr_release_buffer`] 归还；归还后字段全部置零，
//!   重复归还是安全 no-op；
//! - 引擎级调用在同一把引擎互斥锁下**串行执行**；长任务（索引重建）在窗口之间检查
//!   取消标志，因此 [`nr_cancel_request`] 可以从另一个线程在任务运行期间生效。

use std::os::raw::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use reading_core::{ErrorCode, LayoutSignature, NrError, NrResult, TextDirection};
use reading_index::ANCHOR_MAX_QUOTE_BYTES;
use reading_source::TextEncoding;

use crate::abi::{
    internal, invalid, ABI_ENTRY_NAMES, ABI_VERSION, BOUNDARY_LAYOUT_READY, MAX_CHAPTER_BATCH,
    SOURCE_KIND_FILE, SOURCE_KIND_PLATFORM, SUBMIT_BEGIN_LAYOUT, SUBMIT_COMMIT,
};
use crate::buffer::{release_buffer, NrBuffer};
use crate::cancel;
use crate::engine::{Engine, EngineConfig, OpenTxtSpec, PositionSpec};
use crate::external::NrSourceProvider;
use crate::handle;
use crate::structs::{NrBookmarkInput, NrBoundaryResult, NrChapterBatch, NrEngineConfig, NrErrorInfo, NrLayoutSignature, NrOpenRequest, NrOpenResult, NrPageBoundarySubmission, NrPosition, NrPositionResult, NrRebuildResult, NrSetEncodingResult, NrWindow};

/// 在引擎互斥锁下执行一次调用，并把任意 panic 转成 `NR_ERR_INTERNAL`。
fn with_engine(handle_id: u64, body: impl FnOnce(&mut Engine) -> NrResult<()>) -> i32 {
    let engine: Arc<std::sync::Mutex<Engine>> = match handle::lookup(handle_id) {
        Ok(engine) => engine,
        // 句柄无效/已销毁：没有引擎可记录最近错误，只能返回错误码。
        Err(error) => return error.code().as_c(),
    };
    let result = catch_unwind(AssertUnwindSafe(|| {
        let mut guard = engine.lock().map_err(|_| internal("引擎互斥锁中毒"))?;
        let outcome = body(&mut guard);
        if let Err(error) = &outcome {
            guard.record_error(error.clone());
        }
        outcome
    }));
    match result {
        Ok(Ok(())) => 0,
        Ok(Err(error)) => error.code().as_c(),
        Err(panic) => {
            let detail = panic_message(panic);
            let error = internal(format!("ABI 入口拦截到 panic: {detail}"));
            if let Ok(mut guard) = engine.lock() {
                guard.record_error(error.clone());
            }
            error.code().as_c()
        }
    }
}

fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    if let Some(text) = panic.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = panic.downcast_ref::<String>() {
        text.clone()
    } else {
        "未知 panic".to_string()
    }
}

/// 读取 UTF-8 字符串参数（指针 + 长度）；`NULL` 与 0 长度按"未提供"处理。
unsafe fn read_utf8(ptr: *const u8, len: usize, field: &str) -> NrResult<Option<String>> {
    if ptr.is_null() {
        if len != 0 {
            return Err(invalid(format!("{field}: 指针为 NULL 但长度非 0")));
        }
        return Ok(None);
    }
    if len == 0 {
        return Ok(Some(String::new()));
    }
    let bytes = std::slice::from_raw_parts(ptr, len);
    let text = std::str::from_utf8(bytes)
        .map_err(|_| invalid(format!("{field}: 不是合法 UTF-8")))?
        .to_string();
    Ok(Some(text))
}

/// 校验调用方声明的结构体大小。
fn check_struct_size<const N: usize>(declared: u32, name: &str) -> NrResult<()> {
    if declared as usize != N {
        return Err(invalid(format!(
            "{name}.struct_size 必须等于 {}（当前 ABI 版本），实际 {declared}",
            N
        )));
    }
    Ok(())
}

fn write_out<T: Copy>(out: *mut T, value: T) -> NrResult<()> {
    if out.is_null() {
        return Err(invalid("输出参数不得为 NULL"));
    }
    unsafe {
        std::ptr::write(out, value);
    }
    Ok(())
}

/// ABI 版本查询（`nr_abi_version`）。
///
/// # Safety
///
/// 无参数、无副作用，可从任意线程调用。
#[no_mangle]
pub extern "C" fn nr_abi_version() -> u32 {
    ABI_VERSION
}

/// 创建引擎（`nr_engine_create`）。
///
/// # Safety
///
/// `config` 必须指向一个当前 ABI 版本的 [`NrEngineConfig`]；`out_engine` 必须可写。
#[no_mangle]
pub unsafe extern "C" fn nr_engine_create(
    config: *const NrEngineConfig,
    out_engine: *mut u64,
) -> i32 {
    let result = catch_unwind(AssertUnwindSafe(|| -> NrResult<()> {
        if config.is_null() {
            return Err(invalid("nr_engine_create 的 config 不得为 NULL"));
        }
        let config = &*config;
        check_struct_size::<{ std::mem::size_of::<NrEngineConfig>() }>(
            config.struct_size,
            "nr_engine_config",
        )?;
        if config.abi_version != ABI_VERSION {
            return Err(invalid(format!(
                "ABI 版本不匹配：调用方声明 {}，库为 {ABI_VERSION}",
                config.abi_version
            )));
        }
        let max_window = if config.max_window_bytes == 0 {
            crate::abi::MAX_WINDOW_BYTES
        } else {
            config.max_window_bytes as usize
        };
        if max_window > crate::abi::MAX_WINDOW_BYTES {
            return Err(invalid(format!(
                "max_window_bytes {max_window} 超过硬上限 {}",
                crate::abi::MAX_WINDOW_BYTES
            )));
        }
        let provider = if config.provider.is_null() {
            None
        } else {
            let provider = *(config.provider.cast::<NrSourceProvider>());
            crate::external::ensure_provider_available(&provider)?;
            Some(provider)
        };
        let db_path = read_utf8(config.db_path_utf8, config.db_path_len, "db_path")?
            .filter(|path| !path.is_empty())
            .map(std::path::PathBuf::from);
        let engine = Engine::new(EngineConfig {
            max_window_bytes: max_window,
            provider,
            db_path,
        })?;
        let engine_handle = handle::create(engine)?;
        if let Ok(engine) = handle::lookup(engine_handle) {
            if let Ok(mut guard) = handle::lock(&engine) {
                guard.set_handle(engine_handle);
            }
        }
        write_out(out_engine, engine_handle)
    }));
    match result {
        Ok(Ok(())) => 0,
        Ok(Err(error)) => error.code().as_c(),
        Err(_) => ErrorCode::Internal.as_c(),
    }
}

/// 销毁引擎（`nr_engine_destroy`）。
///
/// # Safety
///
/// `engine` 必须是 [`nr_engine_create`] 返回的句柄；调用后该句柄不再可用。
#[no_mangle]
pub unsafe extern "C" fn nr_engine_destroy(engine: u64) -> i32 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let outcome = handle::destroy(engine);
        if outcome.is_ok() {
            cancel::clear_engine(engine);
        }
        outcome
    }));
    match result {
        Ok(Ok(())) => 0,
        Ok(Err(error)) => error.code().as_c(),
        Err(_) => ErrorCode::Internal.as_c(),
    }
}

/// 打开 TXT（`nr_open_txt`）。
///
/// # Safety
///
/// `request` 必须指向当前 ABI 版本的 [`NrOpenRequest`]，其字符串指针必须在本调用期间有效。
#[no_mangle]
pub unsafe extern "C" fn nr_open_txt(
    engine: u64,
    request: *const NrOpenRequest,
    out_result: *mut NrOpenResult,
) -> i32 {
    with_engine(engine, |engine| {
        if request.is_null() {
            return Err(invalid("nr_open_txt 的 request 不得为 NULL"));
        }
        let request = &*request;
        check_struct_size::<{ std::mem::size_of::<NrOpenRequest>() }>(
            request.struct_size,
            "nr_open_request",
        )?;
        if request.kind != SOURCE_KIND_FILE && request.kind != SOURCE_KIND_PLATFORM {
            return Err(invalid(format!(
                "数据源种类必须为 {SOURCE_KIND_FILE} 或 {SOURCE_KIND_PLATFORM}，实际 {}",
                request.kind
            )));
        }
        let identity = read_utf8(request.identity_utf8, request.identity_len, "identity")?
            .ok_or_else(|| invalid("identity 不得为空"))?;
        let display_name = read_utf8(
            request.display_name_utf8,
            request.display_name_len,
            "display_name",
        )?;
        let outcome = engine.open_txt(&OpenTxtSpec {
            kind: request.kind,
            total_bytes: request.total_bytes,
            identity,
            display_name,
        })?;
        write_out(
            out_result,
            NrOpenResult {
                book_id: outcome.book_id,
                total_bytes: outcome.total_bytes,
                content_index_version: outcome.content_index_version,
                content_unit_id: crate::abi::CONTENT_UNIT_SINGLE,
                encoding: outcome.encoding,
                encoding_confidence: outcome.encoding_confidence,
                encoding_score: outcome.encoding_score,
                needs_encoding_confirmation: u32::from(outcome.needs_encoding_confirmation),
                chapters: outcome.chapters,
                index_from_cache: u32::from(outcome.index_from_cache),
                reserved0: 0,
            },
        )
    })
}

/// 关闭书籍（`nr_close_book`）。
///
/// # Safety
///
/// `engine` 必须是有效句柄。
#[no_mangle]
pub unsafe extern "C" fn nr_close_book(engine: u64, book_id: u64) -> i32 {
    with_engine(engine, |engine| engine.close_book(book_id))
}

/// 读取有限窗口（`nr_read_window`）。
///
/// 返回的字节缓冲区由 Rust 分配，调用方必须用 [`nr_release_buffer`] 归还。
///
/// # Safety
///
/// `out_window` / `out_bytes` 必须可写。
#[no_mangle]
pub unsafe extern "C" fn nr_read_window(
    engine: u64,
    chapter_index: u32,
    offset: u64,
    length: u64,
    out_window: *mut NrWindow,
    out_bytes: *mut NrBuffer,
) -> i32 {
    with_engine(engine, |engine| {
        let outcome = engine.read_window(chapter_index, offset, length)?;
        write_out(
            out_window,
            NrWindow {
                offset: outcome.offset,
                end_byte: outcome.end_byte,
                request_id: outcome.request_id,
                total_bytes: outcome.total_bytes,
                len: outcome.len,
                dropped_leading: outcome.dropped_leading,
                trimmed_tail: outcome.trimmed_tail,
                eof: u32::from(outcome.eof),
            },
        )?;
        write_out(out_bytes, NrBuffer::from_vec(outcome.bytes))
    })
}

/// 提交页边界（`nr_submit_page_boundary`）。
///
/// `phase = NR_SUBMIT_BEGIN_LAYOUT` 时登记签名并返回 `layout_id` / `request_id`；
/// `phase = NR_SUBMIT_COMMIT` 时提交平台测量结果（过期结果按状态被丢弃，不覆盖当前页）。
///
/// # Safety
///
/// `submission` 必须指向当前 ABI 版本的 [`NrPageBoundarySubmission`]；`out_result` 必须可写。
#[no_mangle]
pub unsafe extern "C" fn nr_submit_page_boundary(
    engine: u64,
    submission: *const NrPageBoundarySubmission,
    out_result: *mut NrBoundaryResult,
) -> i32 {
    with_engine(engine, |engine| {
        if submission.is_null() {
            return Err(invalid("nr_submit_page_boundary 的 submission 不得为 NULL"));
        }
        let submission = &*submission;
        check_struct_size::<{ std::mem::size_of::<NrPageBoundarySubmission>() }>(
            submission.struct_size,
            "nr_page_boundary_submission",
        )?;
        let outcome = match submission.phase {
            SUBMIT_BEGIN_LAYOUT => {
                let signature = read_layout_signature(submission.signature)?;
                engine.begin_layout(signature, submission.content_index_version)?
            }
            SUBMIT_COMMIT => engine.commit_boundary(
                submission.content_index_version,
                submission.layout_id,
                submission.request_id,
                submission.total_bytes,
                submission.start_byte,
                submission.end_byte,
            )?,
            other => {
                return Err(invalid(format!(
                    "未知的提交阶段: {other}（期望 {SUBMIT_BEGIN_LAYOUT} 或 {SUBMIT_COMMIT}）"
                )))
            }
        };
        let code = outcome.error_code;
        write_out(out_result, boundary_result(&outcome))?;
        record_outcome_error(engine, code, &outcome);
        if code == 0 {
            Ok(())
        } else {
            Err(crate::abi::error_from_code(
                code,
                format!("页边界提交未成功（状态 {}）", outcome.status),
            ))
        }
    })
}

/// 查询页边界（`nr_get_page_boundary`）。
///
/// # Safety
///
/// `out_result` 必须可写。
#[no_mangle]
pub unsafe extern "C" fn nr_get_page_boundary(
    engine: u64,
    layout_id: u64,
    chapter_index: u32,
    start_byte: u64,
    out_result: *mut NrBoundaryResult,
) -> i32 {
    with_engine(engine, |engine| {
        let outcome = engine.query_boundary(layout_id, chapter_index, start_byte)?;
        let code = outcome.error_code;
        write_out(out_result, boundary_result(&outcome))?;
        if code == 0 {
            Ok(())
        } else {
            Err(crate::abi::error_from_code(
                code,
                format!(
                    "布局 {layout_id} 下起点 {start_byte} 没有登记的页边界（状态 {}）",
                    outcome.status
                ),
            ))
        }
    })
}

/// 保存阅读位置（`nr_save_position`）；`out_changed` 可为 `NULL`。
///
/// # Safety
///
/// `position` 必须指向当前 ABI 版本的 [`NrPosition`]，其指纹指针在本调用期间有效。
#[no_mangle]
pub unsafe extern "C" fn nr_save_position(
    engine: u64,
    position: *const NrPosition,
    out_changed: *mut i32,
) -> i32 {
    with_engine(engine, |engine| {
        if position.is_null() {
            return Err(invalid("nr_save_position 的 position 不得为 NULL"));
        }
        let position = &*position;
        check_struct_size::<{ std::mem::size_of::<NrPosition>() }>(
            position.struct_size,
            "nr_position",
        )?;
        if position.content_unit_id != u64::from(crate::abi::CONTENT_UNIT_SINGLE) {
            return Err(invalid(format!(
                "content_unit_id 必须为 {}（V1 一个 TXT 只有一个内容单元）",
                crate::abi::CONTENT_UNIT_SINGLE
            )));
        }
        let fingerprint = read_utf8(
            position.nearby_fingerprint_utf8,
            position.nearby_fingerprint_len,
            "nearby_fingerprint",
        )?
        .filter(|text| !text.is_empty());
        let changed = engine.save_position(&PositionSpec {
            book_id: position.book_id,
            chapter_index: position.chapter_index,
            original_byte_offset: position.original_byte_offset,
            nearby_fingerprint: fingerprint,
        })?;
        if !out_changed.is_null() {
            std::ptr::write(out_changed, i32::from(changed));
        }
        Ok(())
    })
}

/// 读取阅读位置（`nr_get_position`）；指纹缓冲区由调用方归还。
///
/// # Safety
///
/// `out_result` 必须可写。
#[no_mangle]
pub unsafe extern "C" fn nr_get_position(
    engine: u64,
    book_id: u64,
    out_result: *mut NrPositionResult,
) -> i32 {
    with_engine(engine, |engine| {
        let outcome = engine.load_position(book_id)?;
        let fingerprint = NrBuffer::from_vec(
            outcome
                .nearby_fingerprint
                .clone()
                .unwrap_or_default()
                .into_bytes(),
        );
        write_out(
            out_result,
            NrPositionResult {
                struct_size: std::mem::size_of::<NrPositionResult>() as u32,
                found: u32::from(outcome.found),
                chapter_index: outcome.chapter_index,
                restored: u32::from(outcome.restored),
                fallback_to_chapter_start: u32::from(outcome.fallback_to_chapter_start),
                reserved0: 0,
                book_id: outcome.book_id,
                content_unit_id: u64::from(crate::abi::CONTENT_UNIT_SINGLE),
                original_byte_offset: outcome.original_byte_offset,
                updated_at: outcome.updated_at,
                nearby_fingerprint: fingerprint,
            },
        )
    })
}

/// 章节批次（`nr_get_chapters`）：缓冲区由调用方归还。
///
/// # Safety
///
/// `out_batch` 必须可写。
#[no_mangle]
pub unsafe extern "C" fn nr_get_chapters(
    engine: u64,
    book_id: u64,
    start_sequence: u32,
    max_count: u32,
    out_batch: *mut NrChapterBatch,
) -> i32 {
    with_engine(engine, |engine| {
        let batch = engine.chapter_batch(book_id, start_sequence, max_count)?;
        write_out(
            out_batch,
            NrChapterBatch {
                count: batch.count,
                index_version: batch.index_version,
                total_chapters: batch.total_chapters,
                reserved0: 0,
                bytes: NrBuffer::from_vec(batch.bytes),
            },
        )
    })
}

/// 新增书签（`nr_create_bookmark`）。
///
/// # Safety
///
/// `input` 必须指向当前 ABI 版本的 [`NrBookmarkInput`]，其标题指针在本调用期间有效。
#[no_mangle]
pub unsafe extern "C" fn nr_create_bookmark(
    engine: u64,
    input: *const NrBookmarkInput,
    out_bookmark_id: *mut u64,
) -> i32 {
    with_engine(engine, |engine| {
        if input.is_null() {
            return Err(invalid("nr_create_bookmark 的 input 不得为 NULL"));
        }
        let input = &*input;
        check_struct_size::<{ std::mem::size_of::<NrBookmarkInput>() }>(
            input.struct_size,
            "nr_bookmark_input",
        )?;
        if input.content_unit_id != u64::from(crate::abi::CONTENT_UNIT_SINGLE) {
            return Err(invalid(format!(
                "content_unit_id 必须为 {}",
                crate::abi::CONTENT_UNIT_SINGLE
            )));
        }
        let title = read_utf8(input.title_utf8, input.title_len, "title")?;
        let id = engine.create_bookmark(
            input.book_id,
            input.chapter_index,
            input.byte_offset,
            title.as_deref(),
        )?;
        write_out(out_bookmark_id, id)
    })
}

/// 删除书签（`nr_delete_bookmark`）；`out_deleted` 可为 `NULL`。
///
/// # Safety
///
/// `engine` 必须是有效句柄。
#[no_mangle]
pub unsafe extern "C" fn nr_delete_bookmark(
    engine: u64,
    bookmark_id: u64,
    out_deleted: *mut i32,
) -> i32 {
    with_engine(engine, |engine| {
        let deleted = engine.delete_bookmark(bookmark_id)?;
        if !out_deleted.is_null() {
            std::ptr::write(out_deleted, i32::from(deleted));
        }
        Ok(())
    })
}

/// 失效布局（`nr_invalidate_layout`）。
///
/// # Safety
///
/// `out_removed` 可为 `NULL`。
#[no_mangle]
pub unsafe extern "C" fn nr_invalidate_layout(
    engine: u64,
    layout_id: u64,
    out_removed: *mut u64,
) -> i32 {
    with_engine(engine, |engine| {
        let removed = engine.invalidate_layout(layout_id)?;
        if !out_removed.is_null() {
            std::ptr::write(out_removed, removed);
        }
        Ok(())
    })
}

/// 重建章节索引（`nr_rebuild_index`）；可被 [`nr_cancel_request`] 取消。
///
/// # Safety
///
/// `out_result` 必须可写。
#[no_mangle]
pub unsafe extern "C" fn nr_rebuild_index(
    engine: u64,
    book_id: u64,
    request_id: u64,
    out_result: *mut NrRebuildResult,
) -> i32 {
    with_engine(engine, |engine| {
        let outcome = engine.rebuild_index(book_id, request_id)?;
        write_out(
            out_result,
            NrRebuildResult {
                struct_size: std::mem::size_of::<NrRebuildResult>() as u32,
                partial: u32::from(outcome.partial),
                chapters: outcome.chapters,
                index_from_cache: u32::from(outcome.index_from_cache),
                scanned_bytes: outcome.scanned_bytes,
                total_bytes: outcome.total_bytes,
                content_index_version: outcome.content_index_version,
            },
        )?;
        if outcome.partial {
            Err(cancel::cancelled_error(request_id))
        } else {
            Ok(())
        }
    })
}

/// 手动指定书籍内容编码（`nr_set_encoding`，第 19 项导出）。
///
/// `encoding_code` 取 `NR_ENCODING_*`（1..=7）；`0` = 清除覆盖、回到自动检测。
/// 只改变解释方式，不写回原文件；返回后内容索引版本与章节数按新编码更新。
///
/// # Safety
///
/// `out_result` 必须指向可写的 [`NrSetEncodingResult`]（可为 NULL：此时只做副作用不写回执）。
#[no_mangle]
pub unsafe extern "C" fn nr_set_encoding(
    engine: u64,
    book_id: u64,
    encoding_code: u32,
    out_result: *mut NrSetEncodingResult,
) -> i32 {
    with_engine(engine, |engine| {
        let outcome = engine.set_encoding(book_id, encoding_code)?;
        write_out(
            out_result,
            NrSetEncodingResult {
                struct_size: std::mem::size_of::<NrSetEncodingResult>() as u32,
                encoding: encoding_code_of(outcome.encoding),
                content_index_version: outcome.content_index_version,
                chapters: outcome.chapters,
                index_rebuilt: u32::from(outcome.index_rebuilt),
            },
        )
    })
}

/// [`TextEncoding`] → `NR_ENCODING_*` 码（与 `EncodingCode` 一致）。
fn encoding_code_of(encoding: TextEncoding) -> u32 {
    match encoding {
        TextEncoding::Utf8 => 1,
        TextEncoding::Utf8Bom => 2,
        TextEncoding::Utf16Le => 3,
        TextEncoding::Utf16Be => 4,
        TextEncoding::Gb18030 => 5,
        TextEncoding::Gbk => 6,
        TextEncoding::Big5 => 7,
    }
}

/// 取消请求（`nr_cancel_request`）：协作式、跨线程可见。
///
/// # Safety
///
/// `engine` 必须是有效句柄；本调用不持有引擎互斥锁的等待（长任务运行期间也能生效）。
#[no_mangle]
pub unsafe extern "C" fn nr_cancel_request(engine: u64, request_id: u64) -> i32 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        handle::lookup(engine)?;
        cancel::request_cancel(engine, request_id)
    }));
    match result {
        Ok(Ok(())) => 0,
        Ok(Err(error)) => error.code().as_c(),
        Err(_) => ErrorCode::Internal.as_c(),
    }
}

/// 最近一次错误（`nr_get_last_error`）；消息缓冲区由调用方归还。
///
/// # Safety
///
/// `out_info` 必须可写。
#[no_mangle]
pub unsafe extern "C" fn nr_get_last_error(engine: u64, out_info: *mut NrErrorInfo) -> i32 {
    with_engine(engine, |engine| {
        let (code, message) = match engine.last_error() {
            Some(error) => (error.code().as_c(), error.message().to_string()),
            None => (0, String::new()),
        };
        let has_message = u32::from(!message.is_empty());
        write_out(
            out_info,
            NrErrorInfo {
                code,
                has_message,
                message: NrBuffer::from_vec(message.into_bytes()),
            },
        )
    })
}

/// 归还 Rust 分配的缓冲区（`nr_release_buffer`）。
///
/// 归还后 `buf` 的三个字段全部置零；对已归零的缓冲区再次调用是安全 no-op。
/// 本函数不需要引擎句柄，因此引擎销毁后仍可归还此前取得的缓冲区。
///
/// # Safety
///
/// `buf` 必须指向由本库分配并尚未归还的 [`NrBuffer`]（或已归零的同一结构体）。
#[no_mangle]
pub unsafe extern "C" fn nr_release_buffer(buf: *mut NrBuffer) -> i32 {
    let result = catch_unwind(AssertUnwindSafe(|| release_buffer(buf)));
    match result {
        Ok(Ok(())) => 0,
        Ok(Err(error)) => error.code().as_c(),
        Err(_) => ErrorCode::Internal.as_c(),
    }
}

/// 由 C 侧布局签名构造核心签名。
unsafe fn read_layout_signature(ptr: *const NrLayoutSignature) -> NrResult<LayoutSignature> {
    if ptr.is_null() {
        return Err(invalid(
            "BEGIN_LAYOUT 阶段必须提供 nr_layout_signature（重排必须携带完整口径）",
        ));
    }
    let raw = &*ptr;
    if raw.reserved0 != 0 {
        return Err(invalid("nr_layout_signature.reserved0 必须为 0"));
    }
    let direction = TextDirection::from_id(raw.direction as i32)
        .ok_or_else(|| invalid(format!("排版方向 {} 未登记（0 横 / 1 纵）", raw.direction)))?;
    let font_family =
        read_utf8(raw.font_family_utf8, raw.font_family_len, "font_family")?.unwrap_or_default();
    let signature = LayoutSignature {
        viewport_width: raw.viewport_width,
        viewport_height: raw.viewport_height,
        direction,
        font_family,
        font_weight: raw.font_weight as u16,
        font_size_milli: raw.font_size_milli,
        line_spacing_milli: raw.line_spacing_milli,
        paragraph_spacing_milli: raw.paragraph_spacing_milli,
        first_line_indent_milli: raw.first_line_indent_milli,
        margin_horizontal_milli: raw.margin_horizontal_milli,
        margin_vertical_milli: raw.margin_vertical_milli,
        algorithm_version: raw.algorithm_version,
        platform_font_fingerprint: raw.platform_font_fingerprint,
    };
    signature.validate()?;
    Ok(signature)
}

fn boundary_result(outcome: &crate::engine::BoundaryOutcome) -> NrBoundaryResult {
    NrBoundaryResult {
        status: outcome.status,
        rejection: outcome.rejection,
        error_code: outcome.error_code,
        reserved0: 0,
        layout_id: outcome.layout_id,
        request_id: outcome.request_id,
        stale_layout_id: outcome.stale_layout_id,
        stale_request_id: outcome.stale_request_id,
        start_byte: outcome.start_byte,
        end_byte: outcome.end_byte,
        total_bytes: outcome.total_bytes,
        page_count: outcome.page_count,
    }
}

/// 把页边界状态映射为「最近一次错误」，便于平台用 `nr_get_last_error` 取详情。
fn record_outcome_error(engine: &mut Engine, code: i32, outcome: &crate::engine::BoundaryOutcome) {
    if code == 0 {
        return;
    }
    engine.record_error(crate::abi::error_from_code(
        code,
        format!(
            "页边界状态 {}（拒绝原因 {}）",
            outcome.status, outcome.rejection
        ),
    ));
}

/// 供测试与文档使用：冻结入口清单。
pub fn entry_names() -> &'static [&'static str; 19] {
    &ABI_ENTRY_NAMES
}

/// 供测试使用：把 `c_void` 指针类型转换集中在这里。
pub fn void_ptr<T>(value: *const T) -> *const c_void {
    value.cast::<c_void>()
}

/// 供测试使用：编码稳定数值（校验 ABI 映射完整）。
pub fn encoding_code(encoding: TextEncoding) -> u32 {
    crate::abi::EncodingCode::from_encoding(encoding).as_u32()
}

/// 供测试使用：单次章节批次上限。
pub const fn max_chapter_batch() -> u32 {
    MAX_CHAPTER_BATCH
}

/// 供测试使用：最大锚点引用长度（附近文本指纹长度上限的来源）。
pub const fn max_anchor_quote_bytes() -> usize {
    ANCHOR_MAX_QUOTE_BYTES
}

/// 保持 [`NrError`] 在本模块可见（用于文档链接与类型推断）。
#[allow(dead_code)]
fn _error_marker(_error: NrError) {}

/// 供测试使用：`BEGIN_LAYOUT` 状态常量。
pub const fn boundary_layout_ready() -> u32 {
    BOUNDARY_LAYOUT_READY
}

/// 供测试使用：`COMMIT` 阶段常量。
pub const fn submit_commit_phase() -> u32 {
    SUBMIT_COMMIT
}

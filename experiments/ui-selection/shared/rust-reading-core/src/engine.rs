//! 共享阅读业务引擎：生命周期、有限窗口、位置、锚点、页面边界、
//! 布局失效、事件、取消与测量（S-R1/S-R2/S-R3/S-R5）。
//!
//! UI 端（Slint / Kotlin）只可调用本模块或 `ffi`，不得重实现其中语义。

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::c_void;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::{Book, Chapter, Library};
use crate::error::{
    book_not_found, chapter_out_of_range, invalid_argument, no_open_book, out_of_range, ErrorCode,
    NrError, NrResult,
};
use crate::json;
use crate::position::{locate_anchor, make_anchor, ContentPosition, PageAnchor, PageBoundary};
use crate::source::{FileTextSource, SyntheticTextSource, TextSource};

/// 事件订阅 id。
pub type SubscribeId = u64;

/// C ABI 事件回调：`(user, utf8_json_ptr, utf8_json_len)`。
/// 回调在触发线程同步执行，`data` 仅在回调期间有效。
pub type NrEventCallback = extern "C" fn(user: *mut c_void, data: *const u8, len: usize);

/// 打开书籍时的正文来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BookSource {
    /// 真实文件（seek + 有限窗口）。
    File(PathBuf),
    /// 合成文本源（确定性，不占内存）。
    Synthetic { total_bytes: u64 },
}

/// 一次有限窗口读取的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadWindow {
    /// 章节序号。
    pub chapter_index: u32,
    /// 实际起始绝对字节偏移（已修正到 UTF-8 边界）。
    pub offset: u64,
    /// 实际起始章节内字节偏移（已修正到 UTF-8 边界）。
    pub chapter_offset: u64,
    /// 合法 UTF-8 字节。
    pub bytes: Vec<u8>,
    /// 请求读取的原始字节数。
    pub raw_len: usize,
    /// 为修正边界而丢弃的前导字节数。
    pub dropped_leading: usize,
    /// 尾部是否因 UTF-8 边界被截断。
    pub trimmed_tail: bool,
    /// 是否已到全书末尾。
    pub eof: bool,
    /// 是否已到本章末尾。
    pub chapter_eof: bool,
}

impl ReadWindow {
    /// 窗口文本（保证合法 UTF-8，否则返回空串）。
    pub fn text(&self) -> &str {
        std::str::from_utf8(&self.bytes).unwrap_or("")
    }
}

/// 事件类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    LibraryReady,
    BookOpened,
    WindowRead,
    RequestStarted,
    RequestProgress,
    BatchLoaded,
    RequestCancelled,
    PositionSaved,
    PageBoundaryCommitted,
    LayoutInvalidated,
    Error,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::LibraryReady => "library_ready",
            EventKind::BookOpened => "book_opened",
            EventKind::WindowRead => "window_read",
            EventKind::RequestStarted => "request_started",
            EventKind::RequestProgress => "request_progress",
            EventKind::BatchLoaded => "batch_loaded",
            EventKind::RequestCancelled => "request_cancelled",
            EventKind::PositionSaved => "position_saved",
            EventKind::PageBoundaryCommitted => "page_boundary_committed",
            EventKind::LayoutInvalidated => "layout_invalidated",
            EventKind::Error => "error",
        }
    }
}

/// 事件负载（跨语言以 JSON 传输）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub seq: u64,
    pub kind: EventKind,
    pub request_id: u64,
    pub detail: String,
}

impl Event {
    /// UTF-8 JSON 形态。
    pub fn to_json(&self) -> String {
        format!(
            "{{\"seq\":{},\"kind\":{},\"requestId\":{},\"detail\":{}}}",
            self.seq,
            json::quoted(self.kind.as_str()),
            self.request_id,
            json::quoted(&self.detail)
        )
    }
}

/// 事件订阅者。
enum EventSink {
    Rust(Box<dyn FnMut(&Event)>),
    CApi {
        cb: NrEventCallback,
        user: *mut c_void,
    },
}

/// 测量指标（S-R5）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Metrics {
    /// `read_window` 调用次数。
    pub read_calls: u64,
    /// 跨边界复制到 UI 的字节数。
    pub copied_bytes: u64,
    /// 从数据源实际物化的字节数（证明有界内存）。
    pub materialized_bytes: u64,
    /// 最近一次排版耗时（微秒）。
    pub last_layout_us: u64,
    /// 已发出事件数。
    pub events_emitted: u64,
    /// 被取消的请求数。
    pub cancelled_requests: u64,
    /// 已登记页面边界数。
    pub page_boundaries: u64,
}

/// 异步请求进度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestProgress {
    pub done: u32,
    pub total: u32,
    pub cancelled: bool,
    pub finished: bool,
}

#[derive(Debug, Clone, Copy)]
struct RequestState {
    total: u32,
    done: u32,
    cancelled: bool,
}

/// 默认单次窗口上限 256 KiB（PNFR3：绝不整体载入大文件）。
pub const DEFAULT_MAX_WINDOW: usize = 256 * 1024;

/// 共享阅读业务引擎。
pub struct ReadingEngine {
    lib: Library,
    source: Option<Box<dyn TextSource>>,
    /// 外部内容的坐标空间长度（如 Android SAF 文件）；`None` 表示使用模拟书库。
    external_extent: Option<u64>,
    opened_book: Option<u64>,
    chapter_index: u32,
    position: ContentPosition,
    boundaries: BTreeMap<(u64, u32, u64), PageBoundary>,
    sinks: Vec<(SubscribeId, EventSink)>,
    next_subscribe: SubscribeId,
    metrics: Metrics,
    max_window: usize,
    next_request: u64,
    requests: BTreeMap<u64, RequestState>,
    cancelled: BTreeSet<u64>,
    last_error: Option<NrError>,
    seq: u64,
}

impl ReadingEngine {
    /// 创建引擎并生成默认模拟书库。
    pub fn new(max_window_bytes: usize) -> Self {
        Self::with_library(Library::generate(20260905), max_window_bytes)
    }

    /// 使用指定书库创建引擎。
    pub fn with_library(lib: Library, max_window_bytes: usize) -> Self {
        let mut engine = Self {
            lib,
            source: None,
            external_extent: None,
            opened_book: None,
            chapter_index: 0,
            position: ContentPosition::default(),
            boundaries: BTreeMap::new(),
            sinks: Vec::new(),
            next_subscribe: 1,
            metrics: Metrics::default(),
            max_window: if max_window_bytes == 0 {
                DEFAULT_MAX_WINDOW
            } else {
                max_window_bytes
            },
            next_request: 1,
            requests: BTreeMap::new(),
            cancelled: BTreeSet::new(),
            last_error: None,
            seq: 0,
        };
        engine.emit(
            EventKind::LibraryReady,
            0,
            format!("books={}", engine.lib.books.len()),
        );
        engine
    }

    /// 模拟书库。
    pub fn library(&self) -> &Library {
        &self.lib
    }

    /// 单次窗口上限。
    pub fn max_window(&self) -> usize {
        self.max_window
    }

    /// 当前打开的书。
    pub fn opened_book(&self) -> Option<u64> {
        self.opened_book
    }

    /// 当前阅读位置。
    pub fn position(&self) -> ContentPosition {
        self.position
    }

    /// 指标快照。
    pub fn metrics(&self) -> Metrics {
        let mut m = self.metrics;
        m.page_boundaries = self.boundaries.len() as u64;
        m
    }

    /// 最近一次错误。
    pub fn last_error(&self) -> Option<&NrError> {
        self.last_error.as_ref()
    }

    /// 打开书籍。
    pub fn open_book(&mut self, book_id: u64, source: BookSource) -> NrResult<()> {
        let book = self
            .lib
            .book(book_id)
            .ok_or_else(|| book_not_found(book_id))?;
        let total = book.total_bytes;
        let src: Box<dyn TextSource> = match source {
            BookSource::File(path) => Box::new(FileTextSource::open(path)?),
            BookSource::Synthetic { total_bytes } => Box::new(SyntheticTextSource::new(
                if total_bytes == 0 { total } else { total_bytes },
                book_id,
            )),
        };
        self.source = Some(src);
        self.external_extent = None;
        self.opened_book = Some(book_id);
        self.chapter_index = 0;
        self.position = ContentPosition {
            book_id,
            chapter_index: 0,
            byte_offset: 0,
        };
        let desc = self
            .source
            .as_ref()
            .map(|s| s.describe())
            .unwrap_or_default();
        self.emit(
            EventKind::BookOpened,
            0,
            format!("book={book_id} source={desc}"),
        );
        Ok(())
    }

    /// 直接注入文本源（测试与嵌入式使用）。
    pub fn open_source(&mut self, book_id: u64, source: Box<dyn TextSource>) -> NrResult<()> {
        self.lib
            .book(book_id)
            .ok_or_else(|| book_not_found(book_id))?;
        self.source = Some(source);
        self.external_extent = None;
        self.opened_book = Some(book_id);
        self.chapter_index = 0;
        Ok(())
    }

    /// 打开**外部内容**（例如 Android SAF 选中的文件）。
    ///
    /// 共享库不持有字节，只登记「单章、坐标空间长度 = `total_bytes`」；
    /// 实际字节由平台侧按有限窗口提供。页边界登记、位置校验与锚点语义
    /// 仍全部由共享库裁定（S-R3 / H-R4）。
    pub fn open_external(&mut self, book_id: u64, total_bytes: u64) -> NrResult<()> {
        self.lib
            .book(book_id)
            .ok_or_else(|| book_not_found(book_id))?;
        if total_bytes == 0 {
            return Err(invalid_argument("外部内容长度必须大于 0"));
        }
        self.source = Some(Box::new(crate::source::ExternalTextSource::new(
            total_bytes,
            format!("book{book_id}"),
        )));
        self.external_extent = Some(total_bytes);
        self.opened_book = Some(book_id);
        self.chapter_index = 0;
        self.position = ContentPosition {
            book_id,
            chapter_index: 0,
            byte_offset: 0,
        };
        self.emit(
            EventKind::BookOpened,
            0,
            format!("book={book_id} source=external:{total_bytes}bytes"),
        );
        Ok(())
    }

    /// 章节文本在全书中的起始偏移。
    ///
    /// 外部内容（`external_extent` 为 Some）在「已打开的那本书」上视为**单章**，
    /// 坐标空间为 `[0, total_bytes)`，字节由平台侧提供。
    fn space_for(&self, book_id: u64, chapter_index: u32) -> NrResult<(u64, u64)> {
        if let Some(total) = self.external_extent {
            if self.opened_book == Some(book_id) {
                if chapter_index != 0 {
                    return Err(chapter_out_of_range(chapter_index));
                }
                return Ok((0, total));
            }
        }
        let book = self
            .lib
            .book(book_id)
            .ok_or_else(|| book_not_found(book_id))?;
        if chapter_index >= book.chapter_count {
            return Err(chapter_out_of_range(chapter_index));
        }
        Ok((
            Library::chapter_byte_offset(book, chapter_index),
            Library::chapter_byte_len(book, chapter_index),
        ))
    }

    /// 当前书的章节坐标空间。
    fn chapter_offset(&self, chapter_index: u32) -> NrResult<(u64, u64)> {
        let book_id = self.opened_book.ok_or_else(no_open_book)?;
        self.space_for(book_id, chapter_index)
    }

    /// 有限窗口读取（章节内偏移），自动修正 UTF-8 边界。
    pub fn read_window(
        &mut self,
        chapter_index: u32,
        offset: u64,
        len: usize,
    ) -> NrResult<ReadWindow> {
        let (ch_off, ch_len) = self.chapter_offset(chapter_index)?;
        if offset > ch_len {
            return Err(out_of_range(format!(
                "章节内偏移 {offset} 超出章节长度 {ch_len}"
            )));
        }
        let total = self.source.as_ref().map(|s| s.total_bytes()).unwrap_or(0);
        let want = len.min(self.max_window).min((ch_len - offset) as usize);
        let global = ch_off + offset;
        let raw = self
            .source
            .as_ref()
            .ok_or_else(no_open_book)?
            .read_at(global, want)?;
        let raw_len = raw.len();
        let (bytes, dropped_leading, trimmed_tail) = fix_utf8_window(raw)?;
        let eof = global + raw_len as u64 >= total;
        let chapter_offset = offset + dropped_leading as u64;
        let chapter_eof = chapter_offset + raw_len as u64 >= ch_len;
        self.metrics.read_calls += 1;
        self.metrics.copied_bytes += bytes.len() as u64;
        self.metrics.materialized_bytes += raw_len as u64;
        self.chapter_index = chapter_index;
        self.emit(
            EventKind::WindowRead,
            0,
            format!(
                "chapter={chapter_index} offset={offset} bytes={}",
                bytes.len()
            ),
        );
        Ok(ReadWindow {
            chapter_index,
            offset: global + dropped_leading as u64,
            chapter_offset,
            bytes,
            raw_len,
            dropped_leading,
            trimmed_tail,
            eof,
            chapter_eof,
        })
    }

    /// 读取当前章节的一段文本（供排版/渲染使用，上限 `max_window`）。
    ///
    /// 注意：章节起点是按字节均分得到的**近似**偏移，可能落在字符中间；
    /// 此时返回值会向后对齐到合法 UTF-8 边界（S-R2）。
    pub fn chapter_text_window(
        &mut self,
        chapter_index: u32,
        offset: u64,
        max_bytes: usize,
    ) -> NrResult<String> {
        Ok(self
            .chapter_text_window_at(chapter_index, offset, max_bytes)?
            .1)
    }

    /// 同上，但额外返回**实际**起始章节内偏移（已对齐 UTF-8 边界）。
    pub fn chapter_text_window_at(
        &mut self,
        chapter_index: u32,
        offset: u64,
        max_bytes: usize,
    ) -> NrResult<(u64, String)> {
        let w = self.read_window(chapter_index, offset, max_bytes)?;
        let base = w.chapter_offset;
        Ok((base, w.text().to_string()))
    }

    /// 书籍分页批次。
    pub fn book_batch(&self, start: usize, count: usize) -> Vec<Book> {
        self.lib.books_batch(start, count).to_vec()
    }

    /// 章节分页批次。
    pub fn chapter_batch(&self, book_id: u64, start: u32, count: u32) -> Vec<Chapter> {
        let mut out = Vec::new();
        for idx in start..start.saturating_add(count) {
            match self.lib.chapter(book_id, idx) {
                Some(c) => out.push(c),
                None => break,
            }
        }
        out
    }

    /// 保存阅读位置（外部内容按单章坐标空间校验）。
    pub fn save_position(&mut self, pos: ContentPosition) -> NrResult<()> {
        let (_, ch_len) = self.space_for(pos.book_id, pos.chapter_index)?;
        if pos.byte_offset > ch_len {
            return Err(out_of_range(format!(
                "位置偏移 {} 超出章节长度 {ch_len}",
                pos.byte_offset
            )));
        }
        self.position = pos;
        self.emit(
            EventKind::PositionSaved,
            0,
            format!(
                "book={} chapter={} offset={}",
                pos.book_id, pos.chapter_index, pos.byte_offset
            ),
        );
        Ok(())
    }

    /// 登记页面边界（任一端排版结果均可登记）。
    pub fn commit_page_boundary(&mut self, boundary: PageBoundary) -> NrResult<()> {
        if boundary.end_byte < boundary.start_byte {
            return Err(invalid_argument("页面边界 end < start"));
        }
        let (ch_off, ch_len) = self.chapter_offset(boundary.chapter_index)?;
        if boundary.end_byte > ch_len {
            return Err(out_of_range(format!(
                "页面边界 {} 超出章节长度 {ch_len}",
                boundary.end_byte
            )));
        }
        let key = (
            boundary.layout_id,
            boundary.chapter_index,
            boundary.start_byte,
        );
        self.boundaries.insert(key, boundary);
        self.emit(
            EventKind::PageBoundaryCommitted,
            0,
            format!(
                "chapter={} start={} end={} layout={} chapter_base={ch_off}",
                boundary.chapter_index, boundary.start_byte, boundary.end_byte, boundary.layout_id
            ),
        );
        Ok(())
    }

    /// 查询某排版标识下的页面边界。
    pub fn page_boundaries(&self, layout_id: u64, chapter_index: u32) -> Vec<PageBoundary> {
        self.boundaries
            .values()
            .filter(|b| b.layout_id == layout_id && b.chapter_index == chapter_index)
            .copied()
            .collect()
    }

    /// 该排版标识下的全部页面边界。
    pub fn all_page_boundaries(&self, layout_id: u64) -> Vec<PageBoundary> {
        self.boundaries
            .values()
            .filter(|b| b.layout_id == layout_id)
            .copied()
            .collect()
    }

    /// 排版配置变化后失效缓存，返回清除数量。
    pub fn invalidate_layout(&mut self, layout_id: u64) -> usize {
        let before = self.boundaries.len();
        self.boundaries.retain(|k, _| k.0 != layout_id);
        let removed = before - self.boundaries.len();
        self.emit(
            EventKind::LayoutInvalidated,
            0,
            format!("layout={layout_id} removed={removed}"),
        );
        removed
    }

    /// 清空全部边界缓存。
    pub fn invalidate_all_layouts(&mut self) -> usize {
        let removed = self.boundaries.len();
        self.boundaries.clear();
        self.emit(
            EventKind::LayoutInvalidated,
            0,
            format!("layout=all removed={removed}"),
        );
        removed
    }

    /// 为某章节的一段范围生成锚点（读取有限窗口构造页首引用）。
    ///
    /// 返回的 `start_byte` 是实际（对齐后）的页首偏移；若传入的 `start_byte`
    /// 落在字符中间，会被修正到合法边界。
    pub fn anchor_for_range(
        &mut self,
        chapter_index: u32,
        start_byte: u64,
        end_byte: u64,
    ) -> NrResult<PageAnchor> {
        let (base, text) = self.chapter_text_window_at(chapter_index, start_byte, 512)?;
        let mut anchor = make_anchor(&text, chapter_index, 0, end_byte.saturating_sub(base));
        anchor.start_byte = base;
        Ok(anchor)
    }

    /// 依据锚点在章节内恢复原文位置（限定搜索半径，保持有界读取）。
    ///
    /// 内部会读取 `[start - radius, start + radius]` 的有限窗口，并自动把
    /// 锚点提示偏移换算到窗口坐标系后再做「就近匹配」，返回**章节内**偏移。
    pub fn locate_anchor_near(
        &mut self,
        chapter_index: u32,
        anchor: &PageAnchor,
        radius: u64,
    ) -> NrResult<u64> {
        let from = anchor.start_byte.saturating_sub(radius);
        let (base, text) =
            self.chapter_text_window_at(chapter_index, from, (radius * 2) as usize + 1024)?;
        // locate_anchor 的坐标系是传入文本自身，必须把提示偏移换算到窗口内，
        // 否则「就近匹配」会以错误的原点挑选重复引用。
        let mut window_anchor = anchor.clone();
        window_anchor.start_byte = anchor.start_byte.saturating_sub(base);
        let local = locate_anchor(&text, &window_anchor);
        Ok(base + local)
    }

    /// 订阅事件（Rust 回调）。
    pub fn subscribe<F: FnMut(&Event) + 'static>(&mut self, f: F) -> SubscribeId {
        let id = self.next_subscribe;
        self.next_subscribe += 1;
        self.sinks.push((id, EventSink::Rust(Box::new(f))));
        id
    }

    /// 订阅事件（C ABI 回调）。
    pub fn subscribe_cabi(&mut self, cb: NrEventCallback, user: *mut c_void) -> SubscribeId {
        let id = self.next_subscribe;
        self.next_subscribe += 1;
        self.sinks.push((id, EventSink::CApi { cb, user }));
        id
    }

    /// 取消订阅。
    pub fn unsubscribe(&mut self, id: SubscribeId) -> bool {
        let before = self.sinks.len();
        self.sinks.retain(|(sid, _)| *sid != id);
        self.sinks.len() != before
    }

    /// 发出事件。
    fn emit(&mut self, kind: EventKind, request_id: u64, detail: String) {
        self.seq += 1;
        self.metrics.events_emitted += 1;
        let ev = Event {
            seq: self.seq,
            kind,
            request_id,
            detail,
        };
        let payload = ev.to_json();
        for (_, sink) in self.sinks.iter_mut() {
            match sink {
                EventSink::Rust(f) => f(&ev),
                EventSink::CApi { cb, user } => {
                    let bytes = payload.as_bytes();
                    cb(*user, bytes.as_ptr(), bytes.len());
                }
            }
        }
    }

    /// 发起模拟批次请求（可取消）。
    pub fn start_batch_request(&mut self, total: u32) -> u64 {
        let id = self.next_request;
        self.next_request += 1;
        self.requests.insert(
            id,
            RequestState {
                total,
                done: 0,
                cancelled: false,
            },
        );
        self.cancelled.remove(&id);
        self.emit(EventKind::RequestStarted, id, format!("total={total}"));
        id
    }

    /// 推进模拟请求。
    ///
    /// 取消后返回部分成功（`done` 为已加载条数，`cancelled = true`）。
    pub fn advance_request(&mut self, request_id: u64, steps: u32) -> NrResult<RequestProgress> {
        let (total, done, cancelled) = {
            let st = self
                .requests
                .get_mut(&request_id)
                .ok_or_else(|| invalid_argument(format!("请求 {request_id} 不存在")))?;
            if !st.cancelled {
                st.done = st.done.saturating_add(steps).min(st.total);
            }
            (st.total, st.done, st.cancelled)
        };
        if cancelled {
            return Ok(RequestProgress {
                done,
                total,
                cancelled: true,
                finished: false,
            });
        }
        self.emit(
            EventKind::RequestProgress,
            request_id,
            format!("done={done}/{total}"),
        );
        let finished = done >= total;
        if finished {
            self.requests.remove(&request_id);
            self.emit(EventKind::BatchLoaded, request_id, format!("done={done}"));
        }
        Ok(RequestProgress {
            done,
            total,
            cancelled: false,
            finished,
        })
    }

    /// 取消请求（协作式；未开始的请求会记录取消标记）。
    pub fn cancel_request(&mut self, request_id: u64) -> NrResult<()> {
        let mut hit = false;
        if let Some(st) = self.requests.get_mut(&request_id) {
            st.cancelled = true;
            hit = true;
        }
        self.cancelled.insert(request_id);
        self.metrics.cancelled_requests += 1;
        self.emit(
            EventKind::RequestCancelled,
            request_id,
            format!("known={hit}"),
        );
        Ok(())
    }

    /// 请求是否已被取消。
    pub fn is_cancelled(&self, request_id: u64) -> bool {
        self.cancelled.contains(&request_id)
    }

    /// 结果快照 JSON（供 ready.json / 结果记录引用）。
    pub fn snapshot_json(&self) -> String {
        let m = self.metrics();
        format!(
            "{{\"core\":{},\"abiVersion\":{},\"abiEntries\":{},\"books\":{},\"categories\":{},\"history\":{},\"maxWindow\":{},\"openedBook\":{},\"metrics\":{{\"readCalls\":{},\"copiedBytes\":{},\"materializedBytes\":{},\"lastLayoutUs\":{},\"eventsEmitted\":{},\"cancelledRequests\":{},\"pageBoundaries\":{}}}}}",
            json::quoted(crate::CORE_VERSION),
            crate::ABI_VERSION,
            crate::ABI_ENTRY_COUNT,
            self.lib.books.len(),
            self.lib.categories.len(),
            self.lib.history.len(),
            self.max_window,
            self.opened_book.map(|b| b.to_string()).unwrap_or_else(|| "null".into()),
            m.read_calls,
            m.copied_bytes,
            m.materialized_bytes,
            m.last_layout_us,
            m.events_emitted,
            m.cancelled_requests,
            m.page_boundaries
        )
    }
}

/// 修正窗口的 UTF-8 边界：
/// - 丢弃开头落在字符中间的续字节（返回丢弃数）；
/// - 若尾部是不完整序列则截断（返回 `trimmed_tail`）；
/// - 遇到真正的非法字节返回错误。
pub fn fix_utf8_window(mut raw: Vec<u8>) -> NrResult<(Vec<u8>, usize, bool)> {
    let mut dropped = 0usize;
    while let Some(&b) = raw.first() {
        if b & 0xC0 == 0x80 {
            raw.remove(0);
            dropped += 1;
        } else {
            break;
        }
    }
    match std::str::from_utf8(&raw) {
        Ok(_) => Ok((raw, dropped, false)),
        Err(e) => {
            if e.error_len().is_none() {
                let valid = e.valid_up_to();
                raw.truncate(valid);
                Ok((raw, dropped, true))
            } else {
                Err(NrError::new(
                    ErrorCode::IoError,
                    format!("正文不是合法 UTF-8（位置 {}）", e.valid_up_to()),
                ))
            }
        }
    }
}

/// 当前 UNIX 毫秒时间戳（结果记录用）。
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

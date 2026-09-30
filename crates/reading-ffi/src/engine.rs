//! 阅读引擎：把 frozen 入口（18 项 + `nr_set_encoding`）映射到 `reading-source` / `reading-index` /
//! `reading-core` / `reading-storage` 的真实语义上。
//!
//! # 边界（`design.md` §4、`reading-core` CR6）
//!
//! - **串行化**：一个引擎的打开、切章、翻页、重排、保存位置在同一把引擎互斥锁下串行执行；
//! - **不做业务复制**：位置、分页边界、章节、进度与书签语义全部来自共享核心，
//!   本层只做参数编解码、所有权管理与持久化编排；
//! - **不整本读取**：所有正文访问都经 [`reading_source::TextSource`] 的受限窗口
//!   （平台数据源经 [`crate::external::PlatformWindowSource`] 的受限回调）；
//! - **不保存正文**：只在 SQLite 中写元数据、索引、位置与短摘要（书签名称、章节标题、
//!   附近文本指纹），长度上限由 `reading-storage` 强制。

use std::path::PathBuf;

use reading_core::{
    chapter_index_for_offset, generate_anchor, map_source_error, restore_position, AnchorSearch,
    BoundaryCommitOutcome, BoundaryCommitStatus, ChapterIndex, ContentPosition, ErrorCode,
    LayoutSignature, NrError, NrResult, PageBoundaryRequest, PaginationCache,
    DEFAULT_ANCHOR_RADIUS_BYTES, VIRTUAL_CHAPTER_TITLE,
};
use reading_index::ChapterRule;
use reading_source::{
    detect_encoding, FileTextSource, Fingerprint, ReadStats, SourceError, SourceMetadata,
    TextEncoding, TextSource, TextWindow,
};
use reading_storage::validate::{MAX_FINGERPRINT_BYTES, MAX_NAME_BYTES, MAX_TITLE_BYTES};
use reading_storage::{
    bookmarks, books, connection, index as storage_index, meta, progress, Storage, StorageError,
};

use crate::abi::{
    book_not_found, chapter_out_of_range, confidence_permille, invalid, layout_not_found,
    no_open_book, out_of_range, rejection_id, ABI_VERSION, BOUNDARY_LAYOUT_READY,
    BOUNDARY_NOT_FOUND, CONTENT_UNIT_SINGLE, MAX_CHAPTER_BATCH, MAX_WINDOW_BYTES, SOURCE_KIND_FILE,
    SOURCE_KIND_PLATFORM,
};
use crate::cancel;
use crate::external::{NrSourceProvider, PlatformWindowSource};
use crate::structs::NrChapter;

/// 引擎配置（Rust 侧形态；`nr_engine_config` 的解析结果）。
/// 用户手动指定编码时的置信度（千分比上限：手动指定视为完全可信）。
const USER_OVERRIDE_CONFIDENCE_PERMILLE: i32 = 1000;

#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// 单次读取窗口上限（已校验并夹紧到硬上限）。
    pub max_window_bytes: usize,
    /// 平台窗口提供者。
    pub provider: Option<NrSourceProvider>,
    /// 数据库路径；`None` 表示内存数据库。
    pub db_path: Option<PathBuf>,
}

impl EngineConfig {
    /// 内存数据库、无平台提供者（测试与只读会话）。
    pub fn in_memory() -> Self {
        Self {
            max_window_bytes: MAX_WINDOW_BYTES,
            provider: None,
            db_path: None,
        }
    }

    /// 夹具化的默认窗口上限（64 KiB），用于验证"请求超过引擎上限被拒绝"。
    pub fn with_default_window() -> Self {
        Self {
            max_window_bytes: crate::abi::DEFAULT_WINDOW_BYTES,
            ..Self::in_memory()
        }
    }
}

/// 数据源句柄（文件或平台窗口）。
pub enum SourceHandle {
    /// 本地文件（桌面路径 / 私有缓存副本）。
    File(FileTextSource),
    /// 平台窗口（Android SAF 等，字节由平台按受限窗口提供）。
    Platform(PlatformWindowSource),
}

impl SourceHandle {
    /// 动态数据源视图。
    pub fn as_source(&self) -> &dyn TextSource {
        match self {
            SourceHandle::File(source) => source,
            SourceHandle::Platform(source) => source,
        }
    }

    /// 是否为平台数据源。
    pub fn is_platform(&self) -> bool {
        matches!(self, SourceHandle::Platform(_))
    }

    /// 文本编码稳定名（落库用）。
    pub fn encoding_name(encoding: TextEncoding) -> &'static str {
        encoding.as_str()
    }
}

impl TextSource for SourceHandle {
    fn metadata(&self) -> SourceMetadata {
        self.as_source().metadata()
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<TextWindow, SourceError> {
        self.as_source().read_at(offset, len)
    }

    fn read_at_with_encoding(
        &self,
        offset: u64,
        len: usize,
        encoding: TextEncoding,
    ) -> Result<TextWindow, SourceError> {
        self.as_source().read_at_with_encoding(offset, len, encoding)
    }

    fn fingerprint(&self) -> Result<Fingerprint, SourceError> {
        self.as_source().fingerprint()
    }

    fn can_random_access(&self) -> bool {
        self.as_source().can_random_access()
    }

    fn max_window_bytes(&self) -> usize {
        self.as_source().max_window_bytes()
    }

    fn stats(&self) -> ReadStats {
        self.as_source().stats()
    }
}

/// 当前打开的书籍。
pub struct OpenBook {
    /// 书籍标识（本地存储主键）。
    pub book_id: i64,
    /// 数据源种类（`NR_SOURCE_KIND_*`）。
    pub kind: u32,
    /// 来源标识。
    pub identity: String,
    /// 坐标空间总字节数。
    pub total_bytes: u64,
    /// 数据源。
    pub source: SourceHandle,
    /// 章节索引。
    pub index: ChapterIndex,
    /// 内容索引版本（分页缓存键的一半）。
    pub content_index_version: u32,
    /// 识别出的编码。
    pub encoding: TextEncoding,
}

/// [`Engine::open_txt`] 的入参。
#[derive(Debug, Clone)]
pub struct OpenTxtSpec {
    /// 数据源种类。
    pub kind: u32,
    /// 平台数据源自述的总字节数（文件数据源忽略）。
    pub total_bytes: u64,
    /// 来源标识（路径 / URI）。
    pub identity: String,
    /// 展示名（可为空 ⇒ 由来源标识推导）。
    pub display_name: Option<String>,
}

/// [`Engine::open_txt`] 的出参。
#[derive(Debug, Clone)]
pub struct OpenOutcome {
    /// 书籍标识。
    pub book_id: u64,
    /// 坐标空间总字节数。
    pub total_bytes: u64,
    /// 内容索引版本。
    pub content_index_version: u64,
    /// 编码稳定数值。
    pub encoding: u32,
    /// 编码可信度千分比。
    pub encoding_confidence: u32,
    /// 编码识别评分。
    pub encoding_score: u32,
    /// 是否需要用户确认编码。
    pub needs_encoding_confirmation: bool,
    /// 章节数量。
    pub chapters: u32,
    /// 章节索引是否来自本地存储。
    pub index_from_cache: bool,
}

/// [`Engine::read_window`] 的出参。
#[derive(Debug, Clone)]
pub struct WindowOutcome {
    /// 窗口起始偏移。
    pub offset: u64,
    /// 窗口结束偏移（不含）。
    pub end_byte: u64,
    /// 引擎侧读取序号。
    pub request_id: u64,
    /// 总字节数。
    pub total_bytes: u64,
    /// 实际字节数。
    pub len: u32,
    /// 头部被裁掉的字节数。
    pub dropped_leading: u32,
    /// 尾部被裁掉的字节数。
    pub trimmed_tail: u32,
    /// 是否 EOF。
    pub eof: bool,
    /// UTF-8 窗口字节（Rust 拥有）。
    pub bytes: Vec<u8>,
}

/// [`Engine::begin_layout`] / [`Engine::commit_boundary`] / [`Engine::query_boundary`] 的出参。
#[derive(Debug, Clone, Default)]
pub struct BoundaryOutcome {
    /// 状态（`NR_BOUNDARY_*`）。
    pub status: u32,
    /// 拒绝原因（`NR_REJECT_*`）。
    pub rejection: u32,
    /// 跨 ABI 错误码。
    pub error_code: i32,
    /// 当前排版世代。
    pub layout_id: u64,
    /// 请求标识。
    pub request_id: u64,
    /// 被丢弃的排版世代。
    pub stale_layout_id: u64,
    /// 被丢弃的请求标识。
    pub stale_request_id: u64,
    /// 已登记边界起点。
    pub start_byte: u64,
    /// 已登记边界终点。
    pub end_byte: u64,
    /// 总字节数。
    pub total_bytes: u64,
    /// 当前世代边界数量。
    pub page_count: u64,
}

/// [`Engine::save_position`] 的入参。
#[derive(Debug, Clone)]
pub struct PositionSpec {
    /// 书籍标识。
    pub book_id: u64,
    /// 章节序号。
    pub chapter_index: u32,
    /// 原始字节偏移。
    pub original_byte_offset: u64,
    /// 附近文本指纹（`None` ⇒ 引擎自动采样）。
    pub nearby_fingerprint: Option<String>,
}

/// [`Engine::load_position`] 的出参。
#[derive(Debug, Clone, Default)]
pub struct PositionOutcome {
    /// 是否找到已保存位置。
    pub found: bool,
    /// 书籍标识。
    pub book_id: u64,
    /// 章节序号。
    pub chapter_index: u32,
    /// 原始字节偏移。
    pub original_byte_offset: u64,
    /// 附近文本指纹。
    pub nearby_fingerprint: Option<String>,
    /// 更新时间（Unix 秒）。
    pub updated_at: u64,
    /// 是否经过锚点恢复。
    pub restored: bool,
    /// 是否回退到章节开头。
    pub fallback_to_chapter_start: bool,
}

/// 章节批次数据（缓冲区字节布局：`nr_chapter[count]` + 尾随 UTF-8 标题）。
#[derive(Debug, Clone, Default)]
pub struct ChapterBatchData {
    /// 本次条目数。
    pub count: u32,
    /// 章节索引版本。
    pub index_version: u32,
    /// 该书章节总数。
    pub total_chapters: u32,
    /// 缓冲区字节。
    pub bytes: Vec<u8>,
}

/// [`Engine::set_encoding`] 的出参。
#[derive(Debug, Clone, Copy)]
pub struct SetEncodingOutcome {
    /// 生效的内容编码。
    pub encoding: TextEncoding,
    /// 按新编码重扫后的章节数量。
    pub chapters: u32,
    /// 重建后的内容索引版本（旧分页缓存据此失效）。
    pub content_index_version: u64,
    /// 1 = 本次就地重扫了新索引。
    pub index_rebuilt: bool,
}

/// `NR_ENCODING_*` 码 → [`TextEncoding`]（1..=7；0 由调用方解释为「清除覆盖」）。
fn text_encoding_from_code(code: u32) -> NrResult<TextEncoding> {
    let encoding = match code {
        1 => TextEncoding::Utf8,
        2 => TextEncoding::Utf8Bom,
        3 => TextEncoding::Utf16Le,
        4 => TextEncoding::Utf16Be,
        5 => TextEncoding::Gb18030,
        6 => TextEncoding::Gbk,
        7 => TextEncoding::Big5,
        other => {
            return Err(invalid(format!(
                "编码码 {other} 非法（0 = 自动，1..=7 见 NR_ENCODING_*）"
            )))
        }
    };
    Ok(encoding)
}

/// [`Engine::rebuild_index`] 的出参。
#[derive(Debug, Clone, Default)]
pub struct RebuildOutcome {
    /// 是否为部分结果。
    pub partial: bool,
    /// 章节数量。
    pub chapters: u32,
    /// 索引是否来自本地存储。
    pub index_from_cache: bool,
    /// 实际扫描字节数。
    pub scanned_bytes: u64,
    /// 总字节数。
    pub total_bytes: u64,
    /// 重建后的内容索引版本。
    pub content_index_version: u64,
}

/// 阅读引擎。
pub struct Engine {
    config: EngineConfig,
    storage: Storage,
    open_book: Option<OpenBook>,
    cache: Option<PaginationCache>,
    last_error: Option<NrError>,
    handle: u64,
    read_sequence: u64,
}

impl Engine {
    /// 建立引擎（打开/迁移本地存储）。
    pub fn new(config: EngineConfig) -> NrResult<Self> {
        if config.max_window_bytes == 0 || config.max_window_bytes > MAX_WINDOW_BYTES {
            return Err(invalid(format!(
                "max_window_bytes 必须在 1..={MAX_WINDOW_BYTES}，实际 {}",
                config.max_window_bytes
            )));
        }
        if let Some(provider) = config.provider {
            crate::external::ensure_provider_available(&provider)?;
        }
        let mut storage = match &config.db_path {
            Some(path) => Storage::open(path).map_err(map_storage_error)?,
            None => Storage::open_in_memory().map_err(map_storage_error)?,
        };
        storage.migrate().map_err(map_storage_error)?;
        Ok(Self {
            config,
            storage,
            open_book: None,
            cache: None,
            last_error: None,
            handle: 0,
            read_sequence: 0,
        })
    }

    /// 绑定引擎句柄（由 [`crate::handle::create`] 在登记后设置）。
    pub fn set_handle(&mut self, handle: u64) {
        self.handle = handle;
    }

    /// 引擎句柄。
    pub fn handle(&self) -> u64 {
        self.handle
    }

    /// 配置。
    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// 本地存储（测试与审计用）。
    pub fn storage(&self) -> &Storage {
        &self.storage
    }

    /// 当前打开的书籍。
    pub fn open_book(&self) -> Option<&OpenBook> {
        self.open_book.as_ref()
    }

    /// 当前分页缓存。
    pub fn cache(&self) -> Option<&PaginationCache> {
        self.cache.as_ref()
    }

    /// 记录最近一次错误（供 `nr_get_last_error`）。
    pub fn record_error(&mut self, error: NrError) {
        self.last_error = Some(error);
    }

    /// 最近一次错误。
    pub fn last_error(&self) -> Option<&NrError> {
        self.last_error.as_ref()
    }

    /// 打开 TXT（导入 upsert + 编码识别 + 章节索引）。
    pub fn open_txt(&mut self, spec: &OpenTxtSpec) -> NrResult<OpenOutcome> {
        if spec.identity.is_empty() {
            return Err(invalid("来源标识（路径 / URI）不得为空"));
        }
        let max_window = self.config.max_window_bytes;
        let (source, source_kind, total_bytes, derived_name) = match spec.kind {
            SOURCE_KIND_FILE => {
                let file = FileTextSource::open(&spec.identity).map_err(map_source_error)?;
                let metadata = file.metadata();
                let name = metadata
                    .display_name
                    .clone()
                    .unwrap_or_else(|| "book.txt".to_string());
                (
                    SourceHandle::File(file),
                    books::KIND_DESKTOP_PATH,
                    metadata.size,
                    name,
                )
            }
            SOURCE_KIND_PLATFORM => {
                if spec.total_bytes > i64::MAX as u64 {
                    return Err(invalid("平台数据源长度超出 64 位有符号范围"));
                }
                let provider = self
                    .config
                    .provider
                    .ok_or_else(|| invalid("平台数据源必须先在 nr_engine_config 中提供窗口回调"))?;
                crate::external::ensure_provider_available(&provider)?;
                let name = spec
                    .display_name
                    .clone()
                    .unwrap_or_else(|| "platform-book.txt".to_string());
                let source = PlatformWindowSource::new(
                    provider,
                    spec.total_bytes,
                    spec.identity.clone(),
                    Some(name.clone()),
                    max_window,
                );
                (
                    SourceHandle::Platform(source),
                    books::KIND_ANDROID_URI,
                    spec.total_bytes,
                    name,
                )
            }
            other => {
                return Err(invalid(format!(
                "未登记的数据源种类: {other}（期望 {SOURCE_KIND_FILE} 或 {SOURCE_KIND_PLATFORM}）"
            )))
            }
        };

        // 编码识别：只在有限窗口上采样（头部 32 KiB + 中部/尾部各 16 KiB）。
        let detection = detect_encoding(source.as_source()).map_err(map_source_error)?;
        // 指纹：大小 + 头/中/尾各 4 KiB 抽样哈希。
        let fingerprint = source.fingerprint().map_err(map_source_error)?;

        let display_name = spec
            .display_name
            .clone()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(derived_name);
        let display_name = truncate_utf8(&display_name, MAX_NAME_BYTES);
        let import = books::BookImport {
            display_name: display_name.clone(),
            original_name: display_name.clone(),
            source_kind: source_kind.to_string(),
            source_locator: spec.identity.clone(),
            source_size: total_bytes as i64,
            source_modified_hint: None,
            source_fingerprint: Some(compact_fingerprint(&fingerprint)),
            encoding: SourceHandle::encoding_name(detection.encoding).to_string(),
            encoding_confidence: confidence_permille(detection.confidence) as i32,
            encoding_user_override: false,
        };
        let now = connection::now_unix();
        let import_outcome =
            books::import_book(self.storage.conn(), &import, now).map_err(map_storage_error)?;
        let book_id = import_outcome.book_id;

        let (index, index_from_cache) =
            self.chapter_index_for(book_id, total_bytes, &source, detection.encoding)?;

        let content_index_version = meta::content_index_version(self.storage.conn())
            .map_err(map_storage_error)?
            .clamp(0, u32::MAX as i64) as u32;

        let outcome = OpenOutcome {
            book_id: book_id as u64,
            total_bytes,
            content_index_version: u64::from(content_index_version),
            encoding: crate::abi::EncodingCode::from_encoding(detection.encoding).as_u32(),
            encoding_confidence: confidence_permille(detection.confidence),
            encoding_score: detection.score,
            needs_encoding_confirmation: detection.needs_confirmation(),
            chapters: index.len() as u32,
            index_from_cache,
        };
        self.open_book = Some(OpenBook {
            book_id,
            kind: spec.kind,
            identity: spec.identity.clone(),
            total_bytes,
            source,
            index,
            content_index_version,
            encoding: detection.encoding,
        });
        // 换书 ⇒ 旧排版缓存不再适用。
        self.cache = None;
        Ok(outcome)
    }

    /// 取章节索引：优先本地存储缓存（校验覆盖率与版本），否则可取消地扫描并落库。
    fn chapter_index_for(
        &self,
        book_id: i64,
        total_bytes: u64,
        source: &SourceHandle,
        encoding: TextEncoding,
    ) -> NrResult<(ChapterIndex, bool)> {
        // 扫描器版本一致才允许复用缓存索引：规则/解码行为变更后必须重扫，
        // 否则「更新应用」对既有书籍无效（用户须卸载重装才能生效）。
        let stored_scanner = meta::chapter_scanner_version(self.storage.conn())
            .map_err(map_storage_error)?;
        let scanner_current = stored_scanner == reading_index::CHAPTER_SCANNER_VERSION;
        let stored = storage_index::list_chapters(self.storage.conn(), book_id)
            .map_err(map_storage_error)?;
        if scanner_current {
            if let Some(index) = index_from_records(&stored, total_bytes) {
                return Ok((index, true));
            }
        }
        let (index, _stats) = reading_core::scan_chapters_from_source_with_encoding(
            source,
            &|| false,
            encoding,
        )?;
        if !scanner_current {
            // 重扫后：记录版本 + 递增内容索引版本（章节区间可能变化，旧布局/分页缓存必须失效）。
            let now = connection::now_unix();
            meta::set_chapter_scanner_version(
                self.storage.conn(),
                reading_index::CHAPTER_SCANNER_VERSION,
                now,
            )
            .map_err(map_storage_error)?;
            meta::bump_content_index_version(self.storage.conn(), now)
                .map_err(map_storage_error)?;
        }
        let inputs: Vec<storage_index::ChapterInput> = index
            .chapters
            .iter()
            .map(|chapter| storage_index::ChapterInput {
                sequence: i64::from(chapter.index),
                title: chapter.title.clone(),
                start_byte: chapter.start_byte as i64,
                end_byte: chapter.end_byte as i64,
                rule_id: chapter.rule.id().to_string(),
            })
            .collect();
        storage_index::rebuild_chapters(
            self.storage.conn(),
            book_id,
            i64::from(index.index_version),
            &inputs,
            connection::now_unix(),
        )
        .map_err(map_storage_error)?;
        Ok((index, false))
    }

    /// 关闭书籍。
    pub fn close_book(&mut self, book_id: u64) -> NrResult<()> {
        let open = self.open_book.as_ref().ok_or_else(no_open_book)?;
        if open.book_id as u64 != book_id {
            return Err(book_not_found(book_id));
        }
        self.open_book = None;
        self.cache = None;
        Ok(())
    }

    /// 读取窗口（受限、UTF-8 对齐、可证明未整本读取）。
    pub fn read_window(
        &mut self,
        chapter_index: u32,
        offset: u64,
        length: u64,
    ) -> NrResult<WindowOutcome> {
        if length == 0 {
            return Err(invalid("读取长度不得为 0"));
        }
        if length > self.config.max_window_bytes as u64 {
            return Err(invalid(format!(
                "读取长度 {length} 超过窗口上限 {}：拒绝且不读取任何字节",
                self.config.max_window_bytes
            )));
        }
        let open = self.open_book.as_ref().ok_or_else(no_open_book)?;
        let chapter = open
            .index
            .chapter(chapter_index)
            .ok_or_else(|| chapter_out_of_range(format!("章节 {chapter_index} 不存在")))?;
        let inside = chapter.contains(offset)
            || (offset == open.total_bytes && chapter.end_byte == open.total_bytes);
        if !inside {
            return Err(chapter_out_of_range(format!(
                "偏移 {offset} 不在章节 {chapter_index} 的区间 [{}, {}) 内",
                chapter.start_byte, chapter.end_byte
            )));
        }
        if offset > open.total_bytes {
            return Err(out_of_range(format!(
                "偏移 {offset} 超出内容长度 {}",
                open.total_bytes
            )));
        }
        let window = open
            .source
            .read_at_with_encoding(offset, length as usize, open.encoding)
            .map_err(map_source_error)?;
        self.read_sequence += 1;
        let len = window.data.len() as u32;
        Ok(WindowOutcome {
            offset: window.offset,
            end_byte: window.offset + u64::from(len),
            request_id: self.read_sequence,
            total_bytes: open.total_bytes,
            len,
            dropped_leading: window.dropped_leading as u32,
            trimmed_tail: window.trimmed_tail as u32,
            eof: window.eof,
            bytes: window.data,
        })
    }

    /// 登记新排版（`BEGIN_LAYOUT`）。
    pub fn begin_layout(
        &mut self,
        signature: LayoutSignature,
        content_index_version: u64,
    ) -> NrResult<BoundaryOutcome> {
        let open = self.open_book.as_ref().ok_or_else(no_open_book)?;
        if content_index_version > u64::from(u32::MAX) {
            return Err(invalid("content_index_version 超出 32 位范围"));
        }
        let civ = content_index_version as u32;
        if civ != open.content_index_version {
            // 调用方持有过期的内容索引版本：先重新打开书籍再排版。
            return Ok(BoundaryOutcome {
                status: BoundaryCommitStatus::StaleContentIndexVersion as u32,
                error_code: ErrorCode::LayoutNotFound.as_c(),
                layout_id: self.cache.as_ref().map_or(0, PaginationCache::layout_id),
                total_bytes: open.total_bytes,
                page_count: self.cache.as_ref().map_or(0, |cache| cache.len() as u64),
                ..BoundaryOutcome::default()
            });
        }
        let total_bytes = open.total_bytes;
        if self.cache.is_none() {
            // 首个排版世代：直接以当前签名建键（`set_layout` 随即返回 changed = false）。
            self.cache = Some(PaginationCache::new(signature.clone(), civ)?);
        }
        let previous_signature = self.cache.as_ref().map(|cache| cache.signature().clone());
        let cache = self.cache.as_mut().expect("上一步保证存在");
        let change = cache.set_layout(signature, civ)?;
        let request_id = cache.begin_request();
        if change.changed {
            // 排版口径变化 ⇒ 旧签名的落库分页记录整体失效（可重建数据，不影响书籍/进度/书签）。
            if let (Some(book), Some(previous)) =
                (self.open_book.as_ref(), previous_signature.as_ref())
            {
                storage_index::drop_layout_records_for_signature(
                    self.storage.conn(),
                    book.book_id,
                    &previous.encode(),
                )
                .map_err(map_storage_error)?;
            }
        }
        Ok(BoundaryOutcome {
            status: BOUNDARY_LAYOUT_READY,
            rejection: 0,
            error_code: 0,
            layout_id: cache.layout_id(),
            request_id,
            stale_layout_id: if change.changed {
                change.previous_layout_id
            } else {
                0
            },
            stale_request_id: 0,
            start_byte: 0,
            end_byte: 0,
            total_bytes,
            page_count: cache.len() as u64,
        })
    }

    /// 提交页边界（`COMMIT`）。
    pub fn commit_boundary(
        &mut self,
        content_index_version: u64,
        layout_id: u64,
        request_id: u64,
        total_bytes: u64,
        start_byte: u64,
        end_byte: u64,
    ) -> NrResult<BoundaryOutcome> {
        if content_index_version > u64::from(u32::MAX) {
            return Err(invalid("content_index_version 超出 32 位范围"));
        }
        let open = self.open_book.as_ref().ok_or_else(no_open_book)?;
        let total = open.total_bytes;
        let cache = self
            .cache
            .as_mut()
            .ok_or_else(|| layout_not_found(layout_id))?;
        let request = PageBoundaryRequest {
            layout_id,
            request_id,
            content_index_version: content_index_version as u32,
            total_bytes,
            start_byte,
            end_byte,
        };
        // 按**内容实际编码**校验边界（CR4：GBK/GB18030 等文档的合法边界不再是 UTF-8 专属）。
        let outcome = cache.commit_page_boundary(&open.source, &request, open.encoding);
        if outcome.error_code == 0 {
            // 落库：分页进度只保存"可重建"的统计（签名 + 索引版本 + 已生成到哪 + 页数）。
            let record = storage_index::LayoutRecordInput {
                book_id: open.book_id,
                layout_signature: cache.signature().encode(),
                content_index_version: i64::from(cache.layout_key().content_index_version),
                generated_until_byte: end_byte as i64,
                page_count: cache.len() as i64,
                status: storage_index::LAYOUT_PARTIAL.to_string(),
            };
            storage_index::upsert_layout_record(
                self.storage.conn(),
                &record,
                connection::now_unix(),
            )
            .map_err(map_storage_error)?;
        }
        let mut abi_outcome = boundary_from_commit(&outcome, total);
        abi_outcome.page_count = cache.len() as u64;
        Ok(abi_outcome)
    }

    /// 查询页边界。
    pub fn query_boundary(
        &mut self,
        layout_id: u64,
        chapter_index: u32,
        start_byte: u64,
    ) -> NrResult<BoundaryOutcome> {
        let open = self.open_book.as_ref().ok_or_else(no_open_book)?;
        let total_bytes = open.total_bytes;
        let Some(cache) = self.cache.as_ref() else {
            return Ok(BoundaryOutcome {
                status: BOUNDARY_NOT_FOUND,
                error_code: ErrorCode::LayoutNotFound.as_c(),
                layout_id: 0,
                start_byte,
                total_bytes,
                ..BoundaryOutcome::default()
            });
        };
        if layout_id != cache.layout_id() {
            return Ok(BoundaryOutcome {
                status: BoundaryCommitStatus::StaleLayoutId as u32,
                error_code: ErrorCode::LayoutNotFound.as_c(),
                layout_id: cache.layout_id(),
                stale_layout_id: layout_id,
                start_byte,
                total_bytes,
                page_count: cache.len() as u64,
                ..BoundaryOutcome::default()
            });
        }
        let derived = chapter_index_for_offset(&open.index, start_byte);
        if derived != chapter_index {
            return Err(chapter_out_of_range(format!(
                "偏移 {start_byte} 属于章节 {derived}，与请求的章节 {chapter_index} 不一致"
            )));
        }
        match cache.query_page_boundary(layout_id, start_byte) {
            Some(boundary) => Ok(BoundaryOutcome {
                status: BoundaryCommitStatus::Committed as u32,
                error_code: 0,
                layout_id,
                request_id: 0,
                stale_layout_id: 0,
                stale_request_id: 0,
                start_byte: boundary.start_byte,
                end_byte: boundary.end_byte,
                total_bytes,
                page_count: cache.len() as u64,
                rejection: 0,
            }),
            None => Ok(BoundaryOutcome {
                status: BOUNDARY_NOT_FOUND,
                error_code: ErrorCode::LayoutNotFound.as_c(),
                layout_id,
                start_byte,
                total_bytes,
                page_count: cache.len() as u64,
                ..BoundaryOutcome::default()
            }),
        }
    }

    /// 保存阅读位置（幂等；坐标与章节由引擎裁定）。
    pub fn save_position(&mut self, spec: &PositionSpec) -> NrResult<bool> {
        let now = connection::now_unix();
        let open = self.open_book.as_ref().ok_or_else(no_open_book)?;
        if open.book_id as u64 != spec.book_id {
            return Err(book_not_found(spec.book_id));
        }
        if spec.original_byte_offset > open.total_bytes {
            return Err(out_of_range(format!(
                "位置偏移 {} 超出内容长度 {}",
                spec.original_byte_offset, open.total_bytes
            )));
        }
        let derived = chapter_index_for_offset(&open.index, spec.original_byte_offset);
        if derived != spec.chapter_index {
            return Err(chapter_out_of_range(format!(
                "偏移 {} 属于章节 {derived}，与请求的章节 {} 不一致",
                spec.original_byte_offset, spec.chapter_index
            )));
        }
        let fingerprint = match &spec.nearby_fingerprint {
            Some(quote) => quote.clone(),
            None => {
                let sample = generate_anchor(&open.source, spec.original_byte_offset)?;
                sample.fingerprint.quote
            }
        };
        let fingerprint = truncate_utf8(&fingerprint, MAX_FINGERPRINT_BYTES);
        let input = progress::ProgressInput {
            book_id: open.book_id,
            content_unit_id: i64::from(CONTENT_UNIT_SINGLE),
            chapter_index: i64::from(spec.chapter_index),
            byte_offset: spec.original_byte_offset as i64,
            nearby_fingerprint: Some(fingerprint),
            last_page_hint: None,
            index_version: i64::from(open.content_index_version),
        };
        let outcome =
            progress::save_progress(self.storage.conn(), &input, now).map_err(map_storage_error)?;
        progress::touch_last_opened(self.storage.conn(), open.book_id, now)
            .map_err(map_storage_error)?;
        Ok(outcome.changed)
    }

    /// 读取阅读位置（打开同一本书时用锚点在有界半径内恢复）。
    pub fn load_position(&mut self, book_id: u64) -> NrResult<PositionOutcome> {
        if book_id > i64::MAX as u64 {
            return Err(invalid("book_id 超出 64 位有符号范围"));
        }
        let id = book_id as i64;
        let book = books::get_book(self.storage.conn(), id).map_err(map_storage_error)?;
        if book.is_none() {
            return Err(book_not_found(book_id));
        }
        let Some(stored) =
            progress::load_progress(self.storage.conn(), id).map_err(map_storage_error)?
        else {
            return Ok(PositionOutcome {
                found: false,
                book_id,
                ..PositionOutcome::default()
            });
        };
        let stored_position = ContentPosition::new(
            book_id,
            stored.content_unit_id.max(0) as u64,
            stored.chapter_index.max(0) as u32,
            stored.byte_offset.max(0) as u64,
            reading_core::NearbyFingerprint::from_str_prefix(
                stored.nearby_fingerprint.as_deref().unwrap_or_default(),
            ),
        );
        let mut outcome = PositionOutcome {
            found: true,
            book_id,
            chapter_index: stored_position.chapter_index,
            original_byte_offset: stored_position.original_byte_offset,
            nearby_fingerprint: stored.nearby_fingerprint.clone(),
            updated_at: stored.updated_at.max(0) as u64,
            restored: false,
            fallback_to_chapter_start: false,
        };
        let is_open = self
            .open_book
            .as_ref()
            .is_some_and(|open| open.book_id == id);
        if is_open {
            let open = self.open_book.as_ref().ok_or_else(no_open_book)?;
            let search = AnchorSearch::new(DEFAULT_ANCHOR_RADIUS_BYTES);
            let restored = restore_position(&open.source, &open.index, &stored_position, &search)?;
            outcome.restored = true;
            outcome.fallback_to_chapter_start = restored.fallback_to_chapter_start;
            outcome.chapter_index = restored.position.chapter_index;
            outcome.original_byte_offset = restored.position.original_byte_offset;
            outcome.nearby_fingerprint = Some(restored.position.nearby_fingerprint.quote.clone());
        }
        Ok(outcome)
    }

    /// 章节批次（有界；缓冲区为 `nr_chapter[count]` + 尾随 UTF-8 标题）。
    pub fn chapter_batch(
        &mut self,
        book_id: u64,
        start_sequence: u32,
        max_count: u32,
    ) -> NrResult<ChapterBatchData> {
        if max_count == 0 || max_count > MAX_CHAPTER_BATCH {
            return Err(invalid(format!(
                "max_count 必须在 1..={MAX_CHAPTER_BATCH}，实际 {max_count}（禁止一次取走整库章节）"
            )));
        }
        if book_id > i64::MAX as u64 {
            return Err(invalid("book_id 超出 64 位有符号范围"));
        }
        let id = book_id as i64;
        if books::get_book(self.storage.conn(), id)
            .map_err(map_storage_error)?
            .is_none()
        {
            return Err(book_not_found(book_id));
        }
        let records =
            storage_index::list_chapters(self.storage.conn(), id).map_err(map_storage_error)?;
        let total_chapters = records.len() as u32;
        let start = start_sequence.min(total_chapters) as usize;
        let end = (start + max_count as usize).min(records.len());
        let slice = &records[start..end];
        let entry_size = std::mem::size_of::<NrChapter>();
        let mut bytes: Vec<u8> = Vec::with_capacity(slice.len() * entry_size + 64);
        let mut titles: Vec<u8> = Vec::new();
        // 定长数组在缓冲区头部，标题字节紧随其后：一次分配、单一所有者（Rust 归还）。
        let mut title_cursor = (slice.len() * entry_size) as u32;
        for record in slice {
            let title = truncate_utf8(&record.title, MAX_TITLE_BYTES);
            let entry = NrChapter {
                sequence: record.sequence.max(0) as u32,
                rule_id: record
                    .rule_id
                    .parse::<u32>()
                    .ok()
                    .filter(|id| ChapterRule::from_id(*id).is_some())
                    .unwrap_or(ChapterRule::VirtualFullText.id()),
                start_byte: record.start_byte.max(0) as u64,
                end_byte: record.end_byte.max(0) as u64,
                title_offset: title_cursor,
                title_len: title.len() as u32,
                index_version: record.index_version.max(0) as u32,
                reserved0: 0,
            };
            titles.extend_from_slice(title.as_bytes());
            bytes.extend_from_slice(&struct_bytes(&entry));
            title_cursor += title.len() as u32;
        }
        bytes.extend_from_slice(&titles);
        let index_version = slice
            .first()
            .map(|record| record.index_version.max(0) as u32)
            .unwrap_or(reading_index::CHAPTER_INDEX_VERSION);
        Ok(ChapterBatchData {
            count: slice.len() as u32,
            index_version,
            total_chapters,
            bytes,
        })
    }

    /// 新增书签（坐标与章节由引擎裁定；名称为空时使用章节标题）。
    pub fn create_bookmark(
        &mut self,
        book_id: u64,
        chapter_index: u32,
        byte_offset: u64,
        title: Option<&str>,
    ) -> NrResult<u64> {
        let now = connection::now_unix();
        let open = self.open_book.as_ref().ok_or_else(no_open_book)?;
        if open.book_id as u64 != book_id {
            return Err(book_not_found(book_id));
        }
        if byte_offset > open.total_bytes {
            return Err(out_of_range(format!(
                "书签偏移 {byte_offset} 超出内容长度 {}",
                open.total_bytes
            )));
        }
        let derived = chapter_index_for_offset(&open.index, byte_offset);
        if derived != chapter_index {
            return Err(chapter_out_of_range(format!(
                "偏移 {byte_offset} 属于章节 {derived}，与请求的章节 {chapter_index} 不一致"
            )));
        }
        let chapter_title = open
            .index
            .chapter(chapter_index)
            .map(|chapter| chapter.title.clone())
            .unwrap_or_else(|| VIRTUAL_CHAPTER_TITLE.to_string());
        let title = match title {
            Some(title) if !title.trim().is_empty() => truncate_utf8(title, MAX_TITLE_BYTES),
            _ => truncate_utf8(&chapter_title, MAX_TITLE_BYTES),
        };
        let fingerprint = generate_anchor(&open.source, byte_offset)?
            .fingerprint
            .quote;
        let input = bookmarks::BookmarkInput {
            book_id: open.book_id,
            content_unit_id: i64::from(CONTENT_UNIT_SINGLE),
            chapter_index: i64::from(chapter_index),
            byte_offset: byte_offset as i64,
            nearby_fingerprint: Some(truncate_utf8(&fingerprint, MAX_FINGERPRINT_BYTES)),
            title,
        };
        let id = bookmarks::create_bookmark(self.storage.conn(), &input, now)
            .map_err(map_storage_error)?;
        Ok(id as u64)
    }

    /// 删除书签（返回是否确有删除）。
    pub fn delete_bookmark(&mut self, bookmark_id: u64) -> NrResult<bool> {
        if bookmark_id > i64::MAX as u64 {
            return Err(invalid("bookmark_id 超出 64 位有符号范围"));
        }
        bookmarks::delete_bookmark(self.storage.conn(), bookmark_id as i64)
            .map_err(map_storage_error)
    }

    /// 失效当前排版世代（同时清理落库的旧签名分页记录）。
    pub fn invalidate_layout(&mut self, layout_id: u64) -> NrResult<u64> {
        let open = self.open_book.as_ref().ok_or_else(no_open_book)?;
        let book_id = open.book_id;
        let cache = self
            .cache
            .as_mut()
            .ok_or_else(|| layout_not_found(layout_id))?;
        if layout_id != cache.layout_id() {
            return Err(layout_not_found(layout_id));
        }
        let signature = cache.signature().clone();
        let invalidation = cache.invalidate_all();
        storage_index::drop_layout_records_for_signature(
            self.storage.conn(),
            book_id,
            &signature.encode(),
        )
        .map_err(map_storage_error)?;
        Ok(invalidation.removed as u64)
    }

    /// 重建章节索引（可取消；取消时不落库、不替换内存索引）。
    /// 手动指定书籍内容编码（规格 CR2「用户手动覆盖」）。
    ///
    /// # 语义（与 `reading-source::EncodingState::override_encoding` 同一口径）
    ///
    /// - `encoding_code` 取 `NR_ENCODING_*`（1..=7）；**`0` = 清除覆盖**，回到自动检测结果；
    /// - 只改变「如何解释文件字节」，**永不写回原始文件**；
    /// - 覆盖后**立即**按新编码重扫章节索引并递增内容索引版本 ⇒ 旧章节缓存与分页边界全部失效
    ///   （否则会出现「改了编码但正文没变」）；
    /// - 书籍必须正是**当前打开**的那一本（与 `nr_rebuild_index` 同一约定）。
    pub fn set_encoding(
        &mut self,
        book_id: u64,
        encoding_code: u32,
    ) -> NrResult<SetEncodingOutcome> {
        let (book_id_i64, effective) = {
            let open = self.open_book.as_ref().ok_or_else(no_open_book)?;
            if open.book_id as u64 != book_id {
                return Err(book_not_found(book_id));
            }
            let effective = if encoding_code == 0 {
                // 清除覆盖：重新自动检测（不读整本，只抽样）。
                let detection = reading_source::detect_encoding(open.source.as_source())
                    .map_err(map_source_error)?;
                detection.encoding
            } else {
                text_encoding_from_code(encoding_code)?
            };
            (open.book_id, effective)
        };

        // 持久化「用户指定」标记（界面据此区分「识别」与「手动」）。
        let now = connection::now_unix();
        books::set_encoding(
            self.storage.conn(),
            book_id_i64,
            SourceHandle::encoding_name(effective),
            USER_OVERRIDE_CONFIDENCE_PERMILLE,
            now,
        )
        .map_err(map_storage_error)?;

        // 先让引擎采用新编码，再重扫（rebuild 内部会递增版本、重写章节、失效旧布局）。
        if let Some(open) = self.open_book.as_mut() {
            open.encoding = effective;
        }
        let rebuilt = self.rebuild_index(book_id, 0)?;
        Ok(SetEncodingOutcome {
            encoding: effective,
            chapters: rebuilt.chapters,
            content_index_version: rebuilt.content_index_version,
            index_rebuilt: !rebuilt.index_from_cache,
        })
    }

    pub fn rebuild_index(&mut self, book_id: u64, request_id: u64) -> NrResult<RebuildOutcome> {
        let handle = self.handle;
        let open = self.open_book.as_ref().ok_or_else(no_open_book)?;
        if open.book_id as u64 != book_id {
            return Err(book_not_found(book_id));
        }
        let total_bytes = open.total_bytes;
        if cancel::is_cancelled(handle, request_id) {
            // 任务开始前已被取消：直接返回部分结果。
            cancel::clear(handle, request_id);
            return Ok(RebuildOutcome {
                partial: true,
                chapters: open.index.len() as u32,
                index_from_cache: false,
                scanned_bytes: 0,
                total_bytes,
                content_index_version: u64::from(open.content_index_version),
            });
        }
        let cancelled_flag = std::cell::Cell::new(false);
        let (index, _stats) = {
            let cancel_probe = || {
                let cancelled = cancel::is_cancelled(handle, request_id);
                if cancelled {
                    cancelled_flag.set(true);
                }
                cancelled
            };
            let open_ref = self.open_book.as_ref().ok_or_else(no_open_book)?;
            let result = reading_core::scan_chapters_from_source_with_encoding(
                &open_ref.source,
                &cancel_probe,
                open_ref.encoding,
            );
            cancel::clear(handle, request_id);
            result?
        };
        let partial = cancelled_flag.get() || index.cancelled;
        if partial {
            // 取消：只回报部分诊断数据，不落库、不替换正式索引。
            return Ok(RebuildOutcome {
                partial: true,
                chapters: index.len() as u32,
                index_from_cache: false,
                scanned_bytes: index.scanned_bytes,
                total_bytes,
                content_index_version: u64::from(
                    self.open_book
                        .as_ref()
                        .map(|open| open.content_index_version)
                        .unwrap_or(0),
                ),
            });
        }
        let inputs: Vec<storage_index::ChapterInput> = index
            .chapters
            .iter()
            .map(|chapter| storage_index::ChapterInput {
                sequence: i64::from(chapter.index),
                title: chapter.title.clone(),
                start_byte: chapter.start_byte as i64,
                end_byte: chapter.end_byte as i64,
                rule_id: chapter.rule.id().to_string(),
            })
            .collect();
        let now = connection::now_unix();
        let new_version = meta::bump_content_index_version(self.storage.conn(), now)
            .map_err(map_storage_error)?;
        storage_index::rebuild_chapters(
            self.storage.conn(),
            book_id as i64,
            new_version,
            &inputs,
            now,
        )
        .map_err(map_storage_error)?;
        storage_index::drop_stale_layout_records(self.storage.conn(), book_id as i64, new_version)
            .map_err(map_storage_error)?;
        let chapters = index.len() as u32;
        let scanned_bytes = index.scanned_bytes;
        let new_version_u32 = new_version.clamp(0, u32::MAX as i64) as u32;
        if let Some(open) = self.open_book.as_mut() {
            open.index = index;
            open.content_index_version = new_version_u32;
        }
        // 内容索引版本变化 ⇒ 旧分页缓存整体失效（下次 BEGIN_LAYOUT 会重建）。
        self.cache = None;
        Ok(RebuildOutcome {
            partial: false,
            chapters,
            index_from_cache: false,
            scanned_bytes,
            total_bytes,
            content_index_version: new_version_u32 as u64,
        })
    }

    /// 引擎自检摘要（测试与证据文档用）。
    pub fn describe_state(&self) -> String {
        let open = match &self.open_book {
            Some(open) => format!(
                "book={} bytes={} chapters={} encoding={} platform={}",
                open.book_id,
                open.total_bytes,
                open.index.len(),
                open.encoding.as_str(),
                open.source.is_platform()
            ),
            None => "book=none".to_string(),
        };
        format!(
            "abi={} window_cap={} {open}",
            ABI_VERSION, self.config.max_window_bytes
        )
    }
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("handle", &self.handle)
            .field("state", &self.describe_state())
            .field(
                "last_error",
                &self.last_error.as_ref().map(|e| e.code().as_str()),
            )
            .finish()
    }
}

/// `BoundaryCommitOutcome` → ABI 出参。
fn boundary_from_commit(outcome: &BoundaryCommitOutcome, total_bytes: u64) -> BoundaryOutcome {
    BoundaryOutcome {
        status: outcome.status as u32,
        rejection: outcome.rejection.map(rejection_id).unwrap_or(0),
        error_code: outcome.error_code,
        layout_id: outcome.current_layout_id,
        request_id: 0,
        stale_layout_id: outcome.stale_layout_id.unwrap_or(0),
        stale_request_id: outcome.stale_request_id.unwrap_or(0),
        start_byte: outcome.boundary.map_or(0, |boundary| boundary.start_byte),
        end_byte: outcome.boundary.map_or(0, |boundary| boundary.end_byte),
        total_bytes,
        page_count: 0,
    }
}

/// 本地存储错误 → 统一错误码（数值同 `NR_ERR_*`，不重新定义）。
pub fn map_storage_error(error: StorageError) -> NrError {
    crate::abi::error_from_code(error.code(), format!("本地存储错误: {error}"))
}

/// 结构体字节视图（全部字段为整数/标量，无指针 ⇒ 可安全按字节读取）。
fn struct_bytes<T>(value: &T) -> Vec<u8> {
    let bytes = unsafe {
        std::slice::from_raw_parts((value as *const T).cast::<u8>(), std::mem::size_of::<T>())
    };
    bytes.to_vec()
}

/// 由存储记录重建章节索引；覆盖率或版本不满足时返回 `None`（触发重扫）。
fn index_from_records(
    records: &[storage_index::ChapterRecord],
    total_bytes: u64,
) -> Option<ChapterIndex> {
    if records.is_empty() {
        return None;
    }
    let mut chapters = Vec::with_capacity(records.len());
    for (position, record) in records.iter().enumerate() {
        if record.sequence.max(0) as usize != position {
            return None;
        }
        let start_byte = record.start_byte.max(0) as u64;
        let end_byte = record.end_byte.max(0) as u64;
        if end_byte < start_byte || end_byte > total_bytes {
            return None;
        }
        let rule = record
            .rule_id
            .parse::<u32>()
            .ok()
            .and_then(ChapterRule::from_id)
            .unwrap_or(ChapterRule::VirtualFullText);
        chapters.push(reading_index::ChapterRange {
            index: position as u32,
            title: record.title.clone(),
            normalized_title: reading_index::normalize_title(&record.title),
            start_byte,
            end_byte,
            rule,
            number: None,
        });
    }
    if chapters.first().map(|chapter| chapter.start_byte) != Some(0) {
        return None;
    }
    if chapters.last().map(|chapter| chapter.end_byte) != Some(total_bytes) {
        return None;
    }
    let is_virtual = chapters.len() == 1
        && chapters
            .first()
            .is_some_and(|chapter| chapter.rule == ChapterRule::VirtualFullText);
    Some(ChapterIndex {
        chapters,
        is_virtual,
        index_version: reading_index::CHAPTER_INDEX_VERSION,
        total_bytes,
        scanned_bytes: total_bytes,
        lines_scanned: 0,
        undecodable_lines: 0,
        truncated_lines: 0,
        numeric_candidates: 0,
        cancelled: false,
    })
}

/// 紧凑指纹文本（≤ 96 字节上限）：算法 + 大小 + 三段抽样哈希。
pub fn compact_fingerprint(fingerprint: &Fingerprint) -> String {
    format!(
        "{}:{}:{:016x}:{:016x}:{:016x}",
        fingerprint.algo,
        fingerprint.size,
        fingerprint.head_hash,
        fingerprint.mid_hash,
        fingerprint.tail_hash
    )
}

/// 按字节上限截断 UTF-8 文本，且绝不切断字符。
pub fn truncate_utf8(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_rejects_invalid_window_cap() {
        let mut config = EngineConfig::in_memory();
        config.max_window_bytes = MAX_WINDOW_BYTES + 1;
        let error = Engine::new(config).expect_err("超过硬上限必须拒绝");
        assert_eq!(error.code(), ErrorCode::InvalidArgument);
        let mut config = EngineConfig::in_memory();
        config.max_window_bytes = 0;
        assert!(Engine::new(config).is_err());
    }

    #[test]
    fn compact_fingerprint_fits_storage_limit() {
        let fingerprint = Fingerprint {
            size: 1_234_567_890,
            head_hash: 0x0123_4567_89ab_cdef,
            mid_hash: 0x0fed_cba9_8765_4321,
            tail_hash: 0x1111_2222_3333_4444,
            sample_len: 4096,
            sampled_bytes: 12288,
            modified: None,
            algo: reading_source::FINGERPRINT_ALGO,
        };
        let text = compact_fingerprint(&fingerprint);
        assert!(text.len() <= MAX_FINGERPRINT_BYTES, "{text}");
        assert!(text.starts_with("fnv1a64:"));
    }

    #[test]
    fn truncate_utf8_never_splits_a_character() {
        let text = "中文abc";
        assert_eq!(truncate_utf8(text, 100), text);
        let truncated = truncate_utf8(text, 4);
        assert_eq!(truncated, "中", "必须回退到字符边界");
        assert!(text.starts_with(&truncated));
    }

    #[test]
    fn index_from_records_rejects_partial_coverage() {
        let record = |sequence: i64, start: i64, end: i64| storage_index::ChapterRecord {
            id: sequence + 1,
            book_id: 1,
            sequence,
            title: format!("第{}章", sequence + 1),
            start_byte: start,
            end_byte: end,
            rule_id: ChapterRule::ChapterNumber.id().to_string(),
            index_version: 1,
            created_at: 0,
        };
        // 覆盖率不足（末章没有到文件末尾）⇒ 必须重扫。
        assert!(index_from_records(&[record(0, 0, 10)], 20).is_none());
        // 序号不连续 ⇒ 必须重扫。
        assert!(index_from_records(&[record(1, 0, 20)], 20).is_none());
        // 完整覆盖 ⇒ 可用。
        let index = index_from_records(&[record(0, 0, 10), record(1, 10, 20)], 20)
            .expect("完整覆盖必须可重建");
        assert_eq!(index.len(), 2);
        assert_eq!(index.chapter_start(1), Some(10));
        assert!(!index.is_virtual);
    }

    #[test]
    fn storage_errors_map_to_frozen_codes() {
        let error = map_storage_error(StorageError::BookNotFound { book_id: 5 });
        assert_eq!(error.code(), ErrorCode::BookNotFound);
    }
}

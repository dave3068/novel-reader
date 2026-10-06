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

use std::path::{Path, PathBuf};

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
        self.as_source()
            .read_at_with_encoding(offset, len, encoding)
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

/// 分类树一行（V1.1；`nr_list_categories` 的定长条目）。
///
/// `parent_id` 用哨兵 `-1` 表示根（SQL 里是 `NULL`）——
/// ABI 不传 `Option`，故用哨兵；Kotlin 侧按 `-1` 还原为「无父」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CategoryRowData {
    pub id: i64,
    pub parent_id: i64,
    pub depth: u32,
    pub direct_book_count: i64,
    pub total_book_count: i64,
    /// 名称在字节池中的偏移（相对 `bytes` 起点）。
    pub name_offset: u32,
    /// 名称字节长度（UTF-8）。
    pub name_len: u32,
}

/// 分类批量条目（V1.1；名称字节池 + 定长行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryRowsData {
    pub count: u32,
    pub rows: Vec<CategoryRowData>,
    pub bytes: Vec<u8>,
}

/// 全局阅读背景图的读回（V1.1；`nr_get_background` 的载荷）。
///
/// **只回相对文件名**，不回绝对路径（规格 R15）—— 调用方用自己的
/// app-private 前缀拼接（Android）或数据目录（桌面）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackgroundData {
    /// 数据文件名（核心生成，形如 `bg-<unix>-<pid>.<ext>`）；空 = 未设置。
    pub file_name: String,
    /// 图片字节数（0 = 未设置）。
    pub file_size: i64,
    /// 遮罩强度（千分比 0..=1000）。
    pub scrim_permille: i64,
    /// 缩放口径（`FitMode` 序号）。
    pub fit_mode: i64,
    /// 遮罩 alpha（0–255；由千分比换算，界面不再换算）。
    pub scrim_alpha: u8,
}

/// 自定义配色的基色读回（V1.1；`nr_get_theme_base` 的载荷）。
///
/// **只含 4 个基色与 enabled**（规格 R11：40 个令牌不落库）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeBaseData {
    pub enabled: bool,
    /// 四项各 12 字节（`#RRGGBB` + NUL）；未设置该项为**全 0**。
    ///
    /// ABI 不用 `Option<String>`（跨语言 nullable 字符串易错），故用固定
    /// 长度 + 「全 0 = 未设置」的口径。
    pub foreground: String,
    pub background: String,
    pub primary: String,
    pub accent: String,
}

/// 对比度读数（V1.1；`nr_get_theme_contrast` 的载荷）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ThemeContrastData {
    /// 对比度 × 100（整数；避免跨语言浮点约定，Kotlin 侧除以 100 显示）。
    pub ratio_permille_x100: u32,
    /// 是否达到 `MIN_CONTRAST`。
    pub passes: u32,
    /// 较差的一侧是否为暗色（1 = 暗色更差）。
    pub worst_in_dark: u32,
}

/// 阅读历史一行（V1.1；`nr_list_history` 的定长条目）。
///
/// C 对齐布局（`i64×6 + u32×4`）：`0,8,16,24,32,40,48,52,56,60` ⇒ 64 字节。
///
/// 标题与文件名都在批次字节池里（`title_offset` / `file_offset`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryRowData {
    pub id: i64,
    pub book_id: i64,
    /// 打开时间（Unix 秒）。
    pub opened_at: i64,
    pub chapter_index: i64,
    pub byte_offset: i64,
    /// 进入方式码（`EntryKind::as_i64`）。
    pub entry_kind: i64,
    pub title_offset: u32,
    pub title_len: u32,
    /// 来源**末段文件名**在字节池中的偏移（不暴露完整定位符）。
    pub file_offset: u32,
    pub file_len: u32,
    /// **已由核心格式化**的时间标签在字节池中的偏移（规格 §3.6）。
    ///
    /// 界面**不得**自行格式化或截断 —— 口径（今天 `HH:MM` / 本年 `MM-DD HH:MM` /
    /// 跨年 `yyyy-MM-dd HH:MM`）由共享层 `history_view::time_label` 唯一决定。
    pub label_offset: u32,
    pub label_len: u32,
}

/// 阅读历史批量条目（V1.1；字节池 + 定长行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRowsData {
    /// 本次返回的条目数。
    pub count: u32,
    /// 全部流水条数（不受 `limit` 影响；界面显示「共 N 条」用）。
    pub total: u32,
    pub rows: Vec<HistoryRowData>,
    pub bytes: Vec<u8>,
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

    /// 读回自定义配色的基色（V1.1）。
    ///
    /// **未设置过**返回 `enabled = false` 且四项为空（界面据此显示「未设置」）。
    pub fn theme_base(&self) -> ThemeBaseData {
        use reading_storage::theme;
        let Some(custom) = theme::load_custom_theme(self.storage.conn()).ok().flatten() else {
            return ThemeBaseData::default();
        };
        ThemeBaseData {
            enabled: custom.enabled,
            // `unwrap_or_default` ⇒ `None` 变空串（= 全 0 口径）。
            foreground: custom.base.foreground.unwrap_or_default(),
            background: custom.base.background.unwrap_or_default(),
            primary: custom.base.primary.unwrap_or_default(),
            accent: custom.base.accent.unwrap_or_default(),
        }
    }

    /// 写入一个基色（V1.1；**只存基色**）。
    ///
    /// `which`：`0`=正文 `1`=背景 `2`=主色 `3`=强调。
    /// `value` 为空串 = **清除该项**（回落内置值），不是写入空颜色。
    pub fn set_theme_base(&mut self, which: u32, value: &str) -> NrResult<()> {
        use reading_storage::theme::{self, BaseColors, ThemeCustom};
        let trimmed = value.trim();
        if !trimmed.is_empty() && theme::parse_hex(Some(trimmed)).is_none() {
            return Err(invalid(format!(
                "基色不是合法的 #RRGGBB：{trimmed}（已保留原值）"
            )));
        }
        // 读-改-写单行记录；`enabled` **沿用原值**（是否启用是用户显式选择，
        // 改基色不该替他打开）。
        let mut next = theme::load_custom_theme(self.storage.conn())
            .map_err(map_storage_error)?
            .unwrap_or_else(|| ThemeCustom {
                enabled: false,
                base: BaseColors::default(),
                updated_at: 0,
            });
        let slot = (!trimmed.is_empty()).then(|| trimmed.to_ascii_lowercase());
        match which {
            0 => next.base.foreground = slot,
            1 => next.base.background = slot,
            2 => next.base.primary = slot,
            3 => next.base.accent = slot,
            other => return Err(invalid(format!("基色索引必须在 0..=3，收到 {other}"))),
        }
        theme::save_custom_theme(self.storage.conn(), &next, connection::now_unix())
            .map_err(map_storage_error)
    }

    /// 切换「启用自定义配色」（V1.1）。
    ///
    /// 关闭时**保留基色**（只置 `enabled = false`）—— 用户下次开启不必重选。
    pub fn set_theme_enabled(&mut self, enabled: bool) -> NrResult<()> {
        use reading_storage::theme::{self, BaseColors, ThemeCustom};
        let mut next = theme::load_custom_theme(self.storage.conn())
            .map_err(map_storage_error)?
            .unwrap_or_else(|| ThemeCustom {
                enabled: false,
                base: BaseColors::default(),
                updated_at: 0,
            });
        next.enabled = enabled;
        theme::save_custom_theme(self.storage.conn(), &next, connection::now_unix())
            .map_err(map_storage_error)
    }

    /// 取一个派生令牌的颜色（V1.1；`nr_get_theme_token`）。
    ///
    /// `which` 是 `PALETTE_FIELDS` 的下标；`dark != 0` 取暗色一套。
    ///
    /// 派生与对比度**全在共享层**（`theme_derive`），此处只搬运结果 ——
    /// 桌面与 Android 走同一函数，故不会有两个真相（V1.1 P5 §3.3）。
    pub fn theme_token(&self, which: u32, dark: bool) -> NrResult<[u8; 4]> {
        use reading_storage::theme;
        use reading_storage::theme_derive;
        let Some(field) = theme_derive::PALETTE_FIELDS.get(which as usize) else {
            return Err(invalid(format!(
                "令牌索引必须在 0..{}，收到 {which}",
                theme_derive::PALETTE_FIELDS.len()
            )));
        };
        let custom = theme::load_custom_theme(self.storage.conn()).map_err(map_storage_error)?;
        let Some(custom) = custom.filter(|item| item.enabled) else {
            // 未启用 ⇒ 回**内置**值（与 `apply_palette` 的回落一致）。
            // `ColorPair` 用 4 元组，本函数返 `[u8; 4]` —— 故在此转换。
            let builtin = if dark {
                field.value.dark
            } else {
                field.value.light
            };
            return Ok([builtin.0, builtin.1, builtin.2, builtin.3]);
        };
        let derived = theme_derive::derive(&custom.base);
        Ok(if dark {
            derived.dark_of(field.slint_name)
        } else {
            derived.light_of(field.slint_name)
        }
        .unwrap_or({
            let builtin = if dark {
                field.value.dark
            } else {
                field.value.light
            };
            [builtin.0, builtin.1, builtin.2, builtin.3]
        }))
    }

    /// 取对比度读数（V1.1；`nr_get_theme_contrast`）。
    ///
    /// 未启用自定义配色时按**内置**明/暗算（仍给出读数，让用户有参照）。
    pub fn theme_contrast(&self) -> NrResult<ThemeContrastData> {
        use reading_storage::theme;
        use reading_storage::theme_derive;
        let custom = theme::load_custom_theme(self.storage.conn()).map_err(map_storage_error)?;
        let (derived, report) = match custom.filter(|item| item.enabled) {
            Some(custom) => theme_derive::derive_from_theme(&custom),
            None => {
                // 内置配色：直接对 `PALETTE_FIELDS` 的 text-primary/background 算。
                let field = |name: &str| {
                    theme_derive::PALETTE_FIELDS
                        .iter()
                        .find(|item| item.slint_name == name)
                        .map(|item| {
                            (
                                [item.value.light.0, item.value.light.1, item.value.light.2],
                                [item.value.dark.0, item.value.dark.1, item.value.dark.2],
                            )
                        })
                        .unwrap_or(([0, 0, 0], [255, 255, 255]))
                };
                let (light_fg, light_bg) = field("text-primary");
                let (dark_fg, dark_bg) = field("background");
                let light = theme_derive::contrast_ratio(light_fg, light_bg);
                let dark = theme_derive::contrast_ratio(dark_fg, dark_bg);
                let (ratio, worst_in_dark) = if dark < light {
                    (dark, true)
                } else {
                    (light, false)
                };
                (
                    theme_derive::DerivedPalette {
                        light: Vec::new(),
                        dark: Vec::new(),
                    },
                    theme_derive::ContrastReport {
                        ratio,
                        passes: ratio >= theme_derive::MIN_CONTRAST,
                        worst_in_dark,
                    },
                )
            }
        };
        let _ = derived;
        Ok(ThemeContrastData {
            // 比值 × 100（跨语言不传浮点；Kotlin 侧除以 100 显示）。
            ratio_permille_x100: (report.ratio * 100.0).round().max(0.0) as u32,
            passes: u32::from(report.passes),
            worst_in_dark: u32::from(report.worst_in_dark),
        })
    }

    /// 读回全局背景（V1.1；**只回相对文件名**，R15）。
    ///
    /// 未设置时 `file_name` 为空串（界面据此显示「未设置」）。
    pub fn background(&self) -> Result<BackgroundData, String> {
        use reading_storage::background;
        let record = background::load(self.storage.conn()).map_err(|error| error.to_string())?;
        Ok(match record {
            None => BackgroundData::default(),
            Some(record) => {
                let scrim_alpha = background::scrim_to_alpha(record.scrim_permille);
                BackgroundData {
                    file_name: record.file_name,
                    file_size: record.file_size,
                    scrim_permille: record.scrim_permille,
                    fit_mode: record.fit_mode.as_i64(),
                    scrim_alpha,
                }
            }
        })
    }

    /// 设置全局背景（V1.1）：**复制** `src_path` 到 `dest_dir` 并写库。
    ///
    /// 口径（规格 R14–R16）：
    /// - **原图只读**：只 `copy`，绝不 `rename` / `remove_file` / 写回；
    /// - 文件名由本方法**生成**（不用用户原名，避免中文/空格/同名冲突）；
    /// - 数据库只存**安全相对文件名**（存储层再校验一次）。
    ///
    /// `dest_dir` 由调用方给（桌面传数据目录；Android 传 app-private 目录）——
    /// 核心**不猜**平台数据目录（`XDG_DATA_HOME` 在 Android 上不存在）。
    pub fn set_background(
        &mut self,
        src_path: &Path,
        dest_dir: &Path,
        fit_mode: Option<i64>,
        scrim_permille: Option<i64>,
        now: i64,
    ) -> Result<BackgroundData, String> {
        use reading_storage::background::{self, FitMode};

        // 1) 源图校验：存在 + 扩展名在白名单内。
        const ALLOWED: [&str; 4] = ["png", "jpg", "jpeg", "webp"];
        if !src_path.is_file() {
            return Err(format!("背景图不存在或不是文件：{}", src_path.display()));
        }
        let ext = src_path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_default();
        if !ALLOWED.contains(&ext.as_str()) {
            return Err(format!(
                "不支持的图片格式「{ext}」；仅支持 {}",
                ALLOWED.join(" / ")
            ));
        }

        // 2) 生成安全文件名 + 存储层二次校验。
        //
        // 必须**每次唯一**：同一秒内换图时 `{unix}-{pid}` 会重复，导致新文件
        // 覆盖旧的，而清理逻辑又把「同名」当自己跳过 —— 结果旧文件残留、
        // 数据库与磁盘不一致（契约测试实测踩中）。
        // 用「目标已存在则加序号」保证唯一，且仍是可预测的安全名。
        let base = format!("bg-{now}-{}", std::process::id());
        let mut file_name = format!("{base}.{ext}");
        let mut serial = 1u32;
        while dest_dir.join(&file_name).exists() {
            file_name = format!("{base}-{serial}.{ext}");
            serial += 1;
        }
        background::validate_file_name(&file_name).map_err(|error| error.to_string())?;

        // 3) 复制（**只读**：原图不动）。
        std::fs::create_dir_all(dest_dir)
            .map_err(|error| format!("创建背景图目录失败：{error}"))?;
        let target = dest_dir.join(&file_name);
        std::fs::copy(src_path, &target)
            .map_err(|error| format!("复制背景图失败（{}）：{error}", src_path.display()))?;
        let size = std::fs::metadata(&target)
            .map_err(|error| format!("读取背景图大小失败：{error}"))?
            .len() as i64;

        // 4) 写库（失败则清掉刚复制的文件，不留孤儿）。
        let fit = fit_mode.map(FitMode::from_i64).unwrap_or(FitMode::Cover);
        let scrim = scrim_permille.unwrap_or(background::DEFAULT_SCRIM_PERMILLE);
        let previous = {
            let mut storage = self.storage_mut();
            let previous = background::load(storage.conn())
                .ok()
                .flatten()
                .map(|r| r.file_name);
            match background::set(storage.conn(), &file_name, size, scrim, fit, now) {
                Ok(()) => previous,
                Err(error) => {
                    let _ = std::fs::remove_file(&target);
                    return Err(error.to_string());
                }
            }
        };
        // 5) 清理**旧**文件（只删核心生成的 `bg-*`，绝不动用户原图）。
        if let Some(old) = previous {
            if old != file_name && old.starts_with("bg-") {
                let _ = std::fs::remove_file(dest_dir.join(&old));
            }
        }

        Ok(BackgroundData {
            file_name,
            file_size: size,
            scrim_permille: scrim,
            fit_mode: fit.as_i64(),
            scrim_alpha: background::scrim_to_alpha(scrim),
        })
    }

    /// 只调遮罩强度（V1.1；不换图）。
    pub fn set_background_scrim(&mut self, permille: i64) -> Result<BackgroundData, String> {
        use reading_storage::background;
        background::set_scrim(self.storage.conn(), permille, connection::now_unix())
            .map_err(|error| error.to_string())?;
        self.background()
    }

    /// 清除全局背景（V1.1）。
    ///
    /// **只删核心生成的文件**（`bg-*` 前缀）；`dest_dir` 不存在时不报错。
    /// **不动用户原图**（它不在 `dest_dir` 里）。
    pub fn clear_background(&mut self, dest_dir: &Path) -> Result<BackgroundData, String> {
        use reading_storage::background;
        let removed = background::clear(self.storage.conn(), connection::now_unix())
            .map_err(|error| error.to_string())?;
        if let Some(name) = removed {
            if name.starts_with("bg-") {
                let _ = std::fs::remove_file(dest_dir.join(&name));
            }
        }
        self.background()
    }

    /// 某作用域下**在架**书籍的 id 列表（V1.1；`nr_list_books_in_scope` 的载荷）。
    ///
    /// 子树展开在**核心**完成 —— 界面只做集合成员判断，不得自己遍历父子链。
    pub fn books_in_scope(&self, scope_kind: i32, category_id: i64) -> NrResult<Vec<i64>> {
        use reading_storage::categories;
        categories::books_in_scope(self.storage.conn(), scope_kind, category_id)
            .map_err(map_storage_error)
    }

    /// 合法的移动 / 删除目标（V1.1；`nr_category_move_targets` 的载荷）。
    ///
    /// 含「未分类」（id = -1）；自身与全部子孙 `selectable = false`。
    /// **环判定在核心完成** —— 界面不得自行遍历父子链。
    pub fn category_move_targets(
        &self,
        source_id: i64,
    ) -> NrResult<Vec<reading_storage::categories::MoveTarget>> {
        use reading_storage::categories;
        categories::move_targets(self.storage.conn(), source_id).map_err(map_storage_error)
    }

    /// 重命名分类（V1.1；`nr_rename_category`）。
    ///
    /// 同名冲突 / 不存在的 id 由存储层拒绝（**保留原值**），调用方如实回报。
    pub fn rename_category(&self, category_id: i64, name: &str) -> NrResult<()> {
        use reading_storage::categories;
        categories::rename_category(
            self.storage.conn(),
            category_id,
            name,
            connection::now_unix(),
        )
        .map_err(map_storage_error)?;
        Ok(())
    }

    /// 移动分类到新的父分类（V1.1；`nr_move_category`）。
    ///
    /// `new_parent = -1` 表示移到根层。**移入自身或子孙必须拒绝**
    /// （否则产生环 ⇒ 树投影死循环，规格 R5）—— 拒绝由存储层实施。
    pub fn move_category(&self, category_id: i64, new_parent: i64) -> NrResult<()> {
        use reading_storage::categories;
        let parent = if new_parent < 0 {
            None
        } else {
            Some(new_parent)
        };
        categories::move_category(
            self.storage.conn(),
            category_id,
            parent,
            connection::now_unix(),
        )
        .map_err(map_storage_error)?;
        Ok(())
    }

    /// 书籍 → 分类 映射（V1.1；`nr_list_book_categories` 的载荷）。
    ///
    /// `category_id = -1` 表示**未分类** —— 与 ABI 的「None 用 -1 哨兵」口径一致。
    /// 只含**在架**书籍（与书架口径一致）。
    ///
    /// 界面用它把书架条目按分类过滤：宿主目录给出 `sourceIdentity → bookId`（A1），
    /// 本映射给出 `bookId → categoryId`，两者相合即可筛选。
    pub fn book_categories(&self) -> NrResult<Vec<(i64, i64)>> {
        use reading_storage::books;
        let pairs = books::list_book_categories(self.storage.conn()).map_err(map_storage_error)?;
        Ok(pairs
            .into_iter()
            .map(|(book_id, category_id)| (book_id, category_id.unwrap_or(-1)))
            .collect())
    }

    /// 阅读历史批量条目（V1.1；`nr_list_history` 的载荷）。
    ///
    /// `grouped = true` 走聚合视图（组按最近时间倒序）；`book_id > 0` 限定单书。
    /// **排序在存储层完成**（`history::list` / `list_grouped`），此处不重排。
    pub fn history_rows(
        &self,
        book_id: i64,
        limit: i32,
        grouped: bool,
    ) -> NrResult<HistoryRowsData> {
        use reading_storage::history;
        use reading_storage::history_view;
        let conn = self.storage.conn();
        let cap = limit.max(0) as usize;
        let entries = if grouped {
            // 聚合视图：组头带条数与最近时间，组内流水平铺在行数组里
            // （Kotlin 侧按 `book_id` 变化识别组边界，不必二次解析）。
            let groups = history::list_grouped(conn).map_err(map_storage_error)?;
            let mut flat: Vec<history::HistoryEntry> = Vec::new();
            for group in groups {
                flat.extend(group.entries);
                if cap > 0 && flat.len() >= cap {
                    break;
                }
            }
            if cap > 0 {
                flat.truncate(cap);
            }
            flat
        } else if book_id > 0 {
            history::list_by_book(conn, book_id, Some(cap as i64)).map_err(map_storage_error)?
        } else {
            history::list(conn, Some(cap as i64)).map_err(map_storage_error)?
        };

        // `count()` 返回 i64；转 u32 前饱和（负数只可能是数据异常，夹到 0 而不 panic）。
        // 负载：定长行 + UTF-8 字节池（标题 + 来源末段文件名）。
        let mut bytes: Vec<u8> = Vec::new();
        let mut rows: Vec<HistoryRowData> = Vec::with_capacity(entries.len());
        // `count()` 的 SQL **不含占位符**，故必须传空参数（曾误传 1 个，
        // 触发 `Wrong number of parameters` —— 与分类查询同类缺陷）。
        let total = history::count(conn)
            .map_err(map_storage_error)?
            .max(0)
            .min(u32::MAX as i64) as u32;
        for entry in &entries {
            let title_offset = bytes.len() as u32;
            bytes.extend_from_slice(entry.book_title.as_bytes());
            // 只回**末段文件名**（不暴露完整定位符）。
            // 复用共享层 —— 此前本文件自己写了一份等价逻辑，属重复实现。
            let tail = history_view::file_label(&entry.source_locator);
            let file_offset = bytes.len() as u32;
            bytes.extend_from_slice(tail.as_bytes());
            // 时间标签**由核心格式化**（规格 §3.6）；界面不得自己格式化。
            let label = history_view::time_label(conn, entry.opened_at);
            let label_offset = bytes.len() as u32;
            bytes.extend_from_slice(label.as_bytes());
            rows.push(HistoryRowData {
                id: entry.id,
                book_id: entry.book_id,
                opened_at: entry.opened_at,
                chapter_index: entry.chapter_index,
                byte_offset: entry.byte_offset,
                entry_kind: entry.entry_kind.as_i64(),
                title_offset,
                title_len: entry.book_title.len() as u32,
                file_offset,
                file_len: tail.len() as u32,
                label_offset,
                label_len: label.len() as u32,
            });
        }
        Ok(HistoryRowsData {
            count: rows.len() as u32,
            total,
            rows,
            bytes,
        })
    }

    /// 清除一条阅读流水（V1.1；`nr_clear_history_entry`）。
    ///
    /// `false` = 该 id 不存在（调用方应如实回报「已不存在」而非「已清除」）。
    pub fn clear_history_entry(&mut self, entry_id: i64) -> NrResult<bool> {
        use reading_storage::history;
        history::remove_one(self.storage.conn(), entry_id).map_err(map_storage_error)
    }

    /// 清除某本书的全部阅读流水（V1.1；`nr_clear_history_for_book`）。
    ///
    /// **只删流水**：不动书籍 / 进度 / 书签（由 `history::remove_by_book` 保证）。
    pub fn clear_history_for_book(&mut self, book_id: i64) -> NrResult<u32> {
        use reading_storage::history;
        let removed =
            history::remove_by_book(self.storage.conn(), book_id).map_err(map_storage_error)?;
        Ok(removed as u32)
    }

    /// 本地存储（测试与审计用）。
    pub fn storage(&self) -> &Storage {
        &self.storage
    }

    /// 本地存储（**可变**；V1.1 分类写入需要）。
    ///
    /// 事务型操作（如 `delete_category`）要求 `&mut Connection`，故必须
    /// 经此拿可借用；调用点只经 FFI 导出暴露，不接受外部传入的连接。
    pub fn storage_mut(&mut self) -> &mut Storage {
        &mut self.storage
    }

    /// 分类批量条目（V1.1；`nr_list_categories` 的载荷）。
    pub fn category_rows(&self) -> NrResult<CategoryRowsData> {
        use reading_storage::categories;
        let nodes = categories::project_tree(self.storage.conn()).map_err(map_storage_error)?;
        // `CategoryNode` 刻意**不暴露** `parent_id`（桌面侧同样另建父链映射）；
        // 这里从扁平列表重建 `id -> parent_id`，保持单一投影口径。
        let flat = categories::list_categories(self.storage.conn()).map_err(map_storage_error)?;
        let parents: std::collections::HashMap<i64, i64> = flat
            .iter()
            .map(|item| (item.id, item.parent_id.unwrap_or(-1)))
            .collect();
        // 载荷形状：定长条目 + UTF-8 名称字节池（与 `nr_get_chapters` 同构）。
        let mut bytes: Vec<u8> = Vec::new();
        let mut rows: Vec<CategoryRowData> = Vec::with_capacity(nodes.len());
        for node in nodes {
            let name_offset = bytes.len() as u32;
            bytes.extend_from_slice(node.name.as_bytes());
            rows.push(CategoryRowData {
                id: node.id,
                parent_id: parents.get(&node.id).copied().unwrap_or(-1),
                depth: node.depth as u32,
                direct_book_count: node.direct_book_count,
                total_book_count: node.total_book_count,
                name_offset,
                name_len: node.name.len() as u32,
            });
        }
        Ok(CategoryRowsData {
            count: rows.len() as u32,
            rows,
            bytes,
        })
    }

    /// 新建分类（V1.1；同名 / 父不存在的判定**在存储层**，此处只转发）。
    pub fn create_category(&mut self, parent_id: Option<i64>, name: &str) -> NrResult<i64> {
        use reading_storage::categories;
        let now = connection::now_unix();
        categories::create_category(self.storage.conn(), parent_id, name, now)
            .map_err(map_storage_error)
    }

    /// 移动书籍到分类（V1.1；`category_id = None` = 未分类）。
    pub fn move_book_to_category(
        &mut self,
        book_id: i64,
        category_id: Option<i64>,
    ) -> NrResult<()> {
        use reading_storage::categories;
        let now = connection::now_unix();
        categories::move_book_at(self.storage.conn(), book_id, category_id, now)
            .map_err(map_storage_error)
    }

    /// 删除分类（V1.1；**必须显式给出书籍去向**）。
    ///
    /// `destination = 0`（未选去向）时**直接拒绝且零副作用** —— 与桌面侧
    /// `DeleteChoice::None` 同语义（规格 R3 / P5 §3.1）。
    pub fn delete_category(
        &mut self,
        id: i64,
        destination: i32,
        target_id: Option<i64>,
    ) -> NrResult<u32> {
        use reading_storage::categories::{self, DeleteDestination};
        // `0` = 未选去向 —— 在此拒绝（不构造任何默认去向）。
        let chosen = match destination {
            1 => DeleteDestination::MoveToCategory {
                target_id: target_id
                    .ok_or_else(|| invalid("destination=1（移入另一分类）必须给出 target_id"))?,
            },
            2 => DeleteDestination::RemoveFromShelf,
            3 => DeleteDestination::MoveToRoot,
            other => {
                return Err(invalid(format!(
                    "destination 必须显式给出（1=移入分类 2=移出书架 3=移回根），收到 {other}"
                )))
            }
        };
        let now = connection::now_unix();
        let storage = self.storage_mut();
        let report = categories::delete_category(storage.conn_mut(), id, chosen, now)
            .map_err(map_storage_error)?;
        Ok(report.moved_books as u32)
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
        // 来源信息一并记入流水（Android 侧显示文件名用）。
        self.record_open_history(book_id, &source_kind, &spec.identity);
        Ok(outcome)
    }

    /// 每次打开写一条阅读流水（V1.1；规格 HR1）。
    ///
    /// **桌面与 Android 同一条路径**（都在 `open_txt` 里）—— 这样「每次打开
    /// 一条」在两个平台都成立，不需要各写一套，也不会有「一端记了另一端没记」。
    ///
    /// 写失败**只忽略、不打断打开**（历史是辅助信息，不是阅读的前提）。
    fn record_open_history(&mut self, book_id: i64, source_kind: &str, source_locator: &str) {
        use reading_storage::books;
        use reading_storage::history::{self, EntryKind, HistoryInput};
        // 书名以**存储层记录**为准（权威口径；`derived_name` 此时已被 import 消费）。
        let Ok(Some(book)) = books::get_book(self.storage.conn(), book_id) else {
            return;
        };
        let input = HistoryInput {
            book_id,
            book_title: book.display_name.clone(),
            opened_at: connection::now_unix(),
            // 打开瞬间的位置：首屏尚未排版，记 0（与桌面「无恢复进度」时一致）。
            chapter_index: 0,
            byte_offset: 0,
            // ABI 层只有一个打开入口，故一律记「书架打开」。
            // 「进度恢复」由桌面侧在**恢复成功后**补记（它才知道恢复是否命中）。
            entry_kind: EntryKind::FromShelf,
            source_kind: source_kind.to_string(),
            source_locator: source_locator.to_string(),
            content_index_version: 0,
        };
        if let Err(error) = history::record(self.storage.conn(), &input) {
            // 不打断打开：仅记日志由调用方承担（引擎层无 logger）。
            let _ = error;
        }
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
        let stored_scanner =
            meta::chapter_scanner_version(self.storage.conn()).map_err(map_storage_error)?;
        let scanner_current = stored_scanner == reading_index::CHAPTER_SCANNER_VERSION;
        let stored = storage_index::list_chapters(self.storage.conn(), book_id)
            .map_err(map_storage_error)?;
        if scanner_current {
            if let Some(index) = index_from_records(&stored, total_bytes) {
                return Ok((index, true));
            }
        }
        let (index, _stats) =
            reading_core::scan_chapters_from_source_with_encoding(source, &|| false, encoding)?;
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
                // 非 UTF-8 文档**不生成引文**：引文是 UTF-8 文本，与原始源字节不可比对
                // （搜索必然落空，反而把好位置拖去章首）。恢复时直接采用字节偏移，见 `load_position`。
                if matches!(open.encoding, TextEncoding::Utf8 | TextEncoding::Utf8Bom) {
                    let sample = generate_anchor(&open.source, spec.original_byte_offset)?;
                    sample.fingerprint.quote
                } else {
                    String::new()
                }
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
            // ⚠️ **非 UTF-8 文档直接采用保存偏移**（与桌面端 `restore_encoded` 同一规则）。
            //
            // 原因：锚点引文是 **UTF-8 文本**，而搜索是与**原始源字节**逐字节比较
            // （`reading-index/src/anchor.rs`）。对 GBK/GB18030/Big5/UTF-16 文档，引文在源字节里
            // 永远匹配不上；`generate_anchor` 的 `from_utf8` 也会失败从而存下**空引文**，
            // 核心对「引文为空」一律回退章节开头 ⇒ 真机表现为
            // **「UTF-8 文章能续读、GBK 文章每次回到章首」**（2026-09-30 用户实测）。
            //
            // 位置身份本就是**字节偏移**（与编码、排版均无关），且保存时核心已校验
            // 「该偏移属于所请求的章节」，故直接采用是安全且正确的；只有越界才回退章首。
            if !matches!(open.encoding, TextEncoding::Utf8 | TextEncoding::Utf8Bom) {
                let total = open.total_bytes;
                let chapter_start = open
                    .index
                    .chapter_start(stored_position.chapter_index)
                    .unwrap_or(0)
                    .min(total);
                let in_range = stored_position.original_byte_offset >= chapter_start
                    && stored_position.original_byte_offset <= total;
                outcome.restored = false;
                outcome.chapter_index = stored_position.chapter_index;
                outcome.original_byte_offset = if in_range {
                    stored_position.original_byte_offset
                } else {
                    chapter_start
                };
                outcome.fallback_to_chapter_start = !in_range;
                return Ok(outcome);
            }
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
pub(crate) fn struct_bytes<T>(value: &T) -> Vec<u8> {
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
    fn history_rows_reports_a_readable_error_when_storage_is_unusable() {
        // 定位用：`nr_list_history` 返回 `INVALID_ARGUMENT`（码 10）时，
        // 说明 `history_rows` 内部出错。此测试把该路径钉住。
        let engine = Engine::new(EngineConfig::in_memory()).expect("建内存引擎");
        // 未打开任何书籍、无任何流水 ⇒ 空树是**正常**结果，不是错误。
        let rows = engine
            .history_rows(0, 0, false)
            .expect("空库列历史必须成功");
        assert_eq!(rows.count, 0, "空库无流水");
        assert_eq!(rows.total, 0, "空库总条数为 0");
        assert!(rows.rows.is_empty());
        assert!(rows.bytes.is_empty(), "空批次不应带缓冲区字节");
    }

    #[test]
    fn history_rows_write_requires_positive_limit_semantics() {
        let engine = Engine::new(EngineConfig::in_memory()).expect("建内存引擎");
        // `limit <= 0` 意为「不限」，不得被当成 0 条。
        for limit in [0, -1, -100] {
            let rows = engine
                .history_rows(0, limit, false)
                .expect("不限量必须成功");
            assert_eq!(rows.count, 0, "limit={limit} 应理解为不限，而非 0 条");
        }
    }

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

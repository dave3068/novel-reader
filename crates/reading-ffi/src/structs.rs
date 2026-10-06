//! ABI 结构体：与 `include/novel_reader.h` 的 C 结构体**逐字段对应**。
//!
//! 每个结构体都是 `#[repr(C)]` 且只含整数、（标量）指针与 `size_t`，因此字段顺序、
//! 大小与对齐在 C / Rust 两侧一致；`#[repr(C)]` 结构体内部不出现任何 Rust 独有类型。
//!
//! # 头文件一致性
//!
//! [`ABI_STRUCTS`] 是头文件的**机器可读清单**（结构体名 → 有序 `(字段名, C 类型)`），
//! 由 `tests/header_parity.rs` 解析真实头文件逐项比对：字段名、C 类型、出现顺序三者
//! 完全一致才通过。因此"头文件与 Rust 导出符号逐项一致"不是靠人眼复核。
//!
//! # 变长数据只装在自己的缓冲区里
//!
//! 需要字符串的结构体一律用「指针 + 长度」引用**调用方**的字节（`nr_open_request`、
//! `nr_position`、`nr_bookmark_input`），或引用 Rust 归还给调用方的 `nr_buffer`
//! （`nr_position_result`）。结构体里**没有**指向 Rust 堆上 `String` 的裸指针，
//! 因此不会出现"释放谁的内存"这种含糊。

use std::os::raw::c_void;

use crate::buffer::NrBuffer;

/// 引擎配置（`nr_engine_config`）。
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NrEngineConfig {
    /// 结构体字节大小；必须等于本结构体的 `sizeof`，否则拒绝创建。
    pub struct_size: u32,
    /// 调用方编译期使用的 ABI 版本；必须等于 `NR_ABI_VERSION`。
    pub abi_version: u32,
    /// 单次读取窗口上限（字节）；`0` 表示使用硬上限 `NR_MAX_WINDOW_BYTES`。
    pub max_window_bytes: u64,
    /// 平台窗口提供者；平台数据源必需，文件数据源可为 `NULL`。
    pub provider: *const c_void,
    /// 数据库路径（UTF-8）；`NULL` / 长度为 0 表示使用内存数据库（测试与只读会话）。
    pub db_path_utf8: *const u8,
    /// 数据库路径字节数。
    pub db_path_len: usize,
}

/// 打开请求（`nr_open_request`）。
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NrOpenRequest {
    /// 结构体字节大小。
    pub struct_size: u32,
    /// 数据源种类（`NR_SOURCE_KIND_FILE` / `NR_SOURCE_KIND_PLATFORM`）。
    pub kind: u32,
    /// 坐标空间总字节数；平台数据源必填，文件数据源忽略（以文件实际长度为准）。
    pub total_bytes: u64,
    /// 来源标识（文件路径 / SAF URI），UTF-8。
    pub identity_utf8: *const u8,
    /// 来源标识字节数。
    pub identity_len: usize,
    /// 展示名（书名），UTF-8；`NULL` 表示由引擎从来源标识推导。
    pub display_name_utf8: *const u8,
    /// 展示名字节数。
    pub display_name_len: usize,
}

/// 打开结果（`nr_open_result`）。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NrOpenResult {
    /// 书籍标识（本地存储主键，64 位）。
    pub book_id: u64,
    /// 坐标空间总字节数（文件数据源为真实文件长度）。
    pub total_bytes: u64,
    /// 内容索引版本（分页缓存键的一半）。
    pub content_index_version: u64,
    /// 内容单元标识（V1 恒为 `NR_CONTENT_UNIT_SINGLE`）。
    pub content_unit_id: u32,
    /// 编码稳定数值（`NR_ENCODING_*`）。
    pub encoding: u32,
    /// 编码可信度千分比（0..=1000）。
    pub encoding_confidence: u32,
    /// 编码识别综合评分（0..=1000）。
    pub encoding_score: u32,
    /// 是否需要用户确认编码（1 = 低可信度且有内容可判）。
    pub needs_encoding_confirmation: u32,
    /// 章节数量（至少 1：无章节时是单一虚拟全文章节）。
    pub chapters: u32,
    /// 章节索引是否来自本地存储缓存（1 = 未重新扫描）。
    pub index_from_cache: u32,
    /// 保留字段，必须为 0。
    pub reserved0: u32,
}

/// 读取窗口结果（`nr_window`）；字节本体在 `nr_buffer` 中。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NrWindow {
    /// 窗口起始原始字节偏移（= 请求的 offset）。
    pub offset: u64,
    /// 窗口结束偏移（不含）；可能小于 offset + len（尾部裁剪）。
    pub end_byte: u64,
    /// 本次读取的引擎侧请求序号（单调递增；与取消请求无关）。
    pub request_id: u64,
    /// 坐标空间总字节数。
    pub total_bytes: u64,
    /// 实际返回的字节数（= 缓冲区 len）。
    pub len: u32,
    /// 因字符边界被裁掉的头部字节数。
    pub dropped_leading: u32,
    /// 因字符边界被裁掉的尾部字节数。
    pub trimmed_tail: u32,
    /// 是否到达内容末尾（1 = EOF）。
    pub eof: u32,
}

/// 排版签名（`nr_layout_signature`，13 项冻结字段 + 字体族字符串引用）。
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NrLayoutSignature {
    /// 视口宽度（像素）。
    pub viewport_width: u32,
    /// 视口高度（像素）。
    pub viewport_height: u32,
    /// 排版方向（0 横 / 1 纵）。
    pub direction: u32,
    /// 字重。
    pub font_weight: u32,
    /// 字号（千分之一像素）。
    pub font_size_milli: u32,
    /// 行距（千分之一像素）。
    pub line_spacing_milli: u32,
    /// 段距（千分之一像素）。
    pub paragraph_spacing_milli: u32,
    /// 首行缩进（千分之一像素）。
    pub first_line_indent_milli: u32,
    /// 水平边距（千分之一像素）。
    pub margin_horizontal_milli: u32,
    /// 垂直边距（千分之一像素）。
    pub margin_vertical_milli: u32,
    /// 排版算法版本（必须与 `NR_LAYOUT_ALGORITHM_VERSION` 一致）。
    pub algorithm_version: u32,
    /// 保留字段，必须为 0。
    pub reserved0: u32,
    /// 平台字体度量指纹（0 = 尚未取得）。
    pub platform_font_fingerprint: u64,
    /// 字体族（UTF-8，调用方拥有）。
    pub font_family_utf8: *const u8,
    /// 字体族字节数（必须 > 0）。
    pub font_family_len: usize,
}

/// 页边界提交（`nr_page_boundary_submission`）。
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NrPageBoundarySubmission {
    /// 结构体字节大小。
    pub struct_size: u32,
    /// 提交阶段（`NR_SUBMIT_BEGIN_LAYOUT` / `NR_SUBMIT_COMMIT`）。
    pub phase: u32,
    /// 内容索引版本（必须等于引擎当前内容索引版本）。
    pub content_index_version: u64,
    /// 排版世代；提交阶段必填（由 `BEGIN_LAYOUT` 返回）。
    pub layout_id: u64,
    /// 请求标识；提交阶段必填（由 `BEGIN_LAYOUT` 返回）。
    pub request_id: u64,
    /// 坐标空间总字节数（必须等于书籍长度）。
    pub total_bytes: u64,
    /// 页面起始字节。
    pub start_byte: u64,
    /// 页面结束字节（必须位于合法 UTF-8 字符边界）。
    pub end_byte: u64,
    /// 排版签名；仅 `BEGIN_LAYOUT` 阶段需要，提交阶段可为 `NULL`。
    pub signature: *const NrLayoutSignature,
}

/// 页边界结果（`nr_boundary_result`）。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NrBoundaryResult {
    /// 状态（`NR_BOUNDARY_*`）。
    pub status: u32,
    /// 被拒绝原因（`NR_REJECT_*`；未拒绝为 0）。
    pub rejection: u32,
    /// 跨 ABI 错误码（`NR_ERR_*`；成功为 0）。
    pub error_code: i32,
    /// 保留字段，必须为 0。
    pub reserved0: u32,
    /// 当前排版世代。
    pub layout_id: u64,
    /// 请求标识（`BEGIN_LAYOUT` 返回新分配的标识）。
    pub request_id: u64,
    /// 被丢弃的排版世代（过期类状态才有值）。
    pub stale_layout_id: u64,
    /// 被丢弃的请求标识（`NR_BOUNDARY_STALE_REQUEST_ID` 才有值）。
    pub stale_request_id: u64,
    /// 已登记边界的起始字节（未命中为 0）。
    pub start_byte: u64,
    /// 已登记边界的结束字节（未命中为 0）。
    pub end_byte: u64,
    /// 坐标空间总字节数（回显）。
    pub total_bytes: u64,
    /// 当前世代已登记的边界数量。
    pub page_count: u64,
}

/// 内容位置（`nr_position`）；字符串为调用方拥有的 UTF-8 引用。
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NrPosition {
    /// 结构体字节大小。
    pub struct_size: u32,
    /// 章节序号。
    pub chapter_index: u32,
    /// 书籍标识。
    pub book_id: u64,
    /// 内容单元标识（V1 恒为 `NR_CONTENT_UNIT_SINGLE`）。
    pub content_unit_id: u64,
    /// 原始文件字节偏移。
    pub original_byte_offset: u64,
    /// 附近文本指纹（UTF-8）；`NULL` 表示由引擎在保存时自动采样。
    pub nearby_fingerprint_utf8: *const u8,
    /// 附近文本指纹字节数。
    pub nearby_fingerprint_len: usize,
}

/// 位置读取结果（`nr_position_result`）；指纹由 Rust 分配，调用方必须归还。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NrPositionResult {
    /// 结构体字节大小。
    pub struct_size: u32,
    /// 是否找到已保存的位置（0 = 该书尚无进度）。
    pub found: u32,
    /// 章节序号。
    pub chapter_index: u32,
    /// 是否经过锚点恢复（1 = 用附近文本指纹在有界半径内重新定位过）。
    pub restored: u32,
    /// 是否回退到章节开头（锚点为空或半径内未命中）。
    pub fallback_to_chapter_start: u32,
    /// 保留字段，必须为 0。
    pub reserved0: u32,
    /// 书籍标识。
    pub book_id: u64,
    /// 内容单元标识。
    pub content_unit_id: u64,
    /// 原始文件字节偏移（可能已被恢复修正）。
    pub original_byte_offset: u64,
    /// 进度更新时间（Unix 秒）。
    pub updated_at: u64,
    /// 附近文本指纹（UTF-8；Rust 分配，调用方用 `nr_release_buffer` 归还）。
    pub nearby_fingerprint: NrBuffer,
}

/// 章节条目（`nr_chapter`）；标题字节位于批次缓冲区的 `title_offset` 处。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NrChapter {
    /// 章节序号（从 0 起）。
    pub sequence: u32,
    /// 命中规则数值（`reading_index::ChapterRule::id`）。
    pub rule_id: u32,
    /// 章节起始字节。
    pub start_byte: u64,
    /// 章节结束字节（不含）。
    pub end_byte: u64,
    /// 标题字节相对批次缓冲区起始的偏移。
    pub title_offset: u32,
    /// 标题字节数。
    pub title_len: u32,
    /// 章节索引版本。
    pub index_version: u32,
    /// 保留字段，必须为 0。
    pub reserved0: u32,
}

/// 章节批次（`nr_chapter_batch`）：固定长度数组 + 尾随 UTF-8 标题字节。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NrChapterBatch {
    /// 本次返回的章节条目数。
    pub count: u32,
    /// 章节索引版本。
    pub index_version: u32,
    /// 该书章节总数。
    pub total_chapters: u32,
    /// 保留字段，必须为 0。
    pub reserved0: u32,
    /// 字节缓冲区（`nr_chapter[count]` + 标题字节；Rust 分配，调用方归还）。
    pub bytes: NrBuffer,
}

/// 分类树一行（`nr_category`；V1.1 分类批量，规格 P5 §3.1）。
///
/// C 对齐布局（`u32,u32,u32,u32,i64,i64,i64,i64`）：
/// `0,4,8,12,16,24,32,40` ⇒ 48 字节。
///
/// `parent_id` 用哨兵 `-1` 表示根（存储层是 `NULL`）—— ABI 不传 `Option`。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NrCategory {
    /// 分类 id。
    pub id: i64,
    /// 父分类 id；`-1` = 根下。
    pub parent_id: i64,
    /// 直属书籍数。
    pub direct_book_count: i64,
    /// 含子孙的合计书籍数。
    pub total_book_count: i64,
    /// 层级深度（根下为 1）。
    pub depth: u32,
    /// 名称在批次字节池中的偏移。
    pub name_offset: u32,
    /// 名称字节长度（UTF-8）。
    pub name_len: u32,
    /// 保留字段，必须为 0。
    pub reserved0: u32,
}

/// 分类批次（`nr_category_batch`）：固定长度数组 + 尾随 UTF-8 名称字节。
///
/// 与 [`NrChapterBatch`] 同构，故 Kotlin 侧可复用同一套缓冲区归还流程。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NrCategoryBatch {
    /// 本次返回的分类条目数。
    pub count: u32,
    /// 保留字段，必须为 0。
    pub reserved0: u32,
    /// 字节缓冲区（`nr_category[count]` + 名称字节；Rust 分配，调用方归还）。
    pub bytes: NrBuffer,
}

/// 阅读历史一行（`nr_history`；V1.1 阅读历史批量，规格 P5 §3.2）。
///
/// C 对齐布局（`i64×6 + u32×6`）：`0,8,16,24,32,40,48,52,56,60,64,68` ⇒ 72 字节。
///
/// 标题与文件名都在批次字节池里（`title_offset` / `file_offset`）；
/// `file_*` 只含来源**末段文件名**（不暴露完整定位符）。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NrHistory {
    /// 流水 id。
    pub id: i64,
    /// 书籍 id。
    pub book_id: i64,
    /// 打开时间（Unix 秒）。
    pub opened_at: i64,
    /// 打开时的章节序号。
    pub chapter_index: i64,
    /// 打开时的字节偏移。
    pub byte_offset: i64,
    /// 进入方式码（0=书架打开 1=阅读页打开 2=进度恢复 3=书签跳转 4=章节跳转）。
    pub entry_kind: i64,
    /// 书名在字节池中的偏移 / 长度（UTF-8）。
    pub title_offset: u32,
    pub title_len: u32,
    /// 文件名在字节池中的偏移 / 长度（UTF-8）。
    pub file_offset: u32,
    pub file_len: u32,
    /// **已由核心格式化**的时间标签在字节池中的偏移 / 长度（UTF-8；规格 §3.6）。
    ///
    /// 界面**不得**自行格式化 —— 口径由 `reading_storage::history_view::time_label`
    /// 唯一决定（今天 `HH:MM` / 本年 `MM-DD HH:MM` / 跨年 `yyyy-MM-dd HH:MM`）。
    pub label_offset: u32,
    pub label_len: u32,
}

/// 阅读历史批次（`nr_history_batch`）：固定长度数组 + 尾随 UTF-8 字节。
///
/// 与 [`NrCategoryBatch`] 同构，Kotlin 侧复用同一套缓冲区归还流程。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NrHistoryBatch {
    /// 本次返回的条目数。
    pub count: u32,
    /// 全部流水条数（**不受** `limit` 影响；界面显示「共 N 条」用）。
    pub total: u32,
    /// 字节缓冲区（`nr_history[count]` + 标题/文件名字节；Rust 分配，调用方归还）。
    pub bytes: NrBuffer,
}

/// 自定义配色的基色（`nr_theme_base`；V1.1 配色，规格 P5 §3.3）。
///
/// **只含 4 个基色与 enabled** —— 40 个语义令牌**不落库**（规格 R11），
/// 由核心按基色确定性派生（桌面与 Android 同一函数）。
///
/// 四项各 12 字节（`#RRGGBB` + NUL）；**全 0 = 该项未设置**（沿用内置值）。
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NrThemeBase {
    /// 保留字段，必须为 0（对齐既有结构的首字段惯例）。
    pub struct_size: u32,
    /// 是否启用自定义配色（0 = 关闭，走内置明/暗；1 = 启用）。
    pub enabled: u32,
    /// 正文基色 `#RRGGBB` + NUL；全 0 = 未设置。
    pub foreground: [u8; 12],
    /// 背景基色（同上口径）。
    pub background: [u8; 12],
    /// 主色基色（同上口径）。
    pub primary: [u8; 12],
    /// 强调色基色（同上口径）。
    pub accent: [u8; 12],
}

impl Default for NrThemeBase {
    fn default() -> Self {
        Self {
            struct_size: std::mem::size_of::<NrThemeBase>() as u32,
            enabled: 0,
            foreground: [0; 12],
            background: [0; 12],
            primary: [0; 12],
            accent: [0; 12],
        }
    }
}

/// 对比度读数（`nr_theme_contrast_result`；V1.1）。
///
/// 比值用**整数 × 100**（跨语言不传浮点；Kotlin 侧除以 100 显示为 `x.xx : 1`）。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NrThemeContrastResult {
    pub struct_size: u32,
    /// 对比度 × 100（如 `4.54:1` ⇒ 454）。
    pub ratio_x100: u32,
    /// 是否达到可读性建议（1 = 达标；0 = 不足，**已保留用户基色**）。
    pub passes: u32,
    /// 较差的一侧是否为暗色（1 = 暗色更差）。
    pub worst_in_dark: u32,
    /// 保留字段，必须为 0。
    pub reserved0: u32,
    /// 保留字段，必须为 0。
    ///
    /// 存在的原因：ABI 契约要求**每个结构体按 8 字节对齐**（`abi_layout` 逐项断言），
    /// 5 × u32 = 20 不满足，故补到 24。
    pub reserved1: u32,
}

/// 全局阅读背景图（`nr_background`；V1.1 背景，规格 P5 §3.4）。
///
/// **只回相对文件名**，不回绝对路径（R15）—— 调用方用自己的 app-private
/// 前缀（Android）或数据目录（桌面）拼接。
///
/// C 对齐布局（i64×3 + i64 + u8 + u32×3）：`0,8,16,24,32,40,44` ⇒ 48 字节（8 对齐）。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NrBackground {
    /// 图片字节数（0 = 未设置）。
    pub file_size: i64,
    /// 遮罩强度（千分比 0..=1000）。
    pub scrim_permille: i64,
    /// 缩放口径（0=覆盖铺满 1=原始尺寸平铺 2=原始尺寸居中）。
    pub fit_mode: i64,
    /// 保留字段，必须为 0。
    pub reserved0: i64,
    /// 遮罩 alpha（0–255；已由核心从千分比换算，界面不再换算）。
    pub scrim_alpha: u8,
    /// 保留字段，必须为 0。
    pub reserved1: u8,
    pub reserved2: u16,
    /// 数据文件名（核心生成，形如 `bg-<unix>-<pid>.<ext>`）；
    /// **长度 0 = 未设置**（不进字节池，避免未设置时也要归还缓冲区）。
    pub file_name_len: u32,
    /// 文件名在批次字节池中的偏移（相对整块缓冲区）。
    pub file_name_offset: u32,
    /// 保留字段，必须为 0。
    pub reserved3: u32,
}

/// 背景批次（`nr_background_batch`）：定长头 + 文件名字节。
///
/// 与其它批次同构，Kotlin 侧复用同一套缓冲区归还流程。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NrBackgroundBatch {
    /// 本批条目数（恒为 0 或 1；保留字段口径与其它批次一致）。
    pub count: u32,
    /// 保留字段，必须为 0。
    pub reserved0: u32,
    /// 字节缓冲区（`nr_background[1]` + 文件名字节；Rust 分配，调用方归还）。
    pub bytes: NrBuffer,
}

/// 移动 / 删除目标的一行（`nr_category_choice`；V1.1）。
///
/// C 对齐布局：`i64 + i32 + u32 + i64 + u32 × 4` ⇒ **40 字节**（8 字节对齐）。
///
/// `id = -1` 表示「未分类」。`selectable = 0` 表示自身或子孙（选了会成环）——
/// 刻意**不隐藏**而只标记，便于向用户解释「为什么不能选」。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NrCategoryChoice {
    pub id: i64,
    pub depth: i32,
    /// 0 = 不可选（自身 / 子孙）；1 = 可选。
    pub selectable: u32,
    /// 含子孙的在架书籍数（未分类行 = 未分类书籍数）。
    pub total_count: i64,
    pub name_offset: u32,
    pub name_len: u32,
    pub reserved0: u32,
    pub reserved1: u32,
}

/// 目标选项批次（`nr_category_choice_batch`）。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NrCategoryChoiceBatch {
    pub count: u32,
    pub reserved0: u32,
    pub bytes: NrBuffer,
}

/// 通用 i64 列表（`nr_id_list`；V1.1）。
///
/// C 对齐布局（u32 × 2 + 24 字节 `nr_buffer`）⇒ **32 字节**。
/// 字节池是 `count` 个小端 `i64`（`bytes.len == count * 8`）。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NrIdList {
    /// 元素个数。
    pub count: u32,
    /// 保留字段，必须为 0。
    pub reserved0: u32,
    /// 字节缓冲区（`int64_t[count]`；Rust 分配，调用方归还）。
    pub bytes: NrBuffer,
}

/// 书籍 → 分类 映射的一行（`nr_book_category`；V1.1）。
///
/// C 对齐布局（i64 × 2）：`0,8` ⇒ **16 字节**（8 字节对齐）。
///
/// `category_id = -1` 表示**未分类**（ABI 的 None 哨兵口径）。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NrBookCategory {
    pub book_id: i64,
    /// 分类 id；`-1` = 未分类。
    pub category_id: i64,
}

/// 书籍分类批次（`nr_book_category_batch`）。
///
/// 行内**无字符串**，故字节池就是行本身（`bytes.len == count * 16`）；
/// 仍沿用「批次头 + 字节池」的统一形状，Kotlin 侧复用同一套归还流程。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NrBookCategoryBatch {
    /// 本批条目数。
    pub count: u32,
    /// 保留字段，必须为 0。
    pub reserved0: u32,
    /// 字节缓冲区（`nr_book_category[count]`；Rust 分配，调用方归还）。
    pub bytes: NrBuffer,
}

/// 书签输入（`nr_bookmark_input`）。
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NrBookmarkInput {
    /// 结构体字节大小。
    pub struct_size: u32,
    /// 章节序号（必须与偏移所在章节一致）。
    pub chapter_index: u32,
    /// 书籍标识（必须等于当前打开的书籍）。
    pub book_id: u64,
    /// 内容单元标识（V1 恒为 `NR_CONTENT_UNIT_SINGLE`）。
    pub content_unit_id: u64,
    /// 原始字节偏移。
    pub byte_offset: u64,
    /// 书签名称（UTF-8）；`NULL` / 长度 0 表示使用章节标题。
    pub title_utf8: *const u8,
    /// 书签名称字节数。
    pub title_len: usize,
}

/// `nr_set_encoding` 的出参（手动指定内容编码）。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NrSetEncodingResult {
    /// 结构体字节大小。
    pub struct_size: u32,
    /// 生效编码（`NR_ENCODING_*`；0 表示尚未确定）。
    pub encoding: u32,
    /// 重建后的内容索引版本（旧分页缓存据此失效）。
    pub content_index_version: u64,
    /// 按新编码重扫后的章节数量。
    pub chapters: u32,
    /// 1 = 本次就地重扫了新索引。
    pub index_rebuilt: u32,
}

/// 索引重建结果（`nr_rebuild_result`）。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NrRebuildResult {
    /// 结构体字节大小。
    pub struct_size: u32,
    /// 是否为部分结果（1 = 被取消，结果不完整）。
    pub partial: u32,
    /// 章节数量。
    pub chapters: u32,
    /// 索引是否来自本地存储缓存（1 = 未重新扫描）。
    pub index_from_cache: u32,
    /// 本次实际扫描的字节数。
    pub scanned_bytes: u64,
    /// 坐标空间总字节数。
    pub total_bytes: u64,
    /// 重建后的内容索引版本。
    pub content_index_version: u64,
}

/// 最近一次错误（`nr_error_info`）；消息由 Rust 分配，调用方必须归还。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NrErrorInfo {
    /// 跨 ABI 错误码（`NR_ERR_*`；无错误为 0）。
    pub code: i32,
    /// 是否带有可读消息（0/1）。
    pub has_message: u32,
    /// 消息缓冲区（UTF-8；Rust 分配，调用方用 `nr_release_buffer` 归还）。
    pub message: NrBuffer,
}

/// 冻结的 ABI 结构体清单：结构体名 → 有序 `(字段名, C 类型)`。
///
/// 与 `include/novel_reader.h` 中同名结构体的字段名、类型、顺序必须完全一致；
/// `nr_source_provider` 的回调字段在头文件中为函数指针类型，这里以同一 C 类型文本登记。
pub const ABI_STRUCTS: &[(&str, &[(&str, &str)])] = &[
    (
        "nr_buffer",
        &[
            ("ptr", "uint8_t *"),
            ("len", "size_t"),
            ("capacity", "size_t"),
        ],
    ),
    (
        "nr_source_provider",
        &[
            ("user", "void *"),
            (
                "read_window",
                "int32_t (*)(void *, uint64_t, uint64_t, uint32_t, uint8_t *, uint32_t *, int32_t *)",
            ),
        ],
    ),
    (
        "nr_engine_config",
        &[
            ("struct_size", "uint32_t"),
            ("abi_version", "uint32_t"),
            ("max_window_bytes", "uint64_t"),
            ("provider", "const nr_source_provider *"),
            ("db_path_utf8", "const uint8_t *"),
            ("db_path_len", "size_t"),
        ],
    ),
    (
        "nr_open_request",
        &[
            ("struct_size", "uint32_t"),
            ("kind", "uint32_t"),
            ("total_bytes", "uint64_t"),
            ("identity_utf8", "const uint8_t *"),
            ("identity_len", "size_t"),
            ("display_name_utf8", "const uint8_t *"),
            ("display_name_len", "size_t"),
        ],
    ),
    (
        "nr_open_result",
        &[
            ("book_id", "uint64_t"),
            ("total_bytes", "uint64_t"),
            ("content_index_version", "uint64_t"),
            ("content_unit_id", "uint32_t"),
            ("encoding", "uint32_t"),
            ("encoding_confidence", "uint32_t"),
            ("encoding_score", "uint32_t"),
            ("needs_encoding_confirmation", "uint32_t"),
            ("chapters", "uint32_t"),
            ("index_from_cache", "uint32_t"),
            ("reserved0", "uint32_t"),
        ],
    ),
    (
        "nr_window",
        &[
            ("offset", "uint64_t"),
            ("end_byte", "uint64_t"),
            ("request_id", "uint64_t"),
            ("total_bytes", "uint64_t"),
            ("len", "uint32_t"),
            ("dropped_leading", "uint32_t"),
            ("trimmed_tail", "uint32_t"),
            ("eof", "uint32_t"),
        ],
    ),
    (
        "nr_layout_signature",
        &[
            ("viewport_width", "uint32_t"),
            ("viewport_height", "uint32_t"),
            ("direction", "uint32_t"),
            ("font_weight", "uint32_t"),
            ("font_size_milli", "uint32_t"),
            ("line_spacing_milli", "uint32_t"),
            ("paragraph_spacing_milli", "uint32_t"),
            ("first_line_indent_milli", "uint32_t"),
            ("margin_horizontal_milli", "uint32_t"),
            ("margin_vertical_milli", "uint32_t"),
            ("algorithm_version", "uint32_t"),
            ("reserved0", "uint32_t"),
            ("platform_font_fingerprint", "uint64_t"),
            ("font_family_utf8", "const uint8_t *"),
            ("font_family_len", "size_t"),
        ],
    ),
    (
        "nr_page_boundary_submission",
        &[
            ("struct_size", "uint32_t"),
            ("phase", "uint32_t"),
            ("content_index_version", "uint64_t"),
            ("layout_id", "uint64_t"),
            ("request_id", "uint64_t"),
            ("total_bytes", "uint64_t"),
            ("start_byte", "uint64_t"),
            ("end_byte", "uint64_t"),
            ("signature", "const nr_layout_signature *"),
        ],
    ),
    (
        "nr_boundary_result",
        &[
            ("status", "uint32_t"),
            ("rejection", "uint32_t"),
            ("error_code", "int32_t"),
            ("reserved0", "uint32_t"),
            ("layout_id", "uint64_t"),
            ("request_id", "uint64_t"),
            ("stale_layout_id", "uint64_t"),
            ("stale_request_id", "uint64_t"),
            ("start_byte", "uint64_t"),
            ("end_byte", "uint64_t"),
            ("total_bytes", "uint64_t"),
            ("page_count", "uint64_t"),
        ],
    ),
    (
        "nr_position",
        &[
            ("struct_size", "uint32_t"),
            ("chapter_index", "uint32_t"),
            ("book_id", "uint64_t"),
            ("content_unit_id", "uint64_t"),
            ("original_byte_offset", "uint64_t"),
            ("nearby_fingerprint_utf8", "const uint8_t *"),
            ("nearby_fingerprint_len", "size_t"),
        ],
    ),
    (
        "nr_position_result",
        &[
            ("struct_size", "uint32_t"),
            ("found", "uint32_t"),
            ("chapter_index", "uint32_t"),
            ("restored", "uint32_t"),
            ("fallback_to_chapter_start", "uint32_t"),
            ("reserved0", "uint32_t"),
            ("book_id", "uint64_t"),
            ("content_unit_id", "uint64_t"),
            ("original_byte_offset", "uint64_t"),
            ("updated_at", "uint64_t"),
            ("nearby_fingerprint", "nr_buffer"),
        ],
    ),
    (
        "nr_chapter",
        &[
            ("sequence", "uint32_t"),
            ("rule_id", "uint32_t"),
            ("start_byte", "uint64_t"),
            ("end_byte", "uint64_t"),
            ("title_offset", "uint32_t"),
            ("title_len", "uint32_t"),
            ("index_version", "uint32_t"),
            ("reserved0", "uint32_t"),
        ],
    ),
    (
        "nr_chapter_batch",
        &[
            ("count", "uint32_t"),
            ("index_version", "uint32_t"),
            ("total_chapters", "uint32_t"),
            ("reserved0", "uint32_t"),
            ("bytes", "nr_buffer"),
        ],
    ),
    (
        "nr_category",
        &[
            ("id", "int64_t"),
            ("parent_id", "int64_t"),
            ("direct_book_count", "int64_t"),
            ("total_book_count", "int64_t"),
            ("depth", "uint32_t"),
            ("name_offset", "uint32_t"),
            ("name_len", "uint32_t"),
            ("reserved0", "uint32_t"),
        ],
    ),
    (
        "nr_category_batch",
        &[
            ("count", "uint32_t"),
            ("reserved0", "uint32_t"),
            ("bytes", "nr_buffer"),
        ],
    ),
    (
        "nr_history",
        &[
            ("id", "int64_t"),
            ("book_id", "int64_t"),
            ("opened_at", "int64_t"),
            ("chapter_index", "int64_t"),
            ("byte_offset", "int64_t"),
            ("entry_kind", "int64_t"),
            ("title_offset", "uint32_t"),
            ("title_len", "uint32_t"),
            ("file_offset", "uint32_t"),
            ("file_len", "uint32_t"),
            ("label_offset", "uint32_t"),
            ("label_len", "uint32_t"),
        ],
    ),
    (
        "nr_history_batch",
        &[
            ("count", "uint32_t"),
            ("total", "uint32_t"),
            ("bytes", "nr_buffer"),
        ],
    ),
    (
        "nr_theme_base",
        &[
            ("struct_size", "uint32_t"),
            ("enabled", "uint32_t"),
            ("foreground[12]", "uint8_t"),
            ("background[12]", "uint8_t"),
            ("primary[12]", "uint8_t"),
            ("accent[12]", "uint8_t"),
        ],
    ),
    (
        "nr_theme_contrast_result",
        &[
            ("struct_size", "uint32_t"),
            ("ratio_x100", "uint32_t"),
            ("passes", "uint32_t"),
            ("worst_in_dark", "uint32_t"),
            ("reserved0", "uint32_t"),
            ("reserved1", "uint32_t"),
        ],
    ),
    (
        "nr_background",
        &[
            ("file_size", "int64_t"),
            ("scrim_permille", "int64_t"),
            ("fit_mode", "int64_t"),
            ("reserved0", "int64_t"),
            ("scrim_alpha", "uint8_t"),
            ("reserved1", "uint8_t"),
            ("reserved2", "uint16_t"),
            ("file_name_len", "uint32_t"),
            ("file_name_offset", "uint32_t"),
            ("reserved3", "uint32_t"),
        ],
    ),
    (
        "nr_background_batch",
        &[
            ("count", "uint32_t"),
            ("reserved0", "uint32_t"),
            ("bytes", "nr_buffer"),
        ],
    ),
    (
        "nr_category_choice",
        &[
            ("id", "int64_t"),
            ("depth", "int32_t"),
            ("selectable", "uint32_t"),
            ("total_count", "int64_t"),
            ("name_offset", "uint32_t"),
            ("name_len", "uint32_t"),
            ("reserved0", "uint32_t"),
            ("reserved1", "uint32_t"),
        ],
    ),
    (
        "nr_category_choice_batch",
        &[
            ("count", "uint32_t"),
            ("reserved0", "uint32_t"),
            ("bytes", "nr_buffer"),
        ],
    ),
    (
        "nr_id_list",
        &[
            ("count", "uint32_t"),
            ("reserved0", "uint32_t"),
            ("bytes", "nr_buffer"),
        ],
    ),
    (
        "nr_book_category",
        &[
            ("book_id", "int64_t"),
            ("category_id", "int64_t"),
        ],
    ),
    (
        "nr_book_category_batch",
        &[
            ("count", "uint32_t"),
            ("reserved0", "uint32_t"),
            ("bytes", "nr_buffer"),
        ],
    ),
    (
        "nr_bookmark_input",
        &[
            ("struct_size", "uint32_t"),
            ("chapter_index", "uint32_t"),
            ("book_id", "uint64_t"),
            ("content_unit_id", "uint64_t"),
            ("byte_offset", "uint64_t"),
            ("title_utf8", "const uint8_t *"),
            ("title_len", "size_t"),
        ],
    ),
    (
        "nr_set_encoding_result",
        &[
            ("struct_size", "uint32_t"),
            ("encoding", "uint32_t"),
            ("content_index_version", "uint64_t"),
            ("chapters", "uint32_t"),
            ("index_rebuilt", "uint32_t"),
        ],
    ),
    (
        "nr_rebuild_result",
        &[
            ("struct_size", "uint32_t"),
            ("partial", "uint32_t"),
            ("chapters", "uint32_t"),
            ("index_from_cache", "uint32_t"),
            ("scanned_bytes", "uint64_t"),
            ("total_bytes", "uint64_t"),
            ("content_index_version", "uint64_t"),
        ],
    ),
    (
        "nr_error_info",
        &[
            ("code", "int32_t"),
            ("has_message", "uint32_t"),
            ("message", "nr_buffer"),
        ],
    ),
];

/// 结构体字节大小的 `size_of`（供测试打印与断言）。
pub fn struct_sizes() -> Vec<(&'static str, usize)> {
    vec![
        ("nr_buffer", std::mem::size_of::<NrBuffer>()),
        (
            "nr_source_provider",
            std::mem::size_of::<crate::external::NrSourceProvider>(),
        ),
        ("nr_engine_config", std::mem::size_of::<NrEngineConfig>()),
        ("nr_open_request", std::mem::size_of::<NrOpenRequest>()),
        ("nr_open_result", std::mem::size_of::<NrOpenResult>()),
        ("nr_window", std::mem::size_of::<NrWindow>()),
        (
            "nr_layout_signature",
            std::mem::size_of::<NrLayoutSignature>(),
        ),
        (
            "nr_page_boundary_submission",
            std::mem::size_of::<NrPageBoundarySubmission>(),
        ),
        (
            "nr_boundary_result",
            std::mem::size_of::<NrBoundaryResult>(),
        ),
        ("nr_position", std::mem::size_of::<NrPosition>()),
        (
            "nr_position_result",
            std::mem::size_of::<NrPositionResult>(),
        ),
        ("nr_chapter", std::mem::size_of::<NrChapter>()),
        ("nr_chapter_batch", std::mem::size_of::<NrChapterBatch>()),
        ("nr_category", std::mem::size_of::<NrCategory>()),
        ("nr_category_batch", std::mem::size_of::<NrCategoryBatch>()),
        ("nr_history", std::mem::size_of::<NrHistory>()),
        ("nr_history_batch", std::mem::size_of::<NrHistoryBatch>()),
        ("nr_theme_base", std::mem::size_of::<NrThemeBase>()),
        (
            "nr_theme_contrast_result",
            std::mem::size_of::<NrThemeContrastResult>(),
        ),
        ("nr_background", std::mem::size_of::<NrBackground>()),
        (
            "nr_background_batch",
            std::mem::size_of::<NrBackgroundBatch>(),
        ),
        (
            "nr_category_choice",
            std::mem::size_of::<NrCategoryChoice>(),
        ),
        (
            "nr_category_choice_batch",
            std::mem::size_of::<NrCategoryChoiceBatch>(),
        ),
        ("nr_id_list", std::mem::size_of::<NrIdList>()),
        ("nr_book_category", std::mem::size_of::<NrBookCategory>()),
        (
            "nr_book_category_batch",
            std::mem::size_of::<NrBookCategoryBatch>(),
        ),
        ("nr_bookmark_input", std::mem::size_of::<NrBookmarkInput>()),
        (
            "nr_set_encoding_result",
            std::mem::size_of::<NrSetEncodingResult>(),
        ),
        ("nr_rebuild_result", std::mem::size_of::<NrRebuildResult>()),
        ("nr_error_info", std::mem::size_of::<NrErrorInfo>()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_manifest_is_complete_and_unique() {
        let mut names: Vec<&str> = ABI_STRUCTS.iter().map(|(name, _)| *name).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total, "结构体清单不得重复");
        assert_eq!(
            total, 30,
            "头文件与 Rust 必须同时维护 17 个结构体（16 + nr_set_encoding_result）"
        );
        for (name, fields) in ABI_STRUCTS {
            assert!(name.starts_with("nr_"), "{name} 必须使用 nr_ 前缀");
            let mut field_names: Vec<&str> = fields.iter().map(|(field, _)| *field).collect();
            let field_total = field_names.len();
            field_names.sort_unstable();
            field_names.dedup();
            assert_eq!(field_names.len(), field_total, "{name} 字段名不得重复");
            assert!(!fields.is_empty(), "{name} 必须有字段");
        }
    }

    #[test]
    fn struct_sizes_are_stable_and_no_struct_is_empty() {
        let sizes = struct_sizes();
        assert_eq!(sizes.len(), ABI_STRUCTS.len());
        for (name, size) in sizes {
            assert!(size > 0, "{name} 不得为零大小");
            assert_eq!(size % 4, 0, "{name} 大小必须是 4 的倍数（无异常填充）");
        }
    }

    #[test]
    fn layout_signature_is_thirteen_integer_fields_plus_font_reference() {
        // 冻结 13 项尺寸/方向/字重/算法版本字段 + 字体族引用 + 保留字段。
        let (_, fields) = ABI_STRUCTS
            .iter()
            .find(|(name, _)| *name == "nr_layout_signature")
            .expect("必须登记 nr_layout_signature");
        let integer_fields = fields
            .iter()
            .filter(|(_, ty)| ty.starts_with("uint") || ty.starts_with("int"))
            .count();
        assert_eq!(
            integer_fields, 13,
            "13 项冻结字段中 12 项为整数，加上 1 项保留字段必须都是整数类型"
        );
        assert!(
            fields.iter().any(|(name, _)| *name == "font_family_utf8")
                && fields.iter().any(|(name, _)| *name == "font_family_len"),
            "冻结字段『字体族』必须拆成 UTF-8 指针 + 长度两项"
        );
        assert_eq!(fields.len(), 15, "13 项冻结字段（字体族占 2 项）+ 保留字段");
    }

    #[test]
    fn chapter_struct_has_no_payload_and_batch_carries_titles() {
        let (_, chapter) = ABI_STRUCTS
            .iter()
            .find(|(name, _)| *name == "nr_chapter")
            .expect("必须登记 nr_chapter");
        assert!(
            !chapter.iter().any(|(field, _)| field.contains("body")),
            "章节条目不得包含正文字段"
        );
        assert_eq!(std::mem::size_of::<NrChapter>(), 40);
        assert_eq!(std::mem::size_of::<NrWindow>(), 48);
        assert_eq!(std::mem::size_of::<NrLayoutSignature>(), 72);
    }
}

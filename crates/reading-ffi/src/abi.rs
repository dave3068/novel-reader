//! ABI 常量、错误码映射与冻结入口清单。
//!
//! 本模块是 `include/novel_reader.h` 的 Rust 侧对照：头文件里的每一个宏、结构体字段和
//! 函数声明都必须在这里有同数值、同顺序的对应项，并由测试逐项校验
//! （`tests/header_parity.rs` 直接解析头文件比对；`scripts/verify-abi.py` 比对导出符号）。
//!
//! 冻结依据：`results/gates/gate-0-baseline.md` §2.1（ABI 版本）/§2.2（12 项错误码）/
//! §2.3（64 KiB 默认窗口、256 KiB 硬上限）/§2.7（18 项入口清单 + `nr_set_encoding` 手动编码扩展）。

use reading_core::{ErrorCode, NrError};
use reading_index::boundary::BoundaryRejection;
use reading_source::TextEncoding;

/// 跨语言 ABI 版本（等于 `NR_ABI_VERSION` 宏）。
///
/// 仅在**不兼容** ABI 变更时递增；头文件宏、Rust 常量与契约测试三处必须一致（NFR-C4）。
pub const ABI_VERSION: u32 = reading_core::ABI_VERSION;

/// 平台调用方在 [`crate::NrEngineConfig`] 中声明的版本；与 [`ABI_VERSION`] 不一致时拒绝创建引擎。
pub const ABI_VERSION_REQUIRED: u32 = ABI_VERSION;

/// 单次读取的默认窗口：64 KiB（gate-0 §2.3）。
pub const DEFAULT_WINDOW_BYTES: usize = reading_source::DEFAULT_WINDOW_BYTES;

/// 单次读取的硬上限：256 KiB（gate-0 §2.3）。
pub const MAX_WINDOW_BYTES: usize = reading_source::MAX_WINDOW_BYTES;

/// 单次章节批次返回的最大条目数（禁止一次返回整库章节）。
pub const MAX_CHAPTER_BATCH: u32 = 256;

/// 数据源种类：本地文件（桌面路径 / Android 私有缓存副本路径）。
pub const SOURCE_KIND_FILE: u32 = 1;

/// 数据源种类：平台侧字节窗口（Android SAF 等由平台读取字节的来源）。
///
/// 该种类必须配合 [`crate::NrEngineConfig::provider`] 使用：正文字节永远由平台按**受限窗口**
/// 提供，Rust 只裁定坐标、边界与位置，跨边界不存在"整本正文"。
pub const SOURCE_KIND_PLATFORM: u32 = 2;

/// 页边界提交阶段：登记新排版（签名）并取得 `layout_id` 与 `request_id`。
pub const SUBMIT_BEGIN_LAYOUT: u32 = 1;

/// 页边界提交阶段：提交平台测量结果。
pub const SUBMIT_COMMIT: u32 = 2;

/// 页边界结果状态：新排版已就绪（`BEGIN_LAYOUT` 的返回状态）。
pub const BOUNDARY_LAYOUT_READY: u32 = 100;

/// 页边界结果状态：该世代下没有登记该起点（查询未命中）。
pub const BOUNDARY_NOT_FOUND: u32 = 6;

/// V1 全局唯一的内容单元标识（一个普通 TXT 即一个内容单元，gate-0 §2.4）。
pub const CONTENT_UNIT_SINGLE: u32 = 1;

/// 冻结的 ABI 入口（顺序即 `include/novel_reader.h` 的声明顺序）。\n///\n/// 18 项冻结契约 + 向后兼容新增的 `nr_set_encoding`（规格 CR2 手动覆盖编码）。
///
/// 与 `results/gates/gate-0-baseline.md` §2.7 逐项一致：16 项最小接口 +
/// `nr_release_buffer` + `nr_abi_version`。
pub const ABI_ENTRY_NAMES: [&str; 19] = [
    "nr_abi_version",
    "nr_engine_create",
    "nr_engine_destroy",
    "nr_open_txt",
    "nr_close_book",
    "nr_read_window",
    "nr_submit_page_boundary",
    "nr_get_page_boundary",
    "nr_save_position",
    "nr_get_position",
    "nr_get_chapters",
    "nr_create_bookmark",
    "nr_delete_bookmark",
    "nr_invalidate_layout",
    "nr_rebuild_index",
    "nr_set_encoding",
    "nr_cancel_request",
    "nr_get_last_error",
    "nr_release_buffer",
];

/// 头文件中 `NR_ABI_VERSION` 宏的文本形态（供头文件 —— Rust 对照测试使用）。
pub const ABI_VERSION_MACRO_TEXT: &str = "NR_ABI_VERSION";

/// 冻结的入口**签名**（C 形态；空白在比较前被规范化）。
///
/// 与 [`ABI_ENTRY_NAMES`] 一一对应；`tests/header_parity.rs` 用它逐项比对
/// `include/novel_reader.h` 的函数声明，使"头文件与 Rust 导出符号逐项一致"变成机器校验。
pub const ABI_FUNCTIONS: [(&str, &str); 19] = [
    ("nr_abi_version", "uint32_t(void)"),
    (
        "nr_engine_create",
        "int32_t(const nr_engine_config *, uint64_t *)",
    ),
    ("nr_engine_destroy", "int32_t(uint64_t)"),
    (
        "nr_open_txt",
        "int32_t(uint64_t, const nr_open_request *, nr_open_result *)",
    ),
    ("nr_close_book", "int32_t(uint64_t, uint64_t)"),
    (
        "nr_read_window",
        "int32_t(uint64_t, uint32_t, uint64_t, uint64_t, nr_window *, nr_buffer *)",
    ),
    (
        "nr_submit_page_boundary",
        "int32_t(uint64_t, const nr_page_boundary_submission *, nr_boundary_result *)",
    ),
    (
        "nr_get_page_boundary",
        "int32_t(uint64_t, uint64_t, uint32_t, uint64_t, nr_boundary_result *)",
    ),
    (
        "nr_save_position",
        "int32_t(uint64_t, const nr_position *, int32_t *)",
    ),
    (
        "nr_get_position",
        "int32_t(uint64_t, uint64_t, nr_position_result *)",
    ),
    (
        "nr_get_chapters",
        "int32_t(uint64_t, uint64_t, uint32_t, uint32_t, nr_chapter_batch *)",
    ),
    (
        "nr_create_bookmark",
        "int32_t(uint64_t, const nr_bookmark_input *, uint64_t *)",
    ),
    (
        "nr_delete_bookmark",
        "int32_t(uint64_t, uint64_t, int32_t *)",
    ),
    (
        "nr_invalidate_layout",
        "int32_t(uint64_t, uint64_t, uint64_t *)",
    ),
    (
        "nr_rebuild_index",
        "int32_t(uint64_t, uint64_t, uint64_t, nr_rebuild_result *)",
    ),
    (
        "nr_set_encoding",
        "int32_t(uint64_t, uint64_t, uint32_t, nr_set_encoding_result *)",
    ),
    ("nr_cancel_request", "int32_t(uint64_t, uint64_t)"),
    ("nr_get_last_error", "int32_t(uint64_t, nr_error_info *)"),
    ("nr_release_buffer", "int32_t(nr_buffer *)"),
];

/// 头文件中必须出现的宏清单：`(宏名, 冻结值)`。
///
/// 值来自本 crate 与共享核心的冻结常量，任何一侧改动都会让 `header_parity` 测试失败。
pub fn macro_manifest() -> Vec<(&'static str, u64)> {
    let mut macros: Vec<(&'static str, u64)> = vec![
        ("NR_ABI_VERSION", u64::from(ABI_VERSION)),
        ("NR_DEFAULT_WINDOW_BYTES", DEFAULT_WINDOW_BYTES as u64),
        ("NR_MAX_WINDOW_BYTES", MAX_WINDOW_BYTES as u64),
        ("NR_MAX_CHAPTER_BATCH", u64::from(MAX_CHAPTER_BATCH)),
        ("NR_CONTENT_UNIT_SINGLE", u64::from(CONTENT_UNIT_SINGLE)),
        (
            "NR_LAYOUT_ALGORITHM_VERSION",
            u64::from(reading_core::LAYOUT_ALGORITHM_VERSION),
        ),
        ("NR_SOURCE_KIND_FILE", u64::from(SOURCE_KIND_FILE)),
        ("NR_SOURCE_KIND_PLATFORM", u64::from(SOURCE_KIND_PLATFORM)),
        ("NR_SUBMIT_BEGIN_LAYOUT", u64::from(SUBMIT_BEGIN_LAYOUT)),
        ("NR_SUBMIT_COMMIT", u64::from(SUBMIT_COMMIT)),
        ("NR_BOUNDARY_COMMITTED", 0),
        ("NR_BOUNDARY_DUPLICATE_IDENTICAL", 1),
        ("NR_BOUNDARY_REJECTED", 2),
        ("NR_BOUNDARY_STALE_LAYOUT_ID", 3),
        ("NR_BOUNDARY_STALE_REQUEST_ID", 4),
        ("NR_BOUNDARY_STALE_CONTENT_INDEX_VERSION", 5),
        ("NR_BOUNDARY_NOT_FOUND", u64::from(BOUNDARY_NOT_FOUND)),
        ("NR_BOUNDARY_LAYOUT_READY", u64::from(BOUNDARY_LAYOUT_READY)),
    ];
    for (index, (name, value)) in ERROR_CODE_NAMES.iter().enumerate() {
        debug_assert_eq!(*value, index as i32);
        macros.push((name, *value as u64));
    }
    for code in EncodingCode::ALL {
        macros.push((code.macro_name(), u64::from(code.as_u32())));
    }
    for (macro_name, rejection) in REJECTION_MACROS {
        macros.push((macro_name, u64::from(rejection_id(rejection))));
    }
    macros
}

/// 错误码 → 稳定短名（与 `NR_ERR_*` 一一对应；仅供日志与断言）。
pub const ERROR_CODE_NAMES: [(&str, i32); 12] = [
    ("NR_ERR_OK", 0),
    ("NR_ERR_INVALID_ARGUMENT", 1),
    ("NR_ERR_OUT_OF_RANGE", 2),
    ("NR_ERR_UTF8_BOUNDARY", 3),
    ("NR_ERR_BOOK_NOT_FOUND", 4),
    ("NR_ERR_NO_OPEN_BOOK", 5),
    ("NR_ERR_IO", 6),
    ("NR_ERR_CANCELLED", 7),
    ("NR_ERR_LAYOUT_NOT_FOUND", 8),
    ("NR_ERR_ENGINE_DESTROYED", 9),
    ("NR_ERR_INTERNAL", 10),
    ("NR_ERR_CHAPTER_OUT_OF_RANGE", 11),
];

/// 编码稳定数值（跨语言传输用；不得随意变更）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodingCode {
    /// UTF-8（无 BOM）。
    Utf8 = 1,
    /// 带 BOM 的 UTF-8。
    Utf8Bom = 2,
    /// UTF-16 小端。
    Utf16Le = 3,
    /// UTF-16 大端。
    Utf16Be = 4,
    /// GB18030。
    Gb18030 = 5,
    /// GBK / CP936。
    Gbk = 6,
    /// Big5。
    Big5 = 7,
}

impl EncodingCode {
    /// 全部 7 类编码（顺序与头文件宏一致）。
    pub const ALL: [EncodingCode; 7] = [
        EncodingCode::Utf8,
        EncodingCode::Utf8Bom,
        EncodingCode::Utf16Le,
        EncodingCode::Utf16Be,
        EncodingCode::Gb18030,
        EncodingCode::Gbk,
        EncodingCode::Big5,
    ];

    /// 头文件中对应的宏名。
    pub const fn macro_name(self) -> &'static str {
        match self {
            EncodingCode::Utf8 => "NR_ENCODING_UTF8",
            EncodingCode::Utf8Bom => "NR_ENCODING_UTF8_BOM",
            EncodingCode::Utf16Le => "NR_ENCODING_UTF16LE",
            EncodingCode::Utf16Be => "NR_ENCODING_UTF16BE",
            EncodingCode::Gb18030 => "NR_ENCODING_GB18030",
            EncodingCode::Gbk => "NR_ENCODING_GBK",
            EncodingCode::Big5 => "NR_ENCODING_BIG5",
        }
    }

    /// 由 [`TextEncoding`] 取跨语言数值。
    pub const fn from_encoding(encoding: TextEncoding) -> Self {
        match encoding {
            TextEncoding::Utf8 => EncodingCode::Utf8,
            TextEncoding::Utf8Bom => EncodingCode::Utf8Bom,
            TextEncoding::Utf16Le => EncodingCode::Utf16Le,
            TextEncoding::Utf16Be => EncodingCode::Utf16Be,
            TextEncoding::Gb18030 => EncodingCode::Gb18030,
            TextEncoding::Gbk => EncodingCode::Gbk,
            TextEncoding::Big5 => EncodingCode::Big5,
        }
    }

    /// 跨语言数值。
    pub const fn as_u32(self) -> u32 {
        self as u32
    }

    /// 由跨语言数值还原；未登记数值返回 `None`。
    pub const fn from_u32(value: u32) -> Option<Self> {
        match value {
            1 => Some(EncodingCode::Utf8),
            2 => Some(EncodingCode::Utf8Bom),
            3 => Some(EncodingCode::Utf16Le),
            4 => Some(EncodingCode::Utf16Be),
            5 => Some(EncodingCode::Gb18030),
            6 => Some(EncodingCode::Gbk),
            7 => Some(EncodingCode::Big5),
            _ => None,
        }
    }
}

/// 页边界被拒绝的原因 → 跨语言数值（头文件 `NR_REJECT_*`）。
///
/// `reading-index` 的 [`BoundaryRejection`] 不是 `repr(i32)`，因此这里给出显式映射，
/// 避免把 Rust 判别值当成 ABI 数值。
pub const fn rejection_id(rejection: BoundaryRejection) -> u32 {
    match rejection {
        BoundaryRejection::ReversedInterval => 1,
        BoundaryRejection::EmptyInterval => 2,
        BoundaryRejection::OutOfRange => 3,
        BoundaryRejection::IllegalCharBoundary => 4,
        BoundaryRejection::GapBeforePrevious => 5,
        BoundaryRejection::OverlapWithPrevious => 6,
        BoundaryRejection::NotAdvancing => 7,
        BoundaryRejection::ConflictWithCached => 8,
        BoundaryRejection::TotalLengthMismatch => 9,
    }
}

/// 头文件中 `NR_REJECT_*` 宏与拒绝原因的对应关系（比较前空白规范化）。
pub const REJECTION_MACROS: [(&str, BoundaryRejection); 9] = [
    (
        "NR_REJECT_REVERSED_INTERVAL",
        BoundaryRejection::ReversedInterval,
    ),
    ("NR_REJECT_EMPTY_INTERVAL", BoundaryRejection::EmptyInterval),
    ("NR_REJECT_OUT_OF_RANGE", BoundaryRejection::OutOfRange),
    (
        "NR_REJECT_ILLEGAL_CHAR_BOUNDARY",
        BoundaryRejection::IllegalCharBoundary,
    ),
    (
        "NR_REJECT_GAP_BEFORE_PREVIOUS",
        BoundaryRejection::GapBeforePrevious,
    ),
    (
        "NR_REJECT_OVERLAP_WITH_PREVIOUS",
        BoundaryRejection::OverlapWithPrevious,
    ),
    ("NR_REJECT_NOT_ADVANCING", BoundaryRejection::NotAdvancing),
    (
        "NR_REJECT_CONFLICT_WITH_CACHED",
        BoundaryRejection::ConflictWithCached,
    ),
    (
        "NR_REJECT_TOTAL_LENGTH_MISMATCH",
        BoundaryRejection::TotalLengthMismatch,
    ),
];

/// 可信度 → 千分比（0..=1000），跨语言只传数值。
pub const fn confidence_permille(level: reading_source::ConfidenceLevel) -> u32 {
    match level {
        reading_source::ConfidenceLevel::Low => 333,
        reading_source::ConfidenceLevel::Medium => 666,
        reading_source::ConfidenceLevel::High => 1000,
    }
}

/// 由跨 ABI 数值构造 `NrError`（未登记数值按 [`ErrorCode::Internal`] 处理）。
pub fn error_from_code(code: i32, message: impl Into<String>) -> NrError {
    NrError::new(
        ErrorCode::from_code(code).unwrap_or(ErrorCode::Internal),
        message,
    )
}

/// 构造参数错误。
pub fn invalid(message: impl Into<String>) -> NrError {
    NrError::new(ErrorCode::InvalidArgument, message)
}

/// 构造越界错误。
pub fn out_of_range(message: impl Into<String>) -> NrError {
    NrError::new(ErrorCode::OutOfRange, message)
}

/// 构造内部错误。
pub fn internal(message: impl Into<String>) -> NrError {
    NrError::new(ErrorCode::Internal, message)
}

/// 构造「引擎已销毁」错误。
pub fn engine_destroyed() -> NrError {
    NrError::new(
        ErrorCode::EngineDestroyed,
        "引擎句柄已销毁：销毁后不得再调用任何入口",
    )
}

/// 构造「尚未打开书籍」错误。
pub fn no_open_book() -> NrError {
    NrError::new(
        ErrorCode::NoOpenBook,
        "尚未打开书籍（请先调用 nr_open_txt）",
    )
}

/// 构造「书籍不存在」错误。
pub fn book_not_found(book_id: u64) -> NrError {
    NrError::new(
        ErrorCode::BookNotFound,
        format!("书籍 {book_id} 不在本地存储中"),
    )
}

/// 构造「布局未找到」错误。
pub fn layout_not_found(layout_id: u64) -> NrError {
    NrError::new(
        ErrorCode::LayoutNotFound,
        format!("布局世代 {layout_id} 没有登记的页边界"),
    )
}

/// 构造「章节越界」错误。
pub fn chapter_out_of_range(message: impl Into<String>) -> NrError {
    NrError::new(ErrorCode::ChapterOutOfRange, message)
}

/// 构造「请求已取消」错误。
pub fn cancelled(message: impl Into<String>) -> NrError {
    NrError::new(ErrorCode::Cancelled, message)
}

/// 缓冲区契约的两个文档化失败标签（真正的错误码始终来自统一错误模型）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NrBufferError {
    /// `nr_release_buffer` 的 `buf` 参数为 `NULL`。
    NullPointer,
    /// 结构体字段被破坏（`ptr` 为 `NULL` 但长度非零，或 `len > capacity`）。
    ForgedFields,
}

impl NrBufferError {
    /// 稳定短名（日志用）。
    pub const fn as_str(self) -> &'static str {
        match self {
            NrBufferError::NullPointer => "NullPointer",
            NrBufferError::ForgedFields => "ForgedFields",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_version_matches_frozen_baseline() {
        assert_eq!(ABI_VERSION, 1);
        assert_eq!(ABI_VERSION_REQUIRED, ABI_VERSION);
        assert_eq!(ABI_VERSION_MACRO_TEXT, "NR_ABI_VERSION");
    }

    #[test]
    fn window_limits_match_frozen_baseline() {
        assert_eq!(DEFAULT_WINDOW_BYTES, 65_536);
        assert_eq!(MAX_WINDOW_BYTES, 262_144);
        let default_window: usize = DEFAULT_WINDOW_BYTES;
        let max_window: usize = MAX_WINDOW_BYTES;
        assert!(default_window < max_window, "默认窗口必须小于硬上限");
    }

    #[test]
    fn entry_list_is_unique_and_matches_frozen_count() {
        let mut names = ABI_ENTRY_NAMES.to_vec();
        // 18 项冻结契约 + 向后兼容新增的 `nr_set_encoding`。
        assert_eq!(names.len(), 19, "18 项冻结 + nr_set_encoding");
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total, "入口清单不得有重复");
        for name in names {
            assert!(name.starts_with("nr_"), "入口必须以 nr_ 前缀: {name}");
        }
    }

    #[test]
    fn error_codes_match_gate_zero_table() {
        for (index, (name, value)) in ERROR_CODE_NAMES.iter().enumerate() {
            assert_eq!(*value, index as i32, "{name} 数值必须与冻结表一致");
            assert_eq!(ErrorCode::from_code(*value).map(|c| c.as_c()), Some(*value));
        }
        assert_eq!(ERROR_CODE_NAMES.len(), reading_core::ERROR_CODE_COUNT);
    }

    #[test]
    fn encoding_codes_cover_all_encodings_and_round_trip() {
        let mut seen = Vec::new();
        for encoding in TextEncoding::ALL {
            let code = EncodingCode::from_encoding(encoding);
            assert_eq!(EncodingCode::from_u32(code.as_u32()), Some(code));
            assert!(!seen.contains(&code.as_u32()), "编码数值必须唯一");
            seen.push(code.as_u32());
        }
        assert_eq!(seen.len(), 7);
        assert_eq!(EncodingCode::from_u32(0), None);
        assert_eq!(EncodingCode::from_u32(8), None);
    }

    #[test]
    fn rejection_ids_are_unique_and_stable() {
        let all = [
            BoundaryRejection::ReversedInterval,
            BoundaryRejection::EmptyInterval,
            BoundaryRejection::OutOfRange,
            BoundaryRejection::IllegalCharBoundary,
            BoundaryRejection::GapBeforePrevious,
            BoundaryRejection::OverlapWithPrevious,
            BoundaryRejection::NotAdvancing,
            BoundaryRejection::ConflictWithCached,
            BoundaryRejection::TotalLengthMismatch,
        ];
        let mut ids: Vec<u32> = all.iter().copied().map(rejection_id).collect();
        assert_eq!(ids, vec![1, 2, 3, 4, 5, 6, 7, 8, 9]);
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 9);
        assert_eq!(rejection_id(BoundaryRejection::IllegalCharBoundary), 4);
    }

    #[test]
    fn confidence_permille_is_monotonic() {
        let low = confidence_permille(reading_source::ConfidenceLevel::Low);
        let medium = confidence_permille(reading_source::ConfidenceLevel::Medium);
        let high = confidence_permille(reading_source::ConfidenceLevel::High);
        assert!(low < medium && medium < high && high == 1000);
    }
}

//! `reading-core`：V1 TXT 阅读共享核心（正式产品核心）。
//!
//! 本 crate 为桌面 Rust/Slint 与 Android JNI 提供同一套阅读语义：阅读会话、内容位置、
//! 进度/书签语义、分页边界语义与统一错误模型，并编排 [`reading_source`]、
//! [`reading_index`]、[`reading_storage`]。
//!
//! # 依赖边界（reading-core NFR-C1，由 `scripts/verify-core-scope.py` 强制校验）
//!
//! - 不得依赖 Slint、Compose、Android SDK、网络库或窗口系统；
//! - 不得引用 `experiments/` 下的验证探针（模拟书库、合成文本源、空 External 数据源）；
//! - 所有依赖精确锁定版本，不使用浮动区间。
//!
//! # 契约冻结项
//!
//! - [`ABI_VERSION`]：跨语言 ABI 版本；不兼容变更必须递增，并同步头文件/Kotlin 声明/契约测试
//!   （reading-core NFR-C4）。
//! - [`CORE_VERSION`]：核心 crate 语义版本，跟随 `Cargo.toml`。
//! - [`error::ErrorCode`]：12 项稳定错误码（0 = 成功）。
//!
//! 冻结依据：`results/gates/gate-0-baseline.md`。

pub mod chapter;
pub mod content_unit;
pub mod error;
pub mod layout;
pub mod pagination;
pub mod position;

pub use chapter::{
    chapter_scan_session, generate_anchor, map_source_error, position_at_offset, restore_position,
    scan_chapters_from_source, scan_chapters_from_source_with_encoding, virtual_chapter_index, AnchorRecovery, AnchorSearch, AnchorStatus,
    ChapterIndex, ChapterRange, ChapterRule, RestoredPosition, ScanStep, ANCHOR_QUOTE_CHARS,
    DEFAULT_ANCHOR_RADIUS_BYTES, MAX_ANCHOR_RADIUS_BYTES, VIRTUAL_CHAPTER_TITLE,
};
pub use content_unit::{ContentUnit, CONTENT_UNIT_VERSION, SINGLE_CONTENT_UNIT_ID};
pub use error::{ErrorCode, NrError, NrResult, ERROR_CODE_COUNT};
pub use layout::{
    LayoutKey, LayoutSignature, TextDirection, DEFAULT_FONT_WEIGHT, LAYOUT_ALGORITHM_VERSION,
    LAYOUT_SIGNATURE_ENCODING_PREFIX, LAYOUT_SIGNATURE_FIELDS,
};
pub use pagination::{
    BoundaryCommitOutcome, BoundaryCommitStatus, LayoutChange, LayoutInvalidation,
    PageBoundaryRecord, PageBoundaryRequest, PaginationCache, PAGINATION_VERSION,
};
pub use position::{
    chapter_index_for_offset, chapter_start_offset, ContentPosition, NearbyFingerprint,
    POSITION_ENCODED_FIELDS, POSITION_ENCODING_PREFIX,
};
// 页面边界语义定义在 `reading-index`（下层），此处只做再导出，保持全仓单一类型定义。
pub use reading_index::boundary::{
    check_strict_advance, is_char_boundary_at, is_char_boundary_in_window, validate_interval,
    validate_page_boundary, BoundaryError, BoundaryRejection, PageBoundary, PageBoundaryChain,
    PAGE_BOUNDARY_ENCODING_PREFIX,
};

/// 跨语言 ABI 版本（冻结起始值 `1`，`u32`）。
///
/// 仅在**不兼容** ABI 变更时递增；头文件宏、Rust 常量与契约测试三处必须一致
/// （reading-core NFR-C4、`design.md` §4）。
pub const ABI_VERSION: u32 = 1;

/// 核心 crate 版本，跟随 `Cargo.toml` 的 `version`。
pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_version_matches_frozen_baseline() {
        // 冻结值来自 results/gates/gate-0-baseline.md §2.1（与实验原型 abi.rs 一致）。
        assert_eq!(ABI_VERSION, 1);
        assert_eq!(ABI_VERSION, u32::from(ABI_VERSION as u16));
    }

    #[test]
    fn abi_version_and_core_version_are_exposed() {
        // 先绑定到局部变量：既验证常量在运行期可被外部 crate 读取，也避免
        // Clippy `assertions_on_constants`（对字面量常量做断言没有信息量）。
        let abi: u32 = ABI_VERSION;
        let core_version: &str = CORE_VERSION;
        assert_eq!(
            core_version,
            env!("CARGO_PKG_VERSION"),
            "CORE_VERSION 必须跟随 crate 版本",
        );
        assert!(!core_version.is_empty(), "CORE_VERSION 不得为空");
        assert!(
            core_version.split('.').count() >= 3,
            "CORE_VERSION 必须是语义化版本: {core_version}",
        );
        assert_ne!(abi, 0, "ABI_VERSION 必须为正数");
    }

    #[test]
    fn abi_version_exports_error_code_contract() {
        // ABI 常量与错误码契约同属本卡冻结项，二者必须同时可用。
        assert_eq!(ERROR_CODE_COUNT, 12);
        assert_eq!(ErrorCode::ALL.len(), ERROR_CODE_COUNT);
        assert_eq!(
            ErrorCode::from_code(ABI_VERSION as i32),
            Some(ErrorCode::InvalidArgument)
        );
    }
}

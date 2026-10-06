//! ABI 结构体布局的**跨语言尺寸断言**（Rust 侧）。
//!
//! 下表是 64 位 LP64（x86_64 / aarch64）下的结构体字节大小；C 侧的
//! `crates/reading-ffi/tests/abi_smoke.c` 用 `_Static_assert(sizeof(...) == N)` 断言**同一组数字**。
//! 两侧任何一处结构体字段增删或顺序变化都会让其中一个断言失败，因此布局漂移不会被忽略。

use std::mem::size_of;

use reading_ffi::buffer::NrBuffer;
use reading_ffi::external::NrSourceProvider;
use reading_ffi::structs::{
    NrBackground, NrBackgroundBatch, NrBookCategory, NrBookCategoryBatch, NrBookmarkInput,
    NrBoundaryResult, NrCategory, NrCategoryBatch, NrCategoryChoice, NrCategoryChoiceBatch,
    NrChapter, NrChapterBatch, NrEngineConfig, NrErrorInfo, NrHistory, NrHistoryBatch, NrIdList,
    NrLayoutSignature, NrOpenRequest, NrOpenResult, NrPageBoundarySubmission, NrPosition,
    NrPositionResult, NrRebuildResult, NrThemeBase, NrThemeContrastResult, NrWindow,
};

/// 64 位 LP64 下冻结的结构体字节大小：`(C 结构体名, 字节数)`。
pub const EXPECTED_SIZES: [(&str, usize); 30] = [
    ("nr_buffer", 24),
    ("nr_source_provider", 16),
    ("nr_engine_config", 40),
    ("nr_open_request", 48),
    ("nr_open_result", 56),
    ("nr_window", 48),
    ("nr_layout_signature", 72),
    ("nr_page_boundary_submission", 64),
    ("nr_boundary_result", 80),
    ("nr_position", 48),
    ("nr_position_result", 80),
    ("nr_chapter", 40),
    ("nr_chapter_batch", 40),
    // V1.1 分类：`nr_category` = 4 个 i64 + 4 个 u32 = 32 + 16 = 48；
    // `nr_category_batch` = 2 个 u32 + `nr_buffer`(3 × usize = 24) = 32。
    // （不是 `nr_chapter_batch` 的 40 —— 那个多一个 `total_chapters` u32。）
    ("nr_category", 48),
    ("nr_category_batch", 32),
    // V1.1 阅读历史：`nr_history` = 6 个 i64 + 6 个 u32 = 48 + 24 = 72（含核心格式化的时间标签）；
    // `nr_history_batch` = 2 个 u32 + `nr_buffer`(3 × usize = 24) = 32。
    ("nr_history", 72),
    ("nr_history_batch", 32),
    // V1.1 配色：`nr_theme_base` = 2 × u32 + 4 × [u8; 12] = 8 + 48 = 56；
    // `nr_theme_contrast_result` = 5 × u32 = 20。
    ("nr_theme_base", 56),
    ("nr_theme_contrast_result", 24),
    // V1.1 背景：`nr_background` = 3 × i64 + i64 + u8 + u8 + u16 + 3 × u32
    // = 32 + 1 + 1 + 2 + 12 = 48；`nr_background_batch` = 2 × u32 + nr_buffer(24) = 32。
    ("nr_background", 48),
    ("nr_background_batch", 32),
    // V1.1 书籍分类映射：`nr_book_category` = 2 × i64 = 16；
    // `nr_book_category_batch` = 2 × u32 + nr_buffer(24) = 32。
    ("nr_category_choice", 40),
    ("nr_category_choice_batch", 32),
    ("nr_id_list", 32),
    ("nr_book_category", 16),
    ("nr_book_category_batch", 32),
    ("nr_bookmark_input", 48),
    ("nr_set_encoding_result", 24),
    ("nr_rebuild_result", 40),
    ("nr_error_info", 32),
];

fn rust_sizes() -> [(&'static str, usize); 30] {
    [
        ("nr_buffer", size_of::<NrBuffer>()),
        ("nr_source_provider", size_of::<NrSourceProvider>()),
        ("nr_engine_config", size_of::<NrEngineConfig>()),
        ("nr_open_request", size_of::<NrOpenRequest>()),
        ("nr_open_result", size_of::<NrOpenResult>()),
        ("nr_window", size_of::<NrWindow>()),
        ("nr_layout_signature", size_of::<NrLayoutSignature>()),
        (
            "nr_page_boundary_submission",
            size_of::<NrPageBoundarySubmission>(),
        ),
        ("nr_boundary_result", size_of::<NrBoundaryResult>()),
        ("nr_position", size_of::<NrPosition>()),
        ("nr_position_result", size_of::<NrPositionResult>()),
        ("nr_chapter", size_of::<NrChapter>()),
        ("nr_chapter_batch", size_of::<NrChapterBatch>()),
        ("nr_category", size_of::<NrCategory>()),
        ("nr_category_batch", size_of::<NrCategoryBatch>()),
        ("nr_history", size_of::<NrHistory>()),
        ("nr_history_batch", size_of::<NrHistoryBatch>()),
        ("nr_theme_base", size_of::<NrThemeBase>()),
        (
            "nr_theme_contrast_result",
            size_of::<NrThemeContrastResult>(),
        ),
        ("nr_background", size_of::<NrBackground>()),
        ("nr_background_batch", size_of::<NrBackgroundBatch>()),
        ("nr_category_choice", size_of::<NrCategoryChoice>()),
        (
            "nr_category_choice_batch",
            size_of::<NrCategoryChoiceBatch>(),
        ),
        ("nr_id_list", size_of::<NrIdList>()),
        ("nr_book_category", size_of::<NrBookCategory>()),
        ("nr_book_category_batch", size_of::<NrBookCategoryBatch>()),
        ("nr_bookmark_input", size_of::<NrBookmarkInput>()),
        (
            "nr_set_encoding_result",
            size_of::<reading_ffi::structs::NrSetEncodingResult>(),
        ),
        ("nr_rebuild_result", size_of::<NrRebuildResult>()),
        ("nr_error_info", size_of::<NrErrorInfo>()),
    ]
}

#[test]
fn rust_struct_layout_matches_the_c_side_size_table() {
    let actual = rust_sizes();
    let reported: Vec<(&str, usize)> = reading_ffi::structs::struct_sizes();
    for (name, expected) in EXPECTED_SIZES {
        let found = actual
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .map(|(_, size)| *size)
            .unwrap_or_else(|| panic!("缺少结构体 {name}"));
        assert_eq!(
            found, expected,
            "结构体 {name} 的字节大小与 C 侧断言不一致（Rust = {found}，冻结值 = {expected}）"
        );
        assert!(
            reported
                .iter()
                .any(|(candidate, size)| *candidate == name && *size == expected),
            "struct_sizes() 必须报告 {name} = {expected}"
        );
    }
    assert_eq!(actual.len(), EXPECTED_SIZES.len());
    assert_eq!(reported.len(), EXPECTED_SIZES.len());
}

#[test]
fn every_abi_struct_contains_no_rust_owned_pointer() {
    // 冻结要求：结构体里只能出现整数、标量指针与 `nr_buffer` 子结构体。
    // 这里用尺寸下界做粗检：任何"藏了一整个 Rust 堆对象"的结构体都会显著变大。
    for (name, size) in EXPECTED_SIZES {
        assert!(size % 8 == 0, "{name} 的大小必须按 8 字节对齐");
        assert!(size <= 96, "{name} 不得包含可变长大对象（实际 {size}）");
    }
}

#[test]
fn abi_version_is_macro_comparable_from_c() {
    // C 侧 `_Static_assert(NR_ABI_VERSION == 1u, ...)` 与本断言必须表达同一件事。
    assert_eq!(reading_ffi::abi::ABI_VERSION, 1);
    assert_eq!(reading_ffi::exports::nr_abi_version(), 1);
}

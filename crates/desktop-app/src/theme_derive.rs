//! 派生式自定义配色 —— **桌面侧复用共享层**。
//!
//! 派生算法与令牌表已下沉到 [`reading_storage::theme_derive`]，因为
//! **Android（`reading-ffi`）必须拿到完全相同的派生结果** —— 若桌面另算一套，
//! 就会出现「同一个基色在两端显示不同颜色」的两个真相（V1.1 P5 §3.3 的硬要求）。
//!
//! 本模块只做**转出**（`ui.rs` 的 `apply_palette_with` 依赖这些名字）；
//! 派生、对比度与全部单测都在共享层，本文件**不重复实现**。

pub use reading_storage::theme_derive::{
    check_contrast, contrast_ratio, derive, derive_from_theme, preview_swatches,
    relative_luminance, ContrastReport, DerivedPalette, MIN_CONTRAST,
};

/// 令牌表（与 `desktop/ui/app.slint` 的 `Palette` 声明顺序一致）。
pub use reading_storage::theme_derive::{ColorPair, PaletteField, PALETTE_FIELDS};

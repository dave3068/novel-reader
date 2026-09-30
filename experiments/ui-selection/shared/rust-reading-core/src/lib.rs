//! UI 无关的共享阅读业务原型（技术验证范围）。
//!
//! 该 crate 只为 `ui-technology-validation` 规格提供可比较的共享业务语义：
//! 有限窗口读取、模拟领域数据、内容位置、分页锚点、页面边界登记、布局失效、
//! 事件、取消与测量指标。它不实现正式数据库、ZIP、网络或最终字形布局。
//!
//! 入口：
//! - 直接 Rust API：[`ReadingEngine`]、[`layout_page`]、[`Library`]
//! - 跨语言 C ABI：[`ffi`]，基线入口清单见 [`abi`]

pub mod abi;
pub mod domain;
pub mod engine;
pub mod error;
pub mod ffi;
pub mod fixtures;
pub mod json;
pub mod pagination;
pub mod position;
pub mod rng;
pub mod source;

pub use abi::{AbiEntry, ABI_ENTRIES, ABI_ENTRY_COUNT, ABI_VERSION, BASELINE_SYMBOLS};
pub use domain::{Book, Category, Chapter, HistoryEntry, Library};
pub use engine::{
    BookSource, Event, EventKind, Metrics, NrEventCallback, ReadWindow, ReadingEngine, SubscribeId,
    DEFAULT_MAX_WINDOW,
};
pub use error::{ErrorCode, NrError, NrResult};
pub use pagination::{char_advance, layout_id, layout_page, LayoutConfig, PageLayout};
pub use position::{
    locate_anchor, make_anchor, ContentPosition, PageAnchor, PageBoundary, ANCHOR_QUOTE_CHARS,
};
pub use rng::Rng;
pub use source::{ExternalTextSource, FileTextSource, SyntheticTextSource, TextSource};

/// 当前共享库的语义版本（用于结果记录）。
pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");

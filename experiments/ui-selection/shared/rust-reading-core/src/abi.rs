//! C ABI 基线入口清单（11 个入口 / 12 个导出符号）。
//!
//! 设计文档（design.md §3）列出的 12 个符号中，`nr_engine_create` 与
//! `nr_engine_destroy` 构成一对生命周期入口，因此基线入口数为 11。
//! 该清单被 `verify-workspace.py --shared` 与契约测试消费。

/// ABI 版本（不兼容变更时递增）。
pub const ABI_VERSION: u32 = 1;

/// 基线入口数量。
pub const ABI_ENTRY_COUNT: usize = 11;

/// 基线导出符号数量。
pub const ABI_SYMBOL_COUNT: usize = 12;

/// 单个基线入口。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiEntry {
    /// 入口名（结果与日志使用）。
    pub entry: &'static str,
    /// 该入口导出的 C 符号。
    pub symbols: &'static [&'static str],
    /// 覆盖的需求编号。
    pub requirement: &'static str,
    /// 类别。
    pub kind: &'static str,
}

/// 11 个基线入口。
pub const ABI_ENTRIES: [AbiEntry; ABI_ENTRY_COUNT] = [
    AbiEntry {
        entry: "engine_lifecycle",
        symbols: &["nr_engine_create", "nr_engine_destroy"],
        requirement: "S-R1",
        kind: "lifecycle",
    },
    AbiEntry {
        entry: "open_book",
        symbols: &["nr_open_book"],
        requirement: "S-R1",
        kind: "source",
    },
    AbiEntry {
        entry: "read_window",
        symbols: &["nr_read_window"],
        requirement: "S-R2",
        kind: "window",
    },
    AbiEntry {
        entry: "release_buffer",
        symbols: &["nr_release_buffer"],
        requirement: "S-R2",
        kind: "memory",
    },
    AbiEntry {
        entry: "book_batch",
        symbols: &["nr_get_book_batch"],
        requirement: "S-R1",
        kind: "domain",
    },
    AbiEntry {
        entry: "chapter_batch",
        symbols: &["nr_get_chapter_batch"],
        requirement: "S-R1",
        kind: "domain",
    },
    AbiEntry {
        entry: "save_position",
        symbols: &["nr_save_position"],
        requirement: "S-R3",
        kind: "position",
    },
    AbiEntry {
        entry: "commit_page_boundary",
        symbols: &["nr_commit_page_boundary"],
        requirement: "S-R3",
        kind: "pagination",
    },
    AbiEntry {
        entry: "invalidate_layout",
        symbols: &["nr_invalidate_layout"],
        requirement: "S-R3",
        kind: "pagination",
    },
    AbiEntry {
        entry: "subscribe_events",
        symbols: &["nr_subscribe_events"],
        requirement: "S-R5",
        kind: "events",
    },
    AbiEntry {
        entry: "cancel_request",
        symbols: &["nr_cancel_request"],
        requirement: "S-R5",
        kind: "events",
    },
];

/// 12 个基线导出符号（扁平清单，便于静态检查）。
pub const BASELINE_SYMBOLS: [&str; ABI_SYMBOL_COUNT] = [
    "nr_engine_create",
    "nr_engine_destroy",
    "nr_open_book",
    "nr_read_window",
    "nr_release_buffer",
    "nr_get_book_batch",
    "nr_get_chapter_batch",
    "nr_save_position",
    "nr_commit_page_boundary",
    "nr_invalidate_layout",
    "nr_subscribe_events",
    "nr_cancel_request",
];

/// 非基线扩展入口（结果记录与 JNI 适配使用；不影响基线完整性）。
pub const EXTENSION_SYMBOLS: [&str; 8] = [
    "nr_abi_version",
    "nr_layout_page",
    "nr_get_metrics",
    "nr_last_error_message",
    "nr_start_batch_request",
    "nr_advance_request",
    "nr_engine_snapshot_json",
    "nr_open_external_book",
];

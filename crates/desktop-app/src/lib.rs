//! `desktop-app`：novel-reader V1 桌面应用层（Rust + Slint）。
//!
//! # 分层
//!
//! - [`app_state`]：应用唯一状态源与「Rust → Slint」状态投影（`Screen` / `UiSnapshot`）；
//! - [`bridge`]：后台结果切回 Slint UI 线程的机制（`slint::invoke_from_event_loop`）；
//! - [`shelf`]：书架后台读取（只调用冻结核心的存储层）；
//! - [`import`]：TXT 导入链路（文件选择 → 校验 → 来源指纹 → 落库 → 可读结果）；
//! - [`input`]：输入意图映射（键盘按键 / 鼠标滚轮 → 整页翻页；输入框聚焦时不翻页）；
//! - [`source_status`]：来源可用性（失效 / 内容变化）与失效重新关联；
//! - [`layout`]：**版式常量的唯一来源**，Rust 排版与 Slint 渲染共用；
//! - [`reflow`]：排版参数派生（窗口映射后的签名 + 测量/渲染共用参数）与重排入口；
//! - [`pagination`]：桌面分页测量（整页断行）与页边界提交（走冻结核心）；
//! - [`page_cache`]：有界页面缓存（当前页 + 前一页 + 后三页预读）；
//! - [`chapters`]：章节目录（核心扫描 + `chapters` 表缓存）与章节跳转落点；
//! - [`progress`]：进度保存 / 读取 / 重启恢复（`ContentPosition` 为唯一身份）；
//! - [`bookmarks`]：书签增删改与跳转（不绑定页码）；
//! - [`settings`]：全局阅读设置（排版设置表 + 交互模式；V1 只有一套全局设置）；
//! - [`ui`]：Slint 绑定（`desktop/ui/app.slint` 是唯一 UI 定义来源）。
//!
//! # 边界（DT1/DT2 禁止事项）
//!
//! - UI 层不得实现编码识别、章节识别、分页边界、位置校验、书签规则；这些语义一律来自
//!   冻结核心 `reading-core` / `reading-source` / `reading-storage`；
//! - 不使用任何私有 Slint 文本 API（由 `scripts/verify-desktop-scope.py` 强制）；
//! - 不注入模拟书库、合成文本源或空 External 数据源：书架为空时如实显示空态；
//! - 导入与重新关联对用户原始文件**只读**：不移动、不重命名、不删除。

pub mod app_state;
pub mod background_ops;
pub mod batch_import;
pub mod bookmarks;
pub mod bridge;
pub mod category_ops;
pub mod category_tree;
pub mod chapters;
pub mod encoded_source;
pub mod encoding;
pub mod history_view;
pub mod import;
pub mod input;
pub mod layout;
pub mod page_cache;
pub mod pagination;
pub mod progress;
pub mod reflow;
pub mod settings;
pub mod shelf;
pub mod shelf_ops;
pub mod source_status;
pub mod theme_derive;
pub mod ui;

pub use app_state::{
    AppState, ApplyOutcome, BackgroundPayload, BackgroundResult, ChoiceRow, DialogView, ErrorView,
    ImportView, MarkRow, ReaderView, Screen, SettingsView, ShelfRequest, ShelfRow, ShelfView,
    ShortcutEntryView, StepRowView, TocRow, UiSnapshot,
};
pub use bookmarks::{
    add as add_bookmark, default_title as default_mark_title, list as list_bookmarks,
    remove as remove_bookmark, rename as rename_bookmark, target as mark_target, MarkError,
    MarkTarget, MARKS_PER_PAGE,
};
pub use bridge::{JobChannel, UiBridge, UiExecutor};
pub use chapters::{
    ensure_toc, jump_target, load_cached_toc, persist_toc, scan_toc, ChapterError, ChapterToc,
    ScanEvidence, TocEntry, TocOrigin, TocReport,
};
pub use import::{
    import_path, picker_from_env, run_import_flow, CancellingFilePicker, FilePicker, ImportError,
    ImportErrorKind, ImportFlow, ImportReport, SelectedFile, StaticFilePicker, SystemFilePicker,
};
pub use layout::{ContentArea, LayoutMetrics};
pub use progress::{
    capture as capture_position, load as load_progress, restore as restore_position_report,
    save as save_progress, ProgressError, ProgressTracker, RestoreReport, SaveReport,
    SavedPosition,
};
pub use settings::{
    display_rows as settings_display_rows, load as load_settings, save as save_settings,
    view_for as settings_view_for, FieldKind, GlobalSettings, SettingsError, SettingsField,
    SettingsOrigin, SettingsRowView, SETTINGS_FIELDS,
};
pub use shelf::{default_db_path, load_shelf, shelf_job};
pub use shelf_ops::{
    clear_index, counts as shelf_counts, index_clear_preview, purge_book, relink_all, remove_book,
    rename_book, IndexClearPreview,
};
pub use source_status::{
    relink_flow, shelf_job_with_source_check, SourceCheck, SourceState, DEFAULT_RELINK_DECISION,
    LABEL_MISSING, LABEL_NEEDS_REINDEX,
};
pub use ui::run;

//! Slint UI 绑定：UI 定义的唯一来源是 `desktop/ui/app.slint`。
//!
//! `slint!` 宏按**相对本文件**的路径解析 `../../desktop/ui/app.slint`，因此不存在
//! 「内联副本 + 文件副本」两份定义；`scripts/verify-desktop-scope.py` 还会禁止
//! `crates/desktop-app/` 下出现任何 `.slint` 文件。
//!
//! 本模块是唯一的双向通道：
//!
//! - **Rust → Slint**：`apply_layout_metrics` / `apply_palette`（版式与配色令牌注入）与
//!   [`apply_snapshot`]（状态投影）只写 Slint 属性，不在 UI 里保留任何业务状态；
//! - **Slint → Rust**：回调只上报意图（序号/键/增量/id），页面语义与业务判断全部回到
//!   `app_state`、`settings`、`shelf_ops` 与冻结核心；后台结果经 `bridge` 回到 UI 线程后再投影；
//! - 程序化驱动（`NR_*` 环境变量）走与界面回调**同一批函数**，不另建一条旁路。
//!
//! v0.2.0（设计 `docs/design/novel-reader-desktop-redesign.md`）：书架表格/卡片与搜索排序、
//! 阅读页工具条 + 进度条跳转、覆盖式目录/书签面板（含搜索与行内确认）、选项/快捷键/确认
//! 弹窗、Toast、沉浸自动隐藏工具条。

slint::slint! {
    // `UiMetrics` / `Skin` / `Screens` 是 `app.slint` 中的全局单例，必须显式 export
    // 才能在 Rust 侧以 `ui.global::<UiMetrics>()` / `Screens.shelf` 引用。
    export { AppWindow, BookRow, ChoiceRow, MarkRow, Screens, SettingsRow, ShortcutEntry, Skin, StepRow, TocRow, UiMetrics } from "../../desktop/ui/app.slint";
}

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use slint::{ModelRc, VecModel};

use crate::app_state::{
    self, AppState, ChoiceRow as ChoiceRowData, ImportView, Screen, ShortcutEntryView, StepRowView,
    TocRow as TocRowData, UiSnapshot,
};
use crate::bookmarks;
use crate::bridge::{JobChannel, SlintEventLoopExecutor, UiBridge};
use crate::chapters::{self, ChapterToc};
use crate::import::{self, FilePicker};
use crate::input::{self, PageTurn};
use crate::layout::{self, LayoutMetrics};
use crate::pagination::{open_book, PageOutcome, ReaderRuntime};
use crate::progress::{self, ProgressTracker, SavedPosition};
use crate::reflow::{MappedViewport, PlatformFontMetrics};
use crate::settings::{self, GlobalSettings};
use crate::shelf;
use crate::shelf_ops;
use crate::source_status;

/// 启动页面环境变量（程序化验证用）：`shelf` / `reader` / `settings` / `importing` / `error`。
pub const START_SCREEN_ENV: &str = "NR_START_SCREEN";
/// 自动退出毫秒数（程序化验证用；未设置或 0 表示不自动退出）。
pub const EXIT_AFTER_MS_ENV: &str = "NR_EXIT_AFTER_MS";
/// 状态日志开关：`1` 时每次投影向 stdout 输出一行 `NR-STATE ...`。
pub const STATE_LOG_ENV: &str = "NR_STATE_LOG";
/// 阅读目标（程序化验证用）：书籍标识，或 `first` 表示书架第一本。
pub const READER_BOOK_ENV: &str = "NR_READER_BOOK";
/// 启动后自动翻页次数（正数向后、负数向前；DT3 分页证据用，**不**走用户入口）。
pub const READER_TURNS_ENV: &str = "NR_READER_TURNS";
/// 启动后设置字号（像素；程序化验证用，触发一次真实重排）。
pub const READER_FONT_ENV: &str = "NR_READER_FONT";
/// 启动后展开阅读工具条（`1`；程序化验证用，触发一次视口高度变化重排）。
pub const READER_MENU_ENV: &str = "NR_READER_MENU";
/// 启动后以**用户入口**翻页 n 页（正数向后、负数向前），并像用户一样保存进度（DT4）。
pub const UI_TURNS_ENV: &str = "NR_TURNS_UI";
/// 启动后跳转到章节序号（0 基；DT4）。
pub const CHAPTER_JUMP_ENV: &str = "NR_CHAPTER_JUMP";
/// 启动后新增书签（值为书签名；空值用默认名，DT4）。
pub const MARK_ADD_ENV: &str = "NR_MARK_ADD";
/// 启动后跳转到书签 id（DT4）。
pub const MARK_JUMP_ENV: &str = "NR_MARK_JUMP";
/// 启动后重命名书签（`id:新名称`，DT4）。
pub const MARK_RENAME_ENV: &str = "NR_MARK_RENAME";
/// 启动后删除书签（书签 id，DT4；走行内两段确认入口）。
pub const MARK_DELETE_ENV: &str = "NR_MARK_DELETE";
/// 启动后应用全局设置（`key=value,key=value`；同时持久化，DT4）。
pub const SETTINGS_SET_ENV: &str = "NR_SETTINGS_SET";
/// 启动后**手动覆盖内容编码**（值为编码展示名，如 `GBK`；与选项弹窗的「应用」同一入口）。
pub const ENCODING_SELECT_ENV: &str = "NR_ENCODING_SELECT";
/// 书架行「阅读」入口（值为书籍 id；与书架整行点击**同一函数**）。
pub const SHELF_OPEN_ENV: &str = "NR_SHELF_OPEN";
/// 目录滚动条定位（`0..1` 比例；与界面滚动条**同一回调**）。
pub const TOC_SEEK_ENV: &str = "NR_TOC_SEEK";
/// 目录滚轮（逻辑像素 delta-y；与界面滚轮**同一回调**）。
pub const TOC_WHEEL_ENV: &str = "NR_TOC_WHEEL";
/// 目录行「跳转」（值为章节序号；与行点击**同一回调**）。
pub const TOC_JUMP_ENV: &str = "NR_TOC_JUMP";
/// 阅读进度条定位（`0..1` 比例；与界面进度条**同一回调**）。
pub const PROGRESS_SEEK_ENV: &str = "NR_PROGRESS_SEEK";
/// 启动后进入沉浸阅读（`1` = 进入；界面「沉浸阅读」按钮的等价入口）。
pub const IMMERSIVE_ENV: &str = "NR_IMMERSIVE";
/// 沉浸态顶部悬停（`1` = 悬停进入 / `0` = 悬停离开；界面顶部悬停区的等价入口）。
pub const IMMERSIVE_HOVER_ENV: &str = "NR_IMMERSIVE_HOVER";
/// 启动后保存一次进度（用户「保存进度」按钮的等价入口，DT4）。
pub const SAVE_PROGRESS_ENV: &str = "NR_SAVE_PROGRESS";
/// 展开面板（`toc` / `marks`；程序化验证用，DT4）。
pub const PANEL_ENV: &str = "NR_PANEL";
/// 面板搜索（目录 / 书签面板的搜索框文本；与界面输入**同一入口**）。
pub const PANEL_SEARCH_ENV: &str = "NR_PANEL_SEARCH";
/// 书架搜索（与界面搜索框**同一入口**）。
pub const SHELF_SEARCH_ENV: &str = "NR_SHELF_SEARCH";
/// 书架视图（`table` / `cards`；与界面视图切换**同一入口**）。
pub const SHELF_VIEW_ENV: &str = "NR_SHELF_VIEW";
/// 书架排序（`recent` / `title` / `imported`；与界面排序字段**同一入口**）。
pub const SHELF_SORT_ENV: &str = "NR_SHELF_SORT";
/// 展开行级菜单（行下标；与行尾「更多」**同一入口**）。
pub const SHELF_MENU_ENV: &str = "NR_SHELF_MENU";
/// 执行行级动作（`relink:id` / `rename:id` / `remove:id` / `purge:id`；与行级菜单**同一入口**）。
pub const SHELF_ACTION_ENV: &str = "NR_SHELF_ACTION";
/// 确认当前弹窗（`1` = 勾选项打开时确认；`0` = 勾选项关闭时确认）。
pub const DIALOG_CONFIRM_ENV: &str = "NR_DIALOG_CONFIRM";
/// 取消当前弹窗（`1`）。
pub const DIALOG_CANCEL_ENV: &str = "NR_DIALOG_CANCEL";
/// 打开快捷键说明弹窗（`1`；顶栏「?」的等价入口）。
pub const SHORTCUTS_ENV: &str = "NR_SHORTCUTS";
/// 启动后清理可重建索引（`1`；设置页「清理…」的等价入口，走确认弹窗）。
pub const CLEAR_INDEX_ENV: &str = "NR_CLEAR_INDEX";
/// 程序化验证（DT8）：按键序列（便携名称，逗号分隔；如 `page-down,page-up`）。
pub const KEY_EVENTS_ENV: &str = "NR_KEY_EVENTS";
/// 程序化验证（DT8）：滚轮增量序列（逻辑像素，逗号分隔；正值向下滚）。
pub const WHEEL_EVENTS_ENV: &str = "NR_WHEEL_EVENTS";
/// 程序化验证（DT8）：模拟「文本输入框持有焦点」（`1`），用于「输入框聚焦不翻页」负向用例。
pub const TEXT_FOCUS_ENV: &str = "NR_TEXT_FOCUS";
/// 首屏对齐延迟（毫秒）：窗口映射完成后才取正文区域尺寸与字体度量。
///
/// 取值需覆盖「窗口映射 → 窗口管理器套用首选尺寸（1200×800）」的时间窗；
/// 过早读取会拿到初始最小尺寸，导致首屏按错误尺寸分页（随后的
/// `viewport-changed` 会纠正，但首屏证据会失真）。
pub const STARTUP_ALIGN_MS: u64 = 600;
/// 目录面板一次显示的行数（界面不申请无限长列表）。
pub const TOC_WINDOW: usize = 12;
/// Toast 自动消失毫秒数。
pub const TOAST_MS: u64 = 3_000;
/// 沉浸态浮出工具条的淡出延迟（指针离开后；一次性定时器，不产生持续出帧）。
pub const FLOATING_HIDE_MS: u64 = 1_500;

/// 快捷键说明（tooltip 与快捷键面板的**同一来源**；`implemented=false` 为建议项）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortcutSpec {
    /// 动作名。
    pub action: &'static str,
    /// 按键。
    pub keys: &'static str,
    /// 是否已实现（未实现不得写进 tooltip）。
    pub implemented: bool,
}

/// 已实现：上一页。
pub const SHORTCUT_PREV: &str = "PageUp / ↑ / ←";
/// 已实现：下一页。
pub const SHORTCUT_NEXT: &str = "PageDown / ↓ / →";
/// 已实现：整页翻页。
pub const SHORTCUT_WHEEL: &str = "鼠标滚轮";
/// 已实现：退出沉浸。
pub const SHORTCUT_ESCAPE: &str = "Esc";

/// 快捷键清单（只承诺核心已实现的按键；建议项如实标注）。
pub const SHORTCUTS: &[ShortcutSpec] = &[
    ShortcutSpec {
        action: "上一页",
        keys: SHORTCUT_PREV,
        implemented: true,
    },
    ShortcutSpec {
        action: "下一页",
        keys: SHORTCUT_NEXT,
        implemented: true,
    },
    ShortcutSpec {
        action: "整页翻页",
        keys: SHORTCUT_WHEEL,
        implemented: true,
    },
    ShortcutSpec {
        action: "退出沉浸",
        keys: SHORTCUT_ESCAPE,
        implemented: true,
    },
    ShortcutSpec {
        action: "目录",
        keys: "T",
        implemented: false,
    },
    ShortcutSpec {
        action: "书签",
        keys: "B",
        implemented: false,
    },
    ShortcutSpec {
        action: "保存进度",
        keys: "Ctrl+S",
        implemented: false,
    },
    ShortcutSpec {
        action: "沉浸阅读",
        keys: "F11",
        implemented: false,
    },
];

/// 阅读会话：分页运行时 + 章节目录（章节/进度/书签都挂在这里）。
struct ReadingSession {
    runtime: ReaderRuntime,
    toc: ChapterToc,
}

impl ReadingSession {
    /// 当前页起始字节（未分页时为 `None`）。
    fn current_page_start(&self) -> Option<u64> {
        self.runtime.page().map(|page| page.start_byte)
    }

    /// 当前章节序号（区间判定由核心给出）。
    fn current_chapter(&self) -> u32 {
        match self.current_page_start() {
            Some(offset) => self.toc.sequence_at_offset(offset),
            None => 0,
        }
    }

    /// 章节标签（「第 N/M 章 · 标题」；文案由 Rust 生成）。
    fn chapter_label(&self) -> String {
        let current = self.current_chapter();
        let total = self.toc.len();
        let title = self.toc.title_at(current);
        format!("第 {}/{} 章 · {title}", current + 1, total.max(1))
    }

    /// 工具条居中标题（「书名 · 章节」；文案由 Rust 生成）。
    fn heading(&self) -> String {
        format!(
            "{} · {}",
            self.runtime.book().title,
            self.chapter_label()
        )
    }

    /// 正文列几何（**页面局部坐标**；测量与渲染同口径）。
    ///
    /// 说明：测量侧（`layout_signature`）用 `text_column` 的宽度断行；渲染侧直接用本函数
    /// 的结果定位，因此不存在「测量宽度与渲染宽度不一致」的空间。
    fn column_geometry(&self) -> (u32, u32, u32, u32) {
        let metrics = self.runtime.layout_metrics();
        let viewport = self.runtime.viewport();
        let area = metrics.content_area(viewport.width, viewport.height, self.runtime.menu_open());
        let column = metrics.text_column(&area, self.runtime.typography().font_size_px());
        let y_local = column.y.saturating_sub(metrics.top_bar_height);
        (column.x, y_local, column.width, column.height)
    }

    /// 跳转到字节偏移（章节跳转 / 书签跳转 / 进度恢复 / 进度条跳转共用）。
    fn jump_to(&mut self, byte_offset: u64) -> PageOutcome {
        self.runtime.goto_byte(byte_offset)
    }
}

fn state_log_enabled() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| {
        matches!(
            std::env::var(STATE_LOG_ENV).as_deref(),
            Ok("1") | Ok("true") | Ok("yes")
        )
    })
}

/// 输出一行结构化证据（仅在 `NR_STATE_LOG=1` 时）。
fn log_line(line: &str) {
    if state_log_enabled() {
        println!("{line}");
        let _ = std::io::stdout().flush();
    }
}

/// 取锁（中毒时也继续，避免把一次 panic 变成永久卡死）。
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// 启动页面（未设置/非法时返回 `None`，由调用方保留默认书架页）。
fn start_screen_from_env() -> Option<Screen> {
    let value = std::env::var(START_SCREEN_ENV).ok()?;
    Screen::ALL
        .into_iter()
        .find(|screen| screen.slint_name() == value.trim())
}

fn exit_after_ms_from_env() -> Option<u64> {
    let value = std::env::var(EXIT_AFTER_MS_ENV).ok()?;
    value.trim().parse::<u64>().ok().filter(|ms| *ms > 0)
}

/// 把版式常量注入 Slint：数值唯一来源是 [`LayoutMetrics`]（`layout.rs`）。
///
/// 顺序与 `desktop/ui/app.slint` 的 `UiMetrics` 声明一致，字段集合由
/// `tests/layout_constants.rs::layout_constants_single_source` 核对。
pub fn apply_layout_metrics(ui: &AppWindow, metrics: &LayoutMetrics) {
    let globals = ui.global::<UiMetrics>();
    globals.set_top_bar_height(metrics.top_bar_height as f32);
    globals.set_toolbar_height(metrics.toolbar_height as f32);
    globals.set_toolbar_height_compact(metrics.toolbar_height_compact as f32);
    globals.set_status_bar_height(metrics.status_bar_height as f32);
    globals.set_progress_bar_height(metrics.progress_bar_height as f32);
    globals.set_progress_bar_hover_height(metrics.progress_bar_hover_height as f32);
    globals.set_content_padding_h(metrics.content_padding_h as f32);
    globals.set_content_padding_v(metrics.content_padding_v as f32);
    globals.set_content_padding_h_wide(metrics.content_padding_h_wide as f32);
    globals.set_content_padding_compact(metrics.content_padding_compact as f32);
    globals.set_breakpoint_wide(metrics.breakpoint_wide as f32);
    globals.set_breakpoint_compact(metrics.breakpoint_compact as f32);
    globals.set_breakpoint_narrow(metrics.breakpoint_narrow as f32);
    globals.set_default_window_width(metrics.default_window_width as f32);
    globals.set_default_window_height(metrics.default_window_height as f32);
    globals.set_min_window_width(metrics.min_window_width as f32);
    globals.set_min_window_height(metrics.min_window_height as f32);
    globals.set_title_font_size(metrics.title_font_size as f32);
    globals.set_heading_font_size(metrics.heading_font_size as f32);
    globals.set_body_font_size(metrics.body_font_size as f32);
    globals.set_ui_font_size(metrics.ui_font_size as f32);
    globals.set_label_font_size(metrics.label_font_size as f32);
    globals.set_caption_font_size(metrics.caption_font_size as f32);
    globals.set_mono_font_size(metrics.mono_font_size as f32);
    globals.set_tag_font_size(metrics.tag_font_size as f32);
    globals.set_nav_tab_min_width(metrics.nav_tab_min_width as f32);
    globals.set_nav_tab_height(metrics.nav_tab_height as f32);
    globals.set_nav_indicator_height(metrics.nav_indicator_height as f32);
    globals.set_icon_button_size(metrics.icon_button_size as f32);
    globals.set_icon_button_compact_size(metrics.icon_button_compact_size as f32);
    globals.set_button_height(metrics.button_height as f32);
    globals.set_button_height_compact(metrics.button_height_compact as f32);
    globals.set_focus_ring_width(metrics.focus_ring_width as f32);
    globals.set_search_field_width(metrics.search_field_width as f32);
    globals.set_sort_field_width(metrics.sort_field_width as f32);
    globals.set_tag_height(metrics.tag_height as f32);
    globals.set_switch_width(metrics.switch_width as f32);
    globals.set_switch_height(metrics.switch_height as f32);
    globals.set_radio_size(metrics.radio_size as f32);
    globals.set_slider_handle_size(metrics.slider_handle_size as f32);
    globals.set_slider_track_height(metrics.slider_track_height as f32);
    globals.set_list_row_height(metrics.list_row_height as f32);
    globals.set_table_row_height(metrics.table_row_height as f32);
    globals.set_table_header_height(metrics.table_header_height as f32);
    globals.set_column_status_width(metrics.column_status_width as f32);
    globals.set_column_progress_width(metrics.column_progress_width as f32);
    globals.set_column_recent_width(metrics.column_recent_width as f32);
    globals.set_column_actions_width(metrics.column_actions_width as f32);
    globals.set_page_header_height(metrics.page_header_height as f32);
    globals.set_panel_header_height(metrics.panel_header_height as f32);
    globals.set_panel_hint_height(metrics.panel_hint_height as f32);
    globals.set_scrollbar_width(metrics.scrollbar_width as f32);
    globals.set_scrollbar_gutter(metrics.scrollbar_gutter as f32);
    globals.set_scrollbar_min_handle(metrics.scrollbar_min_handle as f32);
    globals.set_toast_radius(metrics.toast_radius as f32);
    globals.set_toast_width(metrics.toast_width as f32);
    globals.set_toast_height(metrics.toast_height as f32);
    globals.set_modal_width_confirm(metrics.modal_width_confirm as f32);
    globals.set_modal_width_settings(metrics.modal_width_settings as f32);
    globals.set_card_width(metrics.card_width as f32);
    globals.set_card_height(metrics.card_height as f32);
    globals.set_cover_size(metrics.cover_size as f32);
    globals.set_empty_icon_size(metrics.empty_icon_size as f32);
    globals.set_space_2(metrics.space_2 as f32);
    globals.set_space_4(metrics.space_4 as f32);
    globals.set_space_8(metrics.space_8 as f32);
    globals.set_space_12(metrics.space_12 as f32);
    globals.set_space_16(metrics.space_16 as f32);
    globals.set_space_24(metrics.space_24 as f32);
    globals.set_space_32(metrics.space_32 as f32);
    globals.set_space_48(metrics.space_48 as f32);
    globals.set_hairline(metrics.hairline as f32);
    globals.set_brand_font_size(metrics.brand_font_size as f32);
    globals.set_icon_size(metrics.icon_size as f32);
    globals.set_cover_font_size(metrics.cover_font_size as f32);
    globals.set_card_title_font_size(metrics.card_title_font_size as f32);
    globals.set_search_field_panel_width(metrics.search_field_panel_width as f32);
}

/// 把配色令牌注入 Slint（按当前主题选择明/暗一套；`Skin` 的属性集合与
/// `layout.rs::PALETTE_FIELDS` 一致，由 `tests/layout_constants.rs` 核对）。
pub fn apply_palette(ui: &AppWindow, dark: bool) {
    let globals = ui.global::<Skin>();
    for field in layout::PALETTE_FIELDS {
        let (r, g, b, a) = if dark {
            field.value.dark
        } else {
            field.value.light
        };
        let color = slint::Color::from_argb_u8(a, r, g, b);
        match field.slint_name {
            "background" => globals.set_background(color),
            "layer-01" => globals.set_layer_01(color),
            "layer-02" => globals.set_layer_02(color),
            "field-01" => globals.set_field_01(color),
            "border-subtle" => globals.set_border_subtle(color),
            "border-strong" => globals.set_border_strong(color),
            "text-primary" => globals.set_text_primary(color),
            "text-secondary" => globals.set_text_secondary(color),
            "text-placeholder" => globals.set_text_placeholder(color),
            "text-disabled" => globals.set_text_disabled(color),
            "primary" => globals.set_primary(color),
            "primary-hover" => globals.set_primary_hover(color),
            "primary-pressed" => globals.set_primary_pressed(color),
            "primary-soft" => globals.set_primary_soft(color),
            "primary-border" => globals.set_primary_border(color),
            "primary-strong" => globals.set_primary_strong(color),
            "on-primary" => globals.set_on_primary(color),
            "focus" => globals.set_focus(color),
            "header" => globals.set_header(color),
            "header-text" => globals.set_header_text(color),
            "header-hover" => globals.set_header_hover(color),
            "support-success" => globals.set_support_success(color),
            "success-soft" => globals.set_success_soft(color),
            "support-warning" => globals.set_support_warning(color),
            "warning-soft" => globals.set_warning_soft(color),
            "support-error" => globals.set_support_error(color),
            "error-soft" => globals.set_error_soft(color),
            "scrollbar-track" => globals.set_scrollbar_track(color),
            "scrollbar-handle" => globals.set_scrollbar_handle(color),
            "control-track" => globals.set_control_track(color),
            "disabled-bg" => globals.set_disabled_bg(color),
            "disabled-border" => globals.set_disabled_border(color),
            "disabled-text" => globals.set_disabled_text(color),
            "overlay" => globals.set_overlay(color),
            "toast-bg" => globals.set_toast_bg(color),
            "toast-text" => globals.set_toast_text(color),
            "reading-text" => globals.set_reading_text(color),
            "cover-bg" => globals.set_cover_bg(color),
            _ => {}
        }
    }
}

/// 一次状态投影：把 [`UiSnapshot`] 写入 Slint 属性（UI 不持有业务状态）。
pub fn apply_snapshot(ui: &AppWindow, snapshot: &UiSnapshot) {
    ui.set_current_screen(snapshot.screen.index());
    ui.set_app_version(snapshot.app_version.as_str().into());
    ui.set_theme_dark(snapshot.theme_dark);
    ui.set_status_text(snapshot.status_text.as_str().into());
    ui.set_placeholder_status(snapshot.placeholder_status.as_str().into());

    // ---- 状态栏 ----
    ui.set_status_chapter(snapshot.reader.chapter_label.as_str().into());
    ui.set_status_page(snapshot.reader.page_label.as_str().into());
    ui.set_status_percent(snapshot.reader.progress_percent.as_str().into());
    ui.set_status_meta(snapshot.reader.encoding_name.as_str().into());
    ui.set_status_reflow(snapshot.reader.reflow_status.as_str().into());

    // ---- 阅读页 ----
    ui.set_reader_heading(snapshot.reader.heading.as_str().into());
    ui.set_reader_toolbar_open(snapshot.reader.toolbar_open);
    ui.set_reader_immersive(snapshot.reader.immersive);
    ui.set_reader_floating_toolbar(snapshot.reader.floating_toolbar);
    ui.set_reader_gesture_mode(snapshot.reader.gesture_mode);
    let page_lines: Vec<slint::SharedString> = snapshot
        .reader
        .page_text
        .split('\n')
        .map(|line| line.into())
        .collect();
    ui.set_reader_page_lines(ModelRc::new(VecModel::from(page_lines)));
    ui.set_reader_column_x(snapshot.reader.column_x as f32);
    ui.set_reader_column_y(snapshot.reader.column_y as f32);
    ui.set_reader_column_width(snapshot.reader.column_width as f32);
    ui.set_reader_column_height(snapshot.reader.column_height as f32);
    ui.set_reader_progress_permille(snapshot.reader.progress_permille as i32);
    ui.set_reader_shortcut_prev(SHORTCUT_PREV.into());
    ui.set_reader_shortcut_next(SHORTCUT_NEXT.into());
    ui.set_reader_shortcut_immersive(SHORTCUT_ESCAPE.into());
    // 目录面板
    ui.set_reader_toc_open(snapshot.reader.toc_open);
    let toc_rows: Vec<TocRow> = snapshot
        .reader
        .toc_rows
        .iter()
        .map(|row| TocRow {
            sequence: row.sequence.as_str().into(),
            title: row.title.as_str().into(),
            meta: row.meta.as_str().into(),
            current: row.current,
        })
        .collect();
    ui.set_reader_toc_rows(ModelRc::new(VecModel::from(toc_rows)));
    ui.set_reader_toc_ratio(snapshot.reader.toc_ratio_permille as f32 / 1_000.0);
    ui.set_reader_toc_handle_ratio(snapshot.reader.toc_handle_permille as f32 / 1_000.0);
    ui.set_reader_toc_count(snapshot.reader.toc_count_label.as_str().into());
    ui.set_reader_toc_readout(snapshot.reader.toc_readout.as_str().into());
    ui.set_reader_toc_search(snapshot.reader.toc_search.as_str().into());
    // 书签面板
    ui.set_reader_marks_open(snapshot.reader.marks_open);
    let mark_rows: Vec<MarkRow> = snapshot
        .reader
        .mark_rows
        .iter()
        .map(|row| MarkRow {
            id: row.id.as_str().into(),
            chapter: row.chapter.as_str().into(),
            title: row.title.as_str().into(),
            position_label: row.position_label.as_str().into(),
            current_chapter: row.current_chapter,
            selected: row.selected,
            delete_pending: row.delete_pending,
        })
        .collect();
    ui.set_reader_mark_rows(ModelRc::new(VecModel::from(mark_rows)));
    ui.set_reader_mark_count(snapshot.reader.mark_count_label.as_str().into());
    ui.set_reader_mark_readout(snapshot.reader.mark_readout.as_str().into());
    ui.set_reader_rename_text(snapshot.reader.mark_rename_text.as_str().into());
    ui.set_reader_mark_search(snapshot.reader.mark_search.as_str().into());

    // ---- 弹窗 ----
    ui.set_dialog(snapshot.dialog.kind);
    ui.set_confirm_title(snapshot.dialog.confirm_title.as_str().into());
    ui.set_confirm_body(snapshot.dialog.confirm_body.as_str().into());
    ui.set_confirm_note(snapshot.dialog.confirm_note.as_str().into());
    ui.set_confirm_checkbox_label(snapshot.dialog.confirm_checkbox_label.as_str().into());
    ui.set_confirm_label(snapshot.dialog.confirm_label.as_str().into());
    ui.set_confirm_danger(snapshot.dialog.confirm_danger);
    ui.set_choice_title(snapshot.dialog.choice_title.as_str().into());
    ui.set_choice_meta(snapshot.dialog.choice_meta.as_str().into());
    let choice_rows: Vec<ChoiceRow> = snapshot
        .dialog
        .choice_rows
        .iter()
        .map(|row| ChoiceRow {
            name: row.name.as_str().into(),
            current: row.current,
        })
        .collect();
    ui.set_choice_rows(ModelRc::new(VecModel::from(choice_rows)));
    ui.set_choice_current_index(snapshot.dialog.choice_current_index);
    let shortcut_rows: Vec<ShortcutEntry> = snapshot
        .dialog
        .shortcuts
        .iter()
        .map(|row| ShortcutEntry {
            action: row.action.as_str().into(),
            keys: row.keys.as_str().into(),
            available: row.available,
        })
        .collect();
    ui.set_shortcut_rows(ModelRc::new(VecModel::from(shortcut_rows)));
    ui.set_rename_title(snapshot.dialog.rename_title.as_str().into());
    ui.set_rename_value(snapshot.dialog.rename_value.as_str().into());

    // ---- Toast ----
    ui.set_toast_visible(snapshot.toast.visible);
    ui.set_toast_text(snapshot.toast.text.as_str().into());
    ui.set_toast_action(snapshot.toast.action_label.as_str().into());

    // ---- 书架 ----
    let book_rows: Vec<BookRow> = snapshot
        .shelf
        .rows
        .iter()
        .map(|row| BookRow {
            id: row.id.as_str().into(),
            title: row.title.as_str().into(),
            initial: row.initial.as_str().into(),
            meta: row.meta.as_str().into(),
            status: row.status.as_str().into(),
            status_tone: row.status_tone,
            progress: row.progress.as_str().into(),
            percent: row.percent,
            recent: row.recent.as_str().into(),
            needs_relink: row.needs_relink,
            can_open: row.can_open,
        })
        .collect();
    ui.set_book_rows(ModelRc::new(VecModel::from(book_rows)));
    ui.set_shelf_empty(snapshot.shelf.empty);
    ui.set_shelf_empty_title(snapshot.shelf.empty_title.as_str().into());
    ui.set_shelf_empty_desc(snapshot.shelf.empty_desc.as_str().into());
    let empty_notes: Vec<slint::SharedString> = snapshot
        .shelf
        .empty_notes
        .iter()
        .map(|note| note.as_str().into())
        .collect();
    ui.set_shelf_empty_notes(ModelRc::new(VecModel::from(empty_notes)));
    ui.set_shelf_view(snapshot.shelf.view);
    ui.set_shelf_search(snapshot.shelf.search.as_str().into());
    ui.set_shelf_sort_value(snapshot.shelf.sort_label.as_str().into());
    ui.set_shelf_sort_current(snapshot.shelf.sort_index);
    ui.set_shelf_count(snapshot.shelf.count_label.as_str().into());
    ui.set_shelf_notice(snapshot.shelf.notice);
    ui.set_shelf_notice_title(snapshot.shelf.notice_title.as_str().into());
    ui.set_shelf_notice_body(snapshot.shelf.notice_body.as_str().into());
    ui.set_shelf_menu_index(snapshot.shelf.menu_index);

    // ---- 设置 ----
    let settings_rows: Vec<SettingsRow> = snapshot
        .settings
        .rows
        .iter()
        .map(|row| SettingsRow {
            key: row.key.as_str().into(),
            label: row.label.as_str().into(),
            value: row.value.as_str().into(),
            note: row.note.as_str().into(),
            effective: row.effective,
            kind: row.kind,
            section: row.section,
            value_min: row.value_min,
            value_max: row.value_max,
            value_step: row.value_step,
            value_current: row.value_current,
            options: ModelRc::new(VecModel::from(
                row.options
                    .iter()
                    .map(|option| slint::SharedString::from(option.as_str()))
                    .collect::<Vec<_>>(),
            )),
            current_index: row.current_index,
            checked: row.checked,
            mono: row.mono,
            range_label: row.range_label.as_str().into(),
        })
        .collect();
    ui.set_settings_rows(ModelRc::new(VecModel::from(settings_rows)));
    ui.set_settings_note(snapshot.settings.note.as_str().into());
    ui.set_settings_content_height(snapshot.settings.content_height as f32);
    ui.set_settings_data_path(snapshot.settings.data_path.as_str().into());

    // ---- 导入 / 错误 ----
    ui.set_import_file(snapshot.import.file.as_str().into());
    ui.set_import_size(snapshot.import.size.as_str().into());
    ui.set_import_percent(snapshot.import.percent);
    let steps: Vec<StepRow> = snapshot
        .import
        .steps
        .iter()
        .map(|step| StepRow {
            label: step.label.as_str().into(),
            state: step.state,
        })
        .collect();
    ui.set_import_steps(ModelRc::new(VecModel::from(steps)));
    ui.set_import_note(snapshot.import.note.as_str().into());
    ui.set_error_title(snapshot.error.title.as_str().into());
    ui.set_error_reason(snapshot.error.reason.as_str().into());
    ui.set_error_details(snapshot.error.details.as_str().into());

    if state_log_enabled() {
        let row_status = snapshot
            .shelf
            .rows
            .first()
            .map(|row| row.status.clone())
            .unwrap_or_default();
        let line = format!(
            "NR-STATE screen={} index={} rows={} menu_open={} theme_dark={} row_status={} \
             chapter={} chapters={} toc_rows={} toc_open={} toc_matches={} marks={} marks_open={} mark_sel={} \
             gesture={} progress={} settings={} encoding={} dialog={} toast={} shelf_view={} shelf_sort={} \
             shelf_search={} settings_h={} status={}",
            snapshot.screen.slint_name(),
            snapshot.screen.index(),
            snapshot.shelf.rows.len(),
            u8::from(snapshot.reader.toolbar_open),
            u8::from(snapshot.theme_dark),
            row_status,
            snapshot.reader.chapter_sequence,
            snapshot.reader.chapter_total,
            snapshot.reader.toc_rows.len(),
            u8::from(snapshot.reader.toc_open),
            snapshot.reader.toc_match_total,
            snapshot.reader.mark_total,
            u8::from(snapshot.reader.marks_open),
            snapshot.reader.mark_selected,
            u8::from(snapshot.reader.gesture_mode),
            snapshot.reader.progress_status.replace(' ', "_"),
            snapshot.settings.rows.len(),
            snapshot.reader.encoding_name.replace(' ', "_"),
            snapshot.dialog.kind,
            u8::from(snapshot.toast.visible),
            snapshot.shelf.view,
            snapshot.shelf.sort_label.replace(' ', "_"),
            snapshot.shelf.search.replace(' ', "_"),
            snapshot.settings.content_height,
            snapshot.status_text
        );
        println!("{line}");
        let _ = std::io::stdout().flush();
    }
}

/// 取当前状态并投影一次（本地交互后调用）。
fn refresh(ui: &AppWindow, state: &Arc<Mutex<AppState>>) {
    let snapshot = lock(state).project();
    apply_snapshot(ui, &snapshot);
}

/// 以「改状态 + 重新投影」的方式处理一次本地交互。
fn mutate_and_refresh(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    mutate: impl FnOnce(&mut AppState),
) {
    {
        let mut guard = lock(state);
        mutate(&mut guard);
    }
    refresh(ui, state);
}

/// 提交一次书架刷新（导入 / 重新关联 / 行级动作后让列表与状态重新来自数据库）。
fn submit_shelf_refresh(state: &Arc<Mutex<AppState>>, channel: &JobChannel, db_path: &Path) {
    let request = lock(state).next_shelf_request(db_path.to_path_buf());
    let _ = channel.submit(request);
}

/// 阅读目标的启动选择（`None` = 不自动打开书籍；`first` = 书架第一本）。
fn reader_selection_from_env() -> Option<String> {
    let value = std::env::var(READER_BOOK_ENV).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

/// 读取整数环境变量（非法/未设置返回 `None`）。
fn env_i64(name: &str) -> Option<i64> {
    std::env::var(name).ok()?.trim().parse::<i64>().ok()
}

/// 读取文本环境变量（未设置/空串返回 `None`）。
fn env_text(name: &str) -> Option<String> {
    let value = std::env::var(name).ok()?;
    if value.trim().is_empty() {
        None
    } else {
        Some(value.trim().to_string())
    }
}

/// 读取浮点环境变量（非法/未设置返回 `None`）。
fn env_f64(name: &str) -> Option<f64> {
    std::env::var(name).ok()?.trim().parse::<f64>().ok()
}

fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name).as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

/// 投影「测量与渲染共用」的排版参数（字号 / 行盒 / 字体族）。
fn apply_reader_typography(
    ui: &AppWindow,
    font_size_px: u32,
    line_pitch_px: u32,
    font_family: &str,
) {
    ui.set_reader_body_font_size(font_size_px as f32);
    ui.set_reader_line_pitch(line_pitch_px as f32);
    ui.set_reader_body_font_family(font_family.into());
}

/// 当前页对应的内容位置（进度保存与书签新增共用；锚点由核心采样）。
fn captured_position(session: &ReadingSession) -> Option<SavedPosition> {
    let page = session.runtime.page()?;
    let chapter_index = session.toc.sequence_at_offset(page.start_byte);
    let book = session.runtime.book();
    progress::capture(&book.path, book.book_id, chapter_index, page.start_byte).ok()
}

/// 把一次分页结果投影到应用状态：正文只来自有限窗口的测量结果（绝不放整本）。
fn project_reader_outcome(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    session: &ReadingSession,
    outcome: &PageOutcome,
    prefix: &str,
) {
    match outcome {
        PageOutcome::Ready(page) => {
            let title = session.runtime.book().title.clone();
            let text = page.text();
            let label = page.label();
            let status = format!(
                "{prefix}：{label} · 字节 {}..{} · 排版世代 {}",
                page.start_byte,
                page.end_byte,
                session.runtime.layout_id()
            );
            let chapter_label = session.chapter_label();
            let heading = session.heading();
            let chapter_title = session.toc.title_at(session.current_chapter());
            let permille = session.runtime.progress_permille();
            let (cx, cy, cw, ch) = session.column_geometry();
            mutate_and_refresh(ui, state, |guard| {
                guard.set_reader_page(title, chapter_title, text, label);
                guard.set_reader_heading(heading);
                guard.set_chapter(
                    session.current_chapter(),
                    session.toc.len(),
                    session.toc.title_at(session.current_chapter()),
                );
                guard.set_chapter_label(chapter_label);
                guard.set_reader_column(cx, cy, cw, ch);
                guard.set_progress_permille(permille);
                guard.set_status(status);
                guard.set_progress_percent(crate::pagination::format_progress_percent(permille));
            });
        }
        other => {
            let status = format!("{prefix}：{}", other.describe());
            mutate_and_refresh(ui, state, |guard| {
                guard.set_status(status);
            });
        }
    }
    let typography = session.runtime.typography();
    apply_reader_typography(
        ui,
        typography.font_size_px(),
        typography.line_pitch_px(),
        &typography.font_family.clone(),
    );
    log_line(&session.runtime.state_log());
}

/// 投影章节目录窗口（有界；按搜索关键词过滤）与当前章节。
fn project_toc(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    session: &ReadingSession,
    window_first: Option<u32>,
) {
    let current = session.current_chapter();
    let total = session.toc.len();
    let search = lock(state).toc_search().to_string();
    // 过滤（有界窗口在过滤后的列表上计算；序号仍是真实章节序号）。
    let matched: Vec<u32> = (0..total as u32)
        .filter(|sequence| {
            let title = session.toc.title_at(*sequence);
            search.trim().is_empty() || title.to_lowercase().contains(&search.trim().to_lowercase())
        })
        .collect();
    let match_total = matched.len();
    let span = TOC_WINDOW.min(match_total);
    let default_first = matched
        .iter()
        .position(|sequence| *sequence == current)
        .map(|index| index.saturating_sub(span / 3) as u32)
        .unwrap_or(0);
    let first = match window_first {
        Some(first) => first,
        None => default_first,
    };
    let max_first = match_total.saturating_sub(span);
    let first = (first as usize).min(max_first);
    let rows: Vec<TocRowData> = matched[first..first + span]
        .iter()
        .filter_map(|sequence| {
            session.toc.entry(*sequence).map(|entry| TocRowData {
                sequence: sequence.to_string(),
                title: entry.title,
                meta: format!("偏移 {}", entry.start_byte),
                current: *sequence == current,
            })
        })
        .collect();
    let readout = if match_total == 0 {
        "0 / 0".to_string()
    } else {
        format!(
            "{} / {}",
            (first + rows.len().min(1)).min(match_total),
            match_total
        )
    };
    let count_label = if search.trim().is_empty() {
        format!("{total} 章")
    } else {
        format!("匹配 {match_total} / {total} 章")
    };
    let label = session.chapter_label();
    let title = session.toc.title_at(current);
    let book_id = session.runtime.book().book_id;
    mutate_and_refresh(ui, state, |guard| {
        guard.set_toc_rows(
            rows.clone(),
            first as u32,
            current,
            total,
            match_total,
            readout.clone(),
            count_label.clone(),
        );
        guard.set_chapter(current, total, title);
        guard.set_chapter_label(label);
    });
    log_line(&session.toc.state_log(book_id, current, first as u32));
}

/// 投影书签窗口（有界；按标题过滤；含选中与删除待确认态）。
fn project_marks(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    db_path: &Path,
    session: &ReadingSession,
    offset: Option<u32>,
) {
    let book_id = session.runtime.book().book_id;
    let targets = bookmarks::list(db_path, book_id).unwrap_or_default();
    let current = session.current_chapter();
    let (search, selected, pending, first_hint) = {
        let guard = lock(state);
        (
            guard.mark_search().to_string(),
            guard.mark_selected().to_string(),
            guard.reader().mark_delete_pending.clone(),
            guard.reader().mark_window_first as usize,
        )
    };
    let first = offset.map(|value| value as usize).unwrap_or(first_hint);
    let (first, rows) = bookmarks::window(
        &targets,
        Some(&session.toc),
        current,
        &search,
        first,
        bookmarks::MARKS_PER_PAGE,
        &selected,
        &pending,
    );
    let match_total = if search.trim().is_empty() {
        targets.len()
    } else {
        targets
            .iter()
            .filter(|target| {
                target
                    .title
                    .to_lowercase()
                    .contains(&search.trim().to_lowercase())
            })
            .count()
    };
    let readout = if match_total == 0 {
        "0 / 0".to_string()
    } else {
        format!(
            "{} / {}",
            (first + rows.len().min(1)).min(match_total),
            match_total
        )
    };
    let count_label = if search.trim().is_empty() {
        format!("{} 条", targets.len())
    } else {
        format!("匹配 {match_total} / {} 条", targets.len())
    };
    mutate_and_refresh(ui, state, |guard| {
        guard.set_mark_rows(rows.clone(), first as u32, targets.len(), readout.clone(), count_label.clone());
    });
}

/// 保存当前位置（用户入口：翻页 / 章节跳转 / 书签跳转 / 「保存进度」/ 退出前的脏位置）。
fn save_current(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    db_path: &Path,
    session: &ReadingSession,
    tracker: &Arc<Mutex<ProgressTracker>>,
    prefix: &str,
) {
    let Some(position) = captured_position(session) else {
        log_line("NR-PROGRESS op=save skipped=1 reason=no_page");
        return;
    };
    let loaded = {
        let mut guard = lock(tracker);
        if !guard.mark(position.clone()) {
            false
        } else {
            guard.take().is_some()
        }
    };
    if !loaded {
        let text = format!(
            "{prefix}：位置未变化，未写库（第 {} 章 · 偏移 {} 字节）",
            position.chapter_index + 1,
            position.byte_offset
        );
        mutate_and_refresh(ui, state, |guard| {
            guard.set_progress_and_status(text.clone(), text);
        });
        log_line(&format!(
            "NR-PROGRESS op=save skipped=1 idempotent=1 chapter={} offset={}",
            position.chapter_index, position.byte_offset
        ));
        return;
    }
    match progress::save(db_path, &position, import::now_unix()) {
        Ok(report) => {
            let line = report.as_log(&position);
            lock(tracker).note_saved(position.clone(), report);
            let progress = format!("已保存进度：{}", position.describe());
            let status = format!("{prefix}：进度已保存（{}）", position.describe());
            mutate_and_refresh(ui, state, |guard| {
                guard.set_progress_and_status(progress, status);
            });
            log_line(&line);
        }
        Err(error) => {
            let text = format!("{prefix}：{}", error.message());
            mutate_and_refresh(ui, state, |guard| {
                guard.set_progress_and_status(text.clone(), text);
            });
            log_line(&format!(
                "NR-PROGRESS op=save error=1 detail={}",
                error.message()
            ));
        }
    }
}

/// 以**用户入口**翻页（n 页；正数向后、负数向前），并在位置变化后保存进度。
fn user_turns(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    db_path: &Path,
    session: &mut ReadingSession,
    tracker: &Arc<Mutex<ProgressTracker>>,
    turns: i32,
    prefix: &str,
) -> PageOutcome {
    let mut outcome = session
        .runtime
        .page()
        .map(PageOutcome::Ready)
        .unwrap_or_else(|| session.jump_to(0));
    for _ in 0..turns.abs() {
        outcome = if turns >= 0 {
            session.runtime.next_page()
        } else {
            session.runtime.prev_page()
        };
        if !outcome.is_ready() {
            break;
        }
    }
    project_reader_outcome(ui, state, session, &outcome, prefix);
    project_toc(ui, state, session, None);
    if turns != 0 {
        save_current(ui, state, db_path, session, tracker, prefix);
    }
    outcome
}

/// 翻页入口（键盘 / 滚轮共用）：与「上一页 / 下一页」按钮**同一批函数**（`user_turns`）。
fn turn_pages(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    db_path: &Path,
    reader: &Arc<Mutex<Option<ReadingSession>>>,
    tracker: &Arc<Mutex<ProgressTracker>>,
    turn: PageTurn,
    prefix: &str,
) {
    let mut guard = lock(reader);
    match guard.as_mut() {
        Some(session) => {
            let _ = user_turns(ui, state, db_path, session, tracker, turn.delta(), prefix);
        }
        None => mutate_and_refresh(ui, state, |guard| {
            guard.set_status(format!(
                "{prefix}：尚未打开书籍（请先在书架导入并打开 TXT）"
            ));
        }),
    }
}

/// 键盘输入（Slint `FocusScope.key-pressed` 的落点）→ 翻页意图 → 用户翻页入口。
fn dispatch_key_input(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    db_path: &Path,
    reader: &Arc<Mutex<Option<ReadingSession>>>,
    tracker: &Arc<Mutex<ProgressTracker>>,
    key_text: &str,
    text_input_focused: bool,
) {
    let focused = u8::from(text_input_focused);
    match (
        input::key_name(key_text),
        input::key_intent(key_text, text_input_focused),
    ) {
        (Some(name), Some(turn)) => {
            log_line(&format!(
                "NR-INPUT kind=key key={name} intent={} focused={focused}",
                turn.log_name()
            ));
            turn_pages(
                ui,
                state,
                db_path,
                reader,
                tracker,
                turn,
                &format!("键盘翻页（{name}）"),
            );
        }
        (Some(name), None) => {
            log_line(&format!(
                "NR-INPUT kind=key key={name} intent=none focused={focused} blocked=text-input"
            ));
        }
        (None, _) => {}
    }
}

/// 鼠标滚轮输入（正文区 `TouchArea.scroll-event` 的落点）→ 翻页意图 → 用户翻页入口。
fn dispatch_wheel_input(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    db_path: &Path,
    reader: &Arc<Mutex<Option<ReadingSession>>>,
    tracker: &Arc<Mutex<ProgressTracker>>,
    delta_x: f32,
    delta_y: f32,
) {
    match input::wheel_intent(delta_x, delta_y) {
        Some(turn) => {
            log_line(&format!(
                "NR-INPUT kind=wheel delta_x={:.0} delta_y={:.0} intent={}",
                delta_x,
                delta_y,
                turn.log_name()
            ));
            turn_pages(
                ui,
                state,
                db_path,
                reader,
                tracker,
                turn,
                &format!("滚轮翻页（delta-y={:.0}）", delta_y),
            );
        }
        None => log_line(&format!(
            "NR-INPUT kind=wheel delta_x={:.0} delta_y={:.0} intent=none",
            delta_x, delta_y
        )),
    }
}

/// 滚轮事件的投递坐标：窗口中心（逻辑坐标）。
fn scroll_target(ui: &AppWindow) -> (f32, f32) {
    let size = ui.window().size();
    let scale = {
        let factor = ui.window().scale_factor();
        if factor > 0.0 {
            factor
        } else {
            1.0
        }
    };
    (
        size.width as f32 / scale / 2.0,
        size.height as f32 / scale / 2.0,
    )
}

/// 程序化输入注入（DT8 证据）：把环境变量给定的事件**投递给 Slint 窗口**。
fn dispatch_input_events(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    db_path: &Path,
    reader: &Arc<Mutex<Option<ReadingSession>>>,
    tracker: &Arc<Mutex<ProgressTracker>>,
    key_events: Option<&str>,
    wheel_events: Option<&str>,
) -> usize {
    let mut count = 0usize;
    let text_focused = env_flag(TEXT_FOCUS_ENV);
    if let Some(spec) = key_events {
        for name in spec
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            if text_focused {
                let Some(text) = input::key_text(name) else {
                    continue;
                };
                dispatch_key_input(ui, state, db_path, reader, tracker, text.as_str(), true);
                count += 1;
            } else if let Some(event) = input::key_event(name) {
                ui.window().dispatch_event(event);
                count += 1;
            }
        }
    }
    if let Some(spec) = wheel_events {
        for item in spec
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            let Ok(delta_y) = item.parse::<f32>() else {
                continue;
            };
            let (x, y) = scroll_target(ui);
            ui.window()
                .dispatch_event(input::scroll_event(x, y, 0.0, delta_y));
            count += 1;
        }
    }
    count
}

/// 跳转到字节偏移（章节跳转 / 书签跳转 / 进度恢复 / 进度条跳转共用），并保存新位置。
fn jump_to_offset(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    db_path: &Path,
    session: &mut ReadingSession,
    tracker: &Arc<Mutex<ProgressTracker>>,
    byte_offset: u64,
    prefix: &str,
) -> PageOutcome {
    let outcome = session.jump_to(byte_offset);
    project_reader_outcome(ui, state, session, &outcome, prefix);
    project_toc(ui, state, session, None);
    save_current(ui, state, db_path, session, tracker, prefix);
    outcome
}

/// 章节跳转（目标 = 章节起始字节，来自核心章节索引）。
fn jump_to_chapter(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    db_path: &Path,
    session: &mut ReadingSession,
    tracker: &Arc<Mutex<ProgressTracker>>,
    sequence: u32,
) -> Result<u64, chapters::ChapterError> {
    let target = chapters::jump_target(&session.toc, sequence)?;
    let prefix = format!("章节跳转（第 {} 章）", sequence + 1);
    let outcome = jump_to_offset(ui, state, db_path, session, tracker, target, &prefix);
    if outcome.is_ready() {
        mutate_and_refresh(ui, state, |guard| {
            guard.set_toc_open(false);
        });
    }
    log_line(&format!(
        "NR-CHAPTER op=jump sequence={} chapters={} target={} toc_closed={}",
        sequence,
        session.toc.len(),
        target,
        u8::from(outcome.is_ready())
    ));
    Ok(target)
}

/// 进度条点击 / 拖拽跳转（`ratio` ∈ [0,1]；与目录跳转共用同一位置校验路径）。
fn seek_progress(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    db_path: &Path,
    session: &mut ReadingSession,
    tracker: &Arc<Mutex<ProgressTracker>>,
    ratio: f64,
) {
    let total = session.runtime.total_bytes();
    if total == 0 {
        mutate_and_refresh(ui, state, |guard| {
            guard.set_status("进度跳转：内容长度未知，本次请求已忽略");
        });
        return;
    }
    let ratio = ratio.clamp(0.0, 1.0);
    let target = ((total as f64) * ratio).round() as u64;
    let target = target.min(total.saturating_sub(1));
    let prefix = format!("进度跳转（{:.0}%）", ratio * 100.0);
    let outcome = jump_to_offset(ui, state, db_path, session, tracker, target, &prefix);
    let landing = session.runtime.progress_permille();
    let status = format!(
        "进度跳转：已到 {}（{}）；校验结果 {}",
        crate::pagination::format_progress_percent(landing),
        session.chapter_label(),
        outcome.describe()
    );
    mutate_and_refresh(ui, state, |guard| {
        guard.set_status(status.clone());
    });
    log_line(&format!(
        "NR-SEEK ratio={:.3} target={} start={} end={} permille={} ready={}",
        ratio,
        target,
        session.runtime.page().map(|page| page.start_byte).unwrap_or(0),
        session.runtime.page().map(|page| page.end_byte).unwrap_or(0),
        landing,
        u8::from(outcome.is_ready())
    ));
}

/// 编码选择行（7 种编码；当前生效项由 Rust 标记）。
fn encoding_rows_for(current: reading_source::TextEncoding) -> Vec<ChoiceRowData> {
    reading_source::TextEncoding::ALL
        .iter()
        .map(|encoding| ChoiceRowData {
            name: encoding.to_string(),
            current: *encoding == current,
        })
        .collect()
}

/// 投影内容编码：展示名（用户指定时带「（手动）」）与选项弹窗行。
fn project_encoding(ui: &AppWindow, state: &Arc<Mutex<AppState>>, session: &ReadingSession) {
    let book = session.runtime.book();
    let manual = book.encoding_user_override;
    let label = if manual {
        format!("编码 {}（手动）", book.encoding)
    } else {
        format!("编码 {}", book.encoding)
    };
    let rows = encoding_rows_for(session.runtime.encoding());
    let current_index = rows
        .iter()
        .position(|row| row.current)
        .map(|index| index as i32)
        .unwrap_or(0);
    mutate_and_refresh(ui, state, |guard| {
        guard.set_encoding_label(label.clone(), manual);
        guard.set_encoding_rows(rows.clone());
    });
    let _ = current_index;
}

/// 打开选项弹窗（S8：编码等；界面只渲染，语义由 Rust 投影）。
fn open_options_dialog(ui: &AppWindow, state: &Arc<Mutex<AppState>>, session: &ReadingSession) {
    let book = session.runtime.book();
    let rows = encoding_rows_for(session.runtime.encoding());
    let current_index = rows
        .iter()
        .position(|row| row.current)
        .map(|index| index as i32)
        .unwrap_or(0);
    let meta = if book.encoding_user_override {
        format!("当前：{}（手动指定）", book.encoding)
    } else {
        format!("当前：{}（自动识别）", book.encoding)
    };
    mutate_and_refresh(ui, state, |guard| {
        guard.open_choices("选项：内容编码", meta.clone(), rows.clone(), current_index);
    });
}

/// 进入 / 退出沉浸阅读（界面按钮与程序化验证共用同一函数）。
fn apply_immersive(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    db_path: &Path,
    session: &mut ReadingSession,
    tracker: &Arc<Mutex<ProgressTracker>>,
    immersive: bool,
) {
    match session.runtime.set_immersive(immersive) {
        Ok(result) => {
            mutate_and_refresh(ui, state, |guard| {
                guard.set_immersive(immersive);
            });
            project_reader_outcome(
                ui,
                state,
                session,
                &result.outcome,
                if immersive {
                    "沉浸阅读"
                } else {
                    "退出沉浸"
                },
            );
            project_toc(ui, state, session, None);
            save_current(ui, state, db_path, session, tracker, "沉浸阅读");
            log_line(&format!(
                "NR-IMMERSIVE immersive={} layout={} start={} lines={}",
                u8::from(immersive),
                session.runtime.layout_id(),
                session
                    .runtime
                    .page()
                    .map(|page| page.start_byte)
                    .unwrap_or(0),
                session
                    .runtime
                    .page()
                    .map(|page| page.line_count())
                    .unwrap_or(0),
            ));
        }
        Err(error) => set_status_text(ui, state, "沉浸阅读", &error.message()),
    }
}

/// 用户手动选择内容编码（选项弹窗「应用」入口）。顺序（每一步都是真实链路）：
/// 落库 + 递增内容索引版本 → 按新编码重建目录 → 按**字节位置**重建分页会话 → 投影与保存进度。
fn apply_encoding_override(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    db_path: &Path,
    session: &mut ReadingSession,
    tracker: &Arc<Mutex<ProgressTracker>>,
    encoding: reading_source::TextEncoding,
) {
    let book_id = session.runtime.book().book_id;
    let now = import::now_unix();
    let report = match crate::encoding::override_encoding(db_path, book_id, encoding, now) {
        Ok(report) => report,
        Err(message) => {
            let text = format!("编码覆盖失败：{message}");
            mutate_and_refresh(ui, state, |guard| {
                guard.set_progress_and_status(text.clone(), text);
            });
            log_line(&format!("NR-ENCODING override=1 error=1 detail={message}"));
            return;
        }
    };

    let mut book = session.runtime.book().clone();
    book.encoding = encoding;
    book.encoding_user_override = true;
    match chapters::ensure_toc(db_path, &book, now) {
        Ok(toc_report) => session.toc = toc_report.toc,
        Err(error) => {
            let text = format!("编码覆盖：目录重建失败（{}）", error.message());
            mutate_and_refresh(ui, state, |guard| {
                guard.set_progress_and_status(text.clone(), text);
            });
            log_line(&format!(
                "NR-ENCODING override=1 error=1 toc=0 detail={}",
                error.message()
            ));
            return;
        }
    }

    let reopen = match session.runtime.reopen_with_encoding(encoding) {
        Ok(result) => result,
        Err(error) => {
            let text = format!("编码覆盖：分页重建失败（{}）", error.message());
            mutate_and_refresh(ui, state, |guard| {
                guard.set_progress_and_status(text.clone(), text);
            });
            log_line(&format!(
                "NR-ENCODING override=1 error=1 reflow=0 detail={}",
                error.message()
            ));
            return;
        }
    };
    let retention = if reopen.offset_retained {
        format!("位置保持（偏移 {}）", reopen.requested_offset)
    } else {
        format!(
            "偏移 {} 在新编码下不是合法边界，已回到内容开头",
            reopen.requested_offset
        )
    };
    let status = format!(
        "编码已切换为 {}（索引版本 {}，{}）：{} 章",
        report.encoding,
        report.index_version,
        retention,
        session.toc.len(),
    );
    log_line(&session.runtime.reopen_state_log(&reopen));
    project_reader_outcome(ui, state, session, &reopen.outcome, "编码切换");
    project_toc(ui, state, session, None);
    project_encoding(ui, state, session);
    mutate_and_refresh(ui, state, |guard| {
        guard.close_dialog();
        guard.set_progress_and_status(status.clone(), status.clone());
    });
    save_current(ui, state, db_path, session, tracker, "编码切换");
}

/// 新增书签（位置 = 当前页的 `ContentPosition`）。
fn add_mark_at_current(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    db_path: &Path,
    session: &ReadingSession,
    title: Option<&str>,
) -> Option<i64> {
    let Some(position) = captured_position(session) else {
        log_line("NR-MARK op=add skipped=1 reason=no_page");
        return None;
    };
    let title = match title {
        Some(title) if !title.trim().is_empty() => title.trim().to_string(),
        _ => bookmarks::default_title(&session.toc, position.chapter_index, position.byte_offset),
    };
    match bookmarks::add(db_path, &position, &title, import::now_unix()) {
        Ok(id) => {
            project_marks(ui, state, db_path, session, None);
            let count = bookmarks::list(db_path, session.runtime.book().book_id)
                .map(|marks| marks.len())
                .unwrap_or(0);
            let text = format!("已新增书签：{title}");
            mutate_and_refresh(ui, state, |guard| {
                guard.set_progress_and_status(text.clone(), text);
                guard.set_mark_selected(id.to_string());
                guard.set_mark_rename_text(String::new());
            });
            log_line(&bookmarks::state_log("add", id, count, None, &title));
            Some(id)
        }
        Err(error) => {
            let text = format!("新增书签失败：{}", error.message());
            mutate_and_refresh(ui, state, |guard| {
                guard.set_progress_and_status(text.clone(), text);
            });
            log_line(&format!(
                "NR-MARK op=add error=1 detail={}",
                error.message()
            ));
            None
        }
    }
}

/// 跳转到书签（目标 = 书签的内容位置）。
fn jump_to_mark(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    db_path: &Path,
    session: &mut ReadingSession,
    tracker: &Arc<Mutex<ProgressTracker>>,
    id: i64,
) {
    match bookmarks::target(db_path, id) {
        Ok(target) => {
            let prefix = format!("书签跳转（{id}）");
            jump_to_offset(
                ui,
                state,
                db_path,
                session,
                tracker,
                target.byte_offset,
                &prefix,
            );
            let count = bookmarks::list(db_path, session.runtime.book().book_id)
                .map(|marks| marks.len())
                .unwrap_or(0);
            log_line(&target.as_log(count));
        }
        Err(error) => {
            let text = format!("书签跳转失败：{}", error.message());
            mutate_and_refresh(ui, state, |guard| {
                guard.set_progress_and_status(text.clone(), text);
            });
            log_line(&format!(
                "NR-MARK op=jump error=1 detail={}",
                error.message()
            ));
        }
    }
}

/// 重命名书签（界面入口：仅用当前选中的书签 + 输入框文本）。
fn rename_selected_mark(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    db_path: &Path,
    session: &ReadingSession,
    id: i64,
    title: &str,
) {
    match bookmarks::rename(db_path, id, title, import::now_unix()) {
        Ok(true) => {
            project_marks(ui, state, db_path, session, None);
            let text = format!("书签已重命名：{title}");
            mutate_and_refresh(ui, state, |guard| {
                guard.set_progress_and_status(text.clone(), text);
                guard.set_mark_rename_text(String::new());
            });
            log_line(&bookmarks::state_log("rename", id, 0, None, title));
        }
        Ok(false) => {
            let text = "重命名失败：书签不存在".to_string();
            mutate_and_refresh(ui, state, |guard| {
                guard.set_progress_and_status(text.clone(), text);
            });
            log_line(&format!("NR-MARK op=rename error=1 id={id}"));
        }
        Err(error) => {
            let text = format!("重命名失败：{}", error.message());
            mutate_and_refresh(ui, state, |guard| {
                guard.set_progress_and_status(text.clone(), text);
            });
            log_line(&format!(
                "NR-MARK op=rename error=1 id={id} detail={}",
                error.message()
            ));
        }
    }
}

/// 删除书签（行内两段确认后的真正删除）。
fn delete_mark(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    db_path: &Path,
    session: &ReadingSession,
    id: i64,
) {
    match bookmarks::remove(db_path, id) {
        Ok(true) => {
            project_marks(ui, state, db_path, session, None);
            let text = format!("已删除书签 {id}");
            mutate_and_refresh(ui, state, |guard| {
                guard.set_progress_and_status(text.clone(), text);
                guard.set_mark_selected(String::new());
                guard.set_mark_delete_pending(String::new());
            });
            log_line(&bookmarks::state_log("delete", id, 0, None, ""));
        }
        Ok(false) => {
            let text = format!("删除失败：书签 {id} 不存在");
            mutate_and_refresh(ui, state, |guard| {
                guard.set_progress_and_status(text.clone(), text);
            });
            log_line(&format!("NR-MARK op=delete error=1 id={id}"));
        }
        Err(error) => {
            let text = format!("删除失败：{}", error.message());
            mutate_and_refresh(ui, state, |guard| {
                guard.set_progress_and_status(text.clone(), text);
            });
            log_line(&format!(
                "NR-MARK op=delete error=1 id={id} detail={}",
                error.message()
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Toast
// ---------------------------------------------------------------------------

/// Toast 令牌序列（旧定时器按令牌失效，不会清除新提示）。
static TOAST_SEQ: AtomicU64 = AtomicU64::new(1);

/// 显示 Toast，并在 [`TOAST_MS`] 后由一次性定时器清除（不产生持续出帧）。
fn show_toast(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    text: impl Into<String>,
    action_label: impl Into<String>,
    action: impl Into<String>,
) {
    let token = TOAST_SEQ.fetch_add(1, Ordering::SeqCst);
    mutate_and_refresh(ui, state, |guard| {
        guard.show_toast(text, action_label, action, token);
    });
    let weak = ui.as_weak();
    let state = Arc::clone(state);
    slint::Timer::single_shot(Duration::from_millis(TOAST_MS), move || {
        let cleared = {
            let mut guard = lock(&state);
            guard.clear_toast(token)
        };
        if cleared {
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &state);
            }
        }
    });
}

// ---------------------------------------------------------------------------
// 设置
// ---------------------------------------------------------------------------

/// 应用一次设置变更并重新投影（含重排与主题重注入）。
fn apply_settings_value(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    db_path: &Path,
    settings: &Arc<Mutex<GlobalSettings>>,
    session: Option<&mut ReadingSession>,
    key: &str,
    change: SettingsChange,
) -> Option<String> {
    let (changed, log, error) = {
        let mut guard = lock(settings);
        match change {
            SettingsChange::Delta(delta) => match guard.adjust(key, delta as i64) {
                Ok(true) => (true, guard.state_log(1), None),
                Ok(false) => (false, guard.state_log(0), None),
                Err(error) => (false, String::new(), Some(error.message())),
            },
            SettingsChange::ControlInt(value) => match guard.set_control_value(key, value) {
                Ok(changed) => (changed, guard.state_log(usize::from(changed)), None),
                Err(error) => (false, String::new(), Some(error.message())),
            },
            SettingsChange::Text(ref text) => match guard.set_text(key, text) {
                Ok(()) => (true, guard.state_log(1), None),
                Err(error) => (false, String::new(), Some(error.message())),
            },
            SettingsChange::OptionIndex(index) => match guard.set_option_index(key, index) {
                Ok(()) => (true, guard.state_log(1), None),
                Err(error) => (false, String::new(), Some(error.message())),
            },
            SettingsChange::Pairs(ref pairs) => {
                let mut changed = 0usize;
                let mut last_error = None;
                for (pair_key, pair_value) in pairs {
                    match guard.set_text(pair_key, pair_value) {
                        Ok(()) => changed += 1,
                        Err(error) => last_error = Some(error.message()),
                    }
                }
                (changed > 0, guard.state_log(changed), last_error)
            }
        }
    };
    if let Some(error) = error {
        let text = format!("设置失败：{error}");
        mutate_and_refresh(ui, state, |guard| {
            guard.set_progress_and_status(text.clone(), text);
        });
        log_line(&format!("NR-SETTINGS error=1 detail={error}"));
        return None;
    }
    if !changed {
        let text = format!("设置未变化：{key}");
        mutate_and_refresh(ui, state, |guard| {
            guard.set_progress_and_status(text.clone(), text);
        });
        log_line(&log);
        return None;
    }
    if let Err(error) = settings::save(db_path, &lock(settings), import::now_unix()) {
        let text = format!("设置落库失败：{}", error.message());
        mutate_and_refresh(ui, state, |guard| {
            guard.set_progress_and_status(text.clone(), text);
        });
        log_line(&format!("NR-SETTINGS error=1 detail={}", error.message()));
        return None;
    }
    let typography = lock(settings).typography();
    let gesture = lock(settings).gesture_mode();
    let dark = lock(settings).theme_dark();
    let view = settings::view_for(db_path, &lock(settings));
    if let Some(session) = session {
        if let Ok(result) = session.runtime.set_typography_settings(typography) {
            project_reader_outcome(ui, state, session, &result.outcome, "设置");
        }
    }
    apply_palette(ui, dark);
    mutate_and_refresh(ui, state, |guard| {
        guard.set_settings_view(view.clone());
        guard.set_gesture_mode(gesture);
        guard.set_status(format!("设置已保存：{key}"));
    });
    log_line(&log);
    Some(log)
}

/// 一次设置变更的口径。
enum SettingsChange {
    /// 控件口径步长（循环字段 / 快捷键式调整）。
    Delta(i32),
    /// 控件口径绝对值（滑杆 / 开关）。
    ControlInt(i32),
    /// 显示口径文本（数值框输入；由 Rust 解析）。
    Text(String),
    /// 枚举下标（单选组）。
    OptionIndex(i32),
    /// `key=value` 组合（程序化驱动）。
    Pairs(Vec<(String, String)>),
}

/// 解析 `key=value,key=value` 形式的设置驱动串。
fn parse_setting_pairs(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut pairs = Vec::new();
    for chunk in text.split(',') {
        let chunk = chunk.trim();
        if chunk.is_empty() {
            continue;
        }
        let (key, value) = chunk
            .split_once('=')
            .ok_or_else(|| format!("设置项 `{chunk}` 缺少 `=`"))?;
        pairs.push((key.trim().to_string(), value.trim().to_string()));
    }
    if pairs.is_empty() {
        return Err("设置驱动串为空".to_string());
    }
    Ok(pairs)
}

/// 阅读页当前几何（真实窗口尺寸 + 实测字体度量 + 工具条状态）。
fn current_reader_geometry(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    viewport_slot: &Arc<Mutex<MappedViewport>>,
    font_slot: &Arc<Mutex<PlatformFontMetrics>>,
) -> (MappedViewport, PlatformFontMetrics, bool) {
    let mut viewport = *lock(viewport_slot);
    if !viewport.is_usable() {
        let size = ui.window().size();
        let scale = ui.window().scale_factor().max(1.0);
        viewport = MappedViewport::new(
            (size.width as f32 / scale).round() as u32,
            (size.height as f32 / scale).round() as u32,
        )
        .with_scale_milli((scale * 1_000.0).round() as u32);
    }
    let measured = PlatformFontMetrics::from_px(
        ui.get_body_ascent().max(0.0).round() as u32,
        ui.get_body_descent().max(0.0).round() as u32,
    );
    let font = if measured.is_known() {
        measured
    } else {
        *lock(font_slot)
    };
    let toolbar_open = lock(state).project().reader.toolbar_open;
    (viewport, font, toolbar_open)
}

/// 从书架打开一本书（整行点击与程序化驱动 `NR_SHELF_OPEN` **共用同一函数**）。
#[allow(clippy::too_many_arguments)]
fn open_reader_from_shelf(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    reader: &Arc<Mutex<Option<ReadingSession>>>,
    settings_slot: &Arc<Mutex<GlobalSettings>>,
    viewport_slot: &Arc<Mutex<MappedViewport>>,
    font_slot: &Arc<Mutex<PlatformFontMetrics>>,
    db_path: &Path,
    selection: &str,
) {
    let (viewport, font, toolbar_open) =
        current_reader_geometry(ui, state, viewport_slot, font_slot);
    let loaded = lock(settings_slot).clone();
    let Some(session) = open_reader(
        ui,
        state,
        db_path,
        viewport,
        font,
        toolbar_open,
        &loaded,
        selection,
    ) else {
        return;
    };
    let book = session.runtime.book();
    log_line(&format!(
        "NR-OPEN selection={} book={} title={} encoding={} screen=reader",
        selection, book.book_id, book.title, book.encoding
    ));
    *lock(reader) = Some(session);
    mutate_and_refresh(ui, state, |guard| {
        guard.set_screen(Screen::Reader);
    });
}

/// 打开阅读目标并建立阅读会话（含目录建立与重启恢复）。
fn open_reader(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    db_path: &Path,
    viewport: MappedViewport,
    font: PlatformFontMetrics,
    toolbar_open: bool,
    settings: &GlobalSettings,
    selection: &str,
) -> Option<ReadingSession> {
    let book = match open_book(db_path, Some(selection)) {
        Ok(book) => book,
        Err(message) => {
            mutate_and_refresh(ui, state, |guard| {
                guard.set_status(format!("阅读：{message}"));
            });
            return None;
        }
    };
    if !book.is_readable() {
        let status = format!("阅读：来源状态为 {}，请先在书架重新关联该 TXT", book.status);
        mutate_and_refresh(ui, state, |guard| {
            guard.set_status(status);
        });
        return None;
    }
    let now = import::now_unix();
    let toc = match chapters::ensure_toc(db_path, &book, now) {
        Ok(report) => {
            log_line(&format!(
                "NR-TOC-INDEX book={} origin={} chapters={} persisted={} index_version={}",
                book.book_id,
                report.toc.origin().as_str(),
                report.toc.len(),
                report.persisted_rows,
                report.index_version
            ));
            report.toc
        }
        Err(error) => {
            let status = format!("阅读：{}", error.message());
            mutate_and_refresh(ui, state, |guard| {
                guard.set_status(status);
            });
            return None;
        }
    };
    let runtime = match ReaderRuntime::open(
        book.clone(),
        LayoutMetrics::DESKTOP,
        viewport,
        settings.typography(),
        font,
        toolbar_open,
    ) {
        Ok(runtime) => runtime,
        Err(error) => {
            let status = format!("阅读：{}", error.message());
            mutate_and_refresh(ui, state, |guard| {
                guard.set_status(status);
            });
            return None;
        }
    };
    let mut session = ReadingSession { runtime, toc };
    match progress::load(db_path, session.runtime.book().book_id) {
        Ok(Some(saved)) => {
            match progress::restore_encoded(
                &session.runtime.book().path,
                &session.toc,
                &saved,
                session.runtime.book().encoding,
            ) {
                Ok(report) => {
                    let _ = session.jump_to(report.byte_offset);
                    log_line(&report.as_log(session.runtime.book().book_id, Some(&saved)));
                    let text = format!(
                        "已恢复阅读位置：{}（{}）",
                        saved.describe(),
                        if report.fallback_to_chapter_start {
                            "未采用保存偏移，已回退到章节开头"
                        } else {
                            "按保存的字节偏移恢复"
                        }
                    );
                    mutate_and_refresh(ui, state, |guard| {
                        guard.set_progress_and_status(text.clone(), text);
                    });
                }
                Err(error) => {
                    log_line(&format!(
                        "NR-POS restored=1 error=1 detail={}",
                        error.message()
                    ));
                }
            }
        }
        Ok(None) => log_line(&format!(
            "NR-POS book={} restored=0 chapter=0 offset=0 fallback=0 anchor=none",
            session.runtime.book().book_id
        )),
        Err(error) => {
            log_line(&format!("NR-POS error=1 detail={}", error.message()));
        }
    }
    project_reader_outcome(
        ui,
        state,
        &session,
        &session
            .runtime
            .page()
            .map(PageOutcome::Ready)
            .unwrap_or(PageOutcome::EndOfBook {
                layout_id: session.runtime.layout_id(),
                cursor: 0,
                total_bytes: 0,
            }),
        "首屏",
    );
    project_toc(ui, state, &session, None);
    project_marks(ui, state, db_path, &session, None);
    project_encoding(ui, state, &session);
    Some(session)
}

/// 只更新状态栏文案（不触碰页面内容）。
fn set_status_text(ui: &AppWindow, state: &Arc<Mutex<AppState>>, prefix: &str, message: &str) {
    mutate_and_refresh(ui, state, |guard| {
        guard.set_status(format!("{prefix}：{message}"));
    });
}

/// 启动桌面应用（阻塞直到窗口关闭）。
pub fn run() -> Result<(), slint::PlatformError> {
    let metrics = LayoutMetrics::DESKTOP;
    let app_version = format!(
        "core {} · abi {}",
        reading_core::CORE_VERSION,
        reading_core::ABI_VERSION
    );
    let db_path = shelf::default_db_path();
    let global_settings = settings::load(&db_path).unwrap_or_else(|error| {
        eprintln!("全局设置不可读，使用核心默认值：{}", error.message());
        GlobalSettings::default()
    });
    log_line(&global_settings.state_log(0));

    // 阅读工具条**常驻**（设计 §5 S5）；`NR_READER_MENU=0` 可关闭，供程序化对照。
    let toolbar_open = !matches!(
        std::env::var(READER_MENU_ENV).ok().as_deref(),
        Some("0") | Some("false")
    );

    let state = Arc::new(Mutex::new(AppState::new(app_version)));
    {
        let mut guard = lock(&state);
        if let Some(screen) = start_screen_from_env() {
            guard.set_screen(screen);
        }
        guard.set_settings_view(settings::view_for(&db_path, &global_settings));
        guard.set_gesture_mode(global_settings.gesture_mode());
        guard.set_shelf_view(settings::load_shelf_view(&db_path));
        guard.set_shelf_sort(settings::load_shelf_sort(&db_path));
        guard.set_shortcuts(
            SHORTCUT_PREV.to_string(),
            SHORTCUT_NEXT.to_string(),
            SHORTCUT_ESCAPE.to_string(),
        );
        if toolbar_open {
            guard.toggle_reader_toolbar();
        }
        guard.set_status("就绪：正在加载本地书架…");
    }
    let settings_slot = Arc::new(Mutex::new(global_settings));

    let ui = AppWindow::new()?;
    apply_layout_metrics(&ui, &metrics);
    apply_palette(&ui, lock(&settings_slot).theme_dark());

    // ---- 分页运行时与视口状态（窗口映射后才取真实尺寸）----
    let reader: Arc<Mutex<Option<ReadingSession>>> = Arc::new(Mutex::new(None));
    let tracker = Arc::new(Mutex::new(ProgressTracker::new()));
    let viewport_slot: Arc<Mutex<MappedViewport>> = Arc::new(Mutex::new(MappedViewport::UNMAPPED));
    let font_slot: Arc<Mutex<PlatformFontMetrics>> =
        Arc::new(Mutex::new(PlatformFontMetrics::UNKNOWN));
    let idle_font_px = (lock(&settings_slot).reader().font_size_milli.max(1) + 999) / 1_000;
    let idle_font_px = idle_font_px as u32;
    apply_reader_typography(
        &ui,
        idle_font_px,
        idle_font_px.saturating_mul(3) / 2,
        lock(&settings_slot).reader().font_family.as_str(),
    );
    // 窗口映射完成/尺寸变化 → 以当前位置重新分页。
    ui.on_viewport_changed({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let viewport_slot = Arc::clone(&viewport_slot);
        let font_slot = Arc::clone(&font_slot);
        let weak = ui.as_weak();
        move |width, height| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let scale = ui.window().scale_factor().max(0.0);
            let viewport = MappedViewport::new(width.max(0.0) as u32, height.max(0.0) as u32)
                .with_scale_milli((scale * 1_000.0).round() as u32);
            {
                let mut slot = lock(&viewport_slot);
                *slot = viewport;
            }
            let font = PlatformFontMetrics::from_px(
                ui.get_body_ascent().max(0.0).round() as u32,
                ui.get_body_descent().max(0.0).round() as u32,
            );
            {
                let mut slot = lock(&font_slot);
                *slot = font;
            }
            let mut guard = lock(&reader);
            let Some(session) = guard.as_mut() else {
                return;
            };
            let mut reflow = None;
            if font.is_known() {
                match session.runtime.set_font_metrics(font) {
                    Ok(outcome) => reflow = Some(("字体度量", outcome)),
                    Err(error) => set_status_text(&ui, &state, "字体度量", &error.message()),
                }
            }
            match session.runtime.apply_viewport(viewport) {
                Ok(outcome) => reflow = Some(("窗口变化", outcome)),
                Err(error) => set_status_text(&ui, &state, "窗口变化", &error.message()),
            }
            if let Some((prefix, result)) = reflow {
                project_reader_outcome(&ui, &state, session, &result.outcome, prefix);
                project_toc(&ui, &state, session, None);
            }
        }
    });

    // ---- Slint → Rust：导航与顶栏 ----
    ui.on_navigate({
        let state = Arc::clone(&state);
        let weak = ui.as_weak();
        move |index| {
            let changed = lock(&state).navigate(index);
            if changed {
                if let Some(ui) = weak.upgrade() {
                    refresh(&ui, &state);
                }
            }
        }
    });
    ui.on_theme_toggle({
        let state = Arc::clone(&state);
        let settings_slot = Arc::clone(&settings_slot);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            // 顶栏主题图标：明亮 ⇄ 暗黑（直接设定；「跟随系统」仍可在设置页选择）。
            let target = if lock(&settings_slot).theme_dark() {
                "light"
            } else {
                "dark"
            };
            apply_settings_value(
                &ui,
                &state,
                &db_path,
                &settings_slot,
                None,
                settings::KEY_THEME,
                SettingsChange::Text(target.to_string()),
            );
        }
    });
    ui.on_shortcuts_toggle({
        let state = Arc::clone(&state);
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let rows: Vec<ShortcutEntryView> = SHORTCUTS
                .iter()
                .map(|spec| ShortcutEntryView {
                    action: spec.action.to_string(),
                    keys: spec.keys.to_string(),
                    available: spec.implemented,
                })
                .collect();
            mutate_and_refresh(&ui, &state, |guard| {
                guard.open_shortcuts(rows.clone());
            });
        }
    });
    ui.on_quit_requested(|| {
        let _ = slint::quit_event_loop();
    });

    // ---- 阅读页：工具条 / 翻页 / 进度条 / 面板 ----
    ui.on_toggle_reader_toolbar({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            mutate_and_refresh(&ui, &state, |guard| {
                guard.toggle_reader_toolbar();
            });
            let mut guard = lock(&reader);
            if let Some(session) = guard.as_mut() {
                match session.runtime.toggle_menu() {
                    Ok(result) => {
                        project_reader_outcome(&ui, &state, session, &result.outcome, "工具条");
                        project_toc(&ui, &state, session, None);
                    }
                    Err(error) => set_status_text(&ui, &state, "工具条", &error.message()),
                }
            }
        }
    });
    ui.on_reader_prev_page({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let tracker = Arc::clone(&tracker);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let mut guard = lock(&reader);
            match guard.as_mut() {
                Some(session) => {
                    let _ = user_turns(&ui, &state, &db_path, session, &tracker, -1, "上一页");
                }
                None => mutate_and_refresh(&ui, &state, |guard| {
                    guard.set_status("上一页：尚未打开书籍（请先在书架导入并打开 TXT）");
                }),
            }
        }
    });
    ui.on_reader_next_page({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let tracker = Arc::clone(&tracker);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let mut guard = lock(&reader);
            match guard.as_mut() {
                Some(session) => {
                    let _ = user_turns(&ui, &state, &db_path, session, &tracker, 1, "下一页");
                }
                None => mutate_and_refresh(&ui, &state, |guard| {
                    guard.set_status("下一页：尚未打开书籍（请先在书架导入并打开 TXT）");
                }),
            }
        }
    });
    ui.on_reader_save_progress({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let tracker = Arc::clone(&tracker);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let guard = lock(&reader);
            match guard.as_ref() {
                Some(session) => save_current(&ui, &state, &db_path, session, &tracker, "保存进度"),
                None => mutate_and_refresh(&ui, &state, |guard| {
                    guard.set_status("保存进度：尚未打开书籍");
                }),
            }
        }
    });
    ui.on_reader_progress_seek({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let tracker = Arc::clone(&tracker);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move |ratio| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let mut guard = lock(&reader);
            match guard.as_mut() {
                Some(session) => seek_progress(
                    &ui,
                    &state,
                    &db_path,
                    session,
                    &tracker,
                    f64::from(ratio),
                ),
                None => mutate_and_refresh(&ui, &state, |guard| {
                    guard.set_status("进度跳转：尚未打开书籍");
                }),
            }
        }
    });
    ui.on_reader_immersive_toggle({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let tracker = Arc::clone(&tracker);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let target = !lock(&state).immersive();
            let mut guard = lock(&reader);
            match guard.as_mut() {
                Some(session) => apply_immersive(&ui, &state, &db_path, session, &tracker, target),
                None => mutate_and_refresh(&ui, &state, |guard| {
                    guard.set_status("沉浸阅读：尚未打开书籍（请先在书架导入并打开 TXT）");
                }),
            }
        }
    });
    ui.on_reader_zone_hover({
        let state = Arc::clone(&state);
        let settings_slot = Arc::clone(&settings_slot);
        let floating_gen = Arc::new(AtomicU64::new(0));
        let weak = ui.as_weak();
        move |hovered| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let autohide = lock(&settings_slot).immersive_autohide();
            let gen = floating_gen.fetch_add(1, Ordering::SeqCst) + 1;
            if !autohide {
                // 关闭自动隐藏：浮出工具条常驻。
                let visible_now = lock(&state).floating_toolbar();
                if !visible_now {
                    mutate_and_refresh(&ui, &state, |guard| {
                        guard.set_floating_toolbar(true);
                    });
                }
                return;
            }
            if hovered {
                let visible_now = lock(&state).floating_toolbar();
                if !visible_now {
                    mutate_and_refresh(&ui, &state, |guard| {
                        guard.set_floating_toolbar(true);
                    });
                }
            } else {
                let weak = weak.clone();
                let state = Arc::clone(&state);
                let gen_slot = Arc::clone(&floating_gen);
                slint::Timer::single_shot(Duration::from_millis(FLOATING_HIDE_MS), move || {
                    if gen_slot.load(Ordering::SeqCst) != gen {
                        return;
                    }
                    let Some(ui) = weak.upgrade() else {
                        return;
                    };
                    let visible_now = lock(&state).floating_toolbar();
                    if visible_now {
                        mutate_and_refresh(&ui, &state, |guard| {
                            guard.set_floating_toolbar(false);
                        });
                    }
                });
            }
        }
    });
    ui.on_reader_options_toggle({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let guard = lock(&reader);
            match guard.as_ref() {
                Some(session) => open_options_dialog(&ui, &state, session),
                None => mutate_and_refresh(&ui, &state, |guard| {
                    guard.set_status("选项：尚未打开书籍（请先在书架导入并打开 TXT）");
                }),
            }
        }
    });
    ui.on_toc_toggle({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            mutate_and_refresh(&ui, &state, |guard| {
                guard.toggle_toc_panel();
            });
            if let Some(session) = lock(&reader).as_ref() {
                project_toc(&ui, &state, session, None);
            }
        }
    });
    ui.on_toc_scroll({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let weak = ui.as_weak();
        move |delta| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let guard = lock(&reader);
            let Some(session) = guard.as_ref() else {
                return;
            };
            let first = lock(&state).reader().toc_window_first as i64;
            let step = (TOC_WINDOW as i64) / 2;
            let next = (first + delta as i64 * step).max(0) as u32;
            project_toc(&ui, &state, session, Some(next));
        }
    });
    ui.on_toc_wheel({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let weak = ui.as_weak();
        move |delta| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let rows = if delta > 0.0 {
                1
            } else if delta < 0.0 {
                -1
            } else {
                return;
            };
            let guard = lock(&reader);
            let Some(session) = guard.as_ref() else {
                return;
            };
            let first = lock(&state).reader().toc_window_first as i64;
            let next = (first + rows).max(0) as u32;
            project_toc(&ui, &state, session, Some(next));
        }
    });
    ui.on_toc_seek({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let weak = ui.as_weak();
        move |pointer_y, track_height| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let guard = lock(&reader);
            let Some(session) = guard.as_ref() else {
                return;
            };
            if track_height <= 0.0 {
                return;
            }
            let ratio = (pointer_y / track_height).clamp(0.0, 1.0);
            let match_total = lock(&state).reader().toc_match_total;
            let visible = TOC_WINDOW.min(match_total);
            let max_first = match_total.saturating_sub(visible);
            let first = (ratio * max_first as f32).round() as u32;
            project_toc(&ui, &state, session, Some(first));
        }
    });
    ui.on_toc_search_edited({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let weak = ui.as_weak();
        move |text| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            mutate_and_refresh(&ui, &state, |guard| {
                guard.set_toc_search(text.as_str());
            });
            if let Some(session) = lock(&reader).as_ref() {
                project_toc(&ui, &state, session, None);
            }
        }
    });
    ui.on_chapter_jump({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let tracker = Arc::clone(&tracker);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move |sequence| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Ok(sequence) = sequence.as_str().trim().parse::<u32>() else {
                mutate_and_refresh(&ui, &state, |guard| {
                    guard.set_status("章节跳转：章节序号非法，本次请求已忽略");
                });
                return;
            };
            let mut guard = lock(&reader);
            if let Some(session) = guard.as_mut() {
                if let Err(error) =
                    jump_to_chapter(&ui, &state, &db_path, session, &tracker, sequence)
                {
                    set_status_text(&ui, &state, "章节跳转", &error.message());
                }
            }
        }
    });
    ui.on_marks_toggle({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            mutate_and_refresh(&ui, &state, |guard| {
                guard.toggle_marks_panel();
            });
            if let Some(session) = lock(&reader).as_ref() {
                project_marks(&ui, &state, &db_path, session, None);
            }
        }
    });
    ui.on_marks_scroll({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move |delta| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let guard = lock(&reader);
            let Some(session) = guard.as_ref() else {
                return;
            };
            let first = lock(&state).reader().mark_window_first as i64;
            let step = (bookmarks::MARKS_PER_PAGE as i64) / 2;
            let next = (first + delta as i64 * step).max(0) as u32;
            project_marks(&ui, &state, &db_path, session, Some(next));
        }
    });
    ui.on_mark_search_edited({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move |text| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            mutate_and_refresh(&ui, &state, |guard| {
                guard.set_mark_search(text.as_str());
            });
            if let Some(session) = lock(&reader).as_ref() {
                project_marks(&ui, &state, &db_path, session, None);
            }
        }
    });
    ui.on_mark_add({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let guard = lock(&reader);
            if let Some(session) = guard.as_ref() {
                add_mark_at_current(&ui, &state, &db_path, session, None);
            }
        }
    });
    ui.on_mark_select({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move |id| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let id = id.as_str().trim().to_string();
            mutate_and_refresh(&ui, &state, |guard| {
                guard.set_mark_selected(id);
            });
            if let Some(session) = lock(&reader).as_ref() {
                project_marks(&ui, &state, &db_path, session, None);
            }
        }
    });
    ui.on_mark_jump({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let tracker = Arc::clone(&tracker);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move |id| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Ok(id) = id.as_str().trim().parse::<i64>() else {
                mutate_and_refresh(&ui, &state, |guard| {
                    guard.set_status("书签跳转：书签标识非法，本次请求已忽略");
                });
                return;
            };
            let mut guard = lock(&reader);
            if let Some(session) = guard.as_mut() {
                jump_to_mark(&ui, &state, &db_path, session, &tracker, id);
            }
        }
    });
    ui.on_mark_rename({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move |title| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let selected = lock(&state).mark_selected().to_string();
            let Ok(id) = selected.trim().parse::<i64>() else {
                mutate_and_refresh(&ui, &state, |guard| {
                    guard.set_status("重命名：请先选中一个书签");
                });
                return;
            };
            let guard = lock(&reader);
            if let Some(session) = guard.as_ref() {
                rename_selected_mark(&ui, &state, &db_path, session, id, title.as_str());
            }
        }
    });
    // S7：删除书签为**行内两段确认**（第一次点击进入待确认态，第二次才真正删除）。
    ui.on_mark_delete({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move |id| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let id = id.as_str().trim().to_string();
            mutate_and_refresh(&ui, &state, |guard| {
                guard.set_mark_delete_pending(id);
            });
            if let Some(session) = lock(&reader).as_ref() {
                project_marks(&ui, &state, &db_path, session, None);
            }
        }
    });
    ui.on_mark_delete_confirm({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move |id| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Ok(id) = id.as_str().trim().parse::<i64>() else {
                return;
            };
            let guard = lock(&reader);
            if let Some(session) = guard.as_ref() {
                delete_mark(&ui, &state, &db_path, session, id);
            }
        }
    });
    ui.on_mark_delete_cancel({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            mutate_and_refresh(&ui, &state, |guard| {
                guard.set_mark_delete_pending(String::new());
            });
            if let Some(session) = lock(&reader).as_ref() {
                project_marks(&ui, &state, &db_path, session, None);
            }
        }
    });
    ui.on_rename_text_edited({
        let state = Arc::clone(&state);
        let weak = ui.as_weak();
        move |text| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            mutate_and_refresh(&ui, &state, |guard| {
                guard.set_mark_rename_text(text.as_str());
            });
        }
    });
    ui.on_overlay_click({
        let state = Arc::clone(&state);
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            mutate_and_refresh(&ui, &state, |guard| {
                // 点击面板外：关闭目录 / 书签面板（O4）。
                guard.set_toc_open(false);
                guard.toggle_marks_panel();
                if guard.reader().marks_open {
                    guard.toggle_marks_panel();
                }
            });
        }
    });

    // ---- 弹窗与 Toast ----
    ui.on_dialog_cancel({
        let state = Arc::clone(&state);
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            mutate_and_refresh(&ui, &state, |guard| {
                guard.close_dialog();
            });
        }
    });
    ui.on_dialog_choice_apply({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let tracker = Arc::clone(&tracker);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move |index| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let rows = lock(&state).dialog().choice_rows.clone();
            let Some(row) = rows.get(index.max(0) as usize) else {
                mutate_and_refresh(&ui, &state, |guard| {
                    guard.set_status("选项：选择无效，本次请求已忽略");
                });
                return;
            };
            let encoding = crate::pagination::parse_encoding(row.name.as_str());
            let mut guard = lock(&reader);
            match guard.as_mut() {
                Some(session) => {
                    apply_encoding_override(&ui, &state, &db_path, session, &tracker, encoding)
                }
                None => mutate_and_refresh(&ui, &state, |guard| {
                    guard.close_dialog();
                    guard.set_status("选项：尚未打开书籍".to_string());
                }),
            }
        }
    });
    ui.on_toast_action_clicked({
        let state = Arc::clone(&state);
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let action = lock(&state).toast_action().to_string();
            match action.as_str() {
                // 当前唯一带动作的 Toast：提示已移出（动作 = 重新打开书架查看）。
                "shelf" => {
                    mutate_and_refresh(&ui, &state, |guard| {
                        guard.set_screen(Screen::Shelf);
                        guard.set_status("已回到书架");
                    });
                }
                _ => {}
            }
        }
    });

    // ---- 设置 ----
    ui.on_settings_cycle({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let settings_slot = Arc::clone(&settings_slot);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move |key| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let key = key.as_str().to_string();
            let mut guard = lock(&reader);
            let session = guard.as_mut();
            apply_settings_value(
                &ui,
                &state,
                &db_path,
                &settings_slot,
                session,
                &key,
                SettingsChange::Delta(1),
            );
        }
    });
    ui.on_settings_set_int({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let settings_slot = Arc::clone(&settings_slot);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move |key, value| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let key = key.as_str().to_string();
            let mut guard = lock(&reader);
            let session = guard.as_mut();
            apply_settings_value(
                &ui,
                &state,
                &db_path,
                &settings_slot,
                session,
                &key,
                SettingsChange::ControlInt(value),
            );
        }
    });
    ui.on_settings_set_text({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let settings_slot = Arc::clone(&settings_slot);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move |key, value| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let key = key.as_str().to_string();
            let value = value.as_str().trim().to_string();
            // 数值框：非整数输入如实报错并恢复原值（重新投影即为恢复）。
            let Ok(number) = value.parse::<i32>() else {
                set_status_text(&ui, &state, "设置", &format!("`{value}` 不是整数"));
                refresh(&ui, &state);
                return;
            };
            let mut guard = lock(&reader);
            let session = guard.as_mut();
            apply_settings_value(
                &ui,
                &state,
                &db_path,
                &settings_slot,
                session,
                &key,
                SettingsChange::ControlInt(number),
            );
        }
    });
    ui.on_settings_set_index({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let settings_slot = Arc::clone(&settings_slot);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move |key, index| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let key = key.as_str().to_string();
            let mut guard = lock(&reader);
            let session = guard.as_mut();
            apply_settings_value(
                &ui,
                &state,
                &db_path,
                &settings_slot,
                session,
                &key,
                SettingsChange::OptionIndex(index),
            );
        }
    });
    ui.on_settings_toggle({
        let state = Arc::clone(&state);
        let settings_slot = Arc::clone(&settings_slot);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move |key, checked| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let key = key.as_str().to_string();
            apply_settings_value(
                &ui,
                &state,
                &db_path,
                &settings_slot,
                None,
                &key,
                SettingsChange::ControlInt(i32::from(checked)),
            );
        }
    });
    ui.on_settings_clear_index({
        let state = Arc::clone(&state);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            match shelf_ops::index_clear_preview(&db_path) {
                Ok(preview) => {
                    mutate_and_refresh(&ui, &state, |guard| {
                        guard.open_confirm(
                            "clear-index",
                            "",
                            "清理可重建索引",
                            preview.dialog_body(),
                            "备份建议：先复制 library.db 再清理；清理不会触碰原始 TXT。",
                            "",
                            "清理索引",
                            true,
                        );
                    });
                }
                Err(error) => set_status_text(&ui, &state, "清理索引", &error.to_string()),
            }
        }
    });

    // ---- 键盘 / 滚轮 ----
    ui.on_key_input({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let tracker = Arc::clone(&tracker);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move |key_text, scope_focus_lost| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            if key_text == input::escape_key_text() {
                // Esc：先关弹窗 → 再退出沉浸 → 最后关闭面板（O4）。
                if lock(&state).dialog_kind() != 0 {
                    mutate_and_refresh(&ui, &state, |guard| {
                        guard.close_dialog();
                    });
                    return;
                }
                if lock(&state).immersive() {
                    let mut guard = lock(&reader);
                    match guard.as_mut() {
                        Some(session) => {
                            apply_immersive(&ui, &state, &db_path, session, &tracker, false)
                        }
                        None => mutate_and_refresh(&ui, &state, |guard| {
                            guard.set_immersive(false);
                        }),
                    }
                    return;
                }
                let mut changed = false;
                mutate_and_refresh(&ui, &state, |guard| {
                    if guard.reader().toc_open {
                        guard.set_toc_open(false);
                        changed = true;
                    } else if guard.reader().marks_open {
                        guard.toggle_marks_panel();
                        changed = true;
                    }
                });
                if changed {
                    log_line("NR-INPUT kind=key key=escape intent=close-panel");
                }
                return;
            }
            let marks_open = lock(&state).reader().marks_open;
            let text_input_focused = input::text_input_focused(scope_focus_lost, marks_open);
            dispatch_key_input(
                &ui,
                &state,
                &db_path,
                &reader,
                &tracker,
                key_text.as_str(),
                text_input_focused,
            );
        }
    });
    ui.on_wheel_input({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let tracker = Arc::clone(&tracker);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move |delta_x, delta_y| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            dispatch_wheel_input(&ui, &state, &db_path, &reader, &tracker, delta_x, delta_y);
        }
    });

    // ---- 后台结果回到 UI 线程 ----
    let bridge = Arc::new(UiBridge::new(
        Arc::clone(&state),
        Arc::new(SlintEventLoopExecutor),
        Arc::new({
            let weak = ui.as_weak();
            move |snapshot: UiSnapshot| {
                if let Some(ui) = weak.upgrade() {
                    apply_snapshot(&ui, &snapshot);
                }
            }
        }),
    ));
    bridge.project_now();

    // 书架后台任务：先校正来源状态（只动状态列），再做书架读取。
    let (channel, worker) = JobChannel::start(
        Arc::clone(&bridge),
        Box::new(source_status::shelf_job_with_source_check),
    );
    let channel = Arc::new(channel);

    // ---- 导入与失效重新关联（选择器可注入；系统对话框本身是模态的）----
    let picker: Arc<dyn FilePicker> = Arc::from(import::picker_from_env());

    ui.on_shelf_import({
        let state = Arc::clone(&state);
        let weak = ui.as_weak();
        let picker = Arc::clone(&picker);
        let channel = Arc::clone(&channel);
        let db_path = db_path.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            // S11：导入中界面按真实步骤投影（当前导入链是单文件同步流程，
            // 完成前界面不重绘；此处如实投影状态，供异步化后直接复用）。
            let steps = vec![
                StepRowView {
                    label: "读取文件".to_string(),
                    state: 1,
                },
                StepRowView {
                    label: "校验与指纹".to_string(),
                    state: 1,
                },
                StepRowView {
                    label: "写入书架".to_string(),
                    state: 0,
                },
                StepRowView {
                    label: "建立索引（首次打开时）".to_string(),
                    state: 0,
                },
            ];
            mutate_and_refresh(&ui, &state, |guard| {
                guard.set_status("导入：正在等待文件选择（只接受 .txt；校验失败不会留下半成品记录）");
                guard.set_import_view(ImportView {
                    file: String::new(),
                    size: String::new(),
                    percent: 0,
                    steps: steps.clone(),
                    note: "导入完成后才能继续；取消对话框不会留下任何记录。".to_string(),
                });
            });
            let result = import::run_import_flow(&db_path, picker.as_ref(), import::now_unix());
            match &result {
                Ok(import::ImportFlow::Imported(report)) => {
                    let file = report
                        .path
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_default();
                    mutate_and_refresh(&ui, &state, |guard| {
                        guard.set_import_view(ImportView {
                            file,
                            size: shelf::human_size(report.size),
                            percent: 100,
                            steps: vec![
                                StepRowView {
                                    label: "读取文件".to_string(),
                                    state: 2,
                                },
                                StepRowView {
                                    label: "校验与指纹".to_string(),
                                    state: 2,
                                },
                                StepRowView {
                                    label: "写入书架".to_string(),
                                    state: 2,
                                },
                                StepRowView {
                                    label: "建立索引（首次打开时）".to_string(),
                                    state: 0,
                                },
                            ],
                            note: "导入完成后才能继续。".to_string(),
                        });
                        guard.set_status(import::status_text_for(&result));
                        guard.set_screen(Screen::Shelf);
                    });
                }
                Ok(import::ImportFlow::Cancelled) => {
                    mutate_and_refresh(&ui, &state, |guard| {
                        guard.set_status(import::status_text_for(&result));
                    });
                }
                Err(error) => {
                    // S12：错误页用用户语言 + 可折叠技术详情（错误码 / 路径 / 判定说明）。
                    let path = error
                        .path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "（无路径）".to_string());
                    let details = format!(
                        "错误码：{}\n路径：{path}\n判定说明：{}",
                        error.kind.code(),
                        error.message()
                    );
                    let (title, reason) = match error.kind {
                        import::ImportErrorKind::NotTxt => (
                            "这不是一个 TXT 文件",
                            "本版本只支持单个 .txt 文件；请选择 TXT 后重试。",
                        ),
                        import::ImportErrorKind::MissingFile => (
                            "找不到所选文件",
                            "文件可能已被移动或删除；请重新选择。",
                        ),
                        import::ImportErrorKind::EmptyFile => (
                            "这个文件是空的",
                            "0 字节的文件没有可阅读内容；请换一个文件。",
                        ),
                        import::ImportErrorKind::Unreadable => (
                            "这个文件打不开",
                            "可能是权限问题；请检查文件权限后重试。",
                        ),
                        import::ImportErrorKind::NoPicker | import::ImportErrorKind::PickerFailed => (
                            "无法打开文件选择对话框",
                            "系统里没有可用的文件对话框程序；请安装 zenity 或 kdialog 后重试。",
                        ),
                        _ => (
                            "导入未完成",
                            "文件校验或写入书架时出现问题；未创建任何书架记录。",
                        ),
                    };
                    mutate_and_refresh(&ui, &state, |guard| {
                        guard.set_error_view(title.to_string(), reason.to_string(), details.clone());
                        guard.set_status(format!(
                            "导入失败 · 未创建书架记录（{}）",
                            error.kind.code()
                        ));
                        guard.set_screen(Screen::Error);
                    });
                }
            }
            submit_shelf_refresh(&state, &channel, &db_path);
        }
    });
    ui.on_import_retry({
        let state = Arc::clone(&state);
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            // 重新选择文件：回到书架后再触发导入（等价于点「导入 TXT…」）。
            mutate_and_refresh(&ui, &state, |guard| {
                guard.set_screen(Screen::Shelf);
                guard.set_status("请重新点击「导入 TXT…」选择文件");
            });
        }
    });
    ui.on_import_back({
        let state = Arc::clone(&state);
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            mutate_and_refresh(&ui, &state, |guard| {
                guard.set_screen(Screen::Shelf);
                guard.set_status("已返回书架");
            });
        }
    });

    // 书架整行「阅读」：按书籍 id 打开（与启动驱动 `NR_READER_BOOK` 同一函数）。
    ui.on_shelf_open_book({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let settings_slot = Arc::clone(&settings_slot);
        let viewport_slot = Arc::clone(&viewport_slot);
        let font_slot = Arc::clone(&font_slot);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move |book_id| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            open_reader_from_shelf(
                &ui,
                &state,
                &reader,
                &settings_slot,
                &viewport_slot,
                &font_slot,
                &db_path,
                book_id.as_str(),
            );
        }
    });
    ui.on_shelf_search_edited({
        let state = Arc::clone(&state);
        let weak = ui.as_weak();
        move |text| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            mutate_and_refresh(&ui, &state, |guard| {
                guard.set_shelf_search(text.as_str());
            });
        }
    });
    ui.on_shelf_view_set({
        let state = Arc::clone(&state);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move |view| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            mutate_and_refresh(&ui, &state, |guard| {
                guard.set_shelf_view(view);
            });
            if let Err(error) = settings::save_shelf_view(&db_path, view, import::now_unix()) {
                set_status_text(&ui, &state, "书架视图", &error.message());
            }
        }
    });
    ui.on_shelf_sort_set({
        let state = Arc::clone(&state);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move |index| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let sort = {
                let mut guard = lock(&state);
                guard.set_shelf_sort(index);
                guard.shelf_sort()
            };
            refresh(&ui, &state);
            if let Err(error) = settings::save_shelf_sort(&db_path, sort, import::now_unix()) {
                set_status_text(&ui, &state, "书架排序", &error.message());
            }
        }
    });
    ui.on_shelf_sort_cycle({
        let state = Arc::clone(&state);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let sort = {
                let mut guard = lock(&state);
                guard.cycle_shelf_sort();
                guard.shelf_sort()
            };
            refresh(&ui, &state);
            if let Err(error) = settings::save_shelf_sort(&db_path, sort, import::now_unix()) {
                set_status_text(&ui, &state, "书架排序", &error.message());
            }
        }
    });
    ui.on_shelf_row_menu({
        let state = Arc::clone(&state);
        let weak = ui.as_weak();
        move |index| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            mutate_and_refresh(&ui, &state, |guard| {
                guard.set_shelf_menu_index(index);
            });
        }
    });
    ui.on_shelf_row_action({
        let state = Arc::clone(&state);
        let weak = ui.as_weak();
        let picker = Arc::clone(&picker);
        let channel = Arc::clone(&channel);
        let db_path = db_path.clone();
        move |action, id| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let action = action.as_str().to_string();
            let id_text = id.as_str().to_string();
            let Ok(book_id) = id_text.trim().parse::<i64>() else {
                set_status_text(&ui, &state, "行操作", "书籍标识非法，本次请求已忽略");
                return;
            };
            let title = lock(&state)
                .shelf_rows()
                .iter()
                .find(|row| row.id == id_text)
                .map(|row| row.title.clone())
                .unwrap_or_else(|| format!("book_id={book_id}"));
            match action.as_str() {
                "relink" => {
                    mutate_and_refresh(&ui, &state, |guard| {
                        guard.set_shelf_menu_index(-2);
                        guard.set_status("重新关联：请选择同一本书的 TXT；内容不一致时不会写入，进度与书签保留");
                    });
                    let result = source_status::relink_flow(
                        &db_path,
                        book_id,
                        picker.as_ref(),
                        source_status::DEFAULT_RELINK_DECISION,
                        import::now_unix(),
                    );
                    mutate_and_refresh(&ui, &state, |guard| {
                        guard.set_status(source_status::relink_status_text(&result));
                    });
                    submit_shelf_refresh(&state, &channel, &db_path);
                }
                "rename" => {
                    mutate_and_refresh(&ui, &state, |guard| {
                        guard.set_shelf_menu_index(-2);
                        guard.open_rename("重命名书架显示名", &title, &id_text);
                    });
                }
                "remove" => {
                    mutate_and_refresh(&ui, &state, |guard| {
                        guard.set_shelf_menu_index(-2);
                        guard.open_confirm(
                            "remove",
                            &id_text,
                            "移出书架",
                            &format!("将把《{title}》移出书架；进度与书签会保留。\n原始 TXT 文件不会被修改或删除；再次导入同一文件即可恢复。"),
                            "脚注：若要连同进度与书签一起删除，请在行级菜单选择「彻底删除记录…」。",
                            "同时删除该书的章节索引与分页缓存",
                            "移出书架",
                            true,
                        );
                    });
                }
                "purge" => {
                    mutate_and_refresh(&ui, &state, |guard| {
                        guard.set_shelf_menu_index(-2);
                        guard.open_confirm(
                            "purge",
                            &id_text,
                            "彻底删除记录",
                            &format!("将删除《{title}》的全部本地记录：来源、章节索引、阅读进度与书签。\n原始 TXT 文件不会被修改或删除。此操作不可撤销。"),
                            "",
                            "",
                            "彻底删除",
                            true,
                        );
                    });
                }
                _ => {}
            }
        }
    });
    ui.on_shelf_relink_all({
        let state = Arc::clone(&state);
        let weak = ui.as_weak();
        let picker = Arc::clone(&picker);
        let channel = Arc::clone(&channel);
        let db_path = db_path.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let ids: Vec<i64> = lock(&state)
                .shelf_rows()
                .iter()
                .filter(|row| row.needs_relink)
                .filter_map(|row| row.id.parse::<i64>().ok())
                .collect();
            if ids.is_empty() {
                set_status_text(&ui, &state, "重新关联", "当前没有待重新关联的条目");
                return;
            }
            let results =
                shelf_ops::relink_all(&db_path, &ids, picker.as_ref(), import::now_unix());
            mutate_and_refresh(&ui, &state, |guard| {
                guard.set_status(format!("重新关联（{} 本）：{}", ids.len(), results.join("；")));
                guard.ignore_shelf_notice();
            });
            submit_shelf_refresh(&state, &channel, &db_path);
        }
    });
    ui.on_shelf_notice_dismiss({
        let state = Arc::clone(&state);
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            mutate_and_refresh(&ui, &state, |guard| {
                // 「忽略」只收起通知：状态是持久事实，下次启动仍会提示。
                guard.ignore_shelf_notice();
                guard.set_status("已忽略失效提示（状态仍会保留，下次启动时再次提示）");
            });
        }
    });
    ui.on_dialog_confirm({
        let state = Arc::clone(&state);
        let weak = ui.as_weak();
        let channel = Arc::clone(&channel);
        let db_path = db_path.clone();
        move |checked| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let dialog = lock(&state).dialog().clone();
            match dialog.confirm_action.as_str() {
                "remove" => {
                    let Ok(book_id) = dialog.confirm_book_id.parse::<i64>() else {
                        return;
                    };
                    match shelf_ops::remove_book(&db_path, book_id, checked, import::now_unix()) {
                        Ok(text) => {
                            mutate_and_refresh(&ui, &state, |guard| {
                                guard.close_dialog();
                                guard.set_status(text.clone());
                            });
                            show_toast(&ui, &state, "已移出书架（记录与进度保留）", "去书架", "shelf");
                        }
                        Err(error) => set_status_text(&ui, &state, "移出书架", &error.to_string()),
                    }
                    submit_shelf_refresh(&state, &channel, &db_path);
                }
                "purge" => {
                    let Ok(book_id) = dialog.confirm_book_id.parse::<i64>() else {
                        return;
                    };
                    match shelf_ops::purge_book(&db_path, book_id) {
                        Ok(text) => {
                            mutate_and_refresh(&ui, &state, |guard| {
                                guard.close_dialog();
                                guard.set_status(text.clone());
                            });
                            show_toast(&ui, &state, "已彻底删除记录（原始 TXT 未动）", "", "");
                        }
                        Err(error) => set_status_text(&ui, &state, "彻底删除", &error.to_string()),
                    }
                    submit_shelf_refresh(&state, &channel, &db_path);
                }
                "clear-index" => match shelf_ops::clear_index(&db_path) {
                    Ok(report) => {
                        let text = shelf_ops::IndexClearPreview::result_text(&report);
                        mutate_and_refresh(&ui, &state, |guard| {
                            guard.close_dialog();
                            guard.set_status(text.clone());
                        });
                        show_toast(&ui, &state, text.clone(), "", "");
                        submit_shelf_refresh(&state, &channel, &db_path);
                    }
                    Err(error) => set_status_text(&ui, &state, "清理索引", &error.to_string()),
                },
                _ => {
                    mutate_and_refresh(&ui, &state, |guard| {
                        guard.close_dialog();
                    });
                }
            }
        }
    });
    ui.on_dialog_rename_save({
        let state = Arc::clone(&state);
        let weak = ui.as_weak();
        let channel = Arc::clone(&channel);
        let db_path = db_path.clone();
        move |name| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let dialog = lock(&state).dialog().clone();
            match shelf_ops::rename_book(&db_path, dialog.rename_book_id.parse().unwrap_or(-1), name.as_str(), import::now_unix()) {
                Ok(text) => {
                    mutate_and_refresh(&ui, &state, |guard| {
                        guard.close_dialog();
                        guard.set_status(text.clone());
                    });
                    show_toast(&ui, &state, "已重命名", "", "");
                    submit_shelf_refresh(&state, &channel, &db_path);
                }
                Err(error) => set_status_text(&ui, &state, "重命名", &error.to_string()),
            }
        }
    });

    // ---- 程序化验证：启动期驱动（走与界面回调同一批函数）----
    if let Some(result) = import::startup_import(&db_path) {
        mutate_and_refresh(&ui, &state, |guard| {
            guard.set_status(match &result {
                Ok(report) => format!("启动期导入（程序化验证）：{}", report.status_text()),
                Err(error) => format!(
                    "启动期导入失败（{}）：{}",
                    error.kind.code(),
                    error.message()
                ),
            });
        });
    }
    if let Some(result) = source_status::startup_relink(&db_path) {
        mutate_and_refresh(&ui, &state, |guard| {
            guard.set_status(format!(
                "启动期重新关联（程序化验证）：{}",
                source_status::relink_status_text(&result)
            ));
        });
    }

    let request = lock(&state).next_shelf_request(db_path.clone());
    if !channel.submit(request) {
        bridge.project_now();
    }

    // ---- 窗口映射完成后取真实尺寸、完成首屏分页并跑启动期驱动 ----
    slint::Timer::single_shot(Duration::from_millis(STARTUP_ALIGN_MS), {
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let tracker = Arc::clone(&tracker);
        let settings_slot = Arc::clone(&settings_slot);
        let viewport_slot = Arc::clone(&viewport_slot);
        let font_slot = Arc::clone(&font_slot);
        let channel = Arc::clone(&channel);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            // 以**当前真实窗口尺寸**为准：槽位里可能仍是窗口管理器套用首选尺寸
            // 之前的瞬时尺寸（曾导致首屏 NR-PAGE 记为 560×400）。
            let size = ui.window().size();
            let scale = ui.window().scale_factor().max(1.0);
            let mut viewport = MappedViewport::new(
                (size.width as f32 / scale).round() as u32,
                (size.height as f32 / scale).round() as u32,
            )
            .with_scale_milli((scale * 1_000.0).round() as u32);
            if !viewport.is_usable() {
                viewport = *lock(&viewport_slot);
            }
            let measured = PlatformFontMetrics::from_px(
                ui.get_body_ascent().max(0.0).round() as u32,
                ui.get_body_descent().max(0.0).round() as u32,
            );
            let font = if measured.is_known() {
                measured
            } else {
                *lock(&font_slot)
            };
            let loaded = lock(&settings_slot).clone();

            // 0) 书架视图/排序/搜索驱动（与界面同一入口）。
            if let Some(view) = env_text(SHELF_VIEW_ENV) {
                let value = if view == "cards" { 1 } else { 0 };
                mutate_and_refresh(&ui, &state, |guard| {
                    guard.set_shelf_view(value);
                });
            }
            if let Some(sort) = env_text(SHELF_SORT_ENV) {
                let value = match sort.as_str() {
                    "title" => app_state::SORT_TITLE,
                    "imported" => app_state::SORT_IMPORTED,
                    _ => app_state::SORT_RECENT,
                };
                mutate_and_refresh(&ui, &state, |guard| {
                    guard.set_shelf_sort(value);
                });
            }
            if let Some(search) = env_text(SHELF_SEARCH_ENV) {
                mutate_and_refresh(&ui, &state, |guard| {
                    guard.set_shelf_search(search);
                });
            }
            if let Some(index) = env_i64(SHELF_MENU_ENV) {
                mutate_and_refresh(&ui, &state, |guard| {
                    guard.set_shelf_menu_index(index as i32);
                });
            }
            if let Some(action) = env_text(SHELF_ACTION_ENV) {
                if let Some((kind, id)) = action.split_once(':') {
                    ui.invoke_shelf_row_action(kind.into(), id.into());
                }
            }
            if env_flag(SHORTCUTS_ENV) {
                ui.invoke_shortcuts_toggle();
            }
            if env_flag(CLEAR_INDEX_ENV) {
                ui.invoke_settings_clear_index();
            }

            // 0a) 书架「阅读」入口（与整行点击**同一函数**）。
            if let Some(target) = env_text(SHELF_OPEN_ENV) {
                open_reader_from_shelf(
                    &ui,
                    &state,
                    &reader,
                    &settings_slot,
                    &viewport_slot,
                    &font_slot,
                    &db_path,
                    &target,
                );
            }
            let Some(selection) = reader_selection_from_env() else {
                return;
            };
            // 注意：`lock(&state)` 的临时 guard 会活到语句结束，绝不能出现在参数位置
            // （否则 open_reader 内投影时会对同一把锁再次加锁 → 自锁死）。
            let toolbar_open_now = lock(&state).project().reader.toolbar_open;
            let Some(mut session) =
                open_reader(&ui, &state, &db_path, viewport, font, toolbar_open_now, &loaded, &selection)
            else {
                return;
            };
            // 全局设置（按文本设置并持久化）。
            if let Some(pairs) = env_text(SETTINGS_SET_ENV) {
                match parse_setting_pairs(&pairs) {
                    Ok(pairs) => {
                        apply_settings_value(
                            &ui,
                            &state,
                            &db_path,
                            &settings_slot,
                            Some(&mut session),
                            &pairs
                                .first()
                                .map(|(key, _)| key.clone())
                                .unwrap_or_default(),
                            SettingsChange::Pairs(pairs),
                        );
                    }
                    Err(error) => set_status_text(&ui, &state, "设置", &error),
                }
            }
            // 章节跳转。
            if let Some(sequence) = env_i64(CHAPTER_JUMP_ENV) {
                if let Err(error) = jump_to_chapter(
                    &ui,
                    &state,
                    &db_path,
                    &mut session,
                    &tracker,
                    sequence.clamp(0, i64::from(u32::MAX)) as u32,
                ) {
                    set_status_text(&ui, &state, "章节跳转", &error.message());
                }
            }
            // 手动覆盖内容编码（选项弹窗「应用」的等价入口）。
            if let Some(name) = env_text(ENCODING_SELECT_ENV) {
                let encoding = crate::pagination::parse_encoding(&name);
                apply_encoding_override(&ui, &state, &db_path, &mut session, &tracker, encoding);
            }
            // 沉浸阅读（界面「沉浸阅读」按钮的等价入口）。
            if env_flag(IMMERSIVE_ENV) {
                apply_immersive(&ui, &state, &db_path, &mut session, &tracker, true);
            }
            // 沉浸态顶部悬停（浮出工具条）。
            if let Some(hover) = env_text(IMMERSIVE_HOVER_ENV) {
                ui.invoke_reader_zone_hover(hover == "1");
            }
            // 用户入口翻页（含进度保存）。
            if let Some(turns) = env_i64(UI_TURNS_ENV) {
                let _ = user_turns(
                    &ui,
                    &state,
                    &db_path,
                    &mut session,
                    &tracker,
                    turns.clamp(-10_000, 10_000) as i32,
                    "翻页",
                );
            }
            // 书签：新增 → 跳转 → 重命名 → 删除（走行内两段确认入口）。
            if let Some(title) = env_text(MARK_ADD_ENV) {
                add_mark_at_current(&ui, &state, &db_path, &session, Some(&title));
            }
            if let Some(id) = env_i64(MARK_JUMP_ENV) {
                jump_to_mark(&ui, &state, &db_path, &mut session, &tracker, id);
            }
            if let Some(spec) = env_text(MARK_RENAME_ENV) {
                if let Some((id, title)) = spec.split_once(':') {
                    if let Ok(id) = id.trim().parse::<i64>() {
                        rename_selected_mark(&ui, &state, &db_path, &session, id, title.trim());
                    }
                }
            }
            if let Some(id) = env_i64(MARK_DELETE_ENV) {
                let id = id.to_string();
                ui.invoke_mark_delete(id.clone().into());
                ui.invoke_mark_delete_confirm(id.into());
            }
            // 面板投影（目录 / 书签窗口）。
            match env_text(PANEL_ENV).as_deref() {
                Some("toc") => {
                    mutate_and_refresh(&ui, &state, |guard| {
                        guard.set_toc_open(true);
                    });
                    project_toc(&ui, &state, &session, None);
                }
                Some("marks") => {
                    mutate_and_refresh(&ui, &state, |guard| {
                        guard.toggle_marks_panel();
                    });
                    project_marks(&ui, &state, &db_path, &session, None);
                }
                _ => {}
            }
            if let Some(text) = env_text(PANEL_SEARCH_ENV) {
                let mode = env_text(PANEL_ENV);
                if mode.as_deref() == Some("marks") {
                    ui.invoke_mark_search_edited(text.clone().into());
                } else {
                    ui.invoke_toc_search_edited(text.into());
                }
            }
            // 进度条跳转（与界面进度条同一回调）。
            if let Some(ratio) = env_f64(PROGRESS_SEEK_ENV) {
                ui.invoke_reader_progress_seek(ratio as f32);
            }
            // 显式保存进度。
            if env_flag(SAVE_PROGRESS_ENV) {
                save_current(&ui, &state, &db_path, &session, &tracker, "保存进度");
            }
            // 字号变化（直接分页，不经过用户入口）。
            if let Some(font_px) = env_i64(READER_FONT_ENV) {
                match session
                    .runtime
                    .set_font_size_px(font_px.clamp(1, 400) as u32)
                {
                    Ok(result) => {
                        project_reader_outcome(&ui, &state, &session, &result.outcome, "字号变化")
                    }
                    Err(error) => set_status_text(&ui, &state, "字号变化", &error.message()),
                }
            }
            if let Some(turns) = env_i64(READER_TURNS_ENV) {
                let outcome = session
                    .runtime
                    .turn_pages(turns.clamp(-10_000, 10_000) as i32);
                project_reader_outcome(&ui, &state, &session, &outcome, "翻页");
                project_toc(&ui, &state, &session, None);
            }
            {
                let mut slot = lock(&reader);
                *slot = Some(session);
            }
            // 目录滚轮 / 滚动条：直接调用 Slint 侧回调（与用户操作**同一条路径**）。
            if let Some(delta) = env_f64(TOC_WHEEL_ENV) {
                ui.invoke_toc_wheel(delta as f32);
            }
            if let Some(ratio) = env_f64(TOC_SEEK_ENV) {
                ui.invoke_toc_seek((ratio as f32) * 1000.0, 1000.0);
            }
            if let Some(sequence) = env_i64(TOC_JUMP_ENV) {
                let text: slint::SharedString = sequence.to_string().into();
                ui.invoke_chapter_jump(text);
            }
            // 弹窗确认 / 取消（与界面弹窗按钮同一回调）。
            if let Some(value) = env_text(DIALOG_CONFIRM_ENV) {
                ui.invoke_dialog_confirm(value == "1");
            }
            if env_flag(DIALOG_CANCEL_ENV) {
                ui.invoke_dialog_cancel();
            }
            // 输入入口程序化验证（投递真实窗口事件）。
            let events = dispatch_input_events(
                &ui,
                &state,
                &db_path,
                &reader,
                &tracker,
                env_text(KEY_EVENTS_ENV).as_deref(),
                env_text(WHEEL_EVENTS_ENV).as_deref(),
            );
            if events > 0 {
                log_line(&format!(
                    "NR-INPUT-DONE events={events} text_focus={}",
                    u8::from(env_flag(TEXT_FOCUS_ENV))
                ));
            }
            let _ = &channel;
        }
    });

    if let Some(ms) = exit_after_ms_from_env() {
        slint::Timer::single_shot(Duration::from_millis(ms), || {
            let _ = slint::quit_event_loop();
        });
    }

    let result = ui.run();

    // 退出前保存「最近一次变化后的位置」（用户入口已保存过时这里是幂等空写）。
    if let Some(session) = lock(&reader).as_ref() {
        let dirty = lock(&tracker).is_dirty();
        if dirty {
            save_current(&ui, &state, &db_path, session, &tracker, "退出");
        }
    }

    // 退出清理：先释放 UI（回调随组件释放），再关闭后台通道，避免残留进程。
    drop(ui);
    drop(channel);
    let _ = worker.join();

    result
}

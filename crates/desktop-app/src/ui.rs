//! Slint UI 绑定：UI 定义的唯一来源是 `desktop/ui/app.slint`。
//!
//! `slint!` 宏按**相对本文件**的路径解析 `../../desktop/ui/app.slint`，因此不存在
//! 「内联副本 + 文件副本」两份定义；`scripts/verify-desktop-scope.py` 还会禁止
//! `crates/desktop-app/` 下出现任何 `.slint` 文件。
//!
//! 本模块是唯一的双向通道：
//!
//! - **Rust → Slint**：`apply_layout_metrics`（版式常量注入）与 `apply_snapshot`
//!   （状态投影）只写 Slint 属性，不在 UI 里保留任何业务状态；
//! - **Slint → Rust**：回调只上报意图（序号/键/增量/id），页面语义与业务判断全部回到
//!   `app_state` 与冻结核心；后台结果经 `bridge` 回到 UI 线程后再投影。
//!
//! DT4 补充（章节 / 进度 / 书签 / 全局设置 / 重启恢复）：
//!
//! - 阅读会话是 [`ReadingSession`]（分页运行时 + 章节目录），章节跳跃、书签跳转与进度恢复
//!   共用同一个入口 [`ReadingSession::jump_to`]（以**字节偏移**重新测量，不用页码）；
//! - 进度保存只发生在**用户入口**（翻页 / 跳转 / 「保存进度」按钮 / 退出时脏位置），
//!   页面内容与位置身份一律来自 `ContentPosition`；
//! - 程序化驱动（`NR_*` 环境变量）走与界面回调**同一批函数**，不另建一条旁路。

slint::slint! {
    // `UiMetrics` / `Screens` 是 `app.slint` 中的全局单例，必须显式 export
    // 才能在 Rust 侧以 `ui.global::<UiMetrics>()` / `Screens.shelf` 引用。
    export { AppWindow, BookRow, ChoiceRow, MarkRow, Screens, SettingsRow, TocRow, UiMetrics } from "../../desktop/ui/app.slint";
}

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use slint::{ModelRc, VecModel};

use crate::app_state::{
    AppState, EncodingRow as EncodingRowData, Screen, SettingsView, TocRow as TocRowData,
    UiSnapshot,
};
use crate::bookmarks;
use crate::bridge::{JobChannel, SlintEventLoopExecutor, UiBridge};
use crate::chapters::{self, ChapterToc};
use crate::import::{self, FilePicker};
use crate::input::{self, PageTurn};
use crate::layout::LayoutMetrics;
use crate::pagination::{open_book, PageOutcome, ReaderRuntime};
use crate::progress::{self, ProgressTracker, SavedPosition};
use crate::reflow::{MappedViewport, PlatformFontMetrics};
use crate::settings::{self, GlobalSettings};
use crate::shelf;
use crate::source_status;

/// 启动页面环境变量（程序化验证用）：`shelf` / `reader` / `settings`。
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
/// 启动后展开阅读菜单（`1`；程序化验证用，触发一次视口高度变化重排）。
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
/// 启动后删除书签（书签 id，DT4）。
pub const MARK_DELETE_ENV: &str = "NR_MARK_DELETE";
/// 启动后应用全局设置（`key=value,key=value`；同时持久化，DT4）。
pub const SETTINGS_SET_ENV: &str = "NR_SETTINGS_SET";
/// 启动后**手动覆盖内容编码**（值为编码展示名，如 `GBK`；与界面「编码」面板的「选用」同一入口）。
pub const ENCODING_SELECT_ENV: &str = "NR_ENCODING_SELECT";
/// 书架行「阅读」入口（值为书籍 id；与书架「阅读」按钮**同一函数**）。
pub const SHELF_OPEN_ENV: &str = "NR_SHELF_OPEN";
/// 目录滚动条定位（`0..1` 比例；与界面滚动条**同一回调**）。
pub const TOC_SEEK_ENV: &str = "NR_TOC_SEEK";
/// 目录滚轮（逻辑像素 delta-y；与界面滚轮**同一回调**）。
pub const TOC_WHEEL_ENV: &str = "NR_TOC_WHEEL";
/// 目录行「跳转」（值为章节序号；与行内「跳转」按钮**同一回调**，用于验证「跳转后收起目录」）。
pub const TOC_JUMP_ENV: &str = "NR_TOC_JUMP";
/// 启动后进入沉浸阅读（`1` = 进入；界面「沉浸阅读」按钮的等价入口）。
pub const IMMERSIVE_ENV: &str = "NR_IMMERSIVE";
/// 启动后保存一次进度（用户「保存进度」按钮的等价入口，DT4）。
pub const SAVE_PROGRESS_ENV: &str = "NR_SAVE_PROGRESS";
/// 展开面板（`toc` / `marks`；程序化验证用，DT4）。
pub const PANEL_ENV: &str = "NR_PANEL";
/// 程序化验证（DT8）：按键序列（便携名称，逗号分隔；如 `page-down,page-up`）。
///
/// 事件**走 Slint 真实窗口事件通道**（`Window::dispatch_event` → 焦点 `FocusScope` →
/// `key-pressed` 回调 → 与界面完全相同的判定与翻页入口），不是旁路函数调用。
pub const KEY_EVENTS_ENV: &str = "NR_KEY_EVENTS";
/// 程序化验证（DT8）：滚轮增量序列（逻辑像素，逗号分隔；正值向下滚）。
pub const WHEEL_EVENTS_ENV: &str = "NR_WHEEL_EVENTS";
/// 程序化验证（DT8）：模拟「书签重命名输入框持有焦点」（`1`），用于「输入框聚焦不翻页」负向用例。
///
/// 说明：Slint 的真实焦点状态由 `FocusScope.key-pressed` 传入的 `!has-focus` 提供；
/// 本开关只用于在无法注入真实鼠标点击的环境（X 会话锁屏）下程序化复现该标志。
pub const TEXT_FOCUS_ENV: &str = "NR_TEXT_FOCUS";
/// 首屏对齐延迟（毫秒）：窗口映射完成后才取正文区域尺寸与字体度量。
/// 一次性定时器（不是周期轮询，不违反「静止阅读不出帧」）。
pub const STARTUP_ALIGN_MS: u64 = 400;
/// 目录面板一次显示的行数（界面不申请无限长列表）。
pub const TOC_WINDOW: usize = 12;

/// 阅读会话：分页运行时 + 章节目录（DT4 的章节/进度/书签都挂在这里）。
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

    /// 跳转到字节偏移（章节跳转 / 书签跳转 / 进度恢复共用）。
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
    globals.set_bottom_bar_height(metrics.bottom_bar_height as f32);
    globals.set_menu_height(metrics.menu_height as f32);
    globals.set_content_padding_h(metrics.content_padding_h as f32);
    globals.set_content_padding_v(metrics.content_padding_v as f32);
    globals.set_nav_tab_width(metrics.nav_tab_width as f32);
    globals.set_nav_tab_height(metrics.nav_tab_height as f32);
    globals.set_default_window_width(metrics.default_window_width as f32);
    globals.set_default_window_height(metrics.default_window_height as f32);
    globals.set_title_font_size(metrics.title_font_size as f32);
    globals.set_body_font_size(metrics.body_font_size as f32);
    globals.set_caption_font_size(metrics.caption_font_size as f32);
    globals.set_corner_radius(metrics.corner_radius as f32);
    globals.set_min_window_width(metrics.min_window_width as f32);
    globals.set_min_window_height(metrics.min_window_height as f32);
    globals.set_scrollbar_width(metrics.scrollbar_width as f32);
    globals.set_scrollbar_gutter(metrics.scrollbar_gutter as f32);
    globals.set_scrollbar_min_handle(metrics.scrollbar_min_handle as f32);
}

/// 一次状态投影：把 [`UiSnapshot`] 写入 Slint 属性（UI 不持有业务状态）。
pub fn apply_snapshot(ui: &AppWindow, snapshot: &UiSnapshot) {
    ui.set_current_screen(snapshot.screen.index());
    ui.set_page_title(snapshot.page_title.as_str().into());
    ui.set_status_text(snapshot.status_text.as_str().into());
    ui.set_app_version(snapshot.app_version.as_str().into());
    ui.set_theme_dark(snapshot.theme_dark);
    ui.set_shelf_hint(snapshot.shelf_hint.as_str().into());
    ui.set_placeholder_title(snapshot.placeholder_title.as_str().into());
    ui.set_placeholder_message(snapshot.placeholder_message.as_str().into());

    let rows: Vec<BookRow> = snapshot
        .shelf_rows
        .iter()
        .map(|row| BookRow {
            id: row.id.as_str().into(),
            title: row.title.as_str().into(),
            status: row.status.as_str().into(),
            progress: row.progress.as_str().into(),
            // 界面不做业务判断：是否提供「重新关联」入口只看 Rust 给出的标签。
            needs_relink: source_status::needs_relink_label(row.status.as_str()),
        })
        .collect();
    ui.set_book_rows(ModelRc::new(VecModel::from(rows)));

    ui.set_reader_book_title(snapshot.reader.book_title.as_str().into());
    ui.set_reader_chapter_title(snapshot.reader.chapter_title.as_str().into());
    ui.set_reader_chapter_label(snapshot.reader.chapter_label.as_str().into());
    ui.set_reader_page_text(snapshot.reader.page_text.as_str().into());
    // 逐行渲染：正文按行拆分后交给 Slint 逐行定位（渲染口径由测量结果唯一决定）。
    let page_lines: Vec<slint::SharedString> = snapshot
        .reader
        .page_text
        .split('\n')
        .map(|line| line.into())
        .collect();
    ui.set_reader_page_lines(ModelRc::new(VecModel::from(page_lines)));
    ui.set_reader_page_label(snapshot.reader.page_label.as_str().into());
    ui.set_reader_menu_open(snapshot.reader.menu_open);
    // 重排状态（DT3B）：文案来自 Rust 侧投影，界面只显示。
    ui.set_reader_reflow_status(snapshot.reader.reflow_status.as_str().into());
    // DT4：进度状态 / 手势模式 / 目录 / 书签（全部来自 Rust 的有界投影）。
    ui.set_reader_progress_status(snapshot.reader.progress_status.as_str().into());
    ui.set_reader_gesture_mode(snapshot.reader.gesture_mode);
    ui.set_reader_toc_open(snapshot.reader.toc_open);
    let toc_rows: Vec<TocRow> = snapshot
        .reader
        .toc_rows
        .iter()
        .map(|row| TocRow {
            sequence: row.sequence.as_str().into(),
            title: row.title.as_str().into(),
            current: row.current,
        })
        .collect();
    ui.set_reader_toc_rows(ModelRc::new(VecModel::from(toc_rows)));
    ui.set_reader_marks_open(snapshot.reader.marks_open);
    let mark_rows: Vec<MarkRow> = snapshot
        .reader
        .mark_rows
        .iter()
        .map(|row| MarkRow {
            id: row.id.as_str().into(),
            title: row.title.as_str().into(),
            position_label: row.position_label.as_str().into(),
            current_chapter: row.current_chapter,
            selected: row.id == snapshot.reader.mark_selected,
        })
        .collect();
    ui.set_reader_mark_rows(ModelRc::new(VecModel::from(mark_rows)));
    ui.set_reader_mark_total(snapshot.reader.mark_total as i32);
    ui.set_reader_rename_text(snapshot.reader.mark_rename_text.as_str().into());
    ui.set_reader_meta_label(snapshot.reader.encoding_name.as_str().into());
    ui.set_reader_choice_open(snapshot.reader.encoding_open);
    let encoding_rows: Vec<ChoiceRow> = snapshot
        .reader
        .encoding_rows
        .iter()
        .map(|row| ChoiceRow {
            name: row.name.as_str().into(),
            current: row.current,
        })
        .collect();
    ui.set_reader_choice_rows(ModelRc::new(VecModel::from(encoding_rows)));
    ui.set_reader_immersive(snapshot.reader.immersive);
    ui.set_reader_toc_ratio(snapshot.reader.toc_ratio_permille as f32 / 1_000.0);
    ui.set_reader_toc_handle_ratio(snapshot.reader.toc_handle_permille as f32 / 1_000.0);
    ui.set_reader_progress_percent(snapshot.reader.progress_percent.as_str().into());

    let settings_rows: Vec<SettingsRow> = snapshot
        .settings
        .items
        .iter()
        .map(|item| SettingsRow {
            key: item.key.as_str().into(),
            label: item.label.as_str().into(),
            value: item.value.as_str().into(),
            note: item.note.as_str().into(),
            effective: item.effective,
        })
        .collect();
    ui.set_settings_rows(ModelRc::new(VecModel::from(settings_rows)));
    ui.set_settings_interaction_mode(snapshot.settings.interaction_mode.as_str().into());
    ui.set_settings_persisted(snapshot.settings.persisted);

    if state_log_enabled() {
        // `row_status` 是书架第一行的界面标签（Rust 投影结果），便于程序化验证「失效条目
        // 在界面上确实进入需要重新关联态」；未改动 DT1 既有字段与顺序（新增字段在 status 之前）。
        let row_status = snapshot
            .shelf_rows
            .first()
            .map(|row| row.status.clone())
            .unwrap_or_default();
        let line = format!(
            "NR-STATE screen={} index={} rows={} menu_open={} theme_dark={} row_status={} \
             chapter={} chapters={} toc_rows={} toc_open={} marks={} marks_open={} mark_sel={} \
             gesture={} progress={} settings={} encoding={} status={}",
            snapshot.screen.slint_name(),
            snapshot.screen.index(),
            snapshot.shelf_rows.len(),
            u8::from(snapshot.reader.menu_open),
            u8::from(snapshot.theme_dark),
            row_status,
            snapshot.reader.chapter_sequence,
            snapshot.reader.chapter_total,
            snapshot.reader.toc_rows.len(),
            u8::from(snapshot.reader.toc_open),
            snapshot.reader.mark_total,
            u8::from(snapshot.reader.marks_open),
            snapshot.reader.mark_selected,
            u8::from(snapshot.reader.gesture_mode),
            snapshot.reader.progress_status.replace(' ', "_"),
            snapshot.settings.items.len(),
            snapshot.reader.encoding_name.replace(' ', "_"),
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

/// 提交一次书架刷新（导入 / 重新关联后让列表与状态重新来自数据库）。
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
///
/// 这三项必须与测量侧完全一致，否则会出现「测量排 22 行、界面只排 16 行」的底部留白。
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
///
/// 失败/末尾/过期时**保留当前页面**，只更新状态文案：旧结果不得覆盖或清空新页面。
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
            let chapter_title = session.toc.title_at(session.current_chapter());
            mutate_and_refresh(ui, state, |guard| {
                guard.set_reader_page(title, chapter_title, text, label);
                guard.set_chapter(
                    session.current_chapter(),
                    session.toc.len(),
                    session.toc.title_at(session.current_chapter()),
                );
                guard.set_chapter_label(chapter_label);
                guard.set_status(status);
                guard.set_progress_percent(crate::pagination::format_progress_percent(
                    session.runtime.progress_permille(),
                ));
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

/// 投影章节目录窗口（有界）与当前章节。
fn project_toc(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    session: &ReadingSession,
    window_first: Option<u32>,
) {
    let current = session.current_chapter();
    let total = session.toc.len();
    let first = match window_first {
        Some(first) => first,
        None => session.toc.window(current, TOC_WINDOW).0,
    };
    let (first, rows) = toc_rows(&session.toc, first, current);
    let label = session.chapter_label();
    let title = session.toc.title_at(current);
    let book_id = session.runtime.book().book_id;
    mutate_and_refresh(ui, state, |guard| {
        guard.set_toc_rows(rows, first, current, total);
        guard.set_chapter(current, total, title);
        guard.set_chapter_label(label);
    });
    log_line(&session.toc.state_log(book_id, current, first));
}

/// 目录窗口（从 `first` 起取 [`TOC_WINDOW`] 行；越界自动夹紧）。
fn toc_rows(toc: &ChapterToc, first: u32, current: u32) -> (u32, Vec<TocRowData>) {
    let total = toc.len();
    if total == 0 {
        return (0, Vec::new());
    }
    let span = TOC_WINDOW.min(total);
    let max_first = (total - span) as u32;
    let first = first.min(max_first);
    let rows = (first..first + span as u32)
        .filter_map(|sequence| {
            toc.entry(sequence).map(|entry| TocRowData {
                sequence: sequence.to_string(),
                title: entry.title,
                current: sequence == current,
            })
        })
        .collect();
    (first, rows)
}

/// 投影书签窗口（有界）。`offset` 为 `None` 时沿用当前窗口起点。
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
    let first = match offset {
        Some(offset) => offset as usize,
        None => {
            let guard = lock(state);
            guard.reader().mark_window_first as usize
        }
    };
    let (first, rows) = bookmarks::window(
        &targets,
        Some(&session.toc),
        current,
        first,
        bookmarks::MARKS_PER_PAGE,
    );
    mutate_and_refresh(ui, state, |guard| {
        guard.set_mark_rows(rows, first as u32, targets.len());
    });
}

/// 保存当前位置（用户入口：翻页 / 章节跳转 / 书签跳转 / 「保存进度」/ 退出前的脏位置）。
///
/// 幂等由两层保证：跟踪器不重复标记同一位置；核心 `save_progress` 对同一位置不写库。
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
///
/// 因此页面位置、进度保存与结构化 `NR-PAGE` / `NR-PROGRESS` 输出完全一致；不引入页码身份
/// （内容位置仍由 `ContentPosition` 决定，DR3 / 卡片第 4 条）。
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
///
/// `key_text` 是 Slint `KeyEvent.text`（原样透传，界面不做按键语义）；
/// `text_input_focused` 表示焦点是否在文本输入元素上（界面传 `!FocusScope.has-focus`）。
/// 判定规则全部在 [`crate::input::key_intent`]（含「输入框聚焦时不翻页」的负向契约）。
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
            // 翻页键被屏蔽（当前唯一原因是文本输入框持有焦点）：如实记录，供负向取证。
            log_line(&format!(
                "NR-INPUT kind=key key={name} intent=none focused={focused} blocked=text-input"
            ));
        }
        // 非翻页键：既不翻页也不记录（不打扰文本输入与其它快捷键）。
        (None, _) => {}
    }
}

/// 鼠标滚轮输入（正文区 `TouchArea.scroll-event` 的落点）→ 翻页意图 → 用户翻页入口。
///
/// DR3：正文**不出现滚动条**，因此滚轮在这里被映射为整页翻页，而不是滚动正文。
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
///
/// 正文区从 `<正文内边距>` 到 `窗口尺寸 - 正文内边距`（顶栏/底部栏之外），
/// 因此窗口中心必然落在正文区内；这里不写死任何尺寸。
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

/// 程序化输入注入（DT8 证据）：把环境变量给定的事件**投递给 Slint 窗口**，让事件经
/// `.slint` 的真实接收点（`FocusScope.key-pressed` / 正文区 `TouchArea.scroll-event`）
/// 回流到与界面完全相同的判定函数；返回实际投递的事件数。
///
/// 负向用例（`NR_TEXT_FOCUS=1`）：本环境无法注入真实鼠标点击（X 会话锁屏，见卡片），
/// 因此把「文本输入框持有焦点」这一标志直接交给**同一个**判定入口 `dispatch_key_input`
/// （真实界面路径下该标志来自 `!FocusScope.has-focus`）。
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

/// 跳转到字节偏移（章节跳转 / 书签跳转 / 进度恢复共用），并保存新位置。
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
    // 目录行「跳转」后应立即展示正文（2026-09-30 主人反馈）：**落点成功即收起目录面板**。
    // 跳转失败时保留面板，便于用户换个章节重试（失败原因已进状态栏）。
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

/// 编码选择行（7 种编码；当前生效项由 Rust 标记）。
fn encoding_rows_for(current: reading_source::TextEncoding) -> Vec<EncodingRowData> {
    reading_source::TextEncoding::ALL
        .iter()
        .map(|encoding| EncodingRowData {
            name: encoding.to_string(),
            current: *encoding == current,
        })
        .collect()
}

/// 投影内容编码：展示名（用户指定时带「（手动）」）与面板行。
fn project_encoding(ui: &AppWindow, state: &Arc<Mutex<AppState>>, session: &ReadingSession) {
    let book = session.runtime.book();
    let manual = book.encoding_user_override;
    let label = if manual {
        format!("编码 {}（手动）", book.encoding)
    } else {
        format!("编码 {}", book.encoding)
    };
    let rows = encoding_rows_for(session.runtime.encoding());
    mutate_and_refresh(ui, state, |guard| {
        guard.set_encoding_label(label.clone(), manual);
        guard.set_encoding_rows(rows.clone());
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

/// 用户手动选择内容编码（界面「编码」面板的「选用」入口）。
///
/// 顺序（每一步都是真实链路）：落库 + 递增内容索引版本 → 按新编码重建目录 →
/// 按**字节位置**重建分页会话 → 投影与保存进度 → 结构化证据行（`NR-ENCODING`）。
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

    // 目录：按新编码重新扫描（内容索引版本已递增，旧缓存自然失效）。
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

    // 分页：按字节位置重建会话（偏移不合法时如实回退到内容起点）。
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
        guard.set_encoding_open(false);
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

/// 删除书签。
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

/// 应用一次全局设置变更：改值 → 落库 → 按需重排 → 重新投影。
///
/// 幅度为 0 表示「按文本设置」（`set_values`），否则按步长增减。
fn apply_settings(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    db_path: &Path,
    settings: &Arc<Mutex<GlobalSettings>>,
    session: Option<&mut ReadingSession>,
    key: &str,
    delta: i64,
) -> Option<String> {
    let (changed, log, error) = {
        let mut guard = lock(settings);
        if delta == 0 {
            match parse_setting_pairs(key) {
                Ok(pairs) => {
                    let mut changed = 0usize;
                    let mut last_error = None;
                    for (pair_key, pair_value) in pairs {
                        match guard.set_text(&pair_key, &pair_value) {
                            Ok(()) => changed += 1,
                            Err(error) => last_error = Some(error.message()),
                        }
                    }
                    (changed, guard.state_log(changed), last_error)
                }
                Err(error) => (0usize, String::new(), Some(error)),
            }
        } else {
            match guard.adjust(key, delta) {
                Ok(true) => (1usize, guard.state_log(1), None),
                Ok(false) => (0usize, guard.state_log(0), None),
                Err(error) => (0usize, String::new(), Some(error.message())),
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
    if changed == 0 {
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
    let view = SettingsView::from_global(&lock(settings));
    if let Some(session) = session {
        if let Ok(result) = session.runtime.set_typography_settings(typography) {
            project_reader_outcome(ui, state, session, &result.outcome, "设置");
        }
    }
    mutate_and_refresh(ui, state, |guard| {
        guard.set_settings_view(view.clone());
        guard.set_gesture_mode(gesture);
        guard.set_status(if gesture {
            "设置已保存：手势翻页模式（隐藏常驻按钮条）".to_string()
        } else {
            "设置已保存：按钮翻页模式".to_string()
        });
    });
    log_line(&log);
    Some(log)
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

/// 阅读页当前几何（真实窗口尺寸 + 实测字体度量 + 菜单状态）。
///
/// 书架「阅读」按钮与启动驱动同源：视口取窗口真实尺寸（未映射时回退到槽位）、
/// 字体取 Slint 上报的行盒度量（不可用才回退）、菜单状态取当前投影。
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
    let menu_open = lock(state).project().reader.menu_open;
    (viewport, font, menu_open)
}

/// 从书架打开一本书（界面「阅读」按钮与程序化驱动 `NR_SHELF_OPEN` **共用同一函数**）：
/// 建立会话 → 放入阅读槽位 → 切到阅读页；失败时状态栏给出原因（不静默）。
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
    let (viewport, font, menu_open) = current_reader_geometry(ui, state, viewport_slot, font_slot);
    let loaded = lock(settings_slot).clone();
    let Some(session) = open_reader(
        ui, state, db_path, viewport, font, menu_open, &loaded, selection,
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
///
/// 顺序（每一步都是真实链路）：读全局设置 → 打开书籍记录 → 目录（缓存或真实扫描）
/// → 读进度 → 核心锚点恢复 → 以该偏移重新测量首屏。
#[allow(clippy::too_many_arguments)]
fn open_reader(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
    db_path: &Path,
    viewport: MappedViewport,
    font: PlatformFontMetrics,
    menu_open: bool,
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
        menu_open,
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
    // ---- 重启恢复：读进度 → 核心锚点恢复 → 以该偏移重新测量 ----
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

    let state = Arc::new(Mutex::new(AppState::new(app_version)));
    {
        let mut guard = lock(&state);
        if let Some(screen) = start_screen_from_env() {
            guard.set_screen(screen);
        }
        guard.set_settings_view(SettingsView::from_global(&global_settings));
        guard.set_gesture_mode(global_settings.gesture_mode());
        guard.set_status("就绪：正在加载本地书架…");
    }
    let settings_slot = Arc::new(Mutex::new(global_settings));

    let ui = AppWindow::new()?;
    apply_layout_metrics(&ui, &metrics);

    // ---- DT3：分页运行时与视口状态（窗口映射后才取真实尺寸）----
    let reader: Arc<Mutex<Option<ReadingSession>>> = Arc::new(Mutex::new(None));
    let tracker = Arc::new(Mutex::new(ProgressTracker::new()));
    let viewport_slot: Arc<Mutex<MappedViewport>> = Arc::new(Mutex::new(MappedViewport::UNMAPPED));
    let font_slot: Arc<Mutex<PlatformFontMetrics>> =
        Arc::new(Mutex::new(PlatformFontMetrics::UNKNOWN));
    let menu_open = env_flag(READER_MENU_ENV);
    // 未打开书籍时也要给出可见的正文行盒（空态提示文案要能看见）。
    let idle_font_px = lock(&state).settings().font_size_px;
    apply_reader_typography(
        &ui,
        idle_font_px,
        idle_font_px.saturating_mul(3) / 2,
        lock(&settings_slot).reader().font_family.as_str(),
    );

    // 窗口映射完成/尺寸变化（含 DPI 变化导致的逻辑尺寸变化）→ 以当前位置重新分页。
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
                // 尚未打开书籍：只记录视口，不做任何排版（0×0 尺寸绝不参与分页）。
                return;
            };
            // 字体度量（行盒）与视口都会改变每页行数：任一变化都以当前位置重新分页。
            // 两者取值未变化时是幂等的（不清缓存、不换世代）。
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

    // ---- Slint → Rust：只上报意图 ----
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

    ui.on_toggle_reader_menu({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            // 菜单展开会改变正文区域高度：先切换显示状态，再以当前位置重新分页。
            mutate_and_refresh(&ui, &state, |guard| {
                guard.toggle_reader_menu();
            });
            let mut guard = lock(&reader);
            if let Some(session) = guard.as_mut() {
                match session.runtime.toggle_menu() {
                    Ok(result) => {
                        project_reader_outcome(&ui, &state, session, &result.outcome, "菜单");
                        project_toc(&ui, &state, session, None);
                    }
                    Err(error) => set_status_text(&ui, &state, "菜单", &error.message()),
                }
            }
        }
    });

    // 上一页 / 下一页：用户入口 —— 翻页后按内容位置保存进度。
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

    // ---- DT8：键盘翻页 / 滚轮翻页（真实输入事件 → 与按钮相同的用户入口）----
    //
    // 界面只转发原始事件数据；翻页意图映射与「输入框聚焦时不翻页」的规则在
    // `crate::input`（有单元测试）。两者都走 `turn_pages` → `user_turns`，
    // 因此进度保存与 `NR-PAGE` 语义与按钮完全一致。
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
            // 沉浸阅读的退出键（`Esc`）：与「退出沉浸」按钮、正文点击同一函数。
            // `input` 模块只给按键取值，语义（何时退出）在 UI 层。
            if key_text == input::escape_key_text() {
                if lock(&state).project().reader.immersive {
                    let mut guard = lock(&reader);
                    match guard.as_mut() {
                        Some(session) => {
                            apply_immersive(&ui, &state, &db_path, session, &tracker, false)
                        }
                        None => mutate_and_refresh(&ui, &state, |guard| {
                            guard.set_immersive(false);
                        }),
                    }
                }
                return;
            }
            // 界面报告的是「焦点不在键盘作用域上」；唯一的其它可聚焦元素是书签重命名
            // 输入框，而它只在书签面板展开时存在，因此这里再结合面板状态判定
            // 「文本输入框持有焦点」（判定规则见 `crate::input::text_input_focused`）。
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

    // ---- DT4：进度 / 目录 / 书签 / 设置 ----
    ui.on_save_progress({
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

    // 选项面板：展开 / 收起（本地交互）+ 用户选用（真实链路，见 apply_encoding_override）。
    ui.on_choice_toggle({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            mutate_and_refresh(&ui, &state, |guard| {
                guard.toggle_encoding_panel();
            });
            if let Some(session) = lock(&reader).as_ref() {
                project_encoding(&ui, &state, session);
            }
        }
    });

    ui.on_choice_select({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let tracker = Arc::clone(&tracker);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move |name| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let encoding = crate::pagination::parse_encoding(name.as_str());
            let mut guard = lock(&reader);
            match guard.as_mut() {
                Some(session) => {
                    apply_encoding_override(&ui, &state, &db_path, session, &tracker, encoding)
                }
                None => mutate_and_refresh(&ui, &state, |guard| {
                    guard.set_status("编码：尚未打开书籍（请先在书架导入并打开 TXT）".to_string());
                }),
            }
        }
    });

    // 沉浸阅读：进入 / 退出（界面显隐 + 版式重排走同一函数）。
    ui.on_immersive_toggle({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let tracker = Arc::clone(&tracker);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let target = !lock(&state).project().reader.immersive;
            let mut guard = lock(&reader);
            match guard.as_mut() {
                Some(session) => apply_immersive(&ui, &state, &db_path, session, &tracker, target),
                None => mutate_and_refresh(&ui, &state, |guard| {
                    guard.set_status(
                        "沉浸阅读：尚未打开书籍（请先在书架导入并打开 TXT）".to_string(),
                    );
                }),
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
            let first = {
                let state_guard = lock(&state);
                state_guard.reader().toc_window_first as i64
            };
            let step = (TOC_WINDOW as i64) / 2;
            let next = (first + delta as i64 * step).max(0) as u32;
            project_toc(&ui, &state, session, Some(next));
        }
    });

    // 目录滚轮：滚动目录本身（一行一格），**不**触达正文翻页。
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

    // 目录滚动条：按比例定位（找偏后章节不必逐页翻）。
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
            let total = session.toc.len();
            if total == 0 || track_height <= 0.0 {
                return;
            }
            let ratio = (pointer_y / track_height).clamp(0.0, 1.0);
            let visible = TOC_WINDOW.min(total);
            let max_first = total.saturating_sub(visible);
            let first = (ratio * max_first as f32).round() as u32;
            project_toc(&ui, &state, session, Some(first));
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
            let selected = lock(&state).reader().mark_selected.clone();
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

    ui.on_mark_delete({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move |id| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Ok(id) = id.as_str().trim().parse::<i64>() else {
                mutate_and_refresh(&ui, &state, |guard| {
                    guard.set_status("删除书签：书签标识非法，本次请求已忽略");
                });
                return;
            };
            let guard = lock(&reader);
            if let Some(session) = guard.as_ref() {
                delete_mark(&ui, &state, &db_path, session, id);
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

    ui.on_settings_change({
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let settings_slot = Arc::clone(&settings_slot);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move |name, delta| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let key = name.as_str().to_string();
            let mut guard = lock(&reader);
            let session = guard.as_mut();
            apply_settings(
                &ui,
                &state,
                &db_path,
                &settings_slot,
                session,
                &key,
                delta as i64,
            );
        }
    });

    ui.on_quit_requested(|| {
        let _ = slint::quit_event_loop();
    });

    // ---- 后台结果回到 UI 线程：slint::invoke_from_event_loop ----
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

    // 书架后台任务：先校正来源状态（只动状态列），再做 DT1 的书架读取。
    let (channel, worker) = JobChannel::start(
        Arc::clone(&bridge),
        Box::new(source_status::shelf_job_with_source_check),
    );
    let channel = Arc::new(channel);

    // ---- DT2：TXT 导入与失效重新关联（选择器可注入；系统对话框本身是模态的）----
    //
    // 边界说明：导入只做「校验 + 有限窗口指纹/编码识别 + 一次事务」，因此直接在 UI 线程
    // 上执行；系统文件对话框由用户交互驱动（模态），其阻塞不影响任何后台语义。
    let picker: Arc<dyn FilePicker> = Arc::from(import::picker_from_env());

    ui.on_import_txt({
        let state = Arc::clone(&state);
        let weak = ui.as_weak();
        let picker = Arc::clone(&picker);
        let channel = Arc::clone(&channel);
        let db_path = db_path.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            mutate_and_refresh(&ui, &state, |guard| {
                guard.set_status(
                    "导入：正在等待文件选择（只接受 .txt；校验失败不会留下半成品记录）",
                );
            });
            let result = import::run_import_flow(&db_path, picker.as_ref(), import::now_unix());
            mutate_and_refresh(&ui, &state, |guard| {
                guard.set_status(import::status_text_for(&result));
            });
            submit_shelf_refresh(&state, &channel, &db_path);
        }
    });

    // 书架行「阅读」：按书籍 id 打开（与启动驱动 `NR_READER_BOOK` 同一函数），
    // 成功后把会话放进阅读槽位并切到阅读页；失败时状态栏给出原因（不静默）。
    ui.on_reader_open_book({
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

    ui.on_relink_book({
        let state = Arc::clone(&state);
        let weak = ui.as_weak();
        let picker = Arc::clone(&picker);
        let channel = Arc::clone(&channel);
        let db_path = db_path.clone();
        move |book_id| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Ok(book_id) = book_id.as_str().trim().parse::<i64>() else {
                mutate_and_refresh(&ui, &state, |guard| {
                    guard.set_status("重新关联：书籍标识非法，本次请求已忽略");
                });
                return;
            };
            mutate_and_refresh(&ui, &state, |guard| {
                guard.set_status(
                    "重新关联：请选择同一本书的 TXT；内容不一致时不会写入，进度与书签保留",
                );
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
    });

    // ---- 程序化验证（DT2）：NR_IMPORT_PATH / NR_RELINK_BOOK 在启动期走真实链路 ----
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

    // ---- DT3/DT4：窗口映射完成后取真实尺寸、完成首屏分页并跑启动期驱动 ----
    //
    // 一次性定时器（不是周期轮询）：Slint 窗口尺寸只在事件循环启动后才有效，
    // 事件循环启动前的 0×0 尺寸绝不参与分页（design.md §正文布局、§8.3 第 5 条）。
    slint::Timer::single_shot(Duration::from_millis(STARTUP_ALIGN_MS), {
        let state = Arc::clone(&state);
        let reader = Arc::clone(&reader);
        let tracker = Arc::clone(&tracker);
        let settings_slot = Arc::clone(&settings_slot);
        let viewport_slot = Arc::clone(&viewport_slot);
        let font_slot = Arc::clone(&font_slot);
        let db_path = db_path.clone();
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let mut viewport = *lock(&viewport_slot);
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
                *lock(&font_slot)
            };
            let loaded = lock(&settings_slot).clone();
            // 0a) 书架「阅读」入口（与书架「阅读」按钮**同一函数**；可独立于 NR_READER_BOOK 使用）。
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
            let Some(mut session) = open_reader(
                &ui, &state, &db_path, viewport, font, menu_open, &loaded, &selection,
            ) else {
                return;
            };
            // ---- 程序化驱动（走与界面回调同一批函数）----
            // 0) 全局设置（按文本设置并持久化）。
            if let Some(pairs) = env_text(SETTINGS_SET_ENV) {
                apply_settings(
                    &ui,
                    &state,
                    &db_path,
                    &settings_slot,
                    Some(&mut session),
                    &pairs,
                    0,
                );
            }
            // 1) 章节跳转。
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
            // 1b) 手动覆盖内容编码（界面「编码」面板「选用」的等价入口）。
            if let Some(name) = env_text(ENCODING_SELECT_ENV) {
                let encoding = crate::pagination::parse_encoding(&name);
                apply_encoding_override(&ui, &state, &db_path, &mut session, &tracker, encoding);
            }
            // 1c) 沉浸阅读（界面「沉浸阅读」按钮的等价入口）。
            if env_flag(IMMERSIVE_ENV) {
                apply_immersive(&ui, &state, &db_path, &mut session, &tracker, true);
            }
            // 2) 用户入口翻页（含进度保存）。
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
            // 3) 书签：新增 → 跳转 → 重命名 → 删除（顺序即卡片 DoD 的「增删改跳」）。
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
                delete_mark(&ui, &state, &db_path, &session, id);
            }
            // 4) 面板投影（目录 / 书签窗口）。
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
            // 5) 显式保存进度（用户按钮的等价入口）。
            if env_flag(SAVE_PROGRESS_ENV) {
                save_current(&ui, &state, &db_path, &session, &tracker, "保存进度");
            }
            // ---- DT3 兼容驱动：直接分页（不经过用户入口，因而不写进度）----
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
            if menu_open {
                let _ = session.runtime.toggle_menu();
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
                // 界面回调收「指针 y + 轨道高度」两个长度：此处用 1000 单位轨道表达比例。
                ui.invoke_toc_seek((ratio as f32) * 1000.0, 1000.0);
            }
            // 目录行「跳转」：走与行内按钮**同一回调**（时序在面板打开之后，复现真实点击）。
            if let Some(sequence) = env_i64(TOC_JUMP_ENV) {
                let text: slint::SharedString = sequence.to_string().into();
                ui.invoke_chapter_jump(text);
            }
            // ---- DT8：输入入口程序化验证（投递真实窗口事件，不另开旁路）----
            //
            // 会话此刻已在共享槽位里，回调路径与用户操作完全一致：按键/滚轮事件由 Slint
            // 分派给持有焦点的 `FocusScope` / 正文区 `TouchArea`，再经 `.slint` 回调回到
            // `dispatch_key_input` / `dispatch_wheel_input`。
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

    // 退出清理：导入/重新关联回调里持有 `Arc<JobChannel>`，因此必须先释放 UI
    // （回调随组件一起释放），再关闭后台通道（工作线程随通道结束），
    // 否则 `worker.join()` 会因为发送端仍然存活而永久阻塞 —— 不在磁盘/内存里留下残留进程。
    drop(ui);
    drop(channel);
    let _ = worker.join();

    result
}

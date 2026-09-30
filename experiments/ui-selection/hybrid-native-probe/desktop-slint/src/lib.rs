//! 桌面端验证探针（优先候选 hybrid-native）。
//!
//! 分层：
//! - [`ui`]：Slint UI 绑定（唯一来源 `ui/app.slint`）
//! - [`app_state`]：状态编排，全部阅读业务委托共享库
//! - [`page_layouter`]：有限窗口 + 公共分页 API 的适配层
//! - [`ready`]：ready.json 就绪探针

pub mod app_state;
pub mod page_layouter;
pub mod ready;
pub mod ui;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

pub use app_state::{
    AppStatus, BookRow, CategoryRow, ChapterRow, ReaderApp, SelfCheck, MAX_WINDOW_BYTES,
    WINDOW_BYTES,
};
pub use page_layouter::{walk_pages, PageLayouter, RenderedPage};
pub use ready::FRAMEWORK_VERSION;

/// 可接受的渲染器变体。
pub const VARIANTS: [&str; 3] = ["femtovg", "skia", "software"];

// ---- 状态投影 ----

impl From<BookRow> for ui::BookRow {
    fn from(value: BookRow) -> Self {
        Self {
            id: value.id.into(),
            title: value.title.into(),
            author: value.author.into(),
            meta: value.meta.into(),
        }
    }
}

impl From<CategoryRow> for ui::CategoryRow {
    fn from(value: CategoryRow) -> Self {
        Self {
            id: value.id.into(),
            name: value.name.into(),
            level: value.level,
            meta: value.meta.into(),
        }
    }
}

impl From<ChapterRow> for ui::ChapterRow {
    fn from(value: ChapterRow) -> Self {
        Self {
            index: value.index,
            title: value.title.into(),
        }
    }
}

/// 把应用状态投影到 UI。
pub fn refresh(window: &ui::AppWindow, app: &mut ReaderApp) {
    let status = app.status();
    window.set_status_text(status.status_text.into());
    window.set_crumb_text(status.crumb_text.into());
    window.set_metrics_text(status.metrics_text.into());
    window.set_engine_text(status.engine_text.into());
    window.set_book_title(status.book_title.into());
    window.set_chapter_title(status.chapter_title.into());
    window.set_page_text(status.page_text.into());
    window.set_page_label(status.page_label.into());
    window.set_progress_text(status.progress_text.into());
    window.set_font_size(status.font_size);
    window.set_line_spacing_x10(status.line_spacing_x10);
    window.set_page_margin(status.page_margin);
    window.set_background_mode(status.background_mode);
    window.set_theme_dark(status.theme_dark);

    let books: Vec<ui::BookRow> = app.book_rows().into_iter().map(Into::into).collect();
    window.set_book_rows(ModelRc::new(VecModel::from(books)));
    let categories: Vec<ui::CategoryRow> =
        app.category_rows().into_iter().map(Into::into).collect();
    window.set_category_rows(ModelRc::new(VecModel::from(categories)));
    let chapters: Vec<ui::ChapterRow> = app.chapter_rows().into_iter().map(Into::into).collect();
    window.set_chapter_rows(ModelRc::new(VecModel::from(chapters)));
}

type AppCell = Rc<RefCell<ReaderApp>>;

// ---------------------------------------------------------------------------
// 版式常量：必须与 `ui/app.slint` 中的对应字面量保持一致（单一事实来源在 UI 文件）。
// 用途：把「真实正文区域」换算给共享库排版，使页边界与可见区域严格对齐。
// ---------------------------------------------------------------------------
/// 根顶栏高度。
const ROOT_TOP_BAR: f32 = 52.0;
/// 阅读页标题栏高度。
const READER_HEAD_BAR: f32 = 44.0;
/// 阅读页底部按钮栏高度。
const READER_FOOT_BAR: f32 = 52.0;
/// 阅读页正文区上内边距。
const READER_TEXT_PAD_TOP: f32 = 12.0;
/// 阅读页内嵌设置浮层高度（展开时正文区相应变矮）。
const READER_MENU: f32 = 116.0;

/// 把 Slint 的真实正文区域尺寸上报给应用状态（不改数据时不做任何重排）。
fn sync_viewport(window: &ui::AppWindow, app: &mut ReaderApp) -> bool {
    let size = window.window().size();
    let scale = window.window().scale_factor();
    if scale <= 0.0 {
        return false;
    }
    let win_w = size.width as f32 / scale;
    let win_h = size.height as f32 / scale;
    let margin = app.page_margin() as f32;
    let menu = if window.get_reader_menu_open() {
        READER_MENU
    } else {
        0.0
    };
    let area_w = win_w - 2.0 * margin;
    let area_h =
        win_h - ROOT_TOP_BAR - READER_HEAD_BAR - READER_FOOT_BAR - READER_TEXT_PAD_TOP - menu;
    let changed = app.set_viewport(area_w, area_h);
    eprintln!(
        "viewport: 窗口 {win_w:.0}x{win_h:.0} → 正文区 {area_w:.0}x{area_h:.0} 重排={changed}"
    );
    changed
}

/// 就地修改应用状态并刷新 UI；窗口已关闭时退出事件循环。
fn apply(window: &slint::Weak<ui::AppWindow>, cell: &AppCell, mutate: impl FnOnce(&mut ReaderApp)) {
    let upgraded = window.upgrade();
    if let Some(win) = upgraded.as_ref() {
        {
            let mut app = cell.borrow_mut();
            mutate(&mut app);
            // 每次操作后都对齐排版区域：字号/页边距/菜单开合/窗口尺寸变化都能生效。
            sync_viewport(win, &mut app);
        }
        refresh(win, &mut cell.borrow_mut());
    } else {
        let _ = slint::quit_event_loop();
    }
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(default)
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

fn arg_value(args: &[String], key: &str) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == key {
            return iter.next().cloned();
        }
        if let Some(rest) = arg.strip_prefix(&format!("{key}=")) {
            return Some(rest.to_string());
        }
    }
    None
}

/// 依变体选择 Slint 后端（仅当未显式设置 SLINT_BACKEND 时）。
fn configure_backend(variant: &str) {
    if std::env::var("SLINT_BACKEND").is_ok() {
        return;
    }
    let backend = match variant {
        "skia" => "winit-skia",
        "software" => "software",
        _ => "winit-femtovg",
    };
    std::env::set_var("SLINT_BACKEND", backend);
}

/// 进程入口。返回进程退出码。
pub fn run() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    let variant = arg_value(&args, "--variant")
        .or_else(|| std::env::var("NR_VARIANT").ok())
        .unwrap_or_else(|| "femtovg".to_string());
    if !VARIANTS.contains(&variant.as_str()) {
        eprintln!("未知渲染变体: {variant}（可选 {:?}）", VARIANTS);
        return 2;
    }
    configure_backend(&variant);

    let ready_pages = env_usize("NR_READY_PAGES", 1000);
    let check_only = args.iter().any(|a| a == "--pagination-self-check");

    if check_only {
        // 无窗口模式：只跑分页自查，输出 JSON 供脚本消费。
        let mut app = ReaderApp::new();
        let check = app.pagination_self_check(ready_pages);
        println!("{}", check.to_json());
        return if check.status == "通过" { 0 } else { 1 };
    }

    let window = match ui::AppWindow::new() {
        Ok(window) => window,
        Err(err) => {
            eprintln!("创建 Slint 窗口失败: {err}");
            return 1;
        }
    };

    let cell: AppCell = Rc::new(RefCell::new(ReaderApp::new()));
    if ready_pages > 0 {
        let mut app = cell.borrow_mut();
        let check = app.pagination_self_check(ready_pages);
        println!("分页自检: {}", check.to_json());
        for note in check.notes.iter() {
            eprintln!("分页自检提示: {note}");
        }
    }
    refresh(&window, &mut cell.borrow_mut());

    // ---- 回调接线 ----
    {
        let weak = window.as_weak();
        let cell = cell.clone();
        window.on_search_submitted(move |query: SharedString| {
            let query = query.to_string();
            apply(&weak, &cell, |app| app.set_query(&query));
        });
    }
    {
        let weak = window.as_weak();
        let cell = cell.clone();
        window.on_open_category(move |id: SharedString| {
            if let Ok(value) = id.to_string().parse::<u64>() {
                apply(&weak, &cell, |app| app.enter_category(value));
            }
        });
    }
    {
        let weak = window.as_weak();
        let cell = cell.clone();
        window.on_open_book(move |id: SharedString| {
            if let Ok(value) = id.to_string().parse::<u64>() {
                apply(&weak, &cell, |app| {
                    app.open_book(value);
                });
            }
        });
    }
    {
        let weak = window.as_weak();
        let cell = cell.clone();
        window.on_shelf_up(move || {
            apply(&weak, &cell, |app| app.up_level());
        });
    }
    {
        let weak = window.as_weak();
        let cell = cell.clone();
        window.on_reader_prev(move || {
            apply(&weak, &cell, |app| {
                let _ = app.prev_page();
            });
        });
    }
    {
        let weak = window.as_weak();
        let cell = cell.clone();
        window.on_reader_next(move || {
            apply(&weak, &cell, |app| {
                let _ = app.next_page();
            });
        });
    }
    {
        let weak = window.as_weak();
        window.on_reader_toggle_menu(move || {
            if let Some(window) = weak.upgrade() {
                let open = window.get_reader_menu_open();
                window.set_reader_menu_open(!open);
            }
        });
    }
    {
        let weak = window.as_weak();
        let cell = cell.clone();
        window.on_reader_save(move || {
            apply(&weak, &cell, |app| app.save_position());
        });
    }
    {
        let weak = window.as_weak();
        let cell = cell.clone();
        window.on_chapter_selected(move |index: i32| {
            apply(&weak, &cell, |app| {
                let _ = app.seek_chapter(index.max(0) as u32);
            });
        });
    }
    {
        let weak = window.as_weak();
        let cell = cell.clone();
        window.on_settings_change(move |key: SharedString, value: i32| {
            let key = key.to_string();
            apply(&weak, &cell, |app| {
                app.set_setting(&key, value);
            });
        });
    }
    {
        let weak = window.as_weak();
        let cell = cell.clone();
        window.on_settings_apply(move || {
            apply(&weak, &cell, |app| {
                app.pagination_self_check(ready_pages);
            });
        });
    }
    {
        let weak = window.as_weak();
        let cell = cell.clone();
        window.on_settings_reset(move || {
            apply(&weak, &cell, |app| {
                app.reset_settings();
            });
        });
    }

    // ---- 启动：可选主题/背景/起始页覆盖（截图与冒烟验证用）+ 对齐排版区域 ----
    {
        let mut app = cell.borrow_mut();
        if std::env::var("NR_START_PAGE").as_deref() == Ok("1") {
            window.set_current_page(1);
        }
        if std::env::var("NR_THEME_DARK")
            .map(|v| v.trim() == "1")
            .unwrap_or(false)
        {
            let _ = app.set_setting("theme", 1);
        }
        if let Ok(raw) = std::env::var("NR_BACKGROUND_MODE") {
            if let Ok(mode) = raw.trim().parse::<i32>() {
                let _ = app.set_setting("background-mode", mode);
            }
        }
        let changed = sync_viewport(&window, &mut app);
        let size = window.window().size();
        eprintln!(
            "启动期 viewport 探测（事件循环前，尺寸可能为 0）: {}x{} 重排={changed}",
            size.width, size.height
        );
    }
    refresh(&window, &mut cell.borrow_mut());

    // ---- 就绪探针 ----
    let exit_after_ready = env_u64("NR_EXIT_AFTER_READY_MS", 0);
    let ready_path = ready::resolve_path();
    {
        let weak = window.as_weak();
        let cell = cell.clone();
        let renderer = variant.clone();
        let backend = backend_name();
        slint::Timer::single_shot(Duration::from_millis(400), move || {
            let Some(window) = weak.upgrade() else {
                let _ = slint::quit_event_loop();
                return;
            };
            // 事件循环已启动、窗口已映射，此时窗口尺寸才是真实值：
            // 在此对齐「排版区域」与「真实正文区域」，页边界才会与可见高度一致。
            {
                let mut app = cell.borrow_mut();
                if sync_viewport(&window, &mut app) {
                    refresh(&window, &mut app);
                }
            }
            let size = window.window().size();
            let json = {
                let mut app = cell.borrow_mut();
                ready::build_json(
                    &mut app,
                    &backend,
                    &renderer,
                    size.width as i32,
                    size.height as i32,
                )
            };
            match ready::write_atomic(&ready_path, &json) {
                Ok(()) => eprintln!("ready.json 已写入: {}", ready_path.display()),
                Err(err) => eprintln!("ready.json 写入失败: {err}"),
            }
            if exit_after_ready > 0 {
                slint::Timer::single_shot(Duration::from_millis(exit_after_ready), || {
                    let _ = slint::quit_event_loop();
                });
            }
        });
    }

    if let Err(err) = window.run() {
        eprintln!("事件循环异常: {err}");
        return 1;
    }
    0
}

/// 当前后端名（x11 / wayland）。
pub fn backend_name() -> String {
    match std::env::var("WAYLAND_DISPLAY") {
        Ok(value) if !value.is_empty() => "wayland".to_string(),
        _ => "x11".to_string(),
    }
}

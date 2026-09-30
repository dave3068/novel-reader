//! 应用状态与「Rust → Slint」状态投影。
//!
//! 依据 `desktop/design.md` §页面状态：桌面应用状态由 Rust 应用层维护，Slint 只接收
//! **可显示字段**与触发动作；每个后台结果都带请求号与布局号，过期结果不得覆盖当前页面。
//!
//! 本模块不含任何阅读业务语义：不做编码识别、章节识别、分页边界、位置校验或书签规则，
//! 只做「状态机 + 可显示字段投影」。阅读语义一律来自冻结核心（`reading-core` /
//! `reading-storage`），由编排层调用。

use std::path::PathBuf;

use reading_storage::settings::ReaderSettings;

/// 应用页面（与 `desktop/ui/app.slint` 的 `Screens` 全局一一对应）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// 书架。
    Shelf,
    /// 阅读。
    Reader,
    /// 设置。
    Settings,
    /// 导入中（等待导入链路的过渡态）。
    Importing,
    /// 错误态。
    Error,
}

impl Screen {
    /// 全部页面（顺序即投影序号）。
    pub const ALL: [Screen; 5] = [
        Screen::Shelf,
        Screen::Reader,
        Screen::Settings,
        Screen::Importing,
        Screen::Error,
    ];

    /// 投影序号（Slint `current-screen` 的取值）。
    pub const fn index(self) -> i32 {
        match self {
            Screen::Shelf => 0,
            Screen::Reader => 1,
            Screen::Settings => 2,
            Screen::Importing => 3,
            Screen::Error => 4,
        }
    }

    /// Slint `Screens` 全局中的字段名（用于一致性自检）。
    pub const fn slint_name(self) -> &'static str {
        match self {
            Screen::Shelf => "shelf",
            Screen::Reader => "reader",
            Screen::Settings => "settings",
            Screen::Importing => "importing",
            Screen::Error => "error",
        }
    }

    /// 页面中文标题（状态栏/顶栏显示）。
    pub const fn title(self) -> &'static str {
        match self {
            Screen::Shelf => "书架",
            Screen::Reader => "阅读",
            Screen::Settings => "设置",
            Screen::Importing => "导入中",
            Screen::Error => "错误",
        }
    }

    /// 由投影序号还原；越界返回 `None`（UI 传入非法序号时不得改变状态）。
    pub const fn from_index(index: i32) -> Option<Screen> {
        match index {
            0 => Some(Screen::Shelf),
            1 => Some(Screen::Reader),
            2 => Some(Screen::Settings),
            3 => Some(Screen::Importing),
            4 => Some(Screen::Error),
            _ => None,
        }
    }
}

/// 书架一行（只含可显示字段）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShelfRow {
    /// 书籍 id（64 位，跨 UI/JSON 一律字符串化，避免精度截断）。
    pub id: String,
    /// 书名。
    pub title: String,
    /// 来源状态（`ok` / `needs_relink` 等，来自核心存储层）。
    pub status: String,
    /// 阅读进度摘要。
    pub progress: String,
}

/// 目录一行（只含可显示字段；目录窗口由 Rust 计算，UI 不申请无限长列表）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocRow {
    /// 章节序号（0 基，字符串化避免精度问题）。
    pub sequence: String,
    /// 章节标题。
    pub title: String,
    /// 是否当前所在章节（Rust 判定，UI 只上色）。
    pub current: bool,
}

/// 内容编码选择面板的一行（名称与本行是否为当前编码均由 Rust 判定；UI 只渲染）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodingRow {
    /// 编码展示名（`TextEncoding` 的展示口径：`UTF-8` / `GBK` / `GB18030` / `UTF-16LE` …）。
    pub name: String,
    /// 是否是当前生效的编码。
    pub current: bool,
}

/// 阅读页可显示字段（正文/目录/书签/进度均为 Rust 投影结果）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReaderView {
    /// 书名。
    pub book_title: String,
    /// 章节标题。
    pub chapter_title: String,
    /// 章节标签（「第 N/M 章 · 标题」；文案由 Rust 生成）。
    pub chapter_label: String,
    /// 当前页正文（有限窗口，由核心提供，绝不放整本）。
    pub page_text: String,
    /// 页码标签。
    pub page_label: String,
    /// 阅读菜单是否展开。
    pub menu_open: bool,
    /// 重排状态文案（DT3B）：重排期间界面必须显示状态（文案唯一来源是
    /// `crate::reflow::ReflowState::label`，此处只做投影）。
    pub reflow_status: String,
    /// 目录面板是否展开（DT4）。
    pub toc_open: bool,
    /// 目录窗口行（有界）。
    pub toc_rows: Vec<TocRow>,
    /// 目录窗口起始序号。
    pub toc_window_first: u32,
    /// 目录滚动条：窗口起点占「可滚动范围」的**千分比**（0..1000；界面换算为比例）。
    pub toc_ratio_permille: u32,
    /// 目录滚动条：可视窗口占列表的千分比（滑块长度；下限 80 保证可抓）。
    pub toc_handle_permille: u32,
    /// 当前章节序号（0 基）。
    pub chapter_sequence: u32,
    /// 章节总数。
    pub chapter_total: usize,
    /// 书签面板是否展开（DT4）。
    pub marks_open: bool,
    /// 书签窗口行（有界）。
    pub mark_rows: Vec<crate::bookmarks::MarkRow>,
    /// 书签窗口起始下标。
    pub mark_window_first: u32,
    /// 书签总数。
    pub mark_total: usize,
    /// 选中的书签标识（重命名/删除的目标；空串表示未选中）。
    pub mark_selected: String,
    /// 重命名输入框文本（UI 双向绑定，语义仍在 Rust）。
    pub mark_rename_text: String,
    /// 进度状态文案（保存 / 恢复结果）。
    pub progress_status: String,
    /// 是否手势翻页模式（手势模式下隐藏常驻翻页按钮条）。
    pub gesture_mode: bool,
    /// 沉浸阅读（隐藏顶栏 / 底部状态栏 / 阅读菜单；正文区使用整个窗口）。
    pub immersive: bool,
    /// 阅读进度读数（按字节位置，如 `42.3%`；空串 = 未知）。
    pub progress_percent: String,
    /// 当前内容编码展示名（空串 = 未知）。
    pub encoding_name: String,
    /// 编码是否由用户手动指定（否则为导入识别结果；只影响展示口径）。
    pub encoding_manual: bool,
    /// 编码选择面板是否展开。
    pub encoding_open: bool,
    /// 编码选择面板的全部行（7 种编码；当前项由 Rust 标记）。
    pub encoding_rows: Vec<EncodingRow>,
}

/// 设置页可显示字段（持久化由 `crate::settings` 接入核心设置表）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsView {
    /// 正文字号（像素，UI 用整数；核心用毫单位）。
    pub font_size_px: u32,
    /// 行距（毫像素 ÷ 100，沿用 DT1 的 ×10 显示口径）。
    pub line_spacing_x10: u32,
    /// 主题（`system` / `light` / `dark`）。
    pub theme: String,
    /// 阅读模式（`paged` / `scroll`；核心设置表字段）。
    pub operation_mode: String,
    /// 字体族。
    pub font_family: String,
    /// 全部全局设置行（文案由 `crate::settings` 生成，UI 只渲染）。
    pub items: Vec<crate::settings::SettingsItem>,
    /// 交互模式（`buttons` / `gesture`）。
    pub interaction_mode: String,
    /// 设置是否已持久化（`origin == stored`）。
    pub persisted: bool,
}

impl Default for SettingsView {
    fn default() -> Self {
        // 默认值来自核心存储层的 `ReaderSettings::default()`，避免在桌面端复制一套默认值。
        Self::from_settings(&ReaderSettings::default())
    }
}

impl SettingsView {
    /// 由核心设置记录投影而来（毫单位 → 显示口径的换算只发生在这里）。
    pub fn from_settings(settings: &ReaderSettings) -> Self {
        Self {
            font_size_px: (settings.font_size_milli / 1000).max(0) as u32,
            line_spacing_x10: (settings.line_spacing_milli / 100).max(0) as u32,
            theme: settings.theme.clone(),
            operation_mode: settings.operation_mode.clone(),
            font_family: settings.font_family.clone(),
            items: Vec::new(),
            interaction_mode: crate::settings::MODE_BUTTONS.to_string(),
            persisted: false,
        }
    }

    /// 由全局设置投影而来（唯一入口：`crate::settings::GlobalSettings`）。
    pub fn from_global(settings: &crate::settings::GlobalSettings) -> Self {
        let mut view = Self::from_settings(settings.reader());
        view.items = settings.items();
        view.interaction_mode = settings.interaction_mode().to_string();
        view.persisted = matches!(settings.origin(), crate::settings::SettingsOrigin::Stored);
        view
    }

    /// 是否使用深色主题（Slint 只接收布尔结果，不解析主题字符串）。
    pub fn theme_dark(&self) -> bool {
        self.theme == reading_storage::settings::THEME_DARK
    }
}

/// 一次投影的完整可显示状态（Slint 侧只读入参）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiSnapshot {
    /// 当前页面。
    pub screen: Screen,
    /// 顶栏页面标题。
    pub page_title: String,
    /// 状态栏文本。
    pub status_text: String,
    /// 应用版本（核心版本 + ABI 版本）。
    pub app_version: String,
    /// 书架行。
    pub shelf_rows: Vec<ShelfRow>,
    /// 书架为空时的提示（Rust 决定文案，UI 不判断）。
    pub shelf_hint: String,
    /// 阅读页字段。
    pub reader: ReaderView,
    /// 设置页字段。
    pub settings: SettingsView,
    /// 过渡页标题（导入中 / 错误）。
    pub placeholder_title: String,
    /// 过渡页说明。
    pub placeholder_message: String,
    /// 是否深色主题。
    pub theme_dark: bool,
}

/// 书架为空时的提示文案（Rust 侧唯一来源）。
pub const SHELF_EMPTY_HINT: &str = "书架为空：尚未导入 TXT。导入链路在 DT2 接入后可用。";
/// 未打开书籍时阅读页的提示文案（Rust 侧唯一来源）。
pub const READER_EMPTY_TEXT: &str =
    "尚未打开书籍：请在书架导入 TXT 后开始阅读（阅读闭环在 DT2–DT4 接入）。";
/// 未打开书籍时的页码标签。
pub const READER_EMPTY_LABEL: &str = "—";

/// 后台读取书架（DT1 唯一真实后台任务；导入/索引由 DT2 起接入）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShelfRequest {
    /// 请求号（单调递增）。
    pub request_id: u64,
    /// 布局号（布局变化后旧结果作废）。
    pub layout_id: u64,
    /// 库文件路径。
    pub db_path: PathBuf,
}

/// 后台结果载荷。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackgroundPayload {
    /// 书架数据。
    Shelf {
        /// 书架行。
        rows: Vec<ShelfRow>,
        /// 核心设置。
        settings: Box<SettingsView>,
        /// 状态栏文本。
        status_text: String,
    },
    /// 失败（保留可读信息，不把错误文本当业务分支——业务分支一律用核心错误码）。
    Failure {
        /// 可读错误信息。
        message: String,
    },
}

/// 后台结果（携带请求号与布局号，供过期判定）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundResult {
    /// 请求号。
    pub request_id: u64,
    /// 布局号。
    pub layout_id: u64,
    /// 载荷。
    pub payload: BackgroundPayload,
}

/// 结果应用结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// 已应用并刷新投影。
    Applied,
    /// 丢弃：布局已变化（重排/窗口变化后的旧结果）。
    DroppedStaleLayout,
    /// 丢弃：请求号不大于已应用的最大请求号（乱序/重复）。
    DroppedStaleRequest,
}

/// 应用状态（唯一状态源；Slint 不持有业务状态）。
#[derive(Debug, Clone)]
pub struct AppState {
    screen: Screen,
    status_text: String,
    app_version: String,
    shelf_rows: Vec<ShelfRow>,
    reader: ReaderView,
    settings: SettingsView,
    next_request_id: u64,
    current_layout_id: u64,
    last_applied_request_id: u64,
    applied_results: u64,
    dropped_results: u64,
}

impl AppState {
    /// 新建初始状态：书架页 + 空书架（不注入任何模拟书籍）。
    pub fn new(app_version: impl Into<String>) -> Self {
        Self {
            screen: Screen::Shelf,
            status_text: String::new(),
            app_version: app_version.into(),
            shelf_rows: Vec::new(),
            reader: ReaderView::default(),
            settings: SettingsView::default(),
            next_request_id: 1,
            current_layout_id: 1,
            last_applied_request_id: 0,
            applied_results: 0,
            dropped_results: 0,
        }
    }

    /// 当前页面。
    pub fn screen(&self) -> Screen {
        self.screen
    }

    /// 当前布局号。
    pub fn layout_id(&self) -> u64 {
        self.current_layout_id
    }

    /// 已应用/丢弃的后台结果计数（自检与测试用）。
    pub fn counters(&self) -> (u64, u64) {
        (self.applied_results, self.dropped_results)
    }

    /// 状态栏文本。
    pub fn status_text(&self) -> &str {
        &self.status_text
    }

    /// 书架行。
    pub fn shelf_rows(&self) -> &[ShelfRow] {
        &self.shelf_rows
    }

    /// 阅读页字段。
    pub fn reader(&self) -> &ReaderView {
        &self.reader
    }

    /// 设置页字段。
    pub fn settings(&self) -> &SettingsView {
        &self.settings
    }

    /// 设置状态栏文本。
    pub fn set_status(&mut self, text: impl Into<String>) {
        self.status_text = text.into();
    }

    /// 切换页面；返回是否发生变化。
    pub fn set_screen(&mut self, screen: Screen) -> bool {
        if self.screen == screen {
            return false;
        }
        self.screen = screen;
        true
    }

    /// 处理 UI 导航意图（UI 只上报序号，不做语义判断）。
    pub fn navigate(&mut self, index: i32) -> bool {
        match Screen::from_index(index) {
            Some(screen) => self.set_screen(screen),
            None => false,
        }
    }

    /// 切换阅读菜单展开状态；返回是否变化。
    pub fn toggle_reader_menu(&mut self) -> bool {
        self.reader.menu_open = !self.reader.menu_open;
        true
    }

    /// 申请一个新的后台请求（请求号单调递增）。
    pub fn next_shelf_request(&mut self, db_path: PathBuf) -> ShelfRequest {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        ShelfRequest {
            request_id,
            layout_id: self.current_layout_id,
            db_path,
        }
    }

    /// 标记布局已变化（窗口尺寸/排版设置变化时调用）：旧布局的后台结果一律作废。
    pub fn invalidate_layout(&mut self) {
        self.current_layout_id += 1;
        self.next_request_id += 1;
    }

    /// 应用后台结果（必须在 UI 线程调用），按请求号/布局号做过期判定。
    pub fn apply_result(&mut self, result: BackgroundResult) -> ApplyOutcome {
        if result.layout_id != self.current_layout_id {
            self.dropped_results += 1;
            return ApplyOutcome::DroppedStaleLayout;
        }
        if result.request_id <= self.last_applied_request_id {
            self.dropped_results += 1;
            return ApplyOutcome::DroppedStaleRequest;
        }
        match result.payload {
            BackgroundPayload::Shelf {
                rows,
                settings,
                status_text,
            } => {
                self.shelf_rows = rows;
                self.settings = *settings;
                self.status_text = status_text;
            }
            BackgroundPayload::Failure { message } => {
                self.status_text = message.clone();
                self.screen = Screen::Error;
            }
        }
        self.last_applied_request_id = result.request_id;
        self.applied_results += 1;
        ApplyOutcome::Applied
    }

    /// 设置正文字号（像素；DT1 为内存态，DT4 起持久化）。
    pub fn set_font_size_px(&mut self, px: u32) {
        self.settings.font_size_px = px;
    }

    /// 设置阅读页可显示字段（DT3：正文来自分页运行时的有限窗口测量结果；章节与进度由 DT4 接入）。
    pub fn set_reader_page(
        &mut self,
        book_title: impl Into<String>,
        chapter_title: impl Into<String>,
        page_text: impl Into<String>,
        page_label: impl Into<String>,
    ) {
        self.reader.book_title = book_title.into();
        self.reader.chapter_title = chapter_title.into();
        self.reader.page_text = page_text.into();
        self.reader.page_label = page_label.into();
    }

    /// 设置重排状态文案（DT3B；文案由 `reflow::ReflowState::label` 提供，此处只存投影）。
    ///
    /// 关键约定：**只改状态文案，绝不触碰正文**——过期结果不得覆盖当前页面。
    pub fn set_reflow_status(&mut self, text: impl Into<String>) {
        self.reader.reflow_status = text.into();
    }

    /// 设置当前章节投影（DT4：章节标题与总数来自核心章节索引）。
    pub fn set_chapter(&mut self, sequence: u32, total: usize, title: impl Into<String>) {
        self.reader.chapter_sequence = sequence;
        self.reader.chapter_total = total;
        self.reader.chapter_title = title.into();
    }

    /// 设置章节标签（「第 N/M 章 · 标题」；文案由 Rust 生成，UI 只显示）。
    pub fn set_chapter_label(&mut self, label: impl Into<String>) {
        self.reader.chapter_label = label.into();
    }

    /// 设置目录窗口（有界）与当前章节。
    pub fn set_toc_rows(
        &mut self,
        rows: Vec<TocRow>,
        window_first: u32,
        current: u32,
        total: usize,
    ) {
        self.reader.toc_rows = rows;
        self.reader.toc_window_first = window_first;
        self.reader.chapter_sequence = current;
        self.reader.chapter_total = total;
        // 目录滚动条比例：只在 Rust 计算（界面不解析列表规模），供「按比例定位」使用。
        // 用**千分比整数**而不是 f32：`ReaderView` 保持 `Eq` 派生（浮点不满足 `Eq`）。
        let visible = self.reader.toc_rows.len().max(1);
        let max_first = total.saturating_sub(visible);
        self.reader.toc_ratio_permille = if max_first == 0 {
            0
        } else {
            ((window_first.min(max_first as u32) as f64 / max_first as f64) * 1_000.0).round()
                as u32
        };
        self.reader.toc_handle_permille = if total == 0 {
            1_000
        } else {
            ((visible as f64 / total as f64) * 1_000.0)
                .round()
                .clamp(80.0, 1_000.0) as u32
        };
    }

    /// 展开/收起目录面板；返回是否变化。
    pub fn set_toc_open(&mut self, open: bool) -> bool {
        if self.reader.toc_open == open {
            return false;
        }
        self.reader.toc_open = open;
        true
    }

    /// 切换目录面板；返回是否变化。
    pub fn toggle_toc_panel(&mut self) -> bool {
        self.reader.toc_open = !self.reader.toc_open;
        true
    }

    /// 设置书签窗口（有界）与总数。
    pub fn set_mark_rows(
        &mut self,
        rows: Vec<crate::bookmarks::MarkRow>,
        window_first: u32,
        total: usize,
    ) {
        self.reader.mark_rows = rows;
        self.reader.mark_window_first = window_first;
        self.reader.mark_total = total;
    }

    /// 切换书签面板；返回是否变化。
    pub fn toggle_marks_panel(&mut self) -> bool {
        self.reader.marks_open = !self.reader.marks_open;
        true
    }

    /// 设置/清空选中的书签标识。
    pub fn set_mark_selected(&mut self, id: impl Into<String>) {
        self.reader.mark_selected = id.into();
    }

    /// 设置重命名输入框文本（UI 双向绑定）。
    pub fn set_mark_rename_text(&mut self, text: impl Into<String>) {
        self.reader.mark_rename_text = text.into();
    }

    /// 设置进度状态文案（保存 / 恢复结果；只改文案，不碰正文）。
    pub fn set_progress_status(&mut self, text: impl Into<String>) {
        self.reader.progress_status = text.into();
    }

    /// 设置是否手势翻页模式（手势模式隐藏常驻翻页按钮条）。
    pub fn set_gesture_mode(&mut self, gesture: bool) {
        self.reader.gesture_mode = gesture;
    }

    /// 投影内容编码展示名与来源（识别 / 用户手动指定）。
    pub fn set_encoding_label(&mut self, name: impl Into<String>, manual: bool) {
        self.reader.encoding_name = name.into();
        self.reader.encoding_manual = manual;
    }

    /// 展开 / 收起编码选择面板（本地交互；面板内容由 [`AppState::set_encoding_rows`] 投影）。
    pub fn toggle_encoding_panel(&mut self) {
        self.reader.encoding_open = !self.reader.encoding_open;
    }

    /// 直接设置编码面板展开状态（选用编码后自动收起）。
    pub fn set_encoding_open(&mut self, open: bool) {
        self.reader.encoding_open = open;
    }

    /// 投影编码选择行（当前项由 Rust 标记，UI 只上色与转发）。
    pub fn set_encoding_rows(&mut self, rows: Vec<EncodingRow>) {
        self.reader.encoding_rows = rows;
    }

    /// 设置沉浸阅读状态（界面显隐的唯一来源；Rust 侧同时据此重排）。
    pub fn set_immersive(&mut self, immersive: bool) {
        self.reader.immersive = immersive;
    }

    /// 切换沉浸阅读（返回切换后的状态，供调用方重排）。
    pub fn toggle_immersive(&mut self) -> bool {
        self.reader.immersive = !self.reader.immersive;
        self.reader.immersive
    }

    /// 投影阅读进度读数（按字节位置；与页码无关）。
    pub fn set_progress_percent(&mut self, text: impl Into<String>) {
        self.reader.progress_percent = text.into();
    }

    /// 整体替换设置投影（设置变化后由 Rust 重新投影，UI 不自行改值）。
    pub fn set_settings_view(&mut self, settings: SettingsView) {
        self.settings = settings;
    }

    /// 设置状态栏文本 + 进度状态文案（保存进度时两处都要可见）。
    pub fn set_progress_and_status(
        &mut self,
        progress: impl Into<String>,
        status: impl Into<String>,
    ) {
        self.reader.progress_status = progress.into();
        self.status_text = status.into();
    }

    /// 当前状态的只读投影。
    pub fn project(&self) -> UiSnapshot {
        let shelf_hint = if self.shelf_rows.is_empty() {
            SHELF_EMPTY_HINT.to_string()
        } else {
            String::new()
        };
        let mut reader = self.reader.clone();
        if reader.book_title.is_empty() {
            reader.page_text = READER_EMPTY_TEXT.to_string();
            reader.page_label = READER_EMPTY_LABEL.to_string();
            reader.chapter_title = "未打开书籍".to_string();
        }
        UiSnapshot {
            screen: self.screen,
            page_title: self.screen.title().to_string(),
            status_text: self.status_text.clone(),
            app_version: self.app_version.clone(),
            shelf_rows: self.shelf_rows.clone(),
            shelf_hint,
            reader,
            settings: self.settings.clone(),
            placeholder_title: self.screen.title().to_string(),
            placeholder_message: self.status_text.clone(),
            theme_dark: self.settings.theme_dark(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shelf_result(request_id: u64, layout_id: u64, rows: Vec<ShelfRow>) -> BackgroundResult {
        BackgroundResult {
            request_id,
            layout_id,
            payload: BackgroundPayload::Shelf {
                rows,
                settings: Box::new(SettingsView::default()),
                status_text: format!("已加载 {request_id}"),
            },
        }
    }

    fn row(id: &str) -> ShelfRow {
        ShelfRow {
            id: id.to_string(),
            title: format!("书 {id}"),
            status: "ok".to_string(),
            progress: "未开始".to_string(),
        }
    }

    #[test]
    fn screen_index_mapping_is_closed_and_stable() {
        for (i, screen) in Screen::ALL.iter().enumerate() {
            assert_eq!(screen.index() as usize, i);
            assert_eq!(Screen::from_index(screen.index()), Some(*screen));
            assert!(!screen.slint_name().is_empty());
            assert!(!screen.title().is_empty());
        }
        assert_eq!(Screen::from_index(-1), None);
        assert_eq!(Screen::from_index(5), None);
    }

    #[test]
    fn navigate_ignores_out_of_range_index() {
        let mut state = AppState::new("test");
        assert_eq!(state.screen(), Screen::Shelf);
        assert!(!state.navigate(99));
        assert_eq!(state.screen(), Screen::Shelf);
        assert!(state.navigate(Screen::Reader.index()));
        assert_eq!(state.screen(), Screen::Reader);
    }

    #[test]
    fn projection_carries_rust_owned_state_only() {
        let mut state = AppState::new("core 0.1.0 / abi 1");
        state.navigate(Screen::Settings.index());
        let snapshot = state.project();
        assert_eq!(snapshot.screen, Screen::Settings);
        assert_eq!(snapshot.app_version, "core 0.1.0 / abi 1");
        assert!(snapshot.shelf_rows.is_empty(), "DT1 不得注入任何模拟书籍");
        assert_eq!(snapshot.shelf_hint, SHELF_EMPTY_HINT);
        assert_eq!(snapshot.reader.page_text, READER_EMPTY_TEXT);
        assert_eq!(snapshot.reader.page_label, READER_EMPTY_LABEL);
        assert_eq!(snapshot.page_title, "设置");
        assert!(!snapshot.theme_dark);
        assert!(snapshot.settings.font_size_px > 0);
    }

    #[test]
    fn stale_layout_results_are_dropped() {
        let mut state = AppState::new("test");
        let request = state.next_shelf_request(PathBuf::from("/tmp/library.db"));
        state.invalidate_layout();
        let outcome = state.apply_result(shelf_result(
            request.request_id,
            request.layout_id,
            vec![row("1")],
        ));
        assert_eq!(outcome, ApplyOutcome::DroppedStaleLayout);
        assert!(state.shelf_rows().is_empty());
        assert_eq!(state.counters(), (0, 1));
    }

    #[test]
    fn out_of_order_results_are_dropped() {
        let mut state = AppState::new("test");
        let path = PathBuf::from("/tmp/library.db");
        let first = state.next_shelf_request(path.clone());
        let second = state.next_shelf_request(path);
        assert_eq!(
            state.apply_result(shelf_result(
                second.request_id,
                second.layout_id,
                vec![row("2")]
            )),
            ApplyOutcome::Applied
        );
        assert_eq!(
            state.apply_result(shelf_result(
                first.request_id,
                first.layout_id,
                vec![row("1")]
            )),
            ApplyOutcome::DroppedStaleRequest
        );
        assert_eq!(state.shelf_rows().len(), 1);
        assert_eq!(state.shelf_rows()[0].id, "2");
        assert_eq!(state.counters(), (1, 1));
    }

    #[test]
    fn failure_payload_switches_to_error_screen() {
        let mut state = AppState::new("test");
        let request = state.next_shelf_request(PathBuf::from("/tmp/library.db"));
        let result = BackgroundResult {
            request_id: request.request_id,
            layout_id: request.layout_id,
            payload: BackgroundPayload::Failure {
                message: "库文件不可读".to_string(),
            },
        };
        assert_eq!(state.apply_result(result), ApplyOutcome::Applied);
        assert_eq!(state.screen(), Screen::Error);
        assert_eq!(state.status_text(), "库文件不可读");
    }

    #[test]
    fn reader_menu_toggle_is_state_owned() {
        let mut state = AppState::new("test");
        assert!(!state.reader().menu_open);
        assert!(state.toggle_reader_menu());
        assert!(state.reader().menu_open);
        assert!(state.toggle_reader_menu());
        assert!(!state.reader().menu_open);
    }

    #[test]
    fn reflow_status_is_projected_without_touching_page() {
        // DT3B：重排期间界面要能显示状态，而状态更新绝不能覆盖当前页面。
        let mut state = AppState::new("test");
        state.set_reader_page("书 1", "第一章", "正文第一页", "第 1 页");
        state.set_reflow_status("正在重排…");
        let snapshot = state.project();
        assert_eq!(snapshot.reader.reflow_status, "正在重排…");
        assert_eq!(snapshot.reader.page_text, "正文第一页");
        assert_eq!(snapshot.reader.page_label, "第 1 页");
        assert_eq!(snapshot.reader.book_title, "书 1");

        // 文案只影响状态字段：再次更新状态不改动正文。
        state.set_reflow_status("重排完成");
        let snapshot = state.project();
        assert_eq!(snapshot.reader.reflow_status, "重排完成");
        assert_eq!(snapshot.reader.page_text, "正文第一页");
    }
}

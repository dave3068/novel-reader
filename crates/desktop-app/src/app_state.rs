//! 应用状态与「Rust → Slint」状态投影（v0.2.0 IBM 重设计）。
//!
//! 依据 `docs/design/novel-reader-desktop-redesign.md`：深色 UI Shell 顶栏、书架表格/卡片、
//! 阅读页工具条 + 状态栏、覆盖式面板、弹窗与 Toast。
//!
//! 本模块不含任何阅读业务语义：不做编码识别、章节识别、分页边界、位置校验或书签规则，
//! 只做「状态机 + 可显示字段投影」。阅读语义一律来自冻结核心（`reading-core` /
//! `reading-storage`），由编排层调用。
//!
//! 书架**过滤与排序**在投影层完成（纯计算，不触库）：原始行来自后台任务，界面只渲染结果。

use std::path::PathBuf;

use crate::settings::SettingsRowView;

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

impl Default for Screen {
    fn default() -> Self {
        Screen::Shelf
    }
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

/// 书架一行（只含可显示字段；额外排序键仅供投影层排序，不投影给界面）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShelfRow {
    /// 书籍 id（64 位，跨 UI/JSON 一律字符串化，避免精度截断）。
    pub id: String,
    /// 书名。
    pub title: String,
    /// 封面首字（本地派生，不引入网络素材）。
    pub initial: String,
    /// 元信息（路径 · 体积；mono）。
    pub meta: String,
    /// 来源状态标签（`source_status.rs` 的既有标签）。
    pub status: String,
    /// 语义色槽位：0=中性 1=信息 2=成功 3=警告 4=错误。
    pub status_tone: i32,
    /// 阅读进度摘要。
    pub progress: String,
    /// 进度千分比（0..1000；2px 进度条用）。
    pub percent: i32,
    /// 最近阅读（`yyyy-MM-dd HH:MM`；未读为空）。
    pub recent: String,
    /// 是否提供「重新关联」入口（标签判定，来自 `source_status.rs`）。
    pub needs_relink: bool,
    /// 是否可以打开阅读（`ready` 状态）。
    pub can_open: bool,
    /// 最近阅读时间（排序键；秒）。
    pub recent_epoch: i64,
    /// 导入时间（排序键；秒）。
    pub created_epoch: i64,
}

/// 目录一行（序号 / 标题 / 偏移量都由 Rust 给出；当前章由 Rust 标记）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocRow {
    /// 章节序号（0 基，字符串化避免精度问题）。
    pub sequence: String,
    /// 章节标题。
    pub title: String,
    /// 偏移量（mono）。
    pub meta: String,
    /// 是否当前所在章节（Rust 判定，UI 只上色）。
    pub current: bool,
}

/// 书签一行（只含可显示字段）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkRow {
    /// 书签 id（字符串化）。
    pub id: String,
    /// 章节读数（mono）。
    pub chapter: String,
    /// 书签标题。
    pub title: String,
    /// 位置文案（偏移量 mono）。
    pub position_label: String,
    /// 是否当前章。
    pub current_chapter: bool,
    /// 是否选中（重命名目标）。
    pub selected: bool,
    /// 是否处于「行内两段确认」的删除待确认态。
    pub delete_pending: bool,
}

/// 内容编码一行（名称与本行是否为当前编码均由 Rust 判定；UI 只渲染）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceRow {
    /// 编码展示名（`TextEncoding` 的展示口径：`UTF-8` / `GBK` / `GB18030` / `UTF-16LE` …）。
    pub name: String,
    /// 是否是当前生效的编码。
    pub current: bool,
    /// 是否可选（V1.1：移动分类时自身与全部子孙不可选）。
    ///
    /// 判定**完全在 Rust**（`category_ops::move_targets`）；界面不得解析
    /// 显示文案来推断 —— 那会把「循环可达性」语义下沉到 `.slint`。
    pub enabled: bool,
}

/// 快捷键说明一行（tooltip 与快捷键面板同源）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutEntryView {
    /// 动作名。
    pub action: String,
    /// 按键（已实现才展示）。
    pub keys: String,
    /// 是否已实现（未实现为「建议」项）。
    pub available: bool,
}

/// 导入步骤一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepRowView {
    /// 步骤文案。
    pub label: String,
    /// 0=未开始 1=进行中 2=已完成。
    pub state: i32,
}

/// 阅读页可显示字段（正文/目录/书签/进度均为 Rust 投影结果）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReaderView {
    /// 书名。
    pub book_title: String,
    /// 章节标题。
    pub chapter_title: String,
    /// 章节标签（「第 N/M 章 · 标题」；文案由 Rust 生成）。
    pub chapter_label: String,
    /// 工具条居中标题（「书名 · 章节」；文案由 Rust 生成）。
    pub heading: String,
    /// 当前页正文（有限窗口，由核心提供，绝不放整本）。
    pub page_text: String,
    /// 页码标签。
    pub page_label: String,
    /// 阅读工具条是否展开。
    pub toolbar_open: bool,
    /// 正文列几何（窗口坐标；测量与渲染同口径）。
    pub column_x: u32,
    /// 正文列上边界。
    pub column_y: u32,
    /// 正文列宽（= min(正文区宽度, 40 × 字号)）。
    pub column_width: u32,
    /// 正文列高。
    pub column_height: u32,
    /// 阅读进度（按字节位置，千分比）。
    pub progress_permille: u32,
    /// 重排状态文案。
    pub reflow_status: String,
    /// 目录面板是否展开。
    pub toc_open: bool,
    /// 目录窗口行（有界）。
    pub toc_rows: Vec<TocRow>,
    /// 目录窗口起始序号（过滤后的窗口）。
    pub toc_window_first: u32,
    /// 目录滚动条：窗口起点占「可滚动范围」的千分比。
    pub toc_ratio_permille: u32,
    /// 目录滚动条：可视窗口占列表的千分比。
    pub toc_handle_permille: u32,
    /// 目录计数 Tag 文案（「128 章」）。
    pub toc_count_label: String,
    /// 目录底部读数（「7 / 128」）。
    pub toc_readout: String,
    /// 目录搜索关键词（UI 双向绑定）。
    pub toc_search: String,
    /// 目录过滤后的匹配总数。
    pub toc_match_total: usize,
    /// 当前章节序号（0 基）。
    pub chapter_sequence: u32,
    /// 章节总数。
    pub chapter_total: usize,
    /// 书签面板是否展开。
    pub marks_open: bool,
    /// 书签窗口行（有界）。
    pub mark_rows: Vec<MarkRow>,
    /// 书签窗口起始下标。
    pub mark_window_first: u32,
    /// 书签总数。
    pub mark_total: usize,
    /// 书签计数 Tag 文案。
    pub mark_count_label: String,
    /// 书签底部读数。
    pub mark_readout: String,
    /// 书签搜索关键词（UI 双向绑定）。
    pub mark_search: String,
    /// 选中的书签标识（重命名/删除的目标；空串表示未选中）。
    pub mark_selected: String,
    /// 重命名输入框文本（UI 双向绑定，语义仍在 Rust）。
    pub mark_rename_text: String,
    /// 删除待确认的书签 id（空串 = 无）。
    pub mark_delete_pending: String,
    /// 进度状态文案（保存 / 恢复结果）。
    pub progress_status: String,
    /// 是否手势翻页模式。
    pub gesture_mode: bool,
    /// 沉浸阅读。
    pub immersive: bool,
    /// 沉浸态浮出工具条是否可见（由 Rust 状态机 + 一次性定时器驱动）。
    pub floating_toolbar: bool,
    /// 阅读进度读数（按字节位置，如 `42.3%`；空串 = 未知）。
    pub progress_percent: String,
    /// 当前内容编码展示名（空串 = 未知）。
    pub encoding_name: String,
    /// 编码是否由用户手动指定。
    pub encoding_manual: bool,
    /// 编码选择面板是否展开。
    pub encoding_open: bool,
    /// 编码选择面板的全部行（7 种编码；当前项由 Rust 标记）。
    pub encoding_rows: Vec<ChoiceRow>,
    /// 工具条快捷键提示（已实现才非空；tooltip 与快捷键面板同源）。
    pub shortcut_prev: String,
    /// 下一页快捷键提示。
    pub shortcut_next: String,
    /// 退出沉浸快捷键提示。
    pub shortcut_immersive: String,
}

/// 侧栏分类树一行（V1.1，规格 R1）。
///
/// 层级、计数、循环可达性**全部由 Rust 投影**，界面只做缩进与高亮
/// （规格 R18 / 门禁 C34：语义算法不得进 `.slint`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryRow {
    /// 分类 id 字符串（64 位字符串化，避免精度截断）。
    pub id: String,
    pub name: String,
    /// 0 基深度（根下为 0；界面按此缩进）。
    pub depth: i32,
    /// 直属书籍数。
    pub direct_count: i32,
    /// 该分类及全部子孙的书籍总数。
    pub total_count: i32,
    /// 是否含子分类（有则显示展开箭头）。
    pub has_children: bool,
    /// 是否已展开（界面局部状态的回写）。
    pub expanded: bool,
    /// 是否为当前选中的过滤分类。
    pub selected: bool,
    /// 移动分类时的不可选标记（自身与全部子孙；规格 R5）。
    /// 界面只置灰 —— **判定由 Rust 给出**。
    pub move_blocked: bool,
    /// 非真实分类（全部书籍 / 未分类）不显示行内菜单。
    pub virtual_row: bool,
    /// 0=全部书籍 1=未分类 2=真实分类。
    pub kind: i32,
    /// 父分类 id 字符串（真实分类才有；用于折叠状态回写）。
    pub parent_id: String,
}

/// 阅读历史一行（V1.1；设计稿 H1）。
///
/// 全部字段由 Rust 的 `history_view::project` 给出：时间标签、位置标签、
/// 进入方式与语义色**都是 Rust 的判定**，界面只渲染（不排序、不格式化）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRow {
    /// 流水 id 字符串（64 位字符串化）。
    pub id: String,
    /// 书籍 id 字符串（「打开这本书」用）。
    pub book_id: String,
    /// 时间标签（今天 `HH:MM` / 本年 `MM-DD HH:MM` / 跨年完整）。
    pub time_label: String,
    /// 书名。
    pub book_title: String,
    /// 文件名（定位符末段，不暴露整条绝对路径）。
    pub file_label: String,
    /// 位置标签（`第 N 章 · 偏移 N 字节`；记录的是**打开时刻**的位置）。
    pub position_label: String,
    /// 进入方式（`书架打开` / `进度恢复` / …）。
    pub entry_kind_label: String,
    /// 语义色槽位：0=中性 2=成功。
    pub entry_kind_tone: i32,
}

/// 按书聚合的组头（V1.1；设计稿 H2）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryGroupHead {
    pub book_id: String,
    pub book_title: String,
    /// 该书的流水条数。
    pub count: i32,
    /// 最近一次时间标签。
    pub last_time_label: String,
    /// 是否展开（界面局部状态的回写）。
    pub expanded: bool,
}

/// 阅读历史视图（V1.1；设计稿 H1–H3）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HistoryView {
    /// 流水行（时间倒序）。
    pub rows: Vec<HistoryRow>,
    /// 按书聚合的组头（组按最近时间倒序）。
    pub groups: Vec<HistoryGroupHead>,
    /// 当前是聚合视图（`false` = H1 流水视图）。
    pub grouped: bool,
    /// 是否空态。
    pub empty: bool,
    pub empty_title: String,
    pub empty_desc: String,
    /// 计数文案（`共 N 条记录` / `暂无记录`）。
    pub count_label: String,
    pub total_count: i64,
    /// 历史视图**不显示**分类侧栏（Q5 裁定）。
    pub sidebar_visible: bool,
    /// 展开行级菜单的行下标（-1 = 未展开）。
    pub menu_index: i32,
}

/// 书架页可显示字段（过滤 / 排序后的行 + 页头与通知 + 分类侧栏）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShelfView {
    /// 侧栏分类树行（V1.1；首两行为「全部书籍」与「未分类」虚拟节点）。
    pub categories: Vec<CategoryRow>,
    /// 当前过滤的分类 id（字符串；空 = 根「全部书籍」）。
    pub selected_category_id: String,
    /// 侧栏是否可见（Rust 按断点投影，界面不自己算）。
    pub sidebar_visible: bool,
    /// 当前打开行内菜单的分类 id（V1.1；空串 = 未打开）。
    pub category_menu_id: String,
    /// 行内菜单挂在第几行（V1.1；-1 = 未打开）。
    pub category_menu_offset: i32,
    /// 过滤排序后的行。
    pub rows: Vec<ShelfRow>,
    /// 是否空态。
    pub empty: bool,
    /// 空态标题。
    pub empty_title: String,
    /// 空态说明。
    pub empty_desc: String,
    /// 空态补充说明（三条）。
    pub empty_notes: Vec<String>,
    /// 0=表格 1=卡片。
    pub view: i32,
    /// 搜索关键词（UI 双向绑定）。
    pub search: String,
    /// 排序显示名。
    pub sort_label: String,
    /// 排序序号（0=最近阅读 1=书名 2=导入时间；下拉高亮用）。
    pub sort_index: i32,
    /// 计数 Tag 文案。
    pub count_label: String,
    /// 是否显示失效源通知。
    pub notice: bool,
    /// 通知标题。
    pub notice_title: String,
    /// 通知正文。
    pub notice_body: String,
    /// 展开行级菜单的行下标（-1 = 未展开）。
    pub menu_index: i32,
    /// 会话内「忽略通知」标记（持久状态仍在，下次启动仍提示）。
    pub notice_ignored: bool,
}

impl Default for ShelfView {
    fn default() -> Self {
        Self {
            categories: Vec::new(),
            selected_category_id: String::new(),
            sidebar_visible: true,
            category_menu_id: String::new(),
            category_menu_offset: -1,
            rows: Vec::new(),
            empty: true,
            empty_title: String::new(),
            empty_desc: String::new(),
            empty_notes: Vec::new(),
            view: 0,
            search: String::new(),
            sort_label: String::new(),
            sort_index: 0,
            count_label: String::new(),
            notice: false,
            notice_title: String::new(),
            notice_body: String::new(),
            menu_index: -1,
            notice_ignored: false,
        }
    }
}

/// 弹窗状态（S8 选项 / S13 移出与彻底删除 / S14 清理索引 / S15 快捷键 / 重命名）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DialogView {
    /// 0=无 1=确认（通用）2=快捷键 3=选项 4=重命名 5=分类操作 6=删除分类去向。
    pub kind: i32,
    /// 确认弹窗标题。
    pub confirm_title: String,
    /// 确认弹窗正文。
    pub confirm_body: String,
    /// 确认弹窗脚注。
    pub confirm_note: String,
    /// 勾选项标签（空 = 无勾选项）。
    pub confirm_checkbox_label: String,
    /// 确认按钮文案。
    pub confirm_label: String,
    /// 是否破坏性（Danger）。
    pub confirm_danger: bool,
    /// 待确认动作标识（Rust 内部使用，决定确认后的动作）。
    pub confirm_action: String,
    /// 待确认对象的书籍 id（书籍相关动作）。
    pub confirm_book_id: String,
    /// 选项弹窗标题。
    pub choice_title: String,
    /// 选项弹窗当前值（mono）。
    pub choice_meta: String,
    /// 选项行。
    pub choice_rows: Vec<ChoiceRow>,
    /// 选项当前下标。
    pub choice_current_index: i32,
    /// 快捷键行。
    pub shortcuts: Vec<ShortcutEntryView>,
    /// 重命名弹窗标题。
    pub rename_title: String,
    /// 重命名初值。
    pub rename_value: String,
    /// 重命名目标书籍 id（Rust 内部使用）。
    pub rename_book_id: String,
    // ---- V1.1 分类操作（R2/R3/R4/R5；设计稿 C3–C6）----
    /// 分类操作弹窗标题（「新建分类」/「重命名分类」）。
    pub category_dialog_title: String,
    /// 分类操作弹窗的父分类行文案。
    pub category_dialog_parent: String,
    /// 分类操作弹窗的名称输入初值。
    pub category_dialog_value: String,
    /// 分类操作弹窗的错误提示（空 = 无；同父同名时由 Rust 给出）。
    pub category_dialog_error: String,
    /// 分类操作弹窗的主按钮文案。
    pub category_dialog_label: String,
    /// 分类操作弹窗针对的分类 id（新建时为空串 = 不针对既有分类）。
    pub category_dialog_id: String,
    /// 分类操作弹窗的父分类 id（新建/移动的落点；空串 = 根）。
    pub category_dialog_parent_id: String,
    /// 「删除分类」：目标分类 id。
    pub category_delete_id: String,
    /// 「删除分类」：目标分类名。
    pub category_delete_name: String,
    /// 「删除分类」：该分类 + 全部子孙下的书籍总数（计数口径 = 分类自身 + 子孙）。
    pub category_delete_book_count: i64,
    /// 「删除分类」：书籍去向（0=未选 1=移入某分类 2=移出书架 3=移回根）。
    ///
    /// 口径：**默认 0（未选）**，此时删除按钮保持 disabled 且不产生任何副作用
    /// （规格 R3 / 设计稿 C4「必选」）。
    pub category_delete_destination: i32,
    /// 「删除分类」：去向为「移入某分类」时的目标 id。
    pub category_delete_target_id: String,
    /// 行内菜单项（新建子分类 / 重命名 / 移动到分类 / 删除）。
    pub category_menu_items: Vec<ChoiceRow>,
    // ---- V1.1 批量导入归类（规格 R6；设计稿 C7）----
    /// 导入归类浮层标题。
    pub organize_title: String,
    /// 导入归类浮层的落点说明（当前分类面包屑）。
    pub organize_scope: String,
    /// 导入归类浮层的摘要（选中文件数、将建分类数）。
    pub organize_summary: String,
    /// 预览行（目录名 + 「N 本」；**由 Rust 拼好**，界面不统计）。
    pub organize_preview: Vec<OrganizeRowView>,
    /// 「记住我的选择」是否勾选。
    pub organize_remember: bool,
    /// 批量导入待处理的文件（绝对路径；浮层期间暂存，确认后消费）。
    pub organize_paths: Vec<std::path::PathBuf>,
    // ---- V1.1 阅读历史视图（规格 R8/R9；设计稿 H1–H3）----
    /// 历史视图投影（`kind = 6` 时有效）。
    pub history: HistoryView,
    /// 是否处于历史视图（书架的另一种视图；Q5：不进分类侧栏）。
    pub history_mode: bool,
    /// 按书聚合的展开集合（会话内状态）。
    pub history_expanded: Vec<i64>,
    /// 历史行级菜单的行 id（空 = 未展开）。
    pub history_menu_id: String,
}

/// 导入归类预览一行（只含可显示字段；计数已由 Rust 格式化为文本）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrganizeRowView {
    pub name: String,
    pub count: String,
}

/// 行内菜单项标识（`CategoryAction` 的字符串口径，供 `row-action` 复用）。
pub mod category_action {
    /// 新建子分类。
    pub const ADD_CHILD: &str = "category-add-child";
    /// 重命名。
    pub const RENAME: &str = "category-rename";
    /// 移动到分类。
    pub const MOVE: &str = "category-move";
    /// 删除。
    pub const DELETE: &str = "category-delete";
}

/// Toast（右下角；Rust 一次性定时器 3s 清除）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToastView {
    /// 是否可见。
    pub visible: bool,
    /// 文案。
    pub text: String,
    /// 动作标签（空 = 无动作）。
    pub action_label: String,
    /// 动作标识（Rust 内部使用）。
    pub action: String,
    /// 令牌（防止旧定时器清除新提示）。
    pub token: u64,
}

/// 导入页可显示字段（S11）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportView {
    /// 文件名。
    pub file: String,
    /// 体积。
    pub size: String,
    /// 百分比（0..100）。
    pub percent: i32,
    /// 四步流程。
    pub steps: Vec<StepRowView>,
    /// 说明（「导入完成后才能继续」）。
    pub note: String,
}

/// 错误页可显示字段（S12）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ErrorView {
    /// 结论式标题（用户语言）。
    pub title: String,
    /// 可理解原因。
    pub reason: String,
    /// 技术详情（mono）。
    pub details: String,
}

/// 设置页可显示字段（持久化由 `crate::settings` 接入核心设置表）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SettingsView {
    /// 表单行（含分组标题行，顺序即界面顺序）。
    pub rows: Vec<SettingsRowView>,
    /// 页头说明。
    pub note: String,
    /// 数据库位置（mono）。
    pub data_path: String,
    /// 表单内容高度（像素；滚动视口用，Rust 与 Slint 同口径）。
    pub content_height: u32,
    /// 设置页滚动偏移（像素；Rust 持有，界面只读投影）。
    pub scroll_y: u32,
    /// 设置页视口高度（像素；Rust 按窗口尺寸计算）。
    pub viewport_height: u32,
    /// 下拉框是否展开（选择型字段，如「字体族」）。
    pub select_open: bool,
    /// 下拉框纵向锚点（内容坐标；= 行 y + 行高 + 4）。
    pub select_y: u32,
    /// 展开中的字段键（未展开为空串）。
    pub select_key: String,
    /// 下拉框当前选中项下标。
    pub select_index: i32,
    /// 下拉框选项列表。
    pub select_options: Vec<String>,
    /// 主题（`system` / `light` / `dark`）。
    pub theme: String,
    /// 交互模式（`buttons` / `gesture`）。
    pub interaction_mode: String,
    /// 沉浸时自动隐藏工具条。
    pub immersive_autohide: bool,
    /// 全部设置是否已持久化（`origin == stored`）。
    pub persisted: bool,
}

impl Default for ReaderView {
    fn default() -> Self {
        Self {
            book_title: String::new(),
            chapter_title: String::new(),
            chapter_label: String::new(),
            heading: String::new(),
            page_text: String::new(),
            page_label: String::new(),
            toolbar_open: false,
            column_x: 0,
            column_y: 0,
            column_width: 0,
            column_height: 0,
            progress_permille: 0,
            reflow_status: String::new(),
            toc_open: false,
            toc_rows: Vec::new(),
            toc_window_first: 0,
            toc_ratio_permille: 0,
            toc_handle_permille: 200,
            toc_count_label: String::new(),
            toc_readout: String::new(),
            toc_search: String::new(),
            toc_match_total: 0,
            chapter_sequence: 0,
            chapter_total: 0,
            marks_open: false,
            mark_rows: Vec::new(),
            mark_window_first: 0,
            mark_total: 0,
            mark_count_label: String::new(),
            mark_readout: String::new(),
            mark_search: String::new(),
            mark_selected: String::new(),
            mark_rename_text: String::new(),
            mark_delete_pending: String::new(),
            progress_status: String::new(),
            gesture_mode: false,
            immersive: false,
            floating_toolbar: false,
            progress_percent: String::new(),
            encoding_name: String::new(),
            encoding_manual: false,
            encoding_open: false,
            encoding_rows: Vec::new(),
            shortcut_prev: String::new(),
            shortcut_next: String::new(),
            shortcut_immersive: String::new(),
        }
    }
}

/// 一次投影的完整可显示状态（Slint 侧只读入参）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UiSnapshot {
    /// 当前页面。
    pub screen: Screen,
    /// 状态栏文本。
    pub status_text: String,
    /// 应用版本（核心版本 + ABI 版本）。
    pub app_version: String,
    /// 书架页字段。
    pub shelf: ShelfView,
    /// 阅读历史视图（V1.1；`history_mode` 为真时界面用它替换书架列表）。
    pub history: HistoryView,
    /// 是否处于历史视图。
    pub history_mode: bool,
    /// 阅读页字段。
    pub reader: ReaderView,
    /// 设置页字段。
    pub settings: SettingsView,
    /// 弹窗字段。
    pub dialog: DialogView,
    /// Toast 字段。
    pub toast: ToastView,
    /// 导入页字段。
    pub import: ImportView,
    /// 错误页字段。
    pub error: ErrorView,
    /// 是否深色主题。
    pub theme_dark: bool,
    /// 过渡页状态文案（导入页状态行）。
    pub placeholder_status: String,
}

/// 书架为空时的提示文案（Rust 侧唯一来源；不得出现内部里程碑代号）。
pub const SHELF_EMPTY_TITLE: &str = "书架还是空的";
/// 空态说明。
pub const SHELF_EMPTY_DESC: &str =
    "导入一本 TXT 小说即可开始阅读：导入只登记来源与阅读位置，不会修改你的原始文件。";
/// 空态补充说明（三条）。
pub const SHELF_EMPTY_NOTES: [&str; 3] = [
    "支持格式：单个 .txt 文件（后续版本再支持 ZIP / EPUB 等）",
    "编码：先自动识别；识别不准时可在阅读页「选项」里手动指定",
    "数据位置：书架记录、进度与书签保存在本地库；原始 TXT 始终留在原处",
];
/// 未打开书籍时阅读页的提示文案（Rust 侧唯一来源）。
pub const READER_EMPTY_TEXT: &str = "尚未打开书籍：请在书架导入 TXT 后开始阅读。";
/// 未打开书籍时的页码标签。
pub const READER_EMPTY_LABEL: &str = "—";
/// 未打开书籍时的工具条标题。
pub const READER_EMPTY_HEADING: &str = "未打开书籍";

/// 按书名排序的标识。
pub const SORT_RECENT: i32 = 0;
/// 排序：书名。
pub const SORT_TITLE: i32 = 1;
/// 排序：导入时间。
pub const SORT_IMPORTED: i32 = 2;

/// 书架排序显示名。
pub fn sort_label(sort: i32) -> &'static str {
    match sort {
        SORT_TITLE => "按书名",
        SORT_IMPORTED => "按导入时间",
        _ => "按最近阅读",
    }
}

/// 后台读取书架（DT1 唯一真实后台任务；导入/索引由 DT2 起接入）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShelfRequest {
    /// 请求号（单调递增）。
    pub request_id: u64,
    /// 布局号（布局变化后旧结果作废）。
    pub layout_id: u64,
    /// 库文件路径。
    pub db_path: PathBuf,
    /// 当前选中的分类 id 字符串（V1.1；空 = 根「全部书籍」）。
    pub selected_category: String,
    /// 选中分类的真实 id（`None` = 根；`Some(-1)` = 未分类；V1.1）。
    pub selected_id: Option<i64>,
    /// 折叠的分类 id 集合（V1.1）。
    pub collapsed_categories: std::collections::HashSet<i64>,
}

impl ShelfRequest {
    /// 构造「根作用域」的请求（不带分类过滤；V1 既有行为的默认值）。
    pub fn new(request_id: u64, layout_id: u64, db_path: PathBuf) -> Self {
        Self {
            request_id,
            layout_id,
            db_path,
            selected_category: String::new(),
            selected_id: None,
            collapsed_categories: std::collections::HashSet::new(),
        }
    }
}

/// 后台结果载荷。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackgroundPayload {
    /// 书架数据。
    Shelf {
        /// 书架行（未过滤、按默认顺序）。
        rows: Vec<ShelfRow>,
        /// 书架记录数（含已移出）。
        total_count: i64,
        /// 已移出数。
        removed_count: i64,
        /// 设置投影（必须完整）。
        settings: Box<SettingsView>,
        /// 状态栏文本。
        status_text: String,
        /// 分类侧栏行（V1.1；由 `category_tree::project` 投影）。
        categories: Vec<CategoryRow>,
        /// 过滤后的行数（用于页头徽标；V1.1）。
        filtered_count: usize,
    },
    /// 失败（保留可读信息，不把错误文本当业务分支）。
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
    /// 原始书架行（后台投影结果；过滤/排序在 [`AppState::project`] 完成）。
    shelf_rows: Vec<ShelfRow>,
    shelf_total: i64,
    shelf_removed: i64,
    shelf_sort: i32,
    shelf_view: i32,
    shelf_search: String,
    shelf_menu_id: Option<String>,
    /// 当前选中的分类 id（字符串；空 = 根「全部书籍」，`category_tree::UNCATEGORIZED_SELECTED_ID`
    /// = 未分类）。V1.1 新增。
    shelf_category: String,
    /// 折叠的分类 id 集合（会话内状态；V1.1 新增）。
    shelf_collapsed: std::collections::HashSet<i64>,
    /// 侧栏分类树行（后台投影；V1.1 新增）。
    shelf_categories: Vec<CategoryRow>,
    /// 当前打开行内菜单的分类 id（V1.1 新增；空串 = 未打开）。
    shelf_category_menu_id: String,
    /// 阅读历史视图投影（V1.1 新增）。
    history: HistoryView,
    /// 是否处于历史视图（V1.1 新增）。
    history_mode: bool,
    /// 按书聚合的展开集合（V1.1 新增；会话内状态）。
    history_expanded: Vec<i64>,
    shelf_notice_ignored: bool,
    reader: ReaderView,
    settings: SettingsView,
    dialog: DialogView,
    toast: ToastView,
    import: ImportView,
    error: ErrorView,
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
            shelf_total: 0,
            shelf_removed: 0,
            shelf_sort: SORT_RECENT,
            shelf_view: 0,
            shelf_search: String::new(),
            shelf_menu_id: None,
            shelf_category: String::new(),
            shelf_collapsed: std::collections::HashSet::new(),
            shelf_categories: Vec::new(),
            shelf_category_menu_id: String::new(),
            history: HistoryView::default(),
            history_mode: false,
            history_expanded: Vec::new(),
            shelf_notice_ignored: false,
            reader: ReaderView::default(),
            settings: SettingsView::default(),
            dialog: DialogView::default(),
            toast: ToastView::default(),
            import: ImportView::default(),
            error: ErrorView::default(),
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

    /// 原始书架行（未过滤）。
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

    /// 弹窗字段。
    pub fn dialog(&self) -> &DialogView {
        &self.dialog
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

    /// 切换阅读工具条展开状态；返回是否变化。
    pub fn toggle_reader_toolbar(&mut self) -> bool {
        self.reader.toolbar_open = !self.reader.toolbar_open;
        true
    }

    /// 工具条是否展开（分页重排取此状态）。
    pub fn toolbar_open(&self) -> bool {
        self.reader.toolbar_open
    }

    /// 书架视图（0=表格 1=卡片）。
    pub fn shelf_view(&self) -> i32 {
        self.shelf_view
    }

    /// 设置书架视图。
    pub fn set_shelf_view(&mut self, view: i32) {
        self.shelf_view = view.clamp(0, 1);
    }

    /// 书架排序。
    pub fn shelf_sort(&self) -> i32 {
        self.shelf_sort
    }

    /// 设置书架排序。
    pub fn set_shelf_sort(&mut self, sort: i32) {
        self.shelf_sort = if (0..=SORT_IMPORTED).contains(&sort) {
            sort
        } else {
            SORT_RECENT
        };
    }

    /// 循环切换排序（点击排序字段）。
    pub fn cycle_shelf_sort(&mut self) {
        self.shelf_sort = (self.shelf_sort + 1) % (SORT_IMPORTED + 1);
    }

    /// 书架搜索关键词。
    pub fn shelf_search(&self) -> &str {
        &self.shelf_search
    }

    /// 设置书架搜索关键词。
    pub fn set_shelf_search(&mut self, text: impl Into<String>) {
        self.shelf_search = text.into();
    }

    // ---- V1.1 分类侧栏状态（规格 R1）----

    /// 当前选中的分类 id（空 = 根「全部书籍」）。
    pub fn shelf_category(&self) -> &str {
        &self.shelf_category
    }

    /// 选中分类（规格 R1；`id` 为空串 = 回到根）。
    pub fn set_shelf_category(&mut self, id: impl Into<String>) {
        self.shelf_category = id.into();
    }

    /// 侧栏分类树行（只读投影）。
    pub fn shelf_categories(&self) -> &[CategoryRow] {
        &self.shelf_categories
    }

    /// 用后台投影结果替换侧栏树。
    pub fn set_shelf_categories(&mut self, rows: Vec<CategoryRow>) {
        self.shelf_categories = rows;
    }

    /// 折叠集合（只读）。
    pub fn shelf_collapsed(&self) -> &std::collections::HashSet<i64> {
        &self.shelf_collapsed
    }

    /// 切换某分类的折叠状态（规格 R1）。
    pub fn toggle_shelf_collapsed(&mut self, id: i64) {
        if !self.shelf_collapsed.remove(&id) {
            self.shelf_collapsed.insert(id);
        }
    }

    /// 当前折叠集合的副本（供后台投影使用）。
    pub fn shelf_collapsed_snapshot(&self) -> std::collections::HashSet<i64> {
        self.shelf_collapsed.clone()
    }

    /// 当前选中分类的 id（供后台投影用；`None` = 根）。
    pub fn shelf_selected_id(&self) -> Option<i64> {
        if self.shelf_category.is_empty() {
            return None;
        }
        if self.shelf_category == crate::category_tree::UNCATEGORIZED_SELECTED_ID {
            return Some(crate::category_tree::UNCATEGORIZED_SENTINEL);
        }
        self.shelf_category.parse::<i64>().ok()
    }

    /// 当前打开行内菜单的分类 id（空串 = 未打开）。
    pub fn category_menu_id(&self) -> &str {
        &self.shelf_category_menu_id
    }

    /// 打开某分类的行内菜单。
    pub fn set_category_menu_id(&mut self, id: impl Into<String>) {
        self.shelf_category_menu_id = id.into();
    }

    /// 关闭行内菜单。
    pub fn close_category_menu(&mut self) {
        self.shelf_category_menu_id = String::new();
    }

    /// 行内菜单挂在第几行（界面按此定位；**-1 = 未打开**）。
    ///
    /// 下标由**已投影的行**反查（`categories`），界面不自己找。
    pub fn category_menu_offset(&self) -> i32 {
        if self.shelf_category_menu_id.is_empty() {
            return -1;
        }
        self.shelf_categories
            .iter()
            .position(|row| row.id == self.shelf_category_menu_id)
            .map(|index| index as i32)
            .unwrap_or(-1)
    }

    // ---- V1.1 阅读历史（规格 R8/R9）----

    /// 是否处于历史视图。
    pub fn history_mode(&self) -> bool {
        self.history_mode
    }

    /// 进入 / 离开历史视图。
    pub fn set_history_mode(&mut self, on: bool) {
        self.history_mode = on;
    }

    /// 历史视图投影（只读）。
    pub fn history(&self) -> &HistoryView {
        &self.history
    }

    /// 用后台投影结果替换历史视图。
    pub fn set_history(&mut self, view: HistoryView) {
        self.history = view;
    }

    /// 切换某本书的聚合展开状态（设计稿 H2）。
    pub fn toggle_history_group(&mut self, book_id: i64) {
        match self.history_expanded.iter().position(|id| *id == book_id) {
            Some(index) => {
                self.history_expanded.remove(index);
            }
            None => self.history_expanded.push(book_id),
        }
    }

    /// 展开集合（只读）。
    pub fn history_expanded(&self) -> &[i64] {
        &self.history_expanded
    }

    /// 切换流水 / 按书聚合视图（设计稿 H2 的开关）。
    ///
    /// **仅界面局部状态**：本期不持久化（重启回到时间倒序，见设计稿 §9.2）。
    pub fn set_history_grouped(&mut self, grouped: bool) {
        self.history.grouped = grouped;
    }

    /// 展开/收起行级菜单（`index` 为行下标；负值 = 收起）。
    pub fn set_shelf_menu_index(&mut self, index: i32) {
        if index < 0 {
            self.shelf_menu_id = None;
            return;
        }
        let rows = self.filtered_shelf_rows();
        self.shelf_menu_id = rows.get(index as usize).map(|row| row.id.clone());
    }

    /// 当前展开行级菜单的行下标（-1 = 未展开）。
    pub fn shelf_menu_index(&self) -> i32 {
        let Some(id) = &self.shelf_menu_id else {
            return -1;
        };
        self.filtered_shelf_rows()
            .iter()
            .position(|row| &row.id == id)
            .map(|index| index as i32)
            .unwrap_or(-1)
    }

    /// 会话内忽略失效通知。
    pub fn ignore_shelf_notice(&mut self) {
        self.shelf_notice_ignored = true;
    }

    /// 申请一个新的后台请求（请求号单调递增）。
    ///
    /// 携带当前分类作用域与折叠状态（V1.1）—— 后台据此过滤行并投影侧栏。
    pub fn next_shelf_request(&mut self, db_path: PathBuf) -> ShelfRequest {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        ShelfRequest {
            request_id,
            layout_id: self.current_layout_id,
            db_path,
            selected_category: self.shelf_category.clone(),
            selected_id: self.shelf_selected_id(),
            collapsed_categories: self.shelf_collapsed_snapshot(),
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
                total_count,
                removed_count,
                settings,
                status_text,
                categories,
                filtered_count: _,
            } => {
                self.shelf_rows = rows;
                self.shelf_categories = categories;
                self.shelf_total = total_count;
                self.shelf_removed = removed_count;
                self.settings = *settings;
                self.status_text = status_text;
            }
            BackgroundPayload::Failure { message } => {
                self.status_text = message.clone();
                self.screen = Screen::Error;
                self.error.title = "书架数据不可读".to_string();
                self.error.reason = message.clone();
                self.error.details = message;
            }
        }
        self.last_applied_request_id = result.request_id;
        self.applied_results += 1;
        ApplyOutcome::Applied
    }

    /// 设置阅读页可显示字段（正文来自分页运行时的有限窗口测量结果）。
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

    /// 设置工具条居中标题（「书名 · 章节」）。
    pub fn set_reader_heading(&mut self, heading: impl Into<String>) {
        self.reader.heading = heading.into();
    }

    /// 设置正文列几何（窗口坐标；测量与渲染同口径）。
    pub fn set_reader_column(&mut self, x: u32, y: u32, width: u32, height: u32) {
        self.reader.column_x = x;
        self.reader.column_y = y;
        self.reader.column_width = width;
        self.reader.column_height = height;
    }

    /// 设置重排状态文案（只改状态文案，绝不触碰正文）。
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

    /// 设置目录窗口（有界；过滤后的匹配窗口）。
    pub fn set_toc_rows(
        &mut self,
        rows: Vec<TocRow>,
        window_first: u32,
        current: u32,
        total: usize,
        match_total: usize,
        readout: String,
        count_label: String,
    ) {
        self.reader.toc_rows = rows;
        self.reader.toc_window_first = window_first;
        self.reader.chapter_sequence = current;
        self.reader.chapter_total = total;
        self.reader.toc_match_total = match_total;
        self.reader.toc_readout = readout;
        self.reader.toc_count_label = count_label;
        let visible = self.reader.toc_rows.len().max(1);
        let max_first = match_total.saturating_sub(visible);
        self.reader.toc_ratio_permille = if max_first == 0 {
            0
        } else {
            ((window_first.min(max_first as u32) as f64 / max_first as f64) * 1_000.0).round()
                as u32
        };
        self.reader.toc_handle_permille = if match_total == 0 {
            1_000
        } else {
            ((visible as f64 / match_total as f64) * 1_000.0)
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
        self.reader.toc_open
    }

    /// 目录搜索关键词。
    pub fn toc_search(&self) -> &str {
        &self.reader.toc_search
    }

    /// 设置目录搜索关键词。
    pub fn set_toc_search(&mut self, text: impl Into<String>) {
        self.reader.toc_search = text.into();
    }

    /// 设置书签窗口（有界）与总数。
    pub fn set_mark_rows(
        &mut self,
        rows: Vec<MarkRow>,
        window_first: u32,
        total: usize,
        readout: String,
        count_label: String,
    ) {
        self.reader.mark_rows = rows;
        self.reader.mark_window_first = window_first;
        self.reader.mark_total = total;
        self.reader.mark_readout = readout;
        self.reader.mark_count_label = count_label;
    }

    /// 切换书签面板；返回是否变化。
    pub fn toggle_marks_panel(&mut self) -> bool {
        self.reader.marks_open = !self.reader.marks_open;
        self.reader.marks_open
    }

    /// 书签搜索关键词。
    pub fn mark_search(&self) -> &str {
        &self.reader.mark_search
    }

    /// 设置书签搜索关键词。
    pub fn set_mark_search(&mut self, text: impl Into<String>) {
        self.reader.mark_search = text.into();
    }

    /// 设置/清空选中的书签标识。
    pub fn set_mark_selected(&mut self, id: impl Into<String>) {
        self.reader.mark_selected = id.into();
    }

    /// 选中书签标识。
    pub fn mark_selected(&self) -> &str {
        &self.reader.mark_selected
    }

    /// 设置重命名输入框文本（UI 双向绑定）。
    pub fn set_mark_rename_text(&mut self, text: impl Into<String>) {
        self.reader.mark_rename_text = text.into();
    }

    /// 设置删除待确认的书签（行内两段确认）。
    pub fn set_mark_delete_pending(&mut self, id: impl Into<String>) {
        self.reader.mark_delete_pending = id.into();
    }

    /// 删除待确认的书签 id。
    pub fn mark_delete_pending(&self) -> &str {
        &self.reader.mark_delete_pending
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
    pub fn set_encoding_rows(&mut self, rows: Vec<ChoiceRow>) {
        self.reader.encoding_rows = rows;
    }

    /// 设置沉浸阅读状态（界面显隐的唯一来源；Rust 侧同时据此重排）。
    pub fn set_immersive(&mut self, immersive: bool) {
        self.reader.immersive = immersive;
        if !immersive {
            self.reader.floating_toolbar = false;
        }
    }

    /// 设置沉浸态浮出工具条可见性。
    pub fn set_floating_toolbar(&mut self, visible: bool) {
        self.reader.floating_toolbar = visible;
    }

    /// 沉浸态浮出工具条是否可见。
    pub fn floating_toolbar(&self) -> bool {
        self.reader.floating_toolbar
    }

    /// 是否沉浸。
    pub fn immersive(&self) -> bool {
        self.reader.immersive
    }

    /// 投影阅读进度读数（按字节位置；与页码无关）。
    pub fn set_progress_percent(&mut self, text: impl Into<String>) {
        self.reader.progress_percent = text.into();
    }

    /// 设置阅读进度千分比（进度条）。
    pub fn set_progress_permille(&mut self, permille: u32) {
        self.reader.progress_permille = permille.min(1_000);
    }

    /// 设置工具条快捷键提示。
    pub fn set_shortcuts(&mut self, prev: String, next: String, immersive: String) {
        self.reader.shortcut_prev = prev;
        self.reader.shortcut_next = next;
        self.reader.shortcut_immersive = immersive;
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

    /// 打开确认弹窗。
    #[allow(clippy::too_many_arguments)]
    pub fn open_confirm(
        &mut self,
        action: impl Into<String>,
        book_id: impl Into<String>,
        title: impl Into<String>,
        body: impl Into<String>,
        note: impl Into<String>,
        checkbox_label: impl Into<String>,
        confirm_label: impl Into<String>,
        danger: bool,
    ) {
        self.dialog = DialogView {
            kind: 1,
            confirm_title: title.into(),
            confirm_body: body.into(),
            confirm_note: note.into(),
            confirm_checkbox_label: checkbox_label.into(),
            confirm_label: confirm_label.into(),
            confirm_danger: danger,
            confirm_action: action.into(),
            confirm_book_id: book_id.into(),
            ..DialogView::default()
        };
    }

    /// 打开快捷键说明弹窗。
    pub fn open_shortcuts(&mut self, rows: Vec<ShortcutEntryView>) {
        self.dialog = DialogView {
            kind: 2,
            shortcuts: rows,
            ..DialogView::default()
        };
    }

    /// 打开选项弹窗（S8：编码等）。
    pub fn open_choices(
        &mut self,
        title: impl Into<String>,
        meta: impl Into<String>,
        rows: Vec<ChoiceRow>,
        current_index: i32,
    ) {
        self.dialog = DialogView {
            kind: 3,
            choice_title: title.into(),
            choice_meta: meta.into(),
            choice_rows: rows,
            choice_current_index: current_index,
            ..DialogView::default()
        };
    }

    /// 打开重命名弹窗。
    pub fn open_rename(
        &mut self,
        title: impl Into<String>,
        value: impl Into<String>,
        book_id: impl Into<String>,
    ) {
        self.dialog = DialogView {
            kind: 4,
            rename_title: title.into(),
            rename_value: value.into(),
            rename_book_id: book_id.into(),
            ..DialogView::default()
        };
    }

    /// 关闭弹窗。
    pub fn close_dialog(&mut self) {
        self.dialog = DialogView::default();
    }

    /// 整体替换弹窗状态（V1.1 分类浮层用：整体替换避免逐字段遗漏）。
    pub fn set_dialog(&mut self, dialog: DialogView) {
        self.dialog = dialog;
    }

    /// 当前弹窗种类。
    pub fn dialog_kind(&self) -> i32 {
        self.dialog.kind
    }

    /// 显示 Toast（令牌由调用方递增；旧定时器按令牌失效）。
    pub fn show_toast(
        &mut self,
        text: impl Into<String>,
        action_label: impl Into<String>,
        action: impl Into<String>,
        token: u64,
    ) {
        self.toast = ToastView {
            visible: true,
            text: text.into(),
            action_label: action_label.into(),
            action: action.into(),
            token,
        };
    }

    /// 清除 Toast（仅当令牌匹配；返回是否真的清除）。
    pub fn clear_toast(&mut self, token: u64) -> bool {
        if self.toast.visible && self.toast.token == token {
            self.toast = ToastView::default();
            return true;
        }
        false
    }

    /// Toast 当前令牌。
    pub fn toast_token(&self) -> u64 {
        self.toast.token
    }

    /// Toast 动作标识。
    pub fn toast_action(&self) -> &str {
        &self.toast.action
    }

    /// 设置页最大滚动偏移（内容高度 - 视口高度；不足一屏时为 0）。
    pub fn settings_scroll_max(&self) -> u32 {
        self.settings
            .content_height
            .saturating_sub(self.settings.viewport_height)
    }

    /// 展开/收起设置页下拉框（同一字段再次触发即收起）。
    ///
    /// 下拉列表与书架排序下拉同一交互口径：点击字段展开、高亮当前项、
    /// 点选生效并收起、点其他处收起。
    pub fn toggle_settings_select(&mut self, key: &str) {
        if self.settings.select_open && self.settings.select_key == key {
            self.close_settings_select();
            return;
        }
        let Some(row) = self
            .settings
            .rows
            .iter()
            .find(|row| row.key == key && row.kind == 0)
        else {
            return;
        };
        self.settings.select_open = true;
        self.settings.select_key = key.to_string();
        // 锚点 = 字段底边 + 4：选择型字段在行内位于 y = space-8、高 = button-height-compact，
        // 故不能用行高（会把列表推到字段下方约 16 像素之外，观感“离得太远”）。
        self.settings.select_y = row.offset_y
            + crate::layout::SPACE_8
            + crate::layout::BUTTON_HEIGHT_COMPACT
            + crate::layout::SPACE_4;
        self.settings.select_index = row.current_index;
        self.settings.select_options = row.options.clone();
    }

    /// 收起设置页下拉框。
    pub fn close_settings_select(&mut self) {
        self.settings.select_open = false;
        self.settings.select_key.clear();
        self.settings.select_options.clear();
    }

    /// 设置页滚动偏移（像素）。
    pub fn settings_scroll(&self) -> u32 {
        self.settings.scroll_y
    }

    /// 设置页视口高度（像素）。
    pub fn settings_viewport(&self) -> u32 {
        self.settings.viewport_height
    }

    /// 设定视口高度（窗口变化时由编排层更新，并夹紧滚动位置）。
    pub fn set_settings_viewport(&mut self, viewport_height: u32) {
        self.settings.viewport_height = viewport_height;
        let max = self.settings_scroll_max();
        self.settings.scroll_y = self.settings.scroll_y.min(max);
    }

    /// 按像素增量滚动（滚轮 / 键盘共用；正数向下）。
    pub fn scroll_settings_by(&mut self, delta: i64) {
        let max = self.settings_scroll_max() as i64;
        let next = (self.settings.scroll_y as i64 + delta).clamp(0, max);
        self.settings.scroll_y = next as u32;
    }

    /// 按比例滚动（滚动条拖动；`ratio` ∈ [0,1]）。
    pub fn scroll_settings_to(&mut self, ratio: f64) {
        let max = self.settings_scroll_max() as f64;
        let next = (ratio.clamp(0.0, 1.0) * max).round();
        self.settings.scroll_y = next as u32;
    }

    /// 投影导入页字段。
    pub fn set_import_view(&mut self, view: ImportView) {
        self.import = view;
    }

    /// 投影错误页字段。
    pub fn set_error_view(&mut self, title: String, reason: String, details: String) {
        self.error.title = title;
        self.error.reason = reason;
        self.error.details = details;
    }

    /// 过滤 + 排序后的书架行（纯计算；搜索匹配书名/元信息）。
    pub fn filtered_shelf_rows(&self) -> Vec<ShelfRow> {
        let keyword = self.shelf_search.trim().to_lowercase();
        let mut rows: Vec<ShelfRow> = self
            .shelf_rows
            .iter()
            .filter(|row| {
                keyword.is_empty()
                    || row.title.to_lowercase().contains(&keyword)
                    || row.meta.to_lowercase().contains(&keyword)
            })
            .cloned()
            .collect();
        match self.shelf_sort {
            SORT_TITLE => rows.sort_by(|a, b| a.title.cmp(&b.title)),
            SORT_IMPORTED => rows.sort_by(|a, b| b.created_epoch.cmp(&a.created_epoch)),
            _ => rows.sort_by(|a, b| b.recent_epoch.cmp(&a.recent_epoch)),
        }
        rows
    }

    /// 当前状态的只读投影。
    pub fn project(&self) -> UiSnapshot {
        let rows = self.filtered_shelf_rows();
        let invalid = rows.iter().filter(|row| row.needs_relink).count();
        let shelf_empty = self.shelf_rows.is_empty();
        let shelf = ShelfView {
            categories: self.shelf_categories.clone(),
            selected_category_id: self.shelf_category.clone(),
            sidebar_visible: true,
            category_menu_id: self.shelf_category_menu_id.clone(),
            category_menu_offset: self.category_menu_offset(),
            rows,
            empty: shelf_empty,
            empty_title: SHELF_EMPTY_TITLE.to_string(),
            empty_desc: SHELF_EMPTY_DESC.to_string(),
            empty_notes: SHELF_EMPTY_NOTES.iter().map(|text| text.to_string()).collect(),
            view: self.shelf_view,
            search: self.shelf_search.clone(),
            sort_label: sort_label(self.shelf_sort).to_string(),
            sort_index: self.shelf_sort,
            count_label: if shelf_empty {
                "0 本".to_string()
            } else if self.shelf_removed > 0 {
                format!("{} 本 · {} 本已移出", self.shelf_total - self.shelf_removed, self.shelf_removed)
            } else {
                format!("{} 本", self.shelf_total - self.shelf_removed)
            },
            notice: invalid > 0 && !self.shelf_notice_ignored,
            notice_title: format!("{invalid} 本书的源文件已失效"),
            notice_body: "原始 TXT 文件未被修改或删除：重新选择同一本书的文件即可恢复阅读，进度与书签会保留。"
                .to_string(),
            menu_index: self.shelf_menu_index(),
            notice_ignored: self.shelf_notice_ignored,
        };

        let mut reader = self.reader.clone();
        if reader.book_title.is_empty() {
            reader.page_text = READER_EMPTY_TEXT.to_string();
            reader.page_label = READER_EMPTY_LABEL.to_string();
            reader.chapter_title = "未打开书籍".to_string();
            reader.heading = READER_EMPTY_HEADING.to_string();
        }

        UiSnapshot {
            screen: self.screen,
            status_text: self.status_text.clone(),
            app_version: self.app_version.clone(),
            shelf,
            history: self.history.clone(),
            history_mode: self.history_mode,
            reader,
            settings: self.settings.clone(),
            dialog: self.dialog.clone(),
            toast: self.toast.clone(),
            import: self.import.clone(),
            error: self.error.clone(),
            theme_dark: crate::settings::theme_is_dark(&self.settings.theme),
            placeholder_status: self.status_text.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn category_row(id: &str, name: &str, depth: i32) -> CategoryRow {
        CategoryRow {
            id: id.to_string(),
            name: name.to_string(),
            depth,
            direct_count: 0,
            total_count: 0,
            has_children: false,
            expanded: true,
            selected: false,
            move_blocked: false,
            virtual_row: false,
            kind: 2,
            parent_id: String::new(),
        }
    }

    #[test]
    fn category_menu_offset_resolves_row_index_from_projected_rows() {
        let mut state = AppState::new("0.5.0");
        state.set_shelf_categories(vec![
            category_row("", "全部书籍", 0),
            category_row("uncategorized", "未分类", 1),
            category_row("1", "科幻", 1),
            category_row("2", "中国科幻", 2),
        ]);

        // 未打开 → -1。
        assert_eq!(state.category_menu_offset(), -1);

        // 打开第 3 行（科幻）。
        state.set_category_menu_id("1");
        assert_eq!(state.category_menu_offset(), 2);

        // 打开子分类行。
        state.set_category_menu_id("2");
        assert_eq!(state.category_menu_offset(), 3);

        // 虚拟行也可打开（但界面不显示「…」，此处只保证下标可解析）。
        state.set_category_menu_id("uncategorized");
        assert_eq!(state.category_menu_offset(), 1);

        // 未知 id → -1（界面据此不渲染菜单），**不**回退到 0。
        state.set_category_menu_id("999");
        assert_eq!(state.category_menu_offset(), -1);

        // 关闭后回到 -1。
        state.set_category_menu_id("1");
        state.close_category_menu();
        assert_eq!(state.category_menu_offset(), -1);
    }

    fn shelf_result(request_id: u64, layout_id: u64, rows: Vec<ShelfRow>) -> BackgroundResult {
        BackgroundResult {
            request_id,
            layout_id,
            payload: BackgroundPayload::Shelf {
                rows,
                total_count: 1,
                removed_count: 0,
                settings: Box::new(SettingsView::default()),
                status_text: format!("已加载 {request_id}"),
                categories: Vec::new(),
                filtered_count: 1,
            },
        }
    }

    fn row(id: &str) -> ShelfRow {
        ShelfRow {
            id: id.to_string(),
            title: format!("书 {id}"),
            initial: "书".to_string(),
            meta: format!("/tmp/{id}.txt · 1.0 KB"),
            status: "可阅读".to_string(),
            status_tone: 2,
            progress: "第 1 章 · 偏移 0".to_string(),
            percent: 0,
            recent: String::new(),
            needs_relink: false,
            can_open: true,
            recent_epoch: 0,
            created_epoch: 0,
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
        let mut state = AppState::new("core 0.2.0 / abi 1");
        state.navigate(Screen::Settings.index());
        let snapshot = state.project();
        assert_eq!(snapshot.screen, Screen::Settings);
        assert_eq!(snapshot.app_version, "core 0.2.0 / abi 1");
        assert!(snapshot.shelf.rows.is_empty(), "不得注入任何模拟书籍");
        assert!(snapshot.shelf.empty);
        assert_eq!(snapshot.shelf.empty_title, SHELF_EMPTY_TITLE);
        assert_eq!(snapshot.reader.page_text, READER_EMPTY_TEXT);
        assert_eq!(snapshot.reader.page_label, READER_EMPTY_LABEL);
        assert!(!snapshot.theme_dark);
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
        assert_eq!(state.project().error.reason, "库文件不可读");
    }

    #[test]
    fn reader_toolbar_toggle_is_state_owned() {
        let mut state = AppState::new("test");
        assert!(!state.toolbar_open());
        assert!(state.toggle_reader_toolbar());
        assert!(state.toolbar_open());
        assert!(state.toggle_reader_toolbar());
        assert!(!state.toolbar_open());
    }

    #[test]
    fn reflow_status_is_projected_without_touching_page() {
        let mut state = AppState::new("test");
        state.set_reader_page("书 1", "第一章", "正文第一页", "第 1 页");
        state.set_reflow_status("正在重排…");
        let snapshot = state.project();
        assert_eq!(snapshot.reader.reflow_status, "正在重排…");
        assert_eq!(snapshot.reader.page_text, "正文第一页");
        assert_eq!(snapshot.reader.page_label, "第 1 页");
        assert_eq!(snapshot.reader.book_title, "书 1");

        state.set_reflow_status("重排完成");
        let snapshot = state.project();
        assert_eq!(snapshot.reader.reflow_status, "重排完成");
        assert_eq!(snapshot.reader.page_text, "正文第一页");
    }

    #[test]
    fn shelf_search_and_sort_are_computed_in_projection() {
        let mut state = AppState::new("test");
        let mut first = row("1");
        first.recent_epoch = 10;
        first.created_epoch = 30;
        let mut second = row("2");
        second.recent_epoch = 20;
        second.created_epoch = 5;
        state.apply_result(shelf_result(1, 1, vec![first.clone(), second.clone()]));

        // 默认按最近阅读倒序（书 2 更近）。
        let rows = state.project().shelf.rows;
        assert_eq!(rows[0].title, "书 2");
        // 按书名排序（码点序：`书 1` < `书 2`）。
        state.set_shelf_sort(SORT_TITLE);
        let rows = state.project().shelf.rows;
        assert_eq!(rows[0].title, "书 1");
        // 按导入时间倒序（书 1 的导入时间更新）。
        state.set_shelf_sort(SORT_IMPORTED);
        let rows = state.project().shelf.rows;
        assert_eq!(rows[0].title, "书 1");
        // 搜索过滤（匹配书名）。
        state.set_shelf_search("书 1");
        let rows = state.project().shelf.rows;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "书 1");
    }

    #[test]
    fn invalid_sources_surface_a_notice_once_ignored_it_stays_hidden() {
        let mut state = AppState::new("test");
        let mut broken = row("1");
        broken.needs_relink = true;
        broken.status = "需要重新关联".to_string();
        broken.status_tone = 4;
        state.apply_result(shelf_result(1, 1, vec![broken]));
        assert!(state.project().shelf.notice);
        state.ignore_shelf_notice();
        assert!(!state.project().shelf.notice);
    }

    #[test]
    fn dialog_and_toast_are_state_owned() {
        let mut state = AppState::new("test");
        state.open_confirm(
            "remove",
            "7",
            "移出书架",
            "正文",
            "原始 TXT 不会被修改或删除",
            "同时删除该书的章节索引与分页缓存",
            "移出书架",
            true,
        );
        assert_eq!(state.dialog_kind(), 1);
        assert!(state.dialog().confirm_danger);
        state.close_dialog();
        assert_eq!(state.dialog_kind(), 0);

        state.show_toast("已移出书架", "撤销", "undo", 1);
        assert!(state.project().toast.visible);
        assert!(!state.clear_toast(2), "旧令牌不得清除新提示");
        assert!(state.clear_toast(1));
        assert!(!state.project().toast.visible);
    }
}

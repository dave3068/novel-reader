//! 版式与配色常量的**唯一来源**（single source of truth）。
//!
//! 依据 `docs/design/novel-reader-desktop-redesign.md`（IBM 设计语言重设计，v0.2.0）：
//!
//! - §2.1 色彩：主色阶（Primary = `#1890ff`）、Carbon 中性阶与语义令牌（明 / 暗）；
//! - §2.2 字号与行高：caption-01 / label-01 / body-compact-01 / body-01 / reading-01 /
//!   heading-03 / heading-04 / mono-01；
//! - §2.3 间距阶（2x 栅格）：2 / 4 / 8 / 12 / 16 / 24 / 32 / 48；
//! - §2.4 尺寸与形态：顶栏 48、工具条 48、状态栏 32、进度条 2（hover 4）、按钮 32/40、
//!   图标按钮 32（行内 28）、Tag 20、列表行 40、表格行 48 等；
//! - §2.5 栅格与断点：基线窗口 1200×800，最小 560×400；≥1440 正文留白 64，
//!   960–1199 书架隐藏「最近阅读」列，720–959 简化，560–719 紧凑（留白 24、工具条 40）；
//! - §5 O1 裁定（方案 A）：正文列最大宽 = 40 × 字号并居中。
//!
//! 单元约定：长度为像素（`u32`）。Slint 侧（`desktop/ui/app.slint`）只有 `UiMetrics` /
//! `Palette` 全局的属性声明（默认 0 / 透明），启动时由 [`crate::ui::apply_layout_metrics`] /
//! [`crate::ui::apply_palette`] 注入下面的数值；任何 `.slint` 文件中都不得写死尺寸
//! （由 `scripts/verify-desktop-scope.py` 与 `tests/layout_constants.rs` 双重强制）。
//!
//! 本模块不做任何文本测量。

// ---------------------------------------------------------------------------
// §2.4 窗口与条带
// ---------------------------------------------------------------------------

/// 顶栏 UI Shell 高度（像素；深色）。
pub const TOP_BAR_HEIGHT: u32 = 48;
/// 阅读工具条高度（像素；常驻，可收起）。
pub const TOOLBAR_HEIGHT: u32 = 48;
/// 紧凑模式（窗口宽 < [`BREAKPOINT_NARROW`]）下的工具条高度。
pub const TOOLBAR_HEIGHT_COMPACT: u32 = 40;
/// 工具条收起态的行高（只承载右上角「展开工具条」按钮；不压正文）。
pub const COLLAPSED_TOOLBAR_HEIGHT: u32 = 40;
/// 沉浸态顶部浮出工具条高度（设计 §5 S9：40；比常驻工具条矮）。
pub const FLOATING_TOOLBAR_HEIGHT: u32 = 40;
/// 底部状态栏高度（像素）。
pub const STATUS_BAR_HEIGHT: u32 = 32;
/// 阅读进度条高度（像素；跨整宽，可点击/拖拽跳转）。
pub const PROGRESS_BAR_HEIGHT: u32 = 2;
/// 阅读进度条 hover 高度（像素）。
pub const PROGRESS_BAR_HOVER_HEIGHT: u32 = 4;
/// 正文区水平内边距（像素，单边；基线档）。
pub const CONTENT_PADDING_H: u32 = 48;
/// 正文区垂直内边距（像素，单边；基线档）。
pub const CONTENT_PADDING_V: u32 = 32;
/// 正文区水平内边距（宽屏档，窗口宽 ≥ [`BREAKPOINT_WIDE`]）。
pub const CONTENT_PADDING_H_WIDE: u32 = 64;
/// 正文区留白（紧凑档，窗口宽 < [`BREAKPOINT_NARROW`]）。
pub const CONTENT_PADDING_COMPACT: u32 = 24;
/// 宽屏断点（像素）。
pub const BREAKPOINT_WIDE: u32 = 1440;
/// 紧凑断点（像素；书架隐藏次要列 / 卡片两列）。
pub const BREAKPOINT_COMPACT: u32 = 960;
/// 紧凑模式上限（设计 §2.5 的 560–719 档：书架切卡片视图，避免固定列把书名挤窄换行）。
pub const BREAKPOINT_TIGHT: u32 = 720;
/// 窄屏断点（像素；书架卡片单列 / 紧凑留白）。
pub const BREAKPOINT_NARROW: u32 = 720;
/// 默认窗口宽度（像素）。
pub const DEFAULT_WINDOW_WIDTH: u32 = 1200;
/// 默认窗口高度（像素）。
pub const DEFAULT_WINDOW_HEIGHT: u32 = 800;
/// 窗口最小宽度（低于此不保证可用）。
pub const MIN_WINDOW_WIDTH: u32 = 560;
/// 窗口最小高度。
pub const MIN_WINDOW_HEIGHT: u32 = 400;
/// 正文列最大宽（单位：字号倍数；O1 方案 A：40 字/行）。
pub const LINE_WIDTH_EM: u32 = 40;

// ---------------------------------------------------------------------------
// §2.2 字号与行高
// ---------------------------------------------------------------------------

/// heading-04：页面大标题（像素）。
pub const TITLE_FONT_SIZE: u32 = 24;
/// heading-03：弹窗标题（像素）。
pub const HEADING_FONT_SIZE: u32 = 20;
/// reading-01：正文（像素）。
pub const BODY_FONT_SIZE: u32 = 18;
/// body-compact-01：按钮 / 输入 / 导航 / 表单标签（像素）。
pub const UI_FONT_SIZE: u32 = 14;
/// label-01：表格行 / 面板行 / 紧凑按钮（像素）。
pub const LABEL_FONT_SIZE: u32 = 13;
/// caption-01：状态栏 / 辅助说明（像素）。
pub const CAPTION_FONT_SIZE: u32 = 12;
/// mono-01：页码 / 偏移 / 版本 / 编码等机器可核对信息（像素）。
pub const MONO_FONT_SIZE: u32 = 12;
/// Tag 文字（像素）。
pub const TAG_FONT_SIZE: u32 = 11;

// ---------------------------------------------------------------------------
// 导航（UI Shell 顶栏）
// ---------------------------------------------------------------------------

/// 导航项最小宽度（文字自适应；不再使用固定 104 蓝块）。
pub const NAV_TAB_MIN_WIDTH: u32 = 96;
/// 导航项高度（像素；与顶栏同高）。
pub const NAV_TAB_HEIGHT: u32 = 48;
/// 导航选中指示条高度（像素）。
pub const NAV_INDICATOR_HEIGHT: u32 = 2;

// ---------------------------------------------------------------------------
// §2.4 控件
// ---------------------------------------------------------------------------

/// 图标按钮尺寸（像素；必须带 tooltip 与可访问名称）。
pub const ICON_BUTTON_SIZE: u32 = 32;
/// 行内图标按钮尺寸（像素）。
pub const ICON_BUTTON_COMPACT_SIZE: u32 = 28;
/// 默认按钮高度（像素）。
pub const BUTTON_HEIGHT: u32 = 40;
/// 紧凑按钮高度（像素）。
pub const BUTTON_HEIGHT_COMPACT: u32 = 32;
/// 聚焦环宽度（像素；2px 主色 + 1px 底色隔离）。
pub const FOCUS_RING_WIDTH: u32 = 2;
/// 书架搜索框宽度（像素）。
pub const SEARCH_FIELD_WIDTH: u32 = 240;
/// 书架排序选择宽度（像素）。
pub const SORT_FIELD_WIDTH: u32 = 150;
/// Tag 高度（像素）。
pub const TAG_HEIGHT: u32 = 20;
/// 开关宽度（像素）。
pub const SWITCH_WIDTH: u32 = 32;
/// 开关高度（像素）。
pub const SWITCH_HEIGHT: u32 = 18;
/// 单选圆点尺寸（像素）。
pub const RADIO_SIZE: u32 = 16;
/// 滑块把手尺寸（像素）。
pub const SLIDER_HANDLE_SIZE: u32 = 16;
/// 滑块轨道高度（像素）。
pub const SLIDER_TRACK_HEIGHT: u32 = 2;

// ---------------------------------------------------------------------------
// §2.4 列表 / 表格 / 页头
// ---------------------------------------------------------------------------

/// 面板行高（像素；目录 / 书签 / 选项行）。
pub const LIST_ROW_HEIGHT: u32 = 40;
/// 表格行高（像素）。
pub const TABLE_ROW_HEIGHT: u32 = 48;
/// 表头高度（像素）。
pub const TABLE_HEADER_HEIGHT: u32 = 40;
/// 书架「状态」列宽（像素）。
pub const COLUMN_STATUS_WIDTH: u32 = 120;
/// 书架「进度」列宽（像素）。
pub const COLUMN_PROGRESS_WIDTH: u32 = 180;
/// 书架「最近阅读」列宽（像素）。
pub const COLUMN_RECENT_WIDTH: u32 = 110;
/// 书架「操作」列宽（像素）。
pub const COLUMN_ACTIONS_WIDTH: u32 = 76;
/// 页头高度（像素；标题 + 计数 Tag + 工具区）。
pub const PAGE_HEADER_HEIGHT: u32 = 80;
/// 面板头高度（像素）。
pub const PANEL_HEADER_HEIGHT: u32 = 48;
/// 面板底部提示条高度（像素）。
pub const PANEL_HINT_HEIGHT: u32 = 32;

// ---------------------------------------------------------------------------
// 滚动条
// ---------------------------------------------------------------------------

/// 滚动条宽度（像素）。
pub const SCROLLBAR_WIDTH: u32 = 8;
/// 滚动条与容器右缘的间距（像素）。
pub const SCROLLBAR_GUTTER: u32 = 12;
/// 滚动条滑块最小长度（像素；列表很长时仍可抓）。
pub const SCROLLBAR_MIN_HANDLE: u32 = 32;

// ---------------------------------------------------------------------------
// 覆盖层（Toast / 弹窗 / 卡片 / 空态）
// ---------------------------------------------------------------------------

/// Toast 圆角（像素；Carbon 直角之外的唯一例外）。
pub const TOAST_RADIUS: u32 = 4;
/// Toast 宽度（像素）。
pub const TOAST_WIDTH: u32 = 320;
/// Toast 高度（像素）。
pub const TOAST_HEIGHT: u32 = 64;
/// 确认型弹窗宽度（像素）。
pub const MODAL_WIDTH_CONFIRM: u32 = 520;
/// 设置型弹窗宽度（像素）。
pub const MODAL_WIDTH_SETTINGS: u32 = 560;
/// 书架卡片宽度（像素）。
pub const CARD_WIDTH: u32 = 320;
/// 书架卡片高度（像素）。
pub const CARD_HEIGHT: u32 = 288;
/// 卡片封面占位尺寸（像素；本地派生色块 + 首字）。
pub const COVER_SIZE: u32 = 128;
/// 空态图标尺寸（像素）。
pub const EMPTY_ICON_SIZE: u32 = 64;

// ---------------------------------------------------------------------------
// §2.3 间距阶（2x 栅格）
// ---------------------------------------------------------------------------

/// 间距：图标与文字。
pub const SPACE_2: u32 = 2;
/// 间距：图标按钮之间。
pub const SPACE_4: u32 = 4;
/// 间距：行内元素。
pub const SPACE_8: u32 = 8;
/// 间距：控件与标签。
pub const SPACE_12: u32 = 12;
/// 间距：区块内 / 页面边距。
pub const SPACE_16: u32 = 16;
/// 间距：区块之间。
pub const SPACE_24: u32 = 24;
/// 间距：分组之间。
pub const SPACE_32: u32 = 32;
/// 间距：栏高 / 正文留白。
pub const SPACE_48: u32 = 48;

// ---------------------------------------------------------------------------
// 线宽 / 图标 / 卡片文字（设计 §2.2 / §3）
// ---------------------------------------------------------------------------

/// 发丝线宽度（像素；1px 分隔线 / 输入底边 / 边框）。
pub const HAIRLINE: u32 = 1;
/// 品牌文字字号（像素；顶栏 UI Shell 品牌）。
pub const BRAND_FONT_SIZE: u32 = 15;
/// 图标尺寸（像素；自绘几何线框）。
pub const ICON_SIZE: u32 = 16;
/// 卡片封面首字字号（像素）。
pub const COVER_FONT_SIZE: u32 = 32;
/// 卡片书名字号（像素）。
pub const CARD_TITLE_FONT_SIZE: u32 = 16;
/// 面板内搜索框宽度（像素；目录 / 书签面板头）。
pub const SEARCH_FIELD_PANEL_WIDTH: u32 = 260;

// ---------------------------------------------------------------------------
// 正文字号边界（设置页与夹紧口径）
// ---------------------------------------------------------------------------

/// 正文字号下限（像素；设置页边界，不作为 Slint 属性投影）。
pub const MIN_FONT_SIZE_PX: u32 = 12;
/// 正文字号上限（像素；设置页边界）。
pub const MAX_FONT_SIZE_PX: u32 = 40;

/// 版式参数集合：Rust 排版与 Slint 渲染共用的同一份数值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutMetrics {
    /// 顶栏高度。
    pub top_bar_height: u32,
    /// 阅读工具条高度。
    pub toolbar_height: u32,
    /// 紧凑模式工具条高度。
    pub toolbar_height_compact: u32,
    /// 工具条收起态行高（只承载右上角「展开工具条」按钮）。
    pub collapsed_toolbar_height: u32,
    /// 沉浸态顶部浮出工具条高度（设计 §5 S9）。
    pub floating_toolbar_height: u32,
    /// 底部状态栏高度。
    pub status_bar_height: u32,
    /// 阅读进度条高度。
    pub progress_bar_height: u32,
    /// 阅读进度条 hover 高度。
    pub progress_bar_hover_height: u32,
    /// 正文区水平内边距（单边）。
    pub content_padding_h: u32,
    /// 正文区垂直内边距（单边）。
    pub content_padding_v: u32,
    /// 宽屏正文留白。
    pub content_padding_h_wide: u32,
    /// 紧凑正文留白。
    pub content_padding_compact: u32,
    /// 宽屏断点。
    pub breakpoint_wide: u32,
    /// 紧凑断点。
    pub breakpoint_compact: u32,
    /// 紧凑模式上限断点（书架切卡片视图）。
    pub breakpoint_tight: u32,
    /// 窄屏断点。
    pub breakpoint_narrow: u32,
    /// 默认窗口宽度。
    pub default_window_width: u32,
    /// 默认窗口高度。
    pub default_window_height: u32,
    /// 最小窗口宽度。
    pub min_window_width: u32,
    /// 最小窗口高度。
    pub min_window_height: u32,
    /// 页面大标题字号。
    pub title_font_size: u32,
    /// 弹窗标题字号。
    pub heading_font_size: u32,
    /// 正文字号。
    pub body_font_size: u32,
    /// 界面文字号（按钮 / 输入 / 导航）。
    pub ui_font_size: u32,
    /// 表格 / 面板行字号。
    pub label_font_size: u32,
    /// 说明 / 状态栏字号。
    pub caption_font_size: u32,
    /// 机器可核对信息字号（mono）。
    pub mono_font_size: u32,
    /// Tag 字号。
    pub tag_font_size: u32,
    /// 导航项最小宽度。
    pub nav_tab_min_width: u32,
    /// 导航项高度。
    pub nav_tab_height: u32,
    /// 导航选中指示条高度。
    pub nav_indicator_height: u32,
    /// 图标按钮尺寸。
    pub icon_button_size: u32,
    /// 行内图标按钮尺寸。
    pub icon_button_compact_size: u32,
    /// 默认按钮高度。
    pub button_height: u32,
    /// 紧凑按钮高度。
    pub button_height_compact: u32,
    /// 聚焦环宽度。
    pub focus_ring_width: u32,
    /// 搜索框宽度。
    pub search_field_width: u32,
    /// 排序选择宽度。
    pub sort_field_width: u32,
    /// Tag 高度。
    pub tag_height: u32,
    /// 开关宽度。
    pub switch_width: u32,
    /// 开关高度。
    pub switch_height: u32,
    /// 单选圆点尺寸。
    pub radio_size: u32,
    /// 滑块把手尺寸。
    pub slider_handle_size: u32,
    /// 滑块轨道高度。
    pub slider_track_height: u32,
    /// 面板行高。
    pub list_row_height: u32,
    /// 表格行高。
    pub table_row_height: u32,
    /// 表头高度。
    pub table_header_height: u32,
    /// 书架「状态」列宽。
    pub column_status_width: u32,
    /// 书架「进度」列宽。
    pub column_progress_width: u32,
    /// 书架「最近阅读」列宽。
    pub column_recent_width: u32,
    /// 书架「操作」列宽。
    pub column_actions_width: u32,
    /// 页头高度。
    pub page_header_height: u32,
    /// 面板头高度。
    pub panel_header_height: u32,
    /// 面板底部提示条高度。
    pub panel_hint_height: u32,
    /// 滚动条宽度。
    pub scrollbar_width: u32,
    /// 滚动条与容器右缘的间距。
    pub scrollbar_gutter: u32,
    /// 滚动条滑块最小长度。
    pub scrollbar_min_handle: u32,
    /// Toast 圆角。
    pub toast_radius: u32,
    /// Toast 宽度。
    pub toast_width: u32,
    /// Toast 高度。
    pub toast_height: u32,
    /// 确认型弹窗宽度。
    pub modal_width_confirm: u32,
    /// 设置型弹窗宽度。
    pub modal_width_settings: u32,
    /// 卡片宽度。
    pub card_width: u32,
    /// 卡片高度。
    pub card_height: u32,
    /// 封面占位尺寸。
    pub cover_size: u32,
    /// 空态图标尺寸。
    pub empty_icon_size: u32,
    /// 间距 2。
    pub space_2: u32,
    /// 间距 4。
    pub space_4: u32,
    /// 间距 8。
    pub space_8: u32,
    /// 间距 12。
    pub space_12: u32,
    /// 间距 16。
    pub space_16: u32,
    /// 间距 24。
    pub space_24: u32,
    /// 间距 32。
    pub space_32: u32,
    /// 间距 48。
    pub space_48: u32,
    /// 发丝线宽度。
    pub hairline: u32,
    /// 品牌文字字号。
    pub brand_font_size: u32,
    /// 图标尺寸。
    pub icon_size: u32,
    /// 卡片封面首字字号。
    pub cover_font_size: u32,
    /// 卡片书名字号。
    pub card_title_font_size: u32,
    /// 面板内搜索框宽度。
    pub search_field_panel_width: u32,
}

/// 正文区域（窗口绝对坐标，像素）。
///
/// 语义与 `desktop/ui/app.slint` 的阅读页一致：
/// `x` = 水平留白，`y` = 顶栏 + 工具条 + 垂直留白，
/// `width` / `height` = 扣除留白后的正文区尺寸（不含进度条）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentArea {
    /// 左边界（相对窗口）。
    pub x: u32,
    /// 上边界（相对窗口）。
    pub y: u32,
    /// 宽度。
    pub width: u32,
    /// 高度。
    pub height: u32,
}

/// 正文列（在 [`ContentArea`] 内居中的限宽列；O1 方案 A）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextColumn {
    /// 左边界（相对窗口）。
    pub x: u32,
    /// 上边界（相对窗口）。
    pub y: u32,
    /// 列宽（= min(正文区宽度, 40 × 字号)）。
    pub width: u32,
    /// 列高（= 正文区高度）。
    pub height: u32,
}

impl LayoutMetrics {
    /// 桌面端当前基线数值（唯一实例）。
    pub const DESKTOP: Self = Self {
        top_bar_height: TOP_BAR_HEIGHT,
        toolbar_height: TOOLBAR_HEIGHT,
        toolbar_height_compact: TOOLBAR_HEIGHT_COMPACT,
        collapsed_toolbar_height: COLLAPSED_TOOLBAR_HEIGHT,
        floating_toolbar_height: FLOATING_TOOLBAR_HEIGHT,
        status_bar_height: STATUS_BAR_HEIGHT,
        progress_bar_height: PROGRESS_BAR_HEIGHT,
        progress_bar_hover_height: PROGRESS_BAR_HOVER_HEIGHT,
        content_padding_h: CONTENT_PADDING_H,
        content_padding_v: CONTENT_PADDING_V,
        content_padding_h_wide: CONTENT_PADDING_H_WIDE,
        content_padding_compact: CONTENT_PADDING_COMPACT,
        breakpoint_wide: BREAKPOINT_WIDE,
        breakpoint_compact: BREAKPOINT_COMPACT,
        breakpoint_tight: BREAKPOINT_TIGHT,
        breakpoint_narrow: BREAKPOINT_NARROW,
        default_window_width: DEFAULT_WINDOW_WIDTH,
        default_window_height: DEFAULT_WINDOW_HEIGHT,
        min_window_width: MIN_WINDOW_WIDTH,
        min_window_height: MIN_WINDOW_HEIGHT,
        title_font_size: TITLE_FONT_SIZE,
        heading_font_size: HEADING_FONT_SIZE,
        body_font_size: BODY_FONT_SIZE,
        ui_font_size: UI_FONT_SIZE,
        label_font_size: LABEL_FONT_SIZE,
        caption_font_size: CAPTION_FONT_SIZE,
        mono_font_size: MONO_FONT_SIZE,
        tag_font_size: TAG_FONT_SIZE,
        nav_tab_min_width: NAV_TAB_MIN_WIDTH,
        nav_tab_height: NAV_TAB_HEIGHT,
        nav_indicator_height: NAV_INDICATOR_HEIGHT,
        icon_button_size: ICON_BUTTON_SIZE,
        icon_button_compact_size: ICON_BUTTON_COMPACT_SIZE,
        button_height: BUTTON_HEIGHT,
        button_height_compact: BUTTON_HEIGHT_COMPACT,
        focus_ring_width: FOCUS_RING_WIDTH,
        search_field_width: SEARCH_FIELD_WIDTH,
        sort_field_width: SORT_FIELD_WIDTH,
        tag_height: TAG_HEIGHT,
        switch_width: SWITCH_WIDTH,
        switch_height: SWITCH_HEIGHT,
        radio_size: RADIO_SIZE,
        slider_handle_size: SLIDER_HANDLE_SIZE,
        slider_track_height: SLIDER_TRACK_HEIGHT,
        list_row_height: LIST_ROW_HEIGHT,
        table_row_height: TABLE_ROW_HEIGHT,
        table_header_height: TABLE_HEADER_HEIGHT,
        column_status_width: COLUMN_STATUS_WIDTH,
        column_progress_width: COLUMN_PROGRESS_WIDTH,
        column_recent_width: COLUMN_RECENT_WIDTH,
        column_actions_width: COLUMN_ACTIONS_WIDTH,
        page_header_height: PAGE_HEADER_HEIGHT,
        panel_header_height: PANEL_HEADER_HEIGHT,
        panel_hint_height: PANEL_HINT_HEIGHT,
        scrollbar_width: SCROLLBAR_WIDTH,
        scrollbar_gutter: SCROLLBAR_GUTTER,
        scrollbar_min_handle: SCROLLBAR_MIN_HANDLE,
        toast_radius: TOAST_RADIUS,
        toast_width: TOAST_WIDTH,
        toast_height: TOAST_HEIGHT,
        modal_width_confirm: MODAL_WIDTH_CONFIRM,
        modal_width_settings: MODAL_WIDTH_SETTINGS,
        card_width: CARD_WIDTH,
        card_height: CARD_HEIGHT,
        cover_size: COVER_SIZE,
        empty_icon_size: EMPTY_ICON_SIZE,
        space_2: SPACE_2,
        space_4: SPACE_4,
        space_8: SPACE_8,
        space_12: SPACE_12,
        space_16: SPACE_16,
        space_24: SPACE_24,
        space_32: SPACE_32,
        space_48: SPACE_48,
        hairline: HAIRLINE,
        brand_font_size: BRAND_FONT_SIZE,
        icon_size: ICON_SIZE,
        cover_font_size: COVER_FONT_SIZE,
        card_title_font_size: CARD_TITLE_FONT_SIZE,
        search_field_panel_width: SEARCH_FIELD_PANEL_WIDTH,
    };

    /// 正文水平留白（按窗口宽度选档：宽屏 64 / 基线 48 / 紧凑 24）。
    pub const fn content_padding_h_for(&self, window_width: u32) -> u32 {
        if window_width >= self.breakpoint_wide {
            self.content_padding_h_wide
        } else if window_width < self.breakpoint_narrow {
            self.content_padding_compact
        } else {
            self.content_padding_h
        }
    }

    /// 正文垂直留白（与水平留白同档）。
    pub const fn content_padding_v_for(&self, window_width: u32) -> u32 {
        if window_width >= self.breakpoint_wide {
            self.content_padding_h_wide
        } else if window_width < self.breakpoint_narrow {
            self.content_padding_compact
        } else {
            self.content_padding_v
        }
    }

    /// 工具条高度（紧凑档 40，其余 48）。
    pub const fn toolbar_height_for(&self, window_width: u32) -> u32 {
        if window_width < self.breakpoint_narrow {
            self.toolbar_height_compact
        } else {
            self.toolbar_height
        }
    }

    /// 正文区域（窗口绝对坐标）：窗口尺寸减去顶栏、工具条、状态栏、进度条与留白。
    ///
    /// 采用饱和减法：窗口小于装饰尺寸时返回 0 面积，不产生负数（Slint 侧同理，
    /// 不会出现负 `width` / `height`）。
    pub fn content_area(
        &self,
        window_width: u32,
        window_height: u32,
        toolbar_open: bool,
    ) -> ContentArea {
        let pad_h = self.content_padding_h_for(window_width);
        let pad_v = self.content_padding_v_for(window_width);
        let toolbar = if toolbar_open {
            self.toolbar_height_for(window_width)
        } else {
            0
        };
        ContentArea {
            x: pad_h,
            y: self
                .top_bar_height
                .saturating_add(toolbar)
                .saturating_add(pad_v),
            width: window_width.saturating_sub(pad_h.saturating_mul(2)),
            height: window_height
                .saturating_sub(self.top_bar_height)
                .saturating_sub(toolbar)
                .saturating_sub(self.status_bar_height)
                .saturating_sub(self.progress_bar_height)
                .saturating_sub(pad_v.saturating_mul(2)),
        }
    }

    /// 正文列（在正文区内居中的限宽列；O1 方案 A：最大 40 字号宽）。
    ///
    /// 测量（断行宽度）与渲染（逐行定位）共用同一结果：宽度不足时等于正文区宽度，
    /// 否则居中并两侧留白相等。
    pub fn text_column(&self, area: &ContentArea, font_size_px: u32) -> TextColumn {
        let max_width = LINE_WIDTH_EM.saturating_mul(font_size_px);
        let width = area.width.min(max_width);
        TextColumn {
            x: area.x.saturating_add(area.width.saturating_sub(width) / 2),
            y: area.y,
            width,
            height: area.height,
        }
    }

    /// 沉浸阅读的版式派生：顶栏 / 工具条 / 状态栏与进度条不再占位
    /// （正文区 = 整个窗口，保留 48 留白；见设计 §5 S9）。**只派生取值、不新增常量**，
    /// 因此 `UiMetrics` 的字段集合与 Slint 侧仍保持单一来源。
    pub const fn immersive(self) -> Self {
        Self {
            top_bar_height: 0,
            toolbar_height: 0,
            status_bar_height: 0,
            progress_bar_height: 0,
            ..self
        }
    }
}

// ---------------------------------------------------------------------------
// 配色令牌（§2.1）：明 / 暗两套；运行时由 `apply_palette` 注入当前主题
// ---------------------------------------------------------------------------

/// 一个颜色令牌的取值（明 / 暗）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorPair {
    /// 明色（`#RRGGBBAA` 口径的分量）。
    pub light: (u8, u8, u8, u8),
    /// 暗色。
    pub dark: (u8, u8, u8, u8),
}

/// 颜色令牌元数据（Slint `Palette` 属性名 + 明暗取值）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteField {
    /// Slint 侧属性名（kebab-case）。
    pub slint_name: &'static str,
    /// 明暗取值。
    pub value: ColorPair,
}

/// 语义颜色令牌表（顺序与 `desktop/ui/app.slint` 的 `Palette` 声明一致）。
///
/// 取值口径来自设计 §2.1：主色阶 p10–p100、Carbon 中性阶与语义令牌（明 / 暗）。
pub const PALETTE_FIELDS: &[PaletteField] = &[
    f("background", (0xff, 0xff, 0xff, 0xff), (0x16, 0x16, 0x16, 0xff)),
    f("layer-01", (0xf4, 0xf4, 0xf4, 0xff), (0x26, 0x26, 0x26, 0xff)),
    f("layer-02", (0xff, 0xff, 0xff, 0xff), (0x39, 0x39, 0x39, 0xff)),
    f("field-01", (0xff, 0xff, 0xff, 0xff), (0x26, 0x26, 0x26, 0xff)),
    f("border-subtle", (0xe0, 0xe0, 0xe0, 0xff), (0x39, 0x39, 0x39, 0xff)),
    f("border-strong", (0x8d, 0x8d, 0x8d, 0xff), (0x6f, 0x6f, 0x6f, 0xff)),
    f("text-primary", (0x16, 0x16, 0x16, 0xff), (0xf4, 0xf4, 0xf4, 0xff)),
    f("text-secondary", (0x52, 0x52, 0x52, 0xff), (0xc6, 0xc6, 0xc6, 0xff)),
    f("text-placeholder", (0x8d, 0x8d, 0x8d, 0xff), (0x6f, 0x6f, 0x6f, 0xff)),
    f("text-disabled", (0xc6, 0xc6, 0xc6, 0xff), (0x6f, 0x6f, 0x6f, 0xff)),
    // 主色：p60 明色主色；暗色提亮为 p50（设计 §2.1 对比度规则）。
    f("primary", (0x18, 0x90, 0xff, 0xff), (0x40, 0x96, 0xff, 0xff)),
    f("primary-hover", (0x09, 0x58, 0xd9, 0xff), (0x69, 0xb1, 0xff, 0xff)),
    f("primary-pressed", (0x00, 0x3e, 0xb3, 0xff), (0x91, 0xca, 0xff, 0xff)),
    // p10 选中底 / 通知底；暗色取深蓝底（保持「层色 + 边框」层级）。
    f("primary-soft", (0xe6, 0xf4, 0xff, 0xff), (0x11, 0x2a, 0x45, 0xff)),
    // p20 Tag 底 / 描边强调。
    f("primary-border", (0xba, 0xe0, 0xff, 0xff), (0x15, 0x3a, 0x5e, 0xff)),
    // p70 正文链接 / 次级按钮文字（<18px 正文本必须用它，对比度 AA）。
    f("primary-strong", (0x09, 0x58, 0xd9, 0xff), (0x69, 0xb1, 0xff, 0xff)),
    f("on-primary", (0xff, 0xff, 0xff, 0xff), (0xff, 0xff, 0xff, 0xff)),
    f("focus", (0x18, 0x90, 0xff, 0xff), (0x40, 0x96, 0xff, 0xff)),
    // 顶栏 UI Shell：两主题同色（深底白字）。
    f("header", (0x16, 0x16, 0x16, 0xff), (0x16, 0x16, 0x16, 0xff)),
    f("header-text", (0xff, 0xff, 0xff, 0xff), (0xff, 0xff, 0xff, 0xff)),
    // 深色顶栏上的图标按钮悬停底色（若沿用浅色 hover，白色图标会看不见）。
    f("header-hover", (0x2e, 0x2e, 0x2e, 0xff), (0x3d, 0x3d, 0x3d, 0xff)),
    f("support-success", (0x19, 0x80, 0x38, 0xff), (0x42, 0xbe, 0x65, 0xff)),
    f("success-soft", (0xde, 0xfb, 0xe6, 0xff), (0x0f, 0x3a, 0x24, 0xff)),
    f("support-warning", (0x8a, 0x61, 0x00, 0xff), (0xf1, 0xc2, 0x1b, 0xff)),
    f("warning-soft", (0xff, 0xf8, 0xe1, 0xff), (0x3a, 0x2c, 0x00, 0xff)),
    f("support-error", (0xda, 0x1e, 0x28, 0xff), (0xff, 0x83, 0x89, 0xff)),
    f("error-soft", (0xff, 0xf1, 0xf0, 0xff), (0x3a, 0x12, 0x14, 0xff)),
    f("scrollbar-track", (0xf4, 0xf4, 0xf4, 0xff), (0x26, 0x26, 0x26, 0xff)),
    f("scrollbar-handle", (0xc6, 0xc6, 0xc6, 0xff), (0x6f, 0x6f, 0x6f, 0xff)),
    // 中性 g40 / g60（暗色），供滑块与次要描边使用。
    f("control-track", (0xa8, 0xa8, 0xa8, 0xff), (0x6f, 0x6f, 0x6f, 0xff)),
    // 禁用态：g10 底 / g20 边 / g30 字（§S16 状态矩阵）。
    f("disabled-bg", (0xf4, 0xf4, 0xf4, 0xff), (0x26, 0x26, 0x26, 0xff)),
    f("disabled-border", (0xe0, 0xe0, 0xe0, 0xff), (0x39, 0x39, 0x39, 0xff)),
    f("disabled-text", (0xc6, 0xc6, 0xc6, 0xff), (0x6f, 0x6f, 0x6f, 0xff)),
    // 弹窗遮罩 `rgba(22,22,22,.5)`。
    f("overlay", (0x16, 0x16, 0x16, 0x80), (0x16, 0x16, 0x16, 0x80)),
    // Toast：深底白字（两主题同色）。
    f("toast-bg", (0x26, 0x26, 0x26, 0xff), (0x26, 0x26, 0x26, 0xff)),
    f("toast-text", (0xff, 0xff, 0xff, 0xff), (0xff, 0xff, 0xff, 0xff)),
    // 暗色阅读正文（≈13:1）。
    f("reading-text", (0x22, 0x22, 0x22, 0xff), (0xe0, 0xe0, 0xe0, 0xff)),
    // 封面占位底（本地派生色块，不引入网络素材）。
    f("cover-bg", (0xe6, 0xf4, 0xff, 0xff), (0x11, 0x2a, 0x45, 0xff)),
];

/// 便捷构造：颜色令牌。
const fn f(name: &'static str, light: (u8, u8, u8, u8), dark: (u8, u8, u8, u8)) -> PaletteField {
    PaletteField {
        slint_name: name,
        value: ColorPair { light, dark },
    }
}

/// 单个版式字段的元数据：Slint 属性名（kebab-case）、Rust 常量名与数值。
///
/// 这张表是「字段集合」的唯一来源：`layout_constants_single_source` 用它核对
/// `desktop/ui/app.slint` 的 `UiMetrics` 全局声明，防止两侧字段漂移。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetricField {
    /// Slint 侧属性名（`desktop/ui/app.slint` 的 `UiMetrics`）。
    pub slint_name: &'static str,
    /// Rust 侧常量名（本模块 `pub const`）。
    pub rust_const: &'static str,
    /// 数值（像素）。
    pub value: u32,
}

/// 版式字段表（顺序与 `UiMetrics` 声明一致）。
pub const LAYOUT_METRIC_FIELDS: &[MetricField] = &[
    m("top-bar-height", "TOP_BAR_HEIGHT", TOP_BAR_HEIGHT),
    m("toolbar-height", "TOOLBAR_HEIGHT", TOOLBAR_HEIGHT),
    m(
        "toolbar-height-compact",
        "TOOLBAR_HEIGHT_COMPACT",
        TOOLBAR_HEIGHT_COMPACT,
    ),
    m(
        "collapsed-toolbar-height",
        "COLLAPSED_TOOLBAR_HEIGHT",
        COLLAPSED_TOOLBAR_HEIGHT,
    ),
    m(
        "floating-toolbar-height",
        "FLOATING_TOOLBAR_HEIGHT",
        FLOATING_TOOLBAR_HEIGHT,
    ),
    m("status-bar-height", "STATUS_BAR_HEIGHT", STATUS_BAR_HEIGHT),
    m("progress-bar-height", "PROGRESS_BAR_HEIGHT", PROGRESS_BAR_HEIGHT),
    m(
        "progress-bar-hover-height",
        "PROGRESS_BAR_HOVER_HEIGHT",
        PROGRESS_BAR_HOVER_HEIGHT,
    ),
    m("content-padding-h", "CONTENT_PADDING_H", CONTENT_PADDING_H),
    m("content-padding-v", "CONTENT_PADDING_V", CONTENT_PADDING_V),
    m(
        "content-padding-h-wide",
        "CONTENT_PADDING_H_WIDE",
        CONTENT_PADDING_H_WIDE,
    ),
    m(
        "content-padding-compact",
        "CONTENT_PADDING_COMPACT",
        CONTENT_PADDING_COMPACT,
    ),
    m("breakpoint-wide", "BREAKPOINT_WIDE", BREAKPOINT_WIDE),
    m("breakpoint-compact", "BREAKPOINT_COMPACT", BREAKPOINT_COMPACT),
    m("breakpoint-tight", "BREAKPOINT_TIGHT", BREAKPOINT_TIGHT),
    m("breakpoint-narrow", "BREAKPOINT_NARROW", BREAKPOINT_NARROW),
    m(
        "default-window-width",
        "DEFAULT_WINDOW_WIDTH",
        DEFAULT_WINDOW_WIDTH,
    ),
    m(
        "default-window-height",
        "DEFAULT_WINDOW_HEIGHT",
        DEFAULT_WINDOW_HEIGHT,
    ),
    m("min-window-width", "MIN_WINDOW_WIDTH", MIN_WINDOW_WIDTH),
    m("min-window-height", "MIN_WINDOW_HEIGHT", MIN_WINDOW_HEIGHT),
    m("title-font-size", "TITLE_FONT_SIZE", TITLE_FONT_SIZE),
    m("heading-font-size", "HEADING_FONT_SIZE", HEADING_FONT_SIZE),
    m("body-font-size", "BODY_FONT_SIZE", BODY_FONT_SIZE),
    m("ui-font-size", "UI_FONT_SIZE", UI_FONT_SIZE),
    m("label-font-size", "LABEL_FONT_SIZE", LABEL_FONT_SIZE),
    m("caption-font-size", "CAPTION_FONT_SIZE", CAPTION_FONT_SIZE),
    m("mono-font-size", "MONO_FONT_SIZE", MONO_FONT_SIZE),
    m("tag-font-size", "TAG_FONT_SIZE", TAG_FONT_SIZE),
    m("nav-tab-min-width", "NAV_TAB_MIN_WIDTH", NAV_TAB_MIN_WIDTH),
    m("nav-tab-height", "NAV_TAB_HEIGHT", NAV_TAB_HEIGHT),
    m(
        "nav-indicator-height",
        "NAV_INDICATOR_HEIGHT",
        NAV_INDICATOR_HEIGHT,
    ),
    m("icon-button-size", "ICON_BUTTON_SIZE", ICON_BUTTON_SIZE),
    m(
        "icon-button-compact-size",
        "ICON_BUTTON_COMPACT_SIZE",
        ICON_BUTTON_COMPACT_SIZE,
    ),
    m("button-height", "BUTTON_HEIGHT", BUTTON_HEIGHT),
    m(
        "button-height-compact",
        "BUTTON_HEIGHT_COMPACT",
        BUTTON_HEIGHT_COMPACT,
    ),
    m("focus-ring-width", "FOCUS_RING_WIDTH", FOCUS_RING_WIDTH),
    m("search-field-width", "SEARCH_FIELD_WIDTH", SEARCH_FIELD_WIDTH),
    m("sort-field-width", "SORT_FIELD_WIDTH", SORT_FIELD_WIDTH),
    m("tag-height", "TAG_HEIGHT", TAG_HEIGHT),
    m("switch-width", "SWITCH_WIDTH", SWITCH_WIDTH),
    m("switch-height", "SWITCH_HEIGHT", SWITCH_HEIGHT),
    m("radio-size", "RADIO_SIZE", RADIO_SIZE),
    m("slider-handle-size", "SLIDER_HANDLE_SIZE", SLIDER_HANDLE_SIZE),
    m("slider-track-height", "SLIDER_TRACK_HEIGHT", SLIDER_TRACK_HEIGHT),
    m("list-row-height", "LIST_ROW_HEIGHT", LIST_ROW_HEIGHT),
    m("table-row-height", "TABLE_ROW_HEIGHT", TABLE_ROW_HEIGHT),
    m("table-header-height", "TABLE_HEADER_HEIGHT", TABLE_HEADER_HEIGHT),
    m(
        "column-status-width",
        "COLUMN_STATUS_WIDTH",
        COLUMN_STATUS_WIDTH,
    ),
    m(
        "column-progress-width",
        "COLUMN_PROGRESS_WIDTH",
        COLUMN_PROGRESS_WIDTH,
    ),
    m(
        "column-recent-width",
        "COLUMN_RECENT_WIDTH",
        COLUMN_RECENT_WIDTH,
    ),
    m(
        "column-actions-width",
        "COLUMN_ACTIONS_WIDTH",
        COLUMN_ACTIONS_WIDTH,
    ),
    m("page-header-height", "PAGE_HEADER_HEIGHT", PAGE_HEADER_HEIGHT),
    m("panel-header-height", "PANEL_HEADER_HEIGHT", PANEL_HEADER_HEIGHT),
    m("panel-hint-height", "PANEL_HINT_HEIGHT", PANEL_HINT_HEIGHT),
    m("scrollbar-width", "SCROLLBAR_WIDTH", SCROLLBAR_WIDTH),
    m("scrollbar-gutter", "SCROLLBAR_GUTTER", SCROLLBAR_GUTTER),
    m(
        "scrollbar-min-handle",
        "SCROLLBAR_MIN_HANDLE",
        SCROLLBAR_MIN_HANDLE,
    ),
    m("toast-radius", "TOAST_RADIUS", TOAST_RADIUS),
    m("toast-width", "TOAST_WIDTH", TOAST_WIDTH),
    m("toast-height", "TOAST_HEIGHT", TOAST_HEIGHT),
    m(
        "modal-width-confirm",
        "MODAL_WIDTH_CONFIRM",
        MODAL_WIDTH_CONFIRM,
    ),
    m(
        "modal-width-settings",
        "MODAL_WIDTH_SETTINGS",
        MODAL_WIDTH_SETTINGS,
    ),
    m("card-width", "CARD_WIDTH", CARD_WIDTH),
    m("card-height", "CARD_HEIGHT", CARD_HEIGHT),
    m("cover-size", "COVER_SIZE", COVER_SIZE),
    m("empty-icon-size", "EMPTY_ICON_SIZE", EMPTY_ICON_SIZE),
    m("space-2", "SPACE_2", SPACE_2),
    m("space-4", "SPACE_4", SPACE_4),
    m("space-8", "SPACE_8", SPACE_8),
    m("space-12", "SPACE_12", SPACE_12),
    m("space-16", "SPACE_16", SPACE_16),
    m("space-24", "SPACE_24", SPACE_24),
    m("space-32", "SPACE_32", SPACE_32),
    m("space-48", "SPACE_48", SPACE_48),
    m("hairline", "HAIRLINE", HAIRLINE),
    m("brand-font-size", "BRAND_FONT_SIZE", BRAND_FONT_SIZE),
    m("icon-size", "ICON_SIZE", ICON_SIZE),
    m("cover-font-size", "COVER_FONT_SIZE", COVER_FONT_SIZE),
    m(
        "card-title-font-size",
        "CARD_TITLE_FONT_SIZE",
        CARD_TITLE_FONT_SIZE,
    ),
    m(
        "search-field-panel-width",
        "SEARCH_FIELD_PANEL_WIDTH",
        SEARCH_FIELD_PANEL_WIDTH,
    ),
];

/// 便捷构造：版式字段。
const fn m(slint_name: &'static str, rust_const: &'static str, value: u32) -> MetricField {
    MetricField {
        slint_name,
        rust_const,
        value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_metrics_follow_constants() {
        let m = LayoutMetrics::DESKTOP;
        assert_eq!(m.top_bar_height, TOP_BAR_HEIGHT);
        assert_eq!(m.toolbar_height, TOOLBAR_HEIGHT);
        assert_eq!(m.status_bar_height, STATUS_BAR_HEIGHT);
        assert_eq!(m.content_padding_h, CONTENT_PADDING_H);
        assert_eq!(m.content_padding_v, CONTENT_PADDING_V);
        assert_eq!(m.default_window_width, DEFAULT_WINDOW_WIDTH);
        assert_eq!(m.default_window_height, DEFAULT_WINDOW_HEIGHT);
        assert_eq!(m.space_48, SPACE_48);
    }

    #[test]
    fn content_area_matches_window_arithmetic() {
        let m = LayoutMetrics::DESKTOP;
        let open = m.content_area(DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT, true);
        assert_eq!(open.x, CONTENT_PADDING_H);
        assert_eq!(open.y, TOP_BAR_HEIGHT + TOOLBAR_HEIGHT + CONTENT_PADDING_V);
        assert_eq!(open.width, DEFAULT_WINDOW_WIDTH - 2 * CONTENT_PADDING_H);
        assert_eq!(
            open.height,
            DEFAULT_WINDOW_HEIGHT
                - TOP_BAR_HEIGHT
                - TOOLBAR_HEIGHT
                - STATUS_BAR_HEIGHT
                - PROGRESS_BAR_HEIGHT
                - 2 * CONTENT_PADDING_V
        );

        let closed = m.content_area(DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT, false);
        assert_eq!(closed.y, TOP_BAR_HEIGHT + CONTENT_PADDING_V);
        assert_eq!(closed.height, open.height + TOOLBAR_HEIGHT);
    }

    #[test]
    fn content_area_is_saturating_for_tiny_windows() {
        let m = LayoutMetrics::DESKTOP;
        let tiny = m.content_area(10, 10, true);
        assert_eq!(tiny.width, 0);
        assert_eq!(tiny.height, 0);
    }

    #[test]
    fn text_column_is_clamped_and_centered() {
        let m = LayoutMetrics::DESKTOP;
        let area = m.content_area(DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT, true);
        // 18px 字号：40 × 18 = 720 < 1104 ⇒ 居中，两侧各 192。
        let column = m.text_column(&area, 18);
        assert_eq!(column.width, 720);
        assert_eq!(column.x, CONTENT_PADDING_H + 192);
        assert_eq!(column.height, area.height);
        // 字号较小时列宽 = 40 × 字号（仍居中；不放大到正文区宽度）。
        let narrow = m.text_column(&area, 4);
        assert_eq!(narrow.width, 160);
        assert_eq!(narrow.x, area.x + (area.width - narrow.width) / 2);
        assert!(narrow.width < area.width);
    }

    #[test]
    fn immersive_keeps_padding_and_drops_decorations() {
        let m = LayoutMetrics::DESKTOP.immersive();
        assert_eq!(m.top_bar_height, 0);
        assert_eq!(m.toolbar_height, 0);
        assert_eq!(m.status_bar_height, 0);
        assert_eq!(m.progress_bar_height, 0);
        let area = m.content_area(DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT, true);
        assert_eq!(area.y, CONTENT_PADDING_V);
        assert_eq!(area.width, DEFAULT_WINDOW_WIDTH - 2 * CONTENT_PADDING_H);
        assert_eq!(area.height, DEFAULT_WINDOW_HEIGHT - 2 * CONTENT_PADDING_V);
    }

    #[test]
    fn breakpoint_padding_switch() {
        let m = LayoutMetrics::DESKTOP;
        assert_eq!(m.content_padding_h_for(1600), CONTENT_PADDING_H_WIDE);
        assert_eq!(m.content_padding_h_for(1200), CONTENT_PADDING_H);
        assert_eq!(m.content_padding_h_for(600), CONTENT_PADDING_COMPACT);
        assert_eq!(m.toolbar_height_for(600), TOOLBAR_HEIGHT_COMPACT);
        assert_eq!(m.toolbar_height_for(1200), TOOLBAR_HEIGHT);
    }

    #[test]
    fn metric_field_table_is_unique_and_consistent() {
        let mut names: Vec<&str> = LAYOUT_METRIC_FIELDS.iter().map(|f| f.slint_name).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "UiMetrics 字段名必须唯一");
        for field in LAYOUT_METRIC_FIELDS {
            assert!(
                field.value > 0,
                "{} 不得为 0（0 保留给 Slint 默认值）",
                field.slint_name
            );
        }
    }

    #[test]
    fn palette_field_table_is_unique() {
        let mut names: Vec<&str> = PALETTE_FIELDS.iter().map(|f| f.slint_name).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "Palette 字段名必须唯一");
        for field in PALETTE_FIELDS {
            assert_ne!(
                field.value.light.3, 0,
                "{} 明色不得全透明",
                field.slint_name
            );
            assert_ne!(
                field.value.dark.3, 0,
                "{} 暗色不得全透明",
                field.slint_name
            );
        }
    }
}

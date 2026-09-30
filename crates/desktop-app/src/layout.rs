//! 版式常量的**唯一来源**（single source of truth）。
//!
//! 依据 `desktop/design.md` §正文布局 与 DT1 交付物要求：顶栏高度、底部栏高度、
//! 正文内边距、菜单高度等版式参数只能有一个定义处，供 Rust 排版（分页测量）与
//! Slint 渲染共用。Slint 侧（`desktop/ui/app.slint`）只有 `UiMetrics` 全局的属性
//! 声明（默认 0），启动时由 [`crate::ui::apply_layout_metrics`] 注入下面的数值；
//! 任何 `.slint` 文件中都不得写死尺寸（由 `scripts/verify-desktop-scope.py` 与
//! `tests/layout_constants.rs::layout_constants_single_source` 双重强制）。
//!
//! 单位约定：像素（`u32`，与 core 的整数毫单位签名在平台适配层换算；换算见
//! `reading_core::LayoutSignature`）。本模块不做任何文本测量。

/// 顶栏高度（像素）。
pub const TOP_BAR_HEIGHT: u32 = 56;
/// 底部状态栏高度（像素）。
pub const BOTTOM_BAR_HEIGHT: u32 = 44;
/// 阅读页菜单条高度（像素）。
pub const MENU_HEIGHT: u32 = 36;
/// 正文区水平内边距（像素，单边）。
pub const CONTENT_PADDING_H: u32 = 32;
/// 正文区垂直内边距（像素，单边）。
pub const CONTENT_PADDING_V: u32 = 28;
/// 顶栏导航按钮宽度（像素）。
pub const NAV_TAB_WIDTH: u32 = 104;
/// 顶栏导航按钮高度（像素）。
pub const NAV_TAB_HEIGHT: u32 = 34;
/// 默认窗口宽度（像素）。
pub const DEFAULT_WINDOW_WIDTH: u32 = 1080;
/// 默认窗口高度（像素）。
pub const DEFAULT_WINDOW_HEIGHT: u32 = 720;
/// 标题字号（像素）。
pub const TITLE_FONT_SIZE: u32 = 20;
/// 正文字号（像素）。
pub const BODY_FONT_SIZE: u32 = 18;
/// 说明/按钮文字字号（像素）。
pub const CAPTION_FONT_SIZE: u32 = 14;
/// 圆角半径（像素）。
pub const CORNER_RADIUS: u32 = 6;
/// 窗口最小宽度（`preferred-*` 让窗口可缩放；最小尺寸仍来自常量集）。
pub const MIN_WINDOW_WIDTH: u32 = 480;
/// 窗口最小高度。
pub const MIN_WINDOW_HEIGHT: u32 = 360;
/// 目录滚动条宽度。
pub const SCROLLBAR_WIDTH: u32 = 6;
/// 目录滚动条与面板右缘的间距。
pub const SCROLLBAR_GUTTER: u32 = 8;
/// 目录滚动条滑块最小长度（列表很长时仍可抓）。
pub const SCROLLBAR_MIN_HANDLE: u32 = 24;
/// 正文字号下限（像素；设置页边界，不作为 Slint 属性投影）。
pub const MIN_FONT_SIZE_PX: u32 = 12;
/// 正文字号上限（像素；设置页边界）。
pub const MAX_FONT_SIZE_PX: u32 = 40;

/// 版式参数集合：Rust 排版与 Slint 渲染共用的同一份数值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutMetrics {
    /// 顶栏高度。
    pub top_bar_height: u32,
    /// 底部状态栏高度。
    pub bottom_bar_height: u32,
    /// 阅读页菜单条高度。
    pub menu_height: u32,
    /// 正文区水平内边距（单边）。
    pub content_padding_h: u32,
    /// 正文区垂直内边距（单边）。
    pub content_padding_v: u32,
    /// 顶栏导航按钮宽度。
    pub nav_tab_width: u32,
    /// 顶栏导航按钮高度。
    pub nav_tab_height: u32,
    /// 默认窗口宽度。
    pub default_window_width: u32,
    /// 默认窗口高度。
    pub default_window_height: u32,
    /// 标题字号。
    pub title_font_size: u32,
    /// 正文字号。
    pub body_font_size: u32,
    /// 说明文字号。
    pub caption_font_size: u32,
    /// 圆角半径。
    pub corner_radius: u32,
    /// 窗口最小宽度（`preferred-*` 让窗口可缩放，最小尺寸仍由常量集给出）。
    pub min_window_width: u32,
    /// 窗口最小高度。
    pub min_window_height: u32,
    /// 目录滚动条宽度。
    pub scrollbar_width: u32,
    /// 目录滚动条与面板右缘的间距。
    pub scrollbar_gutter: u32,
    /// 目录滚动条滑块最小长度（列表很长时仍可抓）。
    pub scrollbar_min_handle: u32,
}

/// 正文区域（窗口绝对坐标，像素）。
///
/// 语义与 `desktop/ui/app.slint` 中 `ReaderPage` 的正文 `Rectangle` 一致：
/// 页面局部坐标下的 y 会减去顶栏高度，`width` / `height` / `x` 完全相同。
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

impl LayoutMetrics {
    /// 桌面端当前基线数值（唯一实例）。
    pub const DESKTOP: Self = Self {
        top_bar_height: TOP_BAR_HEIGHT,
        bottom_bar_height: BOTTOM_BAR_HEIGHT,
        menu_height: MENU_HEIGHT,
        content_padding_h: CONTENT_PADDING_H,
        content_padding_v: CONTENT_PADDING_V,
        nav_tab_width: NAV_TAB_WIDTH,
        nav_tab_height: NAV_TAB_HEIGHT,
        default_window_width: DEFAULT_WINDOW_WIDTH,
        default_window_height: DEFAULT_WINDOW_HEIGHT,
        title_font_size: TITLE_FONT_SIZE,
        body_font_size: BODY_FONT_SIZE,
        caption_font_size: CAPTION_FONT_SIZE,
        corner_radius: CORNER_RADIUS,
        min_window_width: MIN_WINDOW_WIDTH,
        min_window_height: MIN_WINDOW_HEIGHT,
        scrollbar_width: SCROLLBAR_WIDTH,
        scrollbar_gutter: SCROLLBAR_GUTTER,
        scrollbar_min_handle: SCROLLBAR_MIN_HANDLE,
    };

    /// 正文区域（窗口绝对坐标）：窗口尺寸减去顶栏、底部栏、可选菜单条与内边距。
    ///
    /// 采用饱和减法：窗口小于装饰尺寸时返回 0 面积，不产生负数（Slint 侧同理，
    /// 不会出现负 `width` / `height`）。
    pub fn content_area(
        &self,
        window_width: u32,
        window_height: u32,
        menu_open: bool,
    ) -> ContentArea {
        let menu = if menu_open { self.menu_height } else { 0 };
        ContentArea {
            x: self.content_padding_h,
            y: self
                .top_bar_height
                .saturating_add(menu)
                .saturating_add(self.content_padding_v),
            width: window_width.saturating_sub(self.content_padding_h.saturating_mul(2)),
            height: window_height
                .saturating_sub(self.top_bar_height)
                .saturating_sub(self.bottom_bar_height)
                .saturating_sub(menu)
                .saturating_sub(self.content_padding_v.saturating_mul(2)),
        }
    }

    /// 沉浸阅读的版式派生：顶栏 / 底部栏 / 菜单条高度与正文内边距全部置 0
    /// （正文区 = 整个窗口）。**只派生取值、不新增常量**，因此 `UiMetrics` 的
    /// 字段集合与 Slint 侧仍保持单一来源（见 `layout_constants_single_source`）。
    pub const fn immersive(self) -> Self {
        Self {
            top_bar_height: 0,
            bottom_bar_height: 0,
            menu_height: 0,
            content_padding_h: 0,
            content_padding_v: 0,
            ..self
        }
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
    MetricField {
        slint_name: "top-bar-height",
        rust_const: "TOP_BAR_HEIGHT",
        value: TOP_BAR_HEIGHT,
    },
    MetricField {
        slint_name: "bottom-bar-height",
        rust_const: "BOTTOM_BAR_HEIGHT",
        value: BOTTOM_BAR_HEIGHT,
    },
    MetricField {
        slint_name: "menu-height",
        rust_const: "MENU_HEIGHT",
        value: MENU_HEIGHT,
    },
    MetricField {
        slint_name: "content-padding-h",
        rust_const: "CONTENT_PADDING_H",
        value: CONTENT_PADDING_H,
    },
    MetricField {
        slint_name: "content-padding-v",
        rust_const: "CONTENT_PADDING_V",
        value: CONTENT_PADDING_V,
    },
    MetricField {
        slint_name: "nav-tab-width",
        rust_const: "NAV_TAB_WIDTH",
        value: NAV_TAB_WIDTH,
    },
    MetricField {
        slint_name: "nav-tab-height",
        rust_const: "NAV_TAB_HEIGHT",
        value: NAV_TAB_HEIGHT,
    },
    MetricField {
        slint_name: "default-window-width",
        rust_const: "DEFAULT_WINDOW_WIDTH",
        value: DEFAULT_WINDOW_WIDTH,
    },
    MetricField {
        slint_name: "default-window-height",
        rust_const: "DEFAULT_WINDOW_HEIGHT",
        value: DEFAULT_WINDOW_HEIGHT,
    },
    MetricField {
        slint_name: "title-font-size",
        rust_const: "TITLE_FONT_SIZE",
        value: TITLE_FONT_SIZE,
    },
    MetricField {
        slint_name: "body-font-size",
        rust_const: "BODY_FONT_SIZE",
        value: BODY_FONT_SIZE,
    },
    MetricField {
        slint_name: "caption-font-size",
        rust_const: "CAPTION_FONT_SIZE",
        value: CAPTION_FONT_SIZE,
    },
    MetricField {
        slint_name: "corner-radius",
        rust_const: "CORNER_RADIUS",
        value: CORNER_RADIUS,
    },
    MetricField {
        slint_name: "min-window-width",
        rust_const: "MIN_WINDOW_WIDTH",
        value: MIN_WINDOW_WIDTH,
    },
    MetricField {
        slint_name: "min-window-height",
        rust_const: "MIN_WINDOW_HEIGHT",
        value: MIN_WINDOW_HEIGHT,
    },
    MetricField {
        slint_name: "scrollbar-width",
        rust_const: "SCROLLBAR_WIDTH",
        value: SCROLLBAR_WIDTH,
    },
    MetricField {
        slint_name: "scrollbar-gutter",
        rust_const: "SCROLLBAR_GUTTER",
        value: SCROLLBAR_GUTTER,
    },
    MetricField {
        slint_name: "scrollbar-min-handle",
        rust_const: "SCROLLBAR_MIN_HANDLE",
        value: SCROLLBAR_MIN_HANDLE,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_metrics_follow_constants() {
        let m = LayoutMetrics::DESKTOP;
        assert_eq!(m.top_bar_height, TOP_BAR_HEIGHT);
        assert_eq!(m.bottom_bar_height, BOTTOM_BAR_HEIGHT);
        assert_eq!(m.menu_height, MENU_HEIGHT);
        assert_eq!(m.content_padding_h, CONTENT_PADDING_H);
        assert_eq!(m.content_padding_v, CONTENT_PADDING_V);
        assert_eq!(m.default_window_width, DEFAULT_WINDOW_WIDTH);
        assert_eq!(m.default_window_height, DEFAULT_WINDOW_HEIGHT);
    }

    #[test]
    fn content_area_matches_window_arithmetic() {
        let m = LayoutMetrics::DESKTOP;
        let closed = m.content_area(DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT, false);
        assert_eq!(closed.x, CONTENT_PADDING_H);
        assert_eq!(closed.y, TOP_BAR_HEIGHT + CONTENT_PADDING_V);
        assert_eq!(closed.width, DEFAULT_WINDOW_WIDTH - 2 * CONTENT_PADDING_H);
        assert_eq!(
            closed.height,
            DEFAULT_WINDOW_HEIGHT - TOP_BAR_HEIGHT - BOTTOM_BAR_HEIGHT - 2 * CONTENT_PADDING_V
        );

        let open = m.content_area(DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT, true);
        assert_eq!(open.y, closed.y + MENU_HEIGHT);
        assert_eq!(open.height + MENU_HEIGHT, closed.height);
    }

    #[test]
    fn content_area_is_saturating_for_tiny_windows() {
        let m = LayoutMetrics::DESKTOP;
        let tiny = m.content_area(10, 10, true);
        assert_eq!(tiny.width, 0);
        assert_eq!(tiny.height, 0);
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
}

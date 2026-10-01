//! 排版参数派生与重排入口（DT3 / DR3）。
//!
//! # 职责
//!
//! 把**窗口映射后的真实尺寸**与核心设置表里的排版设置派生成两样东西：
//!
//! 1. [`LayoutSignature`]：冻结核心的分页缓存键（`layout_signature + content_index_version`）；
//!    任一字段变化都会让旧页面边界失效并由核心侧的世代号（`layout_id`）丢弃在途结果。
//! 2. [`Typography`]：测量与 Slint 渲染**共用**的同一份派生参数（字号 / 行盒 / 字距 / 段距 /
//!    首行缩进 / 正文区域尺寸 / 每页行数）。
//!
//! # 同口径契约（`docs/novel-reader-v1-development-plan.md` §8.3）
//!
//! - 正文区域尺寸只能在窗口映射完成后取得：未映射（0×0 或 `< 一行`）时本模块拒绝派生，
//!   调用方不得排版（[`MappedViewport::content_area`] / [`layout_signature`] 返回 `Err`）。
//! - 桌面 Slint `Text` **不叠加段距、也不做首行缩进**，因此测量侧同样置零（§8.3 第 4 条）。
//!   之前的技术验证探针正是因为没有对齐这一点，出现「按行高+段间距排版、界面只按行高渲染 →
//!   每页少排若干行、底部大片留白」的问题。
//! - 行盒高度取 `max(字号 + 行距, 真实字体行盒)`：Slint `Text` 没有 `line-height` 属性，
//!   渲染行距不可能小于字体自身行盒，测量侧若取更小值就会重叠；真实字体行盒由
//!   Slint 公开属性 `Text.font-metrics`（`ascent` / `descent`）投影而来（[`PlatformFontMetrics`]），
//!   未取得时退化为保守常数（[`FALLBACK_LINE_BOX_MILLI_PER_EM`]）。
//! - 边距（`margin_*`）在签名里置零：水平/垂直留白已经由 [`LayoutMetrics::content_area`] 的
//!   正文区域常量表达，重复计入会让测量宽度小于渲染宽度（或反之）。
//! - DPI 变化体现为两处：窗口的逻辑尺寸变化（正文区域随之变化）与平台字体指纹里的
//!   `scale_milli` / 真实字体度量，因此 `layout_id` 必然变化并触发重排。

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

use reading_core::{LayoutSignature, LAYOUT_ALGORITHM_VERSION};
use reading_source::hash_bytes;
use reading_storage::settings::ReaderSettings;

use crate::layout::{ContentArea, LayoutMetrics, MAX_FONT_SIZE_PX, MIN_FONT_SIZE_PX};

/// 未取得真实字体度量时的退化行盒高度（字号千分比：`1500` = 1.5 × 字号）。
///
/// 说明：CJK 字体（Noto Sans CJK SC）的 `ascent + descent` 约为 1.48 em，取 1.5 是保守值——
/// 行盒偏大只会让每页少排一行（设计 §8.3 第 7 条允许），偏小会导致行间重叠与末行裁切。
pub const FALLBACK_LINE_BOX_MILLI_PER_EM: u32 = 1_500;

/// 窗口映射状态：**未映射时不得排版**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MappedViewport {
    /// 窗口逻辑宽度（像素）。
    pub width: u32,
    /// 窗口逻辑高度（像素）。
    pub height: u32,
    /// 是否已经完成窗口映射（Slint 窗口尺寸只在事件循环启动后有效）。
    pub mapped: bool,
    /// 设备像素比例 ×1000（DPI；`1000` = 1.0）。
    pub scale_milli: u32,
}

impl MappedViewport {
    /// 尚未映射的视口（事件循环启动前的 0×0；**不得**据此排版）。
    pub const UNMAPPED: Self = Self {
        width: 0,
        height: 0,
        mapped: false,
        scale_milli: 1_000,
    };

    /// 由窗口尺寸构造（宽高都为正才视为已映射）。
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            mapped: width > 0 && height > 0,
            scale_milli: 1_000,
        }
    }

    /// 附带 DPI 比例（×1000）。
    pub fn with_scale_milli(mut self, scale_milli: u32) -> Self {
        self.scale_milli = scale_milli.max(1);
        self
    }

    /// 是否可用于排版（已映射且尺寸非零）。
    pub fn is_usable(&self) -> bool {
        self.mapped && self.width > 0 && self.height > 0
    }

    /// 正文区域（窗口绝对坐标）：`None` 表示窗口尚未映射或面积为 0，调用方不得排版。
    pub fn content_area(&self, metrics: &LayoutMetrics, menu_open: bool) -> Option<ContentArea> {
        if !self.is_usable() {
            return None;
        }
        let area = metrics.content_area(self.width, self.height, menu_open);
        (area.width > 0 && area.height > 0).then_some(area)
    }
}

impl Default for MappedViewport {
    fn default() -> Self {
        Self::UNMAPPED
    }
}

/// 平台真实字体度量（由 Slint `Text.font-metrics` 投影，毫像素）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlatformFontMetrics {
    /// 基线以上高度。
    pub ascent_milli: u32,
    /// 基线以下深度。
    pub descent_milli: u32,
}

impl PlatformFontMetrics {
    /// 尚未取得度量（窗口未映射时 Slint 会给出 0）。
    pub const UNKNOWN: Self = Self {
        ascent_milli: 0,
        descent_milli: 0,
    };

    /// 由像素度量构造。
    pub fn from_px(ascent_px: u32, descent_px: u32) -> Self {
        Self {
            ascent_milli: ascent_px.saturating_mul(1_000),
            descent_milli: descent_px.saturating_mul(1_000),
        }
    }

    /// 字体自身行盒高度（毫像素）；未取得时为 0。
    pub fn line_box_milli(&self) -> u32 {
        self.ascent_milli.saturating_add(self.descent_milli)
    }

    /// 是否已取得真实度量。
    pub fn is_known(&self) -> bool {
        self.line_box_milli() > 0
    }
}

/// 排版设置（毫单位，与核心设置表同口径）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypographySettings {
    /// 字体族。
    pub font_family: String,
    /// 字重。
    pub font_weight: u16,
    /// 字号（毫像素）。
    pub font_size_milli: u32,
    /// 行距（毫像素；与字号相加得到行盒下限）。
    pub line_spacing_milli: u32,
    /// 段距（毫像素；桌面端渲染不叠加段距，签名与测量一律置 0）。
    pub paragraph_spacing_milli: u32,
    /// 首行缩进（毫像素；桌面端渲染不做缩进，一律置 0）。
    pub first_line_indent_milli: u32,
}

impl TypographySettings {
    /// 由核心设置记录投影（毫单位直通，负值视为 0）。
    pub fn from_reader_settings(settings: &ReaderSettings) -> Self {
        Self {
            font_family: settings.font_family.clone(),
            font_weight: settings.font_weight.clamp(1, 1_000) as u16,
            font_size_milli: settings.font_size_milli.max(0) as u32,
            line_spacing_milli: settings.line_spacing_milli.max(0) as u32,
            paragraph_spacing_milli: settings.paragraph_spacing_milli.max(0) as u32,
            first_line_indent_milli: settings.first_line_indent_milli.max(0) as u32,
        }
    }

    /// 桌面端默认设置（来自核心 `ReaderSettings::default()`，桌面不另造一套默认值）。
    pub fn desktop_default() -> Self {
        Self::from_reader_settings(&ReaderSettings::default())
    }

    /// 字号夹紧到桌面边界（[`MIN_FONT_SIZE_PX`]..=[`MAX_FONT_SIZE_PX`]）。
    pub fn clamped_font_size_milli(mut self) -> Self {
        self.font_size_milli = self.font_size_milli.clamp(
            MIN_FONT_SIZE_PX.saturating_mul(1_000),
            MAX_FONT_SIZE_PX.saturating_mul(1_000),
        );
        if self.font_family.trim().is_empty() {
            self.font_family = ReaderSettings::default().font_family;
        }
        self
    }
}

impl Default for TypographySettings {
    fn default() -> Self {
        Self::desktop_default()
    }
}

/// 排版参数派生失败的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypographyError {
    /// 窗口尚未映射（0×0）或正文区域面积为 0：不得排版。
    ViewportNotMapped {
        /// 窗口宽度。
        width: u32,
        /// 窗口高度。
        height: u32,
        /// 是否为「已映射但太小」。
        mapped: bool,
    },
    /// 窗口太小，一行都排不下（不产生裁切半字的页面）。
    WindowTooSmall {
        /// 正文区域高度（毫像素）。
        content_height_milli: u32,
        /// 行盒高度（毫像素）。
        line_pitch_milli: u32,
    },
}

impl TypographyError {
    /// 可读说明（用于状态栏与日志；不作为业务分支）。
    pub fn message(&self) -> String {
        match self {
            TypographyError::ViewportNotMapped {
                width,
                height,
                mapped,
            } => format!(
                "窗口{}（{}×{}）：正文区域尺寸不可用，暂不排版",
                if *mapped { "过小" } else { "尚未映射" },
                width,
                height
            ),
            TypographyError::WindowTooSmall {
                content_height_milli,
                line_pitch_milli,
            } => format!(
                "窗口高度不足：正文区 {} 毫像素 < 行盒 {} 毫像素",
                content_height_milli, line_pitch_milli
            ),
        }
    }
}

/// 测量与渲染共用的派生排版参数（同一份数值）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Typography {
    /// 字体族（Slint `Text.font-family` 与测量共用）。
    pub font_family: String,
    /// 字重。
    pub font_weight: u16,
    /// 字号（毫像素）。
    pub font_size_milli: u32,
    /// 行盒高度（毫像素）：渲染端逐行定位与测量断行共用。
    pub line_pitch_milli: u32,
    /// 字距（毫像素）：桌面恒 0。
    pub letter_spacing_milli: u32,
    /// 段距（毫像素）：桌面恒 0。
    pub paragraph_spacing_milli: u32,
    /// 首行缩进（毫像素）：桌面恒 0。
    pub first_line_indent_milli: u32,
    /// 正文区宽度（毫像素）：断行宽度上限。
    pub line_width_milli: u32,
    /// 正文区高度（毫像素）。
    pub content_height_milli: u32,
    /// 每页行数（`floor(高度 / 行盒)`，至少 1）。
    pub lines_per_page: u32,
}

impl Typography {
    /// 由签名与平台字体度量派生（签名里的视口尺寸**就是**正文区域尺寸）。
    pub fn derive(
        signature: &LayoutSignature,
        font: PlatformFontMetrics,
    ) -> Result<Self, TypographyError> {
        if signature.viewport_width == 0 || signature.viewport_height == 0 {
            return Err(TypographyError::ViewportNotMapped {
                width: signature.viewport_width,
                height: signature.viewport_height,
                mapped: false,
            });
        }
        let fallback_line_box = (signature.font_size_milli as u64
            * FALLBACK_LINE_BOX_MILLI_PER_EM as u64
            / 1_000) as u32;
        let natural_line_box = font.line_box_milli().max(fallback_line_box);
        let line_pitch_milli = signature
            .font_size_milli
            .saturating_add(signature.line_spacing_milli)
            .max(natural_line_box)
            .max(1);
        // 签名里的视口尺寸是**像素**（与 Slint 的 `width` / `height` 同口径）；
        // 派生参数统一折算成毫像素，避免「像素 × 毫像素」混算（会把 564 px 当成 564 毫像素）。
        let content_height_milli = signature.viewport_height.saturating_mul(1_000);
        let line_width_milli = signature.viewport_width.saturating_mul(1_000);
        if content_height_milli < line_pitch_milli {
            return Err(TypographyError::WindowTooSmall {
                content_height_milli,
                line_pitch_milli,
            });
        }
        Ok(Self {
            font_family: signature.font_family.clone(),
            font_weight: signature.font_weight,
            font_size_milli: signature.font_size_milli,
            line_pitch_milli,
            // 渲染端显式置零的字距/段距/首行缩进必须在测量侧同样为零（同口径契约）。
            letter_spacing_milli: 0,
            paragraph_spacing_milli: 0,
            first_line_indent_milli: 0,
            line_width_milli,
            content_height_milli,
            lines_per_page: content_height_milli / line_pitch_milli,
        })
    }

    /// 一页可以排下的行数（至少 1；派生阶段已保证高度足够）。
    pub fn lines_per_page(&self) -> u32 {
        self.lines_per_page.max(1)
    }

    /// 用满一行所需的像素宽度（渲染端与测量端同口径）。
    pub fn line_width_px(&self) -> u32 {
        self.line_width_milli / 1_000
    }

    /// 行盒高度的像素口径（渲染端逐行定位使用）。
    pub fn line_pitch_px(&self) -> u32 {
        self.line_pitch_milli.div_ceil(1_000)
    }

    /// 字号像素口径。
    pub fn font_size_px(&self) -> u32 {
        self.font_size_milli.div_ceil(1_000)
    }
}

/// 重排原因（分类只用于日志与证据，不参与业务分支）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReflowReason {
    /// 首次取得可用视口（首屏排版）。
    Initial,
    /// 视口尺寸变化（含窗口缩放与阅读菜单展开：两者都改变正文区域尺寸）。
    ViewportChanged,
    /// 字号 / 行距 / 字体族变化。
    FontChanged,
    /// DPI 比例或真实字体度量变化（字号与字体族不变）。
    DpiChanged,
}

impl ReflowReason {
    /// 稳定短名（日志用）。
    pub const fn as_str(self) -> &'static str {
        match self {
            ReflowReason::Initial => "Initial",
            ReflowReason::ViewportChanged => "ViewportChanged",
            ReflowReason::FontChanged => "FontChanged",
            ReflowReason::DpiChanged => "DpiChanged",
        }
    }
}

/// 平台字体指纹：字体族 + 字重 + 字号 + 真实度量 + DPI 比例。
///
/// 与 `reading_core::LayoutSignature::font_fingerprint` 同类（同为 FNV-1a 64），
/// 但把**真实字体度量与 DPI**也纳入输入：同一字体族在不同 DPI 或不同度量下不得共享分页边界。
pub fn font_fingerprint(
    font_family: &str,
    font_weight: u16,
    font_size_milli: u32,
    font: PlatformFontMetrics,
    scale_milli: u32,
) -> u64 {
    let mut seed = Vec::with_capacity(font_family.len() + 24);
    seed.extend_from_slice(font_family.as_bytes());
    seed.extend_from_slice(&font_weight.to_le_bytes());
    seed.extend_from_slice(&font_size_milli.to_le_bytes());
    seed.extend_from_slice(&font.ascent_milli.to_le_bytes());
    seed.extend_from_slice(&font.descent_milli.to_le_bytes());
    seed.extend_from_slice(&scale_milli.to_le_bytes());
    hash_bytes(&seed)
}

/// 由「映射后的窗口 + 版式常量 + 排版设置 + 真实字体度量」派生排版签名。
///
/// 返回 `Err` 时**不得**排版：窗口尚未映射或正文区域面积为 0（禁止用事件循环启动前的
/// 0×0 尺寸初始化页面）。
pub fn layout_signature(
    viewport: &MappedViewport,
    metrics: &LayoutMetrics,
    settings: &TypographySettings,
    font: PlatformFontMetrics,
    menu_open: bool,
) -> Result<LayoutSignature, TypographyError> {
    let area =
        viewport
            .content_area(metrics, menu_open)
            .ok_or(TypographyError::ViewportNotMapped {
                width: viewport.width,
                height: viewport.height,
                mapped: viewport.mapped,
            })?;
    let settings = settings.clone().clamped_font_size_milli();
    // O1 方案 A（设计 §5 S5）：正文列最大宽 = 40 × 字号并居中；断行宽度用**列宽**，
    // 高度仍用正文区高度。测量与渲染同口径（渲染端从投影获得同一列的几何）。
    let font_size_px = (settings.font_size_milli / 1_000).max(1);
    let column = metrics.text_column(&area, font_size_px);
    let signature = LayoutSignature {
        viewport_width: column.width,
        viewport_height: area.height,
        direction: reading_core::TextDirection::Horizontal,
        font_family: settings.font_family.clone(),
        font_weight: settings.font_weight,
        font_size_milli: settings.font_size_milli,
        line_spacing_milli: settings.line_spacing_milli,
        // 桌面渲染不叠加段距、不做首行缩进：签名与测量同时置零（§8.3 第 4 条）。
        paragraph_spacing_milli: 0,
        first_line_indent_milli: 0,
        // 留白由正文区域常量表达，签名内不再重复计入边距。
        margin_horizontal_milli: 0,
        margin_vertical_milli: 0,
        algorithm_version: LAYOUT_ALGORITHM_VERSION,
        platform_font_fingerprint: font_fingerprint(
            &settings.font_family,
            settings.font_weight,
            settings.font_size_milli,
            font,
            viewport.scale_milli,
        ),
    };
    Ok(signature)
}

/// 判定两次签名之间的变化属于哪一类（日志/证据用；无需业务分支）。
pub fn reflow_reason(previous: &LayoutSignature, next: &LayoutSignature) -> ReflowReason {
    if previous.platform_font_fingerprint != next.platform_font_fingerprint
        && previous.font_size_milli == next.font_size_milli
        && previous.line_spacing_milli == next.line_spacing_milli
        && previous.font_family == next.font_family
    {
        return ReflowReason::DpiChanged;
    }
    if previous.font_size_milli != next.font_size_milli
        || previous.line_spacing_milli != next.line_spacing_milli
        || previous.font_family != next.font_family
        || previous.font_weight != next.font_weight
    {
        return ReflowReason::FontChanged;
    }
    if previous.viewport_width != next.viewport_width
        || previous.viewport_height != next.viewport_height
    {
        return ReflowReason::ViewportChanged;
    }
    // 签名其余字段变化：按字体族/字重归类（正常情况下不会走到这里）。
    ReflowReason::FontChanged
}

/// 重排计划：新签名 + 新排版参数 + 起始位置（当前位置/锚点）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflowPlan {
    /// 新的排版签名。
    pub signature: LayoutSignature,
    /// 新的派生排版参数。
    pub typography: Typography,
    /// 重新分页的起始字节（当前页起点）。
    pub start_byte: u64,
    /// 变化分类。
    pub reason: ReflowReason,
    /// 签名是否真的发生了变化（未变化时不得清空分页缓存）。
    pub changed: bool,
}

/// 生成一次重排计划。
///
/// `previous` 为当前的签名（首次排版传 `None`）。签名未变化时 `changed == false`：
/// 调用方应复用已有页面，**不得**因为一次无变化的通知就清空分页缓存。
pub fn plan_reflow(
    previous: Option<&LayoutSignature>,
    viewport: &MappedViewport,
    metrics: &LayoutMetrics,
    settings: &TypographySettings,
    font: PlatformFontMetrics,
    menu_open: bool,
    start_byte: u64,
) -> Result<ReflowPlan, TypographyError> {
    let signature = layout_signature(viewport, metrics, settings, font, menu_open)?;
    let typography = Typography::derive(&signature, font)?;
    let (reason, changed) = match previous {
        None => (ReflowReason::Initial, true),
        Some(previous) => {
            if previous.viewport_height == signature.viewport_height
                && previous.viewport_width == signature.viewport_width
                && previous.font_size_milli == signature.font_size_milli
                && previous.line_spacing_milli == signature.line_spacing_milli
                && previous.font_family == signature.font_family
                && previous.font_weight == signature.font_weight
                && previous.platform_font_fingerprint == signature.platform_font_fingerprint
                && previous.margin_horizontal_milli == signature.margin_horizontal_milli
                && previous.margin_vertical_milli == signature.margin_vertical_milli
                && previous.paragraph_spacing_milli == signature.paragraph_spacing_milli
                && previous.first_line_indent_milli == signature.first_line_indent_milli
            {
                (reflow_reason(previous, &signature), false)
            } else {
                (reflow_reason(previous, &signature), true)
            }
        }
    };
    Ok(ReflowPlan {
        signature,
        typography,
        start_byte,
        reason,
        changed,
    })
}

// ===========================================================================
// DT3B：重排合并、同一把锁串行化与过期结果丢弃
// ===========================================================================
//
// 分工（与 `pagination.rs` 的分页语义严格分离）：
//
// - 本组类型只负责**并发与状态正确性**——把字号/窗口/DPI/边距等连续变更合并成一次最终
//   重排、让重排/翻页/保存位置共用同一把锁、按 `request_id` / `layout_id` 丢弃过期结果；
// - 分页与断行算法仍然只存在于 `pagination.rs` + 冻结核心，本模块不复制一行分页算法。
//
// 关键不变量：
//
// 1. **合并**：连续提交只保留最终值（拖拽 30 次字号 → 一次最终重排）；
// 2. **同一把锁**：[`SessionGate`] 的 `state` 是所有状态变更的唯一锁，重排执行、翻页、
//    保存位置都从它下面经过，因此不存在并发重填页面缓存；
// 3. **不阻塞 UI**：UI 入口（[`SessionGate::submit`] 与状态读取）只碰合并收件箱与原子量，
//    **从不等待**正在执行的重排，因此重排期间界面依然可响应，并能读到「正在重排」状态；
// 4. **过期丢弃**：作业带 `request_id` 与 `layout_id`；被更新的请求取代（`Request`）或
//    排版世代已变化（`Layout`）的结果一律丢弃，绝不覆盖当前页面。

/// 连续排版变更的可合并目标（字号/窗口/DPI/边距/菜单都投影到这四个字段）。
///
/// 单位与派生入口一致：视口为**窗口像素**（含 `scale_milli`），设置与字体度量为毫单位。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflowRequest {
    /// 窗口映射后的视口（0×0 表示尚未映射，派生阶段会被拒绝）。
    pub viewport: MappedViewport,
    /// 排版设置（字号/行距/段距/首行缩进/字体族）。
    pub settings: TypographySettings,
    /// 平台真实字体度量（行盒）；未取得时为 [`PlatformFontMetrics::UNKNOWN`]。
    pub font: PlatformFontMetrics,
    /// 阅读菜单是否展开（改变正文区域高度）。
    pub menu_open: bool,
}

impl ReflowRequest {
    /// 构造一次可合并的排版目标。
    pub fn new(
        viewport: MappedViewport,
        settings: TypographySettings,
        font: PlatformFontMetrics,
        menu_open: bool,
    ) -> Self {
        Self {
            viewport,
            settings,
            font,
            menu_open,
        }
    }
}

/// 提交票据：请求号单调递增；`merged` 表示本次提交合并掉了尚未执行的上一次请求。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReflowTicket {
    /// 本次提交的请求号（后续提交只会更大）。
    pub request_id: u64,
    /// 是否合并掉了收件箱里尚未执行的请求（拖拽合并的计数来源）。
    pub merged: bool,
}

/// 合并收件箱：连续变更**只保留最终值**。
///
/// 语义：`submit` 覆盖上一次未取走的请求（不排长队）；`take` 取走的永远是**最新**那一份，
/// 并携带它提交时分配的请求号。因此 30 次连续字号变更只会产生一次最终重排。
#[derive(Debug, Default)]
pub struct ReflowInbox {
    pending: Option<ReflowRequest>,
    next_request_id: u64,
    latest_request_id: u64,
    submitted: u64,
    merged: u64,
}

impl ReflowInbox {
    /// 空收件箱。
    pub fn new() -> Self {
        Self::default()
    }

    /// 提交一次目标：覆盖尚未取走的请求（合并），返回票据。
    pub fn submit(&mut self, request: ReflowRequest) -> ReflowTicket {
        self.next_request_id += 1;
        let request_id = self.next_request_id;
        let merged = self.pending.is_some();
        if merged {
            self.merged += 1;
        }
        self.pending = Some(request);
        self.latest_request_id = request_id;
        self.submitted += 1;
        ReflowTicket { request_id, merged }
    }

    /// 取走最终值（连同它的请求号）；没有待执行目标时返回 `None`。
    pub fn take(&mut self) -> Option<(u64, ReflowRequest)> {
        let request = self.pending.take()?;
        Some((self.latest_request_id, request))
    }
}

/// 过期原因（判定只用于「丢弃」与日志，不参与业务分支）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleReason {
    /// 已被更新的重排请求取代（在途期间用户继续拖拽字号/缩放窗口）。
    Request,
    /// 排版世代已变化（引擎重置/重新打开书籍/另一次重排已应用）。
    Layout,
}

impl StaleReason {
    /// 稳定短名（日志用）。
    pub const fn as_str(self) -> &'static str {
        match self {
            StaleReason::Request => "request",
            StaleReason::Layout => "layout",
        }
    }

    /// 可读说明（状态栏/日志用）。
    pub const fn message(self) -> &'static str {
        match self {
            StaleReason::Request => "结果已被更新的重排请求取代，已丢弃（不覆盖当前页面）",
            StaleReason::Layout => "结果属于旧排版世代，已丢弃（不覆盖当前页面）",
        }
    }
}

/// 过期判定（纯函数，便于独立断言）。
///
/// 先判请求号（同一世代内更新的请求让旧结果作废），再判世代号（引擎世代变化让旧结果作废）。
pub fn stale_reason(
    job: &ReflowJob,
    latest_request_id: u64,
    current_layout_id: u64,
) -> Option<StaleReason> {
    if job.request_id < latest_request_id {
        return Some(StaleReason::Request);
    }
    if job.layout_id != current_layout_id {
        return Some(StaleReason::Layout);
    }
    None
}

/// 重排生命周期状态（UI 唯一来源；Slint 只显示 [`ReflowState::label`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ReflowState {
    /// 空闲（没有待执行目标，也没有在途作业）。
    Idle = 0,
    /// 已提交、等待执行（连续变更被合并到这一份）。
    Pending = 1,
    /// 正在重排（执行期间界面保持可响应）。
    Running = 2,
    /// 最后一次重排已应用。
    Applied = 3,
    /// 最后一次重排失败（保留页面，只更新文案）。
    Failed = 4,
}

impl ReflowState {
    /// 由原子量的编码还原。
    pub const fn from_code(code: u8) -> Self {
        match code {
            1 => ReflowState::Pending,
            2 => ReflowState::Running,
            3 => ReflowState::Applied,
            4 => ReflowState::Failed,
            _ => ReflowState::Idle,
        }
    }

    /// 界面文案（Rust 侧唯一来源）。
    pub const fn label(self) -> &'static str {
        match self {
            ReflowState::Idle => "排版就绪",
            ReflowState::Pending => "等待重排…",
            ReflowState::Running => "正在重排…",
            ReflowState::Applied => "重排完成",
            ReflowState::Failed => "重排失败",
        }
    }
}

/// 重排统计（证据与 UI 都用这一份口径）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReflowStats {
    /// 提交次数（连续变更的每一次）。
    pub submitted: u64,
    /// 被合并掉的次数（未执行就被最终值取代）。
    pub merged: u64,
    /// 实际开始执行的作业数。
    pub started: u64,
    /// 实际应用（未被丢弃）的结果数。
    pub applied: u64,
    /// 因被新请求取代而丢弃的结果数。
    pub stale_request: u64,
    /// 因世代变化而丢弃的结果数。
    pub stale_layout: u64,
}

/// 一次重排作业：携带提交时的请求号与排版世代。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflowJob {
    /// 提交时刻分配的请求号（越大越新）。
    pub request_id: u64,
    /// 取作业时引擎的排版世代。
    pub layout_id: u64,
    /// 合并后的最终目标。
    pub request: ReflowRequest,
}

/// 执行结果（由调用方的执行闭包给出）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReflowApplied {
    /// 重排后的新排版世代。
    pub layout_id: u64,
    /// 签名是否真的变化（未变化时不清空页面缓存）。
    pub changed: bool,
}

/// 一次重排执行（或跳过/丢弃）的结论。
#[derive(Debug)]
pub enum RunOutcome<R> {
    /// 已执行并应用（`output` 供 UI 投影；`changed` 为签名是否变化）。
    Applied {
        /// 新排版世代。
        layout_id: u64,
        /// 签名是否变化。
        changed: bool,
        /// 执行闭包的返回值（投影数据等）。
        output: R,
    },
    /// 执行前就已过期：不执行（省下一次测量），不覆盖当前页面。
    Skipped {
        /// 过期原因。
        reason: StaleReason,
    },
    /// 执行后才发现过期：结果丢弃，不覆盖当前页面。
    Discarded {
        /// 过期原因。
        reason: StaleReason,
    },
    /// 执行失败（保留当前页面，只更新文案）。
    Failed {
        /// 可读错误信息。
        message: String,
    },
    /// 没有待执行目标。
    Idle,
}

impl<R> RunOutcome<R> {
    /// 是否真的应用了结果。
    pub fn is_applied(&self) -> bool {
        matches!(self, RunOutcome::Applied { .. })
    }

    /// 应用后的输出。
    pub fn output(self) -> Option<R> {
        match self {
            RunOutcome::Applied { output, .. } => Some(output),
            _ => None,
        }
    }

    /// 一句话说明（状态栏/日志用）。
    pub fn describe(&self) -> String {
        match self {
            RunOutcome::Applied {
                layout_id, changed, ..
            } => format!(
                "重排已应用（世代 {layout_id}{}）",
                if *changed {
                    "，签名已变化"
                } else {
                    "，签名未变"
                }
            ),
            RunOutcome::Skipped { reason } | RunOutcome::Discarded { reason } => {
                reason.message().to_string()
            }
            RunOutcome::Failed { message } => format!("重排失败：{message}"),
            RunOutcome::Idle => "没有待执行的重排目标".to_string(),
        }
    }
}

/// 门闩内部状态（唯一锁保护：引擎 + 在途作业 + 当前世代）。
struct GateState<E> {
    engine: E,
    layout_id: u64,
    running: Option<ReflowJob>,
}

/// 阅读会话门闩（DT3B）：重排、翻页、保存位置共用**同一把锁**。
///
/// # 锁的划分
///
/// - `state`（唯一状态锁）：引擎与在途作业。重排执行、翻页、保存位置都从这里经过，
///   因此不存在「两条入口并发重填页面缓存」的情况；
/// - `inbox`（合并收件箱）：只有 UI 侧的 `submit` 碰它，**从不等待正在执行的重排**；
/// - 原子量：状态、世代、最新请求号与统计，供 UI 无锁读取（重排期间依然可响应）。
///
/// # worker 两步协议
///
/// 1. [`SessionGate::begin_job`]：取走合并后的最终目标并登记在途（不执行）；
/// 2. [`SessionGate::apply_job`]（或一步到位的 [`SessionGate::run_next`]）：在唯一锁下
///    做过期判定 → 执行 → 结算；过期结果只记账，绝不覆盖当前页面。
pub struct SessionGate<E> {
    state: Mutex<GateState<E>>,
    inbox: Mutex<ReflowInbox>,
    wake: Condvar,
    latest_request_id: AtomicU64,
    current_layout_id: AtomicU64,
    in_flight: AtomicBool,
    state_code: AtomicU8,
    shutdown: AtomicBool,
    submitted: AtomicU64,
    merged: AtomicU64,
    started: AtomicU64,
    applied: AtomicU64,
    stale_request: AtomicU64,
    stale_layout: AtomicU64,
}

impl<E> SessionGate<E> {
    /// 建立门闩（`layout_id` 为引擎当前世代）。
    pub fn new(engine: E, layout_id: u64) -> Self {
        Self {
            state: Mutex::new(GateState {
                engine,
                layout_id,
                running: None,
            }),
            inbox: Mutex::new(ReflowInbox::new()),
            wake: Condvar::new(),
            latest_request_id: AtomicU64::new(0),
            current_layout_id: AtomicU64::new(layout_id),
            in_flight: AtomicBool::new(false),
            state_code: AtomicU8::new(ReflowState::Idle as u8),
            shutdown: AtomicBool::new(false),
            submitted: AtomicU64::new(0),
            merged: AtomicU64::new(0),
            started: AtomicU64::new(0),
            applied: AtomicU64::new(0),
            stale_request: AtomicU64::new(0),
            stale_layout: AtomicU64::new(0),
        }
    }

    fn lock_state(&self) -> MutexGuard<'_, GateState<E>> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn lock_inbox(&self) -> MutexGuard<'_, ReflowInbox> {
        match self.inbox.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// **翻页 / 保存位置 / 打开书籍的唯一入口**：与重排执行共用同一把锁。
    pub fn with_engine<R>(&self, mutate: impl FnOnce(&mut E) -> R) -> R {
        let mut state = self.lock_state();
        mutate(&mut state.engine)
    }

    /// **UI 入口**：提交一次连续变更（合并：只保留最终值）。永不等待正在执行的重排。
    pub fn submit(&self, request: ReflowRequest) -> ReflowTicket {
        let ticket = {
            let mut inbox = self.lock_inbox();
            let ticket = inbox.submit(request);
            self.latest_request_id
                .store(ticket.request_id, Ordering::Relaxed);
            self.wake.notify_all();
            ticket
        };
        self.submitted.fetch_add(1, Ordering::Relaxed);
        if ticket.merged {
            self.merged.fetch_add(1, Ordering::Relaxed);
        }
        // 有在途作业时保持「正在重排」；否则显示「等待重排」。
        if !self.in_flight.load(Ordering::Relaxed) {
            self.set_state(ReflowState::Pending);
        }
        ticket
    }

    /// 收件箱里待执行的最终值（UI 只读）。
    pub fn pending_request(&self) -> Option<ReflowRequest> {
        self.lock_inbox().pending.clone()
    }

    /// 是否有待执行目标。
    pub fn has_pending(&self) -> bool {
        self.lock_inbox().pending.is_some()
    }

    /// 当前排版世代（无锁可读）。
    pub fn layout_id(&self) -> u64 {
        self.current_layout_id.load(Ordering::Relaxed)
    }

    /// 最新请求号（无锁可读）。
    pub fn latest_request_id(&self) -> u64 {
        self.latest_request_id.load(Ordering::Relaxed)
    }

    /// 当前重排状态（无锁可读；重排期间不会阻塞 UI）。
    pub fn state(&self) -> ReflowState {
        ReflowState::from_code(self.state_code.load(Ordering::Relaxed))
    }

    /// 统计快照（无锁可读）。
    pub fn stats(&self) -> ReflowStats {
        ReflowStats {
            submitted: self.submitted.load(Ordering::Relaxed),
            merged: self.merged.load(Ordering::Relaxed),
            started: self.started.load(Ordering::Relaxed),
            applied: self.applied.load(Ordering::Relaxed),
            stale_request: self.stale_request.load(Ordering::Relaxed),
            stale_layout: self.stale_layout.load(Ordering::Relaxed),
        }
    }

    /// 引擎世代外部变化（重新打开书籍/外部重置）：在途结果一律作废。
    pub fn note_layout(&self, layout_id: u64) {
        let mut state = self.lock_state();
        state.layout_id = layout_id;
        drop(state);
        self.current_layout_id.store(layout_id, Ordering::Relaxed);
    }

    /// worker 两步协议第 1 步：取走合并后的最终目标并登记在途。
    ///
    /// 已有在途作业时返回 `None`（单作业在途，先结算再取下一个）。
    pub fn begin_job(&self) -> Option<ReflowJob> {
        let mut state = self.lock_state();
        if state.running.is_some() {
            return None;
        }
        let (request_id, request) = self.lock_inbox().take()?;
        let job = ReflowJob {
            request_id,
            layout_id: state.layout_id,
            request,
        };
        state.running = Some(job.clone());
        drop(state);
        self.started.fetch_add(1, Ordering::Relaxed);
        self.in_flight.store(true, Ordering::Relaxed);
        self.set_state(ReflowState::Running);
        Some(job)
    }

    /// worker 两步协议第 2 步：过期判定 → 执行 → 结算（执行期间持有唯一锁）。
    ///
    /// 已过期的作业**不执行**（[`RunOutcome::Skipped`]）；执行期间出现更新的请求时结果被
    /// 丢弃（[`RunOutcome::Discarded`]）且不计入已应用——任何情况下都不会覆盖当前页面。
    pub fn apply_job<R>(
        &self,
        job: &ReflowJob,
        execute: impl FnOnce(&mut E, &ReflowJob) -> Result<(ReflowApplied, R), String>,
    ) -> RunOutcome<R> {
        let mut state = self.lock_state();
        let latest = self.latest_request_id.load(Ordering::Relaxed);
        if let Some(reason) = stale_reason(job, latest, state.layout_id) {
            state.running = None;
            drop(state);
            self.count_stale(reason);
            self.finish(ReflowState::Idle);
            return RunOutcome::Skipped { reason };
        }
        let result = execute(&mut state.engine, job);
        match result {
            Ok((applied, output)) => {
                // 执行期间可能又来了新请求：结果同样不得覆盖当前页面。
                let latest = self.latest_request_id.load(Ordering::Relaxed);
                let stale = stale_reason(job, latest, state.layout_id);
                state.layout_id = applied.layout_id;
                state.running = None;
                drop(state);
                self.current_layout_id
                    .store(applied.layout_id, Ordering::Relaxed);
                if let Some(reason) = stale {
                    self.count_stale(reason);
                    self.finish(ReflowState::Idle);
                    return RunOutcome::Discarded { reason };
                }
                self.applied.fetch_add(1, Ordering::Relaxed);
                self.finish(ReflowState::Applied);
                RunOutcome::Applied {
                    layout_id: applied.layout_id,
                    changed: applied.changed,
                    output,
                }
            }
            Err(message) => {
                drop(state);
                self.finish(ReflowState::Failed);
                RunOutcome::Failed { message }
            }
        }
    }

    /// 一步完成：取作业并执行（worker 线程的主循环体）。
    pub fn run_next<R>(
        &self,
        execute: impl FnOnce(&mut E, &ReflowJob) -> Result<(ReflowApplied, R), String>,
    ) -> RunOutcome<R> {
        match self.begin_job() {
            Some(job) => self.apply_job(&job, execute),
            None => RunOutcome::Idle,
        }
    }

    /// worker 等待新目标（阻塞至多 `timeout`；返回是否真的有待执行目标）。
    pub fn wait_for_work(&self, timeout: Duration) -> bool {
        let mut inbox = self.lock_inbox();
        if inbox.pending.is_none() && !self.shutdown.load(Ordering::Relaxed) {
            let (guard, _) = self
                .wake
                .wait_timeout(inbox, timeout)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            inbox = guard;
        }
        inbox.pending.is_some()
    }

    /// 请求 worker 退出（进程/窗口关闭时调用；不再接收新目标）。
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.wake.notify_all();
    }

    /// worker 是否已被请求退出。
    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::Relaxed)
    }

    fn set_state(&self, state: ReflowState) {
        self.state_code.store(state as u8, Ordering::Relaxed);
    }

    fn count_stale(&self, reason: StaleReason) {
        match reason {
            StaleReason::Request => {
                self.stale_request.fetch_add(1, Ordering::Relaxed);
            }
            StaleReason::Layout => {
                self.stale_layout.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// 结算收尾：刷新状态（还有待执行目标时回到「等待重排」）。
    fn finish(&self, settled: ReflowState) {
        self.in_flight.store(false, Ordering::Relaxed);
        let state = if self.has_pending() {
            ReflowState::Pending
        } else {
            settled
        };
        self.set_state(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapped_settings() -> TypographySettings {
        TypographySettings::default()
    }

    fn vertical_metrics() -> PlatformFontMetrics {
        PlatformFontMetrics::from_px(22, 5)
    }

    #[test]
    fn viewport_after_map_only_uses_mapped_content_area() {
        let metrics = LayoutMetrics::DESKTOP;
        // 事件循环启动前的 0×0：不得排版，也不得据内容区域算出负数/零面积。
        assert_eq!(MappedViewport::UNMAPPED.content_area(&metrics, false), None);
        assert_eq!(
            MappedViewport::new(0, 720).content_area(&metrics, false),
            None
        );
        assert!(!MappedViewport::new(0, 0).is_usable());

        // 窗口映射完成后：正文区域与版式常量同口径。
        let viewport = MappedViewport::new(1080, 720);
        let area = viewport.content_area(&metrics, false).expect("映射后可用");
        assert_eq!(area, metrics.content_area(1080, 720, false));
        assert!(area.width > 0 && area.height > 0);

        let signature = layout_signature(
            &viewport,
            &metrics,
            &mapped_settings(),
            vertical_metrics(),
            false,
        )
        .expect("映射后可派生签名");
        // 1080 宽窗口：正文列被限宽到 40 × 18px = 720（O1 方案 A）。
        let expected = metrics.text_column(&area, 18);
        assert_eq!(signature.viewport_width, expected.width);
        assert_eq!(signature.viewport_height, area.height);
        assert!(signature.validate().is_ok(), "签名必须通过核心校验");

        // 未映射时签名派生必须失败（禁止 0×0 排版）。
        let unmapped = layout_signature(
            &MappedViewport::UNMAPPED,
            &metrics,
            &mapped_settings(),
            vertical_metrics(),
            false,
        );
        assert!(matches!(
            unmapped,
            Err(TypographyError::ViewportNotMapped { .. })
        ));

        // 已映射但一行都排不下：同样拒绝（不产生裁切半字的页面）。
        // 170 px 高 → 正文区 170-56-44-56 = 14 px，小于行盒（27 px）。
        let tiny = MappedViewport::new(1080, 170).with_scale_milli(1_000);
        let tiny_signature = layout_signature(
            &tiny,
            &metrics,
            &mapped_settings(),
            vertical_metrics(),
            false,
        )
        .expect("尺寸非零仍可派生签名");
        assert!(matches!(
            Typography::derive(&tiny_signature, vertical_metrics()),
            Err(TypographyError::WindowTooSmall { .. })
        ));
    }

    #[test]
    fn paragraph_and_indent_zero_match_slint_rendering() {
        // Slint `Text` 既不叠加段距也不做首行缩进：签名与测量侧必须同时为零。
        let mut settings = mapped_settings();
        settings.paragraph_spacing_milli = 12_000;
        settings.first_line_indent_milli = 40_000;
        let signature = layout_signature(
            &MappedViewport::new(1080, 720),
            &LayoutMetrics::DESKTOP,
            &settings,
            vertical_metrics(),
            false,
        )
        .expect("派生签名");
        assert_eq!(signature.paragraph_spacing_milli, 0, "段距必须置零");
        assert_eq!(signature.first_line_indent_milli, 0, "首行缩进必须置零");
        assert_eq!(signature.margin_horizontal_milli, 0, "边距由正文区域表达");
        assert_eq!(signature.margin_vertical_milli, 0, "边距由正文区域表达");

        let typography = Typography::derive(&signature, vertical_metrics()).expect("派生排版参数");
        assert_eq!(typography.paragraph_spacing_milli, 0);
        assert_eq!(typography.first_line_indent_milli, 0);
        assert_eq!(typography.letter_spacing_milli, 0, "字距必须与渲染一致");
    }

    #[test]
    fn typography_line_pitch_is_never_below_font_line_box() {
        // 行盒必须 ≥ 真实字体行盒：否则逐行定位会让相邻行重叠。
        let font = PlatformFontMetrics::from_px(22, 5);
        let signature = layout_signature(
            &MappedViewport::new(1080, 720),
            &LayoutMetrics::DESKTOP,
            &mapped_settings(),
            font,
            false,
        )
        .expect("派生签名");
        let typography = Typography::derive(&signature, font).expect("派生排版参数");
        assert!(typography.line_pitch_milli >= font.line_box_milli());
        assert!(typography.line_pitch_milli >= typography.font_size_milli);

        // 未取得度量时退化到保守常数（1.5 em），绝不低于字号。
        let unknown =
            Typography::derive(&signature, PlatformFontMetrics::UNKNOWN).expect("派生排版参数");
        assert!(unknown.line_pitch_milli >= signature.font_size_milli * 3 / 2);
    }

    #[test]
    fn typography_pages_are_filled_without_overflow() {
        // 「铺满且不溢出」：每页行数用满可用高度，再加一行就会溢出。
        let font = PlatformFontMetrics::from_px(22, 5);
        for height in [600u32, 720, 900, 1_080] {
            let signature = layout_signature(
                &MappedViewport::new(1_080, height),
                &LayoutMetrics::DESKTOP,
                &mapped_settings(),
                font,
                false,
            )
            .expect("派生签名");
            let typography = Typography::derive(&signature, font).expect("派生排版参数");
            let used = typography
                .lines_per_page()
                .saturating_mul(typography.line_pitch_milli);
            assert!(
                used <= typography.content_height_milli,
                "每页正文必须放得下（不溢出/不出现滚动）"
            );
            assert!(
                used + typography.line_pitch_milli > typography.content_height_milli,
                "正文必须铺满：再加一行会溢出（不得留出整行空白）"
            );
        }
    }

    #[test]
    fn menu_toggle_changes_content_height_and_menu_reason() {
        let metrics = LayoutMetrics::DESKTOP;
        let viewport = MappedViewport::new(1_080, 720);
        let settings = mapped_settings();
        let font = vertical_metrics();
        let closed =
            plan_reflow(None, &viewport, &metrics, &settings, font, false, 0).expect("首次计划");
        assert_eq!(closed.reason, ReflowReason::Initial);
        assert!(closed.changed);

        let open = plan_reflow(
            Some(&closed.signature),
            &viewport,
            &metrics,
            &settings,
            font,
            true,
            0,
        )
        .expect("菜单展开计划");
        assert!(open.changed, "展开菜单改变正文区域高度 → 必须重排");
        assert_eq!(open.reason, ReflowReason::ViewportChanged);
        assert_eq!(
            open.typography.content_height_milli + crate::layout::TOOLBAR_HEIGHT * 1_000,
            closed.typography.content_height_milli
        );

        let same = plan_reflow(
            Some(&closed.signature),
            &viewport,
            &metrics,
            &settings,
            font,
            false,
            0,
        )
        .expect("重复计划");
        assert!(!same.changed, "签名未变化时不得清空分页缓存");
        assert_eq!(same.signature, closed.signature);
    }

    #[test]
    fn font_and_dpi_changes_produce_new_signature() {
        let metrics = LayoutMetrics::DESKTOP;
        let viewport = MappedViewport::new(1_080, 720);
        let font = vertical_metrics();
        let base = plan_reflow(
            None,
            &viewport,
            &metrics,
            &mapped_settings(),
            font,
            false,
            0,
        )
        .expect("基准计划");

        // 字号变化。
        let mut bigger = mapped_settings();
        bigger.font_size_milli = 24_000;
        let font_plan = plan_reflow(
            Some(&base.signature),
            &viewport,
            &metrics,
            &bigger,
            font,
            false,
            0,
        )
        .expect("字号变化计划");
        assert!(font_plan.changed);
        assert_eq!(font_plan.reason, ReflowReason::FontChanged);
        assert_ne!(font_plan.signature.layout_id(), base.signature.layout_id());

        // DPI 变化（同一逻辑尺寸、不同比例/度量）。
        let dpi_plan = plan_reflow(
            Some(&base.signature),
            &viewport.with_scale_milli(2_000),
            &metrics,
            &mapped_settings(),
            PlatformFontMetrics::from_px(30, 7),
            false,
            0,
        )
        .expect("DPI 变化计划");
        assert!(dpi_plan.changed, "DPI/度量变化必须换签名");
        assert_eq!(dpi_plan.reason, ReflowReason::DpiChanged);

        // 窗口尺寸变化。
        let resized = plan_reflow(
            Some(&base.signature),
            &MappedViewport::new(1_200, 800),
            &metrics,
            &mapped_settings(),
            font,
            false,
            0,
        )
        .expect("窗口变化计划");
        assert!(resized.changed);
        assert_eq!(resized.reason, ReflowReason::ViewportChanged);
        assert_eq!(resized.signature.layout_id(), {
            let recomputed = layout_signature(
                &MappedViewport::new(1_200, 800),
                &metrics,
                &mapped_settings(),
                font,
                false,
            )
            .expect("派生签名");
            recomputed.layout_id()
        });
    }

    #[test]
    fn font_size_is_clamped_to_desktop_bounds() {
        let metrics = LayoutMetrics::DESKTOP;
        let viewport = MappedViewport::new(1_080, 720);
        let font = vertical_metrics();
        let mut settings = mapped_settings();
        settings.font_size_milli = 4_000; // 4 px：低于桌面下限
        let plan =
            plan_reflow(None, &viewport, &metrics, &settings, font, false, 0).expect("派生签名");
        assert_eq!(plan.signature.font_size_milli, MIN_FONT_SIZE_PX * 1_000);

        settings.font_size_milli = 900_000;
        let plan =
            plan_reflow(None, &viewport, &metrics, &settings, font, false, 0).expect("派生签名");
        assert_eq!(plan.signature.font_size_milli, MAX_FONT_SIZE_PX * 1_000);

        // 空字体族回落到核心默认值（不产生非法签名）。
        settings.font_family = "  ".to_string();
        let plan =
            plan_reflow(None, &viewport, &metrics, &settings, font, false, 0).expect("派生签名");
        assert_eq!(
            plan.signature.font_family,
            ReaderSettings::default().font_family
        );
        assert!(plan.signature.validate().is_ok());
    }

    // ---- DT3B：重排合并、同一把锁串行化与过期结果丢弃 -----------------------

    /// 测试用引擎：只记录「被应用过的排版目标」，不含任何分页算法。
    ///
    /// 真实产品里这一步是 `ReaderRuntime` 的一次重排（`pagination.rs`）；本测试用它来断言
    /// **合并后的目标只被应用一次**、且过期结果一次都没有落到引擎上。
    #[derive(Debug, Default)]
    struct FakeEngine {
        layout_id: u64,
        reflows: u64,
        applied_font_size_milli: u32,
        applied_viewport: (u32, u32),
        applied_scale_milli: u32,
        applied_menu_open: bool,
        applied_lines_per_page: u32,
        /// 当前页面文本：用于断言过期结果**不得覆盖当前页面**。
        page_text: String,
        /// 互斥检测：正在锁内的入口数、总入口数、并发进入次数。
        active: i64,
        entries: u64,
        concurrent_entries: u64,
        reflow_thread: Option<std::thread::ThreadId>,
    }

    impl FakeEngine {
        fn new(layout_id: u64) -> Self {
            Self {
                layout_id,
                ..Self::default()
            }
        }

        /// 应用一次合并后的目标（等价于真实实现里的一次重排）。
        fn apply(&mut self, request: &ReflowRequest) -> ReflowApplied {
            let plan = plan_reflow(
                None,
                &request.viewport,
                &LayoutMetrics::DESKTOP,
                &request.settings,
                request.font,
                request.menu_open,
                0,
            )
            .expect("合并后的目标必须可派生（否则不允许执行）");
            self.reflows += 1;
            self.applied_font_size_milli = request.settings.font_size_milli;
            self.applied_viewport = (request.viewport.width, request.viewport.height);
            self.applied_scale_milli = request.viewport.scale_milli;
            self.applied_menu_open = request.menu_open;
            self.applied_lines_per_page = plan.typography.lines_per_page();
            self.layout_id += 1;
            self.page_text = format!(
                "第 1 页 · 字号 {} 像素 · 每页 {} 行",
                request.settings.font_size_milli / 1_000,
                plan.typography.lines_per_page()
            );
            ReflowApplied {
                layout_id: self.layout_id,
                changed: true,
            }
        }

        fn enter(&mut self) {
            self.active += 1;
            if self.active > 1 {
                self.concurrent_entries += 1;
            }
            self.entries += 1;
        }

        fn leave(&mut self) {
            self.active -= 1;
        }
    }

    fn reflow_request(
        viewport: MappedViewport,
        font_size_px: u32,
        menu_open: bool,
    ) -> ReflowRequest {
        let settings = TypographySettings {
            font_size_milli: font_size_px * 1_000,
            ..TypographySettings::default()
        };
        ReflowRequest::new(viewport, settings, vertical_metrics(), menu_open)
    }

    #[test]
    fn merged_reflow_requests_coalesce_thirty_font_changes() {
        // DoD：连续 30 次字号变更只产生一次最终重排。
        let viewport = MappedViewport::new(1_080, 720);
        let gate = SessionGate::new(FakeEngine::new(7), 7);
        for px in 12..42u32 {
            let ticket = gate.submit(reflow_request(viewport, px, false));
            assert_eq!(ticket.merged, px > 12, "只有第一次提交没有可合并的前值");
        }
        let stats = gate.stats();
        assert_eq!(stats.submitted, 30, "30 次连续变更都要记账");
        assert_eq!(stats.merged, 29, "29 次被合并掉");
        assert_eq!(stats.started, 0, "合并期间不得启动任何重排");
        assert_eq!(gate.state(), ReflowState::Pending, "等待重排状态对 UI 可见");
        assert_eq!(
            gate.pending_request().map(|r| r.settings.font_size_milli),
            Some(41_000),
            "合并只保留最终值"
        );

        let outcome = gate.run_next(|engine, job| {
            assert_eq!(
                job.request.settings.font_size_milli, 41_000,
                "执行的必须是最终值"
            );
            Ok((engine.apply(&job.request), ()))
        });
        assert!(outcome.is_applied(), "{}", outcome.describe());
        let stats = gate.stats();
        assert_eq!(stats.started, 1, "30 次变更只产生一次重排");
        assert_eq!(stats.applied, 1);
        assert_eq!(
            gate.with_engine(|engine| (engine.reflows, engine.applied_font_size_milli)),
            (1, 41_000)
        );
        assert!(gate.with_engine(|engine| engine.applied_lines_per_page) > 0);
        assert_eq!(gate.state(), ReflowState::Applied);

        let idle =
            gate.run_next(|engine, _| Ok((engine.apply(&reflow_request(viewport, 18, false)), ())));
        assert!(matches!(idle, RunOutcome::Idle), "没有剩余目标");
    }

    #[test]
    fn merged_reflow_requests_keep_final_values_across_change_kinds() {
        // 字号 / 窗口 / DPI / 菜单四类连续变更共用同一份合并目标。
        let gate = SessionGate::new(FakeEngine::new(3), 3);
        let mut settings = TypographySettings::default();
        for step in 0..8u32 {
            let viewport = MappedViewport::new(900 + step * 20, 700 + step * 10)
                .with_scale_milli(1_000 + step * 125);
            settings.font_size_milli = 18_000 + step * 500;
            gate.submit(ReflowRequest::new(
                viewport,
                settings.clone(),
                vertical_metrics(),
                step % 2 == 0,
            ));
        }
        assert_eq!(gate.stats().merged, 7, "四类变更同样只保留最终值");
        let job = gate.begin_job().expect("有且仅有一个作业");
        assert_eq!(job.request.viewport.width, 1_040);
        assert_eq!(job.request.viewport.height, 770);
        assert_eq!(job.request.viewport.scale_milli, 1_875, "DPI 取最终值");
        assert_eq!(job.request.settings.font_size_milli, 21_500, "字号取最终值");
        assert!(!job.request.menu_open, "菜单取最终值");
        assert!(
            gate.begin_job().is_none(),
            "同一时刻只允许一个在途作业（不得并发重排互相覆盖）"
        );
    }

    #[test]
    fn stale_layout_result_rejected() {
        let viewport = MappedViewport::new(1_080, 720);

        // (a) 世代变化：作业在途时引擎世代被外部重置 → 结果既不执行也不覆盖页面。
        let gate = SessionGate::new(FakeEngine::new(10), 10);
        gate.submit(reflow_request(viewport, 18, false));
        let job = gate.begin_job().expect("待执行作业");
        assert_eq!(job.layout_id, 10);
        gate.with_engine(|engine| engine.page_text = "当前页面（不得被过期结果覆盖）".to_string());
        gate.note_layout(11);
        let outcome = gate.apply_job(&job, |engine, job| Ok((engine.apply(&job.request), ())));
        assert!(
            matches!(
                outcome,
                RunOutcome::Skipped {
                    reason: StaleReason::Layout
                }
            ),
            "{}",
            outcome.describe()
        );
        assert_eq!(
            gate.with_engine(|engine| engine.reflows),
            0,
            "过期结果不得执行"
        );
        assert_eq!(
            gate.with_engine(|engine| engine.page_text.clone()),
            "当前页面（不得被过期结果覆盖）",
            "过期结果不得覆盖当前页面"
        );
        assert_eq!(gate.stats().stale_layout, 1);
        assert_eq!(gate.stats().applied, 0);

        // (b) 请求被取代：执行期间用户继续拖拽字号 → 结果丢弃，再按最终值重排一次。
        let gate = SessionGate::new(FakeEngine::new(20), 20);
        gate.submit(reflow_request(viewport, 18, false));
        let job = gate.begin_job().expect("待执行作业");
        let outcome = gate.apply_job(&job, |engine, job| {
            gate.submit(reflow_request(viewport, 30, false));
            Ok((engine.apply(&job.request), ()))
        });
        assert!(
            matches!(
                outcome,
                RunOutcome::Discarded {
                    reason: StaleReason::Request
                }
            ),
            "{}",
            outcome.describe()
        );
        assert_eq!(gate.stats().stale_request, 1);
        assert_eq!(gate.stats().applied, 0, "被取代的结果不计入已应用");
        assert_eq!(gate.state(), ReflowState::Pending, "最终值仍在等待重排");

        let final_outcome = gate.run_next(|engine, job| {
            assert_eq!(job.request.settings.font_size_milli, 30_000, "按最终值重排");
            Ok((engine.apply(&job.request), ()))
        });
        assert!(final_outcome.is_applied());
        assert_eq!(
            gate.with_engine(|engine| engine.applied_font_size_milli),
            30_000
        );
        assert_eq!(gate.stats().applied, 1, "只有最终值被应用一次");
    }

    #[test]
    fn serialized_state_mutations_share_one_lock() {
        use std::sync::Arc;

        let gate = Arc::new(SessionGate::new(FakeEngine::new(5), 5));
        let page_turners = 8usize;
        let turns_per_thread = 25usize;
        let reflows = 12u64;
        let mut handles = Vec::new();
        for _ in 0..page_turners {
            let gate = Arc::clone(&gate);
            handles.push(std::thread::spawn(move || {
                for _ in 0..turns_per_thread {
                    // 翻页 / 保存位置入口：与重排执行共用同一把锁。
                    gate.with_engine(|engine| {
                        engine.enter();
                        std::thread::sleep(Duration::from_millis(1));
                        engine.leave();
                    });
                }
            }));
        }
        let reflow_gate = Arc::clone(&gate);
        let reflow_thread = std::thread::spawn(move || {
            for round in 0..reflows {
                reflow_gate.submit(reflow_request(
                    MappedViewport::new(1_000 + round as u32, 700),
                    18,
                    false,
                ));
                let outcome = reflow_gate.run_next(|engine, job| {
                    engine.enter();
                    std::thread::sleep(Duration::from_millis(1));
                    let applied = engine.apply(&job.request);
                    engine.leave();
                    Ok((applied, ()))
                });
                assert!(outcome.is_applied(), "{}", outcome.describe());
            }
        });
        for handle in handles {
            handle.join().expect("翻页线程不得 panic");
        }
        reflow_thread.join().expect("重排线程不得 panic");

        let (entries, concurrent, applied_reflows) =
            gate.with_engine(|engine| (engine.entries, engine.concurrent_entries, engine.reflows));
        assert_eq!(
            entries,
            (page_turners * turns_per_thread) as u64 + reflows,
            "每个入口恰好进入一次（无丢失更新）"
        );
        assert_eq!(concurrent, 0, "任何两个入口都不得同时进入引擎");
        assert_eq!(applied_reflows, reflows);
        let stats = gate.stats();
        assert_eq!(stats.submitted, reflows);
        assert_eq!(stats.applied, reflows);
    }

    #[test]
    fn reflow_non_blocking_ui_entries_while_worker_reflows() {
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;

        let viewport = MappedViewport::new(1_080, 720);
        let gate = Arc::new(SessionGate::new(FakeEngine::new(1), 1));
        gate.submit(reflow_request(viewport, 18, false));

        let ui_thread = std::thread::current().id();
        let entered = Arc::new(AtomicBool::new(false));
        let worker_gate = Arc::clone(&gate);
        let worker_flag = Arc::clone(&entered);
        let worker = std::thread::spawn(move || {
            worker_gate.run_next(|engine, job| {
                engine.enter();
                engine.reflow_thread = Some(std::thread::current().id());
                worker_flag.store(true, Ordering::SeqCst);
                // 模拟一次较长的重排（真实实现里是整页测量 + 断行）。
                std::thread::sleep(Duration::from_millis(500));
                let applied = engine.apply(&job.request);
                engine.leave();
                Ok((applied, std::thread::current().id()))
            })
        });
        while !entered.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            gate.state(),
            ReflowState::Running,
            "重排期间 UI 必须能读到「正在重排」状态"
        );

        // UI 线程：连续 30 次字号变更 + 状态/统计读取，都不得等待正在执行的重排。
        let started = std::time::Instant::now();
        for px in 12..42u32 {
            gate.submit(reflow_request(viewport, px, false));
        }
        let state = gate.state();
        let stats = gate.stats();
        let elapsed = started.elapsed();
        assert!(
            matches!(state, ReflowState::Running | ReflowState::Pending),
            "状态可见：{state:?}"
        );
        assert!(
            stats.merged >= 29,
            "重排期间的连续变更同样被合并：{stats:?}"
        );
        assert!(
            elapsed < Duration::from_millis(100),
            "UI 入口不得等待正在执行的重排：{elapsed:?}"
        );

        let outcome = worker.join().expect("worker 线程");
        assert!(
            matches!(
                outcome,
                RunOutcome::Discarded {
                    reason: StaleReason::Request
                }
            ),
            "在途结果被新请求取代后必须丢弃：{}",
            outcome.describe()
        );
        assert_eq!(gate.stats().applied, 0, "过期结果不覆盖当前页面");

        let final_outcome = gate.run_next(|engine, job| Ok((engine.apply(&job.request), ())));
        assert!(final_outcome.is_applied());
        let (reflows, thread) = gate.with_engine(|engine| (engine.reflows, engine.reflow_thread));
        assert_eq!(reflows, 2, "在途一次 + 最终值一次");
        assert_ne!(
            thread,
            Some(ui_thread),
            "重排必须在 worker 线程执行，不得占用 UI 线程"
        );
        assert_eq!(gate.stats().applied, 1);
    }
}

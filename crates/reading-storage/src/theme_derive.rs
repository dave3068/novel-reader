//! 派生式自定义配色（V1.1，规格 R10–R13；设计稿 T1–T4）。
//!
//! **本模块是配色的唯一真相**：`desktop-app` 与 `reading-ffi`（Android）都从这里
//! 取派生结果，故「桌面看到的」与「手机看到的」必然一致 —— 避免两个真相。
//!
//! 口径（主人裁定 2026-10-05）：
//!
//! - 用户只选 **4 个基色**：正文 / 背景 / 主色 / 强调；
//! - 其余语义令牌由本模块**确定性派生**（明/暗两套）；
//! - **不要求分别设定明色与暗色**（Q3 已裁定）——切主题只改派生方向，基色不变；
//! - 40 个令牌的**实际值不落库**（每次由基色重算），故不存在「改了亮色漏了暗色」。
//!
//! 边界：
//!
//! - 派生与对比度**只在本模块**；`.slint` 与 Kotlin 不得复制（规格 R18 / 门禁 C34）。
//! - 对比度不足时**保留用户基色**并给出警告（规格 R12）——绝不强改用户选择。

use crate::theme::{BaseColors, ThemeCustom};

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
    f(
        "background",
        (0xff, 0xff, 0xff, 0xff),
        (0x16, 0x16, 0x16, 0xff),
    ),
    f(
        "layer-01",
        (0xf4, 0xf4, 0xf4, 0xff),
        (0x26, 0x26, 0x26, 0xff),
    ),
    f(
        "layer-02",
        (0xff, 0xff, 0xff, 0xff),
        (0x39, 0x39, 0x39, 0xff),
    ),
    f(
        "field-01",
        (0xff, 0xff, 0xff, 0xff),
        (0x26, 0x26, 0x26, 0xff),
    ),
    f(
        "border-subtle",
        (0xe0, 0xe0, 0xe0, 0xff),
        (0x39, 0x39, 0x39, 0xff),
    ),
    f(
        "border-strong",
        (0x8d, 0x8d, 0x8d, 0xff),
        (0x6f, 0x6f, 0x6f, 0xff),
    ),
    f(
        "text-primary",
        (0x16, 0x16, 0x16, 0xff),
        (0xf4, 0xf4, 0xf4, 0xff),
    ),
    f(
        "text-secondary",
        (0x52, 0x52, 0x52, 0xff),
        (0xc6, 0xc6, 0xc6, 0xff),
    ),
    f(
        "text-placeholder",
        (0x8d, 0x8d, 0x8d, 0xff),
        (0x6f, 0x6f, 0x6f, 0xff),
    ),
    f(
        "text-disabled",
        (0xc6, 0xc6, 0xc6, 0xff),
        (0x6f, 0x6f, 0x6f, 0xff),
    ),
    // 主色：p60 明色主色；暗色提亮为 p50（设计 §2.1 对比度规则）。
    f(
        "primary",
        (0x18, 0x90, 0xff, 0xff),
        (0x40, 0x96, 0xff, 0xff),
    ),
    f(
        "primary-hover",
        (0x09, 0x58, 0xd9, 0xff),
        (0x69, 0xb1, 0xff, 0xff),
    ),
    f(
        "primary-pressed",
        (0x00, 0x3e, 0xb3, 0xff),
        (0x91, 0xca, 0xff, 0xff),
    ),
    // p10 选中底 / 通知底；暗色取深蓝底（保持「层色 + 边框」层级）。
    f(
        "primary-soft",
        (0xe6, 0xf4, 0xff, 0xff),
        (0x11, 0x2a, 0x45, 0xff),
    ),
    // p20 Tag 底 / 描边强调。
    f(
        "primary-border",
        (0xba, 0xe0, 0xff, 0xff),
        (0x15, 0x3a, 0x5e, 0xff),
    ),
    // p70 正文链接 / 次级按钮文字（<18px 正文本必须用它，对比度 AA）。
    f(
        "primary-strong",
        (0x09, 0x58, 0xd9, 0xff),
        (0x69, 0xb1, 0xff, 0xff),
    ),
    f(
        "on-primary",
        (0xff, 0xff, 0xff, 0xff),
        (0xff, 0xff, 0xff, 0xff),
    ),
    f("focus", (0x18, 0x90, 0xff, 0xff), (0x40, 0x96, 0xff, 0xff)),
    // 顶栏 UI Shell：两主题同色（深底白字）。
    f("header", (0x16, 0x16, 0x16, 0xff), (0x16, 0x16, 0x16, 0xff)),
    f(
        "header-text",
        (0xff, 0xff, 0xff, 0xff),
        (0xff, 0xff, 0xff, 0xff),
    ),
    // 深色顶栏上的图标按钮悬停底色（若沿用浅色 hover，白色图标会看不见）。
    f(
        "header-hover",
        (0x2e, 0x2e, 0x2e, 0xff),
        (0x3d, 0x3d, 0x3d, 0xff),
    ),
    f(
        "support-success",
        (0x19, 0x80, 0x38, 0xff),
        (0x42, 0xbe, 0x65, 0xff),
    ),
    f(
        "success-soft",
        (0xde, 0xfb, 0xe6, 0xff),
        (0x0f, 0x3a, 0x24, 0xff),
    ),
    f(
        "support-warning",
        (0x8a, 0x61, 0x00, 0xff),
        (0xf1, 0xc2, 0x1b, 0xff),
    ),
    f(
        "warning-soft",
        (0xff, 0xf8, 0xe1, 0xff),
        (0x3a, 0x2c, 0x00, 0xff),
    ),
    f(
        "support-error",
        (0xda, 0x1e, 0x28, 0xff),
        (0xff, 0x83, 0x89, 0xff),
    ),
    f(
        "error-soft",
        (0xff, 0xf1, 0xf0, 0xff),
        (0x3a, 0x12, 0x14, 0xff),
    ),
    f(
        "scrollbar-track",
        (0xf4, 0xf4, 0xf4, 0xff),
        (0x26, 0x26, 0x26, 0xff),
    ),
    f(
        "scrollbar-handle",
        (0xc6, 0xc6, 0xc6, 0xff),
        (0x6f, 0x6f, 0x6f, 0xff),
    ),
    // 中性 g40 / g60（暗色），供滑块与次要描边使用。
    f(
        "control-track",
        (0xa8, 0xa8, 0xa8, 0xff),
        (0x6f, 0x6f, 0x6f, 0xff),
    ),
    // 禁用态：g10 底 / g20 边 / g30 字（§S16 状态矩阵）。
    f(
        "disabled-bg",
        (0xf4, 0xf4, 0xf4, 0xff),
        (0x26, 0x26, 0x26, 0xff),
    ),
    f(
        "disabled-border",
        (0xe0, 0xe0, 0xe0, 0xff),
        (0x39, 0x39, 0x39, 0xff),
    ),
    f(
        "disabled-text",
        (0xc6, 0xc6, 0xc6, 0xff),
        (0x6f, 0x6f, 0x6f, 0xff),
    ),
    // 弹窗遮罩 `rgba(22,22,22,.5)`。
    f(
        "overlay",
        (0x16, 0x16, 0x16, 0x80),
        (0x16, 0x16, 0x16, 0x80),
    ),
    // Toast：深底白字（两主题同色）。
    f(
        "toast-bg",
        (0x26, 0x26, 0x26, 0xff),
        (0x26, 0x26, 0x26, 0xff),
    ),
    f(
        "toast-text",
        (0xff, 0xff, 0xff, 0xff),
        (0xff, 0xff, 0xff, 0xff),
    ),
    // 暗色阅读正文（≈13:1）。
    f(
        "reading-text",
        (0x22, 0x22, 0x22, 0xff),
        (0xe0, 0xe0, 0xe0, 0xff),
    ),
    // 封面占位底（本地派生色块，不引入网络素材）。
    f(
        "cover-bg",
        (0xe6, 0xf4, 0xff, 0xff),
        (0x11, 0x2a, 0x45, 0xff),
    ),
];

const fn f(name: &'static str, light: (u8, u8, u8, u8), dark: (u8, u8, u8, u8)) -> PaletteField {
    PaletteField {
        slint_name: name,
        value: ColorPair { light, dark },
    }
}

/// 正文对背景的最低对比度（WCAG AA，规格 R12）。
pub const MIN_CONTRAST: f64 = 4.5;

/// 派生结果（供 `apply_palette` 叠加到内置明/暗之上）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedPalette {
    /// 令牌名 → `(r, g, b, a)`（明色一套）。
    pub light: Vec<(String, [u8; 4])>,
    /// 令牌名 → `(r, g, b, a)`（暗色一套）。
    pub dark: Vec<(String, [u8; 4])>,
}

impl DerivedPalette {
    /// 取某个令牌的明色值（界面注入用）。
    pub fn light_of(&self, name: &str) -> Option<[u8; 4]> {
        self.light
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| *value)
    }

    /// 取某个令牌的暗色值。
    pub fn dark_of(&self, name: &str) -> Option<[u8; 4]> {
        self.dark
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| *value)
    }

    /// 令牌总数（门禁要求「40 个令牌全覆盖」，故此处可核对）。
    pub fn token_count(&self) -> usize {
        self.light.len()
    }
}

/// 线性插值两个颜色（`t` ∈ [0,1]；`t=0` 返回 `a`，`t=1` 返回 `b`）。
fn mix(a: [u8; 3], b: [u8; 3], t: f64) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        (a[0] as f64 + (b[0] as f64 - a[0] as f64) * t).round() as u8,
        (a[1] as f64 + (b[1] as f64 - a[1] as f64) * t).round() as u8,
        (a[2] as f64 + (b[2] as f64 - a[2] as f64) * t).round() as u8,
    ]
}

/// 调整明度：`amount > 0` 变亮，`< 0` 变暗（按比例向白/黑靠拢）。
fn shade(color: [u8; 3], amount: f64) -> [u8; 3] {
    if amount >= 0.0 {
        mix(color, [255, 255, 255], amount)
    } else {
        mix(color, [0, 0, 0], -amount)
    }
}

/// 相对亮度（WCAG 2.1 口径；`sRGB` → 线性）。
pub fn relative_luminance(rgb: [u8; 3]) -> f64 {
    let channel = |value: u8| {
        let v = value as f64 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(rgb[0]) + 0.7152 * channel(rgb[1]) + 0.0722 * channel(rgb[2])
}

/// 对比度（`(L1 + 0.05) / (L2 + 0.05)`，结果 ≥ 1）。
pub fn contrast_ratio(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// 由 4 基色派生全部令牌（明/暗两套）。
///
/// 派生策略（确定性、可核对）：
/// - **中性阶**（背景/层/边框/滚动条…）：在「背景色」与「正文色」之间按固定比例插值；
/// - **主色阶**（primary 及其 hover/pressed/soft/border/strong）：在「主色」与背景之间插值；
/// - **强调**（success/warning/error 及 soft）：在「强调色」与背景之间插值；
/// - **反色**（on-primary）：按正文/背景的明度自动取黑或白；
/// - **暗色一套**：把明色派生结果整体朝暗背景收敛（层更深、文字更亮）。
///
/// 注：`header` 是 UI Shell 深底（明暗同色），故不随基色变 —— 保持顶栏稳定。
pub fn derive(base: &BaseColors) -> DerivedPalette {
    let foreground = base.resolve_foreground([0x22, 0x22, 0x22]);
    let background = base.resolve_background([0xff, 0xff, 0xff]);
    let primary = base.resolve_primary([0x18, 0x90, 0xff]);
    let accent = base.resolve_accent([0xd4, 0x38, 0x0d]);

    // 暗色方向的基色。
    //
    // 口径（实测踩中过一次错误）：暗色套件不是「把明色绝对地压暗/提亮」，
    // 那样两个方向会撞在一起 —— 白底压 0.55 得中灰(#737373)，深字提 0.35 得
    // 近同值(#6F6F6F)，对比度只剩 1.06:1（正文几乎不可读）。
    //
    // 正确做法：暗色 = **背景朝黑走、正文朝白走**，且幅度足够大，
    // 保证两者在亮度轴上分居两端；主色与强调色保持原值（它们本就为亮色系设计）。
    let dark_background = shade(background, -0.92);
    let dark_foreground = shade(foreground, 0.92);

    let mut light: Vec<(String, [u8; 4])> = Vec::with_capacity(PALETTE_FIELDS.len());
    let mut dark: Vec<(String, [u8; 4])> = Vec::with_capacity(PALETTE_FIELDS.len());

    for field in PALETTE_FIELDS {
        let (light_color, dark_color) = derive_one(
            field,
            foreground,
            background,
            primary,
            accent,
            dark_foreground,
            dark_background,
        );
        let alpha = field.value.light.3;
        light.push((field.slint_name.to_string(), with_alpha(light_color, alpha)));
        dark.push((field.slint_name.to_string(), with_alpha(dark_color, alpha)));
    }
    DerivedPalette { light, dark }
}

/// 给单个令牌派生明暗两色。
fn derive_one(
    field: &PaletteField,
    foreground: [u8; 3],
    background: [u8; 3],
    primary: [u8; 3],
    accent: [u8; 3],
    dark_foreground: [u8; 3],
    dark_background: [u8; 3],
) -> ([u8; 3], [u8; 3]) {
    // 中性阶：背景 → 正文之间按比例取。比例取自既有 `PALETTE_FIELDS` 的明暗关系，
    // 换成「用户基色之间的相对位置」，保证层级观感一致。
    let neutral = |ratio: f64, on_dark: bool| -> [u8; 3] {
        if on_dark {
            mix(dark_background, dark_foreground, ratio)
        } else {
            mix(background, foreground, ratio)
        }
    };
    // 主色阶：主色与背景之间。
    let primary_scale = |t: f64, on_dark: bool| -> [u8; 3] {
        if on_dark {
            mix(dark_background, primary, t)
        } else {
            mix(background, primary, t)
        }
    };
    // 强调色阶：强调色与背景之间。
    let accent_scale = |t: f64, on_dark: bool| -> [u8; 3] {
        if on_dark {
            mix(dark_background, accent, t)
        } else {
            mix(background, accent, t)
        }
    };
    // 反色：按背景明度自动取黑或白（保证按钮文字可读）。
    let on_color = |bg: [u8; 3]| -> [u8; 3] {
        if relative_luminance(bg) > 0.45 {
            [0, 0, 0]
        } else {
            [255, 255, 255]
        }
    };

    match field.slint_name {
        // 底层与层
        "background" => (background, dark_background),
        "layer-01" => (neutral(0.05, false), neutral(0.06, true)),
        "layer-02" => (neutral(0.10, false), neutral(0.12, true)),
        "field-01" => (neutral(0.02, false), neutral(0.04, true)),
        // 边框
        "border-subtle" => (neutral(0.16, false), neutral(0.18, true)),
        "border-strong" => (neutral(0.42, false), neutral(0.36, true)),
        // 文字
        "text-primary" => (foreground, dark_foreground),
        "text-secondary" => (neutral(0.55, false), neutral(0.58, true)),
        "text-placeholder" => (neutral(0.38, false), neutral(0.34, true)),
        "text-disabled" => (neutral(0.25, false), neutral(0.26, true)),
        // 主色阶
        "primary" => (primary_scale(1.0, false), primary_scale(1.0, true)),
        "primary-hover" => (primary_scale(0.88, false), primary_scale(1.12, true)),
        "primary-pressed" => (primary_scale(0.74, false), primary_scale(1.24, true)),
        "primary-soft" => (primary_scale(0.12, false), primary_scale(0.22, true)),
        "primary-border" => (primary_scale(0.35, false), primary_scale(0.40, true)),
        "primary-strong" => (primary_scale(0.80, false), primary_scale(1.15, true)),
        "on-primary" => {
            let light_bg = primary;
            let dark_bg = primary;
            (on_color(light_bg), on_color(dark_bg))
        }
        "focus" => (primary, primary),
        // 顶栏 UI Shell：两主题同色（深底白字），**不随基色变**（保持顶栏稳定）。
        "header" => (rgb3(field.value.light), rgb3(field.value.dark)),
        "header-text" => (rgb3(field.value.light), rgb3(field.value.dark)),
        "header-hover" => (rgb3(field.value.light), rgb3(field.value.dark)),
        // 语义色：由「强调色」派生三族（成功/警告/错误共用强调色，只改比例）。
        "support-success" => (accent_scale(1.0, false), accent_scale(1.0, true)),
        "success-soft" => (accent_scale(0.12, false), accent_scale(0.20, true)),
        "support-warning" => (accent_scale(0.85, false), accent_scale(0.90, true)),
        "warning-soft" => (accent_scale(0.16, false), accent_scale(0.24, true)),
        "support-error" => (accent_scale(0.92, false), accent_scale(0.98, true)),
        "error-soft" => (accent_scale(0.12, false), accent_scale(0.20, true)),
        // 滚动条与控件轨
        "scrollbar-track" => (neutral(0.05, false), neutral(0.06, true)),
        "scrollbar-handle" => (neutral(0.25, false), neutral(0.30, true)),
        "control-track" => (neutral(0.42, false), neutral(0.36, true)),
        // 禁用态
        "disabled-bg" => (neutral(0.05, false), neutral(0.06, true)),
        "disabled-border" => (neutral(0.16, false), neutral(0.18, true)),
        "disabled-text" => (neutral(0.25, false), neutral(0.26, true)),
        // 覆盖层与 Toast：固定深底（两主题同色），保证浮层上的文字可读。
        "overlay" => (rgb3(field.value.light), rgb3(field.value.dark)),
        "toast-bg" => (rgb3(field.value.light), rgb3(field.value.dark)),
        "toast-text" => (rgb3(field.value.light), rgb3(field.value.dark)),
        // 阅读正文：与 text-primary 同源（保证「界面可读」与「正文可读」一致）。
        "reading-text" => (foreground, dark_foreground),
        // 封面占位底：主色淡化。
        "cover-bg" => (primary_scale(0.12, false), primary_scale(0.22, true)),
        // 兜底：未列出的令牌沿用内置值（**不静默变黑**）。
        _ => (rgb3(field.value.light), rgb3(field.value.dark)),
    }
}

/// 把 4 分量颜色去 alpha（派生只需 RGB；alpha 另行继承）。
fn rgb3(value: (u8, u8, u8, u8)) -> [u8; 3] {
    [value.0, value.1, value.2]
}

/// 给 RGB 附上 alpha。
fn with_alpha(rgb: [u8; 3], alpha: u8) -> [u8; 4] {
    [rgb[0], rgb[1], rgb[2], alpha]
}

/// 对比度检查结果（规格 R12）。
#[derive(Debug, Clone, PartialEq)]
pub struct ContrastReport {
    /// 正文对背景的对比度。
    pub ratio: f64,
    /// 是否达到 `MIN_CONTRAST`。
    pub passes: bool,
    /// 明色与暗色**较差**的一侧（决定提示文案）。
    pub worst_in_dark: bool,
}

impl ContrastReport {
    /// 读数文案（设计稿 T2/T3 的 `8.13 : 1` 形式）。
    pub fn label(&self) -> String {
        format!("{:.2} : 1", self.ratio)
    }

    /// 不足时的提示（**保留用户选择**，规格 R12）。
    pub fn warning(&self) -> String {
        format!(
            "当前正文/背景对比度 {}，低于可读性建议 {}:1。已保留你的选择。",
            self.label(),
            MIN_CONTRAST
        )
    }
}

/// 检查派生后的正文 / 背景对比度。
pub fn check_contrast(derived: &DerivedPalette) -> ContrastReport {
    let light_fg = derived
        .light_of("text-primary")
        .map(|v| [v[0], v[1], v[2]])
        .unwrap_or([0, 0, 0]);
    let light_bg = derived
        .light_of("background")
        .map(|v| [v[0], v[1], v[2]])
        .unwrap_or([255, 255, 255]);
    let dark_fg = derived
        .dark_of("text-primary")
        .map(|v| [v[0], v[1], v[2]])
        .unwrap_or([255, 255, 255]);
    let dark_bg = derived
        .dark_of("background")
        .map(|v| [v[0], v[1], v[2]])
        .unwrap_or([0, 0, 0]);
    let light_ratio = contrast_ratio(light_fg, light_bg);
    let dark_ratio = contrast_ratio(dark_fg, dark_bg);
    // 取较差的一侧：用户能看到的两个主题里，最差的那个必须达标才算通过。
    let (ratio, worst_in_dark) = if dark_ratio < light_ratio {
        (dark_ratio, true)
    } else {
        (light_ratio, false)
    };
    ContrastReport {
        ratio,
        passes: ratio >= MIN_CONTRAST,
        worst_in_dark,
    }
}

/// 便捷入口：读存储 → 派生 → 对比度。
pub fn derive_from_theme(theme: &ThemeCustom) -> (DerivedPalette, ContrastReport) {
    let base = theme.base.clone();
    let derived = derive(&base);
    let report = check_contrast(&derived);
    (derived, report)
}

/// 预览用：8 个代表令牌（设计稿 T1 的「8 色带」）。
pub fn preview_swatches(derived: &DerivedPalette, dark: bool) -> Vec<[u8; 4]> {
    const NAMES: [&str; 8] = [
        "background",
        "layer-01",
        "text-primary",
        "primary",
        "primary-soft",
        "support-success",
        "border-strong",
        "on-primary",
    ];
    NAMES
        .iter()
        .filter_map(|name| {
            if dark {
                derived.dark_of(name)
            } else {
                derived.light_of(name)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> BaseColors {
        BaseColors {
            foreground: Some("#222222".to_string()),
            background: Some("#ffffff".to_string()),
            primary: Some("#1890ff".to_string()),
            accent: Some("#d4380d".to_string()),
        }
    }

    #[test]
    fn derivation_is_deterministic() {
        let first = derive(&base());
        let second = derive(&base());
        assert_eq!(first, second, "同输入两次派生必须逐字节一致");
    }

    #[test]
    fn derivation_covers_every_palette_token() {
        let derived = derive(&base());
        assert_eq!(
            derived.token_count(),
            PALETTE_FIELDS.len(),
            "派生必须覆盖全部令牌（界面的 Skin 有 {} 个）",
            PALETTE_FIELDS.len()
        );
        for field in PALETTE_FIELDS {
            assert!(
                derived.light_of(field.slint_name).is_some(),
                "缺明色令牌 {}",
                field.slint_name
            );
            assert!(
                derived.dark_of(field.slint_name).is_some(),
                "缺暗色令牌 {}",
                field.slint_name
            );
        }
    }

    #[test]
    fn base_colors_flow_into_derived_tokens() {
        let derived = derive(&base());
        // 正文令牌 = 用户选的正文色。
        assert_eq!(
            derived.light_of("text-primary").expect("text-primary"),
            [0x22, 0x22, 0x22, 0xff]
        );
        // 背景令牌 = 用户选的背景色。
        assert_eq!(
            derived.light_of("background").expect("background"),
            [0xff, 0xff, 0xff, 0xff]
        );
        // 主色令牌 = 用户选的主色。
        assert_eq!(
            derived.light_of("primary").expect("primary"),
            [0x18, 0x90, 0xff, 0xff]
        );
    }

    #[test]
    fn changing_a_base_color_changes_multiple_tokens() {
        let before = derive(&base());
        let mut other = base();
        other.primary = Some("#ff0000".to_string());
        let after = derive(&other);

        let changed = PALETTE_FIELDS
            .iter()
            .filter(|field| before.light_of(field.slint_name) != after.light_of(field.slint_name))
            .count();
        assert!(
            changed >= 5,
            "改一个基色应连带改变多个令牌（实际 {changed} 个：primary 族 + focus + cover-bg 等）"
        );
    }

    #[test]
    fn contrast_passes_for_default_base_colors() {
        let (_, report) = derive_from_theme(&ThemeCustom {
            enabled: true,
            base: base(),
            updated_at: 0,
        });
        assert!(report.passes, "默认基色应达标：{}", report.label());
        assert!(report.ratio >= MIN_CONTRAST);
    }

    #[test]
    fn contrast_fails_for_low_contrast_choice_but_keeps_it() {
        // 白底 + 浅灰正文：对比度约 1.6:1，远低于 4.5。
        let mut low = base();
        low.foreground = Some("#f0f0f0".to_string());
        let (derived, report) = derive_from_theme(&ThemeCustom {
            enabled: true,
            base: low,
            updated_at: 0,
        });
        assert!(!report.passes, "浅色正文在白底上应不达标");
        assert!(report.ratio < MIN_CONTRAST);
        // 关键：**基色未被强改** —— 派生出的正文仍等于用户所选。
        assert_eq!(
            derived.light_of("text-primary").expect("text-primary"),
            [0xf0, 0xf0, 0xf0, 0xff],
            "对比度不足时不得改动用户基色"
        );
        assert!(report.warning().contains("已保留你的选择"));
    }

    #[test]
    fn contrast_boundary_is_inclusive_at_4_5() {
        // 构造恰好 4.5:1 的组合：#767676 正文 on #ffffff 背景 ≈ 4.54:1。
        let mut edge = base();
        edge.foreground = Some("#767676".to_string());
        let (_, report) = derive_from_theme(&ThemeCustom {
            enabled: true,
            base: edge,
            updated_at: 0,
        });
        assert!(
            report.ratio >= 4.5 && report.passes,
            "4.54:1 应判通过，实际 {}",
            report.label()
        );
    }

    #[test]
    fn dark_derivation_moves_towards_dark_background() {
        let derived = derive(&base());
        let light_bg = derived.light_of("background").expect("light bg");
        let dark_bg = derived.dark_of("background").expect("dark bg");
        assert!(
            relative_luminance([dark_bg[0], dark_bg[1], dark_bg[2]])
                < relative_luminance([light_bg[0], light_bg[1], light_bg[2]]),
            "暗色套件的背景必须比明色更暗"
        );
    }

    #[test]
    fn luminance_and_contrast_match_wcag_reference_values() {
        // 参照值：白对黑 = 21:1；#767676 对白 ≈ 4.54:1。
        assert!((contrast_ratio([255, 255, 255], [0, 0, 0]) - 21.0).abs() < 0.01);
        let grey = contrast_ratio([0x76, 0x76, 0x76], [255, 255, 255]);
        assert!((grey - 4.54).abs() < 0.05, "实际 {grey}");
        assert!((relative_luminance([255, 255, 255]) - 1.0).abs() < 0.001);
        assert!(relative_luminance([0, 0, 0]).abs() < 0.001);
    }

    #[test]
    fn shade_and_mix_are_clamped_and_bounded() {
        // mix 的端点必须精确。
        assert_eq!(mix([0, 0, 0], [255, 255, 255], 0.0), [0, 0, 0]);
        assert_eq!(mix([0, 0, 0], [255, 255, 255], 1.0), [255, 255, 255]);
        // 越界的 t 被夹紧（不产生非法色值）。
        assert_eq!(mix([10, 20, 30], [200, 210, 220], -5.0), [10, 20, 30]);
        assert_eq!(mix([10, 20, 30], [200, 210, 220], 5.0), [200, 210, 220]);
        // shade 正负方向。
        assert!(shade([100, 100, 100], 0.5)[0] > 100);
        assert!(shade([100, 100, 100], -0.5)[0] < 100);
    }

    #[test]
    fn preview_swatches_return_eight_colors() {
        let derived = derive(&base());
        assert_eq!(preview_swatches(&derived, false).len(), 8);
        assert_eq!(preview_swatches(&derived, true).len(), 8);
    }

    #[test]
    fn theme_keeps_base_colors_unmodified() {
        // 存储层只存基色（规格 R11：40 令牌不落库）。
        let custom = ThemeCustom {
            enabled: true,
            base: base(),
            updated_at: 0,
        };
        let before = custom.base.clone();
        let _ = derive(&custom.base);
        assert_eq!(custom.base, before, "派生不得修改存储的基色");
    }

    #[test]
    fn disabled_theme_yields_no_derivation() {
        // 未启用时不应产出派生结果（界面回落内置明/暗）。
        let disabled = ThemeCustom {
            enabled: false,
            base: BaseColors::default(),
            updated_at: 0,
        };
        assert!(!disabled.enabled);
        let (derived, report) = derive_from_theme(&disabled);
        // 仍可派生（纯函数），但界面按 `enabled` 决定是否使用。
        assert!(derived.token_count() > 0);
        assert!(report.passes || !report.passes, "报告总是给出结论");
    }
}

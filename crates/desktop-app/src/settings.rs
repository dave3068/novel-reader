//! 全局阅读设置：字体、字号、行距、段距、首行缩进、边距、明亮/暗黑、手势/按钮模式、
//! 沉浸自动隐藏与书架视图/排序（v0.2.0）。
//!
//! 依据（开发方案 §2.1「全局阅读设置」、§8.3 第 4 条、「每本书独立阅读设置：V1 采用全局设置」）
//! 与设计 `docs/design/novel-reader-desktop-redesign.md` §5 S10：
//!
//! - **只有一套全局设置**：排版参数落在核心 `reader_settings` 单行（`id = 1`），
//!   交互模式 / 沉浸自动隐藏 / 书架视图 / 书架排序落在核心 `app_meta` 的键值行；
//!   本模块不提供任何按 `book_id` 的写入入口，也不存在每本书独立的设置体系；
//! - 尺寸/间距一律以**整数毫单位**存取（与 `LayoutSignature` 同口径）；显示口径与
//!   **控件口径**的换算只发生在本模块（`SettingsRowView`）；
//! - 桌面 Slint 逐行渲染不叠加段距、不做首行缩进、正文区留白由版式常量表达，
//!   因此段距 / 首行缩进 / 边距在桌面 V1 **只持久化、不参与排版**。本模块显式暴露
//!   `effective` 与 `note`，界面据此标注，绝不假装它们已经生效；
//! - 行距在**界面口径**上是「倍数 ×（1.0–2.4）」，与核心的「额外像素」互转只在本模块进行。

use std::path::Path;

use reading_storage::books;
use reading_storage::meta;
use reading_storage::settings::{self as core_settings, ReaderSettings};
use reading_storage::Storage;

use crate::layout::{MAX_FONT_SIZE_PX, MIN_FONT_SIZE_PX};
use crate::reflow::TypographySettings;

/// 交互（翻页）模式的元信息键（存核心 `app_meta`，不新增表/列）。
pub const INTERACTION_MODE_KEY: &str = "ui.interaction_mode";
/// 沉浸时自动隐藏工具条的元信息键。
pub const IMMERSIVE_AUTOHIDE_KEY: &str = "ui.immersive_autohide";
/// 书架视图（`table` / `cards`）的元信息键。
pub const SHELF_VIEW_KEY: &str = "ui.shelf.view";
/// 书架排序（`recent` / `title` / `imported`）的元信息键。
pub const SHELF_SORT_KEY: &str = "ui.shelf.sort";
/// 交互模式：按钮翻页。
pub const MODE_BUTTONS: &str = "buttons";
/// 交互模式：手势翻页。
pub const MODE_GESTURE: &str = "gesture";
/// 允许的交互模式。
pub const INTERACTION_MODES: &[&str] = &[MODE_BUTTONS, MODE_GESTURE];

/// 可选字体族（本机已安装的简体中文字体；`fc-list` 实测）。
pub const FONT_FAMILIES: &[&str] = &[
    "Noto Sans CJK SC",
    "Noto Serif CJK SC",
    "Noto Sans Mono CJK SC",
    "Microsoft YaHei",
    "SimSun",
];

/// 字段键：字体族。
pub const KEY_FONT_FAMILY: &str = "font-family";
/// 字段键：字号。
pub const KEY_FONT_SIZE: &str = "font-size";
/// 字段键：行距。
pub const KEY_LINE_SPACING: &str = "line-spacing";
/// 字段键：段距。
pub const KEY_PARAGRAPH_SPACING: &str = "paragraph-spacing";
/// 字段键：首行缩进。
pub const KEY_FIRST_LINE_INDENT: &str = "first-line-indent";
/// 字段键：左右边距。
pub const KEY_MARGIN_X: &str = "margin-x";
/// 字段键：上下边距。
pub const KEY_MARGIN_Y: &str = "margin-y";
/// 字段键：主题（明亮/暗黑/跟随系统）。
pub const KEY_THEME: &str = "theme";
/// 字段键：交互模式（手势/按钮）。
pub const KEY_INTERACTION_MODE: &str = "interaction-mode";
/// 字段键：沉浸时自动隐藏工具条。
pub const KEY_IMMERSIVE_AUTOHIDE: &str = "immersive-autohide";

/// 数值型字段的取值范围与步长（**控件口径**）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumericRange {
    /// 最小值（控件口径）。
    pub min: i32,
    /// 最大值（控件口径）。
    pub max: i32,
    /// 调整步长（控件口径）。
    pub step: i32,
}

/// 行距倍数的控件口径边界（1.0 – 2.4，单位：千分比）。
pub const LINE_SPACING_MULT_MIN: i32 = 1_000;
/// 行距倍数的控件口径上限（2.4）。
pub const LINE_SPACING_MULT_MAX: i32 = 2_400;
/// 行距倍数的步长（0.1）。
pub const LINE_SPACING_MULT_STEP: i32 = 100;

/// 字段类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    /// 枚举（取值清单 + 展示名）。
    Enum(&'static [&'static str]),
    /// 字体族（可选项来自 [`FONT_FAMILIES`]）。
    FontFamily,
    /// 数值（控件口径）。
    Numeric(NumericRange),
}

/// 设置字段元数据（界面只渲染，不判断语义）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingsField {
    /// 字段键（UI 与程序化驱动使用；不随语言变化）。
    pub key: &'static str,
    /// 中文名。
    pub label: &'static str,
    /// 类型与取值。
    pub kind: FieldKind,
    /// 是否参与桌面排版/渲染。
    pub effective: bool,
    /// 不参与排版时的原因（`effective == true` 时为空）。
    pub note: &'static str,
}

/// 段距/缩进/边距在桌面 V1 的解释（不假装生效）。
const DESKTOP_INERT_NOTE: &str = "只保存、不参与桌面排版";

/// 全局设置字段清单（顺序即界面顺序）。
pub const SETTINGS_FIELDS: &[SettingsField] = &[
    SettingsField {
        key: KEY_FONT_FAMILY,
        label: "字体族",
        kind: FieldKind::FontFamily,
        effective: true,
        note: "",
    },
    SettingsField {
        key: KEY_FONT_SIZE,
        label: "字号",
        kind: FieldKind::Numeric(NumericRange {
            min: MIN_FONT_SIZE_PX as i32,
            max: MAX_FONT_SIZE_PX as i32,
            step: 1,
        }),
        effective: true,
        note: "",
    },
    SettingsField {
        key: KEY_LINE_SPACING,
        label: "行距",
        kind: FieldKind::Numeric(NumericRange {
            min: LINE_SPACING_MULT_MIN,
            max: LINE_SPACING_MULT_MAX,
            step: LINE_SPACING_MULT_STEP,
        }),
        effective: true,
        note: "",
    },
    SettingsField {
        key: KEY_PARAGRAPH_SPACING,
        label: "段距",
        kind: FieldKind::Numeric(NumericRange {
            min: 0,
            max: 48,
            step: 2,
        }),
        effective: false,
        note: DESKTOP_INERT_NOTE,
    },
    SettingsField {
        key: KEY_FIRST_LINE_INDENT,
        label: "首行缩进",
        kind: FieldKind::Numeric(NumericRange {
            min: 0,
            max: 96,
            step: 4,
        }),
        effective: false,
        note: DESKTOP_INERT_NOTE,
    },
    SettingsField {
        key: KEY_MARGIN_X,
        label: "左右边距",
        kind: FieldKind::Numeric(NumericRange {
            min: 0,
            max: 96,
            step: 4,
        }),
        // 已接入桌面排版（LayoutMetrics 的正文留白），改动会触发重排。
        effective: true,
        note: "",
    },
    SettingsField {
        key: KEY_MARGIN_Y,
        label: "上下边距",
        kind: FieldKind::Numeric(NumericRange {
            min: 0,
            max: 96,
            step: 4,
        }),
        // 已接入桌面排版（LayoutMetrics 的正文留白），改动会触发重排。
        effective: true,
        note: "",
    },
    SettingsField {
        key: KEY_THEME,
        label: "主题",
        kind: FieldKind::Enum(&["system", "light", "dark"]),
        effective: true,
        note: "",
    },
    SettingsField {
        key: KEY_INTERACTION_MODE,
        label: "翻页模式",
        kind: FieldKind::Enum(INTERACTION_MODES),
        effective: true,
        note: "",
    },
    SettingsField {
        key: KEY_IMMERSIVE_AUTOHIDE,
        label: "沉浸时自动隐藏工具条",
        kind: FieldKind::Numeric(NumericRange {
            min: 0,
            max: 1,
            step: 1,
        }),
        effective: true,
        note: "",
    },
];

/// 设置行（可显示字段；文案与控件口径由 Rust 唯一生成）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SettingsRowView {
    /// 字段键。
    pub key: String,
    /// 中文名。
    pub label: String,
    /// 当前值（显示口径）。
    pub value: String,
    /// 说明（不参与排版的原因等；无说明时为空）。
    pub note: String,
    /// 是否参与桌面排版/渲染。
    pub effective: bool,
    /// 控件类型：0=循环字段 1=数值滑杆 2=枚举单选 3=布尔开关 4=只读文本 5=分组标题 6=动作行。
    pub kind: i32,
    /// 分组：0=排版 1=版式 2=外观 3=交互 4=数据。
    pub section: i32,
    /// 滑杆最小值（控件口径）。
    pub value_min: i32,
    /// 滑杆最大值（控件口径）。
    pub value_max: i32,
    /// 滑杆步长（控件口径）。
    pub value_step: i32,
    /// 滑杆当前值（控件口径）。
    pub value_current: i32,
    /// 枚举选项（显示名）。
    pub options: Vec<String>,
    /// 枚举当前下标。
    pub current_index: i32,
    /// 布尔开关当前值。
    pub checked: bool,
    /// 值文本用等宽字体（机器可核对信息）。
    pub mono: bool,
    /// 取值范围显示口径（如 `12 … 40`、`1.0 × … 2.4 ×`；空串表示不显示）。
    pub range_label: String,
    /// 行高（像素；与行内容一致：分组标题 48、滑杆行 72、其余 56）。
    pub row_height: u32,
    /// 行在内容坐标系中的 y 偏移（像素；Rust 计算，界面绝对定位）。
    pub offset_y: u32,
}

/// 设置来源（证据用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsOrigin {
    /// 库里已有设置行。
    Stored,
    /// 库中无设置行，使用核心默认值（不写库）。
    Default,
}

impl SettingsOrigin {
    /// 稳定短名。
    pub const fn as_str(self) -> &'static str {
        match self {
            SettingsOrigin::Stored => "stored",
            SettingsOrigin::Default => "default",
        }
    }
}

/// 设置读写失败原因（可读文本，不作为业务分支）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsError {
    /// 存储层失败。
    Storage(String),
    /// 字段不存在。
    UnknownField {
        /// 字段键。
        key: String,
    },
    /// 值非法（枚举越界 / 非数字 / 超范围）。
    InvalidValue {
        /// 字段键。
        key: String,
        /// 原因。
        detail: String,
    },
}

impl SettingsError {
    /// 可读说明。
    pub fn message(&self) -> String {
        match self {
            SettingsError::Storage(message) => format!("设置存取失败：{message}"),
            SettingsError::UnknownField { key } => format!("未知设置项：{key}"),
            SettingsError::InvalidValue { key, detail } => {
                format!("设置项 {key} 取值非法：{detail}")
            }
        }
    }
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for SettingsError {}

/// 主题是否深色（供投影层使用；`system` 在 V1 按明亮处理）。
pub fn theme_is_dark(theme: &str) -> bool {
    theme == core_settings::THEME_DARK
}

/// 全局设置（唯一一套；不含任何按书的覆盖）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalSettings {
    reader: ReaderSettings,
    interaction_mode: String,
    immersive_autohide: bool,
    origin: SettingsOrigin,
}

impl Default for GlobalSettings {
    fn default() -> Self {
        Self {
            reader: ReaderSettings::default(),
            interaction_mode: MODE_BUTTONS.to_string(),
            immersive_autohide: true,
            origin: SettingsOrigin::Default,
        }
    }
}

impl GlobalSettings {
    /// 底层设置记录（毫单位口径）。
    pub fn reader(&self) -> &ReaderSettings {
        &self.reader
    }

    /// 设置来源。
    pub fn origin(&self) -> SettingsOrigin {
        self.origin
    }

    /// 交互模式（`buttons` / `gesture`）。
    pub fn interaction_mode(&self) -> &str {
        &self.interaction_mode
    }

    /// 是否手势翻页模式。
    pub fn gesture_mode(&self) -> bool {
        self.interaction_mode == MODE_GESTURE
    }

    /// 沉浸时是否自动隐藏工具条。
    pub fn immersive_autohide(&self) -> bool {
        self.immersive_autohide
    }

    /// 是否深色主题。
    pub fn theme_dark(&self) -> bool {
        theme_is_dark(&self.reader.theme)
    }

    /// 主题显示名。
    pub fn theme_label(&self) -> String {
        theme_label(&self.reader.theme)
    }

    /// 交互模式显示名。
    pub fn interaction_label(&self) -> &'static str {
        mode_label(&self.interaction_mode)
    }

    /// 投影为测量/渲染共用的排版设置（毫单位直通，桌面不另造一套默认值）。
    pub fn typography(&self) -> TypographySettings {
        TypographySettings::from_reader_settings(&self.reader)
    }

    /// 控件的「行距倍数」口径（千分比；1.0 = 1000）。
    ///
    /// 行距在核心口径上是「额外像素」；界面口径是「倍数 =（字号 + 额外像素）÷ 字号」。
    pub fn line_spacing_mult_permille(&self) -> i32 {
        let font = self.reader.font_size_milli.max(1);
        let pitch = font.saturating_add(self.reader.line_spacing_milli.max(0));
        (((pitch as f64 / font as f64) * 1_000.0).round() as i32)
            .clamp(LINE_SPACING_MULT_MIN, LINE_SPACING_MULT_MAX)
    }

    /// 以「行距倍数」写入（控件口径；换算成核心的额外像素）。
    pub fn set_line_spacing_mult_permille(&mut self, permille: i32) -> Result<(), SettingsError> {
        let permille = permille.clamp(LINE_SPACING_MULT_MIN, LINE_SPACING_MULT_MAX);
        let font = self.reader.font_size_milli.max(1);
        let spacing = (font as f64 * (permille as f64 - 1_000.0) / 1_000.0).round() as i64;
        self.reader.line_spacing_milli = spacing.max(0);
        Ok(())
    }

    /// 控件口径的当前值（滑杆用）。
    pub fn control_value(&self, key: &str) -> i32 {
        match key {
            KEY_FONT_SIZE => (self.reader.font_size_milli / 1_000).max(0) as i32,
            KEY_LINE_SPACING => self.line_spacing_mult_permille(),
            KEY_PARAGRAPH_SPACING => (self.reader.paragraph_spacing_milli / 1_000).max(0) as i32,
            KEY_FIRST_LINE_INDENT => (self.reader.first_line_indent_milli / 1_000).max(0) as i32,
            KEY_MARGIN_X => (self.reader.margin_x_milli / 1_000).max(0) as i32,
            KEY_MARGIN_Y => (self.reader.margin_y_milli / 1_000).max(0) as i32,
            KEY_IMMERSIVE_AUTOHIDE => i32::from(self.immersive_autohide),
            _ => 0,
        }
    }

    /// 按控件值写入（滑杆 / 数值框共用；非法值夹紧到控件范围）。
    pub fn set_control_value(&mut self, key: &str, value: i32) -> Result<bool, SettingsError> {
        let Some(field) = Self::field(key) else {
            return Err(SettingsError::UnknownField {
                key: key.to_string(),
            });
        };
        let before = self.value_text(key);
        match key {
            KEY_LINE_SPACING => self.set_line_spacing_mult_permille(value)?,
            KEY_IMMERSIVE_AUTOHIDE => self.immersive_autohide = value != 0,
            KEY_FONT_SIZE | KEY_PARAGRAPH_SPACING | KEY_FIRST_LINE_INDENT | KEY_MARGIN_X
            | KEY_MARGIN_Y => {
                if let FieldKind::Numeric(range) = field.kind {
                    let clamped = value.clamp(range.min, range.max);
                    let slot = match key {
                        KEY_FONT_SIZE => &mut self.reader.font_size_milli,
                        KEY_PARAGRAPH_SPACING => &mut self.reader.paragraph_spacing_milli,
                        KEY_FIRST_LINE_INDENT => &mut self.reader.first_line_indent_milli,
                        KEY_MARGIN_X => &mut self.reader.margin_x_milli,
                        _ => &mut self.reader.margin_y_milli,
                    };
                    *slot = i64::from(clamped) * 1_000;
                }
            }
            _ => {
                return Err(SettingsError::UnknownField {
                    key: key.to_string(),
                })
            }
        }
        Ok(self.value_text(key) != before)
    }

    /// 枚举选项显示名（界面单选组）。
    pub fn option_labels(key: &str) -> Vec<String> {
        match key {
            KEY_THEME => vec![
                "跟随系统（V1 按明亮处理）".to_string(),
                "明亮".to_string(),
                "暗黑".to_string(),
            ],
            KEY_INTERACTION_MODE => vec!["按钮翻页".to_string(), "手势翻页".to_string()],
            // 字体族：下拉候选 = 本机可用字体（与书架「排序」下拉同一交互口径）。
            KEY_FONT_FAMILY => FONT_FAMILIES.iter().map(|name| name.to_string()).collect(),
            _ => Vec::new(),
        }
    }

    /// 枚举当前下标。
    pub fn option_index(&self, key: &str) -> i32 {
        match key {
            KEY_THEME => match self.reader.theme.as_str() {
                core_settings::THEME_LIGHT => 1,
                core_settings::THEME_DARK => 2,
                _ => 0,
            },
            KEY_INTERACTION_MODE => i32::from(self.gesture_mode()),
            KEY_FONT_FAMILY => FONT_FAMILIES
                .iter()
                .position(|name| *name == self.reader.font_family)
                .map_or(0, |index| index as i32),
            _ => 0,
        }
    }

    /// 按枚举下标写入（Rust 侧映射下标 → 内部取值）。
    pub fn set_option_index(&mut self, key: &str, index: i32) -> Result<(), SettingsError> {
        match key {
            KEY_THEME => {
                self.reader.theme = match index.clamp(0, 2) {
                    1 => core_settings::THEME_LIGHT,
                    2 => core_settings::THEME_DARK,
                    _ => core_settings::THEME_SYSTEM,
                }
                .to_string();
            }
            KEY_INTERACTION_MODE => {
                self.interaction_mode = if index.clamp(0, 1) == 1 {
                    MODE_GESTURE
                } else {
                    MODE_BUTTONS
                }
                .to_string();
            }
            KEY_FONT_FAMILY => {
                let Some(name) = FONT_FAMILIES.get(index.max(0) as usize) else {
                    return Err(SettingsError::UnknownField {
                        key: key.to_string(),
                    });
                };
                self.reader.font_family = (*name).to_string();
            }
            _ => {
                return Err(SettingsError::UnknownField {
                    key: key.to_string(),
                })
            }
        }
        Ok(())
    }

    /// 单字段的显示值（显示口径）。
    pub fn value_text(&self, key: &str) -> String {
        match key {
            KEY_FONT_FAMILY => self.reader.font_family.clone(),
            KEY_FONT_SIZE => format!(
                "{} px",
                (self.reader.font_size_milli / 1_000).max(0)
            ),
            KEY_LINE_SPACING => format!("{:.1} ×", self.line_spacing_mult_permille() as f64 / 1_000.0),
            KEY_PARAGRAPH_SPACING => format_milli_px(self.reader.paragraph_spacing_milli),
            KEY_FIRST_LINE_INDENT => format_milli_px(self.reader.first_line_indent_milli),
            KEY_MARGIN_X => format_milli_px(self.reader.margin_x_milli),
            KEY_MARGIN_Y => format_milli_px(self.reader.margin_y_milli),
            KEY_THEME => self.theme_label(),
            KEY_INTERACTION_MODE => self.interaction_label().to_string(),
            KEY_IMMERSIVE_AUTOHIDE => {
                if self.immersive_autohide {
                    "开".to_string()
                } else {
                    "关".to_string()
                }
            }
            _ => String::new(),
        }
    }

    /// 按字段键取字段元数据。
    pub fn field(key: &str) -> Option<&'static SettingsField> {
        SETTINGS_FIELDS.iter().find(|field| field.key == key)
    }

    /// 调整一个字段（`delta` 为**控件口径**步长个数）；返回是否真的发生变化。
    pub fn adjust(&mut self, key: &str, delta: i64) -> Result<bool, SettingsError> {
        let field = Self::field(key).ok_or_else(|| SettingsError::UnknownField {
            key: key.to_string(),
        })?;
        let before = self.value_text(key);
        match (key, field.kind) {
            (KEY_FONT_FAMILY, _) | (_, FieldKind::FontFamily) => self.cycle_font_family(delta),
            (KEY_THEME, FieldKind::Enum(values)) => {
                self.reader.theme = cycle(values, &self.reader.theme, delta)
            }
            (KEY_INTERACTION_MODE, FieldKind::Enum(values)) => {
                self.interaction_mode = cycle(values, &self.interaction_mode, delta)
            }
            (KEY_IMMERSIVE_AUTOHIDE, _) => {
                if delta % 2 != 0 {
                    self.immersive_autohide = !self.immersive_autohide;
                }
            }
            (_, FieldKind::Enum(_)) => {
                return Err(SettingsError::UnknownField {
                    key: key.to_string(),
                })
            }
            (KEY_LINE_SPACING, FieldKind::Numeric(range)) => {
                let next = (self.line_spacing_mult_permille() + (delta as i32) * range.step)
                    .clamp(range.min, range.max);
                self.set_line_spacing_mult_permille(next)?;
            }
            (_, FieldKind::Numeric(range)) => {
                let current = self.control_value(key);
                let next = (current + (delta as i32) * range.step).clamp(range.min, range.max);
                self.set_control_value(key, next)?;
            }
        }
        Ok(self.value_text(key) != before)
    }

    /// 直接设置字段取值（程序化驱动与界面输入共用同一入口）。
    pub fn set_text(&mut self, key: &str, value: &str) -> Result<(), SettingsError> {
        let field = Self::field(key).ok_or_else(|| SettingsError::UnknownField {
            key: key.to_string(),
        })?;
        let value = value.trim();
        match field.kind {
            FieldKind::FontFamily => {
                if value.is_empty() {
                    return Err(SettingsError::InvalidValue {
                        key: key.to_string(),
                        detail: "字体族不得为空".to_string(),
                    });
                }
                self.reader.font_family = value.to_string();
            }
            FieldKind::Enum(values) => {
                if !values.contains(&value) {
                    return Err(SettingsError::InvalidValue {
                        key: key.to_string(),
                        detail: format!("允许取值 {values:?}"),
                    });
                }
                match key {
                    KEY_THEME => self.reader.theme = value.to_string(),
                    KEY_INTERACTION_MODE => self.interaction_mode = value.to_string(),
                    _ => {
                        return Err(SettingsError::UnknownField {
                            key: key.to_string(),
                        })
                    }
                }
            }
            FieldKind::Numeric(range) => {
                let number: i32 = value.parse().map_err(|_| SettingsError::InvalidValue {
                    key: key.to_string(),
                    detail: format!("`{value}` 不是整数"),
                })?;
                if number < range.min || number > range.max {
                    return Err(SettingsError::InvalidValue {
                        key: key.to_string(),
                        detail: format!("允许范围 {}..={}", range.min, range.max),
                    });
                }
                self.set_control_value(key, number)?;
            }
        }
        Ok(())
    }

    /// 稳定一行（日志与证据用；全部设置值一次打印）。
    pub fn state_log(&self, changes: usize) -> String {
        format!(
            "NR-SETTINGS changes={} origin={} font_family={} font_size_milli={} \
             line_spacing_milli={} paragraph_spacing_milli={} first_line_indent_milli={} \
             margin_x_milli={} margin_y_milli={} theme={} interaction={} autohide={} dark={}",
            changes,
            self.origin.as_str(),
            self.reader.font_family.replace(' ', "_"),
            self.reader.font_size_milli,
            self.reader.line_spacing_milli,
            self.reader.paragraph_spacing_milli,
            self.reader.first_line_indent_milli,
            self.reader.margin_x_milli,
            self.reader.margin_y_milli,
            self.reader.theme,
            self.interaction_mode,
            u8::from(self.immersive_autohide),
            u8::from(self.theme_dark()),
        )
    }

    fn cycle_font_family(&mut self, delta: i64) {
        let options: Vec<&str> = FONT_FAMILIES.to_vec();
        self.reader.font_family = cycle(&options, &self.reader.font_family, delta);
    }
}

/// 枚举循环取值（`delta` 为 0 时保持不变；未知当前值从第 0 项开始）。
fn cycle(values: &[&str], current: &str, delta: i64) -> String {
    if values.is_empty() {
        return current.to_string();
    }
    let count = values.len() as i64;
    let index = values
        .iter()
        .position(|value| *value == current)
        .map(|index| index as i64)
        .unwrap_or(if delta >= 0 { -1 } else { 0 });
    let next = (index + delta).rem_euclid(count);
    values[next as usize].to_string()
}

fn theme_label(theme: &str) -> String {
    match theme {
        core_settings::THEME_LIGHT => "明亮".to_string(),
        core_settings::THEME_DARK => "暗黑".to_string(),
        _ => "跟随系统（V1 按明亮处理）".to_string(),
    }
}

fn mode_label(mode: &str) -> &'static str {
    if mode == MODE_GESTURE {
        "手势翻页"
    } else {
        "按钮翻页"
    }
}

/// 取值范围显示口径（滑杆下方的边界值；与控件口径一致，但换算成界面语言）。
fn range_label(key: &str) -> String {
    let Some(range) = GlobalSettings::field(key).and_then(|field| match field.kind {
        FieldKind::Numeric(range) => Some(range),
        _ => None,
    }) else {
        return String::new();
    };
    if key == KEY_LINE_SPACING {
        return format!(
            "{:.1} × … {:.1} ×",
            range.min as f64 / 1_000.0,
            range.max as f64 / 1_000.0
        );
    }
    format!("{} … {}", range.min, range.max)
}

/// 毫像素 → 显示文本（保留一位小数）。
fn format_milli_px(milli: i64) -> String {
    let value = milli.max(0);
    if value % 1_000 == 0 {
        format!("{} px", value / 1_000)
    } else {
        format!("{:.1} px", value as f64 / 1_000.0)
    }
}

/// 打开库（自动迁移）。
fn open_storage(db_path: &Path) -> Result<Storage, SettingsError> {
    let mut storage = Storage::open(db_path)
        .map_err(|error| SettingsError::Storage(format!("{}（{error}）", db_path.display())))?;
    storage
        .migrate()
        .map_err(|error| SettingsError::Storage(format!("迁移失败：{error}")))?;
    Ok(storage)
}

/// 读取全局设置（无设置行时为默认值；不写库）。
pub fn load(db_path: &Path) -> Result<GlobalSettings, SettingsError> {
    let storage = open_storage(db_path)?;
    let stored_rows = core_settings::settings_row_count(storage.conn())
        .map_err(|error| SettingsError::Storage(error.to_string()))?;
    let reader = core_settings::load_settings(storage.conn())
        .map_err(|error| SettingsError::Storage(error.to_string()))?;
    let interaction_mode = meta::get(storage.conn(), INTERACTION_MODE_KEY)
        .map_err(|error| SettingsError::Storage(error.to_string()))?
        .filter(|mode| INTERACTION_MODES.contains(&mode.as_str()))
        .unwrap_or_else(|| MODE_BUTTONS.to_string());
    let immersive_autohide = meta::get(storage.conn(), IMMERSIVE_AUTOHIDE_KEY)
        .map_err(|error| SettingsError::Storage(error.to_string()))?
        .map(|value| value != "0" && value.to_lowercase() != "false")
        .unwrap_or(true);
    Ok(GlobalSettings {
        reader,
        interaction_mode,
        immersive_autohide,
        origin: if stored_rows > 0 {
            SettingsOrigin::Stored
        } else {
            SettingsOrigin::Default
        },
    })
}

/// 保存全局设置（幂等 upsert；写入前由核心 validate 校验）。
pub fn save(db_path: &Path, settings: &GlobalSettings, now: i64) -> Result<(), SettingsError> {
    let storage = open_storage(db_path)?;
    core_settings::save_settings(storage.conn(), &settings.reader, now)
        .map_err(|error| SettingsError::Storage(error.to_string()))?;
    meta::set(
        storage.conn(),
        INTERACTION_MODE_KEY,
        &settings.interaction_mode,
        now,
    )
    .map_err(|error| SettingsError::Storage(error.to_string()))?;
    meta::set(
        storage.conn(),
        IMMERSIVE_AUTOHIDE_KEY,
        if settings.immersive_autohide { "1" } else { "0" },
        now,
    )
    .map_err(|error| SettingsError::Storage(error.to_string()))?;
    Ok(())
}

/// 读取书架视图（`0` = 表格，`1` = 卡片）。
pub fn load_shelf_view(db_path: &Path) -> i32 {
    let Ok(storage) = open_storage(db_path) else {
        return 0;
    };
    match meta::get(storage.conn(), SHELF_VIEW_KEY) {
        Ok(Some(value)) if value == "cards" => 1,
        _ => 0,
    }
}

/// 保存书架视图。
pub fn save_shelf_view(db_path: &Path, view: i32, now: i64) -> Result<(), SettingsError> {
    let storage = open_storage(db_path)?;
    meta::set(
        storage.conn(),
        SHELF_VIEW_KEY,
        if view == 1 { "cards" } else { "table" },
        now,
    )
    .map_err(|error| SettingsError::Storage(error.to_string()))?;
    Ok(())
}

/// 读取书架排序（0=最近阅读 1=书名 2=导入时间）。
pub fn load_shelf_sort(db_path: &Path) -> i32 {
    let Ok(storage) = open_storage(db_path) else {
        return 0;
    };
    match meta::get(storage.conn(), SHELF_SORT_KEY) {
        Ok(Some(value)) if value == "title" => crate::app_state::SORT_TITLE,
        Ok(Some(value)) if value == "imported" => crate::app_state::SORT_IMPORTED,
        _ => crate::app_state::SORT_RECENT,
    }
}

/// 保存书架排序。
pub fn save_shelf_sort(db_path: &Path, sort: i32, now: i64) -> Result<(), SettingsError> {
    let storage = open_storage(db_path)?;
    let value = match sort {
        crate::app_state::SORT_TITLE => "title",
        crate::app_state::SORT_IMPORTED => "imported",
        _ => "recent",
    };
    meta::set(storage.conn(), SHELF_SORT_KEY, value, now)
        .map_err(|error| SettingsError::Storage(error.to_string()))?;
    Ok(())
}

/// 组装设置页表单行（分组标题 + 字段行 + 数据行；文案与控件口径由本模块唯一生成）。
///
/// `data_path` 为数据库位置（mono 行）；`counts` 为（书架记录数，已移出数）。
pub fn display_rows(
    settings: &GlobalSettings,
    data_path: &str,
    counts: (i64, i64),
) -> Vec<SettingsRowView> {
    let mut rows: Vec<SettingsRowView> = Vec::new();
    // 分组标题（kind 5）：**标题只放 label**，说明不进标题行。
    //
    // 2026-10-04 修复：原先是 `label + "　" + note` 拼成一行，窄窗（最小 560）必然
    // 超出窗口被裁切，且与设计稿 §263「顶部一条解释」不符 —— 说明应当是**独立信息块**。
    let header = |rows: &mut Vec<SettingsRowView>, label: &str, _note: &str| {
        rows.push(SettingsRowView {
            label: label.to_string(),
            note: String::new(),
            kind: 5,
            section: -1,
            ..SettingsRowView::default()
        });
    };
    // 分组说明块（kind 7）：独立成块，供 `.slint` 用 `InlineNotice` 渲染（自动换行、不截断）。
    let section_note = |rows: &mut Vec<SettingsRowView>, section: i32, text: &str| {
        rows.push(SettingsRowView {
            label: String::new(),
            note: text.to_string(),
            kind: 7,
            section,
            ..SettingsRowView::default()
        });
    };
    let row = |key: &str, kind: i32, section: i32| SettingsRowView {
        key: key.to_string(),
        label: field_label(key),
        value: settings.value_text(key),
        note: field_note(key),
        effective: field_effective(key),
        kind,
        section,
        value_min: field_range(key).map_or(0, |range| range.min),
        value_max: field_range(key).map_or(0, |range| range.max),
        value_step: field_range(key).map_or(0, |range| range.step),
        value_current: settings.control_value(key),
        options: GlobalSettings::option_labels(key),
        current_index: settings.option_index(key),
        checked: settings.control_value(key) != 0,
        mono: false,
        range_label: range_label(key),
        // 行高与 y 偏移在 `display_rows` 末尾统一计算。
        row_height: 0,
        offset_y: 0,
    };

    // ---- 排版 ----
    header(&mut rows, "排版", "");
    rows.push(row(KEY_FONT_FAMILY, 0, 0));
    rows.push(row(KEY_FONT_SIZE, 1, 0));
    rows.push(row(KEY_LINE_SPACING, 1, 0));
    // ---- 版式 ----
    // 分两组：**边距**（左右/上下）已接入桌面排版（2026-10-03，改动即时生效并触发重排）；
    // **段距/首行缩进**仍是「只保存不生效」——Slint 逐行渲染不叠加段距、也不做首行缩进
    // （见设计稿 O2）。保持如实标注，不给用户「改了却没反应」的假入口。
    //
    // 方案 A（2026-10-04 主人裁定）：**仅当分组内确有未生效项时**才显示说明块；
    // 「边距」这一组全部生效，故不给它挂说明块。
    header(&mut rows, "版式 · 边距", "");
    rows.push(row(KEY_MARGIN_X, 1, 1));
    rows.push(row(KEY_MARGIN_Y, 1, 1));
    // 说明块承担**唯一**的说明职责（2026-10-04 主人裁定）：原先字段级 note 也写
    // 「只保存、不参与桌面排版」，与说明块文案重复、观感冗余，故字段 note 一律留空。
    header(&mut rows, "版式 · 段落", "");
    section_note(
        &mut rows,
        1,
        "段距与首行缩进暂不生效：Slint 逐行渲染不叠加段距、也不做首行缩进。",
    );
    for key in [KEY_PARAGRAPH_SPACING, KEY_FIRST_LINE_INDENT] {
        let mut row = row(key, 4, 1);
        // 清空字段级说明，避免与上方说明块重复（说明只由说明块承担）。
        row.note = String::new();
        rows.push(row);
    }
    // ---- 外观 ----
    header(&mut rows, "外观", "");
    rows.push(row(KEY_THEME, 2, 2));
    // ---- 交互 ----
    header(&mut rows, "交互", "");
    rows.push(row(KEY_INTERACTION_MODE, 2, 3));
    rows.push(row(KEY_IMMERSIVE_AUTOHIDE, 3, 3));
    // ---- 数据 ----
    header(&mut rows, "数据", "");
    rows.push(SettingsRowView {
        key: "data-books".to_string(),
        label: "书架记录数".to_string(),
        value: format!("{} 本", counts.0 - counts.1),
        kind: 4,
        section: 4,
        ..SettingsRowView::default()
    });
    rows.push(SettingsRowView {
        key: "data-removed".to_string(),
        label: "已移出".to_string(),
        value: format!("{} 本", counts.1),
        kind: 4,
        section: 4,
        ..SettingsRowView::default()
    });
    rows.push(SettingsRowView {
        key: "data-path".to_string(),
        label: "数据库位置".to_string(),
        value: data_path.to_string(),
        note: "备份建议：复制该文件即可完整保留书架、进度与书签".to_string(),
        kind: 4,
        section: 4,
        mono: true,
        ..SettingsRowView::default()
    });
    rows.push(SettingsRowView {
        key: "clear-index".to_string(),
        label: "清理章节索引与分页缓存…".to_string(),
        note: "删除可重建的派生数据（章节、分页缓存）；书籍、进度与书签保留".to_string(),
        kind: 6,
        section: 4,
        ..SettingsRowView::default()
    });
    // 逐行计算 y 偏移与行高：界面按此**绝对定位**每一行（不依赖布局引擎）。
    let mut offset = 0u32;
    for row in rows.iter_mut() {
        offset += 8;
        row.offset_y = offset;
        row.row_height = row_height(row.kind);
        offset += row.row_height;
    }
    rows
}

fn field_label(key: &str) -> String {
    GlobalSettings::field(key)
        .map(|field| field.label.to_string())
        .unwrap_or_default()
}

fn field_note(key: &str) -> String {
    GlobalSettings::field(key)
        .map(|field| field.note.to_string())
        .unwrap_or_default()
}

fn field_effective(key: &str) -> bool {
    GlobalSettings::field(key).is_some_and(|field| field.effective)
}

fn field_range(key: &str) -> Option<NumericRange> {
    match GlobalSettings::field(key).map(|field| field.kind) {
        Some(FieldKind::Numeric(range)) => Some(range),
        _ => None,
    }
}

/// 单行高度（像素；与 `.slint` 行内容一致）。
///
/// 分组标题 48；数值滑杆行 48 + 24；其余单行 48 + 8。
/// 说明块（`kind = 7`）的**实测高度**（像素），按 Slint 回报值覆盖估算值。
///
/// 为什么需要：说明块高度取决于**实际换行行数**，而换行行数又取决于窗口宽度。
/// Rust 侧无法预知 Slint 的真实排版结果（它有自己的换行与标点禁则规则），
/// 固定估算值会在「宽度足够、只占一行」时仍占两三行的高度 → 下方留大片空白。
/// 故由 `SettingsPage` 用 `Text.preferred-height` 实测后回报（2026-10-04 主人要求）。

/// 逐行求和；`note_heights` 为 `kind = 7` 行的实测高度（键为行序号）。
pub fn content_height_with(rows: &[SettingsRowView], note_heights: &std::collections::HashMap<usize, u32>) -> u32 {
    let mut total: u32 = 0;
    for (index, row) in rows.iter().enumerate() {
        total += 8 + note_heights.get(&index).copied().unwrap_or_else(|| row_height(row.kind));
    }
    total + 24
}

pub fn row_height(kind: i32) -> u32 {
    match kind {
        5 => 48,
        1 => 48 + 24,
        // 分组说明块：**首次进入页面时**（Slint 尚未回报实测高度）使用的紧估算。
        // 原先取 48+24+16 = 88（按「窄窗两行」估），而常见情形只占一行 ≈ 42，
        // 于是块下方多出约 46 像素空白（2026-10-04 主人实测反馈）。
        // 现按**单行**取值；Slint 回报实测高度后由 `content_height_with` 覆盖，
        // 窄窗换行变多时会自动增高。此值仍须 ≥ 单行真实高度，否则首帧会截字。
        7 => 12 + 18 + 24,
        _ => 48 + 8,
    }
}

/// 设置页内容高度（像素）：与 `app.slint` 的行高公式**逐项一致**。
///
/// 行高口径（`.slint`）：分组标题 48；数值滑杆行 48 + 24；其余单行 48 + 8；
/// 行间距 8；末尾留白 24。Rust 侧显式求和可避免依赖布局 `preferred-height`
/// 的运行时行为（曾导致「无内容可滚」：滚动条拖不动、滚轮无效）。
pub fn content_height(rows: &[SettingsRowView]) -> u32 {
    // 布局为「每行之前一个 8 像素间距」+ 末尾 24 像素留白（与 `.slint` 的 spacing/尾部占位一致）。
    let mut total: u32 = 0;
    for row in rows {
        total += 8 + row_height(row.kind);
    }
    total + 24
}

/// 组装设置页投影，**并采用 Slint 实测的说明块高度**重算内容高度。
///
/// `note_heights`：行序号 → 实测高度（仅 `kind = 7` 会被查到）。
pub fn view_with_note_heights(
    db_path: &Path,
    settings: &GlobalSettings,
    note_heights: &std::collections::HashMap<usize, u32>,
) -> crate::app_state::SettingsView {
    let mut view = view_for(db_path, settings);
    if !note_heights.is_empty() {
        view.content_height = content_height_with(&view.rows, note_heights);
    }
    view
}

/// 组装设置页投影（含数据行；供启动期与设置变更共用）。
pub fn view_for(db_path: &Path, settings: &GlobalSettings) -> crate::app_state::SettingsView {
    let counts = storage_counts(db_path);
    let rows = display_rows(settings, &db_path.display().to_string(), counts);
    crate::app_state::SettingsView {
        content_height: content_height(&rows),
        scroll_y: 0,
        viewport_height: 0,
        select_open: false,
        select_y: 0,
        select_key: String::new(),
        select_index: 0,
        select_options: Vec::new(),
        rows,
        note: "修改立即生效并保存到本地库（library.db）".to_string(),
        data_path: db_path.display().to_string(),
        theme: settings.reader().theme.clone(),
        interaction_mode: settings.interaction_mode().to_string(),
        immersive_autohide: settings.immersive_autohide(),
        persisted: matches!(settings.origin(), SettingsOrigin::Stored),
    }
}

/// 书架记录数 / 已移出数（读库；失败时按 0 处理，不假装有数据）。
pub fn storage_counts(db_path: &Path) -> (i64, i64) {
    let Ok(storage) = open_storage(db_path) else {
        return (0, 0);
    };
    let total = books::count_books(storage.conn()).unwrap_or(0);
    let removed = books::removed_book_count(storage.conn()).unwrap_or(0);
    (total, removed)
}

#[cfg(test)]
mod tests {
    /// 说明块高度必须**按内容自适应**（2026-10-04 主人要求）。
    ///
    /// 缺陷背景：原先`row_height(7)` 是**固定估算值**，而说明块的真实高度取决于
    /// **实际换行行数**（换行行数又取决于窗口宽度）—— 宽度足够只占一行时，
    /// 固定估算仍占两三行的高度，下方留大片空白。
    ///
    /// 本测试锁定 `content_height_with` 的三条语义：
    /// ①实测高度**真的被采用**（不是仍用固定估算）；
    /// ②只影响对应行（其余行高不变）；
    /// ③行数越多 → 总高度越大（单调）。
    #[test]
    fn note_block_height_follows_measured_content() {
        let settings = GlobalSettings::default();
        let rows = display_rows(&settings, "/tmp/library.db", (3, 1));
        let baseline = content_height(&rows);

        // 找到唯一的说明块行（kind = 7）
        let note_index = rows
            .iter()
            .position(|row| row.kind == 7)
            .expect("应存在说明块行");
        // 首次进入页面时的**紧估算**（2026-10-04 改为单行口径：12顶 + 18行 + 24 底 = 54）。
        // 不再按「窄窗两行」估 88，否则一行内容时块下方会多出大片空白。
        let estimated = row_height(7);
        assert_eq!(
            estimated, 54,
            "紧估算应等于单行真实高度（顶 space-12 + 行高 + 底 space-24）"
        );

        // ① 实测值**真的**被采用：与估算不同则总高度应按差值调整。
        let mut one_line = std::collections::HashMap::new();
        one_line.insert(note_index, 54u32);
        let h_one = content_height_with(&rows, &one_line);
        assert_eq!(
            h_one, baseline,
            "实测值等于紧估算时，总高度应与基线一致"
        );

        // 实测比估算矮（更紧）时应按差值收窄，而不是仍占估算高度。
        let mut tighter = std::collections::HashMap::new();
        tighter.insert(note_index, 30u32);
        assert_eq!(
            content_height_with(&rows, &tighter),
            baseline - (estimated - 30),
            "实测更矮时应按差值收窄"
        );

        // ② 只影响对应行：另一行用不同实测值时，总高度差应恰好等于差值。
        let mut other_index = rows
            .iter()
            .enumerate()
            .find(|(index, _row)| *index != note_index)
            .map(|(index, _)| index)
            .expect("应还有其它行");
        let mut two_rows = std::collections::HashMap::new();
        two_rows.insert(other_index, 999u32);
        let h_other = content_height_with(&rows, &two_rows);
        assert_eq!(
            h_other,
            baseline + 999 - row_height(rows[other_index].kind),
            "实测高度只应影响对应行"
        );

        // ③ 单调：行数越多总高度越大。
        let mut tall = std::collections::HashMap::new();
        tall.insert(note_index, 200u32);
        assert!(content_height_with(&rows, &tall) > h_one);
    }

    /// 分组标题**不得拼接说明**；说明必须是独立的说明块行（kind 7）。
    ///
    /// 2026-10-04 修复：原先 `SettingsRowView { label, note, kind: 5 }` 的渲染层
    /// 拼成 `label + "　" + note` 一行，窄窗（最小宽度）必然被裁切，且与设计稿
    /// §263「顶部一条解释」不符。这是**行为守卫**：一旦有人把说明塞回标题行，本测试必须失败。
    #[test]
    fn section_header_never_inlines_note_and_notes_become_blocks() {
        let settings = GlobalSettings::default();
        let rows = display_rows(&settings, "/tmp/library.db", (3, 1));

        // ① 所有分组标题行（kind 5）的 note 必须为空 —— 说明不得进标题。
        for row in rows.iter().filter(|row| row.kind == 5) {
            assert!(
                row.note.is_empty(),
                "分组标题「{}」不得携带说明（应另起说明块）：{}",
                row.label,
                row.note
            );
        }

        // ② 必须存在独立说明块行，且内容来自段距/缩进那条说明。
        let blocks: Vec<&SettingsRowView> = rows.iter().filter(|row| row.kind == 7).collect();
        assert!(
            !blocks.is_empty(),
            "应至少有一个 kind=7 说明块（段距/首行缩进的「只保存不生效」说明）"
        );
        assert!(
            blocks.iter().any(|row| row.note.contains("首行缩进")),
            "说明块内容应包含「首行缩进」的说明，实际：{:?}",
            blocks.iter().map(|r| r.note.clone()).collect::<Vec<_>>()
        );

        // ③ 方案 A：**只有确有未生效项的分组才挂说明块** —— 「排版」等全生效分组不应有。
        //    说明块只应出现在「版式（部分暂未生效）」之后。
        let first_block = rows.iter().position(|row| row.kind == 7).expect("已有说明块");
        let headers_before: Vec<&str> = rows[..first_block]
            .iter()
            .filter(|row| row.kind == 5)
            .map(|row| row.label.as_str())
            .collect();
        assert_eq!(
            headers_before.last().copied(),
            Some("版式 · 段落"),
            "说明块应位于「版式 · 段落」分组内（该分组含未生效项）"
        );
        // 「排版」与「版式 · 边距」两组全部生效，其标题行**之后紧跟的应是设置项**，
        // 而不是说明块（即说明块不得出现在这两组之下）。
        let notice_sections: Vec<i32> = rows
            .iter()
            .filter(|row| row.kind == 7)
            .map(|row| row.section)
            .collect();
        assert!(
            notice_sections.iter().all(|section| *section == 1),
            "说明块只应归属「版式」分组（section=1），实际 section={notice_sections:?}"
        );
        //分组名不得重复（曾出现两个「版式」）
        let mut names: Vec<&str> = rows.iter().filter(|r| r.kind == 5).map(|r| r.label.as_str()).collect();
        let before = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), before, "分组标题不得重名：{names:?}");

        // ④ 说明块高度是**紧估算**（单行口径），实测值会覆盖它。
        //    2026-10-04 主人反馈「块下方空白过多」后由 88 改为单行紧值。
        assert_eq!(
            row_height(7),
            12 + 18 + 24,
            "kind=7 应取单行紧估算（顶+行高+底），而非按多行的保守值"
        );
        // 精简后文案在常见窗口宽度下均为 1 行，故紧估算必须够容下它。
        let block_note = blocks
            .iter()
            .find(|row| row.note.contains("首行缩进"))
            .expect("应找到说明块");
        assert!(
            block_note.note.chars().count() <= 40,
            "说明块文案应精简到一行可容下（当前 {} 字）：{}",
            block_note.note.chars().count(),
            block_note.note
        );
        // 「版式 · 段落」两项的字段级 note 必须为空 —— 该组说明已由说明块承担一次，
        // 避免同一件事说两遍（2026-10-04 主人裁定）。**只限定这一组**：
        // 其它 kind=4 行（如「数据库位置」）的 note 是各自独立信息，不在此列。
        for row in rows
            .iter()
            .filter(|row| row.kind == 4 && (row.label == "段距" || row.label == "首行缩进"))
        {
            assert!(
                row.note.is_empty(),
                "「{}」的字段级说明应留空（说明已由说明块承担）：{}",
                row.label,
                row.note
            );
        }
    }

    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("nr-dt4-settings-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("创建临时目录");
        dir
    }

    #[test]
    fn field_catalog_covers_every_required_setting_once() {
        let keys: Vec<&str> = SETTINGS_FIELDS.iter().map(|field| field.key).collect();
        let mut unique = keys.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), keys.len(), "字段键必须唯一: {keys:?}");
        for required in [
            KEY_FONT_FAMILY,
            KEY_FONT_SIZE,
            KEY_LINE_SPACING,
            KEY_PARAGRAPH_SPACING,
            KEY_FIRST_LINE_INDENT,
            KEY_MARGIN_X,
            KEY_MARGIN_Y,
            KEY_THEME,
            KEY_INTERACTION_MODE,
            KEY_IMMERSIVE_AUTOHIDE,
        ] {
            assert!(keys.contains(&required), "缺少必需设置项 {required}");
        }
        // 未生效的字段必须给出原因，生效的字段不得带原因（不假装生效）。
        for field in SETTINGS_FIELDS {
            if field.effective {
                assert!(
                    field.note.is_empty(),
                    "{} 生效时不应带未生效说明",
                    field.key
                );
            } else {
                assert!(!field.note.is_empty(), "{} 必须说明未生效原因", field.key);
            }
        }
        assert!(FONT_FAMILIES.contains(&core_settings::DEFAULT_FONT_FAMILY));
    }

    #[test]
    fn defaults_come_from_core_settings_row_and_are_projected() {
        let settings = GlobalSettings::default();
        let reader = ReaderSettings::default();
        assert_eq!(settings.reader(), &reader, "桌面不得另造一套默认值");
        assert_eq!(settings.origin(), SettingsOrigin::Default);
        assert!(!settings.theme_dark());
        assert!(!settings.gesture_mode());
        assert!(settings.immersive_autohide(), "沉浸自动隐藏默认开");
        assert_eq!(settings.value_text(KEY_FONT_SIZE), "18 px");
        // 行距显示口径是倍数（18px 字号 + 默认额外像素）。
        assert!(settings.value_text(KEY_LINE_SPACING).ends_with('×'));
        assert_eq!(settings.typography().font_family, reader.font_family);
    }

    #[test]
    fn adjust_clamps_and_cycles_without_leaving_valid_range() {
        let mut settings = GlobalSettings::default();
        // 字号：步进并夹紧在 [MIN, MAX]。
        assert!(settings.adjust(KEY_FONT_SIZE, 4).expect("增大字号"));
        assert_eq!(settings.value_text(KEY_FONT_SIZE), "22 px");
        assert!(settings.adjust(KEY_FONT_SIZE, 10_000).expect("夹到上限"));
        assert_eq!(
            settings.value_text(KEY_FONT_SIZE),
            format!("{MAX_FONT_SIZE_PX} px")
        );
        assert!(settings.adjust(KEY_FONT_SIZE, -10_000).expect("夹到下限"));
        assert_eq!(
            settings.value_text(KEY_FONT_SIZE),
            format!("{MIN_FONT_SIZE_PX} px")
        );
        // 主题循环 system → light → dark → system；dark 必须被投影为暗黑。
        settings.adjust(KEY_THEME, 1).expect("切主题");
        assert_eq!(settings.value_text(KEY_THEME), "明亮");
        settings.adjust(KEY_THEME, 1).expect("再切主题");
        assert!(settings.theme_dark(), "dark 必须被投影为暗黑");
        settings.adjust(KEY_THEME, 1).expect("再切主题");
        assert!(!settings.theme_dark());
        assert!(settings.value_text(KEY_THEME).starts_with("跟随系统"));
        settings
            .set_text(KEY_INTERACTION_MODE, MODE_GESTURE)
            .expect("手势模式");
        assert!(settings.gesture_mode());
        assert!(settings.set_text(KEY_INTERACTION_MODE, "摇一摇").is_err());
        assert!(settings.set_text("不存在", "1").is_err());
        // 行距：控件口径 = 倍数（千分比）；调整后倍数必须变大。
        let before = settings.line_spacing_mult_permille();
        settings.adjust(KEY_LINE_SPACING, 1).expect("行距");
        assert!(settings.line_spacing_mult_permille() > before);
        // 沉浸自动隐藏：布尔翻转。
        assert!(settings.immersive_autohide());
        settings.adjust(KEY_IMMERSIVE_AUTOHIDE, 1).expect("切换");
        assert!(!settings.immersive_autohide());
    }

    #[test]
    fn round_trips_through_storage_and_keeps_global_scope() {
        let dir = temp_dir("roundtrip");
        let db = dir.join("library.db");
        let mut settings = load(&db).expect("首次读取");
        assert_eq!(settings.origin(), SettingsOrigin::Default);
        settings.set_text(KEY_FONT_SIZE, "26").expect("字号");
        settings.set_text(KEY_LINE_SPACING, "1400").expect("行距倍数");
        settings.set_text(KEY_THEME, "dark").expect("主题");
        settings
            .set_text(KEY_INTERACTION_MODE, MODE_GESTURE)
            .expect("手势");
        settings.set_text(KEY_MARGIN_X, "24").expect("左右边距（仅保存）");
        settings.set_control_value(KEY_IMMERSIVE_AUTOHIDE, 0).expect("关闭自动隐藏");
        save(&db, &settings, 1_700_000_000).expect("保存");

        let reloaded = load(&db).expect("再次读取");
        assert_eq!(reloaded.origin(), SettingsOrigin::Stored);
        assert_eq!(reloaded.reader().font_size_milli, 26_000);
        assert_eq!(reloaded.line_spacing_mult_permille(), 1_400);
        assert_eq!(reloaded.reader().margin_x_milli, 24_000);
        assert!(!reloaded.immersive_autohide());
        assert!(reloaded.theme_dark());
        assert!(reloaded.gesture_mode());

        // 全局唯一：设置表只有一行、app_meta 只有已知键，且没有按书的设置行。
        let storage = open_storage(&db).expect("打开库");
        let rows = core_settings::settings_row_count(storage.conn()).expect("设置行数");
        assert_eq!(rows, 1, "V1 只有一行全局设置");
        let keys: Vec<String> = {
            let mut stmt = storage
                .conn()
                .prepare("SELECT key FROM app_meta ORDER BY key")
                .expect("准备查询");
            let iter = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .expect("查询");
            iter.filter_map(Result::ok).collect()
        };
        assert!(keys.contains(&INTERACTION_MODE_KEY.to_string()));
        let book_columns: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('reader_settings') WHERE name LIKE '%book%'",
                [],
                |row| row.get(0),
            )
            .expect("检查列名");
        assert_eq!(book_columns, 0, "设置表不得出现按书的列");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn stored_values_survive_and_invalid_ones_are_rejected_before_write() {
        let dir = temp_dir("validate");
        let db = dir.join("library.db");
        let mut settings = load(&db).expect("读取");
        let error = settings.set_text(KEY_FONT_SIZE, "999").expect_err("超范围");
        assert!(matches!(error, SettingsError::InvalidValue { .. }));
        let error = settings
            .set_text(KEY_FONT_SIZE, "十八")
            .expect_err("非数字");
        assert!(matches!(error, SettingsError::InvalidValue { .. }));
        save(&db, &settings, 1).expect("保存");
        let reloaded = load(&db).expect("读取");
        assert_eq!(reloaded.reader().font_size_milli, 18_000, "非法值不得落库");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn display_rows_cover_all_sections_and_are_projected() {
        let settings = GlobalSettings::default();
        let rows = display_rows(&settings, "/tmp/library.db", (12, 1));
        let sections: Vec<i32> = rows.iter().map(|row| row.section).collect();
        assert!(sections.contains(&0) && sections.contains(&1));
        assert!(sections.contains(&2) && sections.contains(&3) && sections.contains(&4));
        let font_size = rows
            .iter()
            .find(|row| row.key == KEY_FONT_SIZE)
            .expect("字号行");
        assert_eq!(font_size.kind, 1);
        assert_eq!(font_size.value_min, MIN_FONT_SIZE_PX as i32);
        assert_eq!(font_size.value_max, MAX_FONT_SIZE_PX as i32);
        let theme = rows
            .iter()
            .find(|row| row.key == KEY_THEME)
            .expect("主题行");
        assert_eq!(theme.kind, 2);
        assert_eq!(theme.options.len(), 3);
        assert_eq!(theme.current_index, 0);
        let path = rows
            .iter()
            .find(|row| row.key == "data-path")
            .expect("路径行");
        assert!(path.mono);
        let clear = rows
            .iter()
            .find(|row| row.key == "clear-index")
            .expect("清理行");
        assert_eq!(clear.kind, 6);
    }

    #[test]
    fn shelf_view_and_sort_persist_in_meta_keys() {
        let dir = temp_dir("shelf-keys");
        let db = dir.join("library.db");
        assert_eq!(load_shelf_view(&db), 0);
        assert_eq!(load_shelf_sort(&db), crate::app_state::SORT_RECENT);
        save_shelf_view(&db, 1, 1).expect("保存视图");
        save_shelf_sort(&db, crate::app_state::SORT_TITLE, 1).expect("保存排序");
        assert_eq!(load_shelf_view(&db), 1);
        assert_eq!(load_shelf_sort(&db), crate::app_state::SORT_TITLE);
        let _ = fs::remove_dir_all(&dir);
    }
}

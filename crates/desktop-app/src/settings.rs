//! 全局阅读设置：字体、字号、行距、段距、首行缩进、边距、明亮/暗黑、手势/按钮模式。
//!
//! 依据（开发方案 §2.1「全局阅读设置」、§8.3 第 4 条、「每本书独立阅读设置：V1 采用全局设置」）：
//!
//! - **只有一套全局设置**：排版参数落在核心 `reader_settings` 单行（`id = 1`），
//!   交互模式落在核心 `app_meta` 的键值行；本模块不提供任何按 `book_id` 的写入入口，
//!   也不存在每本书独立的设置体系；
//! - 尺寸/间距一律以**整数毫单位**存取（与 `LayoutSignature` 同口径），显示口径的换算
//!   只发生在 [`GlobalSettings::items`] 里；
//! - 桌面 Slint 逐行渲染不叠加段距、不做首行缩进、正文区留白由版式常量表达，
//!   因此段距 / 首行缩进 / 边距在桌面 V1 **只持久化、不参与排版**（签名与测量两侧同时置零，
//!   见开发方案 §8.3 第 4 条）。本模块显式暴露 `effective` 与 `note`，界面据此标注，
//!   绝不假装它们已经生效。

use std::path::Path;

use reading_storage::meta;
use reading_storage::settings::{self as core_settings, ReaderSettings};
use reading_storage::Storage;

use crate::layout::{MAX_FONT_SIZE_PX, MIN_FONT_SIZE_PX};
use crate::reflow::TypographySettings;

/// 交互（翻页）模式的元信息键（存核心 `app_meta`，不新增表/列）。
pub const INTERACTION_MODE_KEY: &str = "ui.interaction_mode";
/// 交互模式：按钮翻页（底部常驻按钮条）。
pub const MODE_BUTTONS: &str = "buttons";
/// 交互模式：手势翻页（正文左右区域点击翻页，中部点击呼出菜单）。
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

/// 数值型字段的取值范围与步长（显示口径：像素）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumericRange {
    /// 最小值（像素）。
    pub min: u32,
    /// 最大值（像素）。
    pub max: u32,
    /// 调整步长（像素）。
    pub step: u32,
}

/// 字段类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    /// 枚举（取值清单 + 展示名）。
    Enum(&'static [&'static str]),
    /// 字体族（可选项来自 [`FONT_FAMILIES`]）。
    FontFamily,
    /// 数值（像素口径）。
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
const DESKTOP_INERT_NOTE: &str = "桌面 V1 只保存该设置：Slint 逐行渲染不叠加段距/缩进，正文区留白由版式常量表达（签名两侧同时置零）";

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
        label: "字号（像素）",
        kind: FieldKind::Numeric(NumericRange {
            min: MIN_FONT_SIZE_PX,
            max: MAX_FONT_SIZE_PX,
            step: 1,
        }),
        effective: true,
        note: "",
    },
    SettingsField {
        key: KEY_LINE_SPACING,
        label: "行距（额外像素 ×1000）",
        kind: FieldKind::Numeric(NumericRange {
            min: 0,
            max: 24,
            step: 1,
        }),
        effective: true,
        note: "",
    },
    SettingsField {
        key: KEY_PARAGRAPH_SPACING,
        label: "段距（像素）",
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
        label: "首行缩进（像素）",
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
        label: "左右边距（像素）",
        kind: FieldKind::Numeric(NumericRange {
            min: 0,
            max: 96,
            step: 4,
        }),
        effective: false,
        note: DESKTOP_INERT_NOTE,
    },
    SettingsField {
        key: KEY_MARGIN_Y,
        label: "上下边距（像素）",
        kind: FieldKind::Numeric(NumericRange {
            min: 0,
            max: 96,
            step: 4,
        }),
        effective: false,
        note: DESKTOP_INERT_NOTE,
    },
    SettingsField {
        key: KEY_THEME,
        label: "明亮 / 暗黑",
        kind: FieldKind::Enum(&["system", "light", "dark"]),
        effective: true,
        note: "",
    },
    SettingsField {
        key: KEY_INTERACTION_MODE,
        label: "翻页模式（手势 / 按钮）",
        kind: FieldKind::Enum(INTERACTION_MODES),
        effective: true,
        note: "",
    },
];

/// 设置行（可显示字段；文案由 Rust 唯一生成）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsItem {
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

/// 全局设置（唯一一套；不含任何按书的覆盖）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalSettings {
    reader: ReaderSettings,
    interaction_mode: String,
    origin: SettingsOrigin,
}

impl Default for GlobalSettings {
    fn default() -> Self {
        Self {
            reader: ReaderSettings::default(),
            interaction_mode: MODE_BUTTONS.to_string(),
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

    /// 是否深色主题（`system` 在 V1 按浅色处理：桌面未接入系统主题探测）。
    pub fn theme_dark(&self) -> bool {
        self.reader.theme == core_settings::THEME_DARK
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

    /// 界面行（全部可显示字段；值文本由 Rust 生成）。
    pub fn items(&self) -> Vec<SettingsItem> {
        SETTINGS_FIELDS
            .iter()
            .map(|field| SettingsItem {
                key: field.key.to_string(),
                label: field.label.to_string(),
                value: self.value_text(field.key),
                note: field.note.to_string(),
                effective: field.effective,
            })
            .collect()
    }

    /// 单字段的显示值。
    pub fn value_text(&self, key: &str) -> String {
        match key {
            KEY_FONT_FAMILY => self.reader.font_family.clone(),
            KEY_FONT_SIZE => format!("{} px", self.reader.font_size_milli / 1_000),
            KEY_LINE_SPACING => format_milli_px(self.reader.line_spacing_milli),
            KEY_PARAGRAPH_SPACING => format_milli_px(self.reader.paragraph_spacing_milli),
            KEY_FIRST_LINE_INDENT => format_milli_px(self.reader.first_line_indent_milli),
            KEY_MARGIN_X => format_milli_px(self.reader.margin_x_milli),
            KEY_MARGIN_Y => format_milli_px(self.reader.margin_y_milli),
            KEY_THEME => self.theme_label(),
            KEY_INTERACTION_MODE => self.interaction_label().to_string(),
            _ => String::new(),
        }
    }

    /// 按字段键取字段元数据。
    pub fn field(key: &str) -> Option<&'static SettingsField> {
        SETTINGS_FIELDS.iter().find(|field| field.key == key)
    }

    /// 调整一个字段（`delta` 为步长个数）；返回是否真的发生变化。
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
            (_, FieldKind::Enum(_)) => {
                return Err(SettingsError::UnknownField {
                    key: key.to_string(),
                })
            }
            (_, FieldKind::Numeric(range)) => {
                let current = self.numeric_milli_px(key);
                let current_px = current / 1_000;
                let next = (current_px + delta * range.step as i64)
                    .clamp(range.min as i64, range.max as i64);
                self.set_milli_px(key, next * 1_000)?;
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
                let px: u32 = value.parse().map_err(|_| SettingsError::InvalidValue {
                    key: key.to_string(),
                    detail: format!("`{value}` 不是整数像素"),
                })?;
                if px < range.min || px > range.max {
                    return Err(SettingsError::InvalidValue {
                        key: key.to_string(),
                        detail: format!("允许范围 {}..={} px", range.min, range.max),
                    });
                }
                self.set_milli_px(key, (px as i64) * 1_000)?;
            }
        }
        Ok(())
    }

    /// 稳定一行（日志与证据用；全部设置值一次打印）。
    pub fn state_log(&self, changes: usize) -> String {
        format!(
            "NR-SETTINGS changes={} origin={} font_family={} font_size_milli={} \
             line_spacing_milli={} paragraph_spacing_milli={} first_line_indent_milli={} \
             margin_x_milli={} margin_y_milli={} theme={} interaction={} dark={}",
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
            u8::from(self.theme_dark()),
        )
    }

    fn numeric_milli_px(&self, key: &str) -> i64 {
        match key {
            KEY_FONT_SIZE => self.reader.font_size_milli,
            KEY_LINE_SPACING => self.reader.line_spacing_milli,
            KEY_PARAGRAPH_SPACING => self.reader.paragraph_spacing_milli,
            KEY_FIRST_LINE_INDENT => self.reader.first_line_indent_milli,
            KEY_MARGIN_X => self.reader.margin_x_milli,
            KEY_MARGIN_Y => self.reader.margin_y_milli,
            _ => 0,
        }
    }

    fn set_milli_px(&mut self, key: &str, value: i64) -> Result<(), SettingsError> {
        let value = value.max(0);
        let slot = match key {
            KEY_FONT_SIZE => &mut self.reader.font_size_milli,
            KEY_LINE_SPACING => &mut self.reader.line_spacing_milli,
            KEY_PARAGRAPH_SPACING => &mut self.reader.paragraph_spacing_milli,
            KEY_FIRST_LINE_INDENT => &mut self.reader.first_line_indent_milli,
            KEY_MARGIN_X => &mut self.reader.margin_x_milli,
            KEY_MARGIN_Y => &mut self.reader.margin_y_milli,
            _ => {
                return Err(SettingsError::UnknownField {
                    key: key.to_string(),
                })
            }
        };
        *slot = value;
        Ok(())
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
    Ok(GlobalSettings {
        reader,
        interaction_mode,
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
    Ok(())
}

#[cfg(test)]
mod tests {
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
        let items = settings.items();
        assert_eq!(items.len(), SETTINGS_FIELDS.len());
        assert!(items.iter().any(|item| item.key == KEY_FONT_SIZE));
        let font_size = items
            .iter()
            .find(|item| item.key == KEY_FONT_SIZE)
            .expect("字号行");
        assert_eq!(font_size.value, "18 px");
        assert!(font_size.effective);
        let indent = items
            .iter()
            .find(|item| item.key == KEY_FIRST_LINE_INDENT)
            .expect("缩进行");
        assert!(!indent.effective && !indent.note.is_empty());
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
        // 行距调整影响排版（参与签名）。
        let before = settings.typography().line_spacing_milli;
        settings.adjust(KEY_LINE_SPACING, 3).expect("行距");
        assert!(settings.typography().line_spacing_milli > before);
    }

    #[test]
    fn round_trips_through_storage_and_keeps_global_scope() {
        let dir = temp_dir("roundtrip");
        let db = dir.join("library.db");
        let mut settings = load(&db).expect("首次读取");
        assert_eq!(settings.origin(), SettingsOrigin::Default);
        settings.set_text(KEY_FONT_SIZE, "26").expect("字号");
        settings.set_text(KEY_LINE_SPACING, "6").expect("行距");
        settings.set_text(KEY_THEME, "dark").expect("主题");
        settings
            .set_text(KEY_INTERACTION_MODE, MODE_GESTURE)
            .expect("手势");
        settings
            .set_text(KEY_MARGIN_X, "24")
            .expect("左右边距（仅保存）");
        save(&db, &settings, 1_700_000_000).expect("保存");

        let reloaded = load(&db).expect("再次读取");
        assert_eq!(reloaded.origin(), SettingsOrigin::Stored);
        assert_eq!(reloaded.reader().font_size_milli, 26_000);
        assert_eq!(reloaded.reader().line_spacing_milli, 6_000);
        assert_eq!(reloaded.reader().margin_x_milli, 24_000);
        assert!(reloaded.theme_dark());
        assert!(reloaded.gesture_mode());

        // 全局唯一：设置表只有一行、app_meta 只有交互模式一个键，且没有按书的设置行。
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
}

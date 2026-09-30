//! `reader_settings` 仓储：全局排版设置（单行，`id = 1`）。
//!
//! 尺寸/间距一律使用**整数毫单位**（与 reading-core `LayoutSignature` 口径一致，
//! 避免浮点在跨平台/跨语言传输时产生分歧）。主题与操作模式为受控枚举。

use rusqlite::{Connection, OptionalExtension};

use crate::error::{Result, StorageError};
use crate::validate::{ensure_max_len, ensure_one_of, MAX_ENCODING_BYTES};

/// 单行主键。
pub const SETTINGS_ROW_ID: i64 = 1;
/// 默认字体族（本机已安装的简体中文字体）。
pub const DEFAULT_FONT_FAMILY: &str = "Noto Sans CJK SC";
/// 默认字重。
pub const DEFAULT_FONT_WEIGHT: i64 = 400;
/// 默认字号（毫单位 ⇒ 18.000 px）。
pub const DEFAULT_FONT_SIZE_MILLI: i64 = 18_000;
/// 主题枚举：跟随系统。
pub const THEME_SYSTEM: &str = "system";
/// 主题枚举：浅色。
pub const THEME_LIGHT: &str = "light";
/// 主题枚举：暗色。
pub const THEME_DARK: &str = "dark";
/// 允许的主题。
pub const THEMES: &[&str] = &[THEME_SYSTEM, THEME_LIGHT, THEME_DARK];
/// 操作模式：整页翻页。
pub const MODE_PAGED: &str = "paged";
/// 操作模式：连续滚动（V1 未实现交互，仅保存设置）。
pub const MODE_SCROLL: &str = "scroll";
/// 允许的操作模式。
pub const OPERATION_MODES: &[&str] = &[MODE_PAGED, MODE_SCROLL];

/// 排版设置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReaderSettings {
    pub font_family: String,
    pub font_weight: i64,
    pub font_size_milli: i64,
    pub line_spacing_milli: i64,
    pub paragraph_spacing_milli: i64,
    pub first_line_indent_milli: i64,
    pub margin_x_milli: i64,
    pub margin_y_milli: i64,
    pub theme: String,
    pub operation_mode: String,
    pub default_encoding: String,
    pub updated_at: i64,
}

impl Default for ReaderSettings {
    fn default() -> Self {
        Self {
            font_family: DEFAULT_FONT_FAMILY.to_string(),
            font_weight: DEFAULT_FONT_WEIGHT,
            font_size_milli: DEFAULT_FONT_SIZE_MILLI,
            line_spacing_milli: 400,
            paragraph_spacing_milli: 0,
            first_line_indent_milli: 2_000,
            margin_x_milli: 16_000,
            margin_y_milli: 16_000,
            theme: THEME_SYSTEM.to_string(),
            operation_mode: MODE_PAGED.to_string(),
            default_encoding: "utf-8".to_string(),
            updated_at: 0,
        }
    }
}

impl ReaderSettings {
    /// 校验全部字段（防非法枚举、负尺寸、超长文本）。
    pub fn validate(&self) -> Result<()> {
        if self.font_family.trim().is_empty() {
            return Err(StorageError::invalid(
                "reader_settings.font_family 不得为空",
            ));
        }
        ensure_max_len("reader_settings.font_family", &self.font_family, 128)?;
        ensure_one_of("reader_settings.theme", &self.theme, THEMES)?;
        ensure_one_of(
            "reader_settings.operation_mode",
            &self.operation_mode,
            OPERATION_MODES,
        )?;
        ensure_max_len(
            "reader_settings.default_encoding",
            &self.default_encoding,
            MAX_ENCODING_BYTES,
        )?;
        for (field, value) in [
            ("font_weight", self.font_weight),
            ("font_size_milli", self.font_size_milli),
            ("line_spacing_milli", self.line_spacing_milli),
            ("paragraph_spacing_milli", self.paragraph_spacing_milli),
            ("first_line_indent_milli", self.first_line_indent_milli),
            ("margin_x_milli", self.margin_x_milli),
            ("margin_y_milli", self.margin_y_milli),
        ] {
            if value < 0 {
                return Err(StorageError::out_of_range(format!(
                    "reader_settings.{field} 不得为负：{value}"
                )));
            }
        }
        if self.font_weight == 0 {
            return Err(StorageError::out_of_range(
                "reader_settings.font_weight 必须 > 0",
            ));
        }
        Ok(())
    }
}

fn settings_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReaderSettings> {
    Ok(ReaderSettings {
        font_family: row.get(0)?,
        font_weight: row.get(1)?,
        font_size_milli: row.get(2)?,
        line_spacing_milli: row.get(3)?,
        paragraph_spacing_milli: row.get(4)?,
        first_line_indent_milli: row.get(5)?,
        margin_x_milli: row.get(6)?,
        margin_y_milli: row.get(7)?,
        theme: row.get(8)?,
        operation_mode: row.get(9)?,
        default_encoding: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

/// 读取设置：行不存在时返回 [`ReaderSettings::default`]（不写库）。
pub fn load_settings(conn: &Connection) -> Result<ReaderSettings> {
    let settings = conn
        .query_row(
            "SELECT font_family, font_weight, font_size_milli, line_spacing_milli, \
             paragraph_spacing_milli, first_line_indent_milli, margin_x_milli, margin_y_milli, \
             theme, operation_mode, default_encoding, updated_at FROM reader_settings \
             WHERE id = ?1",
            [SETTINGS_ROW_ID],
            settings_from_row,
        )
        .optional()?;
    Ok(settings.unwrap_or_default())
}

/// 保存设置（单行 upsert，幂等）。
pub fn save_settings(conn: &Connection, settings: &ReaderSettings, now: i64) -> Result<()> {
    settings.validate()?;
    conn.execute(
        "INSERT INTO reader_settings (id, font_family, font_weight, font_size_milli, \
         line_spacing_milli, paragraph_spacing_milli, first_line_indent_milli, margin_x_milli, \
         margin_y_milli, theme, operation_mode, default_encoding, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13) \
         ON CONFLICT (id) DO UPDATE SET font_family = excluded.font_family, \
         font_weight = excluded.font_weight, font_size_milli = excluded.font_size_milli, \
         line_spacing_milli = excluded.line_spacing_milli, \
         paragraph_spacing_milli = excluded.paragraph_spacing_milli, \
         first_line_indent_milli = excluded.first_line_indent_milli, \
         margin_x_milli = excluded.margin_x_milli, margin_y_milli = excluded.margin_y_milli, \
         theme = excluded.theme, operation_mode = excluded.operation_mode, \
         default_encoding = excluded.default_encoding, updated_at = excluded.updated_at",
        rusqlite::params![
            SETTINGS_ROW_ID,
            settings.font_family,
            settings.font_weight,
            settings.font_size_milli,
            settings.line_spacing_milli,
            settings.paragraph_spacing_milli,
            settings.first_line_indent_milli,
            settings.margin_x_milli,
            settings.margin_y_milli,
            settings.theme,
            settings.operation_mode,
            settings.default_encoding,
            now
        ],
    )?;
    Ok(())
}

/// 设置行数（0 或 1）。
pub fn settings_row_count(conn: &Connection) -> Result<i64> {
    crate::connection::count_rows(conn, "reader_settings")
}

/// 删除设置行（恢复默认；幂等清理路径不会调用本函数）。
pub fn reset_settings(conn: &Connection) -> Result<bool> {
    let affected = conn.execute(
        "DELETE FROM reader_settings WHERE id = ?1",
        [SETTINGS_ROW_ID],
    )?;
    Ok(affected > 0)
}

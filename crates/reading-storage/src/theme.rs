//! 自定义配色的**基色存取**（V1.1，规格 R10/R11；设计稿 T1）。
//!
//! 存储口径（主人裁定）：
//!
//! - 表 `reader_theme_custom` **只存 4 个基色**（`#RRGGBB`，`NULL` = 该项沿用内置值）；
//! - **40 个语义令牌不落库** —— 每次由基色确定性派生（见 `theme_derive`），
//!   所以「改了亮色忘了暗色」这类不一致在结构上不可能发生；
//! - 明/暗两套由同一组基色按不同方向派生（Q3：用户不分别设定明暗）。

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::StorageError;

/// 自定义配色的基色（四项各可 `None` = 沿用内置值）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BaseColors {
    /// 正文色（`#RRGGBB`）。
    pub foreground: Option<String>,
    /// 背景色（`#RRGGBB`）。
    pub background: Option<String>,
    /// 主色（`#RRGGBB`）。
    pub primary: Option<String>,
    /// 强调色（`#RRGGBB`）。
    pub accent: Option<String>,
}

impl BaseColors {
    /// 解析 `#RRGGBB` → RGB 字节；非法或缺失时回退到给定默认值。
    pub fn resolve_foreground(&self, fallback: [u8; 3]) -> [u8; 3] {
        parse_hex(self.foreground.as_deref()).unwrap_or(fallback)
    }

    /// 同上，背景色。
    pub fn resolve_background(&self, fallback: [u8; 3]) -> [u8; 3] {
        parse_hex(self.background.as_deref()).unwrap_or(fallback)
    }

    /// 同上，主色。
    pub fn resolve_primary(&self, fallback: [u8; 3]) -> [u8; 3] {
        parse_hex(self.primary.as_deref()).unwrap_or(fallback)
    }

    /// 同上，强调色。
    pub fn resolve_accent(&self, fallback: [u8; 3]) -> [u8; 3] {
        parse_hex(self.accent.as_deref()).unwrap_or(fallback)
    }
}

/// 解析 `#RRGGBB`（大小写不限）；非法返回 `None`。
pub fn parse_hex(value: Option<&str>) -> Option<[u8; 3]> {
    let text = value?.trim();
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let raw = u32::from_str_radix(hex, 16).ok()?;
    Some([
        ((raw >> 16) & 0xff) as u8,
        ((raw >> 8) & 0xff) as u8,
        (raw & 0xff) as u8,
    ])
}

/// 一行自定义配色设置。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeCustom {
    /// 是否启用自定义配色（`false` = 用内置明/暗）。
    pub enabled: bool,
    /// 四个基色（任一为 `None` 表示沿用内置值）。
    pub base: BaseColors,
    /// 最近修改时间（Unix 秒）。
    pub updated_at: i64,
}

/// 读取自定义配色；**未设置过**返回 `None`（而不是「全空设置」）。
pub fn load_custom_theme(conn: &Connection) -> Result<Option<ThemeCustom>, StorageError> {
    let row = conn
        .query_row(
            "SELECT enabled, base_foreground, base_background, base_primary,
                    base_accent, updated_at
               FROM reader_theme_custom
              WHERE id = 1",
            [],
            |row| {
                Ok(ThemeCustom {
                    enabled: row.get::<_, i64>(0)? != 0,
                    base: BaseColors {
                        foreground: row.get(1)?,
                        background: row.get(2)?,
                        primary: row.get(3)?,
                        accent: row.get(4)?,
                    },
                    updated_at: row.get(5)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

/// 保存自定义配色（`id` 恒为 1：单行设置）。
///
/// **只写基色** —— 不写任何派生令牌（规格 R11）。
pub fn save_custom_theme(
    conn: &Connection,
    theme: &ThemeCustom,
    now: i64,
) -> Result<(), StorageError> {
    validate_hex(theme.base.foreground.as_deref(), "base_foreground")?;
    validate_hex(theme.base.background.as_deref(), "base_background")?;
    validate_hex(theme.base.primary.as_deref(), "base_primary")?;
    validate_hex(theme.base.accent.as_deref(), "base_accent")?;
    conn.execute(
        "INSERT INTO reader_theme_custom
                (id, enabled, base_foreground, base_background, base_primary,
                 base_accent, updated_at)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET
                enabled = excluded.enabled,
                base_foreground = excluded.base_foreground,
                base_background = excluded.base_background,
                base_primary = excluded.base_primary,
                base_accent = excluded.base_accent,
                updated_at = excluded.updated_at",
        params![
            if theme.enabled { 1i64 } else { 0i64 },
            theme.base.foreground.as_deref(),
            theme.base.background.as_deref(),
            theme.base.primary.as_deref(),
            theme.base.accent.as_deref(),
            now,
        ],
    )?;
    Ok(())
}

/// 删除自定义配色（回到内置明/暗）。
pub fn clear_custom_theme(conn: &Connection) -> Result<(), StorageError> {
    conn.execute("DELETE FROM reader_theme_custom", [])?;
    Ok(())
}

/// 校验 `#RRGGBB`；非法则返回**可理解**错误（不做静默纠正）。
fn validate_hex(value: Option<&str>, field: &str) -> Result<(), StorageError> {
    let Some(text) = value else {
        return Ok(());
    };
    let trimmed = text.trim();
    let valid = trimmed
        .strip_prefix('#')
        .is_some_and(|hex| hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()));
    if valid {
        Ok(())
    } else {
        // `InvalidArgument` 语义最贴（不是 DB 内部错误，也不是数值越界）。
        Err(StorageError::InvalidArgument(format!(
            "{field} 不是合法的颜色值（应为 #RRGGBB）：{text}"
        )))
    }
}

/// 从数据库路径读取自定义配色。
pub fn load_custom_theme_from_db(db_path: &Path) -> Result<Option<ThemeCustom>, String> {
    let mut storage = crate::Storage::open(db_path).map_err(|error| error.to_string())?;
    storage.migrate().map_err(|error| error.to_string())?;
    load_custom_theme(storage.conn()).map_err(|error| error.to_string())
}

/// 保存到数据库路径。
pub fn save_custom_theme_to_db(
    db_path: &Path,
    theme: &ThemeCustom,
    now: i64,
) -> Result<(), String> {
    let mut storage = crate::Storage::open(db_path).map_err(|error| error.to_string())?;
    storage.migrate().map_err(|error| error.to_string())?;
    save_custom_theme(storage.conn(), theme, now).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Storage;

    struct Temp(PathBuf);

    impl Temp {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "nr-theme-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("建目录");
            Self(dir)
        }
        fn db(&self) -> PathBuf {
            self.0.join("library.db")
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    use std::path::PathBuf;

    fn open(temp: &Temp) -> Storage {
        let mut storage = Storage::open(temp.db()).expect("开库");
        storage.migrate().expect("迁移");
        storage
    }

    #[test]
    fn absent_theme_reads_as_none_not_all_empty() {
        let temp = Temp::new("absent");
        let storage = open(&temp);
        assert_eq!(
            load_custom_theme(storage.conn()).expect("读"),
            None,
            "从未设置过应返回 None（界面据此用内置配色）"
        );
    }

    #[test]
    fn round_trips_all_four_base_colors() {
        let temp = Temp::new("roundtrip");
        let storage = open(&temp);
        let theme = ThemeCustom {
            enabled: true,
            base: BaseColors {
                foreground: Some("#222222".to_string()),
                background: Some("#fdfdfd".to_string()),
                primary: Some("#1890FF".to_string()),
                accent: Some("#D4380D".to_string()),
            },
            updated_at: 0,
        };
        save_custom_theme(storage.conn(), &theme, 100).expect("保存");

        let loaded = load_custom_theme(storage.conn())
            .expect("读")
            .expect("应存在");
        assert!(loaded.enabled);
        assert_eq!(loaded.base, theme.base, "四个基色必须逐项往返一致");
        assert_eq!(loaded.updated_at, 100);
    }

    #[test]
    fn partial_base_colors_keep_nulls() {
        let temp = Temp::new("partial");
        let storage = open(&temp);
        // 只改主色：其余三项 `None` = 沿用内置值。
        save_custom_theme(
            storage.conn(),
            &ThemeCustom {
                enabled: true,
                base: BaseColors {
                    primary: Some("#ff0000".to_string()),
                    ..Default::default()
                },
                updated_at: 0,
            },
            200,
        )
        .expect("保存");
        let loaded = load_custom_theme(storage.conn())
            .expect("读")
            .expect("存在");
        assert_eq!(loaded.base.primary.as_deref(), Some("#ff0000"));
        assert_eq!(loaded.base.foreground, None, "未改项应保持 None");
        assert_eq!(loaded.base.background, None);
        assert_eq!(loaded.base.accent, None);
    }

    #[test]
    fn save_overwrites_previous_single_row() {
        let temp = Temp::new("single-row");
        let storage = open(&temp);
        for round in 1..=3 {
            save_custom_theme(
                storage.conn(),
                &ThemeCustom {
                    enabled: round % 2 == 1,
                    base: BaseColors {
                        primary: Some(format!("#{:02x}0000", round)),
                        ..Default::default()
                    },
                    updated_at: round,
                },
                round,
            )
            .expect("保存");
        }
        // 表 `id CHECK (id = 1)`：多次保存只有一行。
        let count: i64 = storage
            .conn()
            .query_row("SELECT COUNT(*) FROM reader_theme_custom", [], |row| {
                row.get(0)
            })
            .expect("计数");
        assert_eq!(count, 1, "单行设置：多次保存不得产生多行");
        let loaded = load_custom_theme(storage.conn())
            .expect("读")
            .expect("存在");
        assert_eq!(
            loaded.base.primary.as_deref(),
            Some("#030000"),
            "最后一次的主色"
        );
        assert_eq!(loaded.updated_at, 3, "最后修改时间");
        // `enabled` 随最后一次保存走：round=3 → 奇数 → 启用。
        assert!(
            loaded.enabled,
            "enabled 必须取最后一次保存的值（round=3 → 启用）"
        );
        // 显式保存「禁用」后应读回禁用（不能被上一次的 true 粘住）。
        save_custom_theme(
            storage.conn(),
            &ThemeCustom {
                enabled: false,
                base: BaseColors {
                    primary: Some("#030000".to_string()),
                    ..Default::default()
                },
                updated_at: 4,
            },
            4,
        )
        .expect("保存");
        let disabled = load_custom_theme(storage.conn())
            .expect("读")
            .expect("存在");
        assert!(
            !disabled.enabled,
            "ON CONFLICT 必须覆盖 enabled（不得粘住上次的 true）"
        );
    }

    #[test]
    fn invalid_hex_is_rejected_with_readable_message_and_no_write() {
        let temp = Temp::new("invalid");
        let storage = open(&temp);
        for bad in ["#12345", "123456", "#1234567", "#GGGGGG", "red", ""] {
            let error = save_custom_theme(
                storage.conn(),
                &ThemeCustom {
                    enabled: true,
                    base: BaseColors {
                        primary: Some(bad.to_string()),
                        ..Default::default()
                    },
                    updated_at: 0,
                },
                1,
            )
            .expect_err("非法色值应被拒");
            let message = error.to_string();
            assert!(
                message.contains("base_primary") && message.contains("#RRGGBB"),
                "错误应指名字段与格式，实际：{message}"
            );
        }
        // 一条都不该写进去。
        let count: i64 = storage
            .conn()
            .query_row("SELECT COUNT(*) FROM reader_theme_custom", [], |row| {
                row.get(0)
            })
            .expect("计数");
        assert_eq!(count, 0, "全部非法输入都不得落库");
    }

    #[test]
    fn database_check_constraint_also_rejects_bad_hex() {
        // 即使绕过应用层校验，SQLite 的 GLOB CHECK 也应拒绝（双层防护）。
        let temp = Temp::new("sql-check");
        let storage = open(&temp);
        let error = storage
            .conn()
            .execute(
                "INSERT INTO reader_theme_custom (id, enabled, base_primary, updated_at)
                 VALUES (1, 1, 'nope', 1)",
                [],
            )
            .expect_err("CHECK 应拒绝");
        assert!(
            error.to_string().contains("CHECK") || error.to_string().contains("constraint"),
            "实际：{error}"
        );
    }

    #[test]
    fn clear_removes_theme_entirely() {
        let temp = Temp::new("clear");
        let storage = open(&temp);
        save_custom_theme(
            storage.conn(),
            &ThemeCustom {
                enabled: true,
                base: BaseColors {
                    primary: Some("#123456".to_string()),
                    ..Default::default()
                },
                updated_at: 0,
            },
            1,
        )
        .expect("保存");
        clear_custom_theme(storage.conn()).expect("清除");
        assert_eq!(load_custom_theme(storage.conn()).expect("读"), None);
    }

    #[test]
    fn parse_hex_accepts_both_cases_and_rejects_garbage() {
        assert_eq!(parse_hex(Some("#FFFFFF")), Some([255, 255, 255]));
        assert_eq!(parse_hex(Some("#ffffff")), Some([255, 255, 255]));
        assert_eq!(parse_hex(Some("  #1890ff  ")), Some([0x18, 0x90, 0xff]));
        assert_eq!(parse_hex(Some("#000000")), Some([0, 0, 0]));
        for bad in [
            None,
            Some(""),
            Some("#fff"),
            Some("#fffffff"),
            Some("ffffff"),
            Some("#gggggg"),
        ] {
            assert_eq!(parse_hex(bad), None, "非法输入：{bad:?}");
        }
    }

    #[test]
    fn resolve_falls_back_when_absent_or_invalid() {
        let empty = BaseColors::default();
        assert_eq!(empty.resolve_foreground([1, 2, 3]), [1, 2, 3]);
        assert_eq!(empty.resolve_primary([9, 9, 9]), [9, 9, 9]);
        let set = BaseColors {
            foreground: Some("#0a0b0c".to_string()),
            ..Default::default()
        };
        assert_eq!(set.resolve_foreground([1, 2, 3]), [0x0a, 0x0b, 0x0c]);
    }

    #[test]
    fn db_path_helpers_round_trip() {
        let temp = Temp::new("db-path");
        assert_eq!(load_custom_theme_from_db(&temp.db()).expect("读"), None);
        save_custom_theme_to_db(
            &temp.db(),
            &ThemeCustom {
                enabled: true,
                base: BaseColors {
                    accent: Some("#00ff00".to_string()),
                    ..Default::default()
                },
                updated_at: 0,
            },
            42,
        )
        .expect("保存");
        let loaded = load_custom_theme_from_db(&temp.db())
            .expect("读")
            .expect("存在");
        assert_eq!(loaded.base.accent.as_deref(), Some("#00ff00"));
        assert_eq!(loaded.updated_at, 42);
    }
}

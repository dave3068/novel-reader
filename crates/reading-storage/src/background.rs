//! `reader_background`：阅读背景图引用（V1.1，规格 R14–R17）。
//!
//! 口径（主人裁定 2026-10-05 + 设计稿 B1）：
//!
//! - **全局一张**（不按分类或每书配置）；
//! - 用户选中的图片**复制**到应用数据目录 `backgrounds/`，本表只存**相对文件名**；
//! - **永不**修改或删除用户原图；
//! - 「移除背景」只删副本与本表单例行。
//!
//! 路径安全（规格 R14 / 门禁 C36、C37）：`file_name` 必须是 `backgrounds/` 下的
//! **单个文件名**，不得含 `/`、`\`（Windows 分隔符）、`..`，也不得是绝对路径。
//! 校验在**应用层与数据库 CHECK 双道**执行 —— 只靠数据库不够（应用层需给出可理解错误），
//! 只靠应用层也不够（任何绕过应用层的写入仍须被拦）。

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{Result, StorageError};

/// 缩放口径（`fit_mode`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitMode {
    /// 覆盖铺满（默认）。
    Cover = 0,
    /// 原始尺寸平铺。
    Tile = 1,
    /// 原始尺寸居中。
    Center = 2,
}

impl FitMode {
    pub fn as_i64(self) -> i64 {
        self as i64
    }

    /// 未知值按「覆盖铺满」处理（向前兼容）。
    pub fn from_i64(value: i64) -> Self {
        match value {
            1 => FitMode::Tile,
            2 => FitMode::Center,
            _ => FitMode::Cover,
        }
    }

    /// 用户可见文案（设置页单选项）。
    pub fn label(self) -> &'static str {
        match self {
            FitMode::Cover => "覆盖铺满",
            FitMode::Tile => "原始尺寸平铺",
            FitMode::Center => "原始尺寸居中",
        }
    }

    /// 全部可选项（界面下拉顺序）。
    pub fn all() -> [FitMode; 3] {
        [FitMode::Cover, FitMode::Tile, FitMode::Center]
    }
}

/// 默认遮罩强度（千分比；850 = 85%，设计稿 B2）。
pub const DEFAULT_SCRIM_PERMILLE: i64 = 850;

/// 背景图记录（单例）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Background {
    pub file_name: String,
    pub file_size: i64,
    pub scrim_permille: i64,
    pub fit_mode: FitMode,
    pub updated_at: i64,
}

/// 校验相对文件名的安全性（**应用层第一道**；可理解错误，不静默改写）。
///
/// 拒绝：`/`、`\`（Windows 分隔符）、`..`、绝对路径、空串、超长名。
pub fn validate_file_name(file_name: &str) -> Result<()> {
    if file_name.trim().is_empty() {
        return Err(StorageError::invalid("背景图文件名不能为空"));
    }
    if file_name != file_name.trim() {
        return Err(StorageError::invalid("背景图文件名不得含首尾空白"));
    }
    if file_name.contains('/') || file_name.contains('\\') {
        return Err(StorageError::invalid(
            "背景图文件名只能是 backgrounds/ 下的文件名，不得包含路径分隔符",
        ));
    }
    if file_name.contains("..") {
        return Err(StorageError::invalid("背景图文件名不得包含 .."));
    }
    if file_name.len() > 200 {
        return Err(StorageError::invalid("背景图文件名过长"));
    }
    Ok(())
}

/// 遮罩强度校验（千分比 0..1000）。
fn validate_scrim(permille: i64) -> Result<()> {
    if !(0..=1000).contains(&permille) {
        return Err(StorageError::out_of_range("遮罩强度必须在 0% 到 100% 之间"));
    }
    Ok(())
}

/// 读取当前背景（`None` = 未设置背景）。
pub fn load(conn: &Connection) -> Result<Option<Background>> {
    conn.query_row(
        "SELECT file_name, file_size, scrim_permille, fit_mode, updated_at
         FROM reader_background WHERE id = 1",
        [],
        |row| {
            Ok(Background {
                file_name: row.get(0)?,
                file_size: row.get(1)?,
                scrim_permille: row.get(2)?,
                fit_mode: FitMode::from_i64(row.get(3)?),
                updated_at: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(StorageError::from)
}

/// 设定背景（单例 upsert）。
pub fn set(
    conn: &Connection,
    file_name: &str,
    file_size: i64,
    scrim_permille: i64,
    fit_mode: FitMode,
    now: i64,
) -> Result<()> {
    validate_file_name(file_name)?;
    if file_size < 0 {
        return Err(StorageError::out_of_range("背景图大小不得为负"));
    }
    validate_scrim(scrim_permille)?;
    conn.execute(
        "INSERT INTO reader_background (id, file_name, file_size, scrim_permille, fit_mode, updated_at)
         VALUES (1, ?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (id) DO UPDATE SET
             file_name = excluded.file_name,
             file_size = excluded.file_size,
             scrim_permille = excluded.scrim_permille,
             fit_mode = excluded.fit_mode,
             updated_at = excluded.updated_at",
        params![
            file_name,
            file_size,
            scrim_permille,
            fit_mode.as_i64(),
            now
        ],
    )
    .map_err(StorageError::from)?;
    Ok(())
}

/// 只调遮罩强度（保留图片与缩放口径）。
pub fn set_scrim(conn: &Connection, permille: i64, now: i64) -> Result<()> {
    validate_scrim(permille)?;
    // 口径：SQL 只用 `?1`/`?2` 两个占位符（值 + 时间戳），不引用 `?3`。
    let changed = conn
        .execute(
            "UPDATE reader_background SET scrim_permille = ?1, updated_at = ?2 WHERE id = 1",
            params![permille, now],
        )
        .map_err(StorageError::from)?;
    if changed == 0 {
        return Err(StorageError::invalid("尚未设置背景图，无法调节遮罩强度"));
    }
    Ok(())
}

/// 只调缩放口径（保留图片与遮罩）。
pub fn set_fit_mode(conn: &Connection, fit_mode: FitMode, now: i64) -> Result<()> {
    // 同上：只用 `?1`/`?2`。
    let changed = conn
        .execute(
            "UPDATE reader_background SET fit_mode = ?1, updated_at = ?2 WHERE id = 1",
            params![fit_mode.as_i64(), now],
        )
        .map_err(StorageError::from)?;
    if changed == 0 {
        return Err(StorageError::invalid("尚未设置背景图，无法调节缩放口径"));
    }
    Ok(())
}

/// 移除背景（**只删副本与本行**；用户原图永不触碰，规格 R14）。
pub fn clear(conn: &Connection, now: i64) -> Result<Option<String>> {
    let previous = load(conn)?;
    conn.execute("DELETE FROM reader_background WHERE id = 1", [])
        .map_err(StorageError::from)?;
    let _ = now;
    Ok(previous.map(|item| item.file_name))
}

/// 应用数据目录下背景副本的相对目录名（`reader_background.file_name` 相对于此）。
pub const BACKGROUNDS_DIR_NAME: &str = "backgrounds";

/// 遮罩强度（千分比）→ 0..=255 的 alpha 分量（Slint 着色用）。
///
/// 口径：千分比线性映射到字节 alpha；`0` = 全透明遮罩，`1000` = 完全遮住背景。
pub fn scrim_to_alpha(permille: i64) -> u8 {
    let clamped = permille.clamp(0, 1000);
    ((clamped * 255 + 500) / 1000) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Storage;

    fn storage() -> Storage {
        let mut storage = Storage::open_in_memory().expect("内存库");
        storage.migrate().expect("迁移");
        storage
    }

    #[test]
    fn relative_file_name_is_accepted_and_round_trips() {
        let storage = storage();
        let conn = storage.conn();
        set(
            conn,
            "bg-1.png",
            2048,
            DEFAULT_SCRIM_PERMILLE,
            FitMode::Cover,
            1,
        )
        .expect("设定");
        let background = load(conn).expect("读取").expect("有背景");
        assert_eq!(background.file_name, "bg-1.png");
        assert_eq!(background.file_size, 2048);
        assert_eq!(background.scrim_permille, 850);
        assert_eq!(background.fit_mode, FitMode::Cover);
    }

    #[test]
    fn path_like_names_are_rejected_with_readable_errors() {
        for bad in [
            "../secret.png",
            "sub/dir/a.png",
            "sub\\dir\\a.png",
            " /abs.png",
            "",
            "   ",
            "a..b.png",
        ] {
            let error = validate_file_name(bad).expect_err("应拒绝");
            assert!(
                !error.to_string().is_empty(),
                "拒绝 {bad:?} 时必须给出可理解错误"
            );
        }
        assert!(validate_file_name("bg-1.png").is_ok());
        assert!(validate_file_name("背景图.png").is_ok(), "中文名应允许");
    }

    #[test]
    fn database_layer_also_rejects_backslash_and_traversal() {
        // 应用层已拦；这里守数据库 CHECK 兜底（变异测试 C37 的对象）。
        let storage = storage();
        let conn = storage.conn();
        for bad in ["../secret.png", "sub/dir/a.png", "sub\\dir\\a.png", ""] {
            let result = conn.execute(
                "INSERT INTO reader_background (id, file_name, file_size, updated_at)
                 VALUES (1, ?1, 1, 1)",
                [bad],
            );
            assert!(result.is_err(), "非法文件名 {bad:?} 应被数据库约束拒绝");
        }
        assert!(conn
            .execute(
                "INSERT INTO reader_background (id, file_name, file_size, updated_at)
                 VALUES (1, 'bg-1.png', 1, 1)",
                [],
            )
            .is_ok());
    }

    #[test]
    fn set_rejects_invalid_scrim_and_negative_size() {
        let storage = storage();
        let conn = storage.conn();
        assert!(
            set(conn, "bg.png", 1, 1001, FitMode::Cover, 1).is_err(),
            "遮罩越界"
        );
        assert!(
            set(conn, "bg.png", 1, -1, FitMode::Cover, 1).is_err(),
            "遮罩为负"
        );
        assert!(
            set(conn, "bg.png", -1, 850, FitMode::Cover, 1).is_err(),
            "大小为负"
        );
    }

    #[test]
    fn set_upserts_single_row() {
        let storage = storage();
        let conn = storage.conn();
        set(conn, "bg-1.png", 10, 850, FitMode::Cover, 1).expect("首次");
        set(conn, "bg-2.jpg", 20, 600, FitMode::Tile, 2).expect("替换");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM reader_background", [], |row| {
                row.get(0)
            })
            .expect("计数");
        assert_eq!(count, 1, "背景是单例，必须覆盖而非追加");
        let background = load(conn).expect("读").expect("有");
        assert_eq!(background.file_name, "bg-2.jpg");
        assert_eq!(background.fit_mode, FitMode::Tile);
    }

    #[test]
    fn scrim_and_fit_tune_only_touch_their_own_fields() {
        let storage = storage();
        let conn = storage.conn();
        set(
            conn,
            "bg-1.png",
            10,
            DEFAULT_SCRIM_PERMILLE,
            FitMode::Cover,
            1,
        )
        .expect("设定");

        set_scrim(conn, 500, 2).expect("调遮罩");
        let background = load(conn).expect("读").expect("有");
        assert_eq!(background.scrim_permille, 500);
        assert_eq!(background.file_name, "bg-1.png", "文件名不应被改动");
        assert_eq!(background.fit_mode, FitMode::Cover, "缩放口径不应被改动");

        set_fit_mode(conn, FitMode::Center, 3).expect("调缩放");
        let background = load(conn).expect("读").expect("有");
        assert_eq!(background.fit_mode, FitMode::Center);
        assert_eq!(background.scrim_permille, 500, "遮罩不应被改动");
    }

    #[test]
    fn tuning_without_background_reports_readable_error() {
        let storage = storage();
        let conn = storage.conn();
        let error = set_scrim(conn, 500, 1).expect_err("无背景应报错");
        assert!(error.to_string().contains("尚未设置背景图"));
        assert!(set_fit_mode(conn, FitMode::Tile, 1).is_err());
    }

    #[test]
    fn clear_returns_previous_name_and_leaves_no_row() {
        let storage = storage();
        let conn = storage.conn();
        set(conn, "bg-1.png", 10, 850, FitMode::Cover, 1).expect("设定");
        let previous = clear(conn, 2).expect("移除").expect("应返回原文件名");
        assert_eq!(previous, "bg-1.png");
        assert!(load(conn).expect("读").is_none());
        // 重复移除不报错（幂等）。
        assert!(clear(conn, 3).expect("重复移除").is_none());
    }

    #[test]
    fn scrim_alpha_mapping_covers_range() {
        assert_eq!(scrim_to_alpha(0), 0);
        assert_eq!(scrim_to_alpha(1000), 255);
        assert_eq!(scrim_to_alpha(850), 217);
        // 越界值被夹紧，不产生非法 alpha。
        assert_eq!(scrim_to_alpha(-10), 0);
        assert_eq!(scrim_to_alpha(5000), 255);
    }

    #[test]
    fn fit_modes_round_trip_and_have_labels() {
        for mode in FitMode::all() {
            assert_eq!(FitMode::from_i64(mode.as_i64()), mode);
            assert!(!mode.label().is_empty());
        }
        assert_eq!(FitMode::from_i64(99), FitMode::Cover);
    }

    #[test]
    fn background_is_independent_from_theme_table() {
        // 规格 R16 / 设计稿 B4：两表互不引用。
        let storage = storage();
        let conn = storage.conn();
        set(conn, "bg-1.png", 10, 850, FitMode::Cover, 1).expect("设定背景");
        conn.execute(
            "INSERT INTO reader_theme_custom (id, enabled, base_foreground, updated_at)
             VALUES (1, 1, '#222222', 1)",
            [],
        )
        .expect("写入配色");

        // 删配色不影响背景。
        conn.execute("DELETE FROM reader_theme_custom", [])
            .expect("删配色");
        assert!(load(conn).expect("读背景").is_some(), "删配色不应影响背景");

        // 删背景不影响配色。
        set(conn, "bg-1.png", 10, 850, FitMode::Cover, 1).expect("重设背景");
        conn.execute(
            "INSERT INTO reader_theme_custom (id, enabled, base_foreground, updated_at)
             VALUES (1, 1, '#222222', 1)",
            [],
        )
        .expect("重写配色");
        clear(conn, 2).expect("删背景");
        let theme_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM reader_theme_custom", [], |row| {
                row.get(0)
            })
            .expect("计数");
        assert_eq!(theme_rows, 1, "删背景不应影响配色");
    }
}

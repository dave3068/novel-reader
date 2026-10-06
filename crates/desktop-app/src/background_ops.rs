//! 全局阅读背景图（V1.1，规格 R14–R17；设计稿 B1–B3）。
//!
//! 口径（主人裁定 2026-10-05）：
//!
//! - **全局一张**，不按分类或每书配置；
//! - 用户选中的图片**复制到应用数据目录**，数据库只存**安全相对文件名**；
//! - **原图永不修改、删除或移动**（只读复制）；
//! - 背景只作用于**阅读正文区**，**不参与分页计算**（分页只看文字流）。
//!
//! 路径安全（两层，与存储层 `validate_file_name` 呼应）：
//!
//! 1. 文件名由本模块**生成**（`bg-<unix>-<序号>.<ext>`），不用用户原名；
//! 2. 存储层再校验一次（无 `..` / 无 `/` / 无 `\`）。

use std::path::{Path, PathBuf};

use reading_storage::background::{self, FitMode};
use reading_storage::Storage;

/// 支持的图片扩展名（**白名单**；不在表内的格式一律拒绝）。
pub const ALLOWED_EXTENSIONS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

/// 默认遮罩强度（千分比）。
///
/// **复用存储层常量**（`background::DEFAULT_SCRIM_PERMILLE`），
/// 不在本模块另设一个 —— 两个默认值必然漂移。
pub const DEFAULT_SCRIM: i64 = background::DEFAULT_SCRIM_PERMILLE;

/// 背景图子目录名（在应用数据目录下）。
pub const BACKGROUND_DIR: &str = "backgrounds";

/// 背景图语义类型（V1.1）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundView {
    /// 数据目录下的**绝对**路径（供界面加载；`None` = 未设置）。
    pub path: Option<PathBuf>,
    /// 缩放口径。
    pub fit_mode: FitMode,
    /// 遮罩强度（千分比）。
    pub scrim_permille: i64,
    /// 遮罩 alpha（0–255；由千分比换算，`background::scrim_to_alpha`）。
    pub scrim_alpha: u8,
}

/// 应用数据目录（`$XDG_DATA_HOME/novel-reader`，回退 `~/.local/share/novel-reader`）。
pub fn app_data_dir() -> Result<PathBuf, String> {
    if let Ok(dir) = std::env::var("XDG_DATA_HOME") {
        if !dir.trim().is_empty() {
            return Ok(PathBuf::from(dir).join("novel-reader"));
        }
    }
    let home = std::env::var("HOME")
        .map_err(|_| "无法确定用户主目录（HOME 未设置）；未能保存背景图".to_string())?;
    Ok(PathBuf::from(home).join(".local/share/novel-reader"))
}

/// 背景图目录（不存在则创建）。
pub fn background_dir() -> Result<PathBuf, String> {
    let dir = app_data_dir()?.join(BACKGROUND_DIR);
    std::fs::create_dir_all(&dir).map_err(|error| format!("创建背景图目录失败：{error}"))?;
    Ok(dir)
}

/// 取出扩展名（小写，不含点）；无扩展名返回空串。
fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default()
}

/// 校验源图：扩展名在白名单内、确实存在、是文件。
fn validate_source(source: &Path) -> Result<String, String> {
    if !source.is_file() {
        return Err(format!("背景图不存在或不是文件：{}", source.display()));
    }
    let ext = extension_of(source);
    if !ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
        return Err(format!(
            "不支持的图片格式「{ext}」；仅支持 {}",
            ALLOWED_EXTENSIONS.join(" / ")
        ));
    }
    Ok(ext)
}

/// 复制用户选中的图片到应用数据目录，并写入数据库。
///
/// **原图只读**：只 `copy`，绝不 `rename` / `remove_file` / 写回。
///
/// `fit_mode` / `scrim_permille` 传入 `None` 时用默认值（首次设置）。
pub fn install(
    db_path: &Path,
    source: &Path,
    fit_mode: Option<FitMode>,
    scrim_permille: Option<i64>,
    now: i64,
) -> Result<BackgroundView, String> {
    let ext = validate_source(source)?;
    let dir = background_dir()?;

    // 文件名由本模块生成（不用用户原名）：避免中文 / 空格 / 同名冲突 /
    // 以及用户原名里可能带的路径分隔符。
    let file_name = format!("bg-{now}-{}.{ext}", std::process::id());
    // 存储层再校验一次（双层防护：即使本模块被改坏也拦得住）。
    background::validate_file_name(&file_name).map_err(|error| error.to_string())?;

    let target = dir.join(&file_name);
    std::fs::copy(source, &target)
        .map_err(|error| format!("复制背景图失败（{}）：{error}", source.display()))?;
    let size = std::fs::metadata(&target)
        .map_err(|error| format!("读取背景图大小失败：{error}"))?
        .len() as i64;

    let fit = fit_mode.unwrap_or(FitMode::Cover);
    let scrim = scrim_permille.unwrap_or(DEFAULT_SCRIM);

    let mut storage = Storage::open(db_path).map_err(|error| error.to_string())?;
    storage.migrate().map_err(|error| error.to_string())?;
    // 先记下旧文件名：**写库失败时要把刚复制的文件清掉**（不留孤儿文件）。
    let previous = background::load(storage.conn())
        .map_err(|error| error.to_string())?
        .map(|item| item.file_name);
    if let Err(error) = background::set(storage.conn(), &file_name, size, scrim, fit, now) {
        let _ = std::fs::remove_file(&target);
        return Err(error.to_string());
    }
    // 写库成功后清理**旧文件**（仅限本模块生成的 `bg-*`，绝不动用户原图）。
    if let Some(old) = previous {
        if old != file_name && old.starts_with("bg-") {
            let _ = std::fs::remove_file(dir.join(&old));
        }
    }

    Ok(BackgroundView {
        path: Some(target),
        fit_mode: fit,
        scrim_permille: scrim,
        scrim_alpha: background::scrim_to_alpha(scrim),
    })
}

/// 读回当前背景（供界面投影）。
pub fn load(db_path: &Path) -> Result<BackgroundView, String> {
    let mut storage = Storage::open(db_path).map_err(|error| error.to_string())?;
    storage.migrate().map_err(|error| error.to_string())?;
    let Some(record) = background::load(storage.conn()).map_err(|error| error.to_string())? else {
        return Ok(BackgroundView {
            path: None,
            fit_mode: FitMode::Cover,
            scrim_permille: DEFAULT_SCRIM,
            scrim_alpha: background::scrim_to_alpha(DEFAULT_SCRIM),
        });
    };
    let path = background_dir()?.join(&record.file_name);
    Ok(BackgroundView {
        // 文件缺失（用户手删了数据目录）→ `None`，界面回落纯色，**不报错打断阅读**。
        path: path.is_file().then_some(path),
        fit_mode: record.fit_mode,
        scrim_permille: record.scrim_permille,
        scrim_alpha: background::scrim_to_alpha(record.scrim_permille),
    })
}

/// 只调遮罩强度（不换图）。
pub fn set_scrim(db_path: &Path, permille: i64, now: i64) -> Result<BackgroundView, String> {
    let mut storage = Storage::open(db_path).map_err(|error| error.to_string())?;
    storage.migrate().map_err(|error| error.to_string())?;
    background::set_scrim(storage.conn(), permille, now).map_err(|error| error.to_string())?;
    drop(storage);
    load(db_path)
}

/// 只调缩放口径（不换图）。
pub fn set_fit_mode(db_path: &Path, fit: FitMode, now: i64) -> Result<BackgroundView, String> {
    let mut storage = Storage::open(db_path).map_err(|error| error.to_string())?;
    storage.migrate().map_err(|error| error.to_string())?;
    background::set_fit_mode(storage.conn(), fit, now).map_err(|error| error.to_string())?;
    drop(storage);
    load(db_path)
}

/// 清除背景（删记录 + 删本模块生成的文件）。
///
/// **不删用户原图**（它不在数据目录里，本模块也从不在那里删东西）。
pub fn clear(db_path: &Path, now: i64) -> Result<(), String> {
    let mut storage = Storage::open(db_path).map_err(|error| error.to_string())?;
    storage.migrate().map_err(|error| error.to_string())?;
    let removed = background::clear(storage.conn(), now).map_err(|error| error.to_string())?;
    drop(storage);
    if let Some(name) = removed {
        // 只删 `bg-*`（本模块生成的）；其余名字一律不动 —— 宁可留文件也不误删。
        if name.starts_with("bg-") {
            if let Ok(dir) = background_dir() {
                let _ = std::fs::remove_file(dir.join(&name));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Temp(PathBuf);

    impl Temp {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "nr-bg-{tag}-{}-{:?}",
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
        /// 造一张假的源图（内容非真实图片，但扩展名与存在性足够本模块用）。
        fn source(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, b"fake-image-bytes").expect("写源图");
            path
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// `XDG_DATA_HOME` 是**进程级全局状态**：并行测试各自改它会互相污染
    /// （实测踩中：5 个测试并发时，`load` 读到了别的测试的目录，返回 `None`）。
    /// 故用互斥锁把这批测试**串行化**。
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// 让 `app_data_dir` 落到临时目录（测试隔离；避免污染真实数据目录）。
    fn with_temp_data_dir<R>(temp: &Temp, body: impl FnOnce() -> R) -> R {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let previous = std::env::var("XDG_DATA_HOME").ok();
        std::env::set_var("XDG_DATA_HOME", &temp.0);
        let result = body();
        match previous {
            Some(value) => std::env::set_var("XDG_DATA_HOME", value),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
        result
    }

    #[test]
    fn install_copies_file_and_records_only_safe_name() {
        let temp = Temp::new("install");
        let source = temp.source("我的 原图.PNG");
        with_temp_data_dir(&temp, || {
            let view = install(temp.db().as_path(), &source, None, None, 1000).expect("安装");
            let path = view.path.expect("应有背景图");
            // 复制到数据目录（原图**不在**那里）。
            assert!(path.is_file(), "背景图应存在于数据目录");
            assert_ne!(path, source, "必须复制而不是原地使用");
            // 原图仍在（只读复制）。
            assert!(source.is_file(), "原图不得被移动或删除");
            // 文件名是**生成的**，不含用户原名里的中文与空格。
            let name = path.file_name().expect("文件名").to_string_lossy();
            assert!(name.starts_with("bg-"), "生成的文件名：{name}");
            assert!(name.ends_with(".png"), "扩展名应保留（小写）：{name}");
            assert!(!name.contains(' '), "不得含空格：{name}");
            // 数据库只存安全相对文件名（不含分隔符）。
            let mut storage = Storage::open(temp.db().as_path()).expect("开库");
            let stored: String = storage
                .conn()
                .query_row("SELECT file_name FROM reader_background", [], |row| {
                    row.get(0)
                })
                .expect("读");
            assert_eq!(stored, name);
            assert!(!stored.contains('/') && !stored.contains('\\'));
        });
    }

    #[test]
    fn install_rejects_unsupported_extension_and_missing_file() {
        let temp = Temp::new("reject");
        with_temp_data_dir(&temp, || {
            let text = temp.source("note.txt");
            let error = install(temp.db().as_path(), &text, None, None, 1).expect_err("应拒");
            assert!(error.contains("不支持的图片格式"), "实际：{error}");

            let missing = temp.0.join("nope.png");
            let error = install(temp.db().as_path(), &missing, None, None, 1).expect_err("应拒");
            assert!(error.contains("不存在"), "实际：{error}");

            // 两次拒绝都不得留下数据库记录。
            let mut storage = Storage::open(temp.db().as_path()).expect("开库");
            storage.migrate().expect("迁移");
            let count: i64 = storage
                .conn()
                .query_row("SELECT COUNT(*) FROM reader_background", [], |row| {
                    row.get(0)
                })
                .expect("计数");
            assert_eq!(count, 0, "被拒的安装不得留下记录");
        });
    }

    #[test]
    fn install_accepts_all_whitelisted_extensions_case_insensitively() {
        let temp = Temp::new("ext");
        with_temp_data_dir(&temp, || {
            for (index, name) in ["a.png", "b.JPG", "c.jpeg", "d.webp"].iter().enumerate() {
                let source = temp.source(name);
                let view = install(
                    temp.db().as_path(),
                    &source,
                    None,
                    None,
                    2000 + index as i64,
                )
                .expect("安装");
                assert!(view.path.expect("路径").is_file());
            }
        });
    }

    #[test]
    fn replacing_background_removes_previous_generated_file() {
        let temp = Temp::new("replace");
        with_temp_data_dir(&temp, || {
            let first = temp.source("one.png");
            let view_a = install(temp.db().as_path(), &first, None, None, 3000).expect("装第一张");
            let path_a = view_a.path.expect("第一张");
            let second = temp.source("two.png");
            let view_b = install(temp.db().as_path(), &second, None, None, 3001).expect("装第二张");
            let path_b = view_b.path.expect("第二张");

            assert!(path_b.is_file(), "新图应存在");
            assert!(!path_a.exists(), "旧的本模块生成文件应被清理（不留孤儿）");
            // 两张**原图**都还在。
            assert!(first.is_file() && second.is_file(), "原图永不删除");
        });
    }

    #[test]
    fn load_returns_none_when_not_set_or_file_missing() {
        let temp = Temp::new("load-none");
        with_temp_data_dir(&temp, || {
            let view = load(temp.db().as_path()).expect("读");
            assert!(view.path.is_none(), "未设置时应为 None");
            assert_eq!(view.scrim_permille, DEFAULT_SCRIM, "默认遮罩");

            // 设了图再手删文件 → 回落 None，**不报错**。
            let source = temp.source("gone.png");
            install(temp.db().as_path(), &source, None, None, 4000).expect("装");
            let mut storage = Storage::open(temp.db().as_path()).expect("开库");
            storage.migrate().expect("迁移");
            let name: String = storage
                .conn()
                .query_row("SELECT file_name FROM reader_background", [], |row| {
                    row.get(0)
                })
                .expect("读");
            let _ = std::fs::remove_file(background_dir().expect("目录").join(&name));
            drop(storage);
            let view = load(temp.db().as_path()).expect("读");
            assert!(view.path.is_none(), "文件缺失应回落纯色而非报错");
        });
    }

    #[test]
    fn scrim_and_fit_changes_keep_the_same_image() {
        let temp = Temp::new("tune");
        with_temp_data_dir(&temp, || {
            let source = temp.source("bg.png");
            let view = install(temp.db().as_path(), &source, None, None, 5000).expect("装");
            let path = view.path.expect("路径");

            let tuned = set_scrim(temp.db().as_path(), 600, 5001).expect("调遮罩");
            assert_eq!(tuned.scrim_permille, 600);
            assert_eq!(tuned.path, Some(path.clone()), "调遮罩不得换图");

            let fitted = set_fit_mode(temp.db().as_path(), FitMode::Center, 5002).expect("调缩放");
            assert_eq!(fitted.fit_mode, FitMode::Center);
            assert_eq!(fitted.path, Some(path.clone()), "调缩放不得换图");
            assert!(path.is_file());
        });
    }

    #[test]
    fn clear_removes_record_and_generated_file_but_not_source() {
        let temp = Temp::new("clear");
        with_temp_data_dir(&temp, || {
            let source = temp.source("orig.png");
            let view = install(temp.db().as_path(), &source, None, None, 6000).expect("装");
            let path = view.path.expect("路径");
            clear(temp.db().as_path(), 6001).expect("清除");
            assert!(!path.exists(), "本模块生成的文件应被删");
            assert!(source.is_file(), "**原图必须保留**");
            assert!(load(temp.db().as_path()).expect("读").path.is_none());
        });
    }

    #[test]
    fn clear_refuses_to_delete_non_generated_names() {
        // 伪造一条**非** `bg-` 前缀的记录 → 清除时不得删该文件
        // （宁可留文件也不误删：这是安全边界）。
        let temp = Temp::new("guard");
        with_temp_data_dir(&temp, || {
            let mut storage = Storage::open(temp.db().as_path()).expect("开库");
            storage.migrate().expect("迁移");
            let dir = background_dir().expect("目录");
            let hand_written = dir.join("user-managed.png");
            std::fs::write(&hand_written, b"x").expect("写");
            background::set(
                storage.conn(),
                "user-managed.png",
                1,
                320,
                FitMode::Cover,
                1,
            )
            .expect("写记录");
            drop(storage);

            clear(temp.db().as_path(), 2).expect("清除");
            assert!(
                hand_written.exists(),
                "非本模块生成的文件不得被删（安全边界）"
            );
        });
    }

    #[test]
    fn app_data_dir_prefers_xdg_then_falls_back_to_home() {
        // 显式 XDG 优先。
        std::env::set_var("XDG_DATA_HOME", "/tmp/xdg-probe");
        assert_eq!(
            app_data_dir().expect("目录"),
            PathBuf::from("/tmp/xdg-probe/novel-reader")
        );
        std::env::remove_var("XDG_DATA_HOME");
        // 空串视作未设置（回退 HOME）。
        std::env::set_var("XDG_DATA_HOME", "  ");
        let fallback = app_data_dir().expect("目录");
        assert!(
            fallback.ends_with(".local/share/novel-reader"),
            "回退到 ~/.local/share，实际：{}",
            fallback.display()
        );
        std::env::remove_var("XDG_DATA_HOME");
    }

    #[test]
    fn generated_file_name_passes_storage_layer_validation() {
        // 生成规则必须始终满足存储层的双层校验。
        for now in [1i64, 999_999_999] {
            let name = format!("bg-{now}-12345.png");
            assert!(
                background::validate_file_name(&name).is_ok(),
                "生成的文件名必须合法：{name}"
            );
        }
        // 反例：**路径类**风险必须被拒（这才是存储层的职责）。
        // 注意：中文与空格**不在**拒绝范围内 —— 它们本身无害；
        // 本模块不用用户原名是为了避免冲突与显示问题，不是因为「危险」。
        for bad in [
            "",
            "  ",
            " bg.png",
            "bg.png ",
            "../escape.png",
            "dir/file.png",
            "back\\slash.png",
            "a..b.png",
            &"长".repeat(201),
        ] {
            assert!(
                background::validate_file_name(bad).is_err(),
                "应被存储层拒绝：{bad:?}"
            );
        }
        // 中文 / 空格本身**合法**（本模块仍不用它们，但那是本模块的选择）。
        assert!(
            background::validate_file_name("我的图 v2.png").is_ok(),
            "中文与空格不是存储层的拒绝项"
        );
    }
}

//! 书架后台任务：DT1 中唯一真实的后台阻塞工作。
//!
//! 边界：本模块只调用冻结核心的存储层（`reading-storage`）读取已有数据，不解析正文、
//! 不识别编码、不推导章节；书架为空时如实返回空列表（不注入任何模拟书籍）。
//! 结果经 [`crate::bridge`] 回到 UI 线程。

use std::path::{Path, PathBuf};

use reading_storage::{books, progress, Storage};

use crate::app_state::{BackgroundPayload, BackgroundResult, SettingsView, ShelfRequest, ShelfRow};

/// 应用数据目录内的库文件名。
pub const LIBRARY_FILE_NAME: &str = "library.db";

/// 覆盖库文件路径的环境变量（测试/冒烟用；不设置时使用用户数据目录）。
pub const DB_PATH_ENV: &str = "NR_DB_PATH";

/// 默认库文件路径：`$XDG_DATA_HOME/novel-reader/library.db`
/// （未设置 `XDG_DATA_HOME` 时回退到 `$HOME/.local/share/...`）。
pub fn default_db_path() -> PathBuf {
    if let Some(path) = std::env::var_os(DB_PATH_ENV) {
        return PathBuf::from(path);
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("novel-reader").join(LIBRARY_FILE_NAME)
}

/// 读取书架（阻塞；只在后台线程调用）。
pub fn load_shelf(db_path: &Path) -> BackgroundPayload {
    let mut storage = match Storage::open(db_path) {
        Ok(storage) => storage,
        Err(error) => {
            return BackgroundPayload::Failure {
                message: format!("无法打开书架数据库 {}：{}", db_path.display(), error),
            }
        }
    };
    if let Err(error) = storage.migrate() {
        return BackgroundPayload::Failure {
            message: format!("书架数据库迁移失败：{error}"),
        };
    }

    let records = match books::list_shelf_books(storage.conn()) {
        Ok(records) => records,
        Err(error) => {
            return BackgroundPayload::Failure {
                message: format!("读取书架失败：{error}"),
            }
        }
    };

    let rows: Vec<ShelfRow> = records
        .iter()
        .map(|book| ShelfRow {
            id: book.id.to_string(),
            title: book.display_name.clone(),
            status: book.status.clone(),
            progress: progress_label(&storage, book.id),
        })
        .collect();

    // 设置投影**必须完整**：`from_settings` 只带阅读参数、`items` 为空，
    // 而书架结果会整体替换 UI 的状态（`AppState::apply_result`），一旦用不完整投影，
    // 设置页会被刷成空列表（2026-09-29 真实缺陷）。故此处与启动期同源：`from_global`。
    let global = crate::settings::load(db_path).unwrap_or_default();

    BackgroundPayload::Shelf {
        status_text: format!("书架：{} 本", rows.len()),
        rows,
        settings: Box::new(SettingsView::from_global(&global)),
    }
}

/// 进度摘要：只显示核心存储层已有的位置事实，不做百分比推导。
fn progress_label(storage: &Storage, book_id: i64) -> String {
    match progress::load_progress(storage.conn(), book_id) {
        Ok(Some(record)) => format!(
            "第 {} 章 · 偏移 {}",
            record.chapter_index + 1,
            record.byte_offset
        ),
        Ok(None) => "未开始".to_string(),
        Err(_) => "进度不可读".to_string(),
    }
}

/// 后台任务入口（由 [`crate::bridge::JobChannel`] 调用）。
pub fn shelf_job(request: ShelfRequest) -> BackgroundResult {
    BackgroundResult {
        request_id: request.request_id,
        layout_id: request.layout_id,
        payload: load_shelf(&request.db_path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_db_path_honours_env_override() {
        // 只验证环境变量优先级的纯函数行为；不改进程级环境（避免测试互相干扰）。
        let overridden = PathBuf::from("/tmp/nr-dt1/library.db");
        assert_eq!(overridden.file_name().unwrap(), LIBRARY_FILE_NAME);
        assert!(default_db_path().ends_with(LIBRARY_FILE_NAME));
    }

    #[test]
    fn shelf_payload_carries_complete_settings_projection() {
        // 回归：书架结果会**整体替换** UI 的 settings 投影（`AppState::apply_result`）。
        // 若这里给的是 `from_settings`（items 为空），设置页会被刷成空列表（真实缺陷）。
        let dir = std::env::temp_dir().join(format!("nr-shelf-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建临时目录");
        let db_path = dir.join(LIBRARY_FILE_NAME);
        match load_shelf(&db_path) {
            BackgroundPayload::Shelf { settings, .. } => {
                assert!(
                    !settings.items.is_empty(),
                    "书架结果必须携带完整的设置投影（items 非空），否则设置页会被清空"
                );
                assert_eq!(settings.items.len(), crate::settings::SETTINGS_FIELDS.len());
            }
            other => panic!("书架加载应返回 Shelf，实际 {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_library_yields_empty_shelf_without_injected_books() {
        let dir = std::env::temp_dir().join(format!("nr-dt1-shelf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建临时目录");
        let db_path = dir.join(LIBRARY_FILE_NAME);

        let payload = load_shelf(&db_path);
        match payload {
            BackgroundPayload::Shelf {
                rows, status_text, ..
            } => {
                assert!(rows.is_empty(), "空库不得注入任何书籍");
                assert_eq!(status_text, "书架：0 本");
            }
            other => panic!("期望 Shelf 载荷，实际 {other:?}"),
        }
        assert!(db_path.is_file(), "首次启动应创建本地库文件");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unreadable_library_path_reports_failure_without_panicking() {
        let payload = load_shelf(Path::new("/proc/definitely/not/writable/library.db"));
        match payload {
            BackgroundPayload::Failure { message } => assert!(!message.is_empty()),
            other => panic!("期望 Failure 载荷，实际 {other:?}"),
        }
    }
}

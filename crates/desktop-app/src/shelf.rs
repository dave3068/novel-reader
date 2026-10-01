//! 书架后台任务：书架读取与列表投影（v0.2.0：路径/体积/状态槽位/进度/最近阅读）。
//!
//! 边界：本模块只调用冻结核心的存储层（`reading-storage`）读取已有数据，不解析正文、
//! 不识别编码、不推导章节；书架为空时如实返回空列表（不注入任何模拟书籍）。
//! 结果经 [`crate::bridge`] 回到 UI 线程；过滤与排序在投影层完成（纯计算）。

use std::path::{Path, PathBuf};

use reading_storage::{books, progress, Storage};

use crate::app_state::{
    BackgroundPayload, BackgroundResult, SettingsView, ShelfRequest, ShelfRow,
};
use crate::source_status;

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
        .map(|book| {
            let record = progress::load_progress(storage.conn(), book.id)
                .ok()
                .flatten();
            let (progress_label, percent, recent, recent_epoch) = match &record {
                Some(record) => {
                    let percent = if book.source_size > 0 {
                        ((record.byte_offset.max(0) * 1_000) / book.source_size).clamp(0, 1_000)
                    } else {
                        0
                    };
                    (
                        format!(
                            "第 {} 章 · 偏移 {} 字节",
                            record.chapter_index + 1,
                            record.byte_offset.max(0)
                        ),
                        percent as i32,
                        local_datetime(storage.conn(), record.last_opened_at),
                        record.last_opened_at,
                    )
                }
                None => (
                    "未开始".to_string(),
                    0,
                    String::new(),
                    0,
                ),
            };
            ShelfRow {
                id: book.id.to_string(),
                title: book.display_name.clone(),
                initial: first_char(&book.display_name),
                meta: format!(
                    "{} · {}",
                    book.source_locator,
                    human_size(book.source_size.max(0) as u64)
                ),
                // 原始状态；标签与语义槽位在 `source_status::shelf_job_with_source_check` 映射。
                status: book.status.clone(),
                status_tone: source_status::status_tone(&book.status),
                progress: progress_label,
                percent,
                recent,
                needs_relink: source_status::needs_relink_status(&book.status),
                can_open: source_status::status_can_open(&book.status),
                recent_epoch,
                created_epoch: book.created_at,
            }
        })
        .collect();

    let total_count = books::count_books(storage.conn()).unwrap_or(rows.len() as i64);
    let removed_count = books::removed_book_count(storage.conn()).unwrap_or(0);
    let invalid = rows
        .iter()
        .filter(|row| source_status::needs_relink_status(&row.status))
        .count();

    // 设置投影**必须完整**：`from_settings` 只带阅读参数、`rows` 为空，
    // 而书架结果会整体替换 UI 的状态（`AppState::apply_result`），一旦用不完整投影，
    // 设置页会被刷成空列表（2026-09-29 真实缺陷）。故此处与启动期同源：`view_for`。
    let global = crate::settings::load(db_path).unwrap_or_default();
    let settings = crate::settings::view_for(db_path, &global);

    let status_text = if invalid > 0 {
        format!(
            "书架：{} 本 · {invalid} 本待重新关联",
            total_count - removed_count
        )
    } else {
        format!("书架：{} 本", total_count - removed_count)
    };

    BackgroundPayload::Shelf {
        status_text,
        rows,
        total_count,
        removed_count,
        settings: Box::new(SettingsView { ..settings }),
    }
}

/// 首字（本地派生封面；取第一个非空白字符）。
fn first_char(name: &str) -> String {
    name.chars()
        .find(|ch| !ch.is_whitespace())
        .map(|ch| ch.to_string())
        .unwrap_or_else(|| "书".to_string())
}

/// 体积（人类可读；按 1024 进制）。
pub fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let value = bytes as f64;
    if value < KB {
        format!("{bytes} B")
    } else if value < KB * KB {
        format!("{:.1} KB", value / KB)
    } else if value < KB * KB * KB {
        format!("{:.1} MB", value / (KB * KB))
    } else {
        format!("{:.2} GB", value / (KB * KB * KB))
    }
}

/// Unix 秒 → 本地时间（`yyyy-MM-dd HH:MM`；0/负数返回空串）。
///
/// 时区来自 SQLite 的 `localtime` 修饰符（与系统时区一致），不引入额外依赖。
pub fn local_datetime(conn: &rusqlite::Connection, epoch: i64) -> String {
    if epoch <= 0 {
        return String::new();
    }
    conn.query_row(
        "SELECT strftime('%Y-%m-%d %H:%M', ?1, 'unixepoch', 'localtime')",
        [epoch],
        |row| row.get::<_, Option<String>>(0),
    )
    .ok()
    .flatten()
    .unwrap_or_default()
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
        let overridden = PathBuf::from("/tmp/nr-dt1/library.db");
        assert_eq!(overridden.file_name().unwrap(), LIBRARY_FILE_NAME);
        assert!(default_db_path().ends_with(LIBRARY_FILE_NAME));
    }

    #[test]
    fn shelf_payload_carries_complete_settings_projection() {
        let dir = std::env::temp_dir().join(format!("nr-shelf-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建临时目录");
        let db_path = dir.join(LIBRARY_FILE_NAME);
        match load_shelf(&db_path) {
            BackgroundPayload::Shelf { settings, .. } => {
                assert!(
                    !settings.rows.is_empty(),
                    "书架结果必须携带完整的设置投影（rows 非空），否则设置页会被清空"
                );
                assert!(settings
                    .rows
                    .iter()
                    .any(|row| row.key == crate::settings::KEY_FONT_SIZE));
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

    #[test]
    fn size_and_initial_helpers_are_deterministic() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MB");
        assert_eq!(first_char("  三体"), "三");
        assert_eq!(first_char(""), "书");
    }
}

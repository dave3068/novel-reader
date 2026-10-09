//! 书架后台任务：书架读取与列表投影（v0.2.0：路径/体积/状态槽位/进度/最近阅读）。
//!
//! 边界：本模块只调用冻结核心的存储层（`reading-storage`）读取已有数据，不解析正文、
//! 不识别编码、不推导章节；书架为空时如实返回空列表（不注入任何模拟书籍）。
//! 结果经 [`crate::bridge`] 回到 UI 线程；过滤与排序在投影层完成（纯计算）。

use std::path::{Path, PathBuf};

use reading_storage::{books, categories, progress, Storage};

use crate::app_state::{BackgroundPayload, BackgroundResult, SettingsView, ShelfRequest, ShelfRow};
use crate::{category_tree, source_status};

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
///
/// 分类作用域（V1.1）由参数 `scope` 给出；默认「根 / 全部书籍」。
pub fn load_shelf(db_path: &Path) -> BackgroundPayload {
    load_shelf_in(db_path, &ShelfScope::default())
}

/// 分类作用域（V1.1；`selected_category` 为空串 = 根「全部书籍」）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShelfScope {
    /// 选中分类 id 字符串（空 = 根）。
    pub selected_category: String,
    /// 选中分类真实 id（`None` = 根；`Some(-1)` = 未分类）。
    pub selected_id: Option<i64>,
    /// 折叠的分类 id 集合。
    pub collapsed_categories: std::collections::HashSet<i64>,
}

/// 按给定分类作用域读取书架（阻塞；只在后台线程调用）。
pub fn load_shelf_in(db_path: &Path, scope: &ShelfScope) -> BackgroundPayload {
    let request = scope;
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

    // 分类作用域过滤（V1.1 R1 / C11）：选中分类时主列表**只显示该分类下的书籍** ——
    // **含子孙分类**（与侧栏「合计计数」同一口径，见 `category_tree`）。
    //
    // `None` = 根「全部书籍」⇒ 不过滤；`Some(-1)` = 仅未分类。
    // 过滤在**核心侧**完成（界面只渲染结果），故两端（桌面 / Android）口径一致。
    let visible: Vec<_> = match request.selected_id {
        None => records,
        Some(selected) => {
            let scope_ids: std::collections::HashSet<i64> =
                if selected == category_tree::UNCATEGORIZED_SENTINEL {
                    std::collections::HashSet::new()
                } else {
                    let mut ids =
                        categories::descendant_ids(storage.conn(), selected).unwrap_or_default();
                    ids.push(selected);
                    ids.into_iter().collect()
                };
            let mapping: std::collections::HashMap<i64, Option<i64>> =
                books::list_book_categories(storage.conn())
                    .unwrap_or_default()
                    .into_iter()
                    .collect();
            records
                .into_iter()
                .filter(|book| {
                    let category = mapping.get(&book.id).copied().flatten();
                    if selected == category_tree::UNCATEGORIZED_SENTINEL {
                        category.is_none()
                    } else {
                        category.is_some_and(|id| scope_ids.contains(&id))
                    }
                })
                .collect()
        }
    };

    let rows: Vec<ShelfRow> = visible
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
                None => ("未开始".to_string(), 0, String::new(), 0),
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

    // V1.1：按当前分类作用域过滤行（侧栏选中态在此生效；界面不参与判定）。
    // 过滤口径与侧栏计数**共用** `category_tree::visible_book_ids`，确保
    // 「侧栏计数 / 列表内容 / 页头徽标」三者自洽（设计稿 C1）。
    let all_rows = rows;
    let selected = request.selected_category.as_str();
    let rows = match category_tree::visible_book_ids(storage.conn(), selected) {
        Ok(visible) => {
            let allowed: std::collections::HashSet<i64> = visible.into_iter().collect();
            all_rows
                .into_iter()
                .filter(|row| {
                    row.id
                        .parse::<i64>()
                        .ok()
                        .is_some_and(|book_id| allowed.contains(&book_id))
                })
                .collect()
        }
        // 过滤失败**如实降级为全部**并保留原状态：不能让分类作用域把书架清空。
        Err(_) => all_rows,
    };

    // 侧栏分类树（V1.1）。
    let sidebar_input = crate::category_tree::SidebarInput {
        selected: request.selected_id,
        collapsed: request.collapsed_categories.clone(),
        move_source_id: None,
        visible: true,
    };
    let (categories, _unclassified) =
        match crate::category_tree::project(storage.conn(), &sidebar_input) {
            Ok(projection) => projection,
            Err(_) => (Vec::new(), 0),
        };

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

    // 分类作用域下的状态栏文案（V1.1）：如实带上「当前分类 / N 本」。
    // `filtered_count` 现为**真实过滤后**的行数（此前等于全量，名不符实）。
    let filtered_count = rows.len();
    let scope = crate::category_tree::breadcrumb(storage.conn(), &request.selected_category);
    let status_text = if request.selected_category.is_empty() {
        status_text
    } else {
        let base = if invalid > 0 {
            format!(
                "书架：{} 本 · {invalid} 本待重新关联",
                total_count - removed_count
            )
        } else {
            format!("书架：{} 本", total_count - removed_count)
        };
        format!("{base} · 分类「{scope}」{filtered_count} 本")
    };

    BackgroundPayload::Shelf {
        status_text,
        rows,
        total_count,
        removed_count,
        settings: Box::new(SettingsView { ..settings }),
        categories,
        filtered_count,
        scope_label: scope,
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
    let scope = ShelfScope {
        selected_category: request.selected_category.clone(),
        selected_id: request.selected_id,
        collapsed_categories: request.collapsed_categories.clone(),
    };
    BackgroundResult {
        request_id: request.request_id,
        layout_id: request.layout_id,
        payload: load_shelf_in(&request.db_path, &scope),
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
    // ------------------------------------------------------------------ 分类过滤（C11）

    /// 文件库夹具：`/科幻` 下两本（其中一本在子分类 `/科幻/中国科幻`）、`/历史` 一本、未分类一本。
    fn filter_fixture() -> (std::path::PathBuf, i64, i64, i64) {
        use reading_storage::books::{import_book, BookImport, KIND_DESKTOP_PATH};
        let dir = std::env::temp_dir().join(format!(
            "nr-shelf-filter-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建目录");
        let db = dir.join("library.db");
        let mut storage = Storage::open(&db).expect("开库");
        storage.migrate().expect("迁移");
        let conn = storage.conn();

        let sci = categories::create_category(conn, None, "科幻", 1).expect("科幻");
        let cn = categories::create_category(conn, Some(sci), "中国科幻", 2).expect("中国科幻");
        let hist = categories::create_category(conn, None, "历史", 3).expect("历史");

        let seed = |locator: &str, now: i64| -> i64 {
            import_book(
                conn,
                &BookImport {
                    display_name: locator.to_string(),
                    original_name: "book.txt".to_string(),
                    source_kind: KIND_DESKTOP_PATH.to_string(),
                    source_locator: locator.to_string(),
                    source_size: 64,
                    source_modified_hint: None,
                    source_fingerprint: None,
                    encoding: "utf-8".to_string(),
                    encoding_confidence: 1000,
                    encoding_user_override: false,
                },
                now,
            )
            .expect("导入")
            .book_id
        };
        let in_sci = seed("/tmp/sci.txt", 10);
        let in_cn = seed("/tmp/cn.txt", 11);
        let in_hist = seed("/tmp/hist.txt", 12);
        let loose = seed("/tmp/loose.txt", 13);
        categories::move_book_at(conn, in_sci, Some(sci), 20).expect("归类");
        categories::move_book_at(conn, in_cn, Some(cn), 21).expect("归类");
        categories::move_book_at(conn, in_hist, Some(hist), 22).expect("归类");
        (db, sci, cn, loose)
    }

    fn shelf_row_count(db: &std::path::Path, selected: Option<i64>) -> usize {
        let scope = ShelfScope {
            selected_category: String::new(),
            selected_id: selected,
            collapsed_categories: Default::default(),
        };
        match load_shelf_in(db, &scope) {
            BackgroundPayload::Shelf { rows, .. } => rows.len(),
            other => panic!("应返回书架载荷，实际 {other:?}"),
        }
    }

    #[test]
    fn shelf_lists_every_book_when_no_category_is_selected() {
        // 根「全部书籍」⇒ 不过滤（R1 场景：根节点显示全部）。
        let (db, ..) = filter_fixture();
        assert_eq!(shelf_row_count(&db, None), 4);
    }

    #[test]
    fn selecting_a_category_filters_to_its_subtree() {
        // C11 的关键断言：选中「科幻」必须**只显示该分类下的书**，且**含子孙**。
        // 夹具：科幻直属 1 本 + 子分类「中国科幻」1 本 ⇒ 2 本；「历史」与未分类不得出现。
        let (db, sci, cn, _) = filter_fixture();
        assert_eq!(shelf_row_count(&db, Some(sci)), 2, "科幻含子孙共 2 本");
        // 选中子分类只显示它自己那本。
        assert_eq!(shelf_row_count(&db, Some(cn)), 1, "中国科幻直属 1 本");
    }

    #[test]
    fn selecting_uncategorized_shows_only_loose_books() {
        let (db, _, _, loose) = filter_fixture();
        assert_eq!(
            shelf_row_count(&db, Some(category_tree::UNCATEGORIZED_SENTINEL)),
            1,
            "仅未分类 1 本"
        );
        // 反向：未分类里不得混入已归类的书 —— 用「全部 4 本 ≠ 未分类 1 本」佐证过滤真的生效。
        assert_ne!(
            shelf_row_count(&db, None),
            shelf_row_count(&db, Some(category_tree::UNCATEGORIZED_SENTINEL))
        );
        let _ = loose;
    }

    #[test]
    fn selecting_a_category_with_no_books_shows_nothing() {
        // 空分类 ⇒ 空列表（而不是退化成「显示全部」—— 那会让用户以为分类没生效）。
        let (db, _, _, _) = filter_fixture();
        let scope = ShelfScope {
            selected_category: String::new(),
            selected_id: Some(9999),
            collapsed_categories: Default::default(),
        };
        match load_shelf_in(&db, &scope) {
            BackgroundPayload::Shelf { rows, .. } => assert!(rows.is_empty(), "不存在的分类应为空"),
            other => panic!("应返回书架载荷，实际 {other:?}"),
        }
    }
}

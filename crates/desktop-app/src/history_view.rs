//! 阅读历史的**投影**（V1.1，规格 R8/R9；设计稿 H1–H3）。
//!
//! 职责边界（延续 `category_tree` 的口径）：
//!
//! - 本模块负责：时间标签格式化、按时间倒序、按书聚合、计数文案；
//! - 界面只消费本投影并按 `grouped` 切换两种视图（H1 流水 / H2 按书聚合），
//!   **不在界面里排序或聚合**；
//! - 「最近阅读」**不在分类侧栏**（Q5 裁定），它是书架页的一种视图。

// 呈现口径（时间标签 / 文件标签）已下沉到共享层 —— 见文件头说明。
//
// Android（`reading-ffi`）不依赖本 crate，若把格式化留在这里，Android 只能
// 复制一份「今天 / 本年 / 跨年」的判定，即规格禁止的两个真相。
pub use reading_storage::history_view::{file_label, time_label};

use std::path::Path;

use reading_storage::history::{self, EntryKind, HistoryEntry};
use reading_storage::Storage;

use crate::app_state::{HistoryGroupHead, HistoryRow, HistoryView};

/// 空态标题（设计稿 H1）。
pub const EMPTY_TITLE: &str = "还没有阅读记录";
/// 空态说明（设计稿 H1 固定文案）。
pub const EMPTY_DESC: &str = "打开一本书后会在这里留下一条流水。";

/// 进入方式的语义色槽位（设计稿 H1：书架打开=中性 b，进度恢复/书签/章节=成功 g）。
fn entry_tone(kind: EntryKind) -> i32 {
    match kind {
        // 「从书架打开」是普通入口 → 中性。
        EntryKind::FromShelf | EntryKind::ReopenInReader => 0,
        // 恢复进度 / 书签 / 章节跳转 = 「接着上次读」→ 成功色。
        EntryKind::ResumeProgress | EntryKind::BookmarkJump | EntryKind::ChapterJump => 2,
    }
}

/// 位置标签：`第 N 章 · 偏移 N 字节`（记录的是**打开时刻**的位置）。
fn position_label(entry: &HistoryEntry) -> String {
    format!(
        "第 {} 章 · 偏移 {} 字节",
        entry.chapter_index + 1,
        entry.byte_offset
    )
}

/// 一条流水 → 界面行。
fn to_row(conn: &rusqlite::Connection, entry: &HistoryEntry) -> HistoryRow {
    HistoryRow {
        id: entry.id.to_string(),
        book_id: entry.book_id.to_string(),
        time_label: time_label(conn, entry.opened_at),
        book_title: entry.book_title.clone(),
        file_label: file_label(&entry.source_locator),
        position_label: position_label(entry),
        entry_kind_label: entry.entry_kind.label().to_string(),
        entry_kind_tone: entry_tone(entry.entry_kind),
    }
}

/// 读取历史并投影（`grouped = false` → H1 流水；`true` → H2 按书聚合）。
pub fn project(
    conn: &rusqlite::Connection,
    grouped: bool,
    expanded_book_ids: &[i64],
) -> Result<HistoryView, String> {
    let entries = history::list(conn, None).map_err(|error| error.to_string())?;
    let total = entries.len() as i64;
    let rows: Vec<HistoryRow> = entries.iter().map(|entry| to_row(conn, entry)).collect();

    // 按书聚合：组按最近时间倒序（`list` 已倒序，首次出现即为最近），组内同样倒序。
    let mut groups: Vec<HistoryGroupHead> = Vec::new();
    for entry in &entries {
        match groups
            .iter_mut()
            .find(|group| group.book_id == entry.book_id.to_string())
        {
            Some(group) => {
                group.count += 1;
            }
            None => groups.push(HistoryGroupHead {
                book_id: entry.book_id.to_string(),
                book_title: entry.book_title.clone(),
                count: 1,
                last_time_label: time_label(conn, entry.opened_at),
                expanded: expanded_book_ids.contains(&entry.book_id),
            }),
        }
    }

    let empty = entries.is_empty();
    Ok(HistoryView {
        rows,
        groups,
        grouped,
        empty,
        empty_title: EMPTY_TITLE.to_string(),
        empty_desc: EMPTY_DESC.to_string(),
        count_label: if total > 0 {
            format!("共 {total} 条记录")
        } else {
            "暂无记录".to_string()
        },
        total_count: total,
        // 视图不显示分类侧栏（设计稿 H1）。
        sidebar_visible: false,
        menu_index: -1,
    })
}

/// 打开数据库并投影（界面刷新入口）。
pub fn project_for_db(
    db_path: &Path,
    grouped: bool,
    expanded_book_ids: &[i64],
) -> Result<HistoryView, String> {
    let mut storage = Storage::open(db_path).map_err(|error| error.to_string())?;
    storage.migrate().map_err(|error| error.to_string())?;
    project(storage.conn(), grouped, expanded_book_ids)
}

/// 清空确认弹窗的文案（设计稿 H3 固定文案；**必须**说明不动书籍/进度/书签）。
///
/// 条数由 Rust 读回，不接受界面传入 —— 避免界面显示过期数字。
pub fn clear_confirm_text(db_path: &Path) -> Result<String, String> {
    let mut storage = Storage::open(db_path).map_err(|error| error.to_string())?;
    storage.migrate().map_err(|error| error.to_string())?;
    let count = history::count(storage.conn()).map_err(|error| error.to_string())?;
    Ok(format!(
        "清空全部阅读流水？当前共 {count} 条记录。只会删除阅读流水：不会删除书籍、阅读进度与书签。"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reading_storage::books::{import_book, BookImport, KIND_DESKTOP_PATH};
    use reading_storage::history::{EntryKind, HistoryInput};

    struct Temp(PathBuf);

    impl Temp {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "nr-histview-{tag}-{}-{:?}",
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

    /// 种一本书（**用传入的连接**，不另开 `Storage`）。
    ///
    /// 早前版本在 `seed` 内部 `Storage::open(db)`，而调用方已持有一个连接 ——
    /// 两个连接各自看到不同的快照，导致「已写入的流水读不到」，排序断言错乱
    /// （实测踩中：期望 甲/甲/乙，实得 甲/乙/甲）。
    fn seed(conn: &rusqlite::Connection, locator: &str, title: &str) -> i64 {
        import_book(
            conn,
            &BookImport {
                display_name: title.to_string(),
                original_name: "book.txt".to_string(),
                source_kind: KIND_DESKTOP_PATH.to_string(),
                source_locator: locator.to_string(),
                source_size: 32,
                source_modified_hint: None,
                source_fingerprint: None,
                encoding: "utf-8".to_string(),
                encoding_confidence: 1000,
                encoding_user_override: false,
            },
            1,
        )
        .expect("导入")
        .book_id
    }

    fn add(
        conn: &rusqlite::Connection,
        book: i64,
        title: &str,
        locator: &str,
        at: i64,
        kind: EntryKind,
    ) {
        history::record(
            conn,
            &HistoryInput {
                book_id: book,
                book_title: title.to_string(),
                opened_at: at,
                chapter_index: 2,
                byte_offset: 4096,
                entry_kind: kind,
                source_kind: KIND_DESKTOP_PATH.to_string(),
                source_locator: locator.to_string(),
                content_index_version: 0,
            },
        )
        .expect("记流水");
    }

    #[test]
    fn empty_library_projects_empty_state_with_frozen_text() {
        let temp = Temp::new("empty");
        let mut storage = Storage::open(temp.db()).expect("开库");
        storage.migrate().expect("迁移");
        let view = project(storage.conn(), false, &[]).expect("投影");
        assert!(view.empty);
        assert_eq!(view.total_count, 0);
        assert!(view.rows.is_empty());
        assert_eq!(view.empty_title, EMPTY_TITLE);
        assert_eq!(view.empty_desc, "打开一本书后会在这里留下一条流水。");
        assert_eq!(view.count_label, "暂无记录");
        assert!(!view.sidebar_visible, "历史视图不显示分类侧栏");
    }

    #[test]
    fn flat_view_is_time_descending_with_labels() {
        let temp = Temp::new("flat");
        let mut storage = Storage::open(temp.db()).expect("开库");
        storage.migrate().expect("迁移");
        let first = seed(storage.conn(), "/books/a.txt", "甲");
        let second = seed(storage.conn(), "/books/b.txt", "乙");
        add(
            storage.conn(),
            first,
            "甲",
            "/books/a.txt",
            1000,
            EntryKind::FromShelf,
        );
        add(
            storage.conn(),
            second,
            "乙",
            "/books/b.txt",
            2000,
            EntryKind::ResumeProgress,
        );
        add(
            storage.conn(),
            first,
            "甲",
            "/books/a.txt",
            3000,
            EntryKind::FromShelf,
        );

        let view = project(storage.conn(), false, &[]).expect("投影");
        assert!(!view.empty);
        assert_eq!(view.total_count, 3);
        assert_eq!(view.rows.len(), 3);
        // 写入顺序：甲@1000 → 乙@2000 → 甲@3000。
        // 倒序（opened_at DESC）：甲@3000 → 乙@2000 → 甲@1000。
        let titles: Vec<&str> = view
            .rows
            .iter()
            .map(|row| row.book_title.as_str())
            .collect();
        assert_eq!(titles, vec!["甲", "乙", "甲"], "时间倒序");
        // 同一本书的两次打开：较近的那条（3000）在最前。
        assert_eq!(view.rows[0].book_title, "甲");
        assert_eq!(view.rows[0].position_label, "第 3 章 · 偏移 4096 字节");
        assert_eq!(view.rows[0].file_label, "a.txt", "只显示路径末段");
        assert_eq!(view.rows[0].entry_kind_label, "书架打开");
        // 中间那条是乙（进度恢复），最后一条是甲@1000。
        assert_eq!(view.rows[1].entry_kind_label, "进度恢复");
        assert_eq!(view.rows[1].book_title, "乙");
        assert_eq!(view.rows[2].book_title, "甲");
        assert!(view.count_label.contains("共 3 条记录"));
    }

    #[test]
    fn entry_kind_tone_distinguishes_shelf_from_resume() {
        let temp = Temp::new("tone");
        let mut storage = Storage::open(temp.db()).expect("开库");
        storage.migrate().expect("迁移");
        let book = seed(storage.conn(), "/books/a.txt", "甲");
        add(
            storage.conn(),
            book,
            "甲",
            "/books/a.txt",
            1000,
            EntryKind::FromShelf,
        );
        add(
            storage.conn(),
            book,
            "甲",
            "/books/a.txt",
            2000,
            EntryKind::ResumeProgress,
        );
        let view = project(storage.conn(), false, &[]).expect("投影");
        let shelf_row = view.rows.iter().find(|r| r.entry_kind_label == "书架打开");
        let resume_row = view.rows.iter().find(|r| r.entry_kind_label == "进度恢复");
        assert_eq!(shelf_row.expect("书架打开").entry_kind_tone, 0, "中性");
        assert_eq!(resume_row.expect("进度恢复").entry_kind_tone, 2, "成功色");
    }

    #[test]
    fn grouped_view_aggregates_by_book_with_counts() {
        let temp = Temp::new("group");
        let mut storage = Storage::open(temp.db()).expect("开库");
        storage.migrate().expect("迁移");
        let first = seed(storage.conn(), "/books/a.txt", "甲");
        let second = seed(storage.conn(), "/books/b.txt", "乙");
        add(
            storage.conn(),
            first,
            "甲",
            "/books/a.txt",
            1000,
            EntryKind::FromShelf,
        );
        add(
            storage.conn(),
            first,
            "甲",
            "/books/a.txt",
            2000,
            EntryKind::FromShelf,
        );
        add(
            storage.conn(),
            second,
            "乙",
            "/books/b.txt",
            3000,
            EntryKind::FromShelf,
        );

        let view = project(storage.conn(), true, &[first]).expect("投影");
        assert!(view.grouped);
        assert_eq!(view.groups.len(), 2);
        // 组按最近时间倒序：乙（3000）在前。
        assert_eq!(view.groups[0].book_title, "乙");
        assert_eq!(view.groups[0].count, 1);
        assert_eq!(view.groups[1].book_title, "甲");
        assert_eq!(view.groups[1].count, 2);
        // 展开状态由入参给出。
        assert!(!view.groups[0].expanded);
        assert!(view.groups[1].expanded);
    }

    #[test]
    fn clear_confirm_text_states_count_and_safety() {
        let temp = Temp::new("clear");
        let mut storage = Storage::open(temp.db()).expect("开库");
        storage.migrate().expect("迁移");
        let book = seed(storage.conn(), "/books/a.txt", "甲");
        add(
            storage.conn(),
            book,
            "甲",
            "/books/a.txt",
            1000,
            EntryKind::FromShelf,
        );
        add(
            storage.conn(),
            book,
            "甲",
            "/books/a.txt",
            2000,
            EntryKind::FromShelf,
        );
        drop(storage);

        let text = clear_confirm_text(temp.db().as_path()).expect("文案");
        assert!(text.contains("共 2 条记录"), "{text}");
        assert!(text.contains("不会删除书籍、阅读进度与书签"), "{text}");
    }
}

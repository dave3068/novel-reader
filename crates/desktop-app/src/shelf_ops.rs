//! 书架治理动作（v0.2.0 新增入口）：移出书架 / 彻底删除记录 / 重命名 /
//! 清理可重建索引 / 全部重新关联。
//!
//! 边界（`desktop/spec.md` DR5 与设计 §6.3）：
//!
//! - 任何动作都**不修改、不移动、不删除用户原始 TXT**；
//! - 移出书架是软移除（`books.status = removed`）：进度与书签保留；是否清理
//!   「可重建索引」（章节 / 分页缓存）由调用方按用户勾选决定；
//! - 彻底删除记录是硬删除（级联进度 / 书签 / 来源），必须由用户单独确认；
//! - 清理可重建索引只删 `chapters` / `layout_records`，并回报「删除量 / 保留量」。

use std::path::Path;

use reading_storage::books::{self, STATUS_REMOVED};
use reading_storage::index::{self, IndexClearReport};
use reading_storage::{Storage, StorageError};

use crate::import::FilePicker;
use crate::source_status::{self, DEFAULT_RELINK_DECISION};

/// 索引清理预览（弹窗「将删除 / 将保留」两栏的事实来源）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexClearPreview {
    /// 将删除：章节行数。
    pub chapters: i64,
    /// 将删除：分页缓存（layout_records）行数。
    pub layout_records: i64,
    /// 将保留：书籍行数。
    pub books: i64,
    /// 将保留：来源行数。
    pub book_sources: i64,
    /// 将保留：进度行数。
    pub progress: i64,
    /// 将保留：书签行数。
    pub bookmarks: i64,
    /// 将保留：全局设置行数。
    pub reader_settings: i64,
    /// 将保留：app_meta 行数。
    pub app_meta: i64,
}

impl IndexClearPreview {
    /// 弹窗正文（用户语言；不出现内部表名之外的解释）。
    pub fn dialog_body(&self) -> String {
        format!(
            "将删除（可重建）：章节索引 {} 条、分页缓存 {} 条。\n将保留：书架记录 {} 条、来源 {} 条、阅读进度 {} 条、书签 {} 条、阅读设置 {} 条。\n清理后各书回到「需重建索引」状态，下次打开时按需重建；原始 TXT 与阅读位置不受影响。",
            self.chapters,
            self.layout_records,
            self.books,
            self.book_sources,
            self.progress,
            self.bookmarks,
            self.reader_settings
        )
    }

    /// 清理完成后的 Toast 文案（实际删除 / 保留条数）。
    pub fn result_text(report: &IndexClearReport) -> String {
        format!(
            "已清理可重建索引：章节 {} 条、分页缓存 {} 条；保留 书架 {} / 进度 {} / 书签 {} / 设置 {} 条",
            report.chapters_deleted,
            report.layout_records_deleted,
            report.books_preserved,
            report.progress_preserved,
            report.bookmarks_preserved,
            report.settings_preserved
        )
    }
}

fn open_storage(db_path: &Path) -> Result<Storage, StorageError> {
    let mut storage = Storage::open(db_path)?;
    storage.migrate()?;
    Ok(storage)
}

/// 统计表行数（预览用；表名固定，不接受外部输入）。
fn count_rows(conn: &rusqlite::Connection, table: &str) -> Result<i64, StorageError> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    conn.query_row(&sql, [], |row| row.get(0))
        .map_err(StorageError::from)
}

/// 清理可重建索引前的预览。
pub fn index_clear_preview(db_path: &Path) -> Result<IndexClearPreview, StorageError> {
    let storage = open_storage(db_path)?;
    let conn = storage.conn();
    Ok(IndexClearPreview {
        chapters: count_rows(conn, "chapters")?,
        layout_records: count_rows(conn, "layout_records")?,
        books: count_rows(conn, "books")?,
        book_sources: count_rows(conn, "book_sources")?,
        progress: count_rows(conn, "reading_progress")?,
        bookmarks: count_rows(conn, "bookmarks")?,
        reader_settings: count_rows(conn, "reader_settings")?,
        app_meta: count_rows(conn, "app_meta")?,
    })
}

/// 清理可重建索引（章节 + 分页缓存）；返回真实报告。
pub fn clear_index(db_path: &Path) -> Result<IndexClearReport, StorageError> {
    let storage = open_storage(db_path)?;
    index::clear_rebuildable_index(storage.conn())
}

/// 书架记录数（含已移出）与已移出数。
pub fn counts(db_path: &Path) -> Result<(i64, i64), StorageError> {
    let storage = open_storage(db_path)?;
    Ok((
        books::count_books(storage.conn())?,
        books::removed_book_count(storage.conn())?,
    ))
}

/// 移出书架（软移除）。`delete_index = true` 时同时清理该书的章节索引与分页缓存。
pub fn remove_book(
    db_path: &Path,
    book_id: i64,
    delete_index: bool,
    now: i64,
) -> Result<String, StorageError> {
    let storage = open_storage(db_path)?;
    let name = books::get_book(storage.conn(), book_id)?
        .map(|book| book.display_name)
        .unwrap_or_else(|| format!("book_id={book_id}"));
    if delete_index {
        let report = books::remove_book_from_shelf(storage.conn(), book_id, now)?;
        Ok(format!(
            "已移出书架：{name}（章节索引 {} 条、分页缓存 {} 条已清理；进度与书签保留；原始 TXT 未被修改）",
            report.chapters_deleted, report.layout_records_deleted
        ))
    } else {
        books::set_book_status(storage.conn(), book_id, STATUS_REMOVED, now)?;
        Ok(format!(
            "已移出书架：{name}（章节索引与分页缓存保留，可在「设置 → 清理章节索引」统一删除；原始 TXT 未被修改）"
        ))
    }
}

/// 彻底删除记录（硬删除：级联来源 / 索引 / 进度 / 书签；原始 TXT 不动）。
pub fn purge_book(db_path: &Path, book_id: i64) -> Result<String, StorageError> {
    let storage = open_storage(db_path)?;
    let name = books::get_book(storage.conn(), book_id)?
        .map(|book| book.display_name)
        .unwrap_or_else(|| format!("book_id={book_id}"));
    books::purge_book_record(storage.conn(), book_id)?;
    Ok(format!(
        "已彻底删除记录：{name}（来源、索引、进度与书签一并删除；原始 TXT 未被修改）"
    ))
}

/// 重命名书架显示名（不触碰原始文件）。
pub fn rename_book(
    db_path: &Path,
    book_id: i64,
    name: &str,
    now: i64,
) -> Result<String, StorageError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(StorageError::invalid("书名不得为空"));
    }
    let storage = open_storage(db_path)?;
    books::rename_book(storage.conn(), book_id, trimmed, now)?;
    Ok(format!("已重命名：{trimmed}"))
}

/// 全部重新关联（对给定的失效书籍依次选择文件；每一步都可取消）。
pub fn relink_all(
    db_path: &Path,
    ids: &[i64],
    picker: &dyn FilePicker,
    now: i64,
) -> Vec<String> {
    ids.iter()
        .map(|id| {
            let outcome = source_status::relink_flow(
                db_path,
                *id,
                picker,
                DEFAULT_RELINK_DECISION,
                now,
            );
            source_status::relink_status_text(&outcome)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use reading_storage::progress::{save_progress, ProgressInput};
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nr-shelfops-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("创建临时目录");
        dir
    }

    fn seed_book(dir: &Path, name: &str) -> (i64, PathBuf) {
        let path = dir.join(name);
        let mut text = String::new();
        for index in 1..=20 {
            text.push_str(&format!("第{index}章 测试\n"));
            text.push_str("用于书架治理动作验证的中文正文行。\n\n");
        }
        fs::write(&path, text).expect("写入测试文件");
        let report = crate::import::import_path(&dir.join("library.db"), &path, 100)
            .expect("导入测试书");
        let mut storage = Storage::open(dir.join("library.db")).expect("打开数据库");
        storage.migrate().expect("迁移");
        save_progress(
            storage.conn(),
            &ProgressInput::at(report.book_id, 1, 32),
            200,
        )
        .expect("写入进度");
        (report.book_id, path)
    }

    #[test]
    fn remove_keeps_user_file_and_progress_and_reports_index_policy() {
        let dir = temp_dir("remove");
        let (book_id, path) = seed_book(&dir, "book.txt");
        let db = dir.join("library.db");

        // 勾选「同时删除索引」：软移除 + 索引清理。
        let text = remove_book(&db, book_id, true, 300).expect("移出书架");
        assert!(text.contains("已移出书架"));
        assert!(path.is_file(), "原始 TXT 不得被删除");
        let storage = Storage::open(&db).expect("打开数据库");
        assert_eq!(
            books::get_book(storage.conn(), book_id)
                .expect("读回")
                .expect("存在")
                .status,
            STATUS_REMOVED
        );
        // 再次移出（未勾选）也允许：索引保留。
        let text = remove_book(&db, book_id, false, 301).expect("再次移出");
        assert!(text.contains("保留"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn purge_removes_record_but_not_the_file() {
        let dir = temp_dir("purge");
        let (book_id, path) = seed_book(&dir, "book.txt");
        let db = dir.join("library.db");
        let text = purge_book(&db, book_id).expect("彻底删除");
        assert!(text.contains("已彻底删除记录"));
        assert!(path.is_file(), "原始 TXT 不得被删除");
        let storage = Storage::open(&db).expect("打开数据库");
        assert!(books::get_book(storage.conn(), book_id)
            .expect("读回")
            .is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_rejects_empty_and_updates_display_name() {
        let dir = temp_dir("rename");
        let (book_id, _path) = seed_book(&dir, "book.txt");
        let db = dir.join("library.db");
        assert!(rename_book(&db, book_id, "   ", 1).is_err());
        rename_book(&db, book_id, "新名字", 2).expect("重命名");
        let storage = Storage::open(&db).expect("打开数据库");
        assert_eq!(
            books::get_book(storage.conn(), book_id)
                .expect("读回")
                .expect("存在")
                .display_name,
            "新名字"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn preview_counts_and_clear_report_are_factual() {
        let dir = temp_dir("clear");
        let (book_id, _path) = seed_book(&dir, "book.txt");
        let db = dir.join("library.db");
        let mut storage = Storage::open(&db).expect("打开数据库");
        storage.migrate().expect("迁移");
        index::rebuild_chapters(
            storage.conn(),
            book_id,
            0,
            &[
                index::ChapterInput {
                    sequence: 0,
                    title: "第一章".to_string(),
                    start_byte: 0,
                    end_byte: 10,
                    rule_id: "test".to_string(),
                },
            ],
            10,
        )
        .expect("写入章节");
        drop(storage);

        let preview = index_clear_preview(&db).expect("预览");
        assert!(preview.chapters >= 1);
        assert_eq!(preview.progress, 1);
        assert!(preview.dialog_body().contains("将删除"));

        let report = clear_index(&db).expect("清理");
        assert!(report.chapters_deleted >= 1);
        assert_eq!(report.progress_preserved, 1);
        let text = IndexClearPreview::result_text(&report);
        assert!(text.contains("已清理可重建索引"));
        let _ = fs::remove_dir_all(&dir);
    }
}

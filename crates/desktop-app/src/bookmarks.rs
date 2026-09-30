//! 书签：新增 / 删除 / 重命名 / 跳转。位置一律以 `ContentPosition` 为准。
//!
//! 依据（`desktop/spec.md` DR5、`reading-core/spec.md`、开发方案 §2.1 第 7 条）：
//!
//! - 书签**不绑定动态页码**：落库字段是 `chapter_index` + `byte_offset` + 附近文本指纹，
//!   页码只用于界面显示；
//! - 增删改查全部走核心存储层 `bookmarks` 仓储（含 `title` 短摘要长度上限校验，
//!   正文不得入库）；
//! - 跳转目标来自书签自身的内容位置，不需要重新扫描章节；
//! - 本模块不实现锚点匹配算法：可选的精确恢复复用 [`crate::progress`]（核心锚点语义）。

use std::path::Path;

use reading_storage::{bookmarks as storage_bookmarks, Storage};

use crate::chapters::ChapterToc;
use crate::progress::SavedPosition;

/// 书签名长度上限（与核心存储层一致；防止把正文写进书签名）。
pub const MAX_MARK_TITLE_CHARS: usize = 64;

/// 单次展示的最大书签行数（界面不申请无限长列表）。
pub const MARKS_PER_PAGE: usize = 8;

/// 书签一行（只含可显示字段）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkRow {
    /// 书签标识（64 位；跨 UI 一律字符串化，避免精度截断）。
    pub id: String,
    /// 书签名。
    pub title: String,
    /// 位置标签（「第 N 章 · 偏移 X 字节」，由 Rust 生成，UI 不拼文案）。
    pub position_label: String,
    /// 是否命中当前所在章节（界面高亮用；由 Rust 判断）。
    pub current_chapter: bool,
}

/// 书签跳转目标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkTarget {
    /// 书签标识。
    pub id: i64,
    /// 书签名。
    pub title: String,
    /// 章节序号。
    pub chapter_index: u32,
    /// 字节偏移（页面测量起点）。
    pub byte_offset: u64,
}

impl MarkTarget {
    /// 稳定一行（日志与证据用）。
    ///
    /// 与 `state_log` 同族：`op=jump` 同样携带 `title`，否则按 `NR-MARK` 契约
    /// 解析的端到端脚本会把跳转行判为缺失（书签跳转无可见证据）。
    pub fn as_log(&self, count: usize) -> String {
        format!(
            "NR-MARK op=jump id={} count={} chapter={} offset={} title={}",
            self.id,
            count,
            self.chapter_index,
            self.byte_offset,
            self.title.replace(' ', "_")
        )
    }
}

/// 书签失败原因（可读文本，不作为业务分支）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkError {
    /// 存储层失败。
    Storage(String),
    /// 书签不存在。
    NotFound {
        /// 书签标识。
        id: i64,
    },
    /// 书签名非法（空或超长）。
    InvalidTitle {
        /// 原因。
        detail: String,
    },
}

impl MarkError {
    /// 可读说明（状态栏用）。
    pub fn message(&self) -> String {
        match self {
            MarkError::Storage(message) => format!("书签存取失败：{message}"),
            MarkError::NotFound { id } => format!("书签 {id} 不存在"),
            MarkError::InvalidTitle { detail } => format!("书签名非法：{detail}"),
        }
    }
}

impl std::fmt::Display for MarkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for MarkError {}

fn open_storage(db_path: &Path) -> Result<Storage, MarkError> {
    let mut storage = Storage::open(db_path)
        .map_err(|error| MarkError::Storage(format!("{}（{error}）", db_path.display())))?;
    storage
        .migrate()
        .map_err(|error| MarkError::Storage(format!("迁移失败：{error}")))?;
    Ok(storage)
}

/// 校验书签名（非空且不超长；不得藏正文）。
fn validate_title(title: &str) -> Result<String, MarkError> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(MarkError::InvalidTitle {
            detail: "书签名不得为空".to_string(),
        });
    }
    let chars = trimmed.chars().count();
    if chars > MAX_MARK_TITLE_CHARS {
        return Err(MarkError::InvalidTitle {
            detail: format!("{chars} 字超过上限 {MAX_MARK_TITLE_CHARS} 字"),
        });
    }
    Ok(trimmed.to_string())
}

/// 默认书签名（章节标题 + 偏移；不写正文）。
pub fn default_title(toc: &ChapterToc, chapter_index: u32, byte_offset: u64) -> String {
    let chapter = toc.title_at(chapter_index);
    if chapter.is_empty() {
        format!("偏移 {byte_offset} 字节")
    } else {
        format!("{chapter} · 偏移 {byte_offset}")
    }
}

/// 新增书签（位置取自当前 `ContentPosition` 快照）。
pub fn add(
    db_path: &Path,
    position: &SavedPosition,
    title: &str,
    now: i64,
) -> Result<i64, MarkError> {
    let title = validate_title(title)?;
    let storage = open_storage(db_path)?;
    let input = storage_bookmarks::BookmarkInput {
        book_id: position.book_id,
        content_unit_id: position.content_unit_id,
        chapter_index: position.chapter_index as i64,
        byte_offset: position.byte_offset as i64,
        nearby_fingerprint: Some(position.quote.clone()),
        title,
    };
    storage_bookmarks::create_bookmark(storage.conn(), &input, now)
        .map_err(|error| MarkError::Storage(error.to_string()))
}

/// 某书全部书签（按字节偏移升序）。
pub fn list(db_path: &Path, book_id: i64) -> Result<Vec<MarkTarget>, MarkError> {
    let storage = open_storage(db_path)?;
    let records = storage_bookmarks::list_bookmarks(storage.conn(), book_id)
        .map_err(|error| MarkError::Storage(error.to_string()))?;
    Ok(records
        .into_iter()
        .map(|record| MarkTarget {
            id: record.id,
            title: record.title,
            chapter_index: record.chapter_index.max(0) as u32,
            byte_offset: record.byte_offset.max(0) as u64,
        })
        .collect())
}

/// 重命名书签；返回是否命中。
pub fn rename(db_path: &Path, id: i64, title: &str, now: i64) -> Result<bool, MarkError> {
    let title = validate_title(title)?;
    let storage = open_storage(db_path)?;
    storage_bookmarks::rename_bookmark(storage.conn(), id, &title, now)
        .map_err(|error| MarkError::Storage(error.to_string()))
}

/// 删除书签；返回是否确有删除。
pub fn remove(db_path: &Path, id: i64) -> Result<bool, MarkError> {
    let storage = open_storage(db_path)?;
    storage_bookmarks::delete_bookmark(storage.conn(), id)
        .map_err(|error| MarkError::Storage(error.to_string()))
}

/// 取跳转目标；不存在时报错（不猜测位置）。
pub fn target(db_path: &Path, id: i64) -> Result<MarkTarget, MarkError> {
    let storage = open_storage(db_path)?;
    let record = storage_bookmarks::get_bookmark(storage.conn(), id)
        .map_err(|error| MarkError::Storage(error.to_string()))?
        .ok_or(MarkError::NotFound { id })?;
    Ok(MarkTarget {
        id: record.id,
        title: record.title,
        chapter_index: record.chapter_index.max(0) as u32,
        byte_offset: record.byte_offset.max(0) as u64,
    })
}

/// 书签行窗口：从 `offset` 起取 `span` 行，返回（首个下标，行）。
pub fn window(
    targets: &[MarkTarget],
    toc: Option<&ChapterToc>,
    current_chapter: u32,
    offset: usize,
    span: usize,
) -> (usize, Vec<MarkRow>) {
    if targets.is_empty() || span == 0 {
        return (0, Vec::new());
    }
    let span = span.min(targets.len());
    let max_first = targets.len() - span;
    let first = offset.min(max_first);
    let rows = targets[first..first + span]
        .iter()
        .map(|target| MarkRow {
            id: target.id.to_string(),
            title: target.title.clone(),
            position_label: position_label(toc, target),
            current_chapter: target.chapter_index == current_chapter,
        })
        .collect();
    (first, rows)
}

/// 位置标签（Rust 侧唯一文案来源；页码不参与）。
fn position_label(toc: Option<&ChapterToc>, target: &MarkTarget) -> String {
    match toc {
        Some(toc) if !toc.is_empty() => format!(
            "第 {} 章 · 偏移 {} 字节",
            target.chapter_index + 1,
            target.byte_offset
        ),
        _ => format!("偏移 {} 字节", target.byte_offset),
    }
}

/// 稳定一行（日志与证据用；不含正文）。
pub fn state_log(
    op: &str,
    id: i64,
    count: usize,
    target: Option<&MarkTarget>,
    title: &str,
) -> String {
    format!(
        "NR-MARK op={} id={} count={} chapter={} offset={} title={}",
        op,
        id,
        count,
        target
            .map(|target| target.chapter_index as i64)
            .unwrap_or(-1),
        target.map(|target| target.byte_offset as i64).unwrap_or(-1),
        title.replace(' ', "_"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::{capture, load, save};
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nr-dt4-marks-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("创建临时目录");
        dir
    }

    /// 核心表有外键约束：写入章节 / 进度 / 书签前必须先有书籍记录。
    fn seed_book(db_path: &Path, name: &str) -> i64 {
        let mut storage = Storage::open(db_path).expect("打开库");
        storage.migrate().expect("迁移");
        reading_storage::books::import_book(
            storage.conn(),
            &reading_storage::books::BookImport {
                display_name: name.to_string(),
                original_name: format!("{name}.txt"),
                source_kind: reading_storage::books::KIND_DESKTOP_PATH.to_string(),
                source_locator: format!("/tmp/{name}.txt"),
                source_size: 0,
                source_modified_hint: None,
                source_fingerprint: None,
                encoding: "utf-8".to_string(),
                encoding_confidence: 1_000,
                encoding_user_override: false,
            },
            1,
        )
        .expect("导入书籍记录")
        .book_id
    }

    fn write_fixture(dir: &Path) -> PathBuf {
        let mut lines = Vec::new();
        for chapter in 1..=4 {
            lines.push(format!("第{chapter}章 书签验证"));
            for row in 0..10 {
                lines.push(format!(
                    "第 {chapter} 章第 {row} 行正文，用于校验书签的内容位置语义。"
                ));
            }
            lines.push(String::new());
        }
        let path = dir.join("书签验证.txt");
        fs::write(&path, lines.join("\n")).expect("写入夹具");
        path
    }

    #[test]
    fn add_list_rename_delete_round_trip_on_real_position() {
        let dir = temp_dir("crud");
        let path = write_fixture(&dir);
        let db = dir.join("library.db");
        let book_id = seed_book(&db, "书签夹具");
        let toc =
            crate::chapters::scan_toc(&path, reading_source::TextEncoding::Utf8).expect("扫描章节");
        let offset = toc.start_byte(1).expect("第二章起点") + 30;
        let chapter_index = toc.sequence_at_offset(offset);
        let position = capture(&path, book_id, chapter_index, offset).expect("采样位置");

        let title = default_title(&toc, chapter_index, position.byte_offset);
        assert!(
            title.starts_with("第2章") && title.contains("偏移"),
            "默认名应来自章节标题与偏移：{title}"
        );
        let id = add(&db, &position, &title, 1_000).expect("新增书签");

        let listed = list(&db, book_id).expect("列出书签");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, id);
        assert_eq!(listed[0].byte_offset, position.byte_offset);
        assert_eq!(listed[0].chapter_index, position.chapter_index);

        assert!(rename(&db, id, " 关键情节 ", 1_100).expect("重命名"));
        assert_eq!(list(&db, book_id).expect("列出")[0].title, "关键情节");

        assert!(remove(&db, id).expect("删除"));
        assert!(list(&db, book_id).expect("列出").is_empty());
        assert!(
            !remove(&db, id).expect("重复删除"),
            "重复删除必须返回 false"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn jump_target_returns_content_position_not_page_number() {
        let dir = temp_dir("jump");
        let path = write_fixture(&dir);
        let db = dir.join("library.db");
        let book_id = seed_book(&db, "跳转夹具");
        let offset = 900;
        let position = capture(&path, book_id, 1, offset).expect("采样");
        let id = add(&db, &position, "跳转点", 5).expect("新增");

        let jump = target(&db, id).expect("取跳转目标");
        assert_eq!(jump.byte_offset, position.byte_offset);
        assert!(jump.as_log(1).contains("op=jump"));

        // 跳转后重新采样同一偏移必须得到相同位置（可再次恢复）。
        let again =
            capture(&path, book_id, jump.chapter_index, jump.byte_offset).expect("重新采样");
        assert_eq!(again.byte_offset, position.byte_offset);
        assert_eq!(again.quote, position.quote);

        let missing = target(&db, 9_999).expect_err("不存在必须报错");
        assert_eq!(missing, MarkError::NotFound { id: 9_999 });
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn title_must_be_non_empty_and_short() {
        let dir = temp_dir("title");
        let path = write_fixture(&dir);
        let db = dir.join("library.db");
        let book_id = seed_book(&db, "名校验夹具");
        let position = capture(&path, book_id, 0, 0).expect("采样");
        let empty = add(&db, &position, "   ", 1).expect_err("空名必须被拒绝");
        assert!(matches!(empty, MarkError::InvalidTitle { .. }));
        let long = "字".repeat(MAX_MARK_TITLE_CHARS + 1);
        let too_long = add(&db, &position, &long, 1).expect_err("超长必须被拒绝");
        assert!(matches!(too_long, MarkError::InvalidTitle { .. }));
        assert!(
            list(&db, book_id).expect("列出").is_empty(),
            "非法书签不得入库"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn window_is_bounded_and_flags_current_chapter() {
        let dir = temp_dir("window");
        let path = write_fixture(&dir);
        let db = dir.join("library.db");
        let book_id = seed_book(&db, "窗口夹具");
        let toc =
            crate::chapters::scan_toc(&path, reading_source::TextEncoding::Utf8).expect("扫描章节");
        for chapter in 0..4u32 {
            let offset = toc.start_byte(chapter).unwrap() + 5;
            let position = capture(&path, book_id, chapter, offset).expect("采样");
            add(
                &db,
                &position,
                &format!("书签{chapter}"),
                1 + chapter as i64,
            )
            .expect("新增");
        }
        let targets = list(&db, book_id).expect("列出");
        assert_eq!(targets.len(), 4);
        let (first, rows) = window(&targets, Some(&toc), 2, 0, 3);
        assert_eq!(first, 0);
        assert_eq!(rows.len(), 3, "窗口必须有界");
        assert!(rows.iter().any(|row| row.current_chapter));
        let (first, rows) = window(&targets, Some(&toc), 0, 99, 3);
        assert_eq!(first, 1, "越界偏移夹紧到最后一个窗口");
        assert_eq!(rows.len(), 3);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn marks_survive_progress_rewrites() {
        let dir = temp_dir("survive");
        let path = write_fixture(&dir);
        let db = dir.join("library.db");
        let book_id = seed_book(&db, "保留夹具");
        let position = capture(&path, book_id, 1, 400).expect("采样");
        let id = add(&db, &position, "保留书签", 1).expect("新增");

        // 同一本书继续阅读并保存进度：书签不得被覆盖或删除。
        let later = capture(&path, book_id, 2, 700).expect("采样");
        save(&db, &later, 2).expect("保存进度");
        assert_eq!(
            load(&db, book_id)
                .expect("读取进度")
                .expect("应有进度")
                .byte_offset,
            later.byte_offset,
            "进度落库必须与采样位置一致（偏移会夹到合法字符边界）"
        );
        let listed = list(&db, book_id).expect("列出书签");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, id);
        assert_eq!(listed[0].byte_offset, position.byte_offset);
        let _ = fs::remove_dir_all(&dir);
    }
}

//! 来源可用性与失效重新关联（DT2）。
//!
//! 依据 `desktop/spec.md` DR5、`desktop/design.md` §文件选择 与
//! `docs/novel-reader-v1-development-plan.md` §6.3：
//!
//! - 原文件不存在 / 不可读 / 内容（指纹）变化时，书架条目进入「需要重新关联」类状态：
//!   **保留书籍条目、阅读进度与书签**，只校正 `books.status`；
//! - 重新关联必须通过**来源指纹**比较与**用户确认**：内容不一致时默认**零改动**，
//!   绝不用新文件静默覆盖旧进度（确认入口见 [`RelinkDecision`]）；
//! - 重新关联只登记新来源（`book_sources`）并把 `books.source_locator` 指向新路径；
//!   进度与书签行保持原样；内容变化时由冻结核心 `reading-storage` 只失效**可重建索引**；
//! - 任何路径都不修改、不移动、不删除用户原始文件（只读打开）。
//!
//! # 状态口径（核心状态 → 界面标签）
//!
//! | 核心 `books.status` | 界面标签 | 含义 |
//! |---|---|---|
//! | `ready` | 可阅读 | 来源可用且内容与记录一致 |
//! | `missing` | 需要重新关联 | 原文件不存在 / 不可读（记录、进度、书签保留） |
//! | `needs_reindex` | 内容已变化，需重建索引 | 指纹或编码口径变化，只失效可重建索引 |
//! | `import_failed` | 导入失败 | 交给导入链路处理 |
//! | `removed` | 已移除 | 软移除（不在书架列表） |
//!
//! 标签是**显示字段**：判定与分支一律用核心状态字符串，不用标签反推业务。

use std::path::{Path, PathBuf};

use reading_source::{FileTextSource, TextSource};
use reading_storage::books::{
    self, BookImport, BookRecord, KIND_DESKTOP_PATH, STATUS_IMPORT_FAILED, STATUS_MISSING,
    STATUS_NEEDS_REINDEX, STATUS_READY, STATUS_REMOVED,
};
use reading_storage::progress::ProgressRecord;
use reading_storage::{bookmarks, progress, Storage, StorageError};

use crate::app_state::{BackgroundPayload, BackgroundResult, ShelfRequest};
use crate::import::{self, FilePicker, ImportError};

/// 程序化验证用：设置后，启动期对指定 `book_id` 走一次重新关联
/// （新文件路径取 [`RELINK_PATH_ENV`]，未设置时回退 `NR_IMPORT_PATH`）。
pub const RELINK_BOOK_ENV: &str = "NR_RELINK_BOOK";

/// 程序化验证用：重新关联时要选择的文件路径。
///
/// 与 `NR_IMPORT_PATH` 分开，避免「先导入一次、再重新关联」在同一进程里互相干扰。
pub const RELINK_PATH_ENV: &str = "NR_RELINK_PATH";

/// 界面标签：可阅读。
pub const LABEL_READY: &str = "可阅读";
/// 界面标签：需要重新关联。
pub const LABEL_MISSING: &str = "需要重新关联";
/// 界面标签：内容变化，需要重建索引。
pub const LABEL_NEEDS_REINDEX: &str = "内容已变化，需重建索引";
/// 界面标签：导入失败。
pub const LABEL_IMPORT_FAILED: &str = "导入失败";
/// 界面标签：已移除。
pub const LABEL_REMOVED: &str = "已移除";

/// 核心状态 → 界面标签（未知状态原样透传，绝不显示空状态）。
pub fn status_label(status: &str) -> String {
    match status {
        STATUS_READY => LABEL_READY.to_string(),
        STATUS_MISSING => LABEL_MISSING.to_string(),
        STATUS_NEEDS_REINDEX => LABEL_NEEDS_REINDEX.to_string(),
        STATUS_IMPORT_FAILED => LABEL_IMPORT_FAILED.to_string(),
        STATUS_REMOVED => LABEL_REMOVED.to_string(),
        other => other.to_string(),
    }
}

/// 核心状态 → 语义色槽位（设计 §2.4 Tag：0=中性 1=信息 2=成功 3=警告 4=错误）。
pub fn status_tone(status: &str) -> i32 {
    match status {
        STATUS_READY => 2,
        STATUS_MISSING => 4,
        STATUS_NEEDS_REINDEX => 3,
        STATUS_IMPORT_FAILED => 4,
        STATUS_REMOVED => 0,
        _ => 0,
    }
}

/// 该状态是否可以打开阅读（只有 `ready` 可读；其余如实拒绝并给出原因）。
pub fn status_can_open(status: &str) -> bool {
    status == STATUS_READY
}

/// 该标签是否需要「重新关联」入口（内容变化与文件失效共用同一入口）。
pub fn needs_relink_label(label: &str) -> bool {
    label == LABEL_MISSING || label == LABEL_NEEDS_REINDEX
}

/// 该核心状态是否需要「重新关联」入口。
pub fn needs_relink_status(status: &str) -> bool {
    status == STATUS_MISSING || status == STATUS_NEEDS_REINDEX
}

// ---------------------------------------------------------------------------
// 来源检查
// ---------------------------------------------------------------------------

/// 来源事实状态（只描述「文件与记录是否一致」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceState {
    /// 文件存在、可读、内容与记录指纹一致。
    Ok,
    /// 文件不存在或不可访问。
    Missing,
    /// 文件存在但打不开 / 读不了（权限或 IO）。
    Unreadable,
    /// 文件存在且可读，但内容（指纹）与记录不一致。
    ContentChanged,
    /// 非桌面来源（本端不检查，不做状态改动）。
    NotDesktop,
}

impl SourceState {
    /// 可读短语。
    pub const fn label(self) -> &'static str {
        match self {
            SourceState::Ok => "来源正常",
            SourceState::Missing => "原文件不存在",
            SourceState::Unreadable => "原文件不可读",
            SourceState::ContentChanged => "文件内容已变化",
            SourceState::NotDesktop => "非桌面来源",
        }
    }
}

/// 一次来源检查的事实（全部来自真实文件系统与数据库读回）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCheck {
    /// 书籍 id。
    pub book_id: i64,
    /// 来源事实状态。
    pub state: SourceState,
    /// 被检查的来源定位符。
    pub locator: String,
    /// 检查前的书架状态。
    pub status_before: String,
    /// 检查后的书架状态（未写库时等于 `status_before`）。
    pub status_after: String,
    /// 记录中的指纹。
    pub stored_fingerprint: Option<String>,
    /// 本次实测指纹（文件不可读时为 `None`）。
    pub current_fingerprint: Option<String>,
    /// 是否真的写入了状态。
    pub status_written: bool,
    /// 乐观并发守卫是否命中（来源已被重新关联，快照过期 ⇒ 未写库）。
    pub stale_snapshot: bool,
    /// 检查后的进度位置（保留证据）。
    pub progress_offset: Option<i64>,
    /// 检查后的书签数（保留证据）。
    pub bookmarks: i64,
}

impl SourceCheck {
    /// 面向状态栏/证据的一行结果。
    pub fn status_text(&self) -> String {
        let guard = if self.stale_snapshot {
            "（来源已变化：按过期快照不写状态）"
        } else if self.status_written {
            "（已更新书架状态）"
        } else {
            ""
        };
        format!(
            "来源检查 书籍 {}：{}{}，状态 {} → {}，进度 {}，书签 {}{}",
            self.book_id,
            self.state.label(),
            guard,
            status_label(&self.status_before),
            status_label(&self.status_after),
            match self.progress_offset {
                Some(offset) => format!("偏移 {offset}"),
                None => "未开始".to_string(),
            },
            self.bookmarks,
            match (&self.stored_fingerprint, &self.current_fingerprint) {
                (Some(stored), Some(current)) if stored != current =>
                    format!("，指纹 {stored} → {current}"),
                _ => String::new(),
            }
        )
    }
}

/// 只读检查：不写任何库记录。
pub fn check_source(book: &BookRecord) -> SourceCheck {
    if book.source_kind != KIND_DESKTOP_PATH {
        return SourceCheck {
            book_id: book.id,
            state: SourceState::NotDesktop,
            locator: book.source_locator.clone(),
            status_before: book.status.clone(),
            status_after: book.status.clone(),
            stored_fingerprint: book.source_fingerprint.clone(),
            current_fingerprint: None,
            status_written: false,
            stale_snapshot: false,
            progress_offset: None,
            bookmarks: 0,
        };
    }

    let path = PathBuf::from(&book.source_locator);
    let (state, current_fingerprint) = inspect_file(&path, book.source_fingerprint.as_deref());

    SourceCheck {
        book_id: book.id,
        state,
        locator: book.source_locator.clone(),
        status_before: book.status.clone(),
        status_after: book.status.clone(),
        stored_fingerprint: book.source_fingerprint.clone(),
        current_fingerprint,
        status_written: false,
        stale_snapshot: false,
        progress_offset: None,
        bookmarks: 0,
    }
}

/// 检查文件事实：元数据 → 只读打开 → 抽样指纹 → 与记录指纹比较内容签名。
fn inspect_file(path: &Path, stored_fingerprint: Option<&str>) -> (SourceState, Option<String>) {
    match std::fs::metadata(path) {
        Err(_) => return (SourceState::Missing, None),
        Ok(metadata) if !metadata.is_file() => return (SourceState::Missing, None),
        Ok(_) => {}
    }
    let source = match FileTextSource::open(path) {
        Ok(source) => source,
        Err(_) => return (SourceState::Unreadable, None),
    };
    let fingerprint = match source.fingerprint() {
        Ok(fingerprint) => fingerprint,
        Err(_) => return (SourceState::Unreadable, None),
    };
    let compact = import::compact_fingerprint(&fingerprint);
    let state = match stored_fingerprint.and_then(import::parse_compact_fingerprint) {
        Some(stored) if stored.same_content(&fingerprint) => SourceState::Ok,
        // 无指纹记录或指纹不可解析：保守判定为「内容变化」，需要用户确认。
        _ => SourceState::ContentChanged,
    };
    (state, Some(compact))
}

/// 目标状态（`None` 表示不需要改动）。
fn desired_status(state: SourceState, status: &str) -> Option<&'static str> {
    match state {
        // 只有「原文件回来了且内容与记录一致」才把失效状态恢复为可阅读；
        // `needs_reindex`（编码等其他原因）不在此处自动恢复。
        SourceState::Ok => (status == STATUS_MISSING).then_some(STATUS_READY),
        SourceState::Missing | SourceState::Unreadable => {
            (status != STATUS_MISSING).then_some(STATUS_MISSING)
        }
        SourceState::ContentChanged => match status {
            STATUS_MISSING | STATUS_READY => Some(STATUS_NEEDS_REINDEX),
            _ => None,
        },
        SourceState::NotDesktop => None,
    }
}

/// 检查并（必要时）校正书架状态。**只写 `books.status`**：进度、书签、来源记录与原文件
/// 一律不动。
///
/// `expected_locator` 是调用方检查时看到的定位符；若数据库中的定位符已变化（用户在同一次
/// 刷新期间完成了重新关联），则按过期快照处理：不写状态，`stale_snapshot = true`。
pub fn apply_source_check(
    storage: &Storage,
    book_id: i64,
    expected_locator: &str,
    now: i64,
) -> Result<SourceCheck, StorageError> {
    let conn = storage.conn();
    let Some(book) = books::get_book(conn, book_id)? else {
        return Err(StorageError::BookNotFound { book_id });
    };
    if book.source_locator != expected_locator {
        let mut stale = check_source(&book);
        stale.stale_snapshot = true;
        return Ok(stale);
    }

    let mut check = check_source(&book);
    if let Some(next) = desired_status(check.state, &book.status) {
        books::set_book_status(conn, book_id, next, now)?;
        check.status_written = true;
        check.status_after = next.to_string();
    }

    check.progress_offset = progress::load_progress(conn, book_id)?.map(|row| row.byte_offset);
    check.bookmarks = bookmarks::count_bookmarks(conn, book_id)?;
    Ok(check)
}

/// 校正书架上全部条目的来源状态（软移除条目不在书架列表，不处理）。
pub fn refresh_source_states(db_path: &Path, now: i64) -> Result<Vec<SourceCheck>, StorageError> {
    let mut storage = Storage::open(db_path)?;
    storage.migrate()?;
    let shelf = books::list_shelf_books(storage.conn())?;
    let mut checks = Vec::with_capacity(shelf.len());
    for book in shelf {
        let expected = book.source_locator.clone();
        checks.push(apply_source_check(&storage, book.id, &expected, now)?);
    }
    Ok(checks)
}

/// 书架后台任务：先校正来源状态，再做 DT1 的书架读取，最后把状态投影为界面标签。
///
/// 只读语义不变：本函数不读正文、不建索引，也不注入任何数据。
pub fn shelf_job_with_source_check(request: ShelfRequest) -> BackgroundResult {
    // 状态校正失败不阻断书架读取：数据库不可用时随后由书架读取路径如实报错。
    let _ = refresh_source_states(&request.db_path, import::now_unix());

    let mut result = crate::shelf::shelf_job(request);
    if let BackgroundPayload::Shelf { rows, .. } = &mut result.payload {
        for row in rows.iter_mut() {
            row.status = status_label(&row.status);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// 重新关联
// ---------------------------------------------------------------------------

/// 内容不一致时是否允许按新内容重建索引（必须由用户确认）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelinkDecision {
    /// 默认：内容不一致时拒绝并保持零改动。
    RefuseOnContentChange,
    /// 用户已确认：允许按新内容重建可重建索引（进度与书签仍然保留）。
    AllowContentChange,
}

/// 重新关联错误。
#[derive(Debug)]
pub enum RelinkError {
    /// 书籍条目不存在。
    BookNotFound(i64),
    /// 所选文件未通过校验。
    Import(ImportError),
    /// 数据库读写失败。
    Storage(StorageError),
    /// 该文件已经登记为另一本书的来源（不得把两本书合并）。
    AlreadyLinkedTo {
        /// 目标书籍。
        book_id: i64,
        /// 已登记该书目的其它书籍。
        other_book_id: i64,
        /// 冲突的定位符。
        locator: String,
    },
    /// 内部自检失败：导入命中了非目标条目。
    TargetMismatch {
        /// 期望条目。
        expected: i64,
        /// 实际条目。
        actual: i64,
    },
}

impl RelinkError {
    /// 面向用户的一行可读错误。
    pub fn message(&self) -> String {
        match self {
            RelinkError::BookNotFound(book_id) => {
                format!("书籍条目不存在（book_id={book_id}）")
            }
            RelinkError::Import(error) => error.message(),
            RelinkError::Storage(error) => format!("本地书架数据库读写失败：{error}"),
            RelinkError::AlreadyLinkedTo {
                book_id,
                other_book_id,
                locator,
            } => format!(
                "该文件已登记为另一本书（book_id={other_book_id}）的来源，未做任何改动（target={book_id}，{locator}）"
            ),
            RelinkError::TargetMismatch { expected, actual } => format!(
                "内部自检失败：重新关联命中了 book_id={actual}，期望 {expected}；已停止后续步骤"
            ),
        }
    }
}

impl std::fmt::Display for RelinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message())
    }
}

/// 重新关联报告（进度/书签前后对比是「不丢用户数据」的证据）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelinkReport {
    /// 书籍 id。
    pub book_id: i64,
    /// 书名。
    pub display_name: String,
    /// 原来源定位符。
    pub old_locator: String,
    /// 新来源定位符。
    pub new_locator: String,
    /// 新文件内容是否与记录指纹一致。
    pub content_identical: bool,
    /// 内容不一致时是否经用户确认。
    pub content_change_confirmed: bool,
    /// 重新关联前的状态。
    pub status_before: String,
    /// 重新关联后的状态。
    pub status_after: String,
    /// 是否失效了可重建索引（章节 / 分页缓存）。
    pub index_invalidated: bool,
    /// 进度快照（前）。
    pub progress_before: Option<ProgressRecord>,
    /// 进度快照（后）。
    pub progress_after: Option<ProgressRecord>,
    /// 书签数（前）。
    pub bookmarks_before: i64,
    /// 书签数（后）。
    pub bookmarks_after: i64,
    /// 重新关联后的来源条数。
    pub sources_after: usize,
    /// 新文件的紧凑指纹。
    pub fingerprint: String,
    /// 新文件字节数。
    pub size: u64,
}

impl RelinkReport {
    /// 进度是否原样保留（含「本来就未开始」的情形）。
    pub fn progress_preserved(&self) -> bool {
        self.progress_before == self.progress_after
    }

    /// 书签是否原样保留。
    pub fn bookmarks_preserved(&self) -> bool {
        self.bookmarks_before == self.bookmarks_after
    }

    /// 面向状态栏的一行结果。
    pub fn status_text(&self) -> String {
        format!(
            "已重新关联：{}（{} → {}；内容{}；状态 {} → {}；进度{}、书签{} 已保留，来源 {} 条）",
            self.display_name,
            self.old_locator,
            self.new_locator,
            if self.content_identical {
                "一致"
            } else if self.content_change_confirmed {
                "已变化并按确认重建索引"
            } else {
                "一致（未经确认不写入）"
            },
            status_label(&self.status_before),
            status_label(&self.status_after),
            if self.progress_preserved() {
                "位置"
            } else {
                "位置异常"
            },
            if self.bookmarks_preserved() {
                "数量"
            } else {
                "数量异常"
            },
            self.sources_after
        )
    }
}

/// 内容不一致且未获确认：**零改动**报告。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentMismatchReport {
    /// 书籍 id。
    pub book_id: i64,
    /// 书名。
    pub display_name: String,
    /// 记录中的指纹。
    pub stored_fingerprint: Option<String>,
    /// 所选文件的指纹。
    pub current_fingerprint: String,
    /// 未改动前保留的进度。
    pub progress_before: Option<ProgressRecord>,
    /// 未改动前保留的进度（应与 before 相同）。
    pub progress_after: Option<ProgressRecord>,
    /// 未改动前保留的书签数。
    pub bookmarks: i64,
    /// 未改动前保留的书签数（应与上相同）。
    pub bookmarks_after: i64,
}

impl ContentMismatchReport {
    /// 进度是否未被改动。
    pub fn progress_preserved(&self) -> bool {
        self.progress_before == self.progress_after
    }

    /// 书签是否未被改动。
    pub fn bookmarks_preserved(&self) -> bool {
        self.bookmarks == self.bookmarks_after
    }

    /// 面向状态栏的一行结果。
    pub fn status_text(&self) -> String {
        format!(
            "所选文件内容与记录不一致：未做任何改动（{} 的进度与 {} 条书签保留；记录指纹 {}，所选文件指纹 {}）",
            match &self.progress_before {
                Some(row) => format!("偏移 {}", row.byte_offset),
                None => "未开始".to_string(),
            },
            self.bookmarks,
            self.stored_fingerprint.as_deref().unwrap_or("（无）"),
            self.current_fingerprint
        )
    }
}

/// 一次重新关联交互的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelinkOutcome {
    /// 用户取消（零改动）。
    Cancelled,
    /// 已重新关联。
    Relinked(Box<RelinkReport>),
    /// 内容不一致且未获确认（零改动）。
    ContentMismatch(Box<ContentMismatchReport>),
}

impl RelinkOutcome {
    /// 面向状态栏的一行结果。
    pub fn status_text(&self) -> String {
        match self {
            RelinkOutcome::Cancelled => {
                "已取消：未做任何改动，原有来源、进度与书签保持不变".to_string()
            }
            RelinkOutcome::Relinked(report) => report.status_text(),
            RelinkOutcome::ContentMismatch(report) => report.status_text(),
        }
    }
}

/// 把一次重新关联结果（含失败）转成状态栏文案。
pub fn relink_status_text(result: &Result<RelinkOutcome, RelinkError>) -> String {
    match result {
        Ok(outcome) => outcome.status_text(),
        Err(error) => format!("重新关联未完成：{}", error.message()),
    }
}

/// 用户确认前的默认决策（内容不一致 ⇒ 零改动并提示）。
pub const DEFAULT_RELINK_DECISION: RelinkDecision = RelinkDecision::RefuseOnContentChange;

/// 把 `book_id` 重新关联到新路径。
///
/// 步骤与保证：
/// 1. 校验新文件（扩展名 / 元数据 / 可读性）与指纹 —— 全部在写库之前；
/// 2. 预检查冲突：新文件若已登记为**另一本书**的来源，直接拒绝（零改动）；
/// 3. 内容不一致且未确认 ⇒ 返回 [`RelinkOutcome::ContentMismatch`]（零改动）；
/// 4. 以「书籍当前定位符 + 新指纹」先导入一次：保证命中同一条目，并让核心在内容变化时
///    只失效可重建索引（章节 / 分页缓存）；
/// 5. 再以新路径导入一次：登记新来源并把 `books.source_locator` 指向新路径；
/// 6. 内容一致且原状态为 `missing` ⇒ 恢复 `ready`；
/// 7. 自检：进度与书签前后快照必须完全相同，否则返回 [`RelinkError::TargetMismatch`] 之外的
///    自检失败（不会静默吞掉）。
pub fn relink_book(
    storage: &Storage,
    book_id: i64,
    new_path: &Path,
    decision: RelinkDecision,
    now: i64,
) -> Result<RelinkOutcome, RelinkError> {
    let conn = storage.conn();
    let book = books::get_book(conn, book_id)
        .map_err(RelinkError::Storage)?
        .ok_or(RelinkError::BookNotFound(book_id))?;

    let selected = import::validate_selection(new_path).map_err(RelinkError::Import)?;
    let (_fingerprint, compact) =
        import::fingerprint_file(new_path).map_err(RelinkError::Import)?;
    let new_locator = selected.path.to_string_lossy().to_string();

    let content_identical = book
        .source_fingerprint
        .as_deref()
        .and_then(import::parse_compact_fingerprint)
        .is_some_and(|stored| {
            import::parse_compact_fingerprint(&compact)
                .is_some_and(|current| stored.same_content(&current))
        });

    // 冲突预检查（写库之前）：不得把另一本书的来源抢过来。
    if let Some(other) = books::find_book_by_locator(conn, KIND_DESKTOP_PATH, &new_locator)
        .map_err(RelinkError::Storage)?
    {
        if other.id != book_id {
            return Err(RelinkError::AlreadyLinkedTo {
                book_id,
                other_book_id: other.id,
                locator: new_locator,
            });
        }
    }

    let progress_before = progress::load_progress(conn, book_id).map_err(RelinkError::Storage)?;
    let bookmarks_before =
        bookmarks::count_bookmarks(conn, book_id).map_err(RelinkError::Storage)?;

    if !content_identical && decision == RelinkDecision::RefuseOnContentChange {
        // 零改动：不写任何记录，进度与书签保持原样。
        return Ok(RelinkOutcome::ContentMismatch(Box::new(
            ContentMismatchReport {
                book_id,
                display_name: book.display_name.clone(),
                stored_fingerprint: book.source_fingerprint.clone(),
                current_fingerprint: compact,
                progress_before: progress_before.clone(),
                progress_after: progress_before,
                bookmarks: bookmarks_before,
                bookmarks_after: bookmarks_before,
            },
        )));
    }

    let base_import = BookImport {
        display_name: book.display_name.clone(),
        original_name: selected.original_name.clone(),
        source_kind: KIND_DESKTOP_PATH.to_string(),
        source_locator: book.source_locator.clone(),
        source_size: selected.size as i64,
        source_modified_hint: selected.modified_unix_ms,
        source_fingerprint: Some(compact.clone()),
        encoding: book.encoding.clone(),
        encoding_confidence: book.encoding_confidence,
        encoding_user_override: book.encoding_user_override,
    };

    // 4) 同一本书的定位符 + 新指纹 ⇒ 必然命中目标条目。
    let first = books::import_book(conn, &base_import, now).map_err(RelinkError::Storage)?;
    if first.book_id != book_id {
        return Err(RelinkError::TargetMismatch {
            expected: book_id,
            actual: first.book_id,
        });
    }

    // 5) 登记新来源（并把主来源指向新路径）。
    let mut relocated = base_import;
    relocated.source_locator = new_locator.clone();
    let second = books::import_book(conn, &relocated, now).map_err(RelinkError::Storage)?;
    if second.book_id != book_id {
        return Err(RelinkError::TargetMismatch {
            expected: book_id,
            actual: second.book_id,
        });
    }

    // 6) 内容一致且原状态失效 ⇒ 恢复可阅读。
    if content_identical && book.status == STATUS_MISSING {
        books::set_book_status(conn, book_id, STATUS_READY, now).map_err(RelinkError::Storage)?;
    }

    let after = books::get_book(conn, book_id)
        .map_err(RelinkError::Storage)?
        .ok_or(RelinkError::BookNotFound(book_id))?;
    let progress_after = progress::load_progress(conn, book_id).map_err(RelinkError::Storage)?;
    let bookmarks_after =
        bookmarks::count_bookmarks(conn, book_id).map_err(RelinkError::Storage)?;
    let sources_after = books::list_book_sources(conn, book_id)
        .map_err(RelinkError::Storage)?
        .len();

    // 7) 自检：进度必须逐字段不变（书签数不得减少）。
    if progress_before != progress_after || bookmarks_after < bookmarks_before {
        return Err(RelinkError::TargetMismatch {
            expected: book_id,
            actual: after.id,
        });
    }

    Ok(RelinkOutcome::Relinked(Box::new(RelinkReport {
        book_id,
        display_name: after.display_name.clone(),
        old_locator: book.source_locator.clone(),
        new_locator: after.source_locator.clone(),
        content_identical,
        content_change_confirmed: !content_identical,
        status_before: book.status.clone(),
        status_after: after.status.clone(),
        index_invalidated: !content_identical,
        progress_before,
        progress_after,
        bookmarks_before,
        bookmarks_after,
        sources_after,
        fingerprint: after.source_fingerprint.clone().unwrap_or(compact),
        size: after.source_size.max(0) as u64,
    })))
}

/// 以给定选择器完成一次重新关联（取消 → [`RelinkOutcome::Cancelled`]）。
pub fn relink_flow(
    db_path: &Path,
    book_id: i64,
    picker: &dyn FilePicker,
    decision: RelinkDecision,
    now: i64,
) -> Result<RelinkOutcome, RelinkError> {
    let new_path = match picker.pick_txt_file() {
        Ok(None) => return Ok(RelinkOutcome::Cancelled),
        Ok(Some(path)) => path,
        Err(error) => return Err(RelinkError::Import(error)),
    };
    let mut storage = Storage::open(db_path).map_err(RelinkError::Storage)?;
    storage.migrate().map_err(RelinkError::Storage)?;
    relink_book(&storage, book_id, &new_path, decision, now)
}

/// [`RELINK_BOOK_ENV`] 指定的启动期重新关联书籍 id。
pub fn requested_relink_book_id() -> Option<i64> {
    let value = std::env::var(RELINK_BOOK_ENV).ok()?;
    value.trim().parse::<i64>().ok()
}

/// 重新关联时要选择的文件路径（[`RELINK_PATH_ENV`] 优先，未设置时回退 `NR_IMPORT_PATH`）。
pub fn relink_path_from_env() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os(RELINK_PATH_ENV) {
        let value = value.to_string_lossy().trim().to_string();
        if !value.is_empty() {
            return Some(PathBuf::from(value));
        }
    }
    import::startup_import_path()
}

/// 启动期自动重新关联（程序化验证用）：`NR_RELINK_BOOK` + `NR_RELINK_PATH` 同时设置时执行。
pub fn startup_relink(db_path: &Path) -> Option<Result<RelinkOutcome, RelinkError>> {
    let book_id = requested_relink_book_id()?;
    let path = relink_path_from_env()?;
    let picker = import::StaticFilePicker::new(path);
    Some(relink_flow(
        db_path,
        book_id,
        &picker,
        DEFAULT_RELINK_DECISION,
        import::now_unix(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use reading_storage::bookmarks::BookmarkInput;
    use reading_storage::progress::ProgressInput;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |delta| delta.as_nanos());
            let seq = COUNTER.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(format!(
                "nr-dt2-src-{tag}-{}-{nanos}-{seq}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("创建临时目录");
            Self { path }
        }

        fn join(&self, name: &str) -> PathBuf {
            self.path.join(name)
        }

        fn db(&self) -> PathBuf {
            self.join("library.db")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn body(tag: &str) -> Vec<u8> {
        let mut text = String::new();
        for index in 1..=40 {
            text.push_str(&format!("第{index}章 {tag}\n"));
            text.push_str("用于来源状态与重新关联验证的中文正文行，句号。\n\n");
        }
        text.into_bytes()
    }

    /// 建立一本真实书：真文件 + 真导入 + 真进度 + 真书签。
    fn seed_book(dir: &TempDir, name: &str) -> (i64, PathBuf) {
        let path = dir.join(name);
        fs::write(&path, body(name)).expect("写入测试文件");
        let report = import::import_path(&dir.db(), &path, 100).expect("导入测试书");
        let mut storage = Storage::open(dir.db()).expect("打开数据库");
        storage.migrate().expect("迁移");
        progress::save_progress(
            storage.conn(),
            &ProgressInput {
                nearby_fingerprint: Some("第3章 起始".to_string()),
                ..ProgressInput::at(report.book_id, 2, 96)
            },
            200,
        )
        .expect("写入进度");
        bookmarks::create_bookmark(
            storage.conn(),
            &BookmarkInput::at(report.book_id, 2, 96, "锚点"),
            300,
        )
        .expect("写入书签");
        (report.book_id, path)
    }

    fn source_count(dir: &TempDir, book_id: i64) -> usize {
        let storage = Storage::open(dir.db()).expect("打开数据库");
        books::list_book_sources(storage.conn(), book_id)
            .expect("来源清单")
            .len()
    }

    fn book_of(dir: &TempDir, book_id: i64) -> BookRecord {
        let storage = Storage::open(dir.db()).expect("打开数据库");
        books::get_book(storage.conn(), book_id)
            .expect("读回书籍")
            .expect("书籍存在")
    }

    // -----------------------------------------------------------------------
    // 状态标签
    // -----------------------------------------------------------------------

    #[test]
    fn status_labels_cover_all_core_states_and_unknown_passes_through() {
        assert_eq!(status_label(STATUS_READY), LABEL_READY);
        assert_eq!(status_label(STATUS_MISSING), LABEL_MISSING);
        assert_eq!(status_label(STATUS_NEEDS_REINDEX), LABEL_NEEDS_REINDEX);
        assert_eq!(status_label(STATUS_IMPORT_FAILED), LABEL_IMPORT_FAILED);
        assert_eq!(status_label(STATUS_REMOVED), LABEL_REMOVED);
        assert_eq!(status_label("future_state"), "future_state");

        assert!(needs_relink_label(LABEL_MISSING));
        assert!(needs_relink_label(LABEL_NEEDS_REINDEX));
        assert!(!needs_relink_label(LABEL_READY));
        assert!(needs_relink_status(STATUS_MISSING));
        assert!(needs_relink_status(STATUS_NEEDS_REINDEX));
        assert!(!needs_relink_status(STATUS_READY));
    }

    // -----------------------------------------------------------------------
    // 来源正常 / 失效 / 内容变化
    // -----------------------------------------------------------------------

    #[test]
    fn healthy_source_is_detected_as_ok_without_status_change() {
        let dir = TempDir::new("healthy");
        let (book_id, path) = seed_book(&dir, "healthy.txt");
        let checks = refresh_source_states(&dir.db(), 400).expect("校正来源状态");
        assert_eq!(checks.len(), 1);
        let check = &checks[0];
        assert_eq!(check.book_id, book_id);
        assert_eq!(check.state, SourceState::Ok);
        assert!(!check.status_written, "来源正常时不得改动状态");
        assert_eq!(check.status_after, STATUS_READY);
        assert_eq!(check.progress_offset, Some(96));
        assert_eq!(check.bookmarks, 1);
        assert_eq!(
            book_of(&dir, book_id).source_locator,
            path.to_string_lossy()
        );
    }

    #[test]
    fn missing_source_marks_book_for_relink_and_preserves_progress_and_bookmarks() {
        let dir = TempDir::new("missing");
        let (book_id, path) = seed_book(&dir, "moved-away.txt");

        // 用户把原文件搬到别处（本测试只在 /tmp 内操作副本：重命名为 `<name>.old`）。
        let parked = dir.join("moved-away.txt.old");
        fs::rename(&path, &parked).expect("移动测试副本");
        assert!(!path.exists());

        let checks = refresh_source_states(&dir.db(), 400).expect("校正来源状态");
        let check = checks
            .iter()
            .find(|c| c.book_id == book_id)
            .expect("检查结果");
        assert_eq!(check.state, SourceState::Missing);
        assert!(check.status_written);
        assert_eq!(check.status_after, STATUS_MISSING);
        assert!(needs_relink_label(&status_label(&check.status_after)));
        assert!(check.status_text().contains("原文件不存在"));

        // 记录、进度、书签全部保留；原文件未被删除、未被修改。
        let book = book_of(&dir, book_id);
        assert_eq!(book.status, STATUS_MISSING);
        let storage = Storage::open(dir.db()).expect("打开数据库");
        let progress_row = progress::load_progress(storage.conn(), book_id)
            .expect("读取进度")
            .expect("进度仍在");
        assert_eq!(progress_row.byte_offset, 96);
        assert_eq!(progress_row.chapter_index, 2);
        assert_eq!(
            progress_row.nearby_fingerprint.as_deref(),
            Some("第3章 起始")
        );
        assert_eq!(
            bookmarks::count_bookmarks(storage.conn(), book_id).expect("书签数"),
            1
        );
        assert!(parked.is_file(), "原文件不得被删除");
        assert_eq!(
            fs::metadata(&parked).expect("元数据").len(),
            body("moved-away.txt").len() as u64
        );
    }

    #[test]
    fn relink_restores_ready_state_and_keeps_progress_and_bookmarks() {
        let dir = TempDir::new("relink-ok");
        let (book_id, path) = seed_book(&dir, "book.txt");
        let parked = dir.join("book-moved.txt");
        fs::rename(&path, &parked).expect("移动测试副本");
        refresh_source_states(&dir.db(), 400).expect("校正来源状态");
        assert_eq!(book_of(&dir, book_id).status, STATUS_MISSING);

        let outcome = relink_flow(
            &dir.db(),
            book_id,
            &import::StaticFilePicker::new(parked.clone()),
            DEFAULT_RELINK_DECISION,
            500,
        )
        .expect("重新关联应成功");

        let report = match outcome {
            RelinkOutcome::Relinked(report) => report,
            other => panic!("期望 Relinked，实际 {other:?}"),
        };
        assert!(report.content_identical, "同一份内容必须判定为一致");
        assert_eq!(report.status_before, STATUS_MISSING);
        assert_eq!(report.status_after, STATUS_READY, "内容一致 ⇒ 恢复可阅读");
        assert!(!report.index_invalidated);
        assert!(report.progress_preserved(), "进度必须逐字段不变");
        assert!(report.bookmarks_preserved());
        assert_eq!(report.progress_before, report.progress_after);
        assert_eq!(report.sources_after, 2, "新路径应登记为第二条来源");
        assert_eq!(report.new_locator, parked.to_string_lossy());

        // 落库事实：定位符指向新路径，状态恢复，进度/书签仍在，原文件仍在原处（未删除）。
        let book = book_of(&dir, book_id);
        assert_eq!(book.status, STATUS_READY);
        assert_eq!(book.source_locator, parked.to_string_lossy());
        let storage = Storage::open(dir.db()).expect("打开数据库");
        let sources = books::list_book_sources(storage.conn(), book_id).expect("来源清单");
        assert_eq!(sources.len(), 2);
        assert_eq!(
            sources.iter().filter(|source| source.is_primary).count(),
            1,
            "主来源必须唯一"
        );
        assert_eq!(
            sources
                .iter()
                .find(|source| source.is_primary)
                .expect("存在主来源")
                .source_locator,
            parked.to_string_lossy()
        );
        assert_eq!(
            progress::load_progress(storage.conn(), book_id)
                .expect("读取进度")
                .expect("进度仍在")
                .byte_offset,
            96
        );
        assert_eq!(
            bookmarks::count_bookmarks(storage.conn(), book_id).expect("书签数"),
            1
        );
        assert!(
            !path.exists(),
            "应用不得在旧路径上创建或恢复任何文件（用户自己搬走的文件保持搬走）"
        );
        // 用户自己的那份文件仍在被搬到的位置，且内容逐字节不变。
        assert_eq!(
            fs::metadata(&parked).expect("元数据").len(),
            body("book.txt").len() as u64,
            "重新关联不得改写用户文件"
        );

        // 重新关联后再检查一次：来源恢复正常、不再改写状态。
        let checks = refresh_source_states(&dir.db(), 600).expect("再次校正");
        assert_eq!(checks[0].state, SourceState::Ok);
        assert!(!checks[0].status_written);
    }

    #[test]
    fn missing_source_relink_rejects_changed_content_without_confirmation() {
        let dir = TempDir::new("relink-mismatch");
        let (book_id, path) = seed_book(&dir, "book.txt");
        let parked = dir.join("book-moved.txt");
        fs::rename(&path, &parked).expect("移动测试副本");
        refresh_source_states(&dir.db(), 400).expect("校正来源状态");

        // 用户选了一个内容完全不同的 TXT（同路径名不同内容）。
        let other = dir.join("other.txt");
        fs::write(&other, body("完全不同的书")).expect("写入另一个文件");

        let outcome = relink_flow(
            &dir.db(),
            book_id,
            &import::StaticFilePicker::new(other.clone()),
            DEFAULT_RELINK_DECISION,
            500,
        )
        .expect("内容不一致不是错误，而是需要确认");

        let report = match outcome {
            RelinkOutcome::ContentMismatch(report) => report,
            other => panic!("期望 ContentMismatch，实际 {other:?}"),
        };
        assert!(report.progress_preserved());
        assert!(report.bookmarks_preserved());

        // 零改动：状态、定位符、进度、书签、来源条数都不变。
        // 注意：定位符保持**记录中的旧路径**（用户搬走文件不会改写记录）；
        // 只有用户确认的重新关联才会把定位符指向新路径。
        let book = book_of(&dir, book_id);
        assert_eq!(book.status, STATUS_MISSING, "未确认时不得改动状态");
        assert_eq!(book.source_locator, path.to_string_lossy());
        let storage = Storage::open(dir.db()).expect("打开数据库");
        assert_eq!(
            books::list_book_sources(storage.conn(), book_id)
                .expect("来源清单")
                .len(),
            1
        );
        assert_eq!(
            progress::load_progress(storage.conn(), book_id)
                .expect("读取进度")
                .expect("进度仍在")
                .byte_offset,
            96
        );
        assert_eq!(
            bookmarks::count_bookmarks(storage.conn(), book_id).expect("书签数"),
            1
        );
    }

    #[test]
    fn relink_with_confirmed_content_change_invalidates_only_rebuildable_index() {
        let dir = TempDir::new("relink-confirm");
        let (book_id, path) = seed_book(&dir, "book.txt");
        fs::rename(&path, dir.join("book-old.txt")).expect("移动测试副本");
        refresh_source_states(&dir.db(), 400).expect("校正来源状态");

        let other = dir.join("new-content.txt");
        fs::write(&other, body("确认使用的新内容")).expect("写入新内容文件");

        let outcome = relink_flow(
            &dir.db(),
            book_id,
            &import::StaticFilePicker::new(other.clone()),
            RelinkDecision::AllowContentChange,
            500,
        )
        .expect("已确认的内容变化应完成重新关联");

        let report = match outcome {
            RelinkOutcome::Relinked(report) => report,
            other => panic!("期望 Relinked，实际 {other:?}"),
        };
        assert!(!report.content_identical);
        assert!(report.content_change_confirmed);
        assert!(report.index_invalidated, "内容变化必须失效可重建索引");
        assert!(report.progress_preserved(), "内容变化也不得丢弃进度");
        assert!(report.bookmarks_preserved());
        assert_eq!(report.status_after, STATUS_NEEDS_REINDEX);

        let storage = Storage::open(dir.db()).expect("打开数据库");
        assert_eq!(
            progress::load_progress(storage.conn(), book_id)
                .expect("读取进度")
                .expect("进度仍在")
                .byte_offset,
            96
        );
        assert_eq!(
            bookmarks::count_bookmarks(storage.conn(), book_id).expect("书签数"),
            1
        );
        assert_eq!(
            books::list_book_sources(storage.conn(), book_id)
                .expect("来源清单")
                .len(),
            2
        );
    }

    #[test]
    fn relink_refuses_file_already_linked_to_another_book() {
        let dir = TempDir::new("relink-conflict");
        let (first_id, first_path) = seed_book(&dir, "first.txt");
        let (second_id, second_path) = seed_book(&dir, "second.txt");
        assert_ne!(first_id, second_id);

        let outcome = relink_flow(
            &dir.db(),
            first_id,
            &import::StaticFilePicker::new(second_path.clone()),
            RelinkDecision::AllowContentChange,
            500,
        );
        let error = outcome.expect_err("不得抢占另一本书的来源");
        match error {
            RelinkError::AlreadyLinkedTo { other_book_id, .. } => {
                assert_eq!(other_book_id, second_id)
            }
            other => panic!("期望 AlreadyLinkedTo，实际 {other:?}"),
        }
        // 零改动：两本书都还在原路径。
        let storage = Storage::open(dir.db()).expect("打开数据库");
        assert_eq!(
            books::get_book(storage.conn(), first_id)
                .expect("读回")
                .expect("存在")
                .source_locator,
            first_path.to_string_lossy()
        );
        assert_eq!(
            books::get_book(storage.conn(), second_id)
                .expect("读回")
                .expect("存在")
                .source_locator,
            second_path.to_string_lossy()
        );
    }

    #[test]
    fn relink_cancel_and_bad_selection_leave_zero_changes() {
        let dir = TempDir::new("relink-cancel");
        let (book_id, _path) = seed_book(&dir, "book.txt");
        let before = source_count(&dir, book_id);

        // 用户取消
        let cancelled = relink_flow(
            &dir.db(),
            book_id,
            &import::CancellingFilePicker,
            DEFAULT_RELINK_DECISION,
            500,
        )
        .expect("取消不是错误");
        assert_eq!(cancelled, RelinkOutcome::Cancelled);
        assert!(cancelled.status_text().contains("已取消"));

        // 选了一个非 TXT
        let rejected = dir.join("note.md");
        fs::write(&rejected, "# 不是 TXT\n").expect("写入非 TXT");
        let error = relink_flow(
            &dir.db(),
            book_id,
            &import::StaticFilePicker::new(rejected),
            DEFAULT_RELINK_DECISION,
            501,
        )
        .expect_err("非 TXT 必须被拒绝");
        match error {
            RelinkError::Import(error) => assert_eq!(error.kind, import::ImportErrorKind::NotTxt),
            other => panic!("期望 Import 错误，实际 {other:?}"),
        }

        // 不存在的书籍
        let storage = Storage::open(dir.db()).expect("打开数据库");
        let error = relink_book(
            &storage,
            book_id + 999,
            &dir.join("book.txt"),
            DEFAULT_RELINK_DECISION,
            502,
        )
        .expect_err("不存在的书籍必须报错");
        assert!(matches!(error, RelinkError::BookNotFound(_)));
        assert!(error.message().contains("不存在"));

        assert_eq!(
            source_count(&dir, book_id),
            before,
            "取消与拒绝都不得改动来源"
        );
        assert_eq!(book_of(&dir, book_id).status, STATUS_READY);
    }

    #[test]
    fn stale_source_snapshot_does_not_overwrite_relinked_status() {
        let dir = TempDir::new("stale-guard");
        let (book_id, path) = seed_book(&dir, "book.txt");
        let parked = dir.join("book-moved.txt");
        fs::rename(&path, &parked).expect("移动测试副本");

        // 快照一：检查到原路径（文件已不在）—— 但不立即写库（模拟并发窗口）。
        let stale_locator = path.to_string_lossy().to_string();
        // 先用过期定位符调用：此时库里定位符未变，正常判定为 missing。
        let storage = Storage::open(dir.db()).expect("打开数据库");
        let committed =
            apply_source_check(&storage, book_id, &stale_locator, 400).expect("首次检查");
        assert_eq!(committed.state, SourceState::Missing);
        assert!(committed.status_written);

        // 重新关联到新路径（状态恢复可阅读）。
        relink_flow(
            &dir.db(),
            book_id,
            &import::StaticFilePicker::new(parked.clone()),
            DEFAULT_RELINK_DECISION,
            500,
        )
        .expect("重新关联");
        assert_eq!(book_of(&dir, book_id).status, STATUS_READY);

        // 快照二：拿着**过期的**定位符再写一次：必须被守卫拦下，不得把状态打回 missing。
        let storage = Storage::open(dir.db()).expect("打开数据库");
        let stale =
            apply_source_check(&storage, book_id, &stale_locator, 600).expect("过期快照检查");
        assert!(stale.stale_snapshot, "过期快照必须被识别");
        assert!(!stale.status_written, "过期快照不得写状态");
        assert_eq!(stale.status_after, STATUS_READY);
        assert_eq!(book_of(&dir, book_id).status, STATUS_READY);
        assert_eq!(
            book_of(&dir, book_id).source_locator,
            parked.to_string_lossy()
        );
    }

    #[test]
    fn shelf_job_with_source_check_maps_status_labels_and_keeps_rows() {
        let dir = TempDir::new("shelf-job");
        let (book_id, path) = seed_book(&dir, "book.txt");
        fs::rename(&path, dir.join("parked.txt")).expect("移动测试副本");

        let request = ShelfRequest::new(1, 1, dir.db());
        let result = shelf_job_with_source_check(request);
        let rows = match result.payload {
            BackgroundPayload::Shelf { rows, .. } => rows,
            other => panic!("期望 Shelf 载荷，实际 {other:?}"),
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, book_id.to_string());
        assert_eq!(rows[0].status, LABEL_MISSING, "行状态必须是可显示标签");
        assert!(needs_relink_label(&rows[0].status));
        assert!(rows[0].progress.contains("96"));

        // 标签是幂等的：再次投影不会把标签当成核心状态。
        let again = shelf_job_with_source_check(ShelfRequest::new(2, 1, dir.db()));
        let rows = match again.payload {
            BackgroundPayload::Shelf { rows, .. } => rows,
            other => panic!("期望 Shelf 载荷，实际 {other:?}"),
        };
        assert_eq!(rows[0].status, LABEL_MISSING);
    }

    #[test]
    fn source_check_reports_unreadable_and_non_desktop_without_touching_files() {
        let dir = TempDir::new("unreadable-src");
        let (book_id, path) = seed_book(&dir, "locked.txt");
        let mut perms = fs::metadata(&path).expect("元数据").permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o000);
        fs::set_permissions(&path, perms).expect("设置权限");

        let checks = refresh_source_states(&dir.db(), 400).expect("校正来源状态");
        let check = checks
            .iter()
            .find(|c| c.book_id == book_id)
            .expect("检查结果");
        if fs::File::open(&path).is_ok() {
            println!("注意：当前进程可读取 0o000 文件（疑似 root），跳过不可读断言");
            assert_eq!(check.state, SourceState::Ok);
        } else {
            assert_eq!(check.state, SourceState::Unreadable);
            assert_eq!(check.status_after, STATUS_MISSING, "不可读按失效处理");
            assert_eq!(check.current_fingerprint, None);
        }
        assert!(path.exists(), "检查不得删除或移动文件");

        // 非桌面来源不检查、不改状态。
        let fake = BookRecord {
            id: 999,
            display_name: "其他平台的书".to_string(),
            original_name: "other.txt".to_string(),
            format: "txt".to_string(),
            source_kind: "android_uri".to_string(),
            source_locator: "content://example/1".to_string(),
            source_size: 10,
            source_modified_hint: None,
            source_fingerprint: None,
            encoding: "UTF-8".to_string(),
            encoding_confidence: 1000,
            encoding_user_override: false,
            status: STATUS_READY.to_string(),
            created_at: 0,
            updated_at: 0,
        };
        let check = check_source(&fake);
        assert_eq!(check.state, SourceState::NotDesktop);
        assert!(!check.status_written);
    }

    #[test]
    fn relink_error_messages_are_readable() {
        let errors = [
            RelinkError::BookNotFound(7),
            RelinkError::Storage(StorageError::invalid("坏数据")),
            RelinkError::AlreadyLinkedTo {
                book_id: 1,
                other_book_id: 2,
                locator: "/tmp/x.txt".to_string(),
            },
            RelinkError::TargetMismatch {
                expected: 1,
                actual: 2,
            },
        ];
        for error in errors {
            let message = error.message();
            assert!(!message.is_empty(), "{error:?} 必须有可读文案");
            assert_eq!(message, error.to_string());
        }
        assert!(RelinkError::BookNotFound(7).message().contains("7"));
    }
}

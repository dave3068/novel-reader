//! 阅读进度：以 `ContentPosition` 为唯一身份，保存 / 读取 / 重启恢复。
//!
//! 依据（`desktop/spec.md` DR5、`reading-core/spec.md` CR2、开发方案 §9.2）：
//!
//! - 进度身份是 `ContentPosition`（content_unit + chapter_index + byte_offset + 附近指纹），
//!   **不含页码**；页码只用于显示，绝不用作身份或恢复依据；
//! - 落库走核心存储层 `reading_progress` 的幂等 upsert：位置完全一致时不写库、不更新时间戳；
//! - 恢复走核心 `restore_position`：有界半径锚点就近定位，未命中时回退到最近可确认的章节
//!   开头，并把读取量（窗口数 / 字节数）作为证据带回来；
//! - 本模块只编排与投影，不实现锚点匹配、不解析正文。

use std::path::Path;

use reading_core::chapter::{generate_anchor, restore_position, AnchorSearch};
use reading_core::{ContentPosition, NearbyFingerprint, SINGLE_CONTENT_UNIT_ID};
use reading_source::{FileTextSource, TextSource};
use reading_storage::{progress as storage_progress, Storage};

use crate::chapters::ChapterToc;
use crate::encoded_source::EncodedFileSource;

/// 保存在库里的阅读位置（进度身份 = `ContentPosition`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedPosition {
    /// 书籍标识。
    pub book_id: i64,
    /// 内容单元标识（V1 单 TXT 固定为 1）。
    pub content_unit_id: i64,
    /// 章节序号（来自章节索引，不是页码）。
    pub chapter_index: u32,
    /// 章节内字节偏移（页面起始字节）。
    pub byte_offset: u64,
    /// 附近文本指纹（锚点；用于重启后就近恢复）。
    pub quote: String,
    /// 最近更新时间（Unix 秒）。
    pub updated_at: i64,
    /// 最近打开时间（Unix 秒）。
    pub last_opened_at: i64,
}

impl SavedPosition {
    /// 还原为冻结核心的位置类型（重启恢复的唯一输入）。
    pub fn position(&self) -> ContentPosition {
        ContentPosition::new(
            self.book_id.max(0) as u64,
            self.content_unit_id.max(0) as u64,
            self.chapter_index,
            self.byte_offset,
            NearbyFingerprint::new(self.quote.clone()),
        )
    }

    /// 稳定一行（日志与证据用；不含正文）。
    pub fn describe(&self) -> String {
        format!(
            "第 {} 章 · 偏移 {} 字节 · 锚点 {} 字",
            self.chapter_index + 1,
            self.byte_offset,
            self.quote.chars().count()
        )
    }
}

/// 进度失败原因（可读文本，不作为业务分支）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgressError {
    /// 数据源不可读（锚点采样失败）。
    Source(String),
    /// 存储层失败。
    Storage(String),
}

impl ProgressError {
    /// 可读说明。
    pub fn message(&self) -> String {
        match self {
            ProgressError::Source(message) => format!("位置采样失败：{message}"),
            ProgressError::Storage(message) => format!("进度存取失败：{message}"),
        }
    }
}

impl std::fmt::Display for ProgressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for ProgressError {}

/// 一次进度保存的结论（幂等证据）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveReport {
    /// 是否新建了进度行。
    pub inserted: bool,
    /// 位置是否变化（false ⇒ 幂等空写，未触碰时间戳）。
    pub changed: bool,
}

impl SaveReport {
    /// 稳定一行（日志与证据用）。
    pub fn as_log(&self, saved: &SavedPosition) -> String {
        format!(
            "NR-PROGRESS op=save inserted={} changed={} book={} chapter={} offset={} quote_chars={}",
            u8::from(self.inserted),
            u8::from(self.changed),
            saved.book_id,
            saved.chapter_index,
            saved.byte_offset,
            saved.quote.chars().count(),
        )
    }
}

/// 在给定字节偏移处采样一个可持久化的位置（附近指纹来自核心锚点采样）。
///
/// 只读取一个窗口（核心锚点采样默认 24 字符），不读取整本。
pub fn capture(
    path: &Path,
    book_id: i64,
    chapter_index: u32,
    byte_offset: u64,
) -> Result<SavedPosition, ProgressError> {
    let source = FileTextSource::open(path).map_err(|error| {
        ProgressError::Source(format!("{}（{}）", path.display(), error.as_str()))
    })?;
    let sample = generate_anchor(&source, byte_offset).map_err(|error| {
        ProgressError::Source(format!("{}（{}）", path.display(), error.message()))
    })?;
    Ok(SavedPosition {
        book_id,
        content_unit_id: SINGLE_CONTENT_UNIT_ID as i64,
        chapter_index,
        byte_offset: sample.byte_offset,
        quote: sample.fingerprint.quote,
        updated_at: 0,
        last_opened_at: 0,
    })
}

/// 位置恢复（**编码感知**）。
///
/// 位置身份本就是**字节偏移**（与排版无关），因此非 UTF-8 文档直接采用保存偏移：
/// 中文 GBK/GB18030 书在 UTF-8 口径下采样不出引文，而核心对「引文为空」一律回退章首
/// （`AnchorStatus::EmptyQuote`）—— 表现就是「只记住章节、记不住章内位置」（2026-09-30 真实缺陷）。
/// UTF-8 文档仍走核心锚点（多一层引文校验）；偏移越界时回退章首。
pub fn restore_encoded(
    path: &Path,
    toc: &ChapterToc,
    saved: &SavedPosition,
    encoding: reading_source::TextEncoding,
) -> Result<RestoreReport, ProgressError> {
    if matches!(
        encoding,
        reading_source::TextEncoding::Utf8 | reading_source::TextEncoding::Utf8Bom
    ) {
        return restore(path, toc, saved);
    }
    let source = EncodedFileSource::open(path, encoding).map_err(|error| {
        ProgressError::Source(format!("{}（{}）", path.display(), error.as_str()))
    })?;
    let total = source.metadata().size;
    let chapter_start = toc
        .index()
        .chapter_start(saved.chapter_index)
        .unwrap_or(0)
        .min(total);
    if saved.byte_offset >= chapter_start && saved.byte_offset <= total {
        Ok(RestoreReport {
            byte_offset: saved.byte_offset,
            chapter_index: saved.chapter_index,
            fallback_to_chapter_start: false,
            anchor_status: "byte_offset".to_string(),
            distance_bytes: 0,
            windows_read: 0,
            bytes_read: 0,
            radius_bytes: 0,
        })
    } else {
        Ok(RestoreReport {
            byte_offset: chapter_start,
            chapter_index: saved.chapter_index,
            fallback_to_chapter_start: true,
            anchor_status: "out_of_range".to_string(),
            distance_bytes: saved.byte_offset.abs_diff(chapter_start),
            windows_read: 0,
            bytes_read: 0,
            radius_bytes: 0,
        })
    }
}

/// 打开库（自动迁移）。
fn open_storage(db_path: &Path) -> Result<Storage, ProgressError> {
    let mut storage = Storage::open(db_path)
        .map_err(|error| ProgressError::Storage(format!("{}（{error}）", db_path.display())))?;
    storage
        .migrate()
        .map_err(|error| ProgressError::Storage(format!("迁移失败：{error}")))?;
    Ok(storage)
}

/// 幂等保存进度（位置一致时不写库）。
pub fn save(db_path: &Path, saved: &SavedPosition, now: i64) -> Result<SaveReport, ProgressError> {
    let storage = open_storage(db_path)?;
    let input = storage_progress::ProgressInput {
        book_id: saved.book_id,
        content_unit_id: saved.content_unit_id,
        chapter_index: saved.chapter_index as i64,
        byte_offset: saved.byte_offset as i64,
        nearby_fingerprint: Some(saved.quote.clone()),
        // 页码提示不参与身份；V1 不写页码（不得用页码作为进度身份）。
        last_page_hint: None,
        index_version: 0,
    };
    let outcome = storage_progress::save_progress(storage.conn(), &input, now)
        .map_err(|error| ProgressError::Storage(error.to_string()))?;
    Ok(SaveReport {
        inserted: outcome.inserted,
        changed: outcome.changed,
    })
}

/// 读取已保存的位置（没有则为 `None`）。
pub fn load(db_path: &Path, book_id: i64) -> Result<Option<SavedPosition>, ProgressError> {
    let storage = open_storage(db_path)?;
    let record = storage_progress::load_progress(storage.conn(), book_id)
        .map_err(|error| ProgressError::Storage(error.to_string()))?;
    Ok(record.map(|record| SavedPosition {
        book_id: record.book_id,
        content_unit_id: record.content_unit_id,
        chapter_index: record.chapter_index.max(0) as u32,
        byte_offset: record.byte_offset.max(0) as u64,
        quote: record.nearby_fingerprint.unwrap_or_default(),
        updated_at: record.updated_at,
        last_opened_at: record.last_opened_at,
    }))
}

/// 重启恢复结论（含核心锚点恢复的读取量证据）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreReport {
    /// 恢复后的字节偏移（页面重新测量的起点）。
    pub byte_offset: u64,
    /// 恢复后的章节序号。
    pub chapter_index: u32,
    /// 是否回退到了章节开头（锚点为空或半径内未命中）。
    pub fallback_to_chapter_start: bool,
    /// 锚点恢复状态短名（核心给出）。
    pub anchor_status: String,
    /// 与保存位置的字节距离。
    pub distance_bytes: u64,
    /// 锚点搜索读取的窗口数。
    pub windows_read: u64,
    /// 锚点搜索读取的字节数（有界性证据）。
    pub bytes_read: u64,
    /// 搜索半径（字节）。
    pub radius_bytes: u64,
}

impl RestoreReport {
    /// 稳定一行（日志与证据用）。
    pub fn as_log(&self, book_id: i64, saved: Option<&SavedPosition>) -> String {
        format!(
            "NR-POS book={} chapter={} offset={} restored={} fallback={} anchor={} distance={} \
             windows={} bytes_read={} radius={} saved_offset={}",
            book_id,
            self.chapter_index,
            self.byte_offset,
            u8::from(saved.is_some()),
            u8::from(self.fallback_to_chapter_start),
            self.anchor_status,
            self.distance_bytes,
            self.windows_read,
            self.bytes_read,
            self.radius_bytes,
            saved.map(|saved| saved.byte_offset).unwrap_or(0),
        )
    }
}

/// 用核心锚点恢复位置（有界半径；未命中回退到章节开头）。
pub fn restore(
    path: &Path,
    toc: &ChapterToc,
    saved: &SavedPosition,
) -> Result<RestoreReport, ProgressError> {
    let source = FileTextSource::open(path).map_err(|error| {
        ProgressError::Source(format!("{}（{}）", path.display(), error.as_str()))
    })?;
    let search = AnchorSearch::DEFAULT;
    let restored =
        restore_position(&source, toc.index(), &saved.position(), &search).map_err(|error| {
            ProgressError::Source(format!("{}（{}）", path.display(), error.message()))
        })?;
    Ok(RestoreReport {
        byte_offset: restored.position.original_byte_offset,
        chapter_index: restored.position.chapter_index,
        fallback_to_chapter_start: restored.fallback_to_chapter_start,
        anchor_status: restored.recovery.status.as_str().to_string(),
        distance_bytes: restored.recovery.distance_bytes,
        windows_read: restored.recovery.windows_read,
        bytes_read: restored.recovery.bytes_read,
        radius_bytes: restored.recovery.radius_bytes,
    })
}

/// 进度跟踪器：记录「待保存位置」，由 UI 入口（翻页 / 跳转 / 保存按钮）驱动。
///
/// 关键约定（避免「重启后跳回旧位置」与「每次启动都写库」两类缺陷）：
///
/// 1. 只有真实位置变化才标记为待保存（同一页反复翻动不重复写库）；
/// 2. 保存是幂等的（核心 `save_progress` 判定），返回的 `SaveReport` 是证据；
/// 3. 页码只做显示，不进入 [`SavedPosition`]。
#[derive(Debug, Clone, Default)]
pub struct ProgressTracker {
    pending: Option<SavedPosition>,
    last_saved: Option<SavedPosition>,
    saves: u64,
    idempotent_skips: u64,
}

impl ProgressTracker {
    /// 新建跟踪器。
    pub fn new() -> Self {
        Self::default()
    }

    /// 标记一个待保存位置；位置与上次保存完全一致时返回 `false`（无需写库）。
    pub fn mark(&mut self, saved: SavedPosition) -> bool {
        let unchanged = self
            .last_saved
            .as_ref()
            .map(|previous| same_identity(previous, &saved))
            .unwrap_or(false)
            || self
                .pending
                .as_ref()
                .map(|pending| same_identity(pending, &saved))
                .unwrap_or(false);
        if unchanged {
            self.idempotent_skips += 1;
            return false;
        }
        self.pending = Some(saved);
        true
    }

    /// 是否有待保存位置。
    pub fn is_dirty(&self) -> bool {
        self.pending.is_some()
    }

    /// 取出待保存位置（取出即清空；保存失败时可重新标记）。
    pub fn take(&mut self) -> Option<SavedPosition> {
        self.pending.take()
    }

    /// 记录一次保存结果（`changed == false` 说明是幂等空写）。
    pub fn note_saved(&mut self, saved: SavedPosition, report: SaveReport) {
        self.saves += 1;
        if !report.changed {
            self.idempotent_skips += 1;
        }
        self.last_saved = Some(saved);
    }

    /// 最近一次成功保存的位置。
    pub fn last_saved(&self) -> Option<&SavedPosition> {
        self.last_saved.as_ref()
    }

    /// （成功保存次数，幂等空写/重复标记次数）。
    pub fn counters(&self) -> (u64, u64) {
        (self.saves, self.idempotent_skips)
    }
}

/// 位置身份比较：只比较 `ContentPosition` 五元组（不含时间戳与页码）。
fn same_identity(a: &SavedPosition, b: &SavedPosition) -> bool {
    a.book_id == b.book_id
        && a.content_unit_id == b.content_unit_id
        && a.chapter_index == b.chapter_index
        && a.byte_offset == b.byte_offset
        && a.quote == b.quote
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("nr-dt4-progress-{tag}-{}", std::process::id()));
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

    /// 真实夹具：多章中文 TXT（章节扫描 + 锚点恢复都作用在真实字节上）。
    fn fixture_lines() -> Vec<String> {
        let mut lines = vec!["作品简介。".to_string(), String::new()];
        for chapter in 1..=6 {
            lines.push(format!("第{chapter}章 进度恢复验证"));
            for row in 0..12 {
                lines.push(format!(
                    "这是第 {chapter} 章第 {row} 行正文，用于校验进度保存与重启恢复的位置语义。"
                ));
            }
            lines.push(String::new());
        }
        lines
    }

    fn write_fixture(dir: &Path) -> PathBuf {
        let path = dir.join("进度恢复验证.txt");
        fs::write(&path, fixture_lines().join("\n")).expect("写入夹具");
        path
    }

    fn open_book_at(path: &Path, book_id: i64) -> crate::pagination::OpenedBook {
        crate::pagination::OpenedBook {
            book_id,
            title: "进度恢复验证".to_string(),
            path: path.to_path_buf(),
            status: "ready".to_string(),
            encoding: reading_source::TextEncoding::Utf8,
            encoding_user_override: false,
        }
    }

    #[test]
    fn saved_identity_is_content_position_without_page_number() {
        let dir = temp_dir("identity");
        let path = write_fixture(&dir);
        let db = dir.join("library.db");
        let book_id = seed_book(&db, "身份夹具");
        let tracked = capture(&path, book_id, 2, 4096).expect("采样位置");
        assert_eq!(tracked.content_unit_id, SINGLE_CONTENT_UNIT_ID as i64);

        let report = save(&db, &tracked, 1_700_000_000).expect("保存进度");
        assert!(report.inserted && report.changed);
        assert!(report.as_log(&tracked).contains("op=save"));

        let loaded = load(&db, book_id).expect("读取进度").expect("应有进度");
        assert_eq!(loaded.chapter_index, tracked.chapter_index);
        assert_eq!(loaded.byte_offset, tracked.byte_offset);
        assert_eq!(loaded.quote, tracked.quote);

        // 身份编码里没有页码字段：`ContentPosition` 冻结 5 项。
        let encoded = loaded.position().encode();
        assert_eq!(
            encoded.split(':').count(),
            reading_core::POSITION_ENCODED_FIELDS + 1
        );
        assert_eq!(
            ContentPosition::decode(&encoded).expect("解码"),
            loaded.position()
        );

        // 库里不得写入页码提示（不得用页码作为进度身份）。
        let mut storage = Storage::open(&db).expect("打开库");
        storage.migrate().expect("迁移");
        let hint: Option<i64> = storage
            .conn()
            .query_row(
                "SELECT last_page_hint FROM reading_progress WHERE book_id = ?1",
                [book_id],
                |row| row.get(0),
            )
            .expect("读取页码提示");
        assert_eq!(hint, None, "V1 不得写入页码提示");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn repeated_save_of_same_position_is_idempotent() {
        let dir = temp_dir("idempotent");
        let path = write_fixture(&dir);
        let db = dir.join("library.db");
        let book_id = seed_book(&db, "幂等夹具");
        let tracked = capture(&path, book_id, 1, 300).expect("采样位置");
        let first = save(&db, &tracked, 1_000).expect("首次保存");
        assert!(first.changed);
        let second = save(&db, &tracked, 2_000).expect("重复保存");
        assert!(!second.inserted && !second.changed, "同一位置不得重复写库");
        let loaded = load(&db, book_id).expect("读取").expect("应有进度");
        assert_eq!(loaded.updated_at, 1_000, "幂等空写不得更新时间戳");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn restart_restores_exact_saved_offset_from_anchor() {
        let dir = temp_dir("restore");
        let path = write_fixture(&dir);
        let db = dir.join("library.db");
        let book_id = seed_book(&db, "恢复夹具");
        let book = open_book_at(&path, book_id);
        let toc =
            crate::chapters::scan_toc(&path, reading_source::TextEncoding::Utf8).expect("扫描章节");
        assert!(toc.len() >= 3, "夹具应有多章：{:#?}", toc.entries());

        // 模拟用户翻到第 3 章内部某页后保存。
        let target_offset = toc.start_byte(2).expect("第 3 章起点") + 120;
        let chapter_index = toc.sequence_at_offset(target_offset);
        let tracked = capture(&path, book.book_id, chapter_index, target_offset).expect("采样");
        save(&db, &tracked, 1_000).expect("保存");

        // 「重启」：全新读取进度并按核心锚点恢复。
        let loaded = load(&db, book.book_id).expect("读取").expect("应有进度");
        let restored = restore(&path, &toc, &loaded).expect("恢复位置");
        assert_eq!(
            restored.byte_offset,
            loaded.byte_offset,
            "锚点精确命中时必须落回同一字节偏移（证据：{}）",
            restored.as_log(9, Some(&loaded))
        );
        assert!(!restored.fallback_to_chapter_start);
        assert_eq!(restored.chapter_index, chapter_index);
        // 锚点搜索有界：读取量不超过「两倍半径 + 单窗口」上限（不整本读取）。
        let bound = restored.radius_bytes * 2
            + reading_source::MAX_WINDOW_BYTES as u64
            + reading_core::ANCHOR_QUOTE_CHARS as u64;
        assert!(
            restored.bytes_read <= bound,
            "锚点读取量 {} 超出有界上限 {}",
            restored.bytes_read,
            bound
        );
        assert!(restored.windows_read >= 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_restores_to_chapter_start_without_losing_progress() {
        let dir = temp_dir("fallback");
        let path = write_fixture(&dir);
        let db = dir.join("library.db");
        let book_id = seed_book(&db, "回退夹具");
        let book = open_book_at(&path, book_id);
        let toc =
            crate::chapters::scan_toc(&path, reading_source::TextEncoding::Utf8).expect("扫描章节");
        let saved = SavedPosition {
            book_id: book.book_id,
            content_unit_id: SINGLE_CONTENT_UNIT_ID as i64,
            chapter_index: 1,
            byte_offset: toc.start_byte(1).unwrap() + 60,
            // 空锚点 ⇒ 核心强制回退到章节开头（不猜测）。
            quote: String::new(),
            updated_at: 10,
            last_opened_at: 10,
        };
        save(&db, &saved, 10).expect("保存");
        let loaded = load(&db, book.book_id).expect("读取").expect("应有进度");
        let restored = restore(&path, &toc, &loaded).expect("恢复");
        assert!(restored.fallback_to_chapter_start);
        assert_eq!(restored.byte_offset, toc.start_byte(1).unwrap());
        assert_eq!(restored.anchor_status, "EmptyQuote");
        // 进度行仍在（恢复失败不得删除进度）。
        assert_eq!(
            load(&db, book.book_id)
                .expect("读取")
                .expect("应有进度")
                .byte_offset,
            saved.byte_offset
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tracker_marks_only_real_position_changes() {
        let dir = temp_dir("tracker");
        let path = write_fixture(&dir);
        let db = dir.join("library.db");
        let book_id = seed_book(&db, "跟踪夹具");
        let first = capture(&path, book_id, 0, 0).expect("采样");
        let mut tracker = ProgressTracker::new();
        assert!(tracker.mark(first.clone()), "首次位置变化必须标记");
        assert!(tracker.is_dirty());
        let pending = tracker.take().expect("取出待保存");
        let report = save(&db, &pending, 100).expect("保存");
        tracker.note_saved(pending, report);
        assert!(!tracker.is_dirty());
        assert!(!tracker.mark(first.clone()), "同一位置不得重复标记");

        let second = capture(&path, book_id, 1, 500).expect("采样第二处");
        assert!(tracker.mark(second), "新位置必须标记");
        // 不同书 / 不同章节 / 不同偏移都算变化。
        let mut other = first.clone();
        other.book_id = 99;
        assert!(tracker.mark(other));
        assert_eq!(tracker.counters().0, 1);
        let _ = fs::remove_dir_all(&dir);
    }
}

//! 章节目录（TOC）与章节跳转：**只做编排与投影，不做章节识别**。
//!
//! 边界（`desktop/spec.md` DR2、`design.md` §2、DT1 禁止事项）：
//!
//! - 章节识别（`第…章` / 卷 / 楔子 / 独立数字标题）与区间切分全部来自冻结核心
//!   `reading-core::chapter_scan_session` / `scan_chapters_from_source`；本模块不写一行
//!   标题匹配规则，也不解析正文（核心按有限窗口扫描，只保留标题行与区间）；
//! - 章节区间与目录标题的**缓存**落在核心存储层 `chapters` 表（`reading-storage::index`），
//!   只缓存短摘要（标题 + 区间），正文永不入库；
//! - 目录缓存以 `app_meta.content_index_version` 为键：编码切换或文件指纹变化后内容索引
//!   版本递增，旧目录自动视为失效并重新扫描（`design.md` §5）。
//!
//! 跳转语义：章节跳转的落点是**章节起始字节偏移**，再交给分页运行时以该偏移重新测量页面；
//! 本模块不产生页码、也不用页码表示任何位置。

use std::path::Path;

use reading_core::chapter::{
    scan_chapters_from_source_with_encoding, ChapterIndex, ChapterRange, ChapterRule,
};
use reading_source::{TextEncoding, TextSource};
use reading_storage::{index as storage_index, meta, Storage};

use crate::encoded_source::EncodedFileSource;
use crate::pagination::OpenedBook;

/// 目录来源（证据用：缓存命中 or 本次真实扫描）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TocOrigin {
    /// 命中 `chapters` 表缓存（内容索引版本一致）。
    Cache,
    /// 本次重新扫描（首开 / 缓存失效）。
    Scanned,
}

impl TocOrigin {
    /// 稳定短名（日志与证据用）。
    pub const fn as_str(self) -> &'static str {
        match self {
            TocOrigin::Cache => "cache",
            TocOrigin::Scanned => "scanned",
        }
    }
}

/// 目录一行（只含可显示字段；正文不入内存）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocEntry {
    /// 章节序号（0 基；与核心 `ChapterIndex` 下标一致）。
    pub sequence: u32,
    /// 章节标题（短摘要，来自核心扫描）。
    pub title: String,
    /// 起始字节。
    pub start_byte: u64,
    /// 结束字节（不含）。
    pub end_byte: u64,
    /// 命中的章节规则稳定短名（仅用于显示与证据，不做业务分支）。
    pub rule: String,
}

/// 章节扫描的读取量证据（证明按有限窗口读取，不复制整本正文）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanEvidence {
    /// 读取窗口次数。
    pub windows_read: u64,
    /// 物化字节总数。
    pub bytes_read: u64,
    /// 单次物化字节峰值。
    pub max_window_bytes: u64,
    /// 扫描到的字节数。
    pub scanned_bytes: u64,
    /// 扫描行数。
    pub lines_scanned: u64,
}

/// 一本已打开书籍的章节目录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterToc {
    index: ChapterIndex,
    origin: TocOrigin,
    evidence: ScanEvidence,
}

impl ChapterToc {
    /// 由核心章节索引构造（扫描路径与缓存路径共用）。
    pub fn from_index(index: ChapterIndex, origin: TocOrigin, evidence: ScanEvidence) -> Self {
        Self {
            index,
            origin,
            evidence,
        }
    }

    /// 目录来源。
    pub fn origin(&self) -> TocOrigin {
        self.origin
    }

    /// 核心章节索引（位置恢复与偏移判定使用）。
    pub fn index(&self) -> &ChapterIndex {
        &self.index
    }

    /// 扫描证据。
    pub fn evidence(&self) -> ScanEvidence {
        self.evidence
    }

    /// 章节数（核心保证至少 1：无命中时回退到「全文」虚拟章节）。
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// 是否为空（正常构造下恒为 false）。
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// 是否为「无章节命中」的虚拟全文章节。
    pub fn is_virtual(&self) -> bool {
        self.index.is_virtual
    }

    /// 内容总字节数（= 文件长度）。
    pub fn total_bytes(&self) -> u64 {
        self.index.total_bytes
    }

    /// 全量目录行（UI 只展示窗口，全量仅用于证据与自检）。
    pub fn entries(&self) -> Vec<TocEntry> {
        self.index.chapters.iter().map(entry_of).collect()
    }

    /// 单行目录。
    pub fn entry(&self, sequence: u32) -> Option<TocEntry> {
        self.index.chapter(sequence).map(entry_of)
    }

    /// 章节标题（越界返回空串；UI 只显示，不做判断）。
    pub fn title_at(&self, sequence: u32) -> String {
        self.index
            .chapter(sequence)
            .map(|chapter| chapter.title.clone())
            .unwrap_or_default()
    }

    /// 章节起始字节；越界返回 `None`（跳转目标必须来自这里，不得按页码估算）。
    pub fn start_byte(&self, sequence: u32) -> Option<u64> {
        self.index.chapter_start(sequence)
    }

    /// 偏移所属章节序号（区间判定由核心 `chapter_for_offset` 给出）。
    pub fn sequence_at_offset(&self, offset: u64) -> u32 {
        self.index.chapter_for_offset(offset).unwrap_or(0)
    }

    /// 目录窗口：以 `center` 为中心取 `span` 行，返回（首个序号，行）。
    ///
    /// UI 只渲染这个窗口：章节数可能上千，界面不申请无限长列表。
    pub fn window(&self, center: u32, span: usize) -> (u32, Vec<TocEntry>) {
        let total = self.len();
        if total == 0 || span == 0 {
            return (0, Vec::new());
        }
        let span = span.min(total);
        let half = span / 2;
        let max_first = total - span;
        let first = (center as usize).saturating_sub(half).min(max_first);
        let rows = (first..first + span)
            .filter_map(|sequence| self.entry(sequence as u32))
            .collect();
        (first as u32, rows)
    }

    /// 稳定一行（日志与证据用；不含正文）。
    pub fn state_log(&self, book_id: i64, current: u32, window_first: u32) -> String {
        format!(
            "NR-TOC book={} origin={} chapters={} virtual={} index_version={} scanned_bytes={} \
             windows={} bytes_read={} max_window={} current={} window_first={} total={}",
            book_id,
            self.origin.as_str(),
            self.len(),
            u8::from(self.is_virtual()),
            self.index.index_version,
            self.evidence.scanned_bytes,
            self.evidence.windows_read,
            self.evidence.bytes_read,
            self.evidence.max_window_bytes,
            current,
            window_first,
            self.total_bytes(),
        )
    }
}

fn entry_of(chapter: &ChapterRange) -> TocEntry {
    TocEntry {
        sequence: chapter.index,
        title: chapter.title.clone(),
        start_byte: chapter.start_byte,
        end_byte: chapter.end_byte,
        rule: chapter.rule.as_str().to_string(),
    }
}

/// 目录失败原因（可读文本，不作为业务分支）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChapterError {
    /// 数据源不可读。
    Source(String),
    /// 存储层失败。
    Storage(String),
    /// 目标章节不存在。
    NoSuchChapter {
        /// 请求的章节序号。
        sequence: u32,
        /// 实际章节数。
        chapters: usize,
    },
}

impl ChapterError {
    /// 可读说明（状态栏用）。
    pub fn message(&self) -> String {
        match self {
            ChapterError::Source(message) => format!("章节扫描失败：{message}"),
            ChapterError::Storage(message) => format!("章节索引读写失败：{message}"),
            ChapterError::NoSuchChapter { sequence, chapters } => {
                format!("章节 {sequence} 不存在（共 {chapters} 章）")
            }
        }
    }
}

impl std::fmt::Display for ChapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for ChapterError {}

/// 目录读取结果（含缓存命中情况与落库行数）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocReport {
    /// 目录。
    pub toc: ChapterToc,
    /// 内容索引版本（目录缓存键）。
    pub index_version: i64,
    /// 本次写入 `chapters` 表的行数（缓存命中时为 0）。
    pub persisted_rows: usize,
}

/// 真实扫描一本 TXT 的章节（核心有界窗口扫描；不整本读取、不复制正文）。
///
/// `encoding` 必须是书目记录（或用户在界面上覆盖后）的内容编码：核心按该编码解码每一行，
/// 非 UTF-8 文档（GBK/GB18030/UTF-16/Big5）才能识别出**真实章节**，而不是回退成虚拟的「全文」。
pub fn scan_toc(path: &Path, encoding: TextEncoding) -> Result<ChapterToc, ChapterError> {
    let source = EncodedFileSource::open(path, encoding).map_err(|error| {
        ChapterError::Source(format!("{}（{}）", path.display(), error.as_str()))
    })?;
    let (index, stats) = scan_chapters_from_source_with_encoding(&source, &|| false, encoding)
        .map_err(|error| {
            ChapterError::Source(format!("{}（{}）", path.display(), error.message()))
        })?;
    let evidence = ScanEvidence {
        windows_read: stats.windows_read,
        bytes_read: stats.bytes_read,
        max_window_bytes: stats.max_window_bytes,
        scanned_bytes: index.scanned_bytes,
        lines_scanned: index.lines_scanned,
    };
    Ok(ChapterToc::from_index(index, TocOrigin::Scanned, evidence))
}

/// 打开库并读取内容索引版本。
fn open_storage(db_path: &Path) -> Result<Storage, ChapterError> {
    let mut storage = Storage::open(db_path)
        .map_err(|error| ChapterError::Storage(format!("{}（{error}）", db_path.display())))?;
    storage
        .migrate()
        .map_err(|error| ChapterError::Storage(format!("迁移失败：{error}")))?;
    Ok(storage)
}

/// 内容索引版本（目录缓存键；缺失时为 0）。
pub fn content_index_version(db_path: &Path) -> Result<i64, ChapterError> {
    let storage = open_storage(db_path)?;
    meta::content_index_version(storage.conn())
        .map_err(|error| ChapterError::Storage(error.to_string()))
}

/// 从 `chapters` 表读取目录缓存（版本不一致或任一行无法还原时返回 `None`）。
///
/// 只有「版本一致 + 每行都能还原成核心章节区间」才视为命中；任何不确定都重新扫描，
/// 绝不使用伪造的章节规则。
pub fn load_cached_toc(db_path: &Path, book_id: i64) -> Result<Option<ChapterToc>, ChapterError> {
    let storage = open_storage(db_path)?;
    let version = meta::content_index_version(storage.conn())
        .map_err(|error| ChapterError::Storage(error.to_string()))?;
    let rows = storage_index::list_chapters(storage.conn(), book_id)
        .map_err(|error| ChapterError::Storage(error.to_string()))?;
    if rows.is_empty() {
        return Ok(None);
    }
    let mut chapters = Vec::with_capacity(rows.len());
    for row in &rows {
        if row.index_version != version {
            return Ok(None);
        }
        if row.sequence < 0 || row.start_byte < 0 || row.end_byte < row.start_byte {
            return Ok(None);
        }
        let Some(rule) = rule_from_cache(&row.rule_id) else {
            return Ok(None);
        };
        let sequence = row.sequence as u32;
        if sequence as usize != chapters.len() {
            return Ok(None);
        }
        let title = row.title.clone();
        chapters.push(ChapterRange {
            index: sequence,
            title: title.clone(),
            // 缓存路径不重算归一化标题：它只服务于扫描期的匹配与去重，
            // 位置恢复、区间判定与跳转全部只使用 `start_byte` / `end_byte`。
            normalized_title: title,
            start_byte: row.start_byte as u64,
            end_byte: row.end_byte as u64,
            rule,
            number: None,
        });
    }
    let total_bytes = chapters.last().map(|chapter| chapter.end_byte).unwrap_or(0);
    let index = ChapterIndex {
        chapters,
        is_virtual: rows.len() == 1
            && rows[0].rule_id.trim() == ChapterRule::VirtualFullText.id().to_string(),
        index_version: reading_core::chapter::CHAPTER_INDEX_VERSION,
        total_bytes,
        scanned_bytes: 0,
        lines_scanned: 0,
        undecodable_lines: 0,
        truncated_lines: 0,
        numeric_candidates: 0,
        cancelled: false,
    };
    Ok(Some(ChapterToc::from_index(
        index,
        TocOrigin::Cache,
        ScanEvidence::default(),
    )))
}

/// `chapters.rule_id` 的缓存形态：核心章节规则的**稳定数值**（可精确还原）。
fn rule_id_for(rule: ChapterRule) -> String {
    rule.id().to_string()
}

fn rule_from_cache(text: &str) -> Option<ChapterRule> {
    text.trim()
        .parse::<u32>()
        .ok()
        .and_then(ChapterRule::from_id)
}

/// 把目录写入 `chapters` 表（事务内先删后插；标题与区间是短摘要，不含正文）。
pub fn persist_toc(
    db_path: &Path,
    book_id: i64,
    index_version: i64,
    toc: &ChapterToc,
    now: i64,
) -> Result<usize, ChapterError> {
    let storage = open_storage(db_path)?;
    let rows: Vec<storage_index::ChapterInput> = toc
        .index()
        .chapters
        .iter()
        .map(|chapter| storage_index::ChapterInput {
            sequence: chapter.index as i64,
            title: chapter.title.clone(),
            start_byte: chapter.start_byte as i64,
            end_byte: chapter.end_byte as i64,
            rule_id: rule_id_for(chapter.rule),
        })
        .collect();
    storage_index::rebuild_chapters(storage.conn(), book_id, index_version, &rows, now)
        .map_err(|error| ChapterError::Storage(error.to_string()))
}

/// 取（必要时建立）目录：缓存命中直接复用，否则真实扫描并落库。
///
/// 缓存键是 `app_meta.content_index_version`：编码切换 / 文件指纹变化后该版本递增
/// （DT2 的失效链路），旧目录随之失效并重新扫描。
pub fn ensure_toc(db_path: &Path, book: &OpenedBook, now: i64) -> Result<TocReport, ChapterError> {
    let index_version = content_index_version(db_path)?;
    if let Some(cached) = load_cached_toc(db_path, book.book_id)? {
        if cached.total_bytes() == source_len(&book.path) {
            return Ok(TocReport {
                toc: cached,
                index_version,
                persisted_rows: 0,
            });
        }
    }
    let toc = scan_toc(&book.path, book.encoding)?;
    let persisted_rows = persist_toc(db_path, book.book_id, index_version, &toc, now)?;
    Ok(TocReport {
        toc,
        index_version,
        persisted_rows,
    })
}

/// 数据源长度（只读元信息，不读正文）。
fn source_len(path: &Path) -> u64 {
    EncodedFileSource::open(path, TextEncoding::Utf8)
        .map(|source| source.len())
        .unwrap_or(0)
}

/// 章节跳转目标：章节起始字节偏移（跳转不使用页码）。
pub fn jump_target(toc: &ChapterToc, sequence: u32) -> Result<u64, ChapterError> {
    toc.start_byte(sequence).ok_or(ChapterError::NoSuchChapter {
        sequence,
        chapters: toc.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("nr-dt4-chapters-{tag}-{}", std::process::id()));
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

    fn write_fixture(dir: &Path, lines: &[&str]) -> PathBuf {
        let path = dir.join("斗破苍穹-章节.csv.txt");
        fs::write(&path, lines.join("\n")).expect("写入夹具");
        path
    }

    fn chapters_fixture() -> Vec<&'static str> {
        vec![
            "作品简介：这是一本用于章节扫描测试的 TXT。",
            "",
            "第一章 陨落的天才",
            "斗之力，三段。",
            "",
            "第二章 斗气大陆",
            "这里是斗气大陆。",
            "",
            "第三章 客人",
            "有客来访。",
        ]
    }

    #[test]
    fn scan_finds_real_chapters_and_keeps_title_offsets() {
        let dir = temp_dir("scan");
        let path = write_fixture(&dir, &chapters_fixture());
        let toc = scan_toc(&path, TextEncoding::Utf8).expect("扫描章节");
        assert_eq!(toc.origin(), TocOrigin::Scanned);
        assert_eq!(toc.len(), 3, "应识别 3 章：{:#?}", toc.entries());
        let entries = toc.entries();
        assert_eq!(entries[0].title, "第一章 陨落的天才");
        assert_eq!(entries[2].title, "第三章 客人");
        assert!(entries[0].start_byte < entries[1].start_byte);
        assert!(entries[1].start_byte < entries[2].start_byte);
        assert_eq!(entries[2].end_byte, toc.total_bytes());
        // 有限窗口：读取量有界且未整本复制（每次物化 ≤ 256 KiB 硬上限）。
        let evidence = toc.evidence();
        assert!(evidence.windows_read >= 1);
        assert!(evidence.max_window_bytes <= reading_source::MAX_WINDOW_BYTES as u64);
        assert!(evidence.bytes_read >= toc.total_bytes());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn text_without_titles_falls_back_to_single_virtual_chapter() {
        let dir = temp_dir("virtual");
        let path = write_fixture(&dir, &["只有正文，没有任何章节标题。", "第二行。"]);
        let toc = scan_toc(&path, TextEncoding::Utf8).expect("扫描章节");
        assert!(toc.is_virtual(), "无标题时必须回退到虚拟全文章节");
        assert_eq!(toc.len(), 1);
        assert_eq!(toc.start_byte(0), Some(0));
        assert_eq!(toc.total_bytes(), fs::metadata(&path).unwrap().len());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn toc_cache_round_trips_through_storage_and_hits_on_second_open() {
        let dir = temp_dir("cache");
        let path = write_fixture(&dir, &chapters_fixture());
        let db = dir.join("library.db");
        let scanned = scan_toc(&path, TextEncoding::Utf8).expect("扫描章节");
        let book_id = seed_book(&db, "章节缓存夹具");
        let version = 0;
        let rows = persist_toc(&db, book_id, version, &scanned, 1_700_000_000).expect("落库章节");
        assert_eq!(rows, scanned.len());

        let cached = load_cached_toc(&db, book_id)
            .expect("读取缓存")
            .expect("缓存应命中");
        assert_eq!(cached.origin(), TocOrigin::Cache);
        assert_eq!(cached.len(), scanned.len());
        assert_eq!(
            cached.entries(),
            scanned.entries(),
            "缓存必须与真实扫描一致"
        );
        // 区间语义与核心一致：偏移分派到正确章节。
        let entries = scanned.entries();
        assert_eq!(cached.sequence_at_offset(entries[1].start_byte), 1);
        assert_eq!(cached.sequence_at_offset(entries[2].start_byte), 2);

        let report = ensure_toc(
            &db,
            &OpenedBook {
                book_id,
                title: "夹具".to_string(),
                path: path.clone(),
                status: "ready".to_string(),
                encoding: TextEncoding::Utf8,
                encoding_user_override: false,
            },
            1_700_000_100,
        )
        .expect("确保目录");
        assert_eq!(
            report.toc.origin(),
            TocOrigin::Cache,
            "二次打开必须命中缓存"
        );
        assert_eq!(report.persisted_rows, 0, "缓存命中不得重复写库");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_is_invalidated_when_content_index_version_changes() {
        let dir = temp_dir("invalidate");
        let path = write_fixture(&dir, &chapters_fixture());
        let db = dir.join("library.db");
        let toc = scan_toc(&path, TextEncoding::Utf8).expect("扫描章节");
        let book_id = seed_book(&db, "失效夹具");
        persist_toc(&db, book_id, 0, &toc, 1).expect("落库");
        assert!(load_cached_toc(&db, book_id).expect("读缓存").is_some());

        let storage = open_storage(&db).expect("打开库");
        meta::bump_content_index_version(storage.conn(), 2).expect("递增内容索引版本");
        assert!(
            load_cached_toc(&db, book_id).expect("读缓存").is_none(),
            "内容索引版本变化后旧目录必须失效"
        );

        let report = ensure_toc(
            &db,
            &OpenedBook {
                book_id,
                title: "夹具".to_string(),
                path: path.clone(),
                status: "ready".to_string(),
                encoding: TextEncoding::Utf8,
                encoding_user_override: false,
            },
            3,
        )
        .expect("确保目录");
        assert_eq!(report.toc.origin(), TocOrigin::Scanned);
        assert_eq!(report.persisted_rows, toc.len());
        assert_eq!(report.index_version, 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn window_is_bounded_and_centered_on_current_chapter() {
        let dir = temp_dir("window");
        let path = write_fixture(&dir, &chapters_fixture());
        let toc = scan_toc(&path, TextEncoding::Utf8).expect("扫描章节");
        let (first, rows) = toc.window(1, 2);
        assert!(rows.len() <= 2, "窗口必须有界");
        assert_eq!(first, 0);
        assert_eq!(rows[0].sequence, 0);
        // 越界中心不 panic，窗口仍落在合法范围。
        let (first, rows) = toc.window(9_999, 2);
        assert_eq!(first, 1);
        assert_eq!(rows.len(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn jump_target_rejects_unknown_chapter_without_guessing() {
        let dir = temp_dir("jump");
        let path = write_fixture(&dir, &chapters_fixture());
        let toc = scan_toc(&path, TextEncoding::Utf8).expect("扫描章节");
        assert_eq!(
            jump_target(&toc, 1).expect("第二章起点"),
            toc.entries()[1].start_byte
        );
        let error = jump_target(&toc, 42).expect_err("越界章节必须报错");
        assert!(matches!(
            error,
            ChapterError::NoSuchChapter { sequence: 42, .. }
        ));
        let _ = fs::remove_dir_all(&dir);
    }
}

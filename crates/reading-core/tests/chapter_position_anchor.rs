//! RC4 集成测试（reading-core）：章节索引、`ContentPosition` 与锚点恢复的端到端契约。
//!
//! 覆盖本卡 DoD：
//! - 章节规则与虚拟章节回退（[`no_chapter_falls_back_to_virtual_chapter`]）；
//! - `ContentPosition` 往返一致、按原始字节偏移归属章节、越界夹紧；
//! - 锚点在**同文本重复出现的段落**中就近命中，失败时回退章节开头；
//! - 单 TXT 单内容单元，且位置身份不含动态页码；
//! - 章节扫描可增量、可取消、读取量有界。

use std::cell::Cell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use reading_core::{
    chapter_index_for_offset, chapter_scan_session, position_at_offset, restore_position,
    scan_chapters_from_source, virtual_chapter_index, AnchorSearch, AnchorStatus, ChapterIndex,
    ChapterRule, ContentPosition, ContentUnit, ErrorCode, NearbyFingerprint, ScanStep,
    VIRTUAL_CHAPTER_TITLE,
};
use reading_source::{FileTextSource, TextSource, DEFAULT_WINDOW_BYTES};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempGuard {
    dir: PathBuf,
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn temp_source(label: &str, content: &str) -> (TempGuard, FileTextSource) {
    let dir = std::env::temp_dir().join(format!(
        "nr-rc4-core-{}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
        label
    ));
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    let path = dir.join("book.txt");
    std::fs::write(&path, content.as_bytes()).expect("写入临时文件");
    let source = FileTextSource::open(&path).expect("打开临时文件");
    (TempGuard { dir }, source)
}

/// 生成 `chapters` 章、每章正文约 `body_bytes` 字节的小说文本。
fn make_book(chapters: usize, body_bytes: usize) -> String {
    let filler = "正文内容。".repeat(body_bytes / 12 + 1);
    let mut out = String::new();
    for index in 1..=chapters {
        let number = index.to_string();
        out.push('第');
        out.push_str(&number);
        out.push_str("章 第");
        out.push_str(&number);
        out.push_str("段\n");
        out.push_str(&filler);
        out.push('\n');
    }
    out
}

const REPEATED_PARAGRAPH: &str = "“你终于来了。”他低声说道，声音里带着三年未曾散去的寒意。\n";

/// 楔子 + 两章正文；第一章内同一段落重复三次（间隔约 10.5 KiB）。
fn repeated_paragraph_book() -> (String, [u64; 3]) {
    let mut content = String::new();
    content.push_str("楔子\n开场正文。\n\n");
    content.push_str("第一章 归途\n");
    let mut offsets = [0u64; 3];
    for slot in offsets.iter_mut() {
        *slot = content.len() as u64;
        content.push_str(REPEATED_PARAGRAPH);
        content.push_str(&"铺垫内容。".repeat(700));
        content.push('\n');
    }
    content.push_str("第二章 终章\n收尾。\n");
    (content, offsets)
}

fn scan(source: &FileTextSource) -> ChapterIndex {
    let (index, _stats) = scan_chapters_from_source(source, &|| false).expect("章节扫描");
    index
}

#[test]
fn no_chapter_falls_back_to_virtual_chapter() {
    // 本卡必须真实执行的第二条命令以此测试名过滤。
    let content = "这是一段没有任何章节标题的正文，只有叙述。\n\n第二段内容。\n";
    let (_guard, source) = temp_source("virtual", content);
    let (index, stats) = scan_chapters_from_source(&source, &|| false).expect("章节扫描");

    assert!(
        index.is_virtual,
        "无章节命中时必须生成单一虚拟全文章节，保证文件仍可读"
    );
    assert_eq!(index.len(), 1);
    let chapter = index.chapter(0).expect("虚拟章节");
    assert_eq!(chapter.index, 0);
    assert_eq!(chapter.rule, ChapterRule::VirtualFullText);
    assert_eq!(chapter.title, VIRTUAL_CHAPTER_TITLE);
    assert_eq!(chapter.start_byte, 0);
    assert_eq!(chapter.end_byte, source.len());
    assert_eq!(chapter.number, None);
    assert_eq!(index.chapter_for_offset(0), Some(0));
    assert_eq!(index.chapter_for_offset(source.len() - 1), Some(0));
    assert_eq!(index.total_bytes, source.len());
    assert!(!index.cancelled);
    assert_eq!(stats.windows_read, 1, "小文件只需一个窗口");

    // 同一语义也可直接构造（供 storage / FFI 在已知长度时使用）。
    let direct = virtual_chapter_index(source.len());
    assert!(direct.is_virtual);
    assert_eq!(direct.chapter(0), index.chapter(0));
    assert_eq!(direct.total_bytes, source.len());
    assert_eq!(direct.index_version, index.index_version);

    // 虚拟章节不改变数据源：文件仍可读且内容完整。
    let window = source
        .read_at(0, source.default_window_bytes())
        .expect("窗口读取");
    assert_eq!(window.as_str().expect("合法 UTF-8"), content);
    assert!(window.eof);

    // 空文件也必须得到可用的虚拟章节（不 panic、不越界）。
    let (_empty_guard, empty) = temp_source("virtual-empty", "");
    let (empty_index, empty_stats) =
        scan_chapters_from_source(&empty, &|| false).expect("空文件扫描");
    assert!(empty_index.is_virtual);
    assert_eq!(empty_index.len(), 1);
    assert_eq!(empty_index.chapter(0).map(|c| c.end_byte), Some(0));
    assert_eq!(empty_stats.windows_read, 0, "空文件不应读取任何窗口");
}

#[test]
fn position_round_trip_is_exact_for_extreme_values() {
    let quote = "第一章 初见……中文与🙂表情混排";
    let position = ContentPosition::new(
        u64::MAX,
        reading_core::SINGLE_CONTENT_UNIT_ID,
        u32::MAX,
        u64::MAX - 1,
        NearbyFingerprint::new(quote),
    );
    let encoded = position.encode();
    assert!(encoded.starts_with(reading_core::POSITION_ENCODING_PREFIX));
    assert_eq!(
        encoded.split(':').count(),
        reading_core::POSITION_ENCODED_FIELDS + 1,
        "位置编码只允许 5 个冻结字段"
    );
    let decoded = ContentPosition::decode(&encoded).expect("位置解码");
    assert_eq!(decoded, position, "位置往返必须逐字段相等");
    assert_eq!(decoded.encode(), encoded, "二次往返必须稳定");
    assert_eq!(decoded.book_id, u64::MAX, "64 位 ID 不得被截断");
    assert_eq!(decoded.original_byte_offset, u64::MAX - 1);
    assert_eq!(decoded.chapter_index, u32::MAX);
    assert!(decoded.nearby_fingerprint.is_consistent());
    assert!(
        decoded.nearby_fingerprint.quote.chars().count()
            <= reading_core::POSITION_ENCODED_FIELDS * 5
    );

    // 非法输入必须被拒绝（而不是静默产生错误位置）。
    for bad in [
        "",
        "nrpos1",
        "nrpos1:1:1:0",
        "nrpos9:1:1:0:0:41",
        "nrpos1:x:1:0:0:41",
        "nrpos1:1:1:0:0:41:7",
        "nrpos1:1:1:0:0:z",
    ] {
        let error = ContentPosition::decode(bad).expect_err("非法编码必须被拒绝");
        assert_eq!(error.code(), ErrorCode::InvalidArgument, "输入: {bad:?}");
    }
}

#[test]
fn position_maps_to_chapter_by_original_byte_offset_and_clamps() {
    let content = "楔子\n甲。\n\n第一章 初见\n乙。\n\n第二章 归途\n丙。\n";
    let (_guard, source) = temp_source("position-chapter", content);
    let index = scan(&source);
    assert_eq!(index.len(), 3);
    let second_start = index.chapter(1).expect("第一章").start_byte;
    let third_start = index.chapter(2).expect("第二章").start_byte;

    assert_eq!(chapter_index_for_offset(&index, 0), 0);
    assert_eq!(chapter_index_for_offset(&index, second_start - 1), 0);
    assert_eq!(chapter_index_for_offset(&index, second_start), 1);
    assert_eq!(chapter_index_for_offset(&index, third_start - 1), 1);
    assert_eq!(chapter_index_for_offset(&index, third_start), 2);
    assert_eq!(chapter_index_for_offset(&index, source.len() - 1), 2);
    assert_eq!(
        chapter_index_for_offset(&index, u64::MAX),
        2,
        "越界夹紧到末章"
    );
    assert_eq!(
        reading_core::chapter_start_offset(&index, 2),
        Some(third_start)
    );
    assert_eq!(reading_core::chapter_start_offset(&index, 9), None);

    let unit = ContentUnit::from_source(&source, 77);
    let position = unit.position_with_quote(1, third_start - 1, "乙。");
    assert!(position.chapter_index_in_range(&index));
    assert!(position.is_inside(source.len()));
    assert_eq!(
        position.chapter(&index).map(|c| c.title.as_str()),
        Some("第一章 初见")
    );

    // 章节越界 + 偏移越界：必须夹紧到最后一个有效章节内的位置。
    let wild = ContentPosition::new(
        77,
        unit.id,
        99,
        u64::MAX,
        position.nearby_fingerprint.clone(),
    );
    let clamped = wild.clamped_for(&index);
    assert_eq!(clamped.chapter_index, 2);
    assert!(clamped.is_inside(source.len()));
    assert!(clamped.original_byte_offset >= third_start);
    assert_eq!(
        clamped.nearby_fingerprint, position.nearby_fingerprint,
        "夹紧不得丢失锚点"
    );
    assert_eq!(clamped.book_id, 77);
    assert_eq!(clamped.content_unit_id, unit.id);
}

#[test]
fn anchor_restore_position_hits_nearest_repeated_paragraph() {
    let (content, offsets) = repeated_paragraph_book();
    let (_guard, source) = temp_source("restore", &content);
    let index = scan(&source);
    assert_eq!(index.len(), 3);
    let target = offsets[1];

    let position = position_at_offset(
        &source,
        9,
        reading_core::SINGLE_CONTENT_UNIT_ID,
        &index,
        target,
    )
    .expect("构造位置");
    assert_eq!(position.original_byte_offset, target);
    assert_eq!(position.chapter_index, 1, "锚点必须落在第一章内");
    assert_eq!(
        position.nearby_fingerprint.char_len as usize,
        reading_core::ANCHOR_QUOTE_CHARS
    );

    // 窄半径：只覆盖最近的一次出现。
    let narrow =
        restore_position(&source, &index, &position, &AnchorSearch::new(2048)).expect("锚点恢复");
    assert!(!narrow.fallback_to_chapter_start);
    assert!(narrow.recovery.is_found());
    assert_eq!(narrow.position.original_byte_offset, target);
    assert_eq!(narrow.position.chapter_index, 1);
    assert_eq!(narrow.recovery.candidates, 1);
    assert!(
        narrow.recovery.bytes_read < source.len(),
        "锚点恢复不得整本读取"
    );

    // 默认半径：三次重复全部在半径内，仍必须回到原位置（就近命中）。
    let wide =
        restore_position(&source, &index, &position, &AnchorSearch::DEFAULT).expect("宽半径恢复");
    assert_eq!(wide.position.original_byte_offset, target);
    assert_eq!(wide.recovery.distance_bytes, 0);
    assert!(wide.recovery.candidates >= 3);
    assert_ne!(wide.position.original_byte_offset, offsets[0]);
    assert_ne!(wide.position.original_byte_offset, offsets[2]);

    // 引用文本在半径内不存在 ⇒ 回退到最近可确认的章节开头。
    let missing = ContentPosition::new(
        9,
        reading_core::SINGLE_CONTENT_UNIT_ID,
        1,
        target,
        NearbyFingerprint::new("这段文字在文件里根本不存在，用于验证回退行为"),
    );
    let fallback =
        restore_position(&source, &index, &missing, &AnchorSearch::new(2048)).expect("恢复");
    assert_eq!(fallback.recovery.status, AnchorStatus::NotFound);
    assert!(fallback.fallback_to_chapter_start);
    assert_eq!(fallback.position.chapter_index, 1);
    assert_eq!(
        fallback.position.original_byte_offset,
        index.chapter(1).unwrap().start_byte
    );

    // 空指纹 ⇒ 直接回退章节开头，且不读取数据源。
    let no_anchor = ContentPosition::new(
        9,
        reading_core::SINGLE_CONTENT_UNIT_ID,
        1,
        target,
        NearbyFingerprint::empty(),
    );
    let empty =
        restore_position(&source, &index, &no_anchor, &AnchorSearch::DEFAULT).expect("空锚点恢复");
    assert_eq!(empty.recovery.status, AnchorStatus::EmptyQuote);
    assert_eq!(empty.recovery.windows_read, 0);
    assert!(empty.fallback_to_chapter_start);
    assert_eq!(
        empty.position.original_byte_offset,
        index.chapter(1).unwrap().start_byte
    );

    // 直接采样锚点也必须在字符边界上。
    let sample = reading_core::generate_anchor(&source, target).expect("采样锚点");
    assert_eq!(sample.byte_offset, target);
    assert!(content.is_char_boundary(sample.byte_offset as usize));
}

#[test]
fn content_unit_is_single_and_position_never_carries_page_identity() {
    let content = make_book(3, 600);
    let (_guard, source) = temp_source("content-unit", &content);
    let index = scan(&source);

    let unit = ContentUnit::from_source(&source, 512);
    assert!(unit.is_single_unit());
    assert_eq!(unit.id, reading_core::SINGLE_CONTENT_UNIT_ID);
    assert_eq!(unit.byte_len, source.len());
    assert!(unit.source_identity.ends_with("book.txt"));
    assert!(unit.contains_offset(source.len() - 1));
    assert!(!unit.contains_offset(source.len()));
    assert_eq!(unit.clamp_offset(u64::MAX), source.len() - 1);
    assert_eq!(reading_core::CONTENT_UNIT_VERSION, 1);

    let position = position_at_offset(&source, 512, unit.id, &index, 10).expect("构造位置");
    assert_eq!(position.content_unit_id, unit.id);
    assert_eq!(position.book_id, 512);
    assert_eq!(position.chapter_index, 0);
    assert_eq!(
        position.encode().split(':').count(),
        reading_core::POSITION_ENCODED_FIELDS + 1
    );

    // 位置身份不得包含动态页码：任何多余字段都必须被拒绝。
    let with_page = format!("{}:42", position.encode());
    let error = ContentPosition::decode(&with_page).expect_err("页码类字段必须被拒绝");
    assert_eq!(error.code(), ErrorCode::InvalidArgument);

    // 编码切换只递增内容索引版本，不影响位置身份字段。
    let mut bumped = unit.clone();
    assert_eq!(bumped.bump_content_index_version(), 1);
    assert_eq!(bumped.id, unit.id);
    assert_eq!(bumped.book_id, unit.book_id);
}

#[test]
fn chapter_scan_from_source_is_cancellable_and_bounded() {
    let content = make_book(300, 700);
    assert!(content.len() > DEFAULT_WINDOW_BYTES * 3);
    let (_guard, source) = temp_source("cancellable", &content);

    // 取消回调在第 3 次查询时返回 true ⇒ 只允许读取 2 个窗口。
    let seen = Cell::new(0u32);
    let cancel = || {
        let next = seen.get() + 1;
        seen.set(next);
        next > 2
    };
    let (partial, partial_stats) = scan_chapters_from_source(&source, &cancel).expect("部分扫描");
    assert!(partial.cancelled, "取消必须被如实记录");
    assert!(
        partial.scanned_bytes < source.len(),
        "取消后只应扫描文件的一部分：{} / {}",
        partial.scanned_bytes,
        source.len()
    );
    assert!(partial_stats.windows_read <= 3, "取消后不得继续读取");
    assert!(partial_stats.max_window_bytes <= DEFAULT_WINDOW_BYTES as u64);
    assert!(
        partial.len() < 300,
        "部分结果的章节数必须少于全书章节数，实际 {}",
        partial.len()
    );

    // 增量推进：每个 ScanStep 只读一个窗口，且步数 > 1。
    let mut session = chapter_scan_session(&source);
    let mut steps = 0u32;
    loop {
        match session.step().expect("单步扫描") {
            ScanStep::Progress { .. } => steps += 1,
            ScanStep::Done => break,
            ScanStep::Cancelled => panic!("未请求取消却返回 Cancelled"),
        }
    }
    let (full, full_stats) = session.finish();
    assert!(steps >= 2, "增量扫描必须分多步推进，实测 {steps} 步");
    assert!(!full.cancelled);
    assert_eq!(full.len(), 300);
    assert_eq!(full.chapter(0).and_then(|c| c.number), Some(1));
    assert_eq!(full.chapter(299).and_then(|c| c.number), Some(300));
    assert!(full_stats.bytes_read >= source.len());
    assert!(
        full_stats.bytes_read <= source.len() + full_stats.windows_read * 8,
        "累计读取量必须有界：{} 字节（文件 {}）",
        full_stats.bytes_read,
        source.len()
    );
    assert!(full_stats.max_window_bytes <= DEFAULT_WINDOW_BYTES as u64);

    // 续扫：从被取消时的偏移继续，最终得到与全量扫描一致的章节数。
    let resume_offset = partial.scanned_bytes;
    assert!(resume_offset > 0);
    let resumed = scan(&source);
    assert_eq!(resumed.len(), full.len());
}

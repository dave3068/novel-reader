//! RC4 集成测试（reading-index）：章节规则、增量/可取消扫描、虚拟章节回退与锚点就近恢复。
//!
//! 所有用例都使用真实临时文件 + [`FileTextSource`]，不依赖 `/tmp` 之外的夹具，
//! 因此 `cargo test --workspace` 在夹具目录被清理后仍然通过。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use reading_index::anchor::{
    make_anchor_from_source, AnchorSearch, AnchorStatus, ANCHOR_QUOTE_CHARS,
    DEFAULT_ANCHOR_RADIUS_BYTES, MAX_ANCHOR_RADIUS_BYTES,
};
use reading_index::chapter::{
    detect_chapter_title, ChapterIndex, ChapterRule, ChapterScanSession, ChapterScanner, ScanStep,
    MAX_LINE_BYTES, MAX_TITLE_LINE_BYTES, MIN_STANDALONE_NUMERIC_SECTIONS,
};
use reading_source::{
    is_utf8_boundary, FileTextSource, TextSource, DEFAULT_WINDOW_BYTES, MAX_WINDOW_BYTES,
};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempGuard {
    dir: PathBuf,
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn temp_book(label: &str, bytes: &[u8]) -> (TempGuard, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "nr-rc4-index-{}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
        label
    ));
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    let path = dir.join("book.txt");
    std::fs::write(&path, bytes).expect("写入临时文件");
    (TempGuard { dir }, path)
}

fn temp_source(label: &str, content: &str) -> (TempGuard, FileTextSource) {
    let (guard, path) = temp_book(label, content.as_bytes());
    let source = FileTextSource::open(&path).expect("打开临时文件");
    (guard, source)
}

fn scan_all(source: &FileTextSource) -> ChapterIndex {
    let (index, _stats) =
        reading_index::chapter::scan_chapters(source, &|| false).expect("章节扫描");
    index
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

/// 构造：楔子 + 三章正文，其中第一章内同一段落重复三次（间隔约 10.5 KiB）。
/// 返回 `(文本, 三次重复段落的起始字节偏移)`。
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

#[test]
fn chapter_rule_detection_covers_required_patterns() {
    let cases: [(&str, ChapterRule, Option<u64>); 19] = [
        ("第一章", ChapterRule::ChapterNumber, Some(1)),
        ("第1章", ChapterRule::ChapterNumber, Some(1)),
        ("第001章", ChapterRule::ChapterNumber, Some(1)),
        ("第 12 章", ChapterRule::ChapterNumber, Some(12)),
        ("第十二章 初见", ChapterRule::ChapterNumber, Some(12)),
        ("第一章初见", ChapterRule::ChapterNumber, Some(1)),
        ("第321章", ChapterRule::ChapterNumber, Some(321)),
        ("第一卷", ChapterRule::VolumeNumber, Some(1)),
        ("卷一", ChapterRule::VolumeNumber, Some(1)),
        ("楔子", ChapterRule::Keyword, None),
        ("序章", ChapterRule::Keyword, None),
        ("前言", ChapterRule::Keyword, None),
        ("后记", ChapterRule::Keyword, None),
        ("尾声", ChapterRule::Keyword, None),
        ("楔子：少年", ChapterRule::Keyword, None),
        ("Chapter 1", ChapterRule::English, Some(1)),
        ("chapter12", ChapterRule::English, Some(12)),
        ("CHAPTER 3 归途", ChapterRule::English, Some(3)),
        ("12", ChapterRule::StandaloneNumeric, Some(12)),
    ];
    for (line, rule, number) in cases {
        let found = detect_chapter_title(line).unwrap_or_else(|| panic!("{line} 必须命中章节规则"));
        assert_eq!(found.rule, rule, "规则不符: {line}");
        assert_eq!(found.number, number, "序号不符: {line}");
        assert_eq!(found.title, line.trim(), "原始标题必须保留");
        assert!(!found.normalized_title.is_empty());
        assert!(found.normalized_title.chars().all(|ch| !ch.is_whitespace()));
    }

    // 归一化：全角标点/空白/大小写统一。
    let found = detect_chapter_title("第一章　初见").expect("全角空格标题");
    assert_eq!(found.normalized_title, "第一章初见");
    let found = detect_chapter_title("CHAPTER 3").expect("英文标题");
    assert_eq!(found.normalized_title, "chapter3");
    let found = detect_chapter_title("第001章").expect("前导零标题");
    assert_eq!(found.number, Some(1));

    // 反向用例：正文/超长/越界写法不得被当作标题。
    let long_title = format!("第一章{}", "很长".repeat(40));
    let negatives = [
        "这是一行正文，不是标题。",
        "他说第一章的内容很重要，但整句是叙述而不是标题说明。",
        "第",
        "章",
        "第三篇",
        "12345",
        "Chapter",
        "chapterhouse",
        "序章之后发生的事情还需要继续叙述很久很久",
        long_title.as_str(),
    ];
    for line in negatives {
        assert!(
            detect_chapter_title(line).is_none(),
            "不应被判为章节标题: {line}"
        );
    }
    assert!(detect_chapter_title("").is_none());
    assert!(detect_chapter_title("   ").is_none());
    assert!(detect_chapter_title(&"甲".repeat(MAX_TITLE_LINE_BYTES)).is_none());
}

#[test]
fn chapter_scan_builds_contiguous_ranges_covering_whole_file() {
    let content = "楔子\n正文甲。\n\n第一章 初见\n正文乙。\n\n第二章 归途\n正文丙。\n\n第三章 终章\n正文丁。\n";
    let (_guard, source) = temp_source("contiguous", content);
    let index = scan_all(&source);

    assert!(!index.is_virtual, "有标题时不应回退虚拟章节");
    assert_eq!(index.len(), 4);
    assert_eq!(index.total_bytes, source.len());
    assert_eq!(index.chapter(0).map(|c| c.rule), Some(ChapterRule::Keyword));
    assert_eq!(index.chapter(0).map(|c| c.title.as_str()), Some("楔子"));
    assert_eq!(
        index.chapter(1).map(|c| c.title.as_str()),
        Some("第一章 初见")
    );
    assert_eq!(index.chapter(1).and_then(|c| c.number), Some(1));
    assert_eq!(index.chapter(2).and_then(|c| c.number), Some(2));
    assert_eq!(index.chapter(3).and_then(|c| c.number), Some(3));

    // 区间必须连续、无重叠、覆盖 [0, len)，且首个章节从文件起点开始。
    assert_eq!(index.chapter(0).map(|c| c.start_byte), Some(0));
    for position in 0..index.len() {
        let chapter = index.chapter(position as u32).expect("章节存在");
        assert_eq!(chapter.index, position as u32);
        assert!(chapter.end_byte > chapter.start_byte, "章节区间必须非空");
        if let Some(next) = index.chapter(position as u32 + 1) {
            assert_eq!(chapter.end_byte, next.start_byte, "区间必须连续");
        } else {
            assert_eq!(chapter.end_byte, source.len(), "末章必须到文件末尾");
        }
    }

    // 任一偏移都能归属到章节（覆盖整个文件）。
    for offset in [0u64, 1, source.len() / 2, source.len() - 1] {
        assert!(
            index.chapter_for_offset(offset).is_some(),
            "offset={offset}"
        );
    }
    assert_eq!(index.chapter_for_offset(0), Some(0));
    assert_eq!(
        index.chapter_for_offset(index.chapter(2).expect("第二章").start_byte),
        Some(2)
    );
    assert_eq!(
        index.chapter_for_offset(index.chapter(2).expect("第二章").start_byte - 1),
        Some(1)
    );
    assert_eq!(
        index.chapter_for_offset(source.len()),
        Some(3),
        "越界偏移夹紧到最后一章"
    );
}

#[test]
fn no_chapter_text_falls_back_to_single_virtual_chapter() {
    let content = "这是一段没有任何章节标题的正文，只有叙述。\n\n第二段内容。\n";
    let (_guard, source) = temp_source("virtual", content);
    let index = scan_all(&source);

    assert!(index.is_virtual, "无章节命中必须回退到虚拟全文章节");
    assert_eq!(index.len(), 1);
    let chapter = index.chapter(0).expect("虚拟章节");
    assert_eq!(chapter.rule, ChapterRule::VirtualFullText);
    assert_eq!(chapter.title, "全文");
    assert_eq!(chapter.start_byte, 0);
    assert_eq!(chapter.end_byte, source.len());
    assert_eq!(index.chapter_for_offset(source.len() - 1), Some(0));

    // 回退后文件仍然可读（虚拟章节不改变数据源）。
    let window = source
        .read_at(0, source.default_window_bytes())
        .expect("窗口读取");
    assert!(!window.is_empty());
    assert_eq!(window.as_str().expect("合法 UTF-8"), content);
}

#[test]
fn chapter_standalone_numeric_titles_require_length_context_and_monotonic_neighbours() {
    // A. 三次出现、前后空行、严格递增 ⇒ 全部采纳。
    let accepted = "1\n\n第一段正文。\n\n2\n\n第二段正文。\n\n3\n\n第三段正文。\n";
    let (_guard, source) = temp_source("numeric-accepted", accepted);
    let index = scan_all(&source);
    assert!(!index.is_virtual);
    let numbers: Vec<Option<u64>> = index.chapters.iter().map(|c| c.number).collect();
    assert_eq!(numbers, vec![Some(1), Some(2), Some(3)]);
    assert!(index
        .chapters
        .iter()
        .all(|c| c.rule == ChapterRule::StandaloneNumeric));
    assert_eq!(index.chapter(0).map(|c| c.start_byte), Some(0));

    // B. 只出现一次 ⇒ 不满足「相邻章节条件」，整批丢弃并回退虚拟章节。
    let single = "正文开头。\n\n7\n\n正文结尾。\n";
    let (_guard, source) = temp_source("numeric-single", single);
    let index = scan_all(&source);
    assert!(index.is_virtual, "仅一处独立数字标题不得被采纳");
    assert_eq!(index.numeric_candidates, 1);
    assert!(index.numeric_candidates < MIN_STANDALONE_NUMERIC_SECTIONS as u32);

    // C. 序号不递增 ⇒ 倒退的那一个被丢弃。
    let unordered = "1\n\n甲。\n\n2\n\n乙。\n\n10\n\n丙。\n\n5\n\n丁。\n";
    let (_guard, source) = temp_source("numeric-unordered", unordered);
    let index = scan_all(&source);
    let numbers: Vec<Option<u64>> = index.chapters.iter().map(|c| c.number).collect();
    assert_eq!(numbers, vec![Some(1), Some(2), Some(10)]);
    assert_eq!(index.numeric_candidates, 4);

    // D. 缺少空行上下文 ⇒ 全部丢弃。
    let no_context = "背景说明\n1\n后续叙述很长\n2\n更多叙述\n";
    let (_guard, source) = temp_source("numeric-context", no_context);
    let index = scan_all(&source);
    assert!(index.is_virtual, "无空行上下文时独立数字不得被采纳");
    assert_eq!(index.numeric_candidates, 2);
}

#[test]
fn chapter_scan_is_incremental_and_cancellable() {
    let content = make_book(400, 700);
    assert!(content.len() > DEFAULT_WINDOW_BYTES * 3);
    let (_guard, source) = temp_source("incremental", &content);

    let mut session = ChapterScanSession::new(&source);
    let mut steps = 0u32;
    loop {
        match session.step().expect("单步扫描") {
            ScanStep::Progress { offset, .. } => {
                steps += 1;
                assert!(offset > 0);
            }
            ScanStep::Done => break,
            ScanStep::Cancelled => panic!("未请求取消却返回 Cancelled"),
        }
    }
    let stats = session.stats();
    let (index, stats_after) = session.finish();
    assert_eq!(stats, stats_after);
    assert!(steps >= 2, "增量扫描必须按窗口分多步推进，实测 {steps} 步");
    assert_eq!(index.len(), 400);
    assert_eq!(index.chapter(0).and_then(|c| c.number), Some(1));
    assert_eq!(index.chapter(399).and_then(|c| c.number), Some(400));
    assert!(!index.cancelled);
    assert!(
        stats.max_window_bytes <= MAX_WINDOW_BYTES as u64,
        "单次物化字节必须 ≤ 窗口硬上限"
    );
    assert!(stats.max_window_bytes <= DEFAULT_WINDOW_BYTES as u64);
    // 窗口两端对齐会重读少量字节，因此累计读取量略大于文件长度但有界。
    assert!(stats.bytes_read >= source.len());
    assert!(
        stats.bytes_read <= source.len() + stats.windows_read * 8,
        "累计读取量必须有界：{} 字节（文件 {}，窗口 {}）",
        stats.bytes_read,
        source.len(),
        stats.windows_read
    );

    // 取消：读取 1 个窗口后取消 ⇒ 结果是部分结果，且不再继续读取。
    let mut session = ChapterScanSession::new(&source);
    assert!(matches!(
        session.step().expect("首步"),
        ScanStep::Progress { .. }
    ));
    let windows_after_first = session.stats().windows_read;
    session.cancel();
    assert!(matches!(
        session.step().expect("取消后的步"),
        ScanStep::Cancelled
    ));
    let (partial, partial_stats) = session.finish();
    assert!(partial.cancelled, "取消必须被如实记录");
    assert_eq!(partial_stats.windows_read, windows_after_first);
    assert!(
        partial.scanned_bytes < source.len(),
        "部分结果的扫描字节必须小于文件长度"
    );
    assert!(partial.len() < index.len());
}

#[test]
fn chapter_scan_is_bounded_on_long_line_input() {
    // 单行 200 KiB（远超 MAX_LINE_BYTES）+ 后续章节：超长行必须被截断，缓冲区不得增长。
    let mut content = String::from("第一章 标题\n");
    content.push_str(&"甲".repeat(200 * 1024));
    content.push('\n');
    content.push_str("第二章 标题\n正文。\n");
    let (_guard, path) = temp_book("long-line", content.as_bytes());
    let bytes = std::fs::read(&path).expect("读取临时文件");
    assert_eq!(bytes.len(), content.len());

    let mut scanner = ChapterScanner::new();
    for (index, chunk) in bytes.chunks(DEFAULT_WINDOW_BYTES).enumerate() {
        scanner.push_bytes((index * DEFAULT_WINDOW_BYTES) as u64, chunk);
        assert!(
            scanner.buffered_line_bytes() <= MAX_LINE_BYTES,
            "未完成行缓冲不得超过 {MAX_LINE_BYTES}，实测 {}",
            scanner.buffered_line_bytes()
        );
    }
    let total = bytes.len() as u64;
    let index = scanner.finish(total);
    assert_eq!(index.len(), 2, "超长行不得成为章节标题");
    assert_eq!(
        index.chapter(0).map(|c| c.title.as_str()),
        Some("第一章 标题")
    );
    assert_eq!(
        index.chapter(1).map(|c| c.title.as_str()),
        Some("第二章 标题")
    );
    assert!(index.truncated_lines >= 1, "超长行必须被截断并计数");
    for chapter in &index.chapters {
        assert!(
            chapter.title.len() <= MAX_TITLE_LINE_BYTES,
            "章节标题必须是有界的短行"
        );
    }

    // 通过数据源扫描：每个窗口都 ≤ 默认窗口，累计读取量有界（不漏读、不放大）。
    let source = FileTextSource::open(&path).expect("打开临时文件");
    let (index, stats) = reading_index::chapter::scan_chapters(&source, &|| false).expect("扫描");
    assert_eq!(stats.max_window_bytes, DEFAULT_WINDOW_BYTES as u64);
    assert!(stats.bytes_read >= source.len());
    assert!(
        stats.bytes_read <= source.len() + stats.windows_read * 8,
        "累计读取量必须有界：{} 字节（文件 {}）",
        stats.bytes_read,
        source.len()
    );
    assert_eq!(index.len(), 2);
}

#[test]
fn anchor_recovers_nearest_occurrence_in_repeated_paragraphs() {
    let (content, offsets) = repeated_paragraph_book();
    let (_guard, source) = temp_source("anchor-repeated", &content);
    let target = offsets[1];
    let sample = make_anchor_from_source(&source, target).expect("生成锚点");
    assert_eq!(sample.byte_offset, target, "字符边界处采样必须原地对齐");
    assert_eq!(sample.fingerprint.char_len as usize, ANCHOR_QUOTE_CHARS);
    assert!(sample.fingerprint.is_consistent());
    let expected_quote: String = content[target as usize..]
        .chars()
        .take(ANCHOR_QUOTE_CHARS)
        .collect();
    assert_eq!(
        sample.fingerprint.quote, expected_quote,
        "锚点引用必须是原位置的 UTF-8 前缀"
    );

    // 默认半径覆盖三次重复 ⇒ 命中数 ≥ 3，且必须选中距离为 0 的那一次。
    let wide = reading_index::anchor::locate_anchor_bounded(
        &source,
        target,
        &sample.fingerprint,
        &AnchorSearch::DEFAULT,
    )
    .expect("宽半径恢复");
    assert_eq!(wide.status, AnchorStatus::Found);
    assert_eq!(wide.byte_offset, target, "必须回到原位置而不是第一次出现");
    assert_eq!(wide.distance_bytes, 0);
    assert!(
        wide.candidates >= 3,
        "同一段落重复三次，宽半径内候选应 ≥ 3，实测 {}",
        wide.candidates
    );
    assert_ne!(wide.byte_offset, offsets[0], "不得跳到第一次出现");
    assert_ne!(wide.byte_offset, offsets[2], "不得跳到第三次出现");
    assert!(
        wide.byte_offset > offsets[0] && wide.byte_offset < offsets[2],
        "命中必须落在中间那一次出现"
    );

    // 窄半径只覆盖最近的一次 ⇒ 候选唯一，命中位置不变。
    let narrow = reading_index::anchor::locate_anchor_bounded(
        &source,
        target,
        &sample.fingerprint,
        &AnchorSearch::new(2048),
    )
    .expect("窄半径恢复");
    assert_eq!(narrow.status, AnchorStatus::Found);
    assert_eq!(narrow.byte_offset, target);
    assert_eq!(narrow.candidates, 1);
    assert_eq!(narrow.windows_read, 1, "有界半径只应读一个窗口");
    let quote_len = sample.fingerprint.byte_len as u64;
    assert!(
        narrow.bytes_read <= 2 * 2048 + 2 * quote_len + 8,
        "有界半径恢复的读取量必须与半径成正比（含 ≤1 个引用长度的窗口重叠），实测 {}",
        narrow.bytes_read
    );
    assert!(narrow.bytes_read < source.len());
    assert!(narrow.radius_bytes <= MAX_ANCHOR_RADIUS_BYTES);
    assert!(is_utf8_boundary(
        content.as_bytes(),
        narrow.byte_offset as usize
    ));
}

#[test]
fn anchor_search_is_radius_bounded_and_degrades_honestly() {
    let (content, offsets) = repeated_paragraph_book();
    let (_guard, source) = temp_source("anchor-bounded", &content);
    let target = offsets[2];

    // 引用文本只存在于距离很远的位置（第一次出现），窄半径内应当 NotFound。
    let distant = reading_index::anchor::make_anchor(&content, offsets[0]);
    let far_target = offsets[2] + 4096;
    let not_found = reading_index::anchor::locate_anchor_bounded(
        &source,
        far_target,
        &distant,
        &AnchorSearch::new(1024),
    )
    .expect("半径受限恢复");
    assert_eq!(
        not_found.status,
        AnchorStatus::NotFound,
        "超出半径的引用不得被命中"
    );
    assert_eq!(not_found.candidates, 0);
    assert_eq!(not_found.byte_offset, far_target, "未命中时保持目标偏移");
    assert!(
        not_found.bytes_read <= 2 * 1024 + 2 * distant.byte_len as u64 + 8,
        "未命中时的读取量同样必须有界（含 ≤1 个引用长度的窗口重叠），实测 {}",
        not_found.bytes_read
    );

    // 空指纹：不做任何读取。
    let empty = reading_index::anchor::NearbyFingerprint::empty();
    let none = reading_index::anchor::locate_anchor_bounded(
        &source,
        target,
        &empty,
        &AnchorSearch::DEFAULT,
    )
    .expect("空指纹恢复");
    assert_eq!(none.status, AnchorStatus::EmptyQuote);
    assert_eq!(none.windows_read, 0);
    assert_eq!(none.bytes_read, 0);

    // 半径夹紧：请求超上限半径不得放大读取量；在远大于半径的文件上验证「不整本读取」。
    let filler = "填充内容。".repeat(60_000);
    let big = format!("{filler}{REPEATED_PARAGRAPH}{filler}{filler}{filler}");
    let (_big_guard, big_source) = temp_source("anchor-clamped", &big);
    let big_target = filler.len() as u64;
    let big_anchor = reading_index::anchor::make_anchor(&big, big_target);
    let before = big_source.stats();
    let clamped = AnchorSearch::new(u64::MAX);
    assert_eq!(clamped.effective_radius(), MAX_ANCHOR_RADIUS_BYTES);
    let recovered = reading_index::anchor::locate_anchor_bounded(
        &big_source,
        big_target,
        &big_anchor,
        &clamped,
    )
    .expect("夹紧半径恢复");
    assert_eq!(recovered.radius_bytes, MAX_ANCHOR_RADIUS_BYTES);
    assert_eq!(recovered.status, AnchorStatus::Found);
    assert_eq!(recovered.byte_offset, big_target);
    assert!(
        recovered.bytes_read < big.len() as u64 / 4,
        "恢复不得整本读取：读取 {} 字节 / 文件 {} 字节",
        recovered.bytes_read,
        big.len()
    );
    let after = big_source.stats();
    assert!(
        after.max_materialized_bytes <= MAX_WINDOW_BYTES as u64,
        "单窗口物化字节必须 ≤ 硬上限"
    );
    assert_eq!(
        after.total_materialized_bytes - before.total_materialized_bytes,
        recovered.bytes_read,
        "报告的读取量必须与实际读取量一致"
    );
    // 常量关系用局部变量断言，避免 clippy `assertions_on_constants`。
    let default_radius: u64 = DEFAULT_ANCHOR_RADIUS_BYTES;
    let max_radius: u64 = MAX_ANCHOR_RADIUS_BYTES;
    assert!(default_radius < max_radius);
}

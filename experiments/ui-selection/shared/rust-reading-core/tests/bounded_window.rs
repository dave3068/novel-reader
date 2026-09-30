//! 有限窗口与内存上界契约测试（S-R2）。
//!
//! 清单入口：`cargo test --release bounded_window_`

use std::io::Write;
use std::path::{Path, PathBuf};

use rust_reading_core::source::{FileTextSource, SyntheticTextSource, TextSource};
use rust_reading_core::{Book, BookSource, Library, ReadingEngine, Rng, DEFAULT_MAX_WINDOW};

const POOL: [char; 24] = [
    '床', '前', '明', '月', '光', '疑', '是', '地', '上', '霜', '举', '头', '望', '低', '思', '故',
    '乡', '春', '眠', '不', '觉', '晓', '处', '闻',
];

/// 构造只含一本书的自定义书库（可指定正文规模与章节数）。
fn library_with(book_id: u64, total_bytes: u64, chapter_count: u32) -> Library {
    Library {
        seed: 20260905,
        categories: Vec::new(),
        books: vec![Book {
            id: book_id,
            title: "大文件夹具".to_string(),
            author: "夹具".to_string(),
            category_id: 1,
            chapter_count,
            total_bytes,
        }],
        history: Vec::new(),
    }
}

/// 生成（或复用）指定大小的确定性 UTF-8 文本文件。
fn large_file(target: u64, seed: u64) -> PathBuf {
    let path = std::env::temp_dir().join(format!("nr-probe-{target}-{seed}.txt"));
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() == target {
            return path;
        }
    }
    let mut rng = Rng::new(seed);
    let mut block = String::with_capacity(64 * 1024);
    while block.len() < 64 * 1024 {
        let n = rng.range_inclusive(20, 60);
        for _ in 0..n {
            block.push(*rng.pick(&POOL));
        }
        block.push('。');
        block.push('\n');
    }
    let block_bytes = block.into_bytes();
    let mut file = std::io::BufWriter::new(std::fs::File::create(&path).expect("创建夹具失败"));
    let mut written = 0u64;
    while written < target {
        file.write_all(&block_bytes).expect("写入夹具失败");
        written += block_bytes.len() as u64;
    }
    file.flush().expect("刷新夹具失败");
    path
}

#[test]
fn bounded_window_aligns_approximate_chapter_start() {
    // 章节起点是按字节均分得到的近似偏移，可能落在字符中间；
    // 读取必须向后对齐到合法 UTF-8 边界并如实报告丢弃字节数（S-R2）。
    let dir = std::env::temp_dir().join("nr-probe-chapter-align");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ch.txt");
    let text = "测".repeat(10); // 30 字节，每字 3 字节
    std::fs::write(&path, text.as_bytes()).unwrap();

    // chapter_count = 3 → 章节起点 0 / 10 / 20 字节（10、20 都落在字符中间）
    let mut engine = ReadingEngine::with_library(library_with(21, 30, 3), 4096);
    engine.open_book(21, BookSource::File(path)).unwrap();

    let w0 = engine.read_window(0, 0, 4096).unwrap();
    assert_eq!(w0.chapter_offset, 0);
    assert_eq!(w0.dropped_leading, 0);

    // 章节内偏移以“章节近似起点”为原点；返回的 chapter_offset 是首个合法位置。
    let w1 = engine.read_window(1, 0, 4096).unwrap();
    assert_eq!(w1.dropped_leading, 2, "偏移 10 落在第 4 个字的第 2 字节");
    assert_eq!(w1.chapter_offset, 2);
    assert_eq!(w1.offset, 12, "全局偏移必须对齐到合法边界");
    assert!(std::str::from_utf8(&w1.bytes).is_ok());

    let w2 = engine.read_window(2, 0, 4096).unwrap();
    assert_eq!(w2.dropped_leading, 1, "偏移 20 落在第 7 个字的第 3 字节");
    assert_eq!(w2.chapter_offset, 1);
    assert_eq!(w2.offset, 21);
    assert!(std::str::from_utf8(&w2.bytes).is_ok());

    // 对齐后文本仍为完整字符序列（章节 1 只剩 10 字节窗口）
    assert_eq!(w1.text().chars().count(), 2);
    assert_eq!(w2.text().chars().count(), 3);

    // 幂等：用返回的 chapter_offset 再读一次，不应再发生对齐丢弃
    let again = engine.read_window(1, w1.chapter_offset, 64).unwrap();
    assert_eq!(again.dropped_leading, 0);
    assert_eq!(again.chapter_offset, w1.chapter_offset);
    assert_eq!(again.text(), &w1.text()[..again.bytes.len()]);
}

#[test]
fn bounded_window_reads_100mb_file_in_windows() {
    let target = 100 * 1024 * 1024u64;
    let path = large_file(target, 4242);
    let actual = std::fs::metadata(&path).unwrap().len();
    assert!(actual >= target, "夹具必须达到 100MB");

    // 直接验证文本源：单次读取物化字节数 == 窗口大小，与文件规模无关。
    let source = FileTextSource::open(&path).unwrap();
    assert_eq!(source.total_bytes(), actual);
    let gauge = source.gauge();
    let w = source.read_at(actual - 4096, 4096).unwrap();
    assert_eq!(w.len(), 4096);
    assert_eq!(gauge.bytes(), 4096, "单次读取只能物化窗口大小的字节");

    // 引擎侧：章节覆盖整文件，按大偏移读取。
    let mut engine = ReadingEngine::with_library(library_with(7, actual, 1), DEFAULT_MAX_WINDOW);
    engine.open_book(7, BookSource::File(path.clone())).unwrap();

    let offsets = [
        0u64,
        actual / 4,
        actual / 2,
        actual - 200_000,
        actual - 4096,
    ];
    let mut copied = 0usize;
    for offset in offsets {
        let window = engine
            .read_window(0, offset, 64 * 1024)
            .expect("窗口读取失败");
        assert!(window.bytes.len() <= 64 * 1024, "窗口不得超过请求上限");
        assert!(window.bytes.len() <= engine.max_window());
        assert!(
            std::str::from_utf8(&window.bytes).is_ok(),
            "窗口必须是合法 UTF-8"
        );
        assert!(window.dropped_leading <= 3);
        copied += window.bytes.len();
    }
    let metrics = engine.metrics();
    assert_eq!(metrics.read_calls, offsets.len() as u64);
    assert!(
        metrics.materialized_bytes <= (offsets.len() as u64) * 64 * 1024,
        "物化字节必须与窗口数成正比，而不是与文件规模成正比: {}",
        metrics.materialized_bytes
    );
    assert_eq!(metrics.copied_bytes as usize, copied);

    // 越界
    assert!(engine.read_window(0, actual + 1, 16).is_err());
}

#[test]
fn bounded_window_reads_1gb_synthetic_without_materializing() {
    let one_gb = 1024u64 * 1024 * 1024;
    let source = SyntheticTextSource::new(one_gb, 9);
    assert_eq!(source.total_bytes(), one_gb);
    let gauge = source.gauge();
    let tail = source.read_at(one_gb - 8192, 4096).unwrap();
    assert_eq!(tail.len(), 4096);
    assert!(std::str::from_utf8(&tail).is_ok());
    assert!(
        gauge.bytes() <= 8 * 1024,
        "1GB 源的窗口读取必须有界: {}",
        gauge.bytes()
    );

    let mut engine = ReadingEngine::with_library(library_with(9, one_gb, 64), DEFAULT_MAX_WINDOW);
    engine
        .open_book(
            9,
            BookSource::Synthetic {
                total_bytes: one_gb,
            },
        )
        .unwrap();
    // 章节 63 覆盖 1GB 的末尾 16MB
    let window = engine
        .read_window(63, 15 * 1024 * 1024, 128 * 1024)
        .unwrap();
    assert!(window.bytes.len() <= 128 * 1024);
    assert!(std::str::from_utf8(&window.bytes).is_ok());
    let metrics = engine.metrics();
    assert!(
        metrics.materialized_bytes <= 256 * 1024,
        "1GB 源单次窗口物化必须远小于文件: {}",
        metrics.materialized_bytes
    );

    // 章节越界与 EOF
    assert!(engine.read_window(64, 0, 16).is_err());
    let eof = engine.read_window(63, 16 * 1024 * 1024, 16).unwrap();
    assert_eq!(eof.bytes.len(), 0);
    assert!(eof.eof);
}

#[test]
fn bounded_window_fixes_utf8_boundaries() {
    let dir = std::env::temp_dir().join("nr-probe-utf8");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("cjk.txt");
    let text = "中".repeat(1000); // 3000 字节，每字 3 字节
    std::fs::write(&path, text.as_bytes()).unwrap();

    let mut engine = ReadingEngine::with_library(library_with(11, text.len() as u64, 1), 64 * 1024);
    engine.open_book(11, BookSource::File(path)).unwrap();

    // 请求 100 字节（非 3 的倍数）→ 必须截断到 99 字节且无非法边界
    let window = engine.read_window(0, 0, 100).unwrap();
    assert_eq!(window.bytes.len(), 99);
    assert!(window.trimmed_tail, "尾部被截断必须标记");
    assert_eq!(window.raw_len, 100);
    assert_eq!(window.text().chars().count(), 33);

    // 从字符中间开始 → 前导续字节被丢弃，偏移被修正
    let mid = engine.read_window(0, 100, 30).unwrap();
    assert_eq!(mid.dropped_leading, 2, "偏移 100 落在第 34 个字中间");
    assert_eq!(mid.offset, 102);
    assert!(std::str::from_utf8(&mid.bytes).is_ok());

    // 连续窗口拼接必须无丢字/重字
    let mut rebuilt = String::new();
    let mut pos = 0u64;
    while pos < text.len() as u64 {
        let w = engine.read_window(0, pos, 97).unwrap();
        if w.bytes.is_empty() {
            break;
        }
        rebuilt.push_str(w.text());
        pos = w.offset + w.bytes.len() as u64;
    }
    assert_eq!(rebuilt, text, "连续窗口拼接必须与原文完全一致");
}

#[test]
fn bounded_window_source_is_utf8_stable_across_blocks() {
    let source = SyntheticTextSource::new(5 * 1024 * 1024, 77);
    // 覆盖块边界（4KiB 的整数倍）读取
    for offset in [4096u64 - 3, 4096, 8192 - 1, 1024 * 1024 + 7] {
        let raw = source.read_at(offset, 3000).unwrap();
        assert_eq!(raw.len(), 3000);
        let fixed = rust_reading_core::engine::fix_utf8_window(raw).unwrap();
        assert!(std::str::from_utf8(&fixed.0).is_ok());
        assert!(fixed.1 <= 3);
    }
    let _ = Path::new("/");
}

#[test]
fn bounded_window_external_content_registers_single_chapter() {
    // 外部内容（如 Android SAF 文件）：共享库只登记坐标空间，字节由平台按窗口提供。
    let declared = 100u64 * 1024;
    let mut engine = ReadingEngine::with_library(library_with(31, 4096, 12), 4096);
    engine.open_external(31, declared).unwrap();

    // 单章：只有 chapter 0 合法
    assert!(engine.read_window(1, 0, 16).is_err());
    assert_eq!(
        engine.read_window(1, 0, 16).unwrap_err().code,
        rust_reading_core::ErrorCode::ChapterOutOfRange
    );

    // 共享库不持有外部字节 → 本地读取为空（而不是伪造内容）
    let window = engine.read_window(0, 0, 4096).unwrap();
    assert!(window.bytes.is_empty());
    assert!(!window.chapter_eof, "空读取不等于到达末尾");

    // 位置与页边界按声明的坐标空间校验
    use rust_reading_core::{ContentPosition, PageBoundary};
    assert!(engine
        .save_position(ContentPosition {
            book_id: 31,
            chapter_index: 0,
            byte_offset: declared
        })
        .is_ok());
    assert!(engine
        .save_position(ContentPosition {
            book_id: 31,
            chapter_index: 0,
            byte_offset: declared + 1
        })
        .is_err());
    assert!(engine
        .commit_page_boundary(PageBoundary {
            chapter_index: 0,
            start_byte: 0,
            end_byte: declared,
            layout_id: 9,
            line_count: 42
        })
        .is_ok());
    assert!(engine
        .commit_page_boundary(PageBoundary {
            chapter_index: 0,
            start_byte: 0,
            end_byte: declared + 1,
            layout_id: 9,
            line_count: 42
        })
        .is_err());

    // 再次打开普通书籍应清除外部坐标空间（本测试书库中的书 id = 31，含 12 章）
    engine
        .open_book(31, BookSource::Synthetic { total_bytes: 0 })
        .unwrap();
    assert!(
        engine.read_window(1, 0, 16).is_ok(),
        "回到模拟书库后章节映射恢复为多章"
    );
    assert!(
        !engine.read_window(0, 0, 4096).unwrap().bytes.is_empty(),
        "回到模拟书库后应能读到真实合成字节（外部占位来源已解除）"
    );
    assert!(engine.open_external(31, 0).is_err(), "长度必须大于 0");
}

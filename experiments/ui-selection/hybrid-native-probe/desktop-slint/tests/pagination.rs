//! 桌面端分页硬门槛测试（H-C1 / SL-AC1 语义）。
//!
//! 走完整桌面路径：共享库有限窗口读取 + 公共分页 API + 桌面适配层。
//! 正向 1000 页、反向重排、配置重排、锚点恢复与非法边界检查。
//!
//! 运行：
//! ```text
//! cargo test --release --test pagination -- --nocapture
//! NR_TEST_PAGES=1000 cargo test --release --test pagination
//! ```

use std::path::PathBuf;

use desktop_slint_probe::page_layouter::{walk_pages, PageLayouter};
use rust_reading_core::position::{locate_anchor, make_anchor, ANCHOR_QUOTE_CHARS};
use rust_reading_core::{Book, LayoutConfig, Library, NrResult, ReadingEngine, TextSource};

/// 测试用内存文本源（仍按字节偏移提供有限窗口）。
struct MemorySource {
    data: Vec<u8>,
}

impl TextSource for MemorySource {
    fn total_bytes(&self) -> u64 {
        self.data.len() as u64
    }

    fn read_at(&self, offset: u64, len: usize) -> NrResult<Vec<u8>> {
        let start = (offset as usize).min(self.data.len());
        let end = (start + len).min(self.data.len());
        Ok(self.data[start..end].to_vec())
    }

    fn describe(&self) -> String {
        format!("memory:{}bytes", self.data.len())
    }
}

fn pages_target() -> usize {
    std::env::var("NR_TEST_PAGES")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(1000)
}

/// 中文语料（确定性）。
fn corpus(paragraphs: usize) -> String {
    let mut rng = rust_reading_core::Rng::new(0xDEAD_BEEF);
    const POOL: [char; 24] = [
        '静', '夜', '思', '床', '前', '明', '月', '光', '疑', '是', '地', '上', '霜', '举', '头',
        '望', '低', '故', '乡', '春', '眠', '不', '觉', '晓',
    ];
    let mut out = String::new();
    for _ in 0..paragraphs {
        let n = rng.range_inclusive(40, 120);
        for i in 0..n {
            out.push(*rng.pick(&POOL));
            if i % 14 == 13 {
                out.push('，');
            }
        }
        out.push('。');
        out.push('\n');
    }
    out
}

fn engine_for(text: &str, max_window: usize) -> ReadingEngine {
    let library = Library {
        seed: 1,
        categories: Vec::new(),
        books: vec![Book {
            id: 1,
            title: "分页夹具".into(),
            author: "夹具".into(),
            category_id: 1,
            chapter_count: 1,
            total_bytes: text.len() as u64,
        }],
        history: Vec::new(),
    };
    let mut engine = ReadingEngine::with_library(library, max_window);
    engine
        .open_source(
            1,
            Box::new(MemorySource {
                data: text.as_bytes().to_vec(),
            }),
        )
        .expect("打开内存文本源失败");
    engine
}

fn report_path() -> PathBuf {
    match std::env::var("NR_PAGINATION_REPORT") {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        _ => PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../results/raw/hybrid-native-desktop-pagination.json"
        )),
    }
}

#[test]
fn pagination_1000_pages_forward_backward_reflow_and_anchor() {
    let target = pages_target();
    let cfg = LayoutConfig::desktop_default();
    // 用共享基座的夹具生成器保证正文足够产生 target 页
    let text = rust_reading_core::fixtures::corpus_for_pages(target, &cfg);
    let mut engine = engine_for(&text, 256 * 1024);
    let mut layouter = PageLayouter::new(cfg.clone(), desktop_slint_probe::WINDOW_BYTES);

    // ---- 正向 ----
    let forward = walk_pages(&mut engine, &mut layouter, 0, 0, target).expect("正向分页失败");
    assert!(
        forward.len() >= target,
        "必须至少产生 {target} 页，实际 {}",
        forward.len()
    );

    let mut expected_start = 0u64;
    let mut rebuilt = String::new();
    for page in forward.iter() {
        assert_eq!(page.start_byte, expected_start, "页起点必须紧邻上一页页末");
        assert!(page.end_byte > page.start_byte, "每页必须前进（无死循环）");
        assert!(
            text.is_char_boundary(page.start_byte as usize),
            "页首必须是合法 UTF-8 边界"
        );
        assert!(
            text.is_char_boundary(page.end_byte as usize),
            "页末必须是合法 UTF-8 边界"
        );
        assert!(!page.text.is_empty(), "不得出现空页");
        expected_start = page.end_byte;
        rebuilt.push_str(&page.text);
    }
    assert_eq!(rebuilt, text[..rebuilt.len()], "不得丢字/重字/拆分");

    // ---- 反向：抽样从页首重新排版，结果必须一致 ----
    let step = (forward.len() / 50).max(1);
    let mut backward_checked = 0usize;
    for page in forward.iter().step_by(step) {
        let again = layouter
            .render(&mut engine, 0, page.start_byte)
            .expect("反向重排失败");
        assert_eq!(again.end_byte, page.end_byte, "反向重排页末必须一致");
        assert_eq!(again.text, page.text, "反向重排页文本必须一致");
        backward_checked += 1;
    }

    // ---- 重排：改变宽度后仍连续，且页边界确实变化 ----
    let mut reflow_cfg = cfg.clone();
    reflow_cfg.width -= 120.0;
    let mut reflow_layouter = PageLayouter::new(reflow_cfg.clone(), 64 * 1024);
    // 重排后页数更多，遍历全文以保证覆盖（上限仅用于防御死循环）
    let reflowed =
        walk_pages(&mut engine, &mut reflow_layouter, 0, 0, 100_000).expect("重排分页失败");
    let mut rebuilt_reflow = String::new();
    let mut last = 0u64;
    for page in reflowed.iter() {
        assert_eq!(page.start_byte, last, "重排后页链必须连续");
        assert!(text.is_char_boundary(page.start_byte as usize));
        assert!(text.is_char_boundary(page.end_byte as usize));
        last = page.end_byte;
        rebuilt_reflow.push_str(&page.text);
    }
    assert_eq!(
        rebuilt_reflow,
        text[..rebuilt_reflow.len()],
        "重排后仍不得丢字/重字"
    );
    let original_starts: Vec<u64> = forward.iter().take(200).map(|p| p.start_byte).collect();
    let reflow_starts: Vec<u64> = reflowed.iter().take(200).map(|p| p.start_byte).collect();
    assert_ne!(original_starts, reflow_starts, "重排必须产生不同的页边界");

    // ---- 锚点：页首引用必须恢复到同一原文位置 ----
    let mut anchor_checked = 0usize;
    for page in forward.iter().step_by(step) {
        let anchor = engine
            .anchor_for_range(0, page.start_byte, page.end_byte)
            .expect("锚点构造失败");
        assert!(!anchor.quote.is_empty());
        assert!(anchor.quote.chars().count() <= ANCHOR_QUOTE_CHARS);
        assert_eq!(
            anchor.quote,
            page.text
                .chars()
                .take(ANCHOR_QUOTE_CHARS)
                .collect::<String>(),
            "锚点引用必须取自页首"
        );
        // 在原文中定位：必须恢复到同一页首（nearest-to-hint 语义）
        let located = locate_anchor(&text, &anchor);
        assert_eq!(
            located, page.start_byte,
            "锚点必须恢复到同一原文位置（页首 {} -> {located}）",
            page.start_byte
        );
        // 该位置必须落在重排后的某一页内（跨页边界仍可恢复）
        let containing = reflowed
            .iter()
            .find(|p| located >= p.start_byte && located < p.end_byte)
            .or_else(|| reflowed.iter().find(|p| p.start_byte == located));
        assert!(containing.is_some(), "重排后必须仍能定位页首 {located}");
        anchor_checked += 1;
    }

    // ---- 证据落盘 ----
    let metrics = engine.metrics();
    let report = format!(
        concat!(
            "{{\n",
            "  \"candidate\": \"hybrid-native\",\n",
            "  \"platform\": \"desktop\",\n",
            "  \"phase\": \"pagination\",\n",
            "  \"status\": \"通过\",\n",
            "  \"framework\": \"slint\",\n",
            "  \"framework_version\": \"1.17.1\",\n",
            "  \"pages\": {},\n",
            "  \"backward_checked\": {},\n",
            "  \"anchor_checked\": {},\n",
            "  \"corpus_bytes\": {},\n",
            "  \"layout_id\": {},\n",
            "  \"reflow_layout_id\": {},\n",
            "  \"read_calls\": {},\n",
            "  \"copied_bytes\": {},\n",
            "  \"materialized_bytes\": {},\n",
            "  \"max_window\": {}\n",
            "}}\n"
        ),
        forward.len(),
        backward_checked,
        anchor_checked,
        text.len(),
        layouter.layout_id(),
        reflow_layouter.layout_id(),
        metrics.read_calls,
        metrics.copied_bytes,
        metrics.materialized_bytes,
        engine.max_window()
    );
    let path = report_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("创建结果目录失败");
    }
    std::fs::write(&path, report).expect("写入分页证据失败");
    println!("分页证据已写入 {}", path.display());
}

#[test]
fn pagination_window_expands_when_page_exceeds_initial_window() {
    let cfg = LayoutConfig::desktop_default();
    let text = corpus(40);
    let mut engine = engine_for(&text, 128 * 1024);
    // 初始窗口远小于一页（1024 字节），适配层必须放大重试
    let mut layouter = PageLayouter::new(cfg, 1024);
    let pages = walk_pages(&mut engine, &mut layouter, 0, 0, 20).expect("放大重试失败");
    assert!(!pages.is_empty());
    for page in pages.iter() {
        assert!(!page.text.is_empty(), "放大重试后不得出现空页");
        assert!(text.is_char_boundary(page.end_byte as usize));
    }
    let mut rebuilt = String::new();
    for page in pages.iter() {
        rebuilt.push_str(&page.text);
    }
    assert_eq!(rebuilt, text[..rebuilt.len()]);
}

#[test]
fn pagination_layout_config_changes_layout_id() {
    let cfg = LayoutConfig::desktop_default();
    let mut layouter = PageLayouter::new(cfg.clone(), 64 * 1024);
    let id_a = layouter.layout_id();
    assert_eq!(id_a, rust_reading_core::layout_id(&cfg));

    let mut next = cfg.clone();
    next.font_size += 4.0;
    layouter.set_config(next.clone());
    let id_b = layouter.layout_id();
    assert_ne!(id_a, id_b, "排版变化必须产生新的布局标识");
    assert_eq!(id_b, rust_reading_core::layout_id(&next));
    assert_eq!(layouter.cache_len(), 0, "改配置必须清空页缓存");

    // 相同配置必须产生相同标识（跨端一致性的前提）
    let mut other = PageLayouter::new(next.clone(), 32 * 1024);
    assert_eq!(other.layout_id(), id_b);
    other.clear_cache();
    assert_eq!(other.cache_len(), 0);
}

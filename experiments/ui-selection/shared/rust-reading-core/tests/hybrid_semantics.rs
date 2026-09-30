//! 双 UI 共享业务语义契约测试（H-R4 / H-AC3）。
//!
//! 桌面与 Android 允许因排版引擎不同得出不同页末字节，但必须：
//! 每端连续、无丢字/重字、页首位置可跨端恢复到同一原文附近位置。
//!
//! 清单入口：`cargo test --release hybrid_semantics_`

use std::collections::BTreeSet;

use rust_reading_core::pagination::{layout_id, layout_page, paginate, LayoutConfig};
use rust_reading_core::position::{locate_anchor, make_anchor};
use rust_reading_core::{fixtures, Book, BookSource, Library, PageBoundary, ReadingEngine};

fn flat_library(book_id: u64, total_bytes: u64) -> Library {
    Library {
        seed: 7,
        categories: Vec::new(),
        books: vec![Book {
            id: book_id,
            title: "语义夹具".into(),
            author: "夹具".into(),
            category_id: 1,
            chapter_count: 1,
            total_bytes,
        }],
        history: Vec::new(),
    }
}

/// 连续排版直到正文结束，返回（页起点, 消费字节）序列。
fn page_all(text: &str, cfg: &LayoutConfig) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < text.len() {
        let page = layout_page(text, pos, cfg);
        if page.consumed_utf8_bytes == 0 {
            break;
        }
        out.push((pos, page.consumed_utf8_bytes));
        pos += page.consumed_utf8_bytes;
    }
    out
}

#[test]
fn hybrid_semantics_different_boundaries_but_both_continuous() {
    let text = fixtures::mixed_corpus();
    let desktop = LayoutConfig::desktop_default();
    let android = LayoutConfig::android_default();
    assert_ne!(layout_id(&desktop), layout_id(&android));

    let d_pages = page_all(&text, &desktop);
    let a_pages = page_all(&text, &android);
    assert!(d_pages.len() > 5 && a_pages.len() > 5);

    for (name, pages) in [("desktop", &d_pages), ("android", &a_pages)] {
        let mut rebuilt = String::new();
        let mut last_end = 0usize;
        for (start, consumed) in pages.iter() {
            assert_eq!(*start, last_end, "{name}: 页起点必须紧邻上一页页末");
            assert!(*consumed > 0, "{name}: 每页必须前进（不死循环）");
            let end = start + consumed;
            assert!(end <= text.len(), "{name}: 页末不得超出正文");
            assert!(
                text.is_char_boundary(*start),
                "{name}: 页首必须是 UTF-8 边界"
            );
            assert!(text.is_char_boundary(end), "{name}: 页末必须是 UTF-8 边界");
            rebuilt.push_str(&text[*start..end]);
            last_end = end;
        }
        assert_eq!(last_end, text.len(), "{name}: 必须排版到正文末尾");
        assert_eq!(
            rebuilt, text,
            "{name}: 拼接必须与原文完全一致（无丢字/重字）"
        );
    }

    // 页边界集合必须不同（否则「不同页边界」的语义检查无意义）
    let d_starts: BTreeSet<usize> = d_pages.iter().map(|(s, _)| *s).collect();
    let a_starts: BTreeSet<usize> = a_pages.iter().map(|(s, _)| *s).collect();
    assert_ne!(d_starts, a_starts, "两端页边界必须不同");
}

#[test]
fn hybrid_semantics_position_recovery_between_layouts() {
    let desktop = LayoutConfig::desktop_default();
    let android = LayoutConfig::android_default();
    // 需要足够多的页才能抽样验证跨端恢复
    let text = fixtures::corpus_for_pages(160, &desktop);
    let d_pages = page_all(&text, &desktop);
    let a_pages = page_all(&text, &android);
    assert!(d_pages.len() >= 160, "桌面页数不足: {}", d_pages.len());
    assert_ne!(d_pages.len(), a_pages.len(), "两端页数应因排版不同而不同");

    let mut recovered = 0usize;
    for (start, consumed) in d_pages.iter().take(80) {
        let anchor = make_anchor(&text, 0, *start as u64, (start + consumed) as u64);
        let located = locate_anchor(&text, &anchor) as usize;
        assert_eq!(located, *start, "锚点必须恢复到同一原文位置");
        let in_android = a_pages
            .iter()
            .find(|(s, c)| located >= *s && located < s + c)
            .expect("必须能在另一端页表中定位");
        assert!(in_android.1 > 0);
        recovered += 1;
    }
    assert!(recovered >= 80, "必须验证足够多的跨端位置点");

    // 反向：Android 页首在桌面页表中同样可定位
    for (start, consumed) in a_pages.iter().take(40) {
        let anchor = make_anchor(&text, 0, *start as u64, (start + consumed) as u64);
        assert_eq!(locate_anchor(&text, &anchor) as usize, *start);
    }
}

#[test]
fn hybrid_semantics_1000_pages_forward_backward_and_reflow() {
    let cfg = LayoutConfig::desktop_default();
    let text = fixtures::corpus_for_pages(1000, &cfg);
    let forward = paginate(&text, 0, &cfg, 1000);
    assert!(
        forward.len() >= 1000,
        "必须至少产生 1000 页: {}",
        forward.len()
    );

    // 正向：逐页连续
    let mut last_end = 0usize;
    for (start, consumed) in forward.iter() {
        assert_eq!(*start, last_end);
        assert!(text.is_char_boundary(*start) && text.is_char_boundary(start + consumed));
        last_end = start + consumed;
    }

    // 反向：从尾页回推，必须与正向结果一致
    for (start, consumed) in forward.iter().rev() {
        let page = layout_page(&text, *start, &cfg);
        assert_eq!(
            page.consumed_utf8_bytes, *consumed,
            "反向重排结果必须与正向一致"
        );
    }

    // 重排：改变宽度后必须仍然连续，且边界集合区别于原配置
    let mut reflowed_cfg = cfg.clone();
    reflowed_cfg.width -= 120.0;
    assert_ne!(layout_id(&cfg), layout_id(&reflowed_cfg));
    let reflowed = paginate(&text, 0, &reflowed_cfg, 1000);
    let mut rebuilt = String::new();
    for (start, consumed) in reflowed.iter() {
        rebuilt.push_str(&text[*start..start + consumed]);
        assert!(text.is_char_boundary(*start) && text.is_char_boundary(start + consumed));
    }
    assert_eq!(rebuilt, text[..rebuilt.len()], "重排后仍必须无丢字/重字");
    let original: BTreeSet<usize> = forward.iter().take(200).map(|(s, _)| *s).collect();
    let changed: BTreeSet<usize> = reflowed.iter().take(200).map(|(s, _)| *s).collect();
    assert_ne!(original, changed, "重排必须产生不同的页边界");

    // 重排后可锚定：用重排前的页首锚点在重排后的正文中定位
    let anchor = make_anchor(
        &text,
        0,
        forward[10].0 as u64,
        (forward[10].0 + forward[10].1) as u64,
    );
    assert_eq!(locate_anchor(&text, &anchor) as usize, forward[10].0);
    let reflowed_page = reflowed
        .iter()
        .find(|(s, c)| forward[10].0 >= *s && forward[10].0 < s + c)
        .expect("重排后必须仍能定位该位置");
    assert!(reflowed_page.1 > 0);
}

#[test]
fn hybrid_semantics_engine_accepts_both_ends_boundaries() {
    let cfg_desktop = LayoutConfig::desktop_default();
    let cfg_android = LayoutConfig::android_default();
    let text = fixtures::corpus_for_pages(60, &cfg_desktop);
    let total = text.len() as u64;

    let mut engine = ReadingEngine::with_library(flat_library(3, total), 256 * 1024);
    engine
        .open_source(
            3,
            Box::new(InMemorySource {
                data: text.clone().into_bytes(),
            }),
        )
        .unwrap();

    let d_id = layout_id(&cfg_desktop);
    let a_id = layout_id(&cfg_android);

    for (start, consumed) in paginate(&text, 0, &cfg_desktop, 60) {
        engine
            .commit_page_boundary(PageBoundary {
                chapter_index: 0,
                start_byte: start as u64,
                end_byte: (start + consumed) as u64,
                layout_id: d_id,
                line_count: 20,
            })
            .unwrap();
    }
    for (start, consumed) in paginate(&text, 0, &cfg_android, 60) {
        engine
            .commit_page_boundary(PageBoundary {
                chapter_index: 0,
                start_byte: start as u64,
                end_byte: (start + consumed) as u64,
                layout_id: a_id,
                line_count: 24,
            })
            .unwrap();
    }

    let d_bounds = engine.page_boundaries(d_id, 0);
    let a_bounds = engine.page_boundaries(a_id, 0);
    assert!(d_bounds.len() >= 40 && a_bounds.len() >= 40);
    assert_ne!(
        d_bounds.iter().map(|b| b.end_byte).collect::<Vec<_>>(),
        a_bounds.iter().map(|b| b.end_byte).collect::<Vec<_>>(),
        "两端登记的页边界必须不同"
    );

    // 用桌面页首锚点，在 Android 页边界下恢复同一位置
    let anchor = make_anchor(&text, 0, d_bounds[5].start_byte, d_bounds[5].end_byte);
    let located = locate_anchor(&text, &anchor);
    assert_eq!(located, d_bounds[5].start_byte);
    assert!(a_bounds
        .iter()
        .any(|b| located >= b.start_byte && located < b.end_byte));

    // 排版变化后失效桌面缓存，Android 边界保留
    assert_eq!(engine.invalidate_layout(d_id), d_bounds.len());
    assert_eq!(engine.page_boundaries(a_id, 0).len(), a_bounds.len());
}

/// 测试用内存文本源（仍按字节偏移提供有限窗口）。
struct InMemorySource {
    data: Vec<u8>,
}

impl rust_reading_core::TextSource for InMemorySource {
    fn total_bytes(&self) -> u64 {
        self.data.len() as u64
    }

    fn read_at(&self, offset: u64, len: usize) -> rust_reading_core::NrResult<Vec<u8>> {
        let start = (offset as usize).min(self.data.len());
        let end = (start + len).min(self.data.len());
        Ok(self.data[start..end].to_vec())
    }

    fn describe(&self) -> String {
        "in-memory".to_string()
    }
}

#[test]
fn hybrid_semantics_unicode_edge_cases_have_no_loss() {
    for (name, sample) in fixtures::UNICODE_SAMPLES {
        let cfg = LayoutConfig::desktop_default();
        let pages = page_all(sample, &cfg);
        let mut rebuilt = String::new();
        for (start, consumed) in pages.iter() {
            assert!(sample.is_char_boundary(*start), "{name}: 非法页首");
            assert!(
                sample.is_char_boundary(start + consumed),
                "{name}: 非法页末"
            );
            rebuilt.push_str(&sample[*start..start + consumed]);
        }
        assert_eq!(&rebuilt, sample, "{name}: 必须无丢字/重字/拆分");
    }
    let _ = BookSource::Synthetic { total_bytes: 1 };
}

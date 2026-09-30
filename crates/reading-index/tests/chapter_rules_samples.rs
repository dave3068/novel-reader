//! 章节规则覆盖**用户真实样本**（真机反馈：`盘龙` 目录只出现两条正文句子）。
//!
//! 用户提供的四种标题形式（原文）：
//! - `深空彼岸`：`第1章 旧土`
//! - `大剑师传奇`：`第1卷第1章 逃出帝國`
//! - `星际浪子`：`第一卷 第一章 方舟一号`
//! - `盘龙`：`第一集 盘龙之戒 第一章 小镇的早晨`
//!
//! 说明：本文件用 UTF-8 夹具聚焦「规则」；**编码链路**由 `reading-ffi` 的 GBK 用例覆盖。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use reading_index::chapter::scan_chapters;
use reading_source::FileTextSource;

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempGuard(PathBuf);

impl Drop for TempGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn temp_source(label: &str, content: &str) -> (TempGuard, FileTextSource) {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "nr-chapter-rules-{}-{}-{}.txt",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst),
        label
    ));
    std::fs::write(&path, content.as_bytes()).expect("写临时夹具");
    let source = FileTextSource::open(&path).expect("打开临时夹具");
    (TempGuard(path), source)
}

fn titles(content: &str) -> Vec<String> {
    let (_guard, source) = temp_source("rules", content);
    let (index, _stats) = scan_chapters(&source, &|| false).expect("章节扫描");
    index.chapters.iter().map(|c| c.title.clone()).collect()
}

#[test]
fn real_world_title_forms_are_all_recognised() {
    let cases: &[(&str, &str, &[&str])] = &[
        (
            "深空彼岸",
            "第1章 旧土\n\n正文……\n\n第2章 韶华易逝\n\n正文……\n",
            &["第1章 旧土", "第2章 韶华易逝"],
        ),
        (
            "大剑师传奇",
            "第1卷第1章 逃出帝國\n\n正文……\n\n第1卷第2章 魔女之秘\n\n正文……\n",
            &["第1卷第1章 逃出帝國", "第1卷第2章 魔女之秘"],
        ),
        (
            "星际浪子",
            "第一卷 第一章 方舟一号\n\n正文……\n\n第一卷 第二章 重返家乡\n\n正文……\n",
            &["第一卷 第一章 方舟一号", "第一卷 第二章 重返家乡"],
        ),
        (
            "盘龙",
            "第一集 盘龙之戒 第一章 小镇的早晨\n\n正文……\n\n第一集 盘龙之戒 第二章 少年的成长\n\n正文……\n",
            &["第一集 盘龙之戒 第一章 小镇的早晨", "第一集 盘龙之戒 第二章 少年的成长"],
        ),
    ];
    for (name, content, expected) in cases {
        let actual = titles(content);
        assert_eq!(
            actual,
            expected.to_vec(),
            "{name} 的章节标题必须全部命中且顺序一致"
        );
    }
}

#[test]
fn narrative_sentences_are_not_chapters() {
    // 用户实测的假阳性原文：这两行是**正文**，不是标题。
    let content = "第二章到！\n\n正文若干……\n\n第四章到！\n\n正文若干……\n";
    let actual = titles(content);
    assert_eq!(
        actual,
        vec!["全文".to_string()],
        "以句末标点结尾的正文句子不得被判为章节（否则目录只剩这两条）"
    );
}

#[test]
fn short_title_without_separator_is_still_accepted() {
    // 无空格的紧凑写法仍应接受（不得因收紧而误伤）。
    let content = "第1章旧土\n\n正文……\n";
    assert_eq!(titles(content), vec!["第1章旧土".to_string()]);
}

//! 确定性测试夹具：中文/繁体/标点/中英数/Emoji/组合字符/缺字/超长行/
//! 空行/混合换行/首行缩进等语料，以及可分页正文生成（S-R4）。

use crate::json;
use crate::pagination::{paginate, LayoutConfig};

/// Unicode 边界样本：(名称, 文本)。
pub const UNICODE_SAMPLES: &[(&str, &str)] = &[
    ("简体中文", "静夜思：床前明月光，疑是地上霜。举头望明月，低头思故乡。"),
    ("繁体中文", "舉頭望明月，低頭思故鄉。春眠不覺曉，處處聞啼鳥。"),
    ("中文标点", "他说：「这是《书》，那（也是）书；对吧？」——嗯……大约三成，约 30%。"),
    ("中英数混排", "Chapter 12 第 12 章 ASCII 1234 mixed 混排测试 ABCdef。"),
    ("Emoji", "阅读进度 100% 📖🎉🚀 家族 👨‍👩‍👧‍👦 组合序列"),
    ("组合字符", "e\u{0301} a\u{0300} o\u{0308} 中文̂ 组合标记"),
    ("缺字符", "缺字占位 \u{FFFD}\u{FFFD} 与私有区 \u{E000}\u{F8FF} 之后仍有正文"),
    ("超长行", "本行故意写得很长很长很长很长很长很长很长很长很长很长很长很长很长很长很长很长很长很长很长很长很长很长很长很长很长很长很长很长很长很长很长，用于验证不裁切不丢字。"),
    ("空行", "第一段。\n\n\n第二段之前有两个空行。\n\n第三段。"),
    ("混合换行", "CRLF 行\r\nLF 行\nCR 行\r尾行"),
    ("首行缩进", "　　全角空格开头的段落，用于验证首行缩进与全角空格宽度的差异。"),
    ("纯空白", "   \t  \n  \t\n"),
];

/// 生成 `count` 个中文段落（每段以 `\n` 结尾，确定性）。
pub fn cjk_paragraphs(count: usize) -> String {
    let mut rng = crate::rng::Rng::new(0x5EED_1234);
    let mut out = String::with_capacity(count * 96);
    for _ in 0..count {
        let n = rng.range_inclusive(18, 64);
        for i in 0..n {
            out.push(*rng.pick(&PARAGRAPH_POOL));
            if i % 12 == 11 {
                out.push(*rng.pick(&PUNCT_POOL));
            }
        }
        out.push('。');
        out.push('\n');
    }
    out
}

const PARAGRAPH_POOL: [char; 40] = [
    '静', '夜', '思', '床', '前', '明', '月', '光', '疑', '是', '地', '上', '霜', '举', '头', '望',
    '低', '故', '乡', '春', '眠', '不', '觉', '晓', '处', '闻', '啼', '鸟', '来', '风', '雨', '声',
    '花', '落', '知', '多', '少', '山', '水', '云',
];
const PUNCT_POOL: [char; 8] = ['，', '。', '；', '：', '、', '？', '！', '—'];

/// 混合语料（覆盖全部边界样本）。
pub fn mixed_corpus() -> String {
    let mut out = String::new();
    for (name, text) in UNICODE_SAMPLES {
        out.push_str(&format!("【{name}】{text}\n"));
    }
    for _ in 0..3 {
        out.push_str(&cjk_paragraphs(64));
    }
    out
}

/// 生成至少 `pages` 页的正文（按给定排版配置校验页数）。
pub fn corpus_for_pages(pages: usize, cfg: &LayoutConfig) -> String {
    let mut text = String::new();
    for _ in 0..64 {
        if !text.is_empty() && paginate(&text, 0, cfg, pages).len() >= pages {
            break;
        }
        text.push_str(&cjk_paragraphs(1024));
    }
    text
}

/// 确定性夹具清单（供 generate-fixtures.py / compare-manifests.py 交叉校验）。
pub fn manifest_json(seed: u64) -> String {
    let lib = crate::domain::Library::generate(seed);
    let corpus = mixed_corpus();
    let cfg = LayoutConfig::desktop_default();
    let pages = paginate(&corpus, 0, &cfg, 100_000).len();
    format!(
        "{{\"seed\":{seed},\"categories\":{},\"books\":{},\"history\":{},\"firstBook\":{},\"corpusBytes\":{},\"corpusChars\":{},\"corpusPages\":{},\"layoutId\":{}}}",
        lib.categories.len(),
        lib.books.len(),
        lib.history.len(),
        json::quoted(&lib.books[0].title),
        corpus.len(),
        corpus.chars().count(),
        pages,
        crate::pagination::layout_id(&cfg)
    )
}

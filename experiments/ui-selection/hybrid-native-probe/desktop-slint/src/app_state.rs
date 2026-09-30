//! 桌面端应用状态。
//!
//! 严格遵循 H-R4：窗口读取、分页、位置、锚点、缓存失效、取消、错误与指标
//! 全部来自共享库 `rust-reading-core`，本层只做状态编排与 UI 投影。

use std::collections::{BTreeSet, HashSet};

use rust_reading_core::position::ANCHOR_QUOTE_CHARS;
use rust_reading_core::{
    Book, BookSource, ContentPosition, LayoutConfig, Library, NrResult, PageAnchor, ReadingEngine,
};

use crate::page_layouter::{walk_pages, PageLayouter, RenderedPage};

/// 模拟数据种子（与共享基座约定一致，保证确定性）。
pub const DEFAULT_SEED: u64 = 20260905;
/// 单次窗口大小。
pub const WINDOW_BYTES: usize = 64 * 1024;
/// 单次窗口上限。
pub const MAX_WINDOW_BYTES: usize = 256 * 1024;
/// 列表投影上限（虚拟化之外的硬上限，避免把 10000 行一次性投影）。
pub const LIST_LIMIT: usize = 2000;

/// 书架行（UI 投影）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookRow {
    pub id: String,
    pub title: String,
    pub author: String,
    pub meta: String,
}

/// 分类行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryRow {
    pub id: String,
    pub name: String,
    pub level: i32,
    pub meta: String,
}

/// 章节行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterRow {
    pub index: i32,
    pub title: String,
}

/// 分页自检结果。
#[derive(Debug, Clone)]
pub struct SelfCheck {
    pub pages: usize,
    pub chapters: u32,
    pub bytes_consumed: u64,
    /// 因章节近似字节偏移落在字符中间而被对齐的章节数（合法修正）。
    pub aligned_chapters: u32,
    pub forward_ok: bool,
    pub backward_ok: bool,
    pub reflow_ok: bool,
    pub anchor_ok: bool,
    pub status: &'static str,
    pub notes: Vec<String>,
}

impl SelfCheck {
    fn empty() -> Self {
        Self {
            pages: 0,
            chapters: 0,
            bytes_consumed: 0,
            aligned_chapters: 0,
            forward_ok: false,
            backward_ok: false,
            reflow_ok: false,
            anchor_ok: false,
            status: "不通过",
            notes: Vec::new(),
        }
    }

    /// 汇总 JSON（写入结果与 ready.json）。
    pub fn to_json(&self) -> String {
        format!(
            "{{\"pages\":{},\"chapters\":{},\"bytes_consumed\":{},\"aligned_chapters\":{},\"forward_ok\":{},\"backward_ok\":{},\"reflow_ok\":{},\"anchor_ok\":{},\"status\":\"{}\"}}",
            self.pages,
            self.chapters,
            self.bytes_consumed,
            self.aligned_chapters,
            self.forward_ok,
            self.backward_ok,
            self.reflow_ok,
            self.anchor_ok,
            self.status
        )
    }
}

/// UI 状态投影。
#[derive(Debug, Clone, Default)]
pub struct AppStatus {
    pub status_text: String,
    pub crumb_text: String,
    pub metrics_text: String,
    pub engine_text: String,
    pub book_title: String,
    pub chapter_title: String,
    pub page_text: String,
    pub page_label: String,
    pub progress_text: String,
    pub font_size: i32,
    pub line_spacing_x10: i32,
    pub page_margin: i32,
    pub background_mode: i32,
    pub theme_dark: bool,
}

/// 桌面阅读应用。
pub struct ReaderApp {
    engine: ReadingEngine,
    layouter: PageLayouter,
    book_id: u64,
    chapter_index: u32,
    page_chain: Vec<u64>,
    page_cursor: usize,
    anchor_quote: String,
    category_path: Vec<u64>,
    query: String,
    layout_invalidations: u64,
    background_mode: i32,
    theme_dark: bool,
    /// UI 实际可用于正文的区域尺寸（逻辑像素，不含页边距）。
    /// 渲染区域与排版配置必须一致，否则页边界与可见区域不匹配
    /// （真机反馈：正文只占上部一小块、下方大片空白，看起来「没有分页」）。
    viewport_area: Option<(f32, f32)>,
    status_note: String,
    self_check: Option<SelfCheck>,
}

impl ReaderApp {
    /// 建立应用：生成模拟书库、打开第一本书、排版首页。
    pub fn new() -> Self {
        let library = Library::generate(DEFAULT_SEED);
        let engine = ReadingEngine::with_library(library, MAX_WINDOW_BYTES);
        // 桌面渲染（Slint `Text`）不叠加段间距、也不做首行缩进，
        // 因此排版配置必须同样置零，否则共享库按「行高 + 段间距」计算，
        // 而界面只按行高渲染 → 每页少排若干行、正文底部留白。
        let mut layout = LayoutConfig::desktop_default();
        layout.paragraph_spacing = 0.0;
        layout.first_line_indent = 0.0;
        let layouter = PageLayouter::new(layout, WINDOW_BYTES);
        let mut app = Self {
            engine,
            layouter,
            book_id: 1,
            chapter_index: 0,
            page_chain: vec![0],
            page_cursor: 0,
            anchor_quote: String::new(),
            category_path: Vec::new(),
            query: String::new(),
            layout_invalidations: 0,
            background_mode: 0,
            theme_dark: false,
            viewport_area: None,
            status_note: "就绪".to_string(),
            self_check: None,
        };
        let total = app.book_total_bytes(1);
        let _ = app
            .engine
            .open_book(1, BookSource::Synthetic { total_bytes: total });
        app
    }

    // ------------------------------------------------------------------
    // 领域投影
    // ------------------------------------------------------------------

    fn book(&self, book_id: u64) -> Option<&Book> {
        self.engine.library().book(book_id)
    }

    fn book_total_bytes(&self, book_id: u64) -> u64 {
        self.book(book_id).map(|b| b.total_bytes).unwrap_or(0)
    }

    /// 当前书。
    pub fn book_id(&self) -> u64 {
        self.book_id
    }

    /// 当前章节序号。
    pub fn chapter_index(&self) -> u32 {
        self.chapter_index
    }

    /// 共享库只读引用（结果记录与指标使用）。
    pub fn engine(&self) -> &ReadingEngine {
        &self.engine
    }

    /// 分页器（测试与结果记录使用）。
    pub fn layouter(&self) -> &PageLayouter {
        &self.layouter
    }

    /// 打开书籍。
    pub fn open_book(&mut self, book_id: u64) -> bool {
        let total = self.book_total_bytes(book_id);
        if total == 0 {
            self.status_note = format!("书籍 {book_id} 不存在");
            return false;
        }
        match self
            .engine
            .open_book(book_id, BookSource::Synthetic { total_bytes: total })
        {
            Ok(()) => {
                self.book_id = book_id;
                self.chapter_index = 0;
                self.page_chain = vec![0];
                self.page_cursor = 0;
                self.layouter.clear_cache();
                self.status_note = format!(
                    "已打开《{}》",
                    self.book(book_id)
                        .map(|b| b.title.clone())
                        .unwrap_or_default()
                );
                true
            }
            Err(err) => {
                self.status_note = format!("打开失败: {err}");
                false
            }
        }
    }

    /// 进入分类。
    pub fn enter_category(&mut self, category_id: u64) {
        self.category_path.push(category_id);
        self.query.clear();
        self.status_note = format!("进入分类 {}", category_id);
    }

    /// 返回上级分类。
    pub fn up_level(&mut self) {
        if self.category_path.pop().is_none() {
            self.status_note = "已在顶层分类".to_string();
        }
    }

    /// 设置搜索词。
    pub fn set_query(&mut self, query: &str) {
        self.query = query.trim().to_string();
        self.status_note = if self.query.is_empty() {
            "已清空搜索".to_string()
        } else {
            format!("搜索「{}」", self.query)
        };
    }

    /// 当前分类下的子树分类 id 集合。
    fn subtree_ids(&self, root: u64) -> BTreeSet<u64> {
        let mut out: BTreeSet<u64> = BTreeSet::new();
        out.insert(root);
        let mut queue = vec![root];
        while let Some(current) = queue.pop() {
            for child in self
                .engine
                .library()
                .categories
                .iter()
                .filter(|c| c.parent_id == current)
            {
                if out.insert(child.id) {
                    queue.push(child.id);
                }
            }
        }
        out
    }

    /// 书架行投影。
    pub fn book_rows(&self) -> Vec<BookRow> {
        if !self.query.is_empty() {
            let mut rows: Vec<BookRow> = self
                .engine
                .library()
                .search_books(&self.query, LIST_LIMIT)
                .into_iter()
                .map(book_row)
                .collect();
            rows.sort_by(|a, b| a.id.cmp(&b.id));
            return rows;
        }
        let allowed: Option<HashSet<u64>> = self
            .category_path
            .last()
            .map(|id| self.subtree_ids(*id).into_iter().collect::<HashSet<u64>>());
        self.engine
            .library()
            .books
            .iter()
            .filter(|b| {
                allowed
                    .as_ref()
                    .map_or(true, |set| set.contains(&b.category_id))
            })
            .take(LIST_LIMIT)
            .map(book_row)
            .collect()
    }

    /// 分类行投影（当前层级的子分类）。
    pub fn category_rows(&self) -> Vec<CategoryRow> {
        let library = self.engine.library();
        let parent = self.category_path.last().copied().unwrap_or(0);
        library
            .categories
            .iter()
            .filter(|c| c.parent_id == parent)
            .map(|c| {
                let subtree = self.subtree_ids(c.id);
                let count = library
                    .books
                    .iter()
                    .filter(|b| subtree.contains(&b.category_id))
                    .count();
                CategoryRow {
                    id: c.id.to_string(),
                    name: c.name.clone(),
                    level: c.level as i32,
                    meta: format!("{count} 本"),
                }
            })
            .collect()
    }

    /// 面包屑文本。
    pub fn crumb_text(&self) -> String {
        if let Some(id) = self.category_path.last() {
            let mut parts = Vec::new();
            let mut current = Some(*id);
            while let Some(cid) = current {
                match self.engine.library().category(cid) {
                    Some(c) => {
                        parts.push(format!("{}（L{}）", c.name, c.level));
                        current = if c.parent_id == 0 {
                            None
                        } else {
                            Some(c.parent_id)
                        };
                    }
                    None => break,
                }
            }
            parts.reverse();
            format!("层级：{}", parts.join(" / "))
        } else {
            format!(
                "层级：顶层（共 {} 个分类 / {} 本书）",
                self.engine.library().categories.len(),
                self.engine.library().books.len()
            )
        }
    }

    /// 章节行投影。
    pub fn chapter_rows(&self) -> Vec<ChapterRow> {
        self.engine
            .chapter_batch(self.book_id, 0, 64)
            .into_iter()
            .map(|c| ChapterRow {
                index: c.index as i32,
                title: c.title,
            })
            .collect()
    }

    // ------------------------------------------------------------------
    // 阅读与分页
    // ------------------------------------------------------------------

    /// 当前页（必要时排版）。
    pub fn current_page(&mut self) -> NrResult<RenderedPage> {
        let offset = self.page_chain.get(self.page_cursor).copied().unwrap_or(0);
        let chapter = self.chapter_index;
        let page = self.layouter.render(&mut self.engine, chapter, offset)?;
        if !page.text.is_empty() {
            self.anchor_quote = page.text.chars().take(ANCHOR_QUOTE_CHARS).collect();
        }
        Ok(page)
    }

    /// 下一页。
    pub fn next_page(&mut self) -> NrResult<()> {
        let page = self.current_page()?;
        if page.is_empty() || page.chapter_eof {
            self.status_note = "已到本章末尾".to_string();
            return Ok(());
        }
        let next = page.end_byte;
        if self.page_cursor + 1 < self.page_chain.len() {
            self.page_cursor += 1;
        } else {
            self.page_chain.push(next);
            self.page_cursor = self.page_chain.len() - 1;
        }
        self.save_position();
        Ok(())
    }

    /// 上一页。
    pub fn prev_page(&mut self) -> NrResult<()> {
        if self.page_cursor > 0 {
            self.page_cursor -= 1;
            self.save_position();
        } else if self.chapter_index > 0 {
            self.seek_chapter(self.chapter_index - 1)?;
        } else {
            self.status_note = "已到本章开头".to_string();
        }
        Ok(())
    }

    /// 跳转到章节开头。
    pub fn seek_chapter(&mut self, chapter_index: u32) -> NrResult<()> {
        let count = self
            .book(self.book_id)
            .map(|b| b.chapter_count)
            .unwrap_or(0);
        if chapter_index >= count {
            self.status_note = format!("章节 {chapter_index} 越界");
            return Ok(());
        }
        self.chapter_index = chapter_index;
        self.page_chain = vec![0];
        self.page_cursor = 0;
        self.save_position();
        Ok(())
    }

    /// 保存当前位置到共享库。
    pub fn save_position(&mut self) {
        let offset = self.page_chain.get(self.page_cursor).copied().unwrap_or(0);
        let position = ContentPosition {
            book_id: self.book_id,
            chapter_index: self.chapter_index,
            byte_offset: offset,
        };
        if let Err(err) = self.engine.save_position(position) {
            self.status_note = format!("位置保存失败: {err}");
        }
    }

    /// 当前页在章节内的起点。
    pub fn page_start(&self) -> u64 {
        self.page_chain.get(self.page_cursor).copied().unwrap_or(0)
    }

    /// 阅读百分比（依据章节内偏移与全书规模）。
    pub fn progress_percent(&self) -> f32 {
        let total = self.book_total_bytes(self.book_id).max(1) as f32;
        let chapter_offset = self
            .book(self.book_id)
            .map(|b| Library::chapter_byte_offset(b, self.chapter_index))
            .unwrap_or(0);
        let consumed = chapter_offset + self.page_start();
        (consumed as f32 / total * 100.0).clamp(0.0, 100.0)
    }

    // ------------------------------------------------------------------
    // 配置与重排
    // ------------------------------------------------------------------

    /// 当前排版配置。
    pub fn layout_config(&self) -> LayoutConfig {
        self.layouter.config().clone()
    }

    /// 当前排版标识。
    pub fn layout_id(&self) -> u64 {
        self.layouter.layout_id()
    }

    /// 布局失效次数（证据用）。
    pub fn layout_invalidations(&self) -> u64 {
        self.layout_invalidations
    }

    pub fn background_mode(&self) -> i32 {
        self.background_mode
    }

    pub fn theme_dark(&self) -> bool {
        self.theme_dark
    }

    /// 当前左右页边距（逻辑像素）。
    pub fn page_margin(&self) -> i32 {
        self.layouter.config().margin_x.round() as i32
    }

    /// 让排版宽度/高度始终等于「实际正文区域 + 页边距」。
    fn align_to_viewport(&self, mut config: LayoutConfig) -> LayoutConfig {
        if let Some((area_w, area_h)) = self.viewport_area {
            config.width = area_w + 2.0 * config.margin_x;
            config.height = area_h + 2.0 * config.margin_y;
        }
        config
    }

    /// 由 UI 上报真实正文区域尺寸（逻辑像素，不含页边距）；尺寸变化时重排并返回 true。
    ///
    /// 这是「分页看起来没生效」的根因修复：此前排版固定用
    /// `desktop_default()` 的 800×560，与真实渲染区域（约 1032×560）不符，
    /// 导致库按 752px 宽排页、界面按 1032px 渲染，页内容远短于可见高度。
    pub fn set_viewport(&mut self, area_width: f32, area_height: f32) -> bool {
        if area_width < 1.0 || area_height < 1.0 {
            return false;
        }
        if let Some((w, h)) = self.viewport_area {
            if (w - area_width).abs() < 1.0 && (h - area_height).abs() < 1.0 {
                return false;
            }
        }
        self.viewport_area = Some((area_width, area_height));
        let config = self.layouter.config().clone();
        self.apply_layout(config)
    }

    /// 修改一项配置；影响排版的键会触发共享库 `invalidate_layout` 与重排。
    /// 返回是否触发了重排。
    pub fn set_setting(&mut self, key: &str, value: i32) -> bool {
        match key {
            "font-size" => {
                let mut config = self.layouter.config().clone();
                config.font_size = (value as f32).clamp(12.0, 34.0);
                return self.apply_layout(config);
            }
            "line-spacing-x10" => {
                let mut config = self.layouter.config().clone();
                config.line_spacing = (value as f32).clamp(10.0, 24.0) / 10.0;
                return self.apply_layout(config);
            }
            "page-margin" => {
                let mut config = self.layouter.config().clone();
                let margin = (value as f32).clamp(8.0, 64.0);
                config.margin_x = margin;
                config.margin_y = margin;
                return self.apply_layout(config);
            }
            "background-mode" => {
                self.background_mode = value.clamp(0, 2);
                self.status_note = format!("背景模式 = {}", self.background_mode);
            }
            "theme" => {
                self.theme_dark = value != 0;
                self.status_note = if self.theme_dark {
                    "夜间模式".into()
                } else {
                    "日间模式".into()
                };
            }
            other => {
                self.status_note = format!("未知配置项 {other}");
            }
        }
        false
    }

    /// 应用排版配置：失效旧布局缓存并重排（保持原文附近位置）。
    pub fn apply_layout(&mut self, config: LayoutConfig) -> bool {
        // 先让排版尺寸等于「真实正文区域 + 页边距」，再交给共享库排版，
        // 保证页边界与可见区域一致（页边距变更也在此处重新推导）。
        let config = self.align_to_viewport(config);
        let old_id = self.layouter.layout_id();
        let old_start = self.page_start();
        let quote = self.anchor_quote.clone();
        let chapter = self.chapter_index;

        let new_id = self.layouter.set_config(config);
        let removed = self.engine.invalidate_layout(old_id);
        self.layout_invalidations += 1;

        // 用共享库锚点语义恢复「原文附近位置」，再重新起页。
        let mut new_start = 0u64;
        if !quote.is_empty() {
            let anchor = PageAnchor {
                chapter_index: chapter,
                start_byte: old_start,
                end_byte: old_start,
                quote: quote.clone(),
                char_index: 0,
            };
            if let Ok(located) = self.engine.locate_anchor_near(chapter, &anchor, 4096) {
                new_start = located;
            }
        }
        self.page_chain = vec![new_start];
        self.page_cursor = 0;
        self.status_note = format!(
            "排版已更新：layout {} -> {}，失效 {} 条边界，位置 {} 恢复",
            old_id, new_id, removed, new_start
        );
        true
    }

    /// 恢复默认排版。
    pub fn reset_settings(&mut self) -> bool {
        self.background_mode = 0;
        self.theme_dark = false;
        self.apply_layout(LayoutConfig::desktop_default())
    }

    // ------------------------------------------------------------------
    // 自检
    // ------------------------------------------------------------------

    /// 分页自检（正向 / 反向 / 重排 / 锚点），全部走有限窗口。
    pub fn pagination_self_check(&mut self, max_pages: usize) -> SelfCheck {
        let mut result = SelfCheck::empty();
        let chapter_count = self
            .book(self.book_id)
            .map(|b| b.chapter_count)
            .unwrap_or(0);
        let cfg = self.layouter.config().clone();

        let mut layouter = PageLayouter::new(cfg.clone(), WINDOW_BYTES);
        let mut collected: Vec<(u32, u64, u64, String)> = Vec::new();
        let mut forward_ok = true;
        let mut bytes_consumed = 0u64;
        let mut aligned_chapters = 0u32;

        'outer: for chapter in 0..chapter_count {
            let pages = match walk_pages(&mut self.engine, &mut layouter, chapter, 0, max_pages) {
                Ok(p) => p,
                Err(err) => {
                    result.notes.push(format!("章节 {chapter} 排版失败: {err}"));
                    forward_ok = false;
                    break 'outer;
                }
            };
            // 章节起点是按字节均分得到的近似偏移，可能落在字符中间；
            // 首页允许 <= 3 字节的 UTF-8 对齐修正，之后必须严格连续。
            let mut expected: Option<u64> = None;
            for page in pages {
                match expected {
                    None => {
                        if page.start_byte > 3 {
                            forward_ok = false;
                            result.notes.push(format!(
                                "章节 {chapter}: 首页起点 {} 对齐修正超过 3 字节",
                                page.start_byte
                            ));
                        }
                        if page.start_byte > 0 {
                            aligned_chapters += 1;
                        }
                    }
                    Some(want) if page.start_byte != want => {
                        forward_ok = false;
                        result.notes.push(format!(
                            "章节 {chapter}: 页起点 {} != 期望 {want}",
                            page.start_byte
                        ));
                    }
                    Some(_) => {}
                }
                expected = Some(page.end_byte);
                bytes_consumed += page.end_byte - page.start_byte;
                // 独立回读同一区间并比对（有限窗口）
                let len = (page.end_byte - page.start_byte) as usize;
                match self.engine.read_window(chapter, page.start_byte, len) {
                    Ok(window) => {
                        if window.text() != page.text {
                            forward_ok = false;
                            result
                                .notes
                                .push(format!("章节 {chapter}: 页文本与回读不一致"));
                        }
                    }
                    Err(err) => {
                        forward_ok = false;
                        result.notes.push(format!("章节 {chapter} 回读失败: {err}"));
                    }
                }
                collected.push((chapter, page.start_byte, page.end_byte, page.text));
                if collected.len() >= max_pages {
                    result.chapters = chapter + 1;
                    break 'outer;
                }
            }
            result.chapters = chapter + 1;
        }

        result.pages = collected.len();
        result.bytes_consumed = bytes_consumed;
        result.forward_ok = forward_ok && !collected.is_empty();

        // 反向：抽样重排同一页，结果必须一致
        let mut backward_ok = true;
        let sample_step = (collected.len() / 25).max(1);
        for (chapter, start, end, _) in collected.iter().step_by(sample_step) {
            match layouter.render(&mut self.engine, *chapter, *start) {
                Ok(page) => {
                    if page.end_byte != *end {
                        backward_ok = false;
                        result.notes.push(format!("反向重排不一致: {start}..{end}"));
                    }
                }
                Err(err) => {
                    backward_ok = false;
                    result.notes.push(format!("反向重排失败: {err}"));
                }
            }
        }
        // 页链必须连续（正向）
        for pair in collected.windows(2) {
            if pair[0].0 == pair[1].0 && pair[0].2 != pair[1].1 {
                backward_ok = false;
                result.notes.push("页链不连续".to_string());
                break;
            }
        }
        result.backward_ok = backward_ok;

        // 重排：改变排版参数后仍必须连续、且（至少有一页）边界发生变化。
        // 注意：刺激量必须足以改变断行——仅改宽度对「一句一行」的语料无效
        // （行在换行符处断开），因此同时改变字号。
        let mut reflowed = cfg.clone();
        reflowed.width = (reflowed.width - 96.0).max(240.0);
        reflowed.font_size = (reflowed.font_size + 4.0).min(34.0);
        let mut reflow_layouter = PageLayouter::new(reflowed, WINDOW_BYTES);
        let mut reflow_ok = true;
        let mut changed = false;
        for (chapter, start, end, _) in collected.iter().step_by(sample_step.max(1)) {
            match reflow_layouter.render(&mut self.engine, *chapter, *start) {
                Ok(page) => {
                    if page.is_empty() {
                        reflow_ok = false;
                        result.notes.push(format!("重排后出现空页: {start}"));
                    }
                    if page.start_byte != *start {
                        reflow_ok = false;
                        result.notes.push(format!("重排后页首偏移被改动: {start}"));
                    }
                    if page.end_byte != *end {
                        changed = true;
                    }
                }
                Err(err) => {
                    reflow_ok = false;
                    result.notes.push(format!("重排失败: {err}"));
                }
            }
        }
        result.reflow_ok = reflow_ok && changed;

        // 锚点：页首引用必须能在共享库里定位回同一位置
        let mut anchor_ok = true;
        for (chapter, start, _end, text) in collected.iter().step_by(sample_step.max(1)) {
            let quote: String = text.chars().take(ANCHOR_QUOTE_CHARS).collect();
            if quote.is_empty() {
                continue;
            }
            let anchor = PageAnchor {
                chapter_index: *chapter,
                start_byte: *start,
                end_byte: *start,
                quote: quote.clone(),
                char_index: 0,
            };
            match self.engine.locate_anchor_near(*chapter, &anchor, 4096) {
                Ok(located) => {
                    if located != *start {
                        anchor_ok = false;
                        result.notes.push(format!("锚点定位 {located} != {start}"));
                    }
                }
                Err(err) => {
                    anchor_ok = false;
                    result.notes.push(format!("锚点定位失败: {err}"));
                }
            }
        }
        result.anchor_ok = anchor_ok;

        result.status = if result.pages > 0
            && result.forward_ok
            && result.backward_ok
            && result.reflow_ok
            && result.anchor_ok
        {
            "通过"
        } else {
            "不通过"
        };
        result.aligned_chapters = aligned_chapters;
        if result.notes.len() > 12 {
            result.notes.truncate(12);
        }
        self.self_check = Some(result.clone());
        result
    }

    /// 已缓存的自检结果。
    pub fn last_self_check(&self) -> Option<&SelfCheck> {
        self.self_check.as_ref()
    }

    // ------------------------------------------------------------------
    // 状态投影
    // ------------------------------------------------------------------

    /// UI 状态快照。
    pub fn status(&mut self) -> AppStatus {
        let config = self.layouter.config().clone();
        let page = self
            .current_page()
            .unwrap_or_else(|_| RenderedPage::empty(self.chapter_index, 0));
        let book_title = self
            .book(self.book_id)
            .map(|b| b.title.clone())
            .unwrap_or_default();
        let chapter_title = self
            .engine
            .library()
            .chapter(self.book_id, self.chapter_index)
            .map(|c| c.title)
            .unwrap_or_default();
        let metrics = self.engine.metrics();
        let check = self.last_self_check().cloned();
        AppStatus {
            status_text: self.status_note.clone(),
            crumb_text: self.crumb_text(),
            metrics_text: format!(
                "read_calls={} copied_bytes={} materialized_bytes={} page_boundaries={} events={} cancellations={} layout_invalidations={} cache_pages={}",
                metrics.read_calls,
                metrics.copied_bytes,
                metrics.materialized_bytes,
                metrics.page_boundaries,
                metrics.events_emitted,
                metrics.cancelled_requests,
                self.layout_invalidations,
                self.layouter.cache_len()
            ),
            engine_text: format!(
                "core={} abi_version={} abi_entries={} max_window={} window={} layout_id={} 分页自检={}",
                rust_reading_core::CORE_VERSION,
                rust_reading_core::ABI_VERSION,
                rust_reading_core::ABI_ENTRY_COUNT,
                self.engine.max_window(),
                self.layouter.window_bytes(),
                self.layouter.layout_id(),
                check.as_ref().map(|c| format!("{}（{} 页）", c.status, c.pages)).unwrap_or_else(|| "未执行".into())
            ),
            book_title,
            chapter_title,
            page_text: page.text.clone(),
            page_label: format!("第 {} 页 · {} 行", self.page_cursor + 1, page.line_count),
            progress_text: format!("{:.2}% · 章内偏移 {}", self.progress_percent(), self.page_start()),
            font_size: config.font_size.round() as i32,
            line_spacing_x10: (config.line_spacing * 10.0).round() as i32,
            page_margin: config.margin_x.round() as i32,
            background_mode: self.background_mode,
            theme_dark: self.theme_dark,
        }
    }

    /// 引擎快照 JSON。
    pub fn engine_snapshot_json(&self) -> String {
        self.engine.snapshot_json()
    }
}

impl Default for ReaderApp {
    fn default() -> Self {
        Self::new()
    }
}

fn book_row(book: &Book) -> BookRow {
    BookRow {
        id: book.id.to_string(),
        title: book.title.clone(),
        author: book.author.clone(),
        meta: format!("{} 章 / {} KB", book.chapter_count, book.total_bytes / 1024),
    }
}

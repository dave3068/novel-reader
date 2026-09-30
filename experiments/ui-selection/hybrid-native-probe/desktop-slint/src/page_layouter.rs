//! 桌面端分页适配层。
//!
//! 只使用共享库的**直接 Rust API**（H-R1）：`read_window` 提供有限窗口，
//! `layout_page` 提供页末合法 UTF-8 边界。本层不实现任何阅读业务，
//! 只负责「窗口不足时放大重试」与结果缓存。

use std::collections::HashMap;

use rust_reading_core::{
    layout_id, layout_page, LayoutConfig, NrResult, PageLayout, ReadingEngine,
};

/// 一页的渲染结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedPage {
    pub chapter_index: u32,
    /// 章节内起始字节偏移（UTF-8 合法边界）。
    pub start_byte: u64,
    /// 章节内结束字节偏移（UTF-8 合法边界）。
    pub end_byte: u64,
    pub text: String,
    pub line_count: u32,
    pub layout_time_us: u64,
    /// 本章是否已到末尾。
    pub chapter_eof: bool,
}

impl RenderedPage {
    /// 空页（章节末尾）。
    pub fn empty(chapter_index: u32, offset: u64) -> Self {
        Self {
            chapter_index,
            start_byte: offset,
            end_byte: offset,
            text: String::new(),
            line_count: 0,
            layout_time_us: 0,
            chapter_eof: true,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// 本章内的页序号（从 0 起，"第 N 页"显示用）。
    pub fn display_index(&self) -> u32 {
        0
    }
}

/// 分页器：持有排版配置与其标识，缓存已渲染页。
#[derive(Debug, Clone)]
pub struct PageLayouter {
    config: LayoutConfig,
    id: u64,
    window_bytes: usize,
    cache: HashMap<(u32, u64), RenderedPage>,
}

impl PageLayouter {
    /// `window_bytes` 为初始窗口大小（会被 `max_window` 上限约束）。
    pub fn new(config: LayoutConfig, window_bytes: usize) -> Self {
        let id = layout_id(&config);
        Self {
            config,
            id,
            window_bytes: window_bytes.max(4096),
            cache: HashMap::new(),
        }
    }

    pub fn config(&self) -> &LayoutConfig {
        &self.config
    }

    pub fn layout_id(&self) -> u64 {
        self.id
    }

    pub fn window_bytes(&self) -> usize {
        self.window_bytes
    }

    /// 更新排版配置，返回新的排版标识，并清空缓存
    /// （对应共享库的 `invalidate_layout` 语义）。
    pub fn set_config(&mut self, config: LayoutConfig) -> u64 {
        self.config = config;
        self.id = layout_id(&self.config);
        self.cache.clear();
        self.id
    }

    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }

    /// 排版从 `offset` 开始的一页（有限窗口读取 + 公共分页 API）。
    ///
    /// 若窗口被整页填满且尚未到章节末尾，则按倍数放大窗口重试，
    /// 直到 `engine.max_window()` 上限。
    pub fn render(
        &mut self,
        engine: &mut ReadingEngine,
        chapter_index: u32,
        offset: u64,
    ) -> NrResult<RenderedPage> {
        if let Some(cached) = self.cache.get(&(chapter_index, offset)) {
            return Ok(cached.clone());
        }

        let max_window = engine.max_window();
        let mut window_bytes = self.window_bytes.min(max_window).max(4096);

        let page = loop {
            let window = engine.read_window(chapter_index, offset, window_bytes)?;
            if window.bytes.is_empty() {
                break RenderedPage::empty(chapter_index, window.chapter_offset);
            }
            let text = window.text();
            let layout: PageLayout = layout_page(text, 0, &self.config);
            if layout.consumed_utf8_bytes == 0 {
                break RenderedPage::empty(chapter_index, window.chapter_offset);
            }
            let filled = layout.consumed_utf8_bytes >= text.len();
            if filled && !window.chapter_eof && window_bytes < max_window {
                window_bytes = (window_bytes * 2).min(max_window);
                continue;
            }
            let start = window.chapter_offset;
            break RenderedPage {
                chapter_index,
                start_byte: start,
                end_byte: start + layout.consumed_utf8_bytes as u64,
                text: text[..layout.consumed_utf8_bytes].to_string(),
                line_count: layout.visible_line_count,
                layout_time_us: layout.layout_time_us,
                chapter_eof: window.chapter_eof,
            };
        };

        self.cache.insert((chapter_index, offset), page.clone());
        Ok(page)
    }
}

/// 从 `start_offset` 起连续向后排版最多 `max_pages` 页。
///
/// 用于正向分页检查与结果证据；不修改分页器缓存之外的任何状态。
pub fn walk_pages(
    engine: &mut ReadingEngine,
    layouter: &mut PageLayouter,
    chapter_index: u32,
    start_offset: u64,
    max_pages: usize,
) -> NrResult<Vec<RenderedPage>> {
    let mut out = Vec::new();
    let mut offset = start_offset;
    for _ in 0..max_pages {
        let page = layouter.render(engine, chapter_index, offset)?;
        if page.is_empty() {
            break;
        }
        offset = page.end_byte;
        out.push(page);
    }
    Ok(out)
}

//! 页面缓存：当前页 + 前一页 + 后三页预读，容量**有明确上限**（DT3 / DR3）。
//!
//! 依据 `docs/novel-reader-v1-development-plan.md` §8.4：
//!
//! > 当前页 + 前一页边界 + 后三页预读作为初始缓存策略；缓存有明确上限，
//! > 不能因多个协程并发而重复填充；下一页优先使用已测量缓存；
//! > 上一页优先使用边界栈或持久化稀疏锚点，不能永久保存全书页面。
//!
//! 本模块只做「有界缓存 + 淘汰」，不含任何阅读语义：
//! 页面文本来自 [`crate::pagination`] 的测量结果，边界语义由冻结核心负责。
//! 缓存键是页起点字节（同一排版世代内唯一）；排版世代（`layout_id`）变化时整批丢弃，
//! 因此旧世代的页面**不可能**被翻页路径读到（旧结果不得覆盖新页面）。

use std::collections::BTreeMap;

use crate::pagination::PagedPage;

/// 反向保留的页数（前一页）。
pub const PREFETCH_BACK: usize = 1;
/// 正向预读的页数（后三页）。
pub const PREFETCH_FORWARD: usize = 3;
/// 最小容量：当前页 + 前一页 + 后三页。
pub const MIN_CACHE_CAPACITY: usize = PREFETCH_BACK + 1 + PREFETCH_FORWARD;
/// 默认容量（在最小窗口之外留出余量，避免一次预读就把缓存挤满）。
pub const DEFAULT_CACHE_CAPACITY: usize = 8;
/// 容量硬上限：内存有界，绝不退化成「全书页面缓存」。
pub const MAX_CACHE_CAPACITY: usize = 64;
/// 单次预读最多测量的页数（防止一次预读无限扩张）。
pub const MAX_PREFETCH_BATCH: usize = PREFETCH_FORWARD;

/// 缓存计数（自检与证据用）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    /// 命中次数。
    pub hits: u64,
    /// 未命中次数。
    pub misses: u64,
    /// 写入次数。
    pub inserts: u64,
    /// 淘汰次数。
    pub evictions: u64,
    /// 因排版世代不符被丢弃的页面数。
    pub stale_layout_drops: u64,
    /// 当前容量上限。
    pub capacity: usize,
    /// 当前页数。
    pub len: usize,
}

/// 有界页面缓存。
#[derive(Debug, Clone)]
pub struct PageCache {
    layout_id: u64,
    capacity: usize,
    entries: BTreeMap<u64, PagedPage>,
    center: Option<u64>,
    stats: CacheStats,
}

impl PageCache {
    /// 建立缓存；容量被夹紧到 `[MIN_CACHE_CAPACITY, MAX_CACHE_CAPACITY]`。
    pub fn new(layout_id: u64, capacity: usize) -> Self {
        let capacity = capacity.clamp(MIN_CACHE_CAPACITY, MAX_CACHE_CAPACITY);
        Self {
            layout_id,
            capacity,
            entries: BTreeMap::new(),
            center: None,
            stats: CacheStats {
                capacity,
                ..CacheStats::default()
            },
        }
    }

    /// 当前排版世代。
    pub fn layout_id(&self) -> u64 {
        self.layout_id
    }

    /// 容量上限。
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// 当前页数。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 是否已缓存该起点的页。
    pub fn contains(&self, start_byte: u64) -> bool {
        self.entries.contains_key(&start_byte)
    }

    /// 读取并计数命中/未命中。
    pub fn get(&mut self, start_byte: u64) -> Option<PagedPage> {
        match self.entries.get(&start_byte) {
            Some(page) => {
                self.stats.hits += 1;
                self.center = Some(start_byte);
                Some(page.clone())
            }
            None => {
                self.stats.misses += 1;
                None
            }
        }
    }

    /// 只读访问（不计入统计；给纯查询/测试用）。
    pub fn peek(&self, start_byte: u64) -> Option<&PagedPage> {
        self.entries.get(&start_byte)
    }

    /// 已缓存的页起点（升序）。
    pub fn starts(&self) -> Vec<u64> {
        self.entries.keys().copied().collect()
    }

    /// 统计快照。
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            len: self.entries.len(),
            capacity: self.capacity,
            ..self.stats
        }
    }

    /// 写入一页；世代不符时丢弃并返回 `false`（旧世代页面不得进入缓存）。
    ///
    /// 写入后若超过容量，按「离当前页最远者优先」淘汰，保证容量恒 ≤ `capacity`。
    pub fn insert(&mut self, page: PagedPage) -> bool {
        if page.layout_id != self.layout_id {
            self.stats.stale_layout_drops += 1;
            return false;
        }
        let start = page.start_byte;
        self.entries.insert(start, page);
        self.center = Some(start);
        self.stats.inserts += 1;
        self.evict_to_capacity();
        true
    }

    /// 切换排版世代：清空全部页面并返回被丢弃的页数（旧世代页面一律失效）。
    pub fn set_layout(&mut self, layout_id: u64) -> usize {
        let removed = self.entries.len();
        self.entries.clear();
        self.center = None;
        self.layout_id = layout_id;
        removed
    }

    /// 只保留 `[center 往前 back 页, center 往后 forward 页]` 内的页面，返回被淘汰的页数。
    ///
    /// 「页」按已缓存页起点的升序位置计数（不是字节距离）：预读窗口是页数语义。
    /// 中心页不在缓存时不做任何裁剪（返回 0）。
    pub fn retain_window(&mut self, center: u64, back: usize, forward: usize) -> usize {
        let keys: Vec<u64> = self.entries.keys().copied().collect();
        let Some(position) = keys.iter().position(|start| *start == center) else {
            return 0;
        };
        let low = position.saturating_sub(back);
        let high = position.saturating_add(forward).min(keys.len() - 1);
        let keep: Vec<u64> = keys[low..=high].to_vec();
        let before = self.entries.len();
        self.entries.retain(|start, _| keep.contains(start));
        let removed = before - self.entries.len();
        self.stats.evictions += removed as u64;
        removed
    }

    /// 淘汰到容量以内：优先丢弃离 `center` 最远的页面（同距离时丢弃靠前的）。
    fn evict_to_capacity(&mut self) {
        while self.entries.len() > self.capacity {
            let center = self.center.unwrap_or_default();
            let victim = self
                .entries
                .keys()
                .copied()
                .max_by_key(|start| (start.abs_diff(center), std::cmp::Reverse(*start)));
            let Some(victim) = victim else { break };
            if self.entries.remove(&victim).is_some() {
                self.stats.evictions += 1;
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(layout_id: u64, start: u64, end: u64) -> PagedPage {
        PagedPage::new(
            layout_id,
            1,
            start,
            end,
            vec![format!("page@{start}")],
            false,
            None,
        )
    }

    #[test]
    fn page_cache_is_bounded_and_keeps_current_window() {
        let mut cache = PageCache::new(1, DEFAULT_CACHE_CAPACITY);
        for page_index in 0..40u64 {
            assert!(cache.insert(page(1, page_index * 100, page_index * 100 + 100)));
        }
        assert!(
            cache.len() <= cache.capacity(),
            "缓存必须有明确上限：len={} capacity={}",
            cache.len(),
            cache.capacity()
        );
        assert_eq!(cache.capacity(), DEFAULT_CACHE_CAPACITY);

        // 当前页窗口（前一页 + 后三页）内的页面必须保留。
        let center = 39 * 100;
        cache.retain_window(center, PREFETCH_BACK, PREFETCH_FORWARD);
        assert!(cache.contains(center));
        assert!(cache.contains(center - 100), "前一页必须在窗口内");
        assert!(!cache.contains(center - 200), "超出前一页的旧页应被淘汰");
        assert!(cache.len() <= MIN_CACHE_CAPACITY);
        assert!(cache.len() <= cache.capacity());

        // 容量硬上限：把容量请求调到超大值也会被夹紧。
        let huge = PageCache::new(1, usize::MAX);
        assert_eq!(huge.capacity(), MAX_CACHE_CAPACITY);
        let tiny = PageCache::new(1, 0);
        assert_eq!(tiny.capacity(), MIN_CACHE_CAPACITY);
    }

    #[test]
    fn page_cache_drops_pages_from_other_layout_generations() {
        let mut cache = PageCache::new(7, DEFAULT_CACHE_CAPACITY);
        assert!(cache.insert(page(7, 0, 100)));
        assert!(cache.contains(0));
        // 旧世代页面写入必须被丢弃（旧结果不得覆盖当前页面）。
        assert!(!cache.insert(page(6, 0, 120)));
        assert_eq!(cache.stats().stale_layout_drops, 1);
        assert_eq!(cache.peek(0).map(|p| p.end_byte), Some(100));

        // 切换世代：整批清空。
        let removed = cache.set_layout(8);
        assert_eq!(removed, 1);
        assert!(cache.is_empty());
        assert_eq!(cache.layout_id(), 8);
        assert!(cache.insert(page(8, 100, 200)));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn page_cache_hit_miss_accounting_is_exact() {
        let mut cache = PageCache::new(1, MIN_CACHE_CAPACITY);
        assert!(cache.get(0).is_none());
        assert!(cache.get(0).is_none());
        assert!(cache.insert(page(1, 0, 100)));
        assert_eq!(cache.get(0).map(|p| p.end_byte), Some(100));
        assert!(cache.get(100).is_none());
        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 3);
        assert_eq!(stats.inserts, 1);
        assert_eq!(stats.len, 1);
        assert_eq!(stats.stale_layout_drops, 0);
    }
}

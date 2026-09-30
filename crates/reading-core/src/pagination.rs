//! 分页协调：`commit_page_boundary` / 查询 / `invalidate(signature)` / 过期结果丢弃。
//!
//! 依据 `reading-core/spec.md` CR4、`reading-core/design.md` §位置规则 与
//! `results/gates/gate-0-baseline.md` §2.4/§2.5：
//!
//! - 页码**不是**进度身份；页面边界只属于某个布局签名（[`LayoutKey`] = 签名 + 内容索引版本）；
//! - 平台侧测量出的 `end_byte` 必须回来做边界校验（`reading-index::boundary`），
//!   非法边界一律拒绝，绝不以「百分比」冒充准确页码；
//! - 每次排版带 `layout_id`，每个请求带 `request_id`；签名变化后旧世代与在途结果全部丢弃。
//!
//! # 生命周期（一次翻页 / 一次重排）
//!
//! ```text
//! begin_request()                       → request_id
//! 平台测量（bounded read + 文本测量）    → end_byte
//! commit_page_boundary(source, request, encoding) → Committed / Rejected / Stale*
//! query_page_boundary(layout_id, start) → 已登记边界（正反向翻页共用）
//! ```
//!
//! 排版设置变化时调用 [`PaginationCache::set_layout`]：缓存键变化则清空边界并递增
//! `layout_id`，此后任何携带旧 `layout_id` 的结果都会被 [`PaginationCache::commit_page_boundary`]
//! 判为过期并丢弃（`design.md` §重排：「旧重排结果不得覆盖当前页面」）。

use reading_index::boundary::{
    validate_page_boundary, BoundaryError, BoundaryRejection, PageBoundary, PageBoundaryChain,
};
use reading_source::TextSource;

use crate::layout::{LayoutKey, LayoutSignature};
use reading_source::TextEncoding;

/// 分页协调版本（不兼容变更必须递增，并同步 FFI/契约测试）。
pub const PAGINATION_VERSION: u32 = 1;

/// `commit_page_boundary` 的结果状态。
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryCommitStatus {
    /// 已登记新的页面边界。
    Committed = 0,
    /// 同一签名、同一起点、同一终点重复提交：结果一致，幂等接受。
    DuplicateIdentical = 1,
    /// 边界非法（见 [`BoundaryCommitOutcome::rejection`]）。
    Rejected = 2,
    /// `layout_id` 不是当前世代（排版设置已变化）。
    StaleLayoutId = 3,
    /// `request_id` 已被更新的请求取代（重排请求合并）。
    StaleRequestId = 4,
    /// `content_index_version` 与缓存键不一致（内容索引已重建）。
    StaleContentIndexVersion = 5,
}

impl BoundaryCommitStatus {
    /// 跨 ABI 错误码（`0` = 成功；过期映射到 `Cancelled(7)` / `LayoutNotFound(8)`）。
    pub const fn code(self) -> i32 {
        match self {
            BoundaryCommitStatus::Committed | BoundaryCommitStatus::DuplicateIdentical => 0,
            BoundaryCommitStatus::Rejected => 1,
            BoundaryCommitStatus::StaleLayoutId
            | BoundaryCommitStatus::StaleContentIndexVersion => 8,
            BoundaryCommitStatus::StaleRequestId => 7,
        }
    }

    /// 是否已（幂等）接受。
    pub const fn is_committed(self) -> bool {
        matches!(
            self,
            BoundaryCommitStatus::Committed | BoundaryCommitStatus::DuplicateIdentical
        )
    }

    /// 是否因为过期而被丢弃。
    pub const fn is_stale(self) -> bool {
        matches!(
            self,
            BoundaryCommitStatus::StaleLayoutId
                | BoundaryCommitStatus::StaleRequestId
                | BoundaryCommitStatus::StaleContentIndexVersion
        )
    }

    /// 稳定短名（日志用，不得作为业务分支依据）。
    pub const fn as_str(self) -> &'static str {
        match self {
            BoundaryCommitStatus::Committed => "Committed",
            BoundaryCommitStatus::DuplicateIdentical => "DuplicateIdentical",
            BoundaryCommitStatus::Rejected => "Rejected",
            BoundaryCommitStatus::StaleLayoutId => "StaleLayoutId",
            BoundaryCommitStatus::StaleRequestId => "StaleRequestId",
            BoundaryCommitStatus::StaleContentIndexVersion => "StaleContentIndexVersion",
        }
    }
}

/// 一次页边界提交请求（跨 C ABI/JNI 的字段子集：全部为整数，无浮点、无指针）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageBoundaryRequest {
    /// 本次排版世代（由 [`PaginationCache::layout_id`] 给出）。
    pub layout_id: u64,
    /// 本次请求标识（由 [`PaginationCache::begin_request`] 分配）。
    pub request_id: u64,
    /// 内容索引版本（必须与缓存键一致）。
    pub content_index_version: u32,
    /// 内容总字节数（用于范围校验；必须等于数据源长度）。
    pub total_bytes: u64,
    /// 页面起始字节。
    pub start_byte: u64,
    /// 平台测量出的页面结束字节（必须位于合法 UTF-8 字符边界）。
    pub end_byte: u64,
}

/// 已登记的一条页面边界（携带其所属签名与世代，便于审计与失效）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageBoundaryRecord {
    /// 排版世代。
    pub layout_id: u64,
    /// 登记该边界的请求标识。
    pub request_id: u64,
    /// 排版签名（同一世代内恒定）。
    pub signature: LayoutSignature,
    /// 内容索引版本。
    pub content_index_version: u32,
    /// 页面边界本身。
    pub boundary: PageBoundary,
}

/// 提交结果（含拒绝原因与当前世代，便于调用方无歧义分支）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundaryCommitOutcome {
    /// 结果状态。
    pub status: BoundaryCommitStatus,
    /// 已登记（或命中缓存）的边界。
    pub boundary: Option<PageBoundary>,
    /// 被拒时的稳定原因。
    pub rejection: Option<BoundaryRejection>,
    /// 跨 ABI 错误码（成功为 `0`）。
    pub error_code: i32,
    /// 当前排版世代（调用方据此发现自己的结果已过期）。
    pub current_layout_id: u64,
    /// 被丢弃的 `layout_id`（仅过期类状态有值）。
    pub stale_layout_id: Option<u64>,
    /// 被丢弃的 `request_id`（仅 [`BoundaryCommitStatus::StaleRequestId`] 有值）。
    pub stale_request_id: Option<u64>,
    /// 内容总字节数（回显，便于日志与端到端核对）。
    pub total_bytes: u64,
}

impl BoundaryCommitOutcome {
    /// 是否已（幂等）接受。
    pub fn is_committed(&self) -> bool {
        self.status.is_committed()
    }

    /// 是否因为过期被丢弃。
    pub fn is_stale(&self) -> bool {
        self.status.is_stale()
    }

    /// 是否因为边界非法被拒绝。
    pub fn is_rejected(&self) -> bool {
        self.status == BoundaryCommitStatus::Rejected
    }
}

/// 一次排版变更的结果（签名或内容索引版本变化 → 旧边界全部失效）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutChange {
    /// 缓存键是否真的发生了变化。
    pub changed: bool,
    /// 因变化而被清空的边界数量。
    pub removed: usize,
    /// 变更前的排版世代。
    pub previous_layout_id: u64,
    /// 变更后的排版世代。
    pub layout_id: u64,
}

/// `invalidate(signature)` 的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutInvalidation {
    /// 被删除的边界数量。
    pub removed: usize,
    /// 失效后的排版世代。
    pub layout_id: u64,
    /// 是否递增了排版世代（签名正是当前签名时）。
    pub generation_bumped: bool,
    /// 被作废的排版世代（未递增时为 `None`）。
    pub invalidated_layout_id: Option<u64>,
}

/// 分页缓存：以 `layout_signature + content_index_version` 为键管理页面边界。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaginationCache {
    key: LayoutKey,
    layout_id: u64,
    next_request_id: u64,
    latest_request_id: u64,
    records: Vec<PageBoundaryRecord>,
}

impl PaginationCache {
    /// 建立分页缓存（首个排版世代为 `1`）。
    ///
    /// 签名非法（零视口、空字体族等）时报 [`crate::ErrorCode::InvalidArgument`]。
    pub fn new(signature: LayoutSignature, content_index_version: u32) -> crate::NrResult<Self> {
        signature.validate()?;
        Ok(Self {
            key: LayoutKey::new(signature, content_index_version),
            layout_id: 1,
            next_request_id: 0,
            latest_request_id: 0,
            records: Vec::new(),
        })
    }

    /// 当前缓存键（签名 + 内容索引版本）。
    pub fn layout_key(&self) -> &LayoutKey {
        &self.key
    }

    /// 当前排版签名。
    pub fn signature(&self) -> &LayoutSignature {
        &self.key.signature
    }

    /// 当前排版世代（所有结果都必须携带该值，否则被丢弃）。
    pub fn layout_id(&self) -> u64 {
        self.layout_id
    }

    /// 最近一次分配的请求标识。
    pub fn latest_request_id(&self) -> u64 {
        self.latest_request_id
    }

    /// 已登记的页面边界（登记顺序）。
    pub fn records(&self) -> &[PageBoundaryRecord] {
        &self.records
    }

    /// 已登记的边界数量。
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// 是否没有任何边界。
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// 分配一个新的请求标识（单调递增）。
    ///
    /// 一旦分配了更新的标识，此前尚未提交的结果即成为过期结果（重排请求合并：
    /// 只有最新一次排版的结果允许覆盖页面）。
    pub fn begin_request(&mut self) -> u64 {
        self.next_request_id += 1;
        self.latest_request_id = self.next_request_id;
        self.next_request_id
    }

    /// 提交一次页边界测量结果。
    ///
    /// 校验顺序（先便宜的世代/标识判定，再做需要读数据源的边界校验）：
    /// 1. `layout_id` 是否仍是当前世代（否则 [`BoundaryCommitStatus::StaleLayoutId`]）；
    /// 2. `content_index_version` 是否与缓存键一致（否则 [`BoundaryCommitStatus::StaleContentIndexVersion`]）；
    /// 3. `request_id` 是否已被更新的请求取代（否则 [`BoundaryCommitStatus::StaleRequestId`]）；
    /// 4. 边界是否合法（`end_byte > start_byte`、`end_byte` 位于合法 UTF-8 字符边界、
    ///    不超出内容长度）——非法即 [`BoundaryCommitStatus::Rejected`]，**不写入任何状态**；
    /// 5. 同一签名同一起点是否已有登记（同终点幂等接受，异终点判为不一致）。
    pub fn commit_page_boundary<S: TextSource + ?Sized>(
        &mut self,
        source: &S,
        request: &PageBoundaryRequest,
        encoding: TextEncoding,
    ) -> BoundaryCommitOutcome {
        let boundary = PageBoundary::new(request.start_byte, request.end_byte);
        let stale = |status: BoundaryCommitStatus| BoundaryCommitOutcome {
            status,
            boundary: None,
            rejection: None,
            error_code: status.code(),
            current_layout_id: self.layout_id,
            stale_layout_id: matches!(
                status,
                BoundaryCommitStatus::StaleLayoutId
                    | BoundaryCommitStatus::StaleContentIndexVersion
            )
            .then_some(request.layout_id),
            stale_request_id: (status == BoundaryCommitStatus::StaleRequestId)
                .then_some(request.request_id),
            total_bytes: request.total_bytes,
        };

        if request.layout_id != self.layout_id {
            return stale(BoundaryCommitStatus::StaleLayoutId);
        }
        if request.content_index_version != self.key.content_index_version {
            return stale(BoundaryCommitStatus::StaleContentIndexVersion);
        }
        if request.request_id < self.latest_request_id {
            return stale(BoundaryCommitStatus::StaleRequestId);
        }

        if let Err(error) = validate_page_boundary(source, &boundary, request.total_bytes, encoding) {
            return BoundaryCommitOutcome {
                status: BoundaryCommitStatus::Rejected,
                boundary: None,
                rejection: Some(error.rejection),
                error_code: error.code(),
                current_layout_id: self.layout_id,
                stale_layout_id: None,
                stale_request_id: None,
                total_bytes: request.total_bytes,
            };
        }

        if let Some(existing) = self
            .records
            .iter()
            .find(|record| record.boundary.start_byte == request.start_byte)
        {
            if existing.boundary.end_byte == request.end_byte {
                return BoundaryCommitOutcome {
                    status: BoundaryCommitStatus::DuplicateIdentical,
                    boundary: Some(existing.boundary),
                    rejection: None,
                    error_code: 0,
                    current_layout_id: self.layout_id,
                    stale_layout_id: None,
                    stale_request_id: None,
                    total_bytes: request.total_bytes,
                };
            }
            let conflict = BoundaryError::new(
                BoundaryRejection::ConflictWithCached,
                format!(
                    "起点({}) 已登记终点({})，本次终点({}) 不一致",
                    request.start_byte, existing.boundary.end_byte, request.end_byte
                ),
            );
            return BoundaryCommitOutcome {
                status: BoundaryCommitStatus::Rejected,
                boundary: None,
                rejection: Some(conflict.rejection),
                error_code: conflict.code(),
                current_layout_id: self.layout_id,
                stale_layout_id: None,
                stale_request_id: None,
                total_bytes: request.total_bytes,
            };
        }

        self.records.push(PageBoundaryRecord {
            layout_id: self.layout_id,
            request_id: request.request_id,
            signature: self.key.signature.clone(),
            content_index_version: self.key.content_index_version,
            boundary,
        });
        BoundaryCommitOutcome {
            status: BoundaryCommitStatus::Committed,
            boundary: Some(boundary),
            rejection: None,
            error_code: 0,
            current_layout_id: self.layout_id,
            stale_layout_id: None,
            stale_request_id: None,
            total_bytes: request.total_bytes,
        }
    }

    /// 查询某个世代下以 `start_byte` 起始的页面边界。
    ///
    /// 世代不匹配一律返回 `None`：旧世代的边界不可能再被翻页路径读到。
    pub fn query_page_boundary(&self, layout_id: u64, start_byte: u64) -> Option<PageBoundary> {
        self.query_record(layout_id, start_byte)
            .map(|record| record.boundary)
    }

    /// 查询某个世代下以 `start_byte` 起始的完整记录。
    pub fn query_record(&self, layout_id: u64, start_byte: u64) -> Option<&PageBoundaryRecord> {
        if layout_id != self.layout_id {
            return None;
        }
        self.records
            .iter()
            .find(|record| record.boundary.start_byte == start_byte)
    }

    /// 按起点升序的全部边界（正反向翻页与连续性断言用）。
    pub fn ordered_boundaries(&self) -> Vec<PageBoundary> {
        let mut boundaries: Vec<PageBoundary> =
            self.records.iter().map(|record| record.boundary).collect();
        boundaries.sort_unstable_by_key(|boundary| boundary.start_byte);
        boundaries
    }

    /// 由已登记边界构建连续性链；存在空隙或重叠时返回错误（丢字/重字会被当场发现）。
    pub fn page_chain(&self) -> Result<PageBoundaryChain, BoundaryError> {
        let ordered = self.ordered_boundaries();
        let origin = ordered.first().map_or(0, |boundary| boundary.start_byte);
        let mut chain = PageBoundaryChain::new(origin);
        for boundary in ordered {
            chain.push(boundary)?;
        }
        Ok(chain)
    }

    /// 偏移落在第几页（精确序号；不是按字节比例估算的百分比）。
    pub fn page_index_for_offset(&self, offset: u64) -> Option<usize> {
        self.ordered_boundaries()
            .iter()
            .position(|boundary| boundary.contains(offset))
    }

    /// 按签名失效：删除属于该签名的全部边界。
    ///
    /// 若该签名正是当前签名，还会递增排版世代——在途的旧结果随之过期，
    /// 因此「签名变更后旧边界全部失效」与「过期结果被丢弃」两件事由同一个动作保证。
    pub fn invalidate(&mut self, signature: &LayoutSignature) -> LayoutInvalidation {
        let before = self.records.len();
        self.records.retain(|record| &record.signature != signature);
        let removed = before - self.records.len();
        let matches_current = signature == &self.key.signature;
        let previous_layout_id = self.layout_id;
        if matches_current {
            self.layout_id += 1;
        }
        LayoutInvalidation {
            removed,
            layout_id: self.layout_id,
            generation_bumped: matches_current,
            invalidated_layout_id: matches_current.then_some(previous_layout_id),
        }
    }

    /// 清空全部边界并递增世代（重排/索引重建后的强制失效）。
    pub fn invalidate_all(&mut self) -> LayoutInvalidation {
        let removed = self.records.len();
        self.records.clear();
        let previous_layout_id = self.layout_id;
        self.layout_id += 1;
        LayoutInvalidation {
            removed,
            layout_id: self.layout_id,
            generation_bumped: true,
            invalidated_layout_id: Some(previous_layout_id),
        }
    }

    /// 设置新的排版参数（字号、视口或字体变化时调用）。
    ///
    /// 缓存键不变时什么都不做（幂等）：**不得**因为一次无变化的通知就清空分页缓存。
    pub fn set_layout(
        &mut self,
        signature: LayoutSignature,
        content_index_version: u32,
    ) -> crate::NrResult<LayoutChange> {
        signature.validate()?;
        let next = LayoutKey::new(signature, content_index_version);
        let previous_layout_id = self.layout_id;
        if next == self.key {
            return Ok(LayoutChange {
                changed: false,
                removed: 0,
                previous_layout_id,
                layout_id: previous_layout_id,
            });
        }
        let removed = self.records.len();
        self.records.clear();
        self.key = next;
        self.layout_id += 1;
        Ok(LayoutChange {
            changed: true,
            removed,
            previous_layout_id,
            layout_id: self.layout_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use reading_source::FileTextSource;

    use super::*;
    use crate::ErrorCode;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempGuard {
        dir: PathBuf,
    }

    impl Drop for TempGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn temp_source(label: &str, content: &[u8]) -> (TempGuard, FileTextSource) {
        let dir = std::env::temp_dir().join(format!(
            "nr-rc5-pagination-{}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
            label
        ));
        std::fs::create_dir_all(&dir).expect("创建临时目录");
        let path = dir.join("book.txt");
        std::fs::write(&path, content).expect("写入临时文件");
        let source = FileTextSource::open(&path).expect("打开临时文件");
        (TempGuard { dir }, source)
    }

    fn request(
        cache: &PaginationCache,
        request_id: u64,
        start: u64,
        end: u64,
        total: u64,
    ) -> PageBoundaryRequest {
        PageBoundaryRequest {
            layout_id: cache.layout_id(),
            request_id,
            content_index_version: cache.layout_key().content_index_version,
            total_bytes: total,
            start_byte: start,
            end_byte: end,
        }
    }

    #[test]
    fn pagination_cache_commits_and_queries_exact_boundaries() {
        let text = "Chapter 1 test\ncontent line here.\n".repeat(20);
        let (_guard, source) = temp_source("commit", text.as_bytes());
        let total = source.len();
        let mut cache = PaginationCache::new(LayoutSignature::default(), 1).expect("建立缓存");

        let end = 40u64;
        let request_id = cache.begin_request();
        let outcome =
            cache.commit_page_boundary(&source, &request(&cache, request_id, 0, end, total), TextEncoding::Utf8);
        assert_eq!(outcome.status, BoundaryCommitStatus::Committed);
        assert_eq!(outcome.error_code, 0);
        assert!(outcome.is_committed());
        assert_eq!(cache.len(), 1);
        assert_eq!(
            cache.query_page_boundary(cache.layout_id(), 0),
            Some(PageBoundary::new(0, end))
        );
        assert_eq!(cache.query_page_boundary(cache.layout_id(), 1), None);
        assert_eq!(
            cache.query_page_boundary(cache.layout_id() + 1, 0),
            None,
            "旧世代边界不得再被读到"
        );
    }

    #[test]
    fn pagination_same_signature_same_start_is_deterministic() {
        let text = "Chapter 1 test\ncontent line.\n".repeat(20);
        let (_guard, source) = temp_source("deterministic", text.as_bytes());
        let total = source.len();
        let mut cache = PaginationCache::new(LayoutSignature::default(), 1).expect("建立缓存");

        let first = cache.begin_request();
        let committed = cache.commit_page_boundary(&source, &request(&cache, first, 0, 30, total), TextEncoding::Utf8);
        assert_eq!(committed.status, BoundaryCommitStatus::Committed);

        // 同签名同起点同终点 → 幂等接受，返回同一份边界。
        let second = cache.begin_request();
        let again = cache.commit_page_boundary(&source, &request(&cache, second, 0, 30, total), TextEncoding::Utf8);
        assert_eq!(again.status, BoundaryCommitStatus::DuplicateIdentical);
        assert_eq!(again.boundary, Some(PageBoundary::new(0, 30)));
        assert_eq!(cache.len(), 1, "幂等提交不得新增记录");

        // 同起点不同终点 → 缓存结果不一致，必须拒绝而不是覆盖。
        let third = cache.begin_request();
        let conflict = cache.commit_page_boundary(&source, &request(&cache, third, 0, 60, total), TextEncoding::Utf8);
        assert!(conflict.is_rejected());
        assert_eq!(
            conflict.rejection,
            Some(BoundaryRejection::ConflictWithCached)
        );
        assert_eq!(conflict.error_code, 1);
        assert_eq!(
            cache.query_page_boundary(cache.layout_id(), 0),
            Some(PageBoundary::new(0, 30)),
            "冲突结果不得覆盖既有边界"
        );
    }

    #[test]
    fn pagination_rejects_illegal_boundaries_without_side_effects() {
        // "中文正文" 的每个字符 3 字节：偏移 1、2 落在字符内部。
        let text = "中文正文内容。".repeat(30);
        let (_guard, source) = temp_source("illegal", text.as_bytes());
        let total = source.len();
        let mut cache = PaginationCache::new(LayoutSignature::default(), 1).expect("建立缓存");

        let mid_char = cache.begin_request();
        let illegal = cache.commit_page_boundary(&source, &request(&cache, mid_char, 0, 1, total), TextEncoding::Utf8);
        assert!(illegal.is_rejected());
        assert_eq!(
            illegal.rejection,
            Some(BoundaryRejection::IllegalCharBoundary)
        );
        assert_eq!(illegal.error_code, 3);

        let empty = cache.begin_request();
        let zero = cache.commit_page_boundary(&source, &request(&cache, empty, 6, 6, total), TextEncoding::Utf8);
        assert_eq!(zero.status, BoundaryCommitStatus::Rejected);
        assert_eq!(zero.rejection, Some(BoundaryRejection::EmptyInterval));

        let reversed = cache.begin_request();
        let backwards =
            cache.commit_page_boundary(&source, &request(&cache, reversed, 12, 6, total), TextEncoding::Utf8);
        assert_eq!(
            backwards.rejection,
            Some(BoundaryRejection::ReversedInterval)
        );

        let beyond = cache.begin_request();
        let out =
            cache.commit_page_boundary(&source, &request(&cache, beyond, 0, total + 1, total), TextEncoding::Utf8);
        assert_eq!(out.rejection, Some(BoundaryRejection::OutOfRange));
        assert_eq!(out.error_code, 2);

        let mismatch = cache.begin_request();
        let wrong_total =
            cache.commit_page_boundary(&source, &request(&cache, mismatch, 0, 6, total + 8), TextEncoding::Utf8);
        assert_eq!(
            wrong_total.rejection.expect("必须带拒绝原因"),
            BoundaryRejection::TotalLengthMismatch
        );
        assert_eq!(wrong_total.error_code, 1);

        assert!(cache.is_empty(), "被拒的边界不得写入任何状态");
    }

    #[test]
    fn pagination_cache_reports_scoped_invalidation() {
        let text = "Chapter 1 test\ncontent text here.\n".repeat(20);
        let (_guard, source) = temp_source("invalidate", text.as_bytes());
        let total = source.len();
        let mut cache = PaginationCache::new(LayoutSignature::default(), 1).expect("建立缓存");
        let signature = cache.signature().clone();

        for (start, end) in [(0u64, 10u64), (10, 20), (20, 30)] {
            let request_id = cache.begin_request();
            let outcome = cache
                .commit_page_boundary(&source, &request(&cache, request_id, start, end, total), TextEncoding::Utf8);
            assert!(outcome.is_committed(), "{outcome:?}");
        }
        assert_eq!(cache.len(), 3);
        let previous_layout_id = cache.layout_id();

        let invalidation = cache.invalidate(&signature);
        assert_eq!(invalidation.removed, 3);
        assert!(invalidation.generation_bumped);
        assert_eq!(invalidation.invalidated_layout_id, Some(previous_layout_id));
        assert!(cache.is_empty());
        assert_eq!(
            cache.query_page_boundary(previous_layout_id, 0),
            None,
            "失效后旧世代边界必须不可查"
        );

        // 不相关签名：不删任何东西，也不递增世代。
        let other = LayoutSignature {
            font_size_milli: 26_000,
            ..signature
        };
        let noop = cache.invalidate(&other);
        assert_eq!(noop.removed, 0);
        assert!(!noop.generation_bumped);
        assert_eq!(noop.layout_id, cache.layout_id());

        // 世代已递增：携带旧 layout_id 的在途结果必须被丢弃，且不写入任何缓存状态。
        let request_id = cache.begin_request();
        let committed =
            cache.commit_page_boundary(&source, &request(&cache, request_id, 0, 10, total), TextEncoding::Utf8);
        assert!(committed.is_committed(), "{committed:?}");
        let mut stale_request = request(&cache, request_id, 40, 50, total);
        stale_request.layout_id = previous_layout_id;
        let dropped = cache.commit_page_boundary(&source, &stale_request, TextEncoding::Utf8);
        assert_eq!(dropped.status, BoundaryCommitStatus::StaleLayoutId);
        assert_eq!(dropped.error_code, 8);
        assert!(dropped.is_stale());
        assert_eq!(dropped.stale_layout_id, Some(previous_layout_id));
        assert_eq!(dropped.boundary, None);
        assert_eq!(cache.len(), 1, "过期结果不得写入缓存");
        assert_eq!(cache.query_page_boundary(cache.layout_id(), 40), None);
    }

    #[test]
    fn pagination_layout_change_invalidates_all_and_rejects_invalid_signature() {
        let text = "Chapter 1 test\ncontent text here.\n".repeat(20);
        let (_guard, source) = temp_source("layout-change", text.as_bytes());
        let total = source.len();
        let mut cache = PaginationCache::new(LayoutSignature::default(), 1).expect("建立缓存");
        let request_id = cache.begin_request();
        assert!(cache
            .commit_page_boundary(&source, &request(&cache, request_id, 0, 10, total), TextEncoding::Utf8)
            .is_committed());
        let old_layout_id = cache.layout_id();

        // 幂等：同一签名 + 同一内容索引版本不得清空缓存。
        let same = cache
            .set_layout(cache.signature().clone(), 1)
            .expect("设置相同签名");
        assert!(!same.changed);
        assert_eq!(same.removed, 0);
        assert_eq!(cache.len(), 1);

        // 字号变化 → 缓存键变化 → 旧边界全部失效并递增世代。
        let bigger = LayoutSignature {
            font_size_milli: 32_000,
            ..cache.signature().clone()
        };
        let change = cache.set_layout(bigger.clone(), 1).expect("设置新签名");
        assert!(change.changed);
        assert_eq!(change.removed, 1);
        assert_eq!(change.previous_layout_id, old_layout_id);
        assert_eq!(change.layout_id, old_layout_id + 1);
        assert!(cache.is_empty());
        assert_eq!(cache.query_page_boundary(old_layout_id, 0), None);
        assert_eq!(cache.signature(), &bigger);

        // 非法签名（零视口）直接报错，不改变现有缓存键。
        let invalid = LayoutSignature {
            viewport_width: 0,
            ..cache.signature().clone()
        };
        let error = cache.set_layout(invalid, 1).expect_err("非法签名必须被拒");
        assert_eq!(error.code(), ErrorCode::InvalidArgument);
        assert_eq!(cache.signature(), &bigger);
    }

    #[test]
    fn pagination_page_index_is_exact_not_percentage() {
        let text = "The quick brown fox jumps over the lazy dog.\n".repeat(30);
        let (_guard, source) = temp_source("page-index", text.as_bytes());
        let total = source.len();
        assert!(total > 900, "文本必须足够长: {total}");
        let mut cache = PaginationCache::new(LayoutSignature::default(), 1).expect("建立缓存");

        // 刻意使用不等长页面：[0,100) [100,400) [400,900) [900,total)
        let sizes = [100u64, 300, 500, total - 900];
        let mut start = 0u64;
        for size in sizes {
            let end = start + size;
            let request_id = cache.begin_request();
            let outcome = cache
                .commit_page_boundary(&source, &request(&cache, request_id, start, end, total), TextEncoding::Utf8);
            assert_eq!(
                outcome.status,
                BoundaryCommitStatus::Committed,
                "{outcome:?}"
            );
            start = end;
        }
        assert_eq!(cache.len(), 4);
        assert_eq!(start, total, "页面必须完整覆盖内容");

        let boundaries = cache.ordered_boundaries();
        let chain = cache
            .page_chain()
            .expect("页面必须首尾相接（不丢字/不重字）");
        assert_eq!(chain.covered(), (0, total));
        assert_eq!(chain.len(), 4);
        // 正反向翻页取自同一份边界。
        assert_eq!(chain.next_of(0), Some(&PageBoundary::new(100, 400)));
        assert_eq!(chain.previous_of(2), Some(&PageBoundary::new(100, 400)));
        assert_eq!(chain.next_of(3), None);
        assert_eq!(chain.previous_of(0), None);

        // 精确页码：页面切换点恰好是边界起点，逐页可验证。
        for (index, boundary) in boundaries.iter().enumerate() {
            assert_eq!(
                cache.page_index_for_offset(boundary.start_byte),
                Some(index)
            );
            assert_eq!(
                cache.page_index_for_offset(boundary.end_byte - 1),
                Some(index)
            );
        }
        assert_eq!(
            cache.page_index_for_offset(total),
            None,
            "文件末尾不属于任何页面"
        );

        // 不得用百分比/等分近似页码：等分近似的切换点在别处。
        let equal_share = total / boundaries.len() as u64;
        let percent_switch = equal_share * 2;
        assert_ne!(
            boundaries[2].start_byte, percent_switch,
            "精确切换点不得等于按字节等分的近似切换点"
        );
        assert_eq!(cache.page_index_for_offset(399), Some(1));
        assert_eq!(cache.page_index_for_offset(400), Some(2));
    }
}

//! 平台窗口内容协议：**平台提供受限窗口字节，Rust 裁定坐标、边界与位置**。
//!
//! # 协议（冻结，写入头文件）
//!
//! 当数据源种类为 `NR_SOURCE_KIND_PLATFORM`（Android SAF 等）时，字节由平台读取并
//! **按窗口**交给共享库：
//!
//! 1. Rust 决定读取区间：`offset`（原始字节坐标）与 `max_len`（≤ 引擎窗口上限，
//!    默认 64 KiB、硬上限 256 KiB）；
//! 2. 平台把该区间的字节写入 `dst`（容量即 `max_len`），并回报 `out_len` 与 `out_eof`；
//! 3. Rust 校验 `out_len <= max_len`（超长返回值一律判为内部错误，防止平台一次性
//!    交付整本正文），随后在**字节层面**修正 UTF-8 边界并回报
//!    `dropped_leading` / `trimmed_tail` / `eof`。
//!
//! 因此跨边界**永远不会出现整本正文**：单次交付量由 Rust 的窗口中核算，而不是由平台自述。
//! 本模块的 [`window_provider::MemoryWindowProvider`] 是协议的可执行参考实现（等价于
//! Kotlin 侧应实现的行为），也被测试用于验证越界/超长/失败/延迟四类响应。
//!
//! # 线程与生命周期
//!
//! - 回调在**调用线程同步执行**（不做线程切换），且同一时刻只会有一次回调在飞
//!   （引擎互斥锁保证串行）；
//! - 回调**不得**回调任何 `nr_*` 入口（重入同一引擎会死锁）；
//! - 回调不得 panic 穿透 C ABI：Rust 侧以 `catch_unwind` 包裹整个入口，
//!   回调 panic 会转换为 `NR_ERR_INTERNAL`；
//! - `user` 指针由平台持有，其生命周期必须覆盖引擎存续期；引擎销毁后 Rust 不再回调。

use std::os::raw::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

use reading_source::{align_to_utf8_boundaries, Fingerprint, ReadStats, SourceError, SourceMetadata, TextSource, TextWindow, TextEncoding};

use crate::abi::invalid;

/// 平台窗口提供者（与头文件 `nr_source_provider` 字段一致）。
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NrSourceProvider {
    /// 平台侧不透明上下文指针（原样回传给回调）。
    pub user: *mut c_void,
    /// 读取回调；`None` 表示未提供（平台数据源将不可用）。
    pub read_window: Option<
        unsafe extern "C" fn(
            user: *mut c_void,
            request_id: u64,
            offset: u64,
            max_len: u32,
            dst: *mut u8,
            out_len: *mut u32,
            out_eof: *mut i32,
        ) -> i32,
    >,
}

// SAFETY：`NrSourceProvider` 只包含一个平台侧不透明指针与一个函数指针，二者都由平台保证
// 在引擎存续期内有效；引擎互斥锁保证同一时刻只有一次回调在飞，因此跨线程只是"移交调用权"，
// 不构成对 Rust 引用的跨边界传递。
unsafe impl Send for NrSourceProvider {}
unsafe impl Sync for NrSourceProvider {}

impl NrSourceProvider {
    /// 未提供回调（文件数据源场景）。
    pub const fn none() -> Self {
        Self {
            user: std::ptr::null_mut(),
            read_window: None,
        }
    }

    /// 是否已提供可用的读取回调。
    pub fn is_available(&self) -> bool {
        self.read_window.is_some()
    }
}

/// 平台窗口数据源：把一次窗口读取转成一次受限回调。
pub struct PlatformWindowSource {
    provider: NrSourceProvider,
    total_bytes: u64,
    identity: String,
    display_name: Option<String>,
    max_window_bytes: usize,
    read_sequence: AtomicU64,
    read_calls: AtomicU64,
    max_materialized_bytes: AtomicU64,
    over_return_rejections: AtomicU64,
}

// SAFETY：句柄与回调指针只在引擎互斥锁下被单线程使用；协议要求平台侧 `user` 上下文
// 至少与引擎同生命周期，并允许从任意单个调用线程访问。这里不是把平台对象当成 Rust 引用，
// 只是承认 C ABI 的 `void *` 事实上跨线程共享。
unsafe impl Send for PlatformWindowSource {}
unsafe impl Sync for PlatformWindowSource {}

impl PlatformWindowSource {
    /// 建立平台窗口数据源。
    pub fn new(
        provider: NrSourceProvider,
        total_bytes: u64,
        identity: impl Into<String>,
        display_name: Option<String>,
        max_window_bytes: usize,
    ) -> Self {
        Self {
            provider,
            total_bytes,
            identity: identity.into(),
            display_name,
            max_window_bytes,
            read_sequence: AtomicU64::new(0),
            read_calls: AtomicU64::new(0),
            max_materialized_bytes: AtomicU64::new(0),
            over_return_rejections: AtomicU64::new(0),
        }
    }

    /// 数据源长度（平台自述的坐标空间长度）。
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    /// 平台侧返回超长窗口而被拒绝的次数（必须恒为 0；测试断言用）。
    pub fn over_return_rejections(&self) -> u64 {
        self.over_return_rejections.load(Ordering::Relaxed)
    }

    fn callback(
        &self,
    ) -> Option<unsafe extern "C" fn(*mut c_void, u64, u64, u32, *mut u8, *mut u32, *mut i32) -> i32>
    {
        self.provider.read_window
    }

    /// 执行一次受控窗口读取。
    fn read_bounded(&self, offset: u64, len: usize) -> Result<(Vec<u8>, bool), SourceError> {
        let Some(callback) = self.callback() else {
            return Err(SourceError::Internal);
        };
        // 窗口上限由 Rust 裁定：绝不请求超过硬上限的区间。
        let request_len = len.min(self.max_window_bytes);
        let request_len = request_len.min(crate::abi::MAX_WINDOW_BYTES);
        if request_len == 0 {
            return Err(SourceError::InvalidArgument);
        }
        let request_seq = self.read_sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let mut buffer = vec![0u8; request_len];
        let mut out_len: u32 = 0;
        let mut out_eof: i32 = 0;
        let code = unsafe {
            callback(
                self.provider.user,
                request_seq,
                offset,
                request_len as u32,
                buffer.as_mut_ptr(),
                &mut out_len,
                &mut out_eof,
            )
        };
        if code != 0 {
            return Err(map_provider_error(code));
        }
        self.read_calls.fetch_add(1, Ordering::Relaxed);
        let delivered = out_len as usize;
        if delivered > request_len {
            // 平台一次性交付超出窗口上限的字节：拒绝且不采信该长度。
            self.over_return_rejections.fetch_add(1, Ordering::Relaxed);
            return Err(SourceError::Internal);
        }
        buffer.truncate(delivered);
        self.max_materialized_bytes
            .fetch_max(delivered as u64, Ordering::Relaxed);
        Ok((buffer, out_eof != 0))
    }

    /// 本次读取实际物化的最大字节数（测试用于证明"从未整本读取"）。
    pub fn max_materialized(&self) -> u64 {
        self.max_materialized_bytes.load(Ordering::Relaxed)
    }
}

/// 把平台回调的错误码映射到数据源错误（数值即 `NR_ERR_*`）。
///
/// 平台回调返回 `7`（取消）时按 I/O 中断处理：`reading-source` 没有取消变体，
/// 上层（章节扫描/索引重建）用请求级取消标志表达取消语义。
fn map_provider_error(code: i32) -> SourceError {
    match code {
        1 => SourceError::InvalidArgument,
        2 => SourceError::OutOfRange,
        3 => SourceError::Utf8Boundary,
        6 | 7 => SourceError::IoError,
        _ => SourceError::Internal,
    }
}

impl TextSource for PlatformWindowSource {
    fn metadata(&self) -> SourceMetadata {
        SourceMetadata {
            size: self.total_bytes,
            modified: None,
            display_name: self.display_name.clone(),
            source_identity: self.identity.clone(),
        }
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<TextWindow, SourceError> {
        self.read_at_with_encoding(offset, len, TextEncoding::Utf8)
    }

    fn read_at_with_encoding(
        &self,
        offset: u64,
        len: usize,
        encoding: TextEncoding,
    ) -> Result<TextWindow, SourceError> {
        if len == 0 {
            return Err(SourceError::InvalidArgument);
        }
        if len > crate::abi::MAX_WINDOW_BYTES {
            // 超出硬上限：拒绝，不分配、不读取。
            return Err(SourceError::InvalidArgument);
        }
        if offset > self.total_bytes {
            return Err(SourceError::OutOfRange);
        }
        let (raw, callback_eof) = self.read_bounded(offset, len)?;
        // 按**内容编码**对齐（SAF 等平台数据源同样适用）。
        let adjustment = reading_source::align_to_boundaries(&raw, offset, encoding);
        let keep = adjustment.keep_range(raw.len());
        let end = offset + raw.len() as u64;
        let eof = callback_eof || end >= self.total_bytes;
        Ok(TextWindow {
            offset,
            data: raw[keep].to_vec(),
            dropped_leading: adjustment.dropped_leading,
            trimmed_tail: adjustment.trimmed_tail,
            eof,
            requested_len: len,
        })
    }

    fn fingerprint(&self) -> Result<Fingerprint, SourceError> {
        Fingerprint::compute(
            self.total_bytes,
            reading_source::FINGERPRINT_SAMPLE_BYTES,
            None,
            |offset, len| {
                // 指纹只取头/中/尾各 4 KiB，同样经由受限窗口回调，不整本读取。
                let (bytes, _) = self.read_bounded(offset, len)?;
                Ok(bytes)
            },
        )
    }

    fn can_random_access(&self) -> bool {
        true
    }

    fn max_window_bytes(&self) -> usize {
        self.max_window_bytes
    }

    fn stats(&self) -> ReadStats {
        ReadStats {
            read_calls: self.read_calls.load(Ordering::Relaxed),
            last_materialized_bytes: self.max_materialized_bytes.load(Ordering::Relaxed),
            total_materialized_bytes: self.max_materialized_bytes.load(Ordering::Relaxed),
            max_materialized_bytes: self.max_materialized_bytes.load(Ordering::Relaxed),
        }
    }
}

/// 校验平台回调返回之后的结构体/句柄状态（供 `engine` 使用）。
pub(crate) fn ensure_provider_available(provider: &NrSourceProvider) -> reading_core::NrResult<()> {
    if provider.is_available() {
        Ok(())
    } else {
        Err(invalid(
            "平台数据源必须提供 nr_source_provider::read_window 回调",
        ))
    }
}

/// 协议的可执行参考实现：内存窗口提供者。
///
/// 它与 Kotlin 侧实现同一套契约（受限窗口、`out_len`/`out_eof`、错误码），
/// 因此既可作为平台适配层的参考，也被测试用来构造超长/失败/延迟等响应。
pub mod window_provider {
    use std::os::raw::c_void;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::Duration;

    use super::NrSourceProvider;

    /// 内存窗口提供者：以一段内存作为平台内容源。
    pub struct MemoryWindowProvider {
        bytes: Vec<u8>,
        delay: Option<Duration>,
        fail_with: Option<i32>,
        over_return: bool,
        pub calls: AtomicU64,
        pub max_requested: AtomicU64,
        pub request_log: std::sync::Mutex<Vec<(u64, u32)>>,
        pub stopped: AtomicBool,
    }

    impl MemoryWindowProvider {
        /// 以给定字节建立提供者（平台侧内容）。
        pub fn new(bytes: Vec<u8>) -> Self {
            Self {
                bytes,
                delay: None,
                fail_with: None,
                over_return: false,
                calls: AtomicU64::new(0),
                max_requested: AtomicU64::new(0),
                request_log: std::sync::Mutex::new(Vec::new()),
                stopped: AtomicBool::new(false),
            }
        }

        /// 每次回调前睡眠（用于构造可被取消的长任务）。
        pub fn with_delay(mut self, delay: Duration) -> Self {
            self.delay = Some(delay);
            self
        }

        /// 固定返回错误码（用于错误映射测试）。
        pub fn with_failure(mut self, code: i32) -> Self {
            self.fail_with = Some(code);
            self
        }

        /// 故意回报 `out_len > max_len` 而不多写字节（用于验证 Rust 侧的超长拒绝）。
        pub fn with_over_return(mut self, enabled: bool) -> Self {
            self.over_return = enabled;
            self
        }

        /// 回调次数。
        pub fn calls(&self) -> u64 {
            self.calls.load(Ordering::Relaxed)
        }

        /// 被请求过的最大窗口长度（必须 ≤ 硬上限）。
        pub fn max_requested(&self) -> u64 {
            self.max_requested.load(Ordering::Relaxed)
        }

        /// 取 ABI 形态的提供者（`user` 指向 `self`）。
        pub fn provider(&self) -> NrSourceProvider {
            NrSourceProvider {
                user: (self as *const Self).cast_mut().cast::<c_void>(),
                read_window: Some(memory_window_read),
            }
        }
    }

    /// 与 Kotlin 侧等价的读取回调实现。
    ///
    /// # Safety
    ///
    /// `user` 必须指向一个仍然存活的 [`MemoryWindowProvider`]；`dst` 必须指向至少
    /// `max_len` 字节的可写内存；`out_len` / `out_eof` 必须可写。
    pub unsafe extern "C" fn memory_window_read(
        user: *mut c_void,
        _request_id: u64,
        offset: u64,
        max_len: u32,
        dst: *mut u8,
        out_len: *mut u32,
        out_eof: *mut i32,
    ) -> i32 {
        if user.is_null() || dst.is_null() || out_len.is_null() || out_eof.is_null() {
            return 1;
        }
        let provider = &*(user.cast::<MemoryWindowProvider>());
        provider.calls.fetch_add(1, Ordering::Relaxed);
        provider
            .max_requested
            .fetch_max(u64::from(max_len), Ordering::Relaxed);
        if let Ok(mut log) = provider.request_log.lock() {
            log.push((offset, max_len));
        }
        if let Some(delay) = provider.delay {
            std::thread::sleep(delay);
        }
        if let Some(code) = provider.fail_with {
            return code;
        }
        let total = provider.bytes.len() as u64;
        if offset > total {
            return 2;
        }
        let available = (total - offset) as usize;
        let take = available.min(max_len as usize);
        if take > 0 {
            std::ptr::copy_nonoverlapping(provider.bytes.as_ptr().add(offset as usize), dst, take);
        }
        if provider.over_return {
            // 恶意/有缺陷的平台实现：回报比容量更长的长度，但不越界写。
            *out_len = max_len.saturating_add(4096);
        } else {
            *out_len = take as u32;
        }
        *out_eof = i32::from(offset + take as u64 >= total);
        0
    }
}

#[cfg(test)]
mod tests {
    use super::window_provider::MemoryWindowProvider;
    use super::*;

    fn text_of(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }

    #[test]
    fn platform_source_never_requests_more_than_window_cap() {
        let provider = MemoryWindowProvider::new(vec![b'a'; 700_000]);
        let source = PlatformWindowSource::new(
            provider.provider(),
            700_000,
            "content://demo/book.txt",
            Some("book.txt".to_string()),
            crate::abi::MAX_WINDOW_BYTES,
        );
        let window = source.read_at(0, 64 * 1024).expect("读取窗口");
        assert_eq!(window.data.len(), 64 * 1024);
        assert_eq!(provider.max_requested(), 64 * 1024);
        assert!(provider.max_requested() <= crate::abi::MAX_WINDOW_BYTES as u64);
    }

    #[test]
    fn platform_source_rejects_over_long_window_request_without_reading() {
        let provider = MemoryWindowProvider::new(vec![b'a'; 1000]);
        let source = PlatformWindowSource::new(
            provider.provider(),
            1000,
            "content://demo/book.txt",
            None,
            crate::abi::MAX_WINDOW_BYTES,
        );
        let error = source
            .read_at(0, crate::abi::MAX_WINDOW_BYTES + 1)
            .expect_err("超过硬上限必须拒绝");
        assert_eq!(error, SourceError::InvalidArgument);
        assert_eq!(provider.calls(), 0, "拒绝必须先于任何读取");
    }

    #[test]
    fn platform_source_rejects_platform_over_return() {
        let provider = MemoryWindowProvider::new(vec![b'a'; 4096]).with_over_return(true);
        let source = PlatformWindowSource::new(
            provider.provider(),
            4096,
            "content://demo/book.txt",
            None,
            crate::abi::MAX_WINDOW_BYTES,
        );
        let error = source.read_at(0, 1024).expect_err("超长回报必须拒绝");
        assert_eq!(error, SourceError::Internal);
        assert_eq!(source.over_return_rejections(), 1);
    }

    #[test]
    fn platform_source_maps_callback_errors_to_stable_codes() {
        for (code, expected) in [
            (1, SourceError::InvalidArgument),
            (2, SourceError::OutOfRange),
            (3, SourceError::Utf8Boundary),
            (6, SourceError::IoError),
            (7, SourceError::IoError),
            (99, SourceError::Internal),
        ] {
            let provider = MemoryWindowProvider::new(vec![b'a'; 64]).with_failure(code);
            let source = PlatformWindowSource::new(
                provider.provider(),
                64,
                "content://demo/book.txt",
                None,
                crate::abi::MAX_WINDOW_BYTES,
            );
            assert_eq!(source.read_at(0, 32).expect_err("回调失败"), expected);
        }
    }

    #[test]
    fn platform_source_aligns_utf8_window_boundaries() {
        let text = "中文abc中文";
        let provider = MemoryWindowProvider::new(text.as_bytes().to_vec());
        let total = text.len() as u64;
        let source = PlatformWindowSource::new(
            provider.provider(),
            total,
            "content://demo/book.txt",
            None,
            crate::abi::MAX_WINDOW_BYTES,
        );
        // 请求到第 4 个字节（落在"中"的中间）：Rust 必须裁剪到合法边界。
        let window = source.read_at(0, 4).expect("读取窗口");
        assert!(window.trimmed_tail > 0, "尾部必须被裁剪到字符边界");
        let decoded = window.as_str().expect("裁剪后必须是合法 UTF-8");
        assert!(text.starts_with(decoded));
        assert!(window.end() <= total);
        assert_eq!(text_of(&window.data), decoded);
    }

    #[test]
    fn platform_source_reports_eof_and_bounded_fingerprint() {
        let provider = MemoryWindowProvider::new(b"0123456789abcdef".to_vec());
        let source = PlatformWindowSource::new(
            provider.provider(),
            16,
            "content://demo/book.txt",
            Some("book.txt".to_string()),
            crate::abi::MAX_WINDOW_BYTES,
        );
        let window = source.read_at(8, 64).expect("读取尾部窗口");
        assert!(window.eof, "越过末尾必须回报 EOF");
        assert_eq!(window.data, b"89abcdef");
        let fingerprint = source.fingerprint().expect("指纹");
        assert_eq!(fingerprint.size, 16);
        assert!(fingerprint.sampled_bytes <= 3 * reading_source::FINGERPRINT_SAMPLE_BYTES as u64);
        assert!(source.max_materialized() <= 3 * reading_source::FINGERPRINT_SAMPLE_BYTES as u64);
        let metadata = source.metadata();
        assert_eq!(metadata.size, 16);
        assert_eq!(metadata.source_identity, "content://demo/book.txt");
        assert_eq!(metadata.display_name.as_deref(), Some("book.txt"));
        assert!(source.can_random_access());
    }

    #[test]
    fn unavailable_provider_is_reported_as_invalid_argument() {
        let provider = NrSourceProvider::none();
        assert!(!provider.is_available());
        let error = ensure_provider_available(&provider).expect_err("缺少回调必须报错");
        assert_eq!(error.code(), reading_core::ErrorCode::InvalidArgument);
        assert!(error.message().contains("read_window"));
    }
}

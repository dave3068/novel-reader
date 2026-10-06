//! `reading-ffi`：正式跨语言 ABI 导出层（C ABI / JNI / 平台窗口内容协议）。
//!
//! 冻结契约见 `results/gates/gate-0-baseline.md` §2.7：**18 项入口** + 向后兼容新增的 `nr_set_encoding`（共 19 项）
//! （16 项最小接口 + [`exports::nr_release_buffer`] + [`exports::nr_abi_version`]）。
//! 头文件 `include/novel_reader.h` 是本层的 C 侧对照，二者由
//! `scripts/verify-abi.py`（导出符号）与 `tests/header_parity.rs`（签名与结构体字段）
//! 逐项校验。
//!
//! # 模块
//!
//! - [`abi`]：常量、错误码/编码/拒绝原因的跨语言映射、冻结入口清单；
//! - [`structs`]：`#[repr(C)]` 结构体与头文件字段清单（`ABI_STRUCTS`）；
//! - [`buffer`]：缓冲区所有权契约与 [`exports::nr_release_buffer`] 的实现；
//! - [`handle`]：不透明 64 位引擎句柄的分配、查找、销毁与销毁后语义；
//! - [`external`]：平台窗口内容协议（平台提供受限窗口字节，Rust 裁定坐标与边界）；
//! - [`cancel`]：跨线程可见的协作式取消；
//! - [`engine`]：把入口映射到共享核心与本地存储的真实语义；
//! - [`exports`]：19 项 `#[no_mangle] extern "C"` 导出（18 项冻结契约 + `nr_set_encoding`）。
//!
//! # 硬性边界（卡片禁止事项）
//!
//! - 跨边界只传整数、指针与长度；**不传** Kotlin/Slint 对象，也不传 Rust 引用；
//! - **不一次返回整本正文**：所有正文访问都受 64 KiB 默认 / 256 KiB 硬上限约束，
//!   平台数据源的单次交付量由 Rust 决定并校验（超长回报被拒绝）；
//! - **缓冲区所有权唯一**：Rust 分配、调用方 [`exports::nr_release_buffer`] 归还，
//!   归还后字段置零且重复归还安全；
//! - 本 crate 不实现 UI、Activity 生命周期或网络逻辑。

pub mod abi;
pub mod buffer;
pub mod cancel;
pub mod engine;
pub mod exports;
pub mod external;
pub mod handle;
pub mod structs;

pub use reading_core::{ABI_VERSION, CORE_VERSION};

/// V1 冻结的 ABI 入口数量（16 项最小接口 + 缓冲区释放 + ABI 版本查询）。
///
/// 冻结依据：`results/gates/gate-0-baseline.md` §2.7。
pub const ABI_ENTRY_COUNT: usize = 40;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_version_is_reexported_from_core() {
        assert_eq!(ABI_VERSION, reading_core::ABI_VERSION);
        assert_eq!(ABI_VERSION, 1);
        assert_eq!(abi::ABI_VERSION, ABI_VERSION);
    }

    #[test]
    fn abi_entry_count_matches_frozen_baseline() {
        assert_eq!(
            ABI_ENTRY_COUNT, 40,
            "18 项冻结 + nr_set_encoding + V1.1 分类 4 + 历史 3 + 配色 5 + 背景 4 + 作用域书籍 1 + 书籍分类映射 1 + 分类改名/移动 2 + 移动目标 1 项"
        );
        assert_eq!(CORE_VERSION, reading_core::CORE_VERSION);
        assert_eq!(abi::ABI_ENTRY_NAMES.len(), ABI_ENTRY_COUNT);
    }

    #[test]
    fn window_limits_are_reexported_with_frozen_values() {
        assert_eq!(abi::DEFAULT_WINDOW_BYTES, 64 * 1024);
        assert_eq!(abi::MAX_WINDOW_BYTES, 256 * 1024);
        let default_window: usize = abi::DEFAULT_WINDOW_BYTES;
        let max_window: usize = abi::MAX_WINDOW_BYTES;
        assert!(default_window <= max_window);
    }
}

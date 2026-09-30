//! 请求取消：**协作式**、跨线程可见、按（引擎, 请求 id）隔离。
//!
//! # 语义（写入头文件）
//!
//! - [`request_cancel`] 把 `(引擎, 请求 id)` 标记为已取消；`request_id` 必须非 0，
//!   否则返回 `NR_ERR_INVALID_ARGUMENT`；
//! - 长任务（章节索引重建）在**每个窗口**查询一次该标志；一旦命中即以
//!   `NR_ERR_CANCELLED`（7）返回**部分结果**（`nr_rebuild_result.partial = 1`），
//!   调用方可携带同一 `request_id` 重试续做；
//! - 取消标志与引擎互斥锁**分离存放**：因此另一个线程可以在长任务运行期间调用
//!   `nr_cancel_request`，不会被引擎锁阻塞（这是"可取消"能真正生效的前提）；
//! - 任务结束（成功、拒绝或取消）后标志被清除；引擎销毁时清除该引擎的全部标志，
//!   不会无界增长。
//!
//! 设计依据：`design.md` §4「每个异步请求带 request id」、`reading-core` CR6「请求取消」。

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

use reading_core::{NrError, NrResult};

use crate::abi::{internal, invalid};

/// 全局取消标志表：键为 `(引擎句柄, 请求 id)`。
static CANCELLED: LazyLock<Mutex<HashSet<(u64, u64)>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

fn table() -> NrResult<std::sync::MutexGuard<'static, HashSet<(u64, u64)>>> {
    CANCELLED
        .lock()
        .map_err(|_| internal("取消标志表互斥锁中毒"))
}

/// 标记请求已取消（`nr_cancel_request` 的实现主体）。
pub fn request_cancel(engine: u64, request_id: u64) -> NrResult<()> {
    if request_id == 0 {
        return Err(invalid(
            "request_id 0 永远无效（取消请求必须给出真实请求标识）",
        ));
    }
    table()?.insert((engine, request_id));
    Ok(())
}

/// 查询取消标志（长任务在窗口之间调用；只取取消表这把锁，不碰引擎锁）。
pub fn is_cancelled(engine: u64, request_id: u64) -> bool {
    match table() {
        Ok(table) => table.contains(&(engine, request_id)),
        Err(_) => false,
    }
}

/// 清除单个请求的取消标志（任务结束时调用）。
pub fn clear(engine: u64, request_id: u64) {
    if let Ok(mut table) = table() {
        table.remove(&(engine, request_id));
    }
}

/// 清除某引擎的全部取消标志（引擎销毁时调用，避免墓碑无界增长）。
pub fn clear_engine(engine: u64) {
    if let Ok(mut table) = table() {
        table.retain(|(handle, _)| *handle != engine);
    }
}

/// 当前登记的标志数量（测试用）。
pub fn tracked_count() -> usize {
    table().map(|table| table.len()).unwrap_or(0)
}

/// 便于调用方在取消时构造统一错误。
pub fn cancelled_error(request_id: u64) -> NrError {
    crate::abi::cancelled(format!("请求 {request_id} 已被取消（结果为部分结果）"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_flag_is_scoped_per_engine_and_request() {
        let engine = 900_001;
        let other = 900_002;
        request_cancel(engine, 42).expect("标记取消");
        assert!(is_cancelled(engine, 42));
        assert!(!is_cancelled(engine, 43), "请求 id 必须隔离");
        assert!(!is_cancelled(other, 42), "引擎必须隔离");
        clear(engine, 42);
        assert!(!is_cancelled(engine, 42));
    }

    #[test]
    fn request_id_zero_is_rejected() {
        let error = request_cancel(1, 0).expect_err("request_id 0 必须拒绝");
        assert_eq!(error.code(), reading_core::ErrorCode::InvalidArgument);
    }

    #[test]
    fn clear_engine_removes_only_that_engine() {
        request_cancel(910_001, 1).expect("标记");
        request_cancel(910_002, 1).expect("标记");
        clear_engine(910_001);
        assert!(!is_cancelled(910_001, 1));
        assert!(is_cancelled(910_002, 1));
        clear_engine(910_002);
        assert_eq!(tracked_count(), 0);
        assert_eq!(
            cancelled_error(7).code(),
            reading_core::ErrorCode::Cancelled
        );
    }
}

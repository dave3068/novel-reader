//! 引擎句柄注册表：不透明 64 位句柄的分配、查找、销毁与**销毁后语义**。
//!
//! # 契约（写入头文件）
//!
//! - 句柄是**不透明整数**（`uint64_t`），值 `0` 永远无效；跨边界不传任何 Rust/Kotlin 对象或引用；
//! - 句柄由 [`create`] 单调分配，**永不复用**；
//! - 销毁后再次使用同一句柄返回 `NR_ERR_ENGINE_DESTROYED`（9），
//!   而从未分配过的句柄返回 `NR_ERR_INVALID_ARGUMENT`（1）——两者可区分，便于平台定位生命周期错误；
//! - 全部入口都在同一把引擎互斥锁下串行执行（`design.md` §4「同一个阅读会话串行化」）；
//!   引擎互斥锁只保护该引擎自身，不影响其它引擎。

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use reading_core::{NrError, NrResult};

use crate::abi::{engine_destroyed, internal, invalid};
use crate::engine::Engine;

/// 活动引擎表 + 已销毁句柄墓碑。
struct Registry {
    engines: HashMap<u64, Arc<Mutex<Engine>>>,
    destroyed: HashSet<u64>,
}

static REGISTRY: LazyLock<Mutex<Registry>> = LazyLock::new(|| {
    Mutex::new(Registry {
        engines: HashMap::new(),
        destroyed: HashSet::new(),
    })
});

static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

fn registry() -> NrResult<MutexGuard<'static, Registry>> {
    REGISTRY
        .lock()
        .map_err(|_| internal("引擎注册表互斥锁中毒"))
}

/// 分配新句柄并登记引擎。
pub fn create(engine: Engine) -> NrResult<u64> {
    let mut registry = registry()?;
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    registry
        .engines
        .insert(handle, Arc::new(Mutex::new(engine)));
    Ok(handle)
}

/// 查找引擎；按「从未存在」/「已销毁」区分错误码。
pub fn lookup(handle: u64) -> NrResult<Arc<Mutex<Engine>>> {
    if handle == 0 {
        return Err(invalid("引擎句柄 0 永远无效"));
    }
    let registry = registry()?;
    if let Some(engine) = registry.engines.get(&handle) {
        return Ok(Arc::clone(engine));
    }
    if registry.destroyed.contains(&handle) {
        return Err(engine_destroyed());
    }
    Err(invalid(format!("未登记的引擎句柄: {handle}")))
}

/// 取出引擎互斥锁（中毒时映射为内部错误，而不是 panic 跨边界）。
///
/// 返回 guard 的生命周期绑定到传入的 `Arc`：调用方必须先持有 `Arc`，因此不存在
/// 悬垂引用，也不需要任何 `unsafe` 生命周期转换。
pub fn lock<'a>(engine: &'a Arc<Mutex<Engine>>) -> NrResult<std::sync::MutexGuard<'a, Engine>> {
    engine.lock().map_err(|_| internal("引擎互斥锁中毒"))
}

/// 销毁引擎：移除并记墓碑。
///
/// 返回值区分「首次销毁（`Ok`）」与「重复销毁（9）」「未登记（1）」。
pub fn destroy(handle: u64) -> NrResult<()> {
    if handle == 0 {
        return Err(invalid("引擎句柄 0 永远无效"));
    }
    let mut registry = registry()?;
    if registry.engines.remove(&handle).is_some() {
        registry.destroyed.insert(handle);
        return Ok(());
    }
    if registry.destroyed.contains(&handle) {
        return Err(engine_destroyed());
    }
    Err(invalid(format!("未登记的引擎句柄: {handle}")))
}

/// 活动引擎数量（测试用）。
pub fn live_engine_count() -> usize {
    registry()
        .map(|registry| registry.engines.len())
        .unwrap_or(0)
}

/// 已销毁句柄墓碑数量（测试用）。
pub fn destroyed_handle_count() -> usize {
    registry()
        .map(|registry| registry.destroyed.len())
        .unwrap_or(0)
}

/// 把统一错误映射为 ABI 错误码（`panic` 已被上层拦截，这里只处理业务错误）。
pub fn error_code(error: &NrError) -> i32 {
    error.code().as_c()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EngineConfig;

    fn engine() -> Engine {
        Engine::new(EngineConfig::in_memory()).expect("创建内存引擎")
    }

    #[test]
    fn handles_are_monotonic_and_never_reused() {
        let first = create(engine()).expect("创建");
        let second = create(engine()).expect("创建");
        assert!(second > first, "句柄必须单调递增");
        destroy(first).expect("销毁");
        destroy(second).expect("销毁");
    }

    #[test]
    fn destroyed_handle_reports_engine_destroyed_while_unknown_reports_invalid() {
        let handle = create(engine()).expect("创建");
        destroy(handle).expect("首次销毁必须成功");
        let error = destroy(handle).expect_err("重复销毁必须报错");
        assert_eq!(error.code(), reading_core::ErrorCode::EngineDestroyed);
        let error = lookup(handle).expect_err("销毁后查找必须报错");
        assert_eq!(error.code(), reading_core::ErrorCode::EngineDestroyed);
        let error = lookup(handle + 9_999).expect_err("未登记句柄必须报错");
        assert_eq!(error.code(), reading_core::ErrorCode::InvalidArgument);
        let error = lookup(0).expect_err("句柄 0 必须报错");
        assert_eq!(error.code(), reading_core::ErrorCode::InvalidArgument);
    }
}

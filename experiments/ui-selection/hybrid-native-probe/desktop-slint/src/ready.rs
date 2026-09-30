//! ready.json 就绪探针：原子写入 + 单文件校验字段（design.md §6）。

use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::app_state::ReaderApp;

/// 候选标识。
pub const CANDIDATE: &str = "hybrid-native";
/// 平台标识。
pub const PLATFORM: &str = "desktop";
/// 框架标识。
pub const FRAMEWORK: &str = "slint";
/// 框架版本（与 Cargo.toml 锁定一致）。
pub const FRAMEWORK_VERSION: &str = "1.17.1";

/// 默认 ready.json 路径（由环境变量 `NR_READY_PATH` 覆盖）。
pub fn default_path() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../results/raw/hybrid-native-desktop-ready.json"
    ))
}

/// 解析 ready.json 路径。
pub fn resolve_path() -> PathBuf {
    match std::env::var("NR_READY_PATH") {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        _ => default_path(),
    }
}

/// 原子写入：先写临时文件再 rename，避免读方看到半截内容。
pub fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}

/// 组装 ready.json 内容。
pub fn build_json(
    app: &mut ReaderApp,
    backend: &str,
    renderer: &str,
    window_width: i32,
    window_height: i32,
) -> String {
    let check = app.last_self_check().cloned();
    let pagination_status = check.as_ref().map(|c| c.status).unwrap_or("未验证");
    let pages = check.as_ref().map(|c| c.pages).unwrap_or(0);
    let backend = if backend == "wayland" {
        "wayland"
    } else {
        "x11"
    };
    let opened_book = app
        .engine()
        .opened_book()
        .map(|b| b.to_string())
        .unwrap_or_else(|| "null".to_string());
    let metrics = app.engine().metrics();

    format!(
        concat!(
            "{{",
            "\"candidate\":\"{}\",",
            "\"platform\":\"{}\",",
            "\"framework\":\"{}\",",
            "\"framework_version\":\"{}\",",
            "\"backend\":\"{}\",",
            "\"renderer\":\"{}\",",
            "\"pid\":{},",
            "\"ui_ready\":true,",
            "\"event_loop\":\"running\",",
            "\"pagination_status\":\"{}\",",
            "\"pages\":{},",
            "\"layout_id\":{},",
            "\"window\":{{\"width\":{},\"height\":{}}},",
            "\"core\":{{\"version\":\"{}\",\"abi_version\":{},\"abi_entries\":{},\"max_window\":{},\"opened_book\":{}}},",
            "\"self_check\":{},",
            "\"metrics\":{{\"read_calls\":{},\"copied_bytes\":{},\"materialized_bytes\":{},\"page_boundaries\":{},\"events_emitted\":{},\"layout_invalidations\":{}}},",
            "\"rust_business_coverage\":{},",
            "\"platform_specific_loc\":{},",
            "\"duplicated_ui_loc\":{},",
            "\"timestamp\":\"{}\"",
            "}}"
        ),
        CANDIDATE,
        PLATFORM,
        FRAMEWORK,
        FRAMEWORK_VERSION,
        backend,
        renderer,
        std::process::id(),
        pagination_status,
        pages,
        app.layout_id(),
        window_width,
        window_height,
        rust_reading_core::CORE_VERSION,
        rust_reading_core::ABI_VERSION,
        rust_reading_core::ABI_ENTRY_COUNT,
        app.engine().max_window(),
        opened_book,
        check
            .as_ref()
            .map(|c| c.to_json())
            .unwrap_or_else(|| "null".to_string()),
        metrics.read_calls,
        metrics.copied_bytes,
        metrics.materialized_bytes,
        metrics.page_boundaries,
        metrics.events_emitted,
        app.layout_invalidations(),
        // 复用覆盖率与代码量由 scripts/verify-hybrid-native-contract.py 复算并覆盖。
        100.0,
        0,
        0,
        iso8601_now()
    )
}

/// 当前 UTC 时间的 ISO-8601 形式。
pub fn iso8601_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year,
        month,
        day,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Howard Hinnant 的 civil-from-days 算法（1970-01-01 起的天数 → 年月日）。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_iso8601_is_well_formed() {
        let value = iso8601_now();
        assert!(value.ends_with('Z'));
        assert_eq!(value.len(), 20);
        assert_eq!(&value[4..5], "-");
        assert_eq!(&value[10..11], "T");
    }

    #[test]
    fn ready_atomic_write_replaces_file() {
        let dir = std::env::temp_dir().join("nr-ready-test");
        let path = dir.join("ready.json");
        write_atomic(&path, "{\"a\":1}").unwrap();
        write_atomic(&path, "{\"a\":2}").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\":2}");
    }
}

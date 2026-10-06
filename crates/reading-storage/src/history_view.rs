//! 阅读历史的**呈现口径**（V1.1；规格 §3.6「时间标签由核心格式化」）。
//!
//! **本模块是历史呈现语义的唯一真相**：桌面与 Android 都从这里取
//! 「时间标签」与「文件标签」，故两端显示的文本必然一致。
//!
//! 为什么在这里而不是 `desktop-app`：
//!
//! - 规格 P5 §3.6 明确要求 **时间标签由核心格式化后返回字符串**，Kotlin 不得自己
//!   格式化（那会让「今天 / 本年 / 跨年」的判定在两端各有一份，属语义下沉）；
//! - `reading-ffi` **不依赖** `desktop-app`（依赖方向：ffi → storage），
//!   若留在桌面侧，Android 就只能复制一份逻辑 —— 正是规格禁止的两个真相。
//!
//! 口径（与 `shelf::local_datetime` 一致）：
//!
//! - 时区用 SQLite `localtime`（跟随系统时区），**不引入时区库**；
//! - 今天 → `HH:MM`；本年 → `MM-DD HH:MM`；跨年 → `yyyy-MM-DD HH:MM`；
//! - 非正时间戳 → 空串（调用方据此走「无时间」分支）。

use rusqlite::Connection;

/// 时间标签：今天只显示 `HH:MM`，本年 `MM-DD HH:MM`，跨年 `yyyy-MM-dd HH:MM`。
///
/// **只在核心格式化** —— 界面拿到的已是成品文本，不得再拼装或截断。
///
/// `epoch <= 0` 返回空串（表示「无时间」，不是「1970 年」）。
pub fn time_label(conn: &Connection, epoch: i64) -> String {
    if epoch <= 0 {
        return String::new();
    }
    let full: Option<String> = conn
        .query_row(
            "SELECT strftime('%Y-%m-%d %H:%M', ?1, 'unixepoch', 'localtime')",
            [epoch],
            |row| row.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten();
    let Some(full) = full else {
        return String::new();
    };
    // `full` 形如 `2026-10-05 14:32`。
    let Some((date, time)) = full.split_once(' ') else {
        return full;
    };
    let today: Option<String> = conn
        .query_row(
            "SELECT strftime('%Y-%m-%d', 'now', 'localtime')",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok();
    let this_year: Option<String> = conn
        .query_row("SELECT strftime('%Y', 'now', 'localtime')", [], |row| {
            row.get::<_, String>(0)
        })
        .ok();
    if today.as_deref() == Some(date) {
        return time.to_string();
    }
    if let Some(year) = this_year.as_deref() {
        if date.len() == 10 && date.starts_with(year) {
            return format!("{} {}", &date[5..], time);
        }
    }
    full
}

/// 文件标签：取定位符**末段**（不暴露整条绝对路径）。
///
/// 两种分隔符都支持（`/` 与 `\`），空串回空串。
pub fn file_label(locator: &str) -> String {
    locator
        .rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or(locator)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Storage;
    use std::path::PathBuf;

    /// 独立临时库（进程 id + 线程 id 避免并行测试撞名）。
    struct Temp(PathBuf);

    impl Temp {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "nr-history-view-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("建临时目录");
            Self(dir)
        }
        fn db(&self) -> PathBuf {
            self.0.join("nr.db")
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn time_label_shortens_today_and_same_year() {
        let temp = Temp::new("time");
        let mut storage = Storage::open(temp.db()).expect("开库");
        storage.migrate().expect("迁移");
        let conn = storage.conn();
        let now = crate::connection::now_unix();

        let today = time_label(conn, now);
        assert!(!today.contains('-'), "今天只显示 HH:MM，实际：{today}");
        assert_eq!(today.len(), 5, "HH:MM 五字符，实际：{today}");

        // 2000-01-01：跨年 → 完整 yyyy-MM-dd HH:MM。
        let old: i64 = 946_684_800; // 2000-01-01 UTC
        let old_label = time_label(conn, old);
        assert_eq!(old_label.len(), 16, "跨年应完整显示，实际：{old_label}");
        assert!(old_label.contains('-'));

        assert_eq!(time_label(conn, 0), "", "0/负数应为空串");
    }

    #[test]
    fn file_label_takes_last_segment_for_both_separators() {
        assert_eq!(file_label("/home/u/小说/三体.txt"), "三体.txt");
        assert_eq!(file_label(r"C:\books\a.txt"), "a.txt");
        assert_eq!(file_label(""), "");
    }
}

//! 桌面应用入口：只做进程级启动，逻辑全部在 [`desktop_app::ui::run`]。

// Windows 发布构建使用 **GUI 子系统**：双击启动不再弹出多余的控制台窗口。
// 仅在非 debug 构建生效 —— debug 构建保留控制台，便于 `NR_STATE_LOG` 等结构化输出排查。
// Linux/macOS 上该属性无效果。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> Result<(), slint::PlatformError> {
    desktop_app::run()
}

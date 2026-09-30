//! 桌面端 Slint UI 绑定：唯一来源是 `ui/app.slint`。
//!
//! `slint!` 宏以相对 `src/ui.rs` 的路径解析 `ui/app.slint`，因此不存在
//! 「内联副本 + 文件副本」两份定义（H-R5 的重复 UI 代码量为 0）。

slint::slint! {
    export { AppWindow, BookRow, CategoryRow, ChapterRow } from "../ui/app.slint";
}

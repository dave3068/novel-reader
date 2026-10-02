//! 桌面输入意图映射（DT8 / DR4）：键盘按键与鼠标滚轮 → **整页翻页意图**。
//!
//! 边界（`desktop/spec.md` DR2/DR4、`design.md` §2）：
//!
//! - 本模块是**纯映射**：输入事件 → 翻页意图，不持有状态、不碰阅读语义。页面位置、
//!   页边界提交、进度保存全部仍然走 `pagination` / `progress`（冻结核心）；
//! - `.slint` 只做事件转发（`FocusScope.key-pressed` / `TouchArea.scroll-event`），
//!   「哪个物理按键 / 哪个滚动方向代表翻页」以及「输入框聚焦时必须不翻页」这两条规则
//!   集中在这里，并由本文件的单元测试强制（DR4 的可运行门禁）；
//! - `KeyEvent.text` 的取值来自 Slint 的 [`slint::platform::Key`] 常量（私有区字符）；
//!   这里只通过 `char::from(Key::…)` 取值，不手写魔法字符，避免与 Slint 版本脱节。
//!
//! DR3 说明：滚轮**不滚动正文**（正文区永不出现滚动条），而是映射为整页翻页。

use slint::platform::{Key, WindowEvent};
use slint::SharedString;

/// 整页翻页方向（与界面上「上一页 / 下一页」按钮同义）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageTurn {
    /// 上一页（PageUp / 上方向键 / 左方向键 / 向上滚动）。
    Prev,
    /// 下一页（PageDown / 下方向键 / 右方向键 / 向下滚动）。
    Next,
}

impl PageTurn {
    /// 供用户入口 [`crate::ui`] 使用的页数增量（负数向前、正数向后）。
    pub const fn delta(self) -> i32 {
        match self {
            PageTurn::Prev => -1,
            PageTurn::Next => 1,
        }
    }

    /// 结构化日志与证据里的稳定名称。
    pub const fn log_name(self) -> &'static str {
        match self {
            PageTurn::Prev => "prev",
            PageTurn::Next => "next",
        }
    }
}

/// 键盘翻页键表：`(便携名称, Slint Key 常量, 翻页方向)`。
///
/// 便携名称用于结构化证据（`NR-INPUT key=…`）与程序化验证的环境变量取值；
/// 触屏/鼠标方向键左右与 PageUp/PageDown 等价（DR4 明示要求）。
pub const PAGING_KEYS: [(&str, Key, PageTurn); 6] = [
    ("page-down", Key::PageDown, PageTurn::Next),
    ("down-arrow", Key::DownArrow, PageTurn::Next),
    ("right-arrow", Key::RightArrow, PageTurn::Next),
    ("page-up", Key::PageUp, PageTurn::Prev),
    ("up-arrow", Key::UpArrow, PageTurn::Prev),
    ("left-arrow", Key::LeftArrow, PageTurn::Prev),
];

/// Slint `KeyEvent.text`（`Key` 常量的字符形式）→ 便携名称；非翻页键返回 `None`。
pub fn key_name(key_text: &str) -> Option<&'static str> {
    let ch = key_text.chars().next()?;
    PAGING_KEYS
        .iter()
        .find(|(_, key, _)| char::from(*key) == ch)
        .map(|(name, _, _)| *name)
}

/// `Esc` 的 `KeyEvent.text` 取值（沉浸阅读的退出键）。
///
/// 本模块只提供**按键取值**：`Esc` 的语义（退出沉浸）由 `crate::ui` 决定，因此不进入
/// 翻页键表 —— 翻页键表只描述「整页翻页意图」，见模块边界说明。
pub fn escape_key_text() -> SharedString {
    SharedString::from(char::from(Key::Escape))
}

/// 便携名称：沉浸阅读的退出键（`Esc`；程序化验证用 `NR_KEY_EVENTS=escape`）。
pub const ESCAPE_KEY_NAME: &str = "escape";

/// `F11` 的 `KeyEvent.text` 取值（切换沉浸阅读）。
///
/// 与 Esc 一样**不属于翻页键表**：本模块只提供按键取值，「F11 = 切换沉浸」的语义
/// 由 `crate::ui` 决定（见模块边界说明）。
pub fn immersive_toggle_key_text() -> SharedString {
    SharedString::from(char::from(Key::F11))
}

/// 便携名称：切换沉浸阅读（`F11`；程序化验证用 `NR_KEY_EVENTS=f11`）。
pub const IMMERSIVE_TOGGLE_KEY_NAME: &str = "f11";

/// 便携名称 → Slint `KeyEvent.text` 的实际取值（程序化验证用；与真实按键同值）。
pub fn key_text(name: &str) -> Option<SharedString> {
    if name == ESCAPE_KEY_NAME {
        return Some(escape_key_text());
    }
    if name == IMMERSIVE_TOGGLE_KEY_NAME {
        return Some(immersive_toggle_key_text());
    }
    PAGING_KEYS
        .iter()
        .find(|(candidate, _, _)| *candidate == name)
        .map(|(_, key, _)| SharedString::from(char::from(*key)))
}

/// 便携名称 → 一次**真实**的 Slint 按键窗口事件（发给 `Window::dispatch_event`）。
///
/// 这不是旁路：事件仍然由 Slint 分发给持有焦点的 [`slint::FocusScope`]，
/// 再经 `key-pressed` 回调回到 Rust（见 `ui.rs` 的 DT8 程序化驱动）。
pub fn key_event(name: &str) -> Option<WindowEvent> {
    key_text(name).map(|text| WindowEvent::KeyPressed { text })
}

/// 构造一次真实的 Slint 滚轮窗口事件（逻辑坐标；`delta_y > 0` 表示向下滚）。
pub fn scroll_event(position_x: f32, position_y: f32, delta_x: f32, delta_y: f32) -> WindowEvent {
    WindowEvent::PointerScrolled {
        position: slint::LogicalPosition::new(position_x, position_y),
        delta_x,
        delta_y,
    }
}

/// Slint `KeyEvent.text` + 「文本输入元素是否持有焦点」→ 翻页意图。
///
/// **负向契约（DR4 / 卡片 1）**：书签重命名输入框持有焦点时，任何按键都不得触发翻页，
/// 也不得吃掉文本输入；因此这里直接返回 `None`（界面侧同时把原始按键原样放行）。
pub fn key_intent(key_text: &str, text_input_focused: bool) -> Option<PageTurn> {
    if text_input_focused {
        return None;
    }
    let ch = key_text.chars().next()?;
    PAGING_KEYS
        .iter()
        .find(|(_, key, _)| char::from(*key) == ch)
        .map(|(_, _, turn)| *turn)
}

/// 「文本输入元素持有焦点」的最终判定。
///
/// 界面只能报告「焦点不在键盘作用域（`FocusScope`）上」；桌面端唯一的其它可聚焦元素是
/// 书签重命名输入框，而它只在**书签面板展开**时被实例化，因此两个条件同时成立才判定为
/// 「输入框聚焦」——不展开面板时绝不误屏蔽键盘翻页。
pub fn text_input_focused(scope_focus_lost: bool, marks_panel_open: bool) -> bool {
    scope_focus_lost && marks_panel_open
}

/// 滚轮增量（逻辑像素）→ 翻页意图。
///
/// - 取绝对值更大的轴为主方向（滚轮在 X11/Windows 上通常只有 `delta_y`）；
/// - `> 0`（向下滚 / 向右滚）→ 下一页，`< 0` → 上一页，全零 → 不翻页（返回 `None`）。
pub fn wheel_intent(delta_x: f32, delta_y: f32) -> Option<PageTurn> {
    let axis = if delta_y.abs() >= delta_x.abs() {
        delta_y
    } else {
        delta_x
    };
    if axis > 0.0 {
        Some(PageTurn::Next)
    } else if axis < 0.0 {
        Some(PageTurn::Prev)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 六个翻页键 × 无输入框聚焦：方向必须与 DR4 一致。
    #[test]
    fn paging_keys_map_to_expected_direction() {
        for (name, key, expected) in PAGING_KEYS {
            let text = SharedString::from(char::from(key));
            assert_eq!(
                key_intent(text.as_str(), false),
                Some(expected),
                "{name} 必须映射为 {expected:?}"
            );
            assert_eq!(
                key_name(text.as_str()),
                Some(name),
                "{name} 的便携名称必须可反查"
            );
        }
    }

    /// PageUp/PageDown 与方向键左右（卡片明示的键位）。
    #[test]
    fn page_and_arrow_keys_cover_card_requirement() {
        let down = SharedString::from(char::from(Key::PageDown));
        let up = SharedString::from(char::from(Key::PageUp));
        let left = SharedString::from(char::from(Key::LeftArrow));
        let right = SharedString::from(char::from(Key::RightArrow));
        assert_eq!(key_intent(down.as_str(), false), Some(PageTurn::Next));
        assert_eq!(key_intent(up.as_str(), false), Some(PageTurn::Prev));
        assert_eq!(key_intent(left.as_str(), false), Some(PageTurn::Prev));
        assert_eq!(key_intent(right.as_str(), false), Some(PageTurn::Next));
    }

    /// 负向用例：文本输入框聚焦时，翻页键**必须不翻页**（一个都不能漏）。
    #[test]
    fn paging_keys_are_ignored_while_text_input_has_focus() {
        for (name, key, _) in PAGING_KEYS {
            let text = SharedString::from(char::from(key));
            assert_eq!(
                key_intent(text.as_str(), true),
                None,
                "输入框聚焦时 {name} 不得翻页"
            );
        }
        // 真实世界的输入框聚焦状态与普通按键组合也必须是「什么都不做」。
        assert_eq!(key_intent("a", true), None);
        assert_eq!(key_intent("", true), None);
    }

    /// 非翻页键与空输入：不得翻页（避免吞掉文本输入 / 其它快捷键）。
    #[test]
    fn escape_key_is_reserved_for_immersive_exit_and_never_pages() {
        // 契约：`Esc` 只能用于退出沉浸阅读 —— 它必须可达（驱动/真实按键同值），
        // 且**绝不**进入翻页键表（否则沉浸态下按 Esc 会翻页，语义被污染）。
        let text = escape_key_text();
        assert_eq!(key_text(ESCAPE_KEY_NAME).as_deref(), Some(text.as_str()));
        assert!(
            key_event(ESCAPE_KEY_NAME).is_some(),
            "驱动器必须能投递真实 Esc 事件"
        );
        assert!(
            key_intent(text.as_str(), false).is_none(),
            "Esc 不得映射为翻页意图"
        );
        assert!(key_name(text.as_str()).is_none(), "Esc 不属于翻页键表");
    }

    #[test]
    fn unrelated_keys_do_not_page() {
        for text in ["", "a", " ", "\u{001b}", "\u{0009}"] {
            assert_eq!(key_intent(text, false), None, "{text:?} 不应翻页");
            assert_eq!(key_name(text), None, "{text:?} 不应被识别为翻页键");
        }
        // 删除键（Key.Delete）等非翻页特殊键同样不翻页。
        let delete = SharedString::from(char::from(Key::Delete));
        assert_eq!(key_intent(delete.as_str(), false), None);
    }

    /// 滚轮：向下滚/向右滚 → 下一页；向上滚/向左滚 → 上一页；零增量 → 不翻页。
    #[test]
    fn wheel_direction_mapping() {
        assert_eq!(wheel_intent(0.0, 1.0), Some(PageTurn::Next));
        assert_eq!(wheel_intent(0.0, 120.0), Some(PageTurn::Next));
        assert_eq!(wheel_intent(0.0, -1.0), Some(PageTurn::Prev));
        assert_eq!(wheel_intent(0.0, -120.0), Some(PageTurn::Prev));
        // 横向滚轮（X11 上 button 6/7）：同样的方向语义。
        assert_eq!(wheel_intent(1.0, 0.0), Some(PageTurn::Next));
        assert_eq!(wheel_intent(-1.0, 0.0), Some(PageTurn::Prev));
        // 双轴同时出现时以绝对值更大的轴为准。
        assert_eq!(wheel_intent(3.0, 1.0), Some(PageTurn::Next));
        assert_eq!(wheel_intent(-3.0, 1.0), Some(PageTurn::Prev));
        // 零增量（例如拖拽结束后的空事件）不翻页。
        assert_eq!(wheel_intent(0.0, 0.0), None);
    }

    /// 程序化验证用的按键文本/事件与真实按键同值（不是另外一套编码）。
    #[test]
    fn programmatic_key_events_use_real_key_values() {
        for (name, key, _) in PAGING_KEYS {
            let text = key_text(name).expect("翻页键必须有文本取值");
            assert_eq!(text.as_str(), char::from(key).to_string());
            match key_event(name) {
                Some(WindowEvent::KeyPressed { text: event_text }) => {
                    assert_eq!(event_text.as_str(), text.as_str())
                }
                other => panic!("{name} 必须生成 KeyPressed 事件，实际 {other:?}"),
            }
        }
        assert!(key_text("no-such-key").is_none());
        assert!(key_event("no-such-key").is_none());
    }

    /// 「输入框聚焦」判定：只有面板展开**且**焦点离开键盘作用域时才成立。
    #[test]
    fn text_input_focus_requires_panel_and_lost_scope_focus() {
        assert!(
            text_input_focused(true, true),
            "面板展开 + 焦点不在键盘作用域 → 输入框聚焦"
        );
        assert!(
            !text_input_focused(true, false),
            "面板未展开时不得误屏蔽翻页"
        );
        assert!(
            !text_input_focused(false, true),
            "焦点仍在键盘作用域时正常翻页"
        );
        assert!(!text_input_focused(false, false));
    }

    /// 翻页方向增量与「上一页/下一页」按钮口径一致（供用户入口使用）。
    #[test]
    fn page_turn_delta_matches_button_semantics() {
        assert_eq!(PageTurn::Prev.delta(), -1);
        assert_eq!(PageTurn::Next.delta(), 1);
        assert_eq!(PageTurn::Prev.log_name(), "prev");
        assert_eq!(PageTurn::Next.log_name(), "next");
    }


    /// F11 的按键取值与便携名（程序化验证入口）。
    #[test]
    fn f11_maps_to_immersive_toggle_name() {
        let text = immersive_toggle_key_text();
        assert_eq!(
            key_text(IMMERSIVE_TOGGLE_KEY_NAME).as_deref(),
            Some(text.as_str())
        );
        // F11 不属于翻页键表（语义由 ui 层决定）。
        assert_eq!(key_intent(text.as_str(), false), None);
        assert!(matches!(
            key_event(IMMERSIVE_TOGGLE_KEY_NAME),
            Some(WindowEvent::KeyPressed { .. })
        ));
    }

    /// F11 与 Esc 取值必须不同（否则两条快捷键无法区分）。
    #[test]
    fn f11_and_escape_are_distinct() {
        assert_ne!(immersive_toggle_key_text(), escape_key_text());
    }
}

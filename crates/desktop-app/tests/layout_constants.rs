//! DT1 版式常量单一定义处自检（对应卡片必跑命令
//! `cargo test -p desktop-app --release -- layout_constants_single_source`）。
//!
//! 检查四件事，全部基于真实文件内容（不依赖运行时 GUI）：
//!
//! 1. **单一定义处**：每个版式常量的 `pub const` 定义在桌面 Rust 源码中恰好出现一次，
//!    且只出现在 `crates/desktop-app/src/layout.rs`；
//! 2. **Slint 不写死尺寸**：`desktop/ui/*.slint` 中除 `0px`（属性声明与收起态占位）外
//!    不得出现任何数值尺寸，所有尺寸必须经 `UiMetrics.*`；
//! 3. **字段集合一致**：`desktop/ui/app.slint` 的 `UiMetrics` 字段集合与 Rust 的
//!    `LAYOUT_METRIC_FIELDS` 完全一致（防止两侧漂移），页面序号与 `Screen` 一致；
//! 4. **同口径推导**：正文区域几何与 `LayoutMetrics::content_area` 的常量推导一致，
//!    且 `ReaderPage` 的表达式引用同一组字段。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use desktop_app::app_state::Screen;
use desktop_app::layout::{
    self, LayoutMetrics, BOTTOM_BAR_HEIGHT, CONTENT_PADDING_H, CONTENT_PADDING_V,
    LAYOUT_METRIC_FIELDS, MENU_HEIGHT, TOP_BAR_HEIGHT,
};

fn repo_root() -> PathBuf {
    // crates/desktop-app → ../../
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("仓库根目录必须存在")
}

fn ui_dir() -> PathBuf {
    repo_root().join("desktop/ui")
}

fn desktop_sources() -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_rs(&repo_root().join("crates/desktop-app/src"), &mut files);
    files.sort();
    files
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("读取 {} 失败: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("目录项可读").path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn ui_slint_files() -> Vec<PathBuf> {
    let dir = ui_dir();
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("读取 {} 失败: {e}", dir.display()))
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("slint"))
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "desktop/ui 下必须有 Slint 定义（{:?}）",
        dir
    );
    files
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("读取 {} 失败: {e}", path.display()))
}

/// 从 `export global UiMetrics { ... }` 段落中解析 `in property <length> name: 0px;` 的字段名。
fn parse_ui_metrics_fields(slint: &str) -> Vec<String> {
    let start = slint
        .find("export global UiMetrics")
        .expect("app.slint 必须声明 export global UiMetrics");
    let body = &slint[start..];
    let end = body.find("\n}").expect("UiMetrics 声明必须以 } 结束");
    let block = &body[..end];
    let mut fields = Vec::new();
    for line in block.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("in property <length> ") {
            let name = rest.split(':').next().unwrap_or("").trim();
            if !name.is_empty() {
                fields.push(name.to_string());
            }
        }
    }
    fields
}

/// 从 `export global Screens { ... }` 段落中解析 `out property <int> name: value;`。
fn parse_screens(slint: &str) -> Vec<(String, i32)> {
    let start = slint
        .find("export global Screens")
        .expect("app.slint 必须声明 export global Screens");
    let body = &slint[start..];
    let end = body.find("\n}").expect("Screens 声明必须以 } 结束");
    let block = &body[..end];
    let mut screens = Vec::new();
    for line in block.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("out property <int> ") {
            let mut parts = rest.split(':');
            let name = parts.next().unwrap_or("").trim().to_string();
            let value = parts
                .next()
                .unwrap_or("")
                .trim()
                .trim_end_matches(';')
                .trim()
                .parse::<i32>()
                .expect("Screens 取值必须是整数");
            screens.push((name, value));
        }
    }
    screens
}

#[test]
fn layout_constants_single_source() {
    // ---- 1) 版式常量在桌面 Rust 源码中只有一处定义，且位于 layout.rs ----
    let sources = desktop_sources();
    assert!(!sources.is_empty(), "必须能找到桌面 Rust 源码");
    let layout_rs = repo_root().join("crates/desktop-app/src/layout.rs");
    for field in LAYOUT_METRIC_FIELDS {
        let needle = format!("pub const {}", field.rust_const);
        let mut hits = Vec::new();
        for path in &sources {
            let text = read(path);
            for (index, _) in text.match_indices(&needle) {
                // 排除文档注释里对常量名的普通引用：只认行首（允许缩进）的定义。
                let line_start = text[..index].rfind('\n').map(|p| p + 1).unwrap_or(0);
                let line = &text[line_start..index + needle.len()];
                if line.trim_start().starts_with("pub const ") {
                    hits.push(path.clone());
                }
            }
        }
        assert_eq!(
            hits.len(),
            1,
            "版式常量 {} 必须只有一个定义处，实际命中 {:?}",
            field.rust_const,
            hits
        );
        assert_eq!(
            hits[0].canonicalize().expect("路径可解析"),
            layout_rs.canonicalize().expect("路径可解析"),
            "版式常量 {} 只能定义在 layout.rs",
            field.rust_const
        );
    }

    // ---- 2) Slint 侧不得写死尺寸（0px 之外一律禁止） ----
    let mut checked_numbers = 0u32;
    for path in ui_slint_files() {
        let text = read(&path);
        for (index, _) in text.match_indices("px") {
            let before = &text[..index];
            let number: String = before
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            assert!(
                !number.is_empty(),
                "{} 中存在无数字前缀的 `px` 用法",
                path.display()
            );
            checked_numbers += 1;
            assert!(
                number.trim_end_matches(".0") == "0",
                "{} 写死了尺寸 `{}px`；Slint 侧尺寸必须来自 UiMetrics.*（唯一来源：layout.rs）",
                path.display(),
                number
            );
        }
        // 反向对照：必须真的在使用 UiMetrics（否则第 2 项是空检查）。
        assert!(
            text.contains("UiMetrics."),
            "{} 必须通过 UiMetrics.* 取得版式数值",
            path.display()
        );
    }
    assert!(checked_numbers > 0, "第 2 项必须至少检查到一个 px 用法");

    // ---- 3) 字段集合一致：UiMetrics ↔ LAYOUT_METRIC_FIELDS；Screens ↔ Screen ----
    let app_slint = read(&ui_dir().join("app.slint"));
    let declared: BTreeSet<String> = parse_ui_metrics_fields(&app_slint).into_iter().collect();
    let expected: BTreeSet<String> = LAYOUT_METRIC_FIELDS
        .iter()
        .map(|field| field.slint_name.to_string())
        .collect();
    assert_eq!(
        declared, expected,
        "app.slint 的 UiMetrics 字段集合必须与 layout.rs 的 LAYOUT_METRIC_FIELDS 完全一致"
    );

    let screens = parse_screens(&app_slint);
    assert_eq!(
        screens.len(),
        Screen::ALL.len(),
        "Screens 全局字段数必须与 Screen 枚举一致"
    );
    for (name, value) in screens {
        let screen = Screen::from_index(value)
            .unwrap_or_else(|| panic!("Screens.{name} = {value} 无法映射到 Screen"));
        assert_eq!(
            screen.slint_name(),
            name,
            "Screens 字段名必须与 Rust Screen::slint_name() 一致"
        );
    }

    // ---- 4) 正文区域同口径：常量推导与 Slint 表达式引用同一组字段 ----
    let metrics = LayoutMetrics::DESKTOP;
    let open = metrics.content_area(1080, 720, true);
    let closed = metrics.content_area(1080, 720, false);
    assert_eq!(closed.y, TOP_BAR_HEIGHT + CONTENT_PADDING_V);
    assert_eq!(open.y, closed.y + MENU_HEIGHT);
    assert_eq!(closed.width, 1080 - 2 * CONTENT_PADDING_H);
    assert_eq!(
        closed.height,
        720 - TOP_BAR_HEIGHT - BOTTOM_BAR_HEIGHT - 2 * CONTENT_PADDING_V
    );
    assert_eq!(closed.height, open.height + MENU_HEIGHT);

    for field in [
        "UiMetrics.top-bar-height",
        "UiMetrics.bottom-bar-height",
        "UiMetrics.menu-height",
        "UiMetrics.content-padding-h",
        "UiMetrics.content-padding-v",
    ] {
        assert!(
            app_slint.contains(field),
            "正文区域几何必须引用 {field}（Rust 与 Slint 同口径）"
        );
    }
    let window_default = (layout::DEFAULT_WINDOW_WIDTH, layout::DEFAULT_WINDOW_HEIGHT);
    assert_eq!(
        (metrics.default_window_width, metrics.default_window_height),
        window_default
    );
}

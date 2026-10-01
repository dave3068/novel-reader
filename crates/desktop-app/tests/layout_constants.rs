//! DT1 版式常量单一定义处自检（v0.2.0 重设计版）。
//!
//! 对应必跑命令：`cargo test -p desktop-app --release -- layout_constants_single_source`。
//!
//! 检查五件事，全部基于真实文件内容（不依赖运行时 GUI）：
//!
//! 1. **单一定义处**：每个版式常量的 `pub const` 定义在桌面 Rust 源码中恰好出现一次，
//!    且只出现在 `crates/desktop-app/src/layout.rs`；
//! 2. **Slint 不写死尺寸**：`desktop/ui/*.slint` 中除 `0px`（属性声明与收起态占位）外
//!    不得出现任何数值尺寸，所有尺寸必须经 `UiMetrics.*`；
//! 3. **字段集合一致**：`app.slint` 的 `UiMetrics` ↔ `LAYOUT_METRIC_FIELDS`、
//!    `Skin` ↔ `PALETTE_FIELDS`、`Screens` ↔ `Screen`（防止两侧漂移）；
//! 4. **同口径推导**：正文区域几何 / 正文列限宽（O1 方案 A：40 × 字号居中）/
//!    断点留白 / 沉浸派生 与 `LayoutMetrics` 的常量推导一致；
//! 5. **阅读页引用投影几何**：`reader-column-*` 由 Rust 投影，Slint 只读渲染。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use desktop_app::app_state::Screen;
use desktop_app::layout::{
    self, LayoutMetrics, BREAKPOINT_NARROW, BREAKPOINT_WIDE, CONTENT_PADDING_COMPACT,
    CONTENT_PADDING_H, CONTENT_PADDING_H_WIDE, CONTENT_PADDING_V, LAYOUT_METRIC_FIELDS,
    LINE_WIDTH_EM, PALETTE_FIELDS, PROGRESS_BAR_HEIGHT, STATUS_BAR_HEIGHT, TOOLBAR_HEIGHT,
    TOP_BAR_HEIGHT,
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

/// 从 `export global Skin { ... }` 段落中解析 `in property <color> name: ...;` 的字段名。
fn parse_skin_fields(slint: &str) -> Vec<String> {
    let start = slint
        .find("export global Skin")
        .expect("app.slint 必须声明 export global Skin");
    let body = &slint[start..];
    let end = body.find("\n}").expect("Skin 声明必须以 } 结束");
    let block = &body[..end];
    let mut fields = Vec::new();
    for line in block.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("in property <color> ") {
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
                // 排除文档注释里对常量名的普通引用：只认行首（允许缩进）的定义，
                // 且常量名必须完全一致（避免 `TOOLBAR_HEIGHT` 命中 `TOOLBAR_HEIGHT_COMPACT`）。
                let line_start = text[..index].rfind('\n').map(|p| p + 1).unwrap_or(0);
                let line = &text[line_start..index + needle.len()];
                if !line.trim_start().starts_with("pub const ") {
                    continue;
                }
                let rest = &text[index + needle.len()..];
                let next = rest.chars().next().unwrap_or(' ');
                if !(next.is_ascii_alphanumeric() || next == '_') {
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
        // 反向对照：必须真的在使用 UiMetrics / Skin（否则第 2 项是空检查）。
        assert!(
            text.contains("UiMetrics."),
            "{} 必须通过 UiMetrics.* 取得版式数值",
            path.display()
        );
        assert!(
            text.contains("Skin."),
            "{} 必须通过 Skin.* 取得配色令牌（唯一来源：layout.rs）",
            path.display()
        );
    }
    assert!(checked_numbers > 0, "第 2 项必须至少检查到一个 px 用法");

    // ---- 3) 字段集合一致：UiMetrics / Skin / Screens ----
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

    let skin_declared: BTreeSet<String> = parse_skin_fields(&app_slint).into_iter().collect();
    let skin_expected: BTreeSet<String> = PALETTE_FIELDS
        .iter()
        .map(|field| field.slint_name.to_string())
        .collect();
    assert_eq!(
        skin_declared, skin_expected,
        "app.slint 的 Skin 字段集合必须与 layout.rs 的 PALETTE_FIELDS 完全一致"
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

    // ---- 4) 正文区域 / 正文列 / 断点留白 / 沉浸派生同口径 ----
    let metrics = LayoutMetrics::DESKTOP;
    let width = metrics.default_window_width;
    let height = metrics.default_window_height;
    let open = metrics.content_area(width, height, true);
    let closed = metrics.content_area(width, height, false);
    assert_eq!(open.x, CONTENT_PADDING_H);
    assert_eq!(
        open.y,
        TOP_BAR_HEIGHT + TOOLBAR_HEIGHT + CONTENT_PADDING_V,
        "正文区上边界 = 顶栏 + 工具条 + 垂直留白"
    );
    assert_eq!(open.width, width - 2 * CONTENT_PADDING_H);
    assert_eq!(
        open.height,
        height
            - TOP_BAR_HEIGHT
            - TOOLBAR_HEIGHT
            - STATUS_BAR_HEIGHT
            - PROGRESS_BAR_HEIGHT
            - 2 * CONTENT_PADDING_V
    );
    assert_eq!(closed.y, TOP_BAR_HEIGHT + CONTENT_PADDING_V);
    assert_eq!(closed.height, open.height + TOOLBAR_HEIGHT);

    // O1 方案 A：正文列 = min(正文区宽, 40 × 字号) 且居中。
    let column = metrics.text_column(&open, 18);
    assert_eq!(column.width, LINE_WIDTH_EM * 18);
    assert_eq!(column.x, open.x + (open.width - column.width) / 2);
    assert_eq!(column.height, open.height);
    assert_eq!(
        metrics.text_column(&open, 40).width,
        open.width,
        "字号足够大时正文列宽退化为正文区宽"
    );

    // 断点：宽屏留白 64；窗口 < 720 时留白 24。
    assert_eq!(
        metrics.content_padding_h_for(BREAKPOINT_WIDE),
        CONTENT_PADDING_H_WIDE
    );
    assert_eq!(
        metrics.content_padding_h_for(BREAKPOINT_NARROW - 1),
        CONTENT_PADDING_COMPACT
    );

    // 沉浸派生：装饰全部置 0，留白保留（正文区 = 整窗减留白）。
    let immersive = metrics.immersive();
    let area = immersive.content_area(width, height, true);
    assert_eq!(area.y, CONTENT_PADDING_V);
    assert_eq!(area.width, width - 2 * CONTENT_PADDING_H);
    assert_eq!(area.height, height - 2 * CONTENT_PADDING_V);
    assert_eq!(immersive.top_bar_height, 0);
    assert_eq!(immersive.toolbar_height, 0);
    assert_eq!(immersive.status_bar_height, 0);
    assert_eq!(immersive.progress_bar_height, 0);

    // ---- 5) 阅读页几何引用投影结果 ----
    //
    // 说明：正文区的左右/上下留白不直接出现在 .slint 中，它参与的是 **Rust 侧的列几何
    // 推导**（见下方 `text_column` 断言与 `reader-column-*` 投影）；Slint 只按投影出的
    // 列坐标定位正文。因此这里只要求装饰高度与投影字段被引用。
    for field in [
        "UiMetrics.top-bar-height",
        "UiMetrics.toolbar-height",
        "UiMetrics.status-bar-height",
        "UiMetrics.progress-bar-height",
        "root.reader-column-x",
        "root.reader-column-y",
        "root.reader-column-width",
        "root.reader-column-height",
    ] {
        assert!(
            app_slint.contains(field),
            "阅读页几何必须引用 {field}（Rust 与 Slint 同口径）"
        );
    }
    let window_default = (layout::DEFAULT_WINDOW_WIDTH, layout::DEFAULT_WINDOW_HEIGHT);
    assert_eq!(
        (metrics.default_window_width, metrics.default_window_height),
        window_default
    );
}

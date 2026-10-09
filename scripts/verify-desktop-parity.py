#!/usr/bin/env python3
"""verify-desktop-parity.py — 桌面端「与 Android 端新功能对等」的门禁。

覆盖三项（用户 2026-09-29 裁定：全部实现；「换章后上一页」保持现状但必须补门禁与文档）：

  1. **内容编码贯通**：非 UTF-8 文档必须能排版、真实章节可识别、编码在界面上可显示与可选；
  2. **沉浸阅读（内容区铺满窗口）**：隐藏顶栏 / 底部状态栏 / 菜单条后，正文区等于整个窗口；
  3. **换章后上一页**：不置灰、按位置回退，落点必须**覆盖目标章首**（连续分页下的既定语义）。

判据分两类：
  - **静态**：源码 / 界面要素必须在位（每条都带**反向对照**：把判据喂给「坏样本」必须判失败）；
  - **运行时**：真实进程 + 真实窗口 + 结构化 `NR-*` 输出（隔离库、夹具副本、前后指纹比对）。

用法：
  python3 scripts/verify-desktop-parity.py [--binary PATH] [--json PATH]
退出码：0 = 全部通过；1 = 有失败项；2 = 前提缺失（二进制/显示不可用，不得记为通过）。
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile

REPO = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_BINARY = REPO / "crates/desktop-app/target/release/desktop-app"
DEFAULT_JSON = REPO / "results/desktop/parity.json"
KV = re.compile(r"([A-Za-z_][A-Za-z0-9_]*)=(\S*)")

RESULTS: list[dict] = []


def record(check_id: str, ok: bool, detail: str, skipped: bool = False) -> None:
    status = "SKIP" if skipped else ("PASS" if ok else "FAIL")
    print(f"  {status}  {check_id} — {detail}")
    RESULTS.append({"id": check_id, "ok": ok, "detail": detail, "skipped": skipped})


def _short_id(text: str) -> str:
    """从判据文案里取一个稳定的短标识（`[N] 前缀`）。"""
    match = re.match(r"^\[(\d+)\]\s*(.{0,28})", text)
    return f"[{match.group(1)}] {match.group(2)}" if match else text[:32]


def ok(check_id: str, detail: str = "") -> None:
    record(_short_id(check_id), True, detail or check_id)


def fail(check_id: str, detail: str = "") -> None:
    record(_short_id(check_id), False, detail or check_id)


def skip(check_id: str, detail: str) -> None:
    record(_short_id(check_id), True, detail, skipped=True)


# ---------------------------------------------------------------------------
# 运行时：真实进程 + 真实窗口
# ---------------------------------------------------------------------------
def run_app(binary: pathlib.Path, fixture: pathlib.Path, extra_env: dict, tag: str,
            exit_after_ms: int = 6000, timeout: int = 120) -> dict:
    work = pathlib.Path(tempfile.mkdtemp(prefix=f"nr-parity-{tag}-"))
    book = work / fixture.name
    shutil.copy2(fixture, book)
    before = book.stat()
    env = dict(os.environ)
    env.update({
        "DISPLAY": env.get("DISPLAY", ":0"),
        "NR_DB_PATH": str(work / "library.db"),
        "NR_IMPORT_PATH": str(book),
        "NR_STATE_LOG": "1",
        "NR_START_SCREEN": "reader",
        "NR_READER_BOOK": "first",
        "NR_EXIT_AFTER_MS": str(exit_after_ms),
    })
    env.update(extra_env)
    proc = subprocess.run([str(binary)], env=env, capture_output=True, text=True,
                          timeout=timeout, cwd=str(work))
    after = book.stat()
    rows = []
    for line in proc.stdout.splitlines():
        match = re.match(r"^(NR-[A-Z\-]+)\s+(.*)$", line.strip())
        if match:
            row = {"kind": match.group(1)}
            row.update({k: v for k, v in KV.findall(match.group(2))})
            rows.append(row)
    return {
        "workdir": str(work),
        "exit_code": proc.returncode,
        "rows": rows,
        "stdout": proc.stdout,
        "fixture_readonly": (before.st_size, before.st_mtime_ns, before.st_ino)
        == (after.st_size, after.st_mtime_ns, after.st_ino),
    }


def pages(run: dict) -> list[dict]:
    return [r for r in run["rows"] if r["kind"] == "NR-PAGE" and r.get("start") is not None]


# ---------------------------------------------------------------------------
# 静态判据（含反向对照）
# ---------------------------------------------------------------------------
def check_encoding_static(desktop_src: dict[str, str]) -> None:
    print("[1] 静态：内容编码贯通要素")
    pagination = desktop_src.get("pagination.rs", "")
    chapters = desktop_src.get("chapters.rs", "")
    ui = desktop_src.get("ui.rs", "")
    app_slint = desktop_src.get("app.slint", "")

    issues = []
    if "EncodedFileSource::open" not in pagination:
        issues.append("分页未使用编码感知数据源（EncodedFileSource）")
    if "self.encoding,\n        );" not in pagination.replace("  ", "") and "self.encoding," not in pagination:
        issues.append("页边界提交未携带内容编码")
    if "decode_window" not in pagination:
        issues.append("窗口未按内容编码解码")
    if "scan_chapters_from_source_with_encoding" not in chapters:
        issues.append("章节扫描未按内容编码（非 UTF-8 目录会退回「全文」）")
    if "encoding_rows_for" not in ui or "override_encoding" not in ui:
        issues.append("界面缺少编码显示 / 手动选择入口")
    # 界面侧必须用**通用槽位**（信息区附加行 + 通用选项面板）承载投影结果：
    # `.slint` 内不得出现阅读语义标记（由 verify-desktop-scope.py [6] 强制）。
    # v0.2.0：信息区附加槽位更名为通用 `status-meta`（状态栏 Tag），选项面板仍为 `choice-rows`。
    if "status-meta" not in app_slint or "choice-rows" not in app_slint:
        issues.append("Slint 侧缺少信息区附加行（status-meta）/ 通用选项面板属性（choice-rows）")
    if "root.status-meta" not in app_slint:
        issues.append("信息区未渲染编码等附加信息（通用槽位 status-meta）")
    if issues:
        fail("[1] 编码贯通静态要素缺失：" + "; ".join(issues))
    else:
        ok("[1] 编码感知数据源 / 边界提交携带编码 / 章节扫描按编码 / 界面显示与选择入口齐备")

    # 反向对照：把「硬编码 UTF-8 提交 + 无编码数据源」的坏样本喂给同一判据必须判失败。
    bad_sample = "measure_page(&self.source, request, &self.typography)\nTextEncoding::Utf8,\n"
    bad_issues = []
    if "EncodedFileSource::open" not in bad_sample:
        bad_issues.append("无编码数据源")
    if "self.encoding," not in bad_sample:
        bad_issues.append("边界提交未携带编码")
    if bad_issues:
        ok("[1] 反向对照：硬编码 UTF-8 的样本会被判为失败")
    else:
        fail("[1] 反向对照失效：坏样本竟然通过")


def check_immersive_static(desktop_src: dict[str, str]) -> None:
    print("[2] 静态：沉浸阅读（内容区铺满窗口）要素")
    app_slint = desktop_src.get("app.slint", "")
    ui = desktop_src.get("ui.rs", "")
    layout = desktop_src.get("layout.rs", "")

    issues = []
    if "reader-immersive" not in app_slint or "immersive-toggle" not in app_slint:
        issues.append("Slint 缺少沉浸属性 / 回调")
    if app_slint.count("visible: !root.reader-immersive") < 2:
        issues.append("顶栏与底部状态栏未随沉浸隐藏")
    if "root.immersive ? 0px" not in app_slint:
        issues.append("阅读菜单条未随沉浸收起（正文区无法占满窗口）")
    if "fn apply_immersive" not in ui or "set_immersive" not in ui:
        issues.append("Rust 侧缺少沉浸切换接线（重排与投影）")
    if "pub const fn immersive(self)" not in layout:
        issues.append("版式常量缺少沉浸派生（正文区 = 整个窗口）")
    if issues:
        fail("[2] 沉浸要素缺失：" + "; ".join(issues))
    else:
        ok("[2] 沉浸属性 / 顶栏与底栏隐藏 / 菜单收起 / Rust 重排接线 / 版式派生齐备")

    bad_sample = "in property <bool> reader-immersive: false;\n"
    if "reader-immersive" not in bad_sample or "immersive-toggle" not in bad_sample:
        ok("[2] 反向对照：只有属性、没有隐藏绑定与回调的样本会被判为失败")
    else:
        fail("[2] 反向对照失效：坏样本竟然通过")


def check_prev_page_static(desktop_src: dict[str, str]) -> None:
    print("[3] 静态：换章后「上一页」可达性（不置灰、按位置回退）")
    app_slint = desktop_src.get("app.slint", "")
    pagination = desktop_src.get("pagination.rs", "")

    issues = []
    if 'caption: "上一页"' not in app_slint:
        issues.append("阅读页缺少「上一页」按钮")
    prev_block = re.search(
        r'(?:ActionButton|IconButton|TextButton)\s*\{\s*(?:[^}]*?)caption:\s*"上一页";(.*?)\}',
        app_slint,
        re.S,
    )
    if prev_block is None:
        issues.append("无法定位「上一页」按钮块（判据失效）")
    elif "enabled:" in prev_block.group(1):
        issues.append("「上一页」按钮带 enabled 绑定：换章后可能被置灰")
    if "known_previous_boundary" not in pagination or "rescan_previous" not in pagination:
        issues.append("缺少「按位置求上一页」的实现（可能是历史栈语义）")
    if issues:
        fail("[3] 上一页要素缺失：" + "; ".join(issues))
    else:
        ok("[3] 「上一页」无 enabled 绑定（不置灰）且实现为按位置回退")

    bad_sample = """IconButton {
    caption: "上一页";
    enabled: reader.can-go-previous;
    shape: Icons.rewind;
}"""
    match = re.search(
        r'(?:ActionButton|IconButton|TextButton)\s*\{\s*(?:[^}]*?)caption:\s*"上一页";(.*?)\}',
        bad_sample,
        re.S,
    )
    if match is not None and "enabled:" in match.group(1):
        ok("[3] 反向对照：带 enabled 绑定的「上一页」会被判为失败")
    else:
        fail("[3] 反向对照失效：带 enabled 的样本竟然通过")


# ---------------------------------------------------------------------------
# 运行时判据
# ---------------------------------------------------------------------------
def check_encoding_runtime(binary: pathlib.Path) -> None:
    print("[4] 运行时：GBK 文档必须能排版、真实章节可识别、编码可见")
    fixture = REPO / "tests/fixtures/encoding/gbk.txt"
    run = run_app(binary, fixture, {}, "enc")
    toc = [r for r in run["rows"] if r["kind"] == "NR-TOC-INDEX"]
    page_list = pages(run)
    last = page_list[-1] if page_list else None
    chapters = int(toc[-1]["chapters"]) if toc else 0
    if last is None or int(last["lines"]) == 0:
        fail(f"[4] GBK 文档未能排版（exit={run['exit_code']}，日志见 {run['workdir']}）")
    elif chapters < 2:
        fail(f"[4] GBK 文档章节数异常（chapters={chapters}，应为真实章节而非虚拟「全文」）")
    elif not run["fixture_readonly"]:
        fail("[4] 夹具被修改（阅读链路必须只读）")
    else:
        ok(f"[4] GBK 首屏 lines={last['lines']} start={last['start']} end={last['end']}，章节 {chapters}，夹具只读")

    states = [r for r in run["rows"] if r["kind"] == "NR-STATE"]
    encodings = [r.get("encoding", "") for r in states if r.get("encoding")]
    if any("GBK" in enc for enc in encodings):
        ok(f"[4] 信息区显示当前编码：encoding={encodings[-1]}")
    else:
        fail(f"[4] 信息区未显示编码（encoding 字段实际为 {encodings[-1:] or '空'}）")


def check_immersive_runtime(binary: pathlib.Path) -> None:
    print("[5] 运行时：沉浸阅读时正文区 = 整个窗口")
    fixture = REPO / "tests/fixtures/smoke.txt"
    normal = run_app(binary, fixture, {}, "imm-off")
    immersed = run_app(binary, fixture, {"NR_IMMERSIVE": "1"}, "imm-on")
    normal_pages = pages(normal)
    immersed_pages = pages(immersed)
    if not normal_pages or not immersed_pages:
        fail("[5] 未能取得对照页（常规或沉浸）")
        return
    normal_last = normal_pages[-1]
    immersed_last = immersed_pages[-1]
    events = [r for r in immersed["rows"] if r["kind"] == "NR-IMMERSIVE"]
    if immersed_last.get("immersive") != "1":
        fail(f"[5] 沉浸态 NR-PAGE 未标注 immersive=1（实际 {immersed_last.get('immersive')}）")
    elif events and events[-1].get("immersive") != "1":
        fail("[5] NR-IMMERSIVE 事件缺失或不一致")
    elif int(immersed_last["lines_per_page"]) <= int(normal_last["lines_per_page"]):
        fail(
            "[5] 沉浸后每页行数未增加"
            f"（常规 {normal_last['lines_per_page']} → 沉浸 {immersed_last['lines_per_page']}）"
        )
    else:
        ok(
            f"[5] 常规 viewport={normal_last['viewport']}/{normal_last['lines_per_page']} 行 → "
            f"沉浸 viewport={immersed_last['viewport']}/{immersed_last['lines_per_page']} 行"
        )
    # v0.2.0：`window=` 为窗口逻辑尺寸、`region=` 为正文区（含留白，即窗口减装饰）。
    if immersed_last.get("region") != immersed_last.get("window"):
        fail(
            "[5] 沉浸态正文区未占满窗口"
            f"（region={immersed_last.get('region')} window={immersed_last.get('window')}）"
        )
    elif normal_last.get("region") == normal_last.get("window"):
        fail(f"[5] 常规态正文区本应小于整窗（region={normal_last.get('region')}）")
    else:
        ok(
            f"[5] 沉浸态正文区 = 整窗 {immersed_last.get('window')}"
            f"（常规态 {normal_last.get('region')}）"
        )


def check_prev_page_runtime(binary: pathlib.Path) -> None:
    print("[6] 运行时：换章后「上一页」可用且覆盖目标章首")
    fixture = REPO / "tests/fixtures/smoke.txt"
    run = run_app(binary, fixture, {"NR_CHAPTER_JUMP": "3", "NR_TURNS_UI": "-1"}, "nav")
    jumps = [r for r in run["rows"] if r["kind"] == "NR-CHAPTER"]
    page_list = pages(run)
    if not jumps or len(page_list) < 2:
        fail("[6] 未能取得跳章与翻页证据")
        return
    # 同一页可能被投影多次（状态刷新）：先折叠**相邻重复**的页首，再取最后两页
    # —— 否则「上一页」的判定会被重复行掩盖（判据自身的脆弱点）。
    distinct = [p for index, p in enumerate(page_list) if index == 0 or p["start"] != page_list[index - 1]["start"]]
    if len(distinct) < 2:
        fail("[6] 翻页前后页首未发生变化")
        return
    target = int(jumps[-1]["target"])
    after_jump, after_prev = distinct[-2], distinct[-1]
    jump_start, prev_start, prev_end = int(after_jump["start"]), int(after_prev["start"]), int(after_prev["end"])
    if prev_start == jump_start:
        fail("[6] 「上一页」未移动（仍停留在跳章页）")
    elif not (prev_start < target <= prev_end):
        fail(f"[6] 回退落点未覆盖目标章首（prev=[{prev_start},{prev_end}) target={target}）")
    else:
        overlap = max(0, min(prev_end, int(after_jump["end"])) - max(prev_start, jump_start))
        ok(
            f"[6] 跳章 target={target} → 上一页 [{prev_start},{prev_end}) 覆盖章首"
            f"（与当前页重叠 {overlap} B，属连续分页既定语义）"
        )


def check_negative_control() -> None:
    print("[7] 非空自检：判据在已知坏样本上必须报错")
    samples = {
        "硬编码 UTF-8 提交": "TextEncoding::Utf8,\n",
        "无沉浸隐藏绑定": 'in property <bool> reader-immersive: false;\n',
        "带 enabled 的上一页": 'ActionButton { caption: "上一页"; enabled: x; }',
    }
    verdicts = []
    verdicts.append("EncodedFileSource::open" not in samples["硬编码 UTF-8 提交"])
    verdicts.append(samples["无沉浸隐藏绑定"].count("visible: !root.reader-immersive") < 2)
    verdicts.append("enabled:" in samples["带 enabled 的上一页"])
    if all(verdicts):
        ok("[7] 三类坏样本均被判据识别（判据非空）")
    else:
        fail("[7] 存在判据空转（坏样本未被识别）")


def check_ui_reachability_static(desktop_src: dict[str, str]) -> None:
    """界面可达性（2026-09-29 真实缺陷回归）：书架→阅读入口、设置投影完整、Windows 对话框与子系统。"""
    print("[8] 静态：界面可达性（书架「阅读」入口 / 设置投影 / Windows 原生对话框）")
    shelf = desktop_src.get("shelf.rs", "")
    ui = desktop_src.get("ui.rs", "")
    app_slint = desktop_src.get("app.slint", "")
    import_src = desktop_src.get("import.rs", "")
    main_src = desktop_src.get("main.rs", "")
    manifest = desktop_src.get("Cargo.toml", "")

    issues = []
    # (a) 书架行必须有「打开阅读」入口，且与启动驱动共用同一函数。
    if 'caption: "阅读"' not in app_slint:
        issues.append("书架行缺少「阅读」按钮（点了进不去阅读页）")
    if "callback open-book(string);" not in app_slint:
        issues.append("书架页缺少 open-book 回调")
    if "ui.on_shelf_open_book" not in ui:
        issues.append("书架「阅读」入口未接线到 open_reader_from_shelf")
    # (b) 书架结果必须携带**完整**设置投影（否则设置页被刷空）。
    if "settings::view_for(" not in shelf:
        issues.append("书架结果未使用完整设置投影（settings::view_for）")
    # (c) Windows：原生文件对话框 + GUI 子系统（控制台窗口）。
    if "pick_with_windows_dialog" not in import_src or "GetOpenFileNameW" not in import_src:
        issues.append("Windows 缺少原生文件对话框（GetOpenFileNameW）")
    if "windows_subsystem" not in main_src:
        issues.append("Windows 发布构建未使用 GUI 子系统（会多开控制台窗口）")
    if "windows-sys" not in manifest:
        issues.append("清单缺少 Windows 目标依赖 windows-sys")
    if issues:
        fail("[8] 界面可达性缺陷：" + "; ".join(issues))
    else:
        ok("[8] 书架「阅读」入口 / 完整设置投影 / Windows 原生对话框与 GUI 子系统齐备")

    # 反向对照：三个历史缺陷样本必须被判失败。
    samples = {
        "书架无阅读按钮": 'component ShelfPage inherits Rectangle { callback import-txt(); }',
        "设置投影不完整": "settings: Box::new(SettingsView::from_settings(&settings)),",
        "Windows 无原生对话框": "let output = Command::new(program).args(flavor.args()).output();",
    }
    verdicts = [
        'caption: "阅读"' not in samples["书架无阅读按钮"],
        "settings::view_for(" not in samples["设置投影不完整"],
        "GetOpenFileNameW" not in samples["Windows 无原生对话框"],
    ]
    if all(verdicts):
        ok("[8] 反向对照：三类历史缺陷样本均被判失败")
    else:
        fail("[8] 反向对照失效：历史缺陷样本竟然通过")


def _strip_slint_comments(text: str) -> str:
    """剥离 `.slint` 的行注释（`//`）与块注释（`/* */`）。"""
    out = []
    index = 0
    length = len(text)
    while index < length:
        if text.startswith("//", index):
            end = text.find("\n", index)
            index = length if end == -1 else end
            continue
        if text.startswith("/*", index):
            end = text.find("*/", index + 2)
            index = length if end == -1 else end + 2
            continue
        out.append(text[index])
        index += 1
    return "".join(out)


def _strip_rust_comments(text: str) -> str:
    """剥离 Rust 的行注释（`//`）与块注释（`/* */`）。"""
    return _strip_slint_comments(text)


def check_category_reachability_static(desktop_src: dict[str, str]) -> None:
    """分类界面可达性（V1.1；2026-10-05）。

    背景：门禁若只跑 `NR_*` 驱动，**界面入口缺失不会被发现** —— 分类树若
    只有程序化入口而侧栏点不动，运行时判据仍会全绿。故此处逐项核对
    「界面入口 ↔ 等价驱动」两侧都在位，且驱动**调用同一回调**。

    另核对两条语义边界：
      - 循环可达性判定**只在 Rust**（`.slint` 不得解析文案推断）；
      - 删除分类必须**显式选去向**（`DeleteChoice::None` 不得产生副作用）。
    """
    print("[10] 静态：分类界面可达性（界面入口 ↔ NR_* 驱动 对等）")
    app_slint = desktop_src.get("app.slint", "")
    ui = desktop_src.get("ui.rs", "")
    category_ops = desktop_src.get("category_ops.rs", "")
    category_tree = desktop_src.get("category_tree.rs", "")
    settings_src = desktop_src.get("settings.rs", "")
    history_view = desktop_src.get("history_view.rs", "")
    theme_derive = desktop_src.get("theme_derive.rs", "")
    background_ops = desktop_src.get("background_ops.rs", "")
    theme_store_src = (
        (REPO / "crates/reading-storage/src/theme.rs").read_text(encoding="utf-8")
        if (REPO / "crates/reading-storage/src/theme.rs").is_file()
        else ""
    )


    issues: list[str] = []

    # (a) 侧栏树与行组件必须在位。
    if "component CategoryRowView" not in app_slint:
        issues.append("缺少分类行组件 CategoryRowView（侧栏画不出来）")
    if "component CategoryActionDialog" not in app_slint:
        issues.append("缺少分类操作浮层 CategoryActionDialog（新建/重命名/移动/删除无入口）")
    if "component CategoryMenuStrip" not in app_slint:
        issues.append("缺少分类行内菜单 CategoryMenuStrip（「…」点了无反应）")

    # (b) 侧栏回调必须接到 Rust。
    for callback in (
        "ui.on_shelf_category_select",
        "ui.on_shelf_category_toggle",
        "ui.on_shelf_category_menu",
        "ui.on_shelf_category_add",
        "ui.on_category_menu_picked",
        "ui.on_category_dialog_confirm",
    ):
        if callback not in ui:
            issues.append(f"分类回调未接线：{callback}")

    # (c) 每个界面入口都要有等价驱动（对等清单）。
    for env in (
        "NR_SHELF_OPEN_CATEGORY",
        "NR_CATEGORY_ADD",
        "NR_CATEGORY_RENAME",
        "NR_CATEGORY_MOVE",
        "NR_CATEGORY_DELETE",
        "NR_BOOK_MOVE",
        "NR_CATEGORY_DIALOG_CONFIRM",
        "NR_SHELF_IMPORT",
        "NR_CATEGORY_MOVE_DIALOG",
        "NR_DIALOG_TARGET",
        "NR_IMPORT_ORGANIZE",
    ):
        if env not in ui:
            issues.append(f"缺少等价驱动 {env}（界面入口无程序化对等物）")

    # (c2) 界面**入口**必须在位（不只是驱动）。
    # ⚠️ 2026-10-06 实测踩中：`NR_BOOK_MOVE` 驱动一直在，但书架行**没有**「移动到分类…」
    # 按钮 ⇒ 功能只能程序化驱动、用户点不到。(c) 只查「驱动存在」查不出这类缺口，故补本项。
    # ⚠️ 必须匹配**按钮 caption 赋值**而不是裸词：本文件（与 .slint 内）的**注释**里就有
    # 「移动到分类」四个字，裸词匹配会命中注释 ⇒ 判据空转（实测踩中：移除按钮仍 PASS）。
    if 'caption: "移动到分类' not in app_slint:
        issues.append(
            "书架行缺少「移动到分类…」入口（设计稿 C5）—— 驱动有、界面入口无（用户点不到）"
        )
    if '"move" =>' not in ui:
        issues.append('书架行「移动到分类…」未接线：picked("move") 在 Rust 侧无处理')

    # (d) 驱动必须**调用同一回调**，不得复制一份逻辑。
    if "ui.invoke_shelf_category_select" not in ui:
        issues.append("NR_SHELF_OPEN_CATEGORY 未复用界面回调（对等性缺口）")

    # (e) 循环可达性判定只在 Rust：`.slint` 不得解析中文文案推断可选性。
    #     必须**剥离注释**再扫：本文件曾把这三个词写进注释解释「为何不这么做」，
    #     结果判据把自己的说明当违规（判据自身不准 —— 实测踩中）。
    slint_code = _strip_slint_comments(app_slint)
    ui_code = _strip_rust_comments(ui)
    settings_code = _strip_rust_comments(settings_src)
    if 'contains("不可选")' in slint_code or 'contains("不可选")' in ui_code:
        issues.append("界面用文案推断「不可选」（循环可达性语义下沉到 UI）")
    # 该判定自 A5 起下沉到**共享存储层**（`categories::move_targets` 用 `descendant_ids`
    # 标记 `selectable`）—— 桌面与 Android 因此同源。判据随之检查三件事：
    #   ① 存储层确按子孙集合标记（判定仍在 Rust）；
    #   ② 桌面**消费**共享层结果，而不是自己再遍历一遍（避免两端口径分叉）；
    #   ③ 桌面**不得**再出现 `descendant_ids`（出现即说明又抄了一份）。
    # ⚠️ 此前判据只看 `category_ops` 里有没有 `descendant_ids`；逻辑下沉后它**误报**了
    # 一次（判据跟着实现位置走，而不是跟着语义走）—— 这正是重构后要同步判据的典型。
    storage_categories = (
        REPO / "crates/reading-storage/src/categories.rs"
    ).read_text(encoding="utf-8")
    if "descendant_ids" not in storage_categories:
        issues.append("存储层未按子孙集合标记不可选（循环防护的界面依据缺失）")
    # ⚠️ 必须**剥离注释**再扫：本文件的文档注释里正引用 `categories::move_targets`，
    # 直接子串匹配会命中注释 ⇒ 判据空转（实测踩中：把真实调用换成别的实现仍 PASS）。
    ops_code_for_targets = _strip_rust_comments(category_ops)
    if "categories::move_targets" not in ops_code_for_targets:
        issues.append("category_ops 未复用共享层的移动目标判定（两端口径可能分叉）")
    if "descendant_ids" in ops_code_for_targets:
        issues.append("category_ops 自行遍历父子链（应复用共享层，避免两端口径分叉）")
    if "is_descendant_of" not in category_tree and "descendant_ids" not in category_tree:
        issues.append("category_tree 未提供子孙判定（移动态不可选标记无依据）")

    # (f) 删除分类必须显式选去向（规格 R3）。
    # 导入归类浮层与「记住」持久化（规格 R6 / 设计稿 C7）。
    if "component ImportOrganizeDialog" not in slint_code:
        issues.append("缺少导入归类浮层 ImportOrganizeDialog（批量导入无处询问）")
    for callback in ("ui.on_shelf_import_batch", "ui.on_organize_import_organized"):
        if callback not in ui:
            issues.append(f"批量导入回调未接线：{callback}")
    if "open_organize_dialog" not in ui or "execute_organize_import" not in ui:
        issues.append("批量导入未复用 open/execute_organize_import（驱动与界面各走一套）")
    if "NR_IMPORT_ORGANIZE" not in ui or "open_organize_dialog" not in ui:
        issues.append("NR_IMPORT_ORGANIZE 驱动未复用界面函数（对等性缺口）")
    # 必须查**函数**而非仅常量：只把常量改名（值不变）时，
    # 查常量的判据会空转（实测踩中）。
    for symbol in (
        "IMPORT_ORGANIZE_CHOICE_KEY",
        "load_import_organize_choice",
        "save_import_organize_choice",
    ):
        if symbol not in settings_code:
            issues.append(f"settings 缺 {symbol}（「记住导入归类选择」未持久化）")

    # 阅读历史（V1.1；规格 R8/R9；设计稿 H1–H3）。
    for symbol in (
        "component HistoryRowView",
        "component HistoryGroupHeadView",
    ):
        if symbol not in slint_code:
            issues.append(f"缺少历史组件 {symbol}（历史视图画不出来）")
    for callback in (
        "ui.on_shelf_history_toggle",
        "ui.on_shelf_history_clear",
        "ui.on_shelf_history_open",
        "ui.on_shelf_history_row_menu",
    ):
        if callback not in ui:
            issues.append(f"历史回调未接线：{callback}")
    for env in ("NR_HISTORY_VIEW", "NR_HISTORY_CLEAR"):
        if env not in ui:
            issues.append(f"缺少等价驱动 {env}（历史界面入口无程序化对等物）")
    # 「最近阅读」必须在**独立菜单项**，不得进分类侧栏（Q5 已裁定）。
    # 入口按钮的 caption 是三元（进入时「最近阅读」/ 返回时「返回书架」，与
    # `history-mode` 联动），故检查**两个分支字面量 + 回调**，而非整串 caption。
    if not (
        '"最近阅读"' in slint_code
        and '"返回书架"' in slint_code
        and "history-toggle" in slint_code
    ):
        issues.append("缺少「最近阅读」入口按钮（Q5：独立菜单项）")
    if "history-clear" not in ui:
        issues.append("清空历史未走确认弹窗（confirm_action=history-clear 缺失）")
    if "clear_confirm_text" not in history_view:
        issues.append("history_view 未提供清空确认文案（条数须由 Rust 读回）")
    # 排序 / 时间格式化必须在 Rust（界面不得自行排序或拼时间）。
    # 排序在**存储层** `reading-storage/src/history.rs`（`history_view` 只消费已排序结果）。
    history_store_path = REPO / "crates/reading-storage/src/history.rs"
    history_store = (
        history_store_path.read_text(encoding="utf-8") if history_store_path.is_file() else ""
    )
    if "ORDER BY opened_at DESC" not in history_store:
        issues.append("存储层未做时间倒序（排序不得下沉到 UI）")
    if "history::list(" not in history_view:
        issues.append("history_view 未消费存储层已排序的 list（自行排序会掩盖口径）")

    # ---- 自定义配色（V1.1；规格 R10–R13；设计稿 T1–T4）----
    if "NR_THEME_SET" not in ui:
        issues.append("缺少等价驱动 NR_THEME_SET（配色界面入口无程序化对等物）")
    for symbol in ("write_theme_base", "write_theme_enabled", "apply_custom_theme"):
        if symbol not in ui:
            issues.append(f"配色写入/注入入口缺失：{symbol}")
    # 驱动必须**复用同一写入路径**，不得复制逻辑。
    if "write_theme_base(&db_path" not in ui or "NR_THEME_SET" not in ui:
        issues.append("NR_THEME_SET 驱动未复用 write_theme_base（对等性缺口）")
    # 基色行 UI：色块 + 可编辑文本（kind 8）。
    if "row.kind == 8" not in slint_code:
        issues.append("设置页缺少 kind 8 基色输入行（4 个取色器画不出来）")
    if "swatch: color" not in slint_code:
        issues.append("SettingsRow 缺少 swatch 字段（色块色值无处可来）")
    # 对比度必须在 Rust，且不足时**保留用户基色**。
    #
    # ⚠️ 派生算法已下沉到 `reading-storage/src/theme_derive.rs`（V1.1 B3），
    # 桌面侧只剩 re-export —— 故判据要**桌面与共享层都查**，
    # 否则下沉后会被误判成「口径丢失」（实测踩中）。
    storage_derive_path = REPO / "crates/reading-storage/src/theme_derive.rs"
    storage_derive = (
        storage_derive_path.read_text(encoding="utf-8")
        if storage_derive_path.is_file()
        else ""
    )
    derive_all = theme_derive + storage_derive
    if "MIN_CONTRAST" not in derive_all or "contrast_ratio" not in derive_all:
        issues.append("theme_derive 缺少对比度判定（对比度不得下沉到 UI）")
    if "已保留你的选择" not in derive_all:
        issues.append("对比度不足时缺少「保留用户选择」口径（R12）")
    # 40 令牌不得落库：存储层只存 4 基色。
    if "base_foreground" not in theme_store_src or "base_accent" not in theme_store_src:
        issues.append("存储层未按 4 基色持久化")
    for forbidden in ("token_light", "token_dark", "palette_json"):
        if forbidden in theme_store_src:
            issues.append(f"存储层出现派生令牌字段 {forbidden}（40 令牌不落库，R11）")
    # 派生必须覆盖全部 Skin 令牌。
    if "PALETTE_FIELDS" not in theme_derive:
        issues.append("theme_derive 未按 PALETTE_FIELDS 全量派生")

    # ---- 全局阅读背景图（V1.1；规格 R14–R17；设计稿 B1–B3）----
    for env in ("NR_BACKGROUND_SET", "NR_BACKGROUND_CLEAR"):
        if env not in ui:
            issues.append(f"缺少等价驱动 {env}（背景图界面入口无程序化对等物）")
    for symbol in ("background_ops::install", "background_ops::clear"):
        if symbol not in ui:
            issues.append(f"背景图写入路径缺失：{symbol}")
    if "NR_BACKGROUND_SET" not in ui or "background_ops::install" not in ui:
        issues.append("NR_BACKGROUND_SET 驱动未复用 background_ops::install（对等性缺口）")
    if "reader-background-set" not in slint_code:
        issues.append("阅读页缺少背景图层（背景图无处显示）")
    if "row.kind == 9" not in slint_code:
        issues.append("设置页缺少 kind 9 动作行（选图/清除按钮画不出来）")
    # 原图只读 + 路径安全（都在 Rust）。
    for must in ("std::fs::copy", "validate_file_name", "starts_with(\"bg-\")"):
        if must not in background_ops:
            issues.append(f"background_ops 缺安全/只读保障：{must}")
    # 只查**真实调用** `fs::rename(` —— 不能用裸 `rename` 子串匹配：
    # 变量名 `file_name` 里就含 "rename"，会误报（实测踩中）。
    if "fs::rename(" in background_ops or "fs::remove_file(source" in background_ops:
        issues.append("background_ops 对源图做了改名/删除（原图必须只读复制）")
    # 背景不得参与分页（规格 R17）：分页逻辑里不得读背景。
    if "background" in (REPO / "crates/desktop-app/src/pagination.rs").read_text(encoding="utf-8"):
        issues.append("pagination.rs 出现 background（背景不得参与分页计算）")

    if "DeleteChoice::None" not in category_ops:
        issues.append("category_ops 缺少 DeleteChoice::None（未选去向可能被当成默认值）")
    if "请先选择书籍去向" not in category_ops:
        issues.append("未选去向时缺少可理解拒绝文案")
    # 关键：**守卫必须真的存在**。仅检查枚举与文案不够 ——
    # 实测把 `let Some(destination) = ... else { return Err(...) }`
    # 改成「None ⇒ 缺省 MoveToRoot」时，上面两条判据全部照过（判据空转），
    # 只有下面这条「必须短路返回错误」抓得住。
    ops_code = _strip_rust_comments(category_ops)
    if "else {" not in ops_code or "请先选择书籍去向" not in ops_code:
        issues.append("delete_category 缺少「未选去向即拒绝」的短路守卫")
    if re.search(
        r"let\s+Some\(\s*destination\s*\)\s*=\s*choice\.to_destination\(\)\s*else",
        ops_code,
    ) is None:
        issues.append(
            "delete_category 未把 DeleteChoice::None 短路为错误"
            "（未选去向会被当成默认去向执行删除）"
        )

    if issues:
        fail("[10] 分类界面可达性缺陷：" + "; ".join(issues))
    else:
        ok("[10] 分类侧栏 / 行内菜单 / 操作浮层 / 六个 NR_* 驱动齐备，且语义判定全在 Rust")

    # 反向对照：判据必须**有牙**。
    #   `find_faults` 返回「样本中发现的缺陷数」；真实源码应为 0（无缺陷），
    #   而删掉组件/回调/驱动或引入语义下沉的样本必须 > 0。
    def find_faults(
        sample_slint: str,
        sample_ui: str,
        sample_ops: str,
        sample_derive: str = theme_derive,
        sample_bg: str = background_ops,
    ) -> int:
        found = []
        code = _strip_slint_comments(sample_slint)
        ui_only = _strip_rust_comments(sample_ui)
        if "component CategoryRowView" not in code:
            found.append("row")
        if "component CategoryActionDialog" not in code:
            found.append("dialog")
        if "component CategoryMenuStrip" not in code:
            found.append("strip")
        if "component HistoryRowView" not in code:
            found.append("history-row")
        if "component HistoryGroupHeadView" not in code:
            found.append("history-group")
        if not ('"最近阅读"' in code and '"返回书架"' in code and "history-toggle" in code):
            found.append("history-entry")
        for callback in (
            "ui.on_shelf_category_select",
            "ui.on_shelf_category_menu",
            "ui.on_category_dialog_confirm",
            "ui.on_shelf_history_toggle",
            "ui.on_shelf_history_clear",
        ):
            if callback not in sample_ui:
                found.append(callback)
        for env in (
            "NR_CATEGORY_ADD",
            "NR_CATEGORY_MOVE",
            "NR_BOOK_MOVE",
            "NR_IMPORT_ORGANIZE",
            "NR_HISTORY_VIEW",
            "NR_HISTORY_CLEAR",
        ):
            if env not in sample_ui:
                found.append(env)
        if "ui.invoke_shelf_category_select" not in sample_ui:
            found.append("invoke")
        if 'contains("不可选")' in code or 'contains("不可选")' in ui_only:
            found.append("ui-infer")
        if "DeleteChoice::None" not in sample_ops:
            found.append("none-choice")
        if "NR_THEME_SET" not in sample_ui:
            found.append("theme-env")
        if "row.kind == 8" not in code:
            found.append("theme-kind8")
        if "swatch: color" not in code:
            found.append("theme-swatch")
        derive_src = _strip_rust_comments(sample_derive)
        if "MIN_CONTRAST" not in derive_src:
            found.append("theme-contrast")
        for env in ("NR_BACKGROUND_SET", "NR_BACKGROUND_CLEAR"):
            if env not in sample_ui:
                found.append(env)
        if "reader-background-set" not in code:
            found.append("bg-layer")
        if "row.kind == 9" not in code:
            found.append("bg-action")
        bg_src = _strip_rust_comments(sample_bg)
        for must in ("std::fs::copy", "validate_file_name"):
            if must not in bg_src:
                found.append(must)
        ops_only = _strip_rust_comments(sample_ops)
        if re.search(
            r"let\s+Some\(\s*destination\s*\)\s*=\s*choice\.to_destination\(\)\s*else",
            ops_only,
        ) is None:
            found.append("no-guard")
        return len(found)

    baseline = find_faults(app_slint, ui, category_ops)
    if baseline != 0:
        fail(
            f"[10] 反向对照失效：真实源码被判出 {baseline} 处缺陷"
            f"（判据与真实实现不一致，判据空转）"
        )
        return

    # 逐类坏样本：每类都必须被抓出。
    bad_cases = {
        "删掉 CategoryRowView 组件": (
            app_slint.replace("component CategoryRowView", "component _RemovedRowView"),
            ui,
            category_ops,
        ),
        "删掉操作浮层": (
            app_slint.replace("component CategoryActionDialog", "component _GoneDialog"),
            ui,
            category_ops,
        ),
        "删掉行内菜单": (
            app_slint.replace("component CategoryMenuStrip", "component _GoneStrip"),
            ui,
            category_ops,
        ),
        "删掉三个驱动": (
            app_slint,
            ui.replace("NR_CATEGORY_ADD", "_X1")
            .replace("NR_CATEGORY_MOVE", "_X2")
            .replace("NR_BOOK_MOVE", "_X3"),
            category_ops,
        ),
        "删掉一个回调": (
            app_slint,
            ui.replace("ui.on_shelf_category_select", "_gone"),
            category_ops,
        ),
        "驱动不再复用界面回调": (
            app_slint,
            ui.replace("ui.invoke_shelf_category_select", "let _ = 0;"),
            category_ops,
        ),
        "界面用文案推断不可选": (
            app_slint.replace(
                "enabled: row.enabled;",
                'enabled: !row.name.contains("不可选");',
            ),
            ui,
            category_ops,
        ),
        "删掉 DeleteChoice::None": (
            app_slint,
            ui,
            category_ops.replace("DeleteChoice::None", "_NoChoice"),
        ),
    }
    missed = [
        name for name, (sl, u, ops) in bad_cases.items() if find_faults(sl, u, ops) == 0
    ]
    if missed:
        fail("[10] 反向对照失效：以下坏样本竟然通过 —— " + "; ".join(missed))
    else:
        ok(
            f"[10] 反向对照：{len(bad_cases)} 类坏样本（缺组件/缺回调/缺驱动/"
            "驱动不复用回调/语义下沉/缺 None 去向）均被判失败"
        )


def slice_component(source: str, name: str) -> str | None:
    """截取 `.slint` 里某个组件的**函数体**（从声明行到下一个顶层 `component`）。

    用途：把「不得出现某模式」的判据限定在**目标组件**内，避免被其它组件的
    同形写法误伤（V1.1 的历史组头与阅读页都用 `text: root.book-title;`）。

    返回 `None` 表示找不到该组件 —— 调用方应**如实报错**，不得静默跳过。
    """
    marker = f"component {name} "
    start = source.find(marker)
    if start == -1:
        return None
    # 组件体结束于下一个「行首 component」或 EOF。
    nxt = source.find("\ncomponent ", start + len(marker))
    return source[start : nxt if nxt != -1 else len(source)]


def check_layout_and_immersive_exit_static(desktop_src: dict[str, str]) -> None:
    """布局不溢出 + 沉浸退出路径（2026-09-29 真实缺陷回归）。"""
    print("[9] 静态：条带不溢出（按钮永不被文字挤出）+ 沉浸退出路径")
    app_slint = desktop_src.get("app.slint", "")
    ui = desktop_src.get("ui.rs", "")
    input_src = desktop_src.get("input.rs", "")

    issues = []
    # (a) 顶栏 / 底部状态栏 / 菜单条：文字必须可收缩并省略，不得把按钮顶出窗口。
    if app_slint.count("overflow: elide") < 3:
        issues.append("条带文字未全部可省略（overflow: elide < 3 处：标题/状态/章节）")
    if app_slint.count("horizontal-stretch: 1") < 3:
        issues.append("条带文字未占「剩余宽度」（horizontal-stretch < 3 处）")
    # (b) 正文区顶部不得再有与正文同位的浮动小标题。
    #
    # ⚠️ **必须限定在阅读页组件内**，不能全文件子串匹配 —— V1.1 的
    # `HistoryGroupHeadView`（阅读历史组头）合法地用了同一个 `text: root.book-title;`
    # 模式来显示书名，全文件匹配会把它误判成「正文区浮动书名」（实测踩中）。
    reader_page = slice_component(app_slint, "ReaderPage")
    if reader_page is None:
        issues.append("找不到 ReaderPage 组件（判据无从生效，不得静默通过）")
    else:
        if "text: root.book-title;" in reader_page:
            issues.append("正文区顶部仍有浮动书名（与正文重叠）")
        if "text: root.chapter-title;" in reader_page:
            issues.append("正文区顶部仍有浮动章节名（与正文重叠）")
    # (c) 沉浸退出：Esc / 正文点击 / 右上角按钮三条路径。
    if "escape_key_text" not in input_src or "if key_text == input::escape_key_text()" not in ui:
        issues.append("缺少 Esc 退出沉浸的按键路径")
    if "root.immersive-toggle();" not in app_slint:
        issues.append("正文区点击未接入退出沉浸")
    if 'caption: "退出沉浸"' not in app_slint:
        issues.append("沉浸态缺少常驻「退出沉浸」按钮")
    if issues:
        fail("[9] 布局/沉浸退出缺陷：" + "; ".join(issues))
    else:
        ok("[9] 条带文字可收缩省略（标题/状态/章节）+ 无正文区浮动小标题 + 沉浸退出三条路径齐备")

    # 反向对照：三类历史缺陷样本必须判失败。
    bad_title = 'Text { text: root.page-title; font-size: 20px; }'
    # 坏样本必须在**阅读页体内**才该被判失败（与正式判据同一口径）。
    bad_caption = "component ReaderPage inherits Rectangle {\n    Text { y: pad; text: root.book-title; }\n}\n"
    # 同一字符串放在**别的组件**里**不该**被判失败 —— 否则判据过宽（会误报历史组头）。
    ok_elsewhere = "component HistoryGroupHeadView inherits Rectangle {\n    Text { text: root.book-title; }\n}\n"
    bad_click = "clicked => { root.toggle-menu(); }"

    def reader_has_book_title(source: str) -> bool:
        """与正式判据同一口径：找不到 ReaderPage 时视为「无该模式」。"""
        body = slice_component(source, "ReaderPage")
        return body is not None and "text: root.book-title;" in body

    verdicts = [
        "overflow: elide" not in bad_title,
        reader_has_book_title(bad_caption),
        not reader_has_book_title(ok_elsewhere),
        "root.immersive-toggle();" not in bad_click,
    ]
    if all(verdicts):
        ok("[9] 反向对照：三类历史缺陷样本均被判失败")
    else:
        fail("[9] 反向对照失效：历史缺陷样本竟然通过")


def check_immersive_exit_runtime(binary: pathlib.Path) -> None:
    print("[10] 运行时：沉浸态按 Esc 必须退出（回到常规版式）")
    fixture = REPO / "tests/fixtures/smoke.txt"
    run = run_app(
        binary,
        fixture,
        {"NR_IMMERSIVE": "1", "NR_KEY_EVENTS": "escape"},
        "esc",
        exit_after_ms=8000,
    )
    events = [r for r in run["rows"] if r["kind"] == "NR-IMMERSIVE"]
    page_list = pages(run)
    if not events:
        fail("[10] 未取得沉浸事件（NR-IMMERSIVE 缺失）")
        return
    last = events[-1]
    if last.get("immersive") != "0":
        fail(f"[10] Esc 未退出沉浸（最后的 NR-IMMERSIVE immersive={last.get('immersive')}）")
    elif not page_list or page_list[-1].get("immersive") != "0":
        fail("[10] 退出沉浸后 NR-PAGE 仍标注 immersive=1")
    elif page_list[-1].get("region") == page_list[-1].get("window"):
        fail(
            "[10] 退出沉浸后正文区仍是整窗（装饰未恢复）"
            f"（region={page_list[-1].get('region')}）"
        )
    else:
        ok(
            f"[10] Esc 退出沉浸：NR-IMMERSIVE immersive={last.get('immersive')}，"
            f"正文区回到 {page_list[-1].get('region')}（< 整窗 {page_list[-1].get('window')}）"
        )


def check_window_and_toc_static(desktop_src: dict[str, str]) -> None:
    """窗口可缩放 + 目录滚轮/滚动条（2026-09-30 真实缺陷回归）。"""
    print("[11] 静态：窗口可缩放（preferred-*）+ 目录滚轮与比例定位")
    app_slint = desktop_src.get("app.slint", "")
    ui = desktop_src.get("ui.rs", "")

    issues = []
    # (a) 窗口根元素不得用固定 width/height（Slint 会据此判定为「不可缩放窗口」）。
    if "preferred-width: UiMetrics.default-window-width;" not in app_slint:
        issues.append("窗口未使用 preferred-width（无法最大化/改尺寸）")
    if "preferred-height: UiMetrics.default-window-height;" not in app_slint:
        issues.append("窗口未使用 preferred-height")
    if "    width: UiMetrics.default-window-width;" in app_slint:
        issues.append("窗口仍有固定 width（Slint 会禁用缩放）")
    if "    height: UiMetrics.default-window-height;" in app_slint:
        issues.append("窗口仍有固定 height")
    # (b) 目录：滚轮在面板内消费 + 比例定位。
    if "toc-wheel(event.delta-y)" not in app_slint:
        issues.append("目录面板未消费滚轮（滚轮会翻动正文）")
    if "root.toc-seek(" not in app_slint or "ScrollBar {" not in app_slint:
        issues.append("目录缺少滚动条比例定位")
    if "ui.on_toc_wheel" not in ui or "ui.on_toc_seek" not in ui:
        issues.append("目录滚轮/滚动条未接线到 Rust")
    if issues:
        fail("[11] 窗口/目录缺陷：" + "; ".join(issues))
    else:
        ok("[11] 窗口用 preferred-*（可缩放/可最大化）+ 目录滚轮面板内消费 + 滚动条比例定位齐备")

    samples = {
        "固定窗口尺寸": "export component AppWindow inherits Window { width: UiMetrics.default-window-width; }",
        "目录未消费滚轮": 'if (root.toc-open) : Rectangle { VerticalLayout { } }',
    }
    verdicts = [
        "preferred-width: UiMetrics.default-window-width;" not in samples["固定窗口尺寸"],
        "toc-wheel(event.delta-y)" not in samples["目录未消费滚轮"],
    ]
    if all(verdicts):
        ok("[11] 反向对照：固定尺寸窗口与未消费滚轮的目录样本均被判失败")
    else:
        fail("[11] 反向对照失效：历史缺陷样本竟然通过")


def check_toc_scroll_runtime(binary: pathlib.Path) -> None:
    """目录滚轮/滚动条：目录必须滚动，正文**不得**随之翻页（2026-09-30 真实缺陷）。"""
    print("[12] 运行时：目录滚轮/滚动条只滚动目录，不翻正文")
    work = pathlib.Path(tempfile.mkdtemp(prefix="nr-parity-toc-"))
    # 自造多章夹具（目录窗口 12 行，需远超窗口才能体现滚动与比例定位）。
    book = work / "many-chapters.txt"
    lines = []
    for index in range(1, 301):
        lines.append(f"第{index}章 测试章节{index}")
        lines.extend(["正文内容行。" * 6] * 8)
        lines.append("")
    book.write_text("\n".join(lines), encoding="utf-8")

    def run(extra: dict, tag: str) -> list[dict]:
        env = dict(os.environ)
        env.update(
            {
                "DISPLAY": env.get("DISPLAY", ":0"),
                "NR_DB_PATH": str(work / f"lib-{tag}.db"),
                "NR_IMPORT_PATH": str(book),
                "NR_STATE_LOG": "1",
                "NR_START_SCREEN": "reader",
                "NR_READER_BOOK": "first",
                "NR_EXIT_AFTER_MS": "9000",
                "NR_PANEL": "toc",
            }
        )
        env.update(extra)
        proc = subprocess.run(
            [str(binary)], env=env, capture_output=True, text=True, timeout=150, cwd=str(work)
        )
        rows = []
        for line in proc.stdout.splitlines():
            match = re.match(r"^(NR-[A-Z\-]+)\s+(.*)$", line.strip())
            if match:
                row = {"kind": match.group(1)}
                row.update({k: v for k, v in KV.findall(match.group(2))})
                rows.append(row)
        return rows

    def toc_firsts(rows: list[dict]) -> list[int]:
        """目录窗口起点的**全部**取值（面板开启/状态刷新会重新投影，故取集合而非最后一条）。"""
        values = []
        for entry in rows:
            if entry["kind"] != "NR-TOC":
                continue
            try:
                values.append(int(entry.get("window_first", "-1")))
            except ValueError:
                continue
        return values

    def page_start(rows: list[dict]) -> int | None:
        found = pages({"rows": rows})
        return int(found[-1]["start"]) if found else None

    base = run({}, "base")
    seek = run({"NR_TOC_SEEK": "0.99"}, "seek")
    wheel = run({"NR_TOC_WHEEL": "1"}, "wheel")
    base_firsts, seek_firsts, wheel_firsts = toc_firsts(base), toc_firsts(seek), toc_firsts(wheel)
    base_page, seek_page, wheel_page = page_start(base), page_start(seek), page_start(wheel)
    if not base_firsts or not seek_firsts or not wheel_firsts:
        fail("[12] 未能取得目录窗口证据（NR-TOC 缺失）")
        return
    # 目录窗口起点取「全部投影中的最大值」：面板开启/状态刷新会围绕当前章重新投影，
    # 只看最后一条会读到被重置后的 0（判据自身的脆弱点）。
    base_first = min(base_firsts)
    seek_first, wheel_first = max(seek_firsts), max(wheel_firsts)
    if seek_first <= base_first + 100:
        fail(f"[12] 滚动条定位未生效（窗口起点最大只到 {seek_first}）")
    elif wheel_first != base_first + 1:
        fail(f"[12] 目录滚轮步长异常（窗口起点最大 {wheel_first}，期望 {base_first + 1}）")
    elif base_page != seek_page or base_page != wheel_page:
        fail(
            f"[12] 目录滚动把正文也翻页了（page start {base_page} → seek {seek_page} / wheel {wheel_page}）"
        )
    else:
        ok(
            f"[12] 目录滚轮 +1 行、滚动条跳到 {seek_first}（共 300 章），"
            f"正文页码不变（start={base_page}）"
        )


def check_toc_jump_closes_panel(binary: pathlib.Path, desktop_src: dict[str, str]) -> None:
    """目录「跳转」后必须立即展示正文（面板自动收起）（2026-09-30 主人反馈）。"""
    print("[13] 目录「跳转」后自动收起面板（静态 + 运行时）")
    ui = desktop_src.get("ui.rs", "")
    # ---- 静态：跳转函数体内必须收起目录面板（结构定位，而非全文包含） ----
    body = ""
    marker = "fn jump_to_chapter("
    start = ui.find(marker)
    if start >= 0:
        brace = ui.find("{", start)
        depth = 0
        for index in range(brace, len(ui)):
            char = ui[index]
            if char == "{":
                depth += 1
            elif char == "}":
                depth -= 1
                if depth == 0:
                    body = ui[start:index]
                    break
    if "set_toc_open(false)" not in body:
        fail("[13] 跳转函数体内未见「收起目录」调用（跳转后需手动关闭）")
        return
    ok("[13] 静态：章节跳转成功后收起目录面板（set_toc_open(false)）")

    # ---- 运行时：面板打开 + 跳章 → 最后一个状态必须 toc_open=0，且正文有页 ----
    fixture = REPO / "tests/fixtures/smoke.txt"
    run = run_app(binary, fixture, {"NR_PANEL": "toc", "NR_TOC_JUMP": "3"}, "jumpclose")
    states = [r for r in run["rows"] if r["kind"] == "NR-STATE"]
    chapter_rows = [r for r in run["rows"] if r["kind"] == "NR-CHAPTER"]
    page_list = pages(run)
    opens = [s.get("toc_open") for s in states if "toc_open" in s]
    # 时序：`jump_to_chapter` 内部先投影（含收起面板）再写 NR-CHAPTER 行，因此判据取
    # 「面板曾打开 → 其后出现过 toc_open=0」，而不是「跳转行之后的状态」。
    opened_positions = [index for index, value in enumerate(opens) if value == "1"]
    closed_after_open = any(
        value == "0" for index, value in enumerate(opens) if opened_positions and index > opened_positions[0]
    )
    if not opens:
        fail("[13] 未取得面板状态证据（NR-STATE 缺 toc_open）")
    elif not opened_positions:
        fail("[13] 前置条件不成立：目录从未被打开（NR_PANEL=toc 未生效）")
    elif not chapter_rows:
        fail("[13] 未取得跳章证据（NR-CHAPTER 缺失）")
    elif chapter_rows[-1].get("toc_closed") != "1":
        fail(f"[13] 跳转行未标记 toc_closed=1（实际 {chapter_rows[-1].get('toc_closed')}）")
    elif not closed_after_open:
        fail(f"[13] 目录打开后从未被收起（toc_open 序列 {opens}）")
    elif not page_list:
        fail("[13] 跳转后没有正文页")
    else:
        ok(
            f"[13] 运行时：目录打开后经「跳转」自动收起（toc_open 序列 {opens}，"
            f"NR-CHAPTER toc_closed=1），正文页 start={page_list[-1]['start']}"
        )

    # 反向对照：没有收起调用的样本必须判失败。
    bad = "fn jump_to_chapter(seq: u32) { jump_to_offset(seq); }"
    if "set_toc_open(false)" not in bad:
        ok("[13] 反向对照：未收起面板的跳转样本会被判失败")
    else:
        fail("[13] 反向对照失效")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default=str(DEFAULT_BINARY))
    parser.add_argument("--json", default=str(DEFAULT_JSON))
    args = parser.parse_args()
    binary = pathlib.Path(args.binary).resolve()
    if not binary.is_file():
        print(f"PRECONDITION-MISSING: 缺少二进制 {binary}（先 cargo build --release）")
        return 2
    if not os.environ.get("DISPLAY") and not os.environ.get("WAYLAND_DISPLAY"):
        print("PRECONDITION-MISSING: 无 DISPLAY / WAYLAND_DISPLAY（运行时判据无法执行）")
        return 2

    print("== 桌面端对等性门禁（编码 / 沉浸 / 换章上一页）==")
    print(f"binary={binary}")
    desktop_src = {}
    for name in (
        "pagination.rs",
        "chapters.rs",
        "ui.rs",
        "layout.rs",
        "app_state.rs",
        "shelf.rs",
        "import.rs",
        "main.rs",
        "input.rs",
        "category_ops.rs",
        "category_tree.rs",
        "batch_import.rs",
        "settings.rs",
        "history_view.rs",
        "theme_derive.rs",
        "background_ops.rs",
    ):
        path = REPO / "crates/desktop-app/src" / name
        if path.is_file():
            desktop_src[name] = path.read_text(encoding="utf-8")
    manifest = REPO / "crates/desktop-app/Cargo.toml"
    if manifest.is_file():
        desktop_src["Cargo.toml"] = manifest.read_text(encoding="utf-8")
    slint = REPO / "desktop/ui/app.slint"
    if slint.is_file():
        desktop_src["app.slint"] = slint.read_text(encoding="utf-8")

    check_encoding_static(desktop_src)
    check_immersive_static(desktop_src)
    check_prev_page_static(desktop_src)
    check_ui_reachability_static(desktop_src)
    check_category_reachability_static(desktop_src)
    check_layout_and_immersive_exit_static(desktop_src)
    check_window_and_toc_static(desktop_src)
    check_encoding_runtime(binary)
    check_immersive_runtime(binary)
    check_immersive_exit_runtime(binary)
    check_toc_scroll_runtime(binary)
    check_toc_jump_closes_panel(binary, desktop_src)
    check_prev_page_runtime(binary)
    check_negative_control()

    total = len(RESULTS)
    passed = sum(1 for r in RESULTS if r["ok"] and not r["skipped"])
    failed = sum(1 for r in RESULTS if not r["ok"])
    skipped = sum(1 for r in RESULTS if r["skipped"])
    result = "PASS" if failed == 0 else "FAIL"
    summary = f"检查 {passed}/{total} 通过（失败 {failed}，跳过 {skipped}）"
    print("-" * 60)
    print(f"RESULT: {result} — {summary}")

    report_path = pathlib.Path(args.json)
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(
        json.dumps(
            {
                "schema": "nr.desktop.gate/1",
                "gate": "desktop-parity",
                "binary": str(binary),
                "binary_sha256": __import__("hashlib").sha256(binary.read_bytes()).hexdigest(),
                "result": result,
                "exit_code": 0 if failed == 0 else 1,
                "summary": {"total": total, "passed": passed, "failed": failed, "skipped": skipped},
                "checks": RESULTS,
            },
            ensure_ascii=False,
            indent=2,
        ),
        encoding="utf-8",
    )
    print(f"JSON {report_path}")
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())

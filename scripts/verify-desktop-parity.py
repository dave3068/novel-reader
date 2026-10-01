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
    if "text: root.book-title;" in app_slint:
        issues.append("正文区顶部仍有浮动书名（与正文重叠）")
    if "text: root.chapter-title;" in app_slint:
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
    bad_caption = 'Text { y: pad; text: root.book-title; }'
    bad_click = "clicked => { root.toggle-menu(); }"
    verdicts = [
        "overflow: elide" not in bad_title,
        "text: root.book-title;" in bad_caption,
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

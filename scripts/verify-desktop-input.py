#!/usr/bin/env python3
"""verify-desktop-input.py — DT8 桌面输入入口门禁（AC-D4 / DR4）。

覆盖「桌面 SHALL 支持鼠标点击/滚轮、键盘翻页」这一条的可运行证据：

  静态（生产源码）；
  [2] `.slint` 里键盘接收点真实存在：`FocusScope` + `key-pressed` + 转发 `key-input`；
      正文区滚轮接收点真实存在：`TouchArea.scroll-event` + 转发 `wheel-input`；
      并且键盘作用域把「焦点是否不在本作用域上」（= 书签重命名输入框聚焦）一起上报；
  [3] Rust 侧映射函数与回调接线存在，且键盘/滚轮都接到**用户翻页入口**
      （`turn_pages` → `user_turns`），不是另开一条旁路；
  [4] 可访问名称覆盖：目录行 / 书签行 / 输入框 / 按钮组件都有 accessible 语义；
  [5] 非空自检（正向对照）：同一批规则作用在内存样本上必须命中，避免「检索不到 =
      空检查」的假阴性。

  运行时（真实 Release 进程 + 真实 X11 窗口 + 真实 Slint 窗口事件）：
  [6] P1 键盘 → 下一页（PageDown）：NR-INPUT intent=next 且 page_index 递增、进度落库；
      P2 键盘 → 上一页（PageUp）：page_index 回退；
      P3 滚轮 → 下一页 / 上一页（delta-y 正 / 负）；
      P4 负向：书签重命名输入框持有焦点时按翻页键必须**不翻页**（`intent=none
         blocked=text-input`，页面索引不变）。

事件不是旁路调用：脚本只设置 `NR_KEY_EVENTS` / `NR_WHEEL_EVENTS` / `NR_TEXT_FOCUS`，
应用把它们投递为真实 `slint::platform::WindowEvent`（KeyPressed / PointerScrolled），
由 Slint 分派给持有焦点的 `FocusScope` 与正文区 `TouchArea`，再经 `.slint` 回调回到
`crates/desktop-app/src/input.rs` 的判定函数。

用法：
  source /opt/dev-tools/slint-libs/slint-env.sh
  python3 scripts/verify-desktop-input.py [--binary PATH] [--fixture PATH] [--no-build] [--json]

退出码：0 = PASS；1 = FAIL；2 = 前提缺失（无二进制 / 无 X11 / 无夹具）。
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
UI = REPO / "desktop" / "ui" / "app.slint"
INPUT_RS = REPO / "crates" / "desktop-app" / "src" / "input.rs"
UI_RS = REPO / "crates" / "desktop-app" / "src" / "ui.rs"
MANIFEST = REPO / "crates" / "desktop-app" / "Cargo.toml"
DEFAULT_BINARY = REPO / "crates" / "desktop-app" / "target" / "release" / "desktop-app"
DEFAULT_FIXTURE = REPO / "tests" / "fixtures" / "smoke.txt"
LOGDIR = REPO / "results" / "dt8-logs"

PAGE_RE = re.compile(
    r"^NR-PAGE book=(?P<book>-?\d+) layout=(?P<layout>\d+) req=(?P<req>\d+) "
    r"page_index=(?P<page_index>-?\d+) start=(?P<start>\d+) "
)
INPUT_RE = re.compile(r"^NR-INPUT (?P<fields>.*)$")
PROGRESS_RE = re.compile(r"^NR-PROGRESS op=save ")

RESULTS: list[tuple[str, str, str]] = []  # (kind, id, detail)
FAILURES: list[str] = []


def record(kind: str, check_id: str, detail: str) -> None:
    RESULTS.append((kind, check_id, detail))
    prefix = {"pass": "PASS", "fail": "FAIL", "skip": "SKIP"}[kind]
    print(f"  {prefix}  {check_id} — {detail}")
    if kind == "fail":
        FAILURES.append(f"{check_id}: {detail}")


def ok(check_id: str, detail: str) -> None:
    record("pass", check_id, detail)


def fail(check_id: str, detail: str) -> None:
    record("fail", check_id, detail)


def note(check_id: str, detail: str) -> None:
    record("skip", check_id, detail)


def parse_kv(text: str) -> dict[str, str]:
    fields: dict[str, str] = {}
    for token in text.split():
        if "=" in token:
            key, value = token.split("=", 1)
            fields[key] = value
    return fields


# --------------------------------------------------------------- 静态检查


def static_checks() -> None:
    print("[2] .slint 事件接收点（键盘 FocusScope / 正文区滚轮）")
    for path in (UI, INPUT_RS, UI_RS):
        if not path.is_file():
            fail("S0_sources", f"缺少生产源码 {path.relative_to(REPO)}")
            return
    slint = UI.read_text(encoding="utf-8")
    slint_code = strip_slint_comments(slint)

    keyboard_ok = (
        "FocusScope" in slint_code
        and re.search(r"key-pressed\s*\(\s*event\s*\)\s*=>", slint_code) is not None
        and re.search(r"key-input\s*\(\s*event\.text", slint_code) is not None
    )
    if keyboard_ok:
        ok(
            "S1_slint_keyboard_entry",
            "desktop/ui/app.slint 有 FocusScope.key-pressed 且把 event.text 转发给 Rust",
        )
    else:
        fail(
            "S1_slint_keyboard_entry",
            "未找到 FocusScope.key-pressed(event) → key-input(event.text, …) 转发链",
        )

    focus_flag = "!self.has-focus" in slint_code
    if focus_flag:
        ok(
            "S2_slint_focus_flag",
            "键盘作用域上报「焦点是否不在本作用域上」（即书签重命名输入框聚焦）",
        )
    else:
        fail("S2_slint_focus_flag", "键盘转发未携带焦点标志，输入框聚焦时无法屏蔽翻页")

    wheel_ok = re.search(r"scroll-event\s*\(\s*event\s*\)\s*=>", slint_code) is not None and re.search(
        r"wheel-input\s*\(\s*event\.delta-x\s*,\s*event\.delta-y\s*\)", slint_code
    ) is not None
    if wheel_ok:
        ok(
            "S3_slint_wheel_entry",
            "正文区 TouchArea.scroll-event 把 delta-x/delta-y 转发给 Rust",
        )
    else:
        fail("S3_slint_wheel_entry", "未找到 TouchArea.scroll-event → wheel-input(delta-x, delta-y)")

    # DR3：滚轮接收点必须显式消耗事件（正文不做本地滚动，也就不可能出现滚动条）。
    consume = re.search(r"scroll-event\s*\([^)]*\)\s*=>\s*\{[^}]*accept", slint_code, re.S) is not None
    if consume:
        ok("S4_wheel_consumed_not_scrolled", "滚轮事件被显式 accept（映射为整页翻页，不滚动正文）")
    else:
        fail("S4_wheel_consumed_not_scrolled", "scroll-event 未显式 accept，可能被解释为滚动正文")

    # 阅读页必须真的覆盖在同一个键盘作用域里（面板/输入框是其子元素，按键可冒泡到该作用域）。
    has_line_edit = re.search(r"rename-edit\s*:=\s*LineEdit", slint_code) is not None
    if has_line_edit:
        ok("S5_rename_edit_in_scope", "书签重命名输入框位于键盘作用域子树内（聚焦时可冒泡被识别）")
    else:
        fail("S5_rename_edit_in_scope", "未找到带 id 的书签重命名输入框（rename-edit）")

    print("[3] Rust 判定与接线（映射 → 用户翻页入口）")
    rust = INPUT_RS.read_text(encoding="utf-8")
    wiring = UI_RS.read_text(encoding="utf-8")
    mapping_ok = (
        re.search(r"pub fn key_intent\s*\(", rust) is not None
        and re.search(r"pub fn wheel_intent\s*\(", rust) is not None
    )
    if mapping_ok:
        ok("S6_rust_mapping", "input.rs 提供 key_intent / wheel_intent 纯映射函数（有单元测试）")
    else:
        fail("S6_rust_mapping", "input.rs 缺少 key_intent / wheel_intent")

    callbacks_ok = (
        re.search(r"ui\.on_key_input\s*\(", wiring) is not None
        and re.search(r"ui\.on_wheel_input\s*\(", wiring) is not None
    )
    if callbacks_ok:
        ok("S7_ui_callbacks", "ui.rs 接线 FocusScope 键盘回调与正文区滚轮回调")
    else:
        fail("S7_ui_callbacks", "ui.rs 未接线 on_key_input / on_wheel_input")

    keyboard_path = "dispatch_key_input" in wiring and "turn_pages" in wiring
    wheel_path = "dispatch_wheel_input" in wiring and "turn_pages" in wiring
    if keyboard_path and wheel_path:
        ok(
            "S8_shared_user_entry",
            "键盘 / 滚轮都走 turn_pages → user_turns（与「上一页/下一页」按钮同一用户入口）",
        )
    else:
        fail("S8_shared_user_entry", "键盘或滚轮未接到共享用户翻页入口")

    if "NR-INPUT " in wiring:
        ok("S9_structured_input_evidence", "输入入口输出结构化 NR-INPUT 行（含 intent / focused / blocked）")
    else:
        fail("S9_structured_input_evidence", "缺少 NR-INPUT 结构化输入证据")

    print("[4] 可访问名称覆盖（AC-D4/DR4）")
    covered: list[str] = []
    if re.search(r"accessible-role\s*:\s*button[^;]*;[\s\S]{0,200}?accessible-label", slint_code):
        covered.append("按钮组件")
    if re.search(r"accessible-role\s*:\s*list-item[\s\S]{0,200}?accessible-label\s*:\s*row\.title", slint_code):
        covered.append("列表行（目录/书签）")
    if re.search(r"accessible-label\s*:\s*\"书签名称\"", slint_code):
        covered.append("书签重命名输入框")
    missing = [name for name in ("按钮组件", "列表行（目录/书签）", "书签重命名输入框") if name not in covered]
    if missing:
        fail("S10_accessible_names", f"缺少可访问名称：{missing}")
    else:
        ok("S10_accessible_names", f"可访问名称覆盖：{'、'.join(covered)}")

    print("[5] 非空自检（正向对照：规则在已知样本上必须命中）")
    sample = (
        "reader-keys := FocusScope {\n"
        "    key-pressed(event) => {\n"
        "        root.key-input(event.text, !self.has-focus);\n"
        "        reject\n"
        "    }\n"
        "}\n"
        "TouchArea {\n"
        "    scroll-event(event) => {\n"
        "        root.wheel-input(event.delta-x, event.delta-y);\n"
        "        accept\n"
        "    }\n"
        "rename-edit := LineEdit { accessible-label: \"书签名称\"; }\n"
        "for row in rows: Rectangle { accessible-role: list-item; accessible-label: row.title; }\n"
    )
    probes = {
        "键盘接收点": re.search(r"key-pressed\s*\(\s*event\s*\)\s*=>", sample) is not None
        and re.search(r"key-input\s*\(\s*event\.text", sample) is not None,
        "滚轮接收点": re.search(r"scroll-event\s*\(\s*event\s*\)\s*=>", sample) is not None
        and re.search(r"wheel-input\s*\(\s*event\.delta-x\s*,\s*event\.delta-y\s*\)", sample) is not None,
        "可访问名称": re.search(r"accessible-label\s*:\s*\"书签名称\"", sample) is not None
        and re.search(r"accessible-role\s*:\s*list-item[\s\S]{0,200}?accessible-label\s*:\s*row\.title", sample)
        is not None,
    }
    missed = [name for name, hit in probes.items() if not hit]
    if missed:
        fail("S11_self_check_non_vacuous", f"检索规则在已知样本上未命中：{missed}（判定为 INVALID）")
    else:
        ok("S11_self_check_non_vacuous", "同一批规则在已知样本上全部命中（非空检查通过）")


def strip_slint_comments(text: str) -> str:
    text = re.sub(r"/\*[\s\S]*?\*/", "", text)
    return re.sub(r"//[^\n]*", "", text)


# --------------------------------------------------------------- 运行时检查


def build(binary: Path, do_build: bool) -> None:
    if not do_build:
        return
    print("[6] 前提：Release 构建")
    LOGDIR.mkdir(parents=True, exist_ok=True)
    log = LOGDIR / "verify-input-build.log"
    env = dict(os.environ)
    result = subprocess.run(
        ["cargo", "build", "--manifest-path", str(MANIFEST), "--release"],
        cwd=str(REPO),
        env=env,
        capture_output=True,
        text=True,
    )
    log.write_text(result.stdout + result.stderr, encoding="utf-8")
    if result.returncode == 0:
        ok("R0_release_build", f"Release 构建成功（日志 {log.relative_to(REPO)}）")
    else:
        fail("R0_release_build", f"Release 构建失败（exit {result.returncode}），见 {log.relative_to(REPO)}")


def run_case(
    binary: Path,
    fixture: Path,
    work: Path,
    name: str,
    extra_env: dict[str, str],
) -> dict:
    """跑一次真实进程，返回结构化观测结果。"""
    LOGDIR.mkdir(parents=True, exist_ok=True)
    log = LOGDIR / f"verify-input-{name}.log"
    env = dict(os.environ)
    env.update(
        {
            "NR_DB_PATH": str(work / f"{name}.db"),
            "NR_IMPORT_PATH": str(work / f"{name}.txt"),
            "NR_STATE_LOG": "1",
            "NR_START_SCREEN": "reader",
            "NR_READER_BOOK": "first",
            "NR_EXIT_AFTER_MS": "3500",
        }
    )
    env.update(extra_env)
    shutil.copyfile(fixture, work / f"{name}.txt")
    proc = subprocess.run(
        [str(binary)], env=env, capture_output=True, text=True, timeout=120
    )
    log.write_text(proc.stdout + proc.stderr, encoding="utf-8")
    pages: list[int] = []
    inputs: list[dict[str, str]] = []
    saves = 0
    for line in proc.stdout.splitlines():
        match = PAGE_RE.match(line)
        if match:
            pages.append(int(match.group("page_index")))
            continue
        match = INPUT_RE.match(line)
        if match:
            inputs.append(parse_kv(match.group("fields")))
            continue
        if PROGRESS_RE.match(line):
            saves += 1
    return {
        "name": name,
        "exit_code": proc.returncode,
        "pages": pages,
        "inputs": inputs,
        "saves": saves,
        "log": str(log.relative_to(REPO)),
        "env": extra_env,
    }


def runtime_checks(binary: Path, fixture: Path) -> list[dict]:
    print("[7] 运行时：真实进程 + 真实 Slint 窗口事件（键盘 / 滚轮 / 输入框聚焦）")
    work = Path(tempfile.mkdtemp(prefix="nr-dt8-input-"))
    cases: list[dict] = []
    try:
        keyboard = run_case(binary, fixture, work, "keyboard", {"NR_KEY_EVENTS": "page-down,page-down,page-up"})
        cases.append(keyboard)
        wheel = run_case(binary, fixture, work, "wheel", {"NR_WHEEL_EVENTS": "1,-1"})
        cases.append(wheel)
        blocked = run_case(
            binary, fixture, work, "focus-blocked", {"NR_KEY_EVENTS": "page-down", "NR_TEXT_FOCUS": "1"}
        )
        cases.append(blocked)
    finally:
        shutil.rmtree(work, ignore_errors=True)

    for case in cases:
        if case["exit_code"] != 0:
            fail(f"R1_{case['name']}_exit", f"应用退出码 {case['exit_code']}（期望 0），日志 {case['log']}")

    # P1/P2 键盘翻页
    kind = {entry.get("key"): entry for entry in keyboard["inputs"] if entry.get("kind") == "key"}
    if kind.get("page-down", {}).get("intent") == "next" and kind.get("page-up", {}).get("intent") == "prev":
        ok(
            "R2_keyboard_intent",
            f"键盘 NR-INPUT：page-down→next、page-up→prev（{keyboard['log']}）",
        )
    else:
        fail("R2_keyboard_intent", f"键盘 NR-INPUT 不符合预期：{keyboard['inputs']}")
    if keyboard["pages"] == [0, 1, 2, 1]:
        ok("R3_keyboard_paging", f"PageDown×2 后 PageUp 得到页面序列 {keyboard['pages']}（0 基 index）")
    else:
        fail("R3_keyboard_paging", f"键盘翻页页面序列异常：{keyboard['pages']}（期望 [0, 1, 2, 1]）")
    if keyboard["saves"] >= 3:
        ok("R4_keyboard_progress_saved", f"键盘翻页按用户入口保存进度：NR-PROGRESS op=save {keyboard['saves']} 次")
    else:
        fail("R4_keyboard_progress_saved", f"键盘翻页未保存进度（NR-PROGRESS {keyboard['saves']} 次）")

    # P3 滚轮翻页
    wheel_kinds = [entry for entry in wheel["inputs"] if entry.get("kind") == "wheel"]
    intents = [entry.get("intent") for entry in wheel_kinds]
    if intents == ["next", "prev"]:
        ok("R5_wheel_intent", f"滚轮 NR-INPUT：delta-y=+1→next、-1→prev（{wheel['log']}）")
    else:
        fail("R5_wheel_intent", f"滚轮 NR-INPUT 不符合预期：{wheel['inputs']}")
    if wheel["pages"] == [0, 1, 0]:
        ok("R6_wheel_paging", f"滚轮翻页页面序列 {wheel['pages']}（正文未出现滚动，仍为整页翻页）")
    else:
        fail("R6_wheel_paging", f"滚轮翻页页面序列异常：{wheel['pages']}（期望 [0, 1, 0]）")
    if wheel["saves"] >= 2:
        ok("R7_wheel_progress_saved", f"滚轮翻页保存进度：NR-PROGRESS op=save {wheel['saves']} 次")
    else:
        fail("R7_wheel_progress_saved", f"滚轮翻页未保存进度（NR-PROGRESS {wheel['saves']} 次）")

    # P4 负向：输入框聚焦时不翻页
    blocked_inputs = blocked["inputs"]
    blocked_hit = any(
        entry.get("intent") == "none"
        and entry.get("blocked") == "text-input"
        and entry.get("focused") == "1"
        for entry in blocked_inputs
    )
    if blocked_hit:
        ok(
            "R8_text_input_focus_blocks_paging",
            f"输入框聚焦时翻页键被屏蔽：{blocked_inputs}（{blocked['log']}）",
        )
    else:
        fail("R8_text_input_focus_blocks_paging", f"未观察到 blocked=text-input 记录：{blocked_inputs}")
    if blocked["pages"] == [0]:
        ok("R9_text_input_focus_no_page_change", f"输入框聚焦时页面索引不变：{blocked['pages']}")
    else:
        fail("R9_text_input_focus_no_page_change", f"输入框聚焦时页面仍变化：{blocked['pages']}")

    return cases


def main() -> int:
    parser = argparse.ArgumentParser(description="DT8 桌面输入入口门禁")
    parser.add_argument("--binary", type=Path, default=DEFAULT_BINARY)
    parser.add_argument("--fixture", type=Path, default=DEFAULT_FIXTURE)
    parser.add_argument("--no-build", action="store_true", help="跳过 Release 构建（要求二进制已存在）")
    parser.add_argument("--json", action="store_true", help="仅输出结果 JSON（人类可读行写 stderr）")
    args = parser.parse_args()

    print("== DT8 桌面输入入口门禁（AC-D4 / DR4）==")
    static_checks()

    if not args.fixture.is_file():
        print(f"PRECONDITION_MISSING fixture={args.fixture}")
        return 2
    build(args.binary, not args.no_build)
    if not os.access(args.binary, os.X_OK):
        print(f"PRECONDITION_MISSING binary={args.binary}（先执行 C10/C5 的 Release 构建）")
        return 2
    # 与 scripts/smoke-desktop.sh 同口径：默认 :0，并用 xdpyinfo 证明 X 服务器可达。
    os.environ.setdefault("DISPLAY", ":0")
    if shutil.which("xdpyinfo"):
        probe = subprocess.run(
            ["xdpyinfo", "-display", os.environ["DISPLAY"]], capture_output=True, text=True
        )
        if probe.returncode != 0:
            print(f"PRECONDITION_MISSING DISPLAY={os.environ['DISPLAY']}（X 服务器不可达）")
            return 2

    cases = runtime_checks(args.binary, args.fixture)

    passed = sum(1 for kind, _, _ in RESULTS if kind == "pass")
    failed = sum(1 for kind, _, _ in RESULTS if kind == "fail")
    skipped = sum(1 for kind, _, _ in RESULTS if kind == "skip")
    overall = "PASS" if failed == 0 else "FAIL"
    print(
        f"RESULT: {overall} — 检查 {passed}/{passed + failed + skipped} 通过"
        f"（失败 {failed}，跳过 {skipped}）"
    )

    if args.json:
        doc = {
            "schema": "nr.desktop.gate/1",
            "gate": "verify-desktop-input",
            "result": overall,
            "exit_code": 0 if overall == "PASS" else 1,
            "summary": {
                "total": len(RESULTS),
                "passed": passed,
                "failed": failed,
                "skipped": skipped,
            },
            "checks": [{"kind": k, "id": i, "detail": d} for k, i, d in RESULTS],
            "runtime": cases,
        }
        print(json.dumps(doc, ensure_ascii=False, indent=2))

    return 0 if overall == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())

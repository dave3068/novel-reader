#!/usr/bin/env python3
"""verify-desktop-targets.py — DT6 桌面端构建/打包目标的状态校验（卡片 `t_e83b8ad0`）。

用途（desktop/spec.md DR6、desktop checklist C7）：
  校验「Windows 10/11 与 Linux Wayland 的构建/打包路径」是否齐全，并**如实**输出每个目标的状态：
    - `verified`  → 输出「通过」，且必须有一份真实存在、`result == PASS` 的证据文件（含新鲜度核对）；
    - `pending`   → 输出「待验证」，且**不得**携带 PASS 证据（未实测不得写成通过）。

单一事实来源：`desktop/packaging/targets.json`（打包脚本、发布目录布局、依赖与系统要求、状态与证据）。
本脚本只读仓库内文件 + 只读探测本机环境，不构建、不安装、不修改任何文件（`--out` 除外）。

两种模式：
  1) `--dry-run`：**只做静态校验**，不探测本机环境、不执行任何打包脚本 ——
     校验打包脚本存在/可执行/语法、依赖清单（运行库/字体/系统要求）与状态字段齐全、发布目录布局口径；
  2) 默认（无 `--dry-run`）：在静态校验之上，探测本机环境（os/arch/会话/glibc/字体/工具链），
     逐目标输出状态；未实测目标输出 `待验证` 并给出原因。

检查项：
  [1] 目标清单可解析（schema `nr.desktop.targets/1`）；
  [2] 请求的目标 id 全部存在于清单；
  [3] 打包脚本：存在、bash 可执行位、`bash -n` 语法通过（有 pwsh 时对 .ps1 做 AST 解析）、
      必需令牌齐备、引用依赖文档；
  [4] 依赖与系统要求文档存在且含必需章节（glibc / 字体 / 运行库）；
  [5] 逐目标字段齐全：构建命令、打包命令、发布目录名模式、布局（required 文件/目录、非单可执行文件口径）、
      依赖清单（构建/运行库/字体/系统要求/gui）、验证门禁、状态与状态标签；
  [6] 状态诚实性：通过目标必须有真实 PASS 证据；待验证目标不得带 PASS 证据，且必须有阻塞原因与未实测清单；
  [7] 默认模式额外：本机环境探测 + 通过目标的证据新鲜度（二进制 sha256）核对；
  [8] 负向对照（自检）：故意制造 5 类缺陷，检查器必须全部报错 —— 证明上面不是空检查。

退出码：0 = 校验通过（含「待验证」目标，未实测不算失败）；1 = 存在违规；2 = 前提缺失（清单缺失/目标 id 未知）。
"""

from __future__ import annotations

import argparse
import json
import platform
import re
import shutil
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]

MANIFEST_PATH = REPO_ROOT / "desktop" / "packaging" / "targets.json"
MANIFEST_SCHEMA = "nr.desktop.targets/1"
BUILD_ARTIFACT = REPO_ROOT / "crates" / "desktop-app" / "target" / "release" / "desktop-app"
DEP_DOC_TOKEN = "platform-requirements.md"

ALLOWED_STATUS = ("verified", "pending")
STATUS_LABEL = {"verified": "通过", "pending": "待验证"}
ALLOWED_HOST_OS = ("linux", "windows", "macos")
ALLOWED_LAYOUT_KIND = ("file", "dir")

REQUIRED_TARGET_FIELDS = (
    "id",
    "display_name",
    "host_os",
    "arch",
    "session",
    "packaging_mode",
    "package_script",
    "build_command",
    "package_command",
    "release_dir_name_pattern",
    "release_layout",
    "dependencies",
    "verification_gate",
    "status",
    "status_label",
)

REQUIRED_DEP_KEYS = ("build", "runtime_shared_libraries", "fonts", "glibc", "gui")

# 至少这么多 required 文件 / required 目录，避免「布局只写一个可执行文件」蒙混过关。
MIN_REQUIRED_FILES = 4
MIN_REQUIRED_DIRS = 2

FAILURES: list[str] = []
NOTES: list[str] = []
CHECKS: list[dict] = []
JSON_MODE = False


# ---------------------------------------------------------------------------
# 输出
# ---------------------------------------------------------------------------
def emit(text: str = "") -> None:
    stream = sys.stderr if JSON_MODE else sys.stdout
    print(text, file=stream)


def ok(detail: str) -> None:
    emit(f"    OK   {detail}")


def fail(detail: str) -> None:
    FAILURES.append(detail)
    emit(f"    FAIL {detail}")


def note(detail: str) -> None:
    NOTES.append(detail)
    emit(f"    SKIP {detail}")


def record(check_id: str, title: str, oks: int, fails: int, skipped: int = 0) -> None:
    CHECKS.append(
        {
            "id": check_id,
            "title": title,
            "ok": fails == 0,
            "ok_count": oks,
            "fail_count": fails,
            "skipped_count": skipped,
        }
    )


# ---------------------------------------------------------------------------
# 工具
# ---------------------------------------------------------------------------
def run(cmd: list[str], timeout: int = 20) -> tuple[int, str]:
    """只读执行外部命令；命令不存在或超时返回 (-1, 说明)。"""
    try:
        proc = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=timeout,
            errors="replace",
        )
    except FileNotFoundError:
        return -1, f"命令不存在：{cmd[0]}"
    except subprocess.TimeoutExpired:
        return -1, f"命令超时（{timeout}s）：{' '.join(cmd)}"
    except OSError as exc:  # pragma: no cover - 环境相关
        return -1, f"命令失败：{exc}"
    return proc.returncode, (proc.stdout or "") + (proc.stderr or "")


def read_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return ""


# ---------------------------------------------------------------------------
# [1] 目标清单
# ---------------------------------------------------------------------------
def load_manifest() -> tuple[dict | None, int]:
    emit(f"verify-desktop-targets: repo={REPO_ROOT}")
    emit("[1] 目标清单 desktop/packaging/targets.json")
    if not MANIFEST_PATH.is_file():
        fail(f"清单缺失：{MANIFEST_PATH}（前提缺失）")
        record("T1_manifest_parsable", "目标清单可解析", 0, 1)
        return None, 2
    try:
        doc = json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        fail(f"清单无法解析：{exc}")
        record("T1_manifest_parsable", "目标清单可解析", 0, 1)
        return None, 2

    target_count = len(doc.get("targets") or [])
    if doc.get("schema") != MANIFEST_SCHEMA:
        fail(f"schema 不是 {MANIFEST_SCHEMA}：{doc.get('schema')!r}")
        record("T1_manifest_parsable", "目标清单可解析", 0, 1)
        return None, 2
    if target_count == 0:
        fail("清单里没有任何目标")
        record("T1_manifest_parsable", "目标清单可解析", 0, 1)
        return None, 2
    ok(f"schema={MANIFEST_SCHEMA}，目标 {target_count} 个：{', '.join(t.get('id', '?') for t in doc['targets'])}")
    record("T1_manifest_parsable", "目标清单可解析", 1, 0)
    return doc, 0


def select_targets(doc: dict, requested: list[str]) -> tuple[list[dict], int]:
    emit("[2] 请求目标")
    known = {t.get("id"): t for t in doc.get("targets", [])}
    if not requested:
        emit(f"    未指定 --targets，校验清单内全部目标：{', '.join(known)}")
        record("T2_requested_targets", "请求目标存在于清单", 1, 0)
        return list(known.values()), 0
    unknown = [tid for tid in requested if tid not in known]
    if unknown:
        fail(f"未知目标 id：{', '.join(unknown)}（清单内可用：{', '.join(known)}）—— 前提缺失")
        record("T2_requested_targets", "请求目标存在于清单", 0, 1)
        return [], 2
    ok(f"请求目标全部存在：{', '.join(requested)}")
    record("T2_requested_targets", "请求目标存在于清单", 1, 0)
    return [known[tid] for tid in requested], 0


# ---------------------------------------------------------------------------
# [3] 打包脚本
# ---------------------------------------------------------------------------
def check_scripts(targets: list[dict]) -> None:
    emit("[3] 打包脚本（存在 / 可执行 / 语法 / 必需令牌 / 引用依赖文档）")
    oks = fails = skips = 0
    seen: set[str] = set()
    pwsh = shutil.which("pwsh")
    for target in targets:
        rel = target.get("package_script") or ""
        if not rel:
            fail(f"{target.get('id')}：未登记 package_script")
            fails += 1
            continue
        if rel in seen:
            continue
        seen.add(rel)
        path = REPO_ROOT / rel
        kind = target.get("package_script_kind") or ""
        if not path.is_file():
            fail(f"{rel} 不存在（{target.get('id')} 的打包脚本）")
            fails += 1
            continue
        text = read_text(path)

        if kind == "bash":
            if not path.stat().st_mode & 0o111:
                fail(f"{rel} 缺少可执行位（chmod +x）")
                fails += 1
            else:
                ok(f"{rel} 可执行位已设置")
                oks += 1
            bash = shutil.which("bash")
            if bash:
                rc, out = run([bash, "-n", str(path)])
                if rc == 0:
                    ok(f"{rel} bash -n 语法通过")
                    oks += 1
                else:
                    fail(f"{rel} bash -n 语法失败：{out.strip()[:200]}")
                    fails += 1
            else:
                note(f"{rel} 本机无 bash，跳过语法解析（改为令牌校验）")
                skips += 1
        elif kind == "powershell":
            if pwsh:
                rc, out = run(
                    [
                        pwsh,
                        "-NoProfile",
                        "-Command",
                        "$null = [System.Management.Automation.Language.Parser]::ParseFile"
                        f"('{path}', [ref]$null, [ref]$null); 'PARSE-OK'",
                    ]
                )
                if rc == 0 and "PARSE-OK" in out:
                    ok(f"{rel} PowerShell AST 解析通过（{pwsh}）")
                    oks += 1
                else:
                    fail(f"{rel} PowerShell 解析失败：{out.strip()[:200]}")
                    fails += 1
            else:
                note(
                    f"{rel} 本机无 pwsh，跳过语法解析（改为静态令牌校验；"
                    "该脚本从未在 Windows 主机执行过，状态保持「待验证」）"
                )
                skips += 1

        tokens = target.get("script_tokens") or []
        if not tokens:
            fail(f"{target.get('id')}：script_tokens 为空，无法校验脚本内容")
            fails += 1
        else:
            missing = [tok for tok in tokens if tok not in text]
            if missing:
                fail(f"{rel} 缺少必需令牌：{missing}")
                fails += 1
            else:
                ok(f"{rel} 必需令牌齐备（{len(tokens)}/{len(tokens)}）")
                oks += 1

        if DEP_DOC_TOKEN in text:
            ok(f"{rel} 引用依赖与系统要求文档（{DEP_DOC_TOKEN}）")
            oks += 1
        else:
            fail(f"{rel} 未引用依赖与系统要求文档（{DEP_DOC_TOKEN}）—— 依赖说明不会随发布目录分发")
            fails += 1
    record("T3_package_scripts", "打包脚本静态校验", oks, fails, skips)


# ---------------------------------------------------------------------------
# [4] 依赖与系统要求文档
# ---------------------------------------------------------------------------
def check_dep_doc(doc: dict) -> None:
    emit("[4] 依赖与系统要求文档")
    rel = doc.get("dep_doc") or ""
    keywords = doc.get("dep_doc_required_keywords") or []
    path = REPO_ROOT / rel
    oks = fails = 0
    if not rel or not path.is_file():
        fail(f"依赖文档缺失：{rel or '(未登记)'}")
        record("T4_dep_doc", "依赖与系统要求文档", 0, 1)
        return
    ok(f"{rel} 存在（{path.stat().st_size} 字节）")
    oks += 1
    text = read_text(path)
    missing = [kw for kw in keywords if kw not in text]
    if missing:
        fail(f"{rel} 缺少必需章节/关键词：{missing}")
        fails += 1
    else:
        ok(f"必需关键词齐备：{keywords}")
        oks += 1
    # 「待验证」是否真的写在文档里（未实测不得写成通过）
    if "待验证" in text:
        ok("文档明确标注「待验证」项")
        oks += 1
    else:
        fail("文档未出现「待验证」字样：未实测平台的口径可能被写成通过")
        fails += 1
    record("T4_dep_doc", "依赖与系统要求文档", oks, fails)


# ---------------------------------------------------------------------------
# [5][6] 逐目标字段齐全 + 状态诚实性
# ---------------------------------------------------------------------------
def validate_target(target: dict) -> list[str]:
    """返回该目标的问题列表（空 = 齐全）。不做外部命令调用。"""
    problems: list[str] = []
    tid = target.get("id") or "(缺少 id)"

    for field in REQUIRED_TARGET_FIELDS:
        value = target.get(field)
        if value is None or (isinstance(value, (str, list, dict)) and len(value) == 0):
            problems.append(f"{tid}：缺少字段 {field}")

    if tid == "(缺少 id)":
        return problems

    if target.get("host_os") not in ALLOWED_HOST_OS:
        problems.append(f"{tid}：host_os 非法（{target.get('host_os')!r}）")

    # 发布目录口径：必须是完整发布目录，禁止单可执行文件
    mode = target.get("packaging_mode")
    if mode != "release-directory":
        problems.append(f"{tid}：packaging_mode 必须为 release-directory（当前 {mode!r}）")

    build_command = target.get("build_command") or ""
    if "--release" not in build_command:
        problems.append(f"{tid}：build_command 必须包含 --release（当前 {build_command!r}）")

    pattern = target.get("release_dir_name_pattern") or ""
    if "{version}" not in pattern:
        problems.append(f"{tid}：release_dir_name_pattern 缺少 {{version}} 占位符（当前 {pattern!r}）")

    # 布局
    layout = target.get("release_layout")
    if not isinstance(layout, list) or not layout:
        problems.append(f"{tid}：release_layout 必须是非空数组")
    else:
        req_files: list[str] = []
        req_dirs: list[str] = []
        for index, entry in enumerate(layout):
            if not isinstance(entry, dict):
                problems.append(f"{tid}：release_layout[{index}] 不是对象")
                continue
            for key in ("path", "kind", "required"):
                if key not in entry:
                    problems.append(f"{tid}：release_layout[{index}] 缺少 {key}")
            if entry.get("kind") not in ALLOWED_LAYOUT_KIND:
                problems.append(f"{tid}：release_layout[{index}].kind 非法（{entry.get('kind')!r}）")
                continue
            if not entry.get("required"):
                continue
            if entry["kind"] == "file":
                req_files.append(str(entry.get("path")))
            else:
                req_dirs.append(str(entry.get("path")).rstrip("/"))
        if len(req_files) < MIN_REQUIRED_FILES:
            problems.append(
                f"{tid}：required 文件仅 {len(req_files)} 项（至少 {MIN_REQUIRED_FILES} 项）——发布目录不完整"
            )
        if len(req_dirs) < MIN_REQUIRED_DIRS:
            problems.append(
                f"{tid}：required 目录仅 {len(req_dirs)} 项（至少 {MIN_REQUIRED_DIRS} 项）——疑似单可执行文件口径"
            )
        if not any(p.endswith("MANIFEST.sha256") for p in req_files):
            problems.append(f"{tid}：required 文件缺少 MANIFEST.sha256（发布目录无法自校验）")
        if not any(p.endswith("VERSION") for p in req_files):
            problems.append(f"{tid}：required 文件缺少 VERSION")
        if not any("doc" in p for p in req_dirs):
            problems.append(f"{tid}：required 目录缺少文档目录（docs/ 或 share/doc/）")

    # 依赖清单
    deps = target.get("dependencies")
    if not isinstance(deps, dict):
        problems.append(f"{tid}：dependencies 必须是对象")
    else:
        for key in REQUIRED_DEP_KEYS:
            if key not in deps:
                problems.append(f"{tid}：dependencies 缺少 {key}")
        for key in ("runtime_shared_libraries", "fonts"):
            value = deps.get(key)
            if not isinstance(value, list) or not value:
                problems.append(f"{tid}：dependencies.{key} 必须是非空数组（依赖清单不得为空）")
        build = deps.get("build")
        if not isinstance(build, dict) or not build:
            problems.append(f"{tid}：dependencies.build 必须是非空对象")
        elif not build.get("target_triple"):
            problems.append(f"{tid}：dependencies.build 缺少 target_triple")
        if target.get("host_os") == "linux" and not str(deps.get("glibc") or "").strip():
            problems.append(f"{tid}：Linux 目标必须在 dependencies.glibc 说明 glibc 口径")

    # 第三方运行时依赖（本卡禁止新增）
    extra = target.get("dependencies", {}).get("third_party_runtime_added_by_this_card")
    if extra:
        problems.append(f"{tid}：本卡不得新增第三方运行时依赖，但登记了 {extra!r}")

    # 状态与诚实性
    status = target.get("status")
    if status not in ALLOWED_STATUS:
        problems.append(f"{tid}：status 非法（{status!r}），允许 {list(ALLOWED_STATUS)}")
    else:
        label = target.get("status_label")
        if label != STATUS_LABEL[status]:
            problems.append(
                f"{tid}：status={status} 必须配 status_label={STATUS_LABEL[status]}（当前 {label!r}）"
            )
        evidence = target.get("status_evidence")
        if status == "verified":
            if not isinstance(evidence, dict) or not evidence:
                problems.append(f"{tid}：status=verified 但没有 status_evidence（未实测不得写「通过」）")
            else:
                ev_path = evidence.get("path")
                if not ev_path:
                    problems.append(f"{tid}：status_evidence 缺少 path")
                else:
                    abs_path = REPO_ROOT / str(ev_path)
                    if not abs_path.is_file():
                        problems.append(f"{tid}：证据文件不存在（{ev_path}）")
                    else:
                        try:
                            ev_doc = json.loads(abs_path.read_text(encoding="utf-8"))
                        except (OSError, json.JSONDecodeError) as exc:
                            problems.append(f"{tid}：证据文件无法解析（{ev_path}：{exc}）")
                        else:
                            if ev_doc.get("result") != "PASS":
                                problems.append(
                                    f"{tid}：证据 {ev_path} 的 result={ev_doc.get('result')!r}，非 PASS"
                                )
                if evidence.get("result") != "PASS":
                    problems.append(f"{tid}：status_evidence.result 必须为 PASS（当前 {evidence.get('result')!r}）")
        elif status == "pending":
            if isinstance(evidence, dict) and evidence.get("result") == "PASS":
                problems.append(
                    f"{tid}：status=pending 不得携带 result=PASS 的证据（未实测不得写成通过）"
                )
            if not str(target.get("blocking_reason") or "").strip():
                problems.append(f"{tid}：status=pending 必须填写 blocking_reason（未实测原因）")
            if not target.get("unverified_items"):
                problems.append(f"{tid}：status=pending 必须列出 unverified_items（未实测清单）")

    gate = target.get("verification_gate") or ""
    if not gate.strip():
        problems.append(f"{tid}：verification_gate 为空（未实测目标也要写清将来怎么测）")

    return problems


def check_targets(targets: list[dict]) -> None:
    emit("[5] 逐目标字段齐全性（构建/打包/布局/依赖/门禁/状态字段）")
    oks = fails = 0
    for target in targets:
        problems = validate_target(target)
        tid = target.get("id") or "(缺少 id)"
        if problems:
            fails += 1
            for item in problems:
                fail(item)
            continue
        oks += 1
        layout = target.get("release_layout") or []
        req_files = [e["path"] for e in layout if e.get("required") and e.get("kind") == "file"]
        req_dirs = [e["path"] for e in layout if e.get("required") and e.get("kind") == "dir"]
        ok(
            f"{tid}：状态={target.get('status_label')}；打包方式={target.get('packaging_mode')}；"
            f"布局 {len(layout)} 项（required {len(req_files)} 文件 / {len(req_dirs)} 目录）；"
            f"运行库 {len(target['dependencies']['runtime_shared_libraries'])} 项、"
            f"字体 {len(target['dependencies']['fonts'])} 项、系统要求字段齐备"
        )
    record("T5_target_fields", "逐目标字段齐全性", oks, fails)

    emit("[6] 状态诚实性（通过必须有 PASS 证据；待验证不得带 PASS 证据）")
    oks = fails = 0
    for target in targets:
        tid = target.get("id")
        if target.get("status") == "verified":
            evidence = target.get("status_evidence") or {}
            ok(
                f"{tid}：通过 ← 证据 {evidence.get('path')}"
                f"（gate={evidence.get('gate')}, run_id={evidence.get('run_id')}, result={evidence.get('result')}）"
            )
            oks += 1
        else:
            reason = str(target.get("blocking_reason") or "").strip()
            ok(f"{tid}：待验证 ← {reason[:120]}{'…' if len(reason) > 120 else ''}")
            oks += 1
    record("T6_status_honesty", "状态诚实性", oks, fails)


# ---------------------------------------------------------------------------
# [7] 默认模式：本机环境探测与证据新鲜度
# ---------------------------------------------------------------------------
def probe_environment() -> dict:
    def command_available(name: str) -> bool:
        return shutil.which(name) is not None

    host = platform.system().lower()
    host_os = {"linux": "linux", "windows": "windows", "darwin": "macos"}.get(host, host)

    env: dict = {
        "host_os": host_os,
        "host_os_raw": platform.system(),
        "arch": platform.machine().lower(),
        "python": platform.python_version(),
        "session": "unknown",
        "glibc": None,
        "font_match": None,
        "wayland_display": None,
        "xdg_session_type": None,
        "display": None,
        "tools": {
            name: command_available(name)
            for name in ("cargo", "rustc", "bash", "pwsh", "xdotool", "fc-match", "ldd", "dpkg")
        },
    }

    if host_os == "windows":
        env["session"] = "win32"
    else:
        import os

        env["wayland_display"] = os.environ.get("WAYLAND_DISPLAY") or None
        env["xdg_session_type"] = os.environ.get("XDG_SESSION_TYPE") or None
        env["display"] = os.environ.get("DISPLAY") or None
        if env["wayland_display"] or env["xdg_session_type"] == "wayland":
            env["session"] = "wayland"
        elif env["display"] or env["xdg_session_type"] == "x11":
            env["session"] = "x11"
        else:
            env["session"] = "none"

    if host_os == "linux":
        if env["tools"]["ldd"]:
            rc, out = run(["ldd", "--version"])
            if rc == 0:
                first = out.strip().splitlines()[0] if out.strip() else ""
                match = re.search(r"(\d+\.\d+(?:\.\d+)?)", first)
                env["glibc"] = match.group(1) if match else first
        if env["tools"]["fc-match"]:
            rc, out = run(["fc-match", "sans-serif:lang=zh-cn"])
            if rc == 0:
                env["font_match"] = out.strip().splitlines()[0] if out.strip() else ""
    return env


def session_matches(target: dict, env: dict) -> bool:
    if target.get("host_os") != env.get("host_os"):
        return False
    if target.get("arch") and target.get("arch") != env.get("arch"):
        return False
    return target.get("session") == env.get("session")


def check_environment(targets: list[dict], env: dict) -> None:
    emit("[7] 本机环境探测（仅默认模式；--dry-run 不做）")
    ok(
        f"host_os={env['host_os_raw']} arch={env['arch']} session={env['session']}"
        f" glibc={env['glibc'] or 'n/a'}"
    )
    if env.get("font_match"):
        ok(f"fc-match sans-serif:lang=zh-cn → {env['font_match']}")
    elif env["host_os"] == "linux":
        note("fc-match 不可用，未探测中文字体")
    available = [name for name, present in env["tools"].items() if present]
    ok(f"可用工具：{', '.join(available) if available else '(无)'}")
    missing_tools = [name for name, present in env["tools"].items() if not present]
    if missing_tools:
        note(f"缺少工具：{', '.join(missing_tools)}（相关目标仍可做静态校验，但无法实测）")

    for target in targets:
        if target.get("status") != "verified":
            continue
        evidence = target.get("status_evidence") or {}
        recorded_sha = evidence.get("binary_sha256")
        if not recorded_sha:
            note(f"{target.get('id')}：证据未记录 binary_sha256，跳过新鲜度核对")
            continue
        if not BUILD_ARTIFACT.is_file():
            note(f"{target.get('id')}：本机构件 {BUILD_ARTIFACT.relative_to(REPO_ROOT)} 不存在，跳过新鲜度核对")
            continue
        rc, out = run(["sha256sum", str(BUILD_ARTIFACT)])
        if rc != 0:
            note(f"{target.get('id')}：无法计算构件 sha256，跳过新鲜度核对")
            continue
        actual = out.split()[0] if out.split() else ""
        if actual == recorded_sha:
            ok(f"{target.get('id')}：证据新鲜度通过（构件 sha256 与证据一致 {actual[:16]}…）")
        else:
            fail(
                f"{target.get('id')}：证据过期 —— 构件 sha256={actual[:16]}…，"
                f"证据记录={recorded_sha[:16]}…（需重跑 {target.get('verification_gate')}）"
            )
    record("T7_environment", "本机环境探测与证据新鲜度", 1, 0)


# ---------------------------------------------------------------------------
# [8] 负向对照（自检非空）
# ---------------------------------------------------------------------------
def negative_control(sample: dict) -> None:
    emit("[8] 负向对照（自检）：故意制造 5 类缺陷，检查器必须全部报错")
    import copy

    cases = [
        (
            "缺少 release_layout",
            lambda t: t.pop("release_layout", None),
            "release_layout",
        ),
        (
            "待验证目标伪装成通过",
            lambda t: t.update({"status": "pending", "status_label": "通过", "blocking_reason": None,
                                "unverified_items": None}),
            "status_label",
        ),
        (
            "发布目录口径被改成单可执行文件",
            lambda t: t.update({"packaging_mode": "single-executable"}),
            "packaging_mode",
        ),
        (
            "required 文件不足（只剩一个可执行文件）",
            lambda t: t.update(
                {
                    "release_layout": [
                        {"path": "novel-reader", "kind": "file", "required": True, "note": None}
                    ]
                }
            ),
            "required",
        ),
        (
            "偷偷新增第三方运行时依赖",
            lambda t: t["dependencies"].update(
                {"third_party_runtime_added_by_this_card": ["libfoo.so.1"]}
            ),
            "第三方运行时依赖",
        ),
    ]
    detected = 0
    for name, mutate, expect_token in cases:
        probe = copy.deepcopy(sample)
        mutate(probe)
        problems = validate_target(probe)
        if problems and any(expect_token in item for item in problems):
            ok(f"检出「{name}」")
            detected += 1
        else:
            fail(f"未检出「{name}」—— 该项为空检查（problems={problems}）")
    record("T8_negative_control", "负向对照自检", detected, len(cases) - detected)


# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------
def main() -> int:
    global JSON_MODE

    parser = argparse.ArgumentParser(
        description="校验 desktop/packaging/targets.json 与打包脚本，并输出各平台目标状态（未实测输出「待验证」）。",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "--targets",
        nargs="*",
        default=[],
        metavar="ID",
        help="要校验的目标 id（默认全部）：linux-x11 / linux-wayland / windows-x86_64",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="只做静态校验（打包脚本、依赖清单、状态字段齐全），不探测本机环境、不执行打包脚本",
    )
    parser.add_argument("--json", action="store_true", help="仅输出结果 JSON 到 stdout（人类可读行写 stderr）")
    parser.add_argument("--out", metavar="PATH", help="把结果 JSON 额外写入该文件")
    args = parser.parse_args()
    JSON_MODE = args.json

    run_id_rc, run_id_out = run(["date", "-u", "+%Y%m%dT%H%M%SZ"])
    run_id = run_id_out.strip() if run_id_rc == 0 and run_id_out.strip() else "unknown"

    doc, rc = load_manifest()
    if doc is None:
        return finish(None, [], run_id, args)
    targets, rc2 = select_targets(doc, args.targets)
    if rc2 != 0:
        return finish(doc, [], run_id, args)

    check_scripts(targets)
    check_dep_doc(doc)
    check_targets(targets)

    env = None
    if not args.dry_run:
        env = probe_environment()
        check_environment(targets, env)
    else:
        emit("[7] 本机环境探测：--dry-run 跳过（不产生任何平台结论）")

    if targets:
        negative_control(targets[0])

    emit("-" * 68)
    emit("目标状态：")
    for target in targets:
        label = target.get("status_label", target.get("status"))
        if target.get("status") == "verified":
            evidence = target.get("status_evidence") or {}
            detail = f"证据 {evidence.get('path')}（run_id={evidence.get('run_id')}, result={evidence.get('result')}）"
        else:
            detail = f"原因：{target.get('blocking_reason')}"
            if env is not None:
                extra = (
                    f"；本机 host_os={env['host_os']} session={env['session']}"
                    f"{'（与本目标环境一致，但无实测证据）' if session_matches(target, env) else '（环境不一致，无法在本机实测）'}"
                )
                detail += extra
        emit(f"  {target.get('id'):<18}{label:<6}{detail}")

    pending = [t.get("id") for t in targets if t.get("status") != "verified"]
    if pending:
        emit(f"说明：{len(pending)} 个目标为「待验证」（{', '.join(pending)}）—— 未实测，不得视为已发布。")

    return finish(doc, targets, run_id, args, env=env)


def finish(doc, targets, run_id, args, env=None) -> int:
    if FAILURES:
        result, exit_code = "FAIL", 1
        emit(f"RESULT: FAIL（{len(FAILURES)} 项违规）")
        for item in FAILURES:
            emit(f"  - {item}")
    else:
        result, exit_code = "PASS", 0
        skipped = f"（{len(NOTES)} 项 SKIP，已在上面逐条说明）" if NOTES else ""
        emit(f"RESULT: PASS{skipped} — 打包路径/依赖清单/状态字段齐全；未实测目标标记「待验证」")

    if args.json or args.out:
        payload = {
            "schema": "nr.desktop.targets.gate/1",
            "gate": "desktop-targets",
            "mode": "dry-run" if args.dry_run else "status",
            "run_id": run_id,
            "result": result,
            "exit_code": exit_code,
            "repo_root": str(REPO_ROOT),
            "command": "python3 scripts/verify-desktop-targets.py"
            + (f" --targets {' '.join(args.targets)}" if args.targets else "")
            + (" --dry-run" if args.dry_run else ""),
            "manifest": {
                "path": str(MANIFEST_PATH.relative_to(REPO_ROOT)),
                "schema": (doc or {}).get("schema"),
                "targets_total": len((doc or {}).get("targets") or []),
            },
            "requested_targets": [t.get("id") for t in targets],
            "targets": [
                {
                    "id": t.get("id"),
                    "status": t.get("status"),
                    "status_label": t.get("status_label"),
                    "packaging_mode": t.get("packaging_mode"),
                    "package_script": t.get("package_script"),
                    "blocking_reason": t.get("blocking_reason"),
                    "unverified_items": t.get("unverified_items"),
                    "status_evidence": t.get("status_evidence"),
                    "release_dir_name_pattern": t.get("release_dir_name_pattern"),
                }
                for t in targets
            ],
            "environment": env,
            "checks": CHECKS,
            "summary": {
                "checks_total": len(CHECKS),
                "checks_failed": sum(1 for c in CHECKS if not c["ok"]),
                "targets_total": len(targets),
                "targets_verified": sum(1 for t in targets if t.get("status") == "verified"),
                "targets_pending": sum(1 for t in targets if t.get("status") != "verified"),
            },
            "failures": FAILURES,
            "notes": NOTES,
        }
        text = json.dumps(payload, ensure_ascii=False, indent=2)
        if args.json:
            print(text)
        if args.out:
            out_path = Path(args.out)
            if not out_path.is_absolute():
                out_path = REPO_ROOT / out_path
            out_path.parent.mkdir(parents=True, exist_ok=True)
            out_path.write_text(text + "\n", encoding="utf-8")
            emit(f"结果 JSON 已写入：{out_path}")
    return exit_code


if __name__ == "__main__":
    sys.exit(main())

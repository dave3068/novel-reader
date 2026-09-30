#!/usr/bin/env python3
"""verify-core-scope.py — V1 生产核心的依赖边界与数据源来源检查。

用途（reading-core NFR-C1、checklist C8）：证明正式产品核心不依赖 UI / Android /
网络 / 窗口系统，且生产路径中不出现模拟（Synthetic）或空 External 生产数据源。

检查项：
  1. workspace 成员恰为 5 个生产 crate，且 `experiments/` 被 exclude（不进依赖图）；
  2. `cargo metadata` 中生产 crate 的依赖：无禁用 crate、无 workspace 外 path 依赖、
     版本一律精确锁定（`=X.Y.Z`）；
  3. `cargo tree -p reading-core --edges normal` 的解析结果无禁用 crate；
  4. 生产 crate 源码（去除注释后）无 Synthetic / 空 External / 模拟书库标记；
  5. 正向对照（自检）：同一扫描规则作用在 `experiments/` 探针源码上必须能命中，
     以证明第 4 项不是空检查（探针缺失时该项 SKIP 并说明）。

退出码：0 = 全部通过；1 = 存在违规；2 = 脚本自身前提缺失（如 cargo 不可用）。

局限说明：源码扫描是轻量词法处理（行注释、嵌套块注释、普通字符串/字符字面量），
不处理 raw string（`r#"..."#`）；若未来引入 raw string 内含违规标识符，需人工复核。
"""

from __future__ import annotations

import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]

PRODUCTION_CRATES = (
    "reading-core",
    "reading-ffi",
    "reading-index",
    "reading-source",
    "reading-storage",
)

EXPERIMENTS_DIR = "experiments"

# 禁用 crate：UI / 窗口系统 / Android / 网络 / 运行时窗口绑定。
FORBIDDEN_EXACT = {
    "slint",
    "slint-build",
    "i-slint-core",
    "i-slint-backend-winit",
    "winit",
    "tao",
    "wry",
    "gtk",
    "gtk4",
    "gdk",
    "gdk-sys",
    "glib",
    "egui",
    "iced",
    "druid",
    "qml",
    "qt",
    "qt_core",
    "flutter_rust_bridge",
    "android-activity",
    "ndk",
    "ndk-sys",
    "ndk-context",
    "jni",
    "jni-sys",
    "cargo-ndk",
    "tokio-tungstenite",
    "tungstenite",
    "reqwest",
    "hyper",
    "h2",
    "ureq",
    "isahc",
    "curl",
    "surf",
    "attohttpc",
    "quinn",
    "rustls",
    "native-tls",
    "openssl",
    "openssl-sys",
    "tonic",
    "libp2p",
    "webview2-com",
}
FORBIDDEN_PREFIX = ("slint", "i-slint", "jni", "ndk", "android", "gtk", "gdk", "qt")

# 生产路径中禁止出现的模拟/空数据源与模拟书库标记。
FORBIDDEN_SOURCE_TOKENS = (
    "SyntheticTextSource",
    "SyntheticSource",
    "SyntheticLibrary",
    "SyntheticFixture",
    "ExternalTextSource",
    "EmptyExternalSource",
    "MockLibrary",
    "mock_library",
    "模拟书库",
    "合成文本源",
)

FAILURES: list[str] = []
NOTES: list[str] = []


def ok(message: str) -> None:
    print(f"  PASS  {message}")


def fail(message: str) -> None:
    FAILURES.append(message)
    print(f"  FAIL  {message}")


def note(message: str) -> None:
    NOTES.append(message)
    print(f"  SKIP  {message}")


def normalize(name: str) -> str:
    return name.strip().lower().replace("_", "-")


def is_forbidden(name: str) -> bool:
    norm = normalize(name)
    if norm in FORBIDDEN_EXACT:
        return True
    return any(norm == p or norm.startswith(p + "-") for p in FORBIDDEN_PREFIX)


def strip_rust_comments_and_literals(text: str) -> str:
    """去除 Rust 源码中的注释与字符串/字符字面量，保留代码结构用于违规扫描。"""
    out: list[str] = []
    i = 0
    n = len(text)
    while i < n:
        ch = text[i]
        nxt = text[i + 1] if i + 1 < n else ""
        if ch == "/" and nxt == "/":
            while i < n and text[i] != "\n":
                i += 1
            continue
        if ch == "/" and nxt == "*":
            depth = 1
            i += 2
            while i < n and depth > 0:
                if text[i] == "/" and i + 1 < n and text[i + 1] == "*":
                    depth += 1
                    i += 2
                elif text[i] == "*" and i + 1 < n and text[i + 1] == "/":
                    depth -= 1
                    i += 2
                else:
                    i += 1
            continue
        if ch == '"':
            i += 1
            while i < n and text[i] != '"':
                if text[i] == "\\":
                    i += 1
                i += 1
            i += 1
            continue
        if ch == "'":
            # 字符字面量或生命周期：生命周期（'a）无需跳过，直接原样保留。
            if i + 2 < n and text[i + 2] == "'":
                i += 3
                continue
            if i + 1 < n and text[i + 1] == "\\":
                j = i + 2
                while j < n and text[j] != "'":
                    j += 1
                i = j + 1
                continue
            out.append(ch)
            i += 1
            continue
        out.append(ch)
        i += 1
    return "".join(out)


def cargo(args: list[str]) -> subprocess.CompletedProcess:
    return subprocess.run(
        args,
        cwd=str(REPO_ROOT),
        capture_output=True,
        text=True,
        check=False,
    )


def check_workspace_membership() -> None:
    print("[1] workspace 成员与 experiments 排除")
    manifest = REPO_ROOT / "Cargo.toml"
    if not manifest.is_file():
        fail(f"缺少根 manifest: {manifest}")
        return
    text = manifest.read_text(encoding="utf-8")
    workspace_section = text.split("[workspace", 1)[-1]
    if not re.search(r"exclude\s*=\s*\[[^\]]*" + EXPERIMENTS_DIR, workspace_section, re.S):
        fail(
            "[workspace].exclude 必须包含 "
            f"`{EXPERIMENTS_DIR}`，否则验证探针会进入正式依赖图"
        )
    else:
        ok(f"[workspace].exclude 已包含 `{EXPERIMENTS_DIR}`")

    members = re.findall(r'"([^"]+)"', re.search(r"members\s*=\s*\[([^\]]*)\]", workspace_section, re.S).group(1)) if re.search(r"members\s*=\s*\[([^\]]*)\]", workspace_section, re.S) else []
    member_names = sorted(Path(m).name for m in members)
    if member_names != sorted(PRODUCTION_CRATES):
        fail(f"[workspace].members 应为 {sorted(PRODUCTION_CRATES)}，实际 {member_names}")
    else:
        ok(f"[workspace].members 恰为 5 个生产 crate: {member_names}")

    for member in members:
        if EXPERIMENTS_DIR in Path(member).parts:
            fail(f"生产 workspace 成员不得位于 `{EXPERIMENTS_DIR}/`: {member}")


def check_manifests_and_metadata() -> None:
    print("[2] crate manifest 依赖边界与版本锁定（cargo metadata）")
    if shutil.which("cargo") is None:
        fail("cargo 不可用，无法执行依赖边界检查")
        return
    proc = cargo(["cargo", "metadata", "--format-version", "1", "--no-deps"])
    if proc.returncode != 0:
        fail(f"cargo metadata 失败(exit {proc.returncode}): {proc.stderr.strip()[:400]}")
        return
    meta = json.loads(proc.stdout)
    packages = {pkg["name"]: pkg for pkg in meta["packages"]}
    # `workspace_members` 存的是 package id（如 `path+file:///...#0.1.0`），必须经 id 映射取名字，
    # 否则会退化成空列表并掩盖成员检查失效。
    by_id = {pkg["id"]: pkg["name"] for pkg in meta["packages"]}
    member_names = sorted(
        by_id[member] for member in meta["workspace_members"] if member in by_id
    )

    if member_names != sorted(PRODUCTION_CRATES):
        fail(
            f"cargo metadata workspace 成员应为 {sorted(PRODUCTION_CRATES)}，"
            f"实际 {member_names}"
        )
    else:
        ok(f"cargo metadata workspace 成员恰为 5 个生产 crate: {member_names}")

    non_members = sorted(name for name in packages if name not in set(member_names))
    if non_members:
        fail(f"cargo metadata 中出现非成员包（可能引入探针）: {non_members}")

    for required in PRODUCTION_CRATES:
        if required not in packages:
            fail(f"生产 crate 缺失: {required}")
    for name, pkg in sorted(packages.items()):
        if EXPERIMENTS_DIR in Path(pkg["manifest_path"]).parts:
            fail(
                f"`{EXPERIMENTS_DIR}/` 下的探针不得成为 workspace 成员: "
                f"{pkg['manifest_path']}"
            )

    workspace_names = set(packages)
    for name, pkg in sorted(packages.items()):
        for dep in pkg["dependencies"]:
            dep_name = dep["name"]
            if is_forbidden(dep_name):
                fail(f"{name} 依赖禁用 crate: {dep_name}")
            dep_path = dep.get("path")
            if dep_path:
                resolved = Path(dep_path).resolve()
                try:
                    rel = resolved.relative_to(REPO_ROOT)
                except ValueError:
                    fail(f"{name} 的 path 依赖越出仓库: {dep_path}")
                    continue
                if EXPERIMENTS_DIR in rel.parts:
                    fail(f"{name} 的 path 依赖指向验证探针: {rel}")
                if dep_name not in workspace_names:
                    fail(f"{name} 的 path 依赖不是本 workspace 生产 crate: {dep_name}")
            req = (dep.get("req") or "").strip()
            if not req:
                continue
            if not req.startswith("="):
                fail(
                    f"{name} -> {dep_name} 使用浮动版本约束 `{req}`；"
                    '必须锁定为精确版本（`version = "=X.Y.Z"`）'
                )
    if not FAILURES:
        ok("所有生产 crate 依赖均为精确锁定版本，且无禁用/探针依赖")


def check_core_tree() -> None:
    print("[3] cargo tree -p reading-core --edges normal")
    if shutil.which("cargo") is None:
        fail("cargo 不可用，无法执行 cargo tree")
        return
    proc = cargo(["cargo", "tree", "-p", "reading-core", "--edges", "normal"])
    if proc.returncode != 0:
        fail(f"cargo tree 失败(exit {proc.returncode}): {proc.stderr.strip()[:400]}")
        return
    tree = proc.stdout
    found: list[str] = []
    for line in tree.splitlines():
        match = re.search(r"([A-Za-z0-9_.-]+) v[0-9]", line)
        if not match:
            continue
        name = normalize(match.group(1))
        if is_forbidden(name):
            found.append(name)
    if found:
        fail(f"reading-core 依赖树包含禁用 crate: {sorted(set(found))}")
    else:
        roots = [ln.strip() for ln in tree.splitlines() if ln.strip()]
        ok(f"reading-core 依赖树无禁用 crate（{len(roots)} 行解析通过）")

    # 核心也不得经由 path 依赖引入 experiments/ 内容。
    if EXPERIMENTS_DIR in tree:
        fail(f"reading-core 依赖树出现 `{EXPERIMENTS_DIR}/` 路径")
    else:
        ok(f"reading-core 依赖树不含 `{EXPERIMENTS_DIR}/` 路径")


def production_sources() -> list[Path]:
    files: list[Path] = []
    for crate in PRODUCTION_CRATES:
        src = REPO_ROOT / "crates" / crate / "src"
        if src.is_dir():
            files.extend(sorted(src.rglob("*.rs")))
    return files


def scan_tokens(paths: list[Path]) -> list[tuple[Path, int, str]]:
    hits: list[tuple[Path, int, str]] = []
    for path in paths:
        code = strip_rust_comments_and_literals(path.read_text(encoding="utf-8", errors="replace"))
        for lineno, line in enumerate(code.splitlines(), start=1):
            for token in FORBIDDEN_SOURCE_TOKENS:
                if token in line:
                    hits.append((path, lineno, token))
    return hits


def check_production_sources() -> None:
    print("[4] 生产核心源码中的模拟/空数据源标记")
    files = production_sources()
    if not files:
        fail("未找到生产 crate 源码文件，检查范围无效")
        return
    hits = scan_tokens(files)
    if hits:
        for path, lineno, token in hits[:20]:
            fail(f"{path.relative_to(REPO_ROOT)}:{lineno} 出现禁用数据源标记 `{token}`")
    else:
        ok(f"{len(files)} 个生产源文件均无 Synthetic/空 External/模拟书库标记")


def check_detector_positive_control() -> None:
    print("[5] 正向对照：扫描规则在 experiments/ 探针上必须命中（自检非空检查）")
    experiment_sources = sorted((REPO_ROOT / EXPERIMENTS_DIR).rglob("*.rs"))
    if not experiment_sources:
        note(f"`{EXPERIMENTS_DIR}/` 无 Rust 源码，跳过正向对照（无法证明扫描灵敏度）")
        return
    hits = scan_tokens(experiment_sources)
    if hits:
        token = hits[0][2]
        ok(f"扫描规则在 {len(hits)} 处命中探针源码（如 `{token}`），证明检测非空")
    else:
        fail(
            "扫描规则在探针源码上零命中，说明第 4 项为空检查，"
            "无法证明生产路径无模拟/空 External 数据源"
        )


def main() -> int:
    print(f"verify-core-scope: repo={REPO_ROOT}")
    check_workspace_membership()
    check_manifests_and_metadata()
    check_core_tree()
    check_production_sources()
    check_detector_positive_control()
    print("-" * 60)
    if FAILURES:
        print(f"RESULT: FAIL ({len(FAILURES)} 项违规)")
        for item in FAILURES:
            print(f"  - {item}")
        return 1
    print("RESULT: PASS — 核心无 UI/Android/网络依赖，无 Synthetic/空 External 生产数据源")
    return 0


if __name__ == "__main__":
    sys.exit(main())

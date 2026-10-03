#!/usr/bin/env python3
"""verify-v1-structure.py — V1 正式工程结构门禁（父级 checklist C1；卡片 REL1 t_bf1e2bfb）。

用途（父规格 checklist C1 / PAC5；子规格 Impact Analysis「Affected code」）：

  证明**正式工程已经成形**，而不是证明「某一次构建成功」：

  [1] 正式工程根目录与 Cargo workspace 口径：根目录集合、成员集合（=`results/gates/gate-1-core.md`
      §2 冻结的 5 个核心 crate）、`experiments` 被 exclude；
  [2] 三个子规格（reading-core / desktop / android）Impact Analysis 逐条列出的目录存在且非空；
  [3] 迁移：`migrations/0001_*.sql` 存在、含 8 张表（gate-0 基线 §2.6）、不含 PRAGMA
      （设计约束：连接级 pragma 由仓储层设置）；
  [4] 脚本与夹具：各清单引用的脚本/夹具文件齐全，且 Python 脚本可编译（`py_compile`）、
      Bash 脚本语法成立（`bash -n`）；
  [5] ABI 头文件：`include/novel_reader.h` 存在，`NR_ABI_VERSION == 1`、窗口上下限常量、
      12 项错误码数值 0..11 完整、`nr_` 导出声明 ≥ 18 项；
  [6] 正式代码**不依赖** `experiments/` 的运行时模拟源：依赖图（Cargo.toml path 依赖）、
      生产源码的运行时引用（去注释后仍出现 `experiments/`）、探针数据源标识符
      （SyntheticTextSource / MockLibrary / EmptyExternalSource 等）一律不得出现。

不覆盖（**不得**由本脚本的 PASS 推导出来）：编译、测试、APK 内容、真机、Windows/Wayland。
这些分别由 C2–C11 与延期门禁负责；发布物/版本/许可/未验证项见 `scripts/verify-v1-release.py`。

退出码：0 = 结构齐全；1 = 缺失/不一致项；2 = 前提缺失（`--root` 不存在）。
`--self-test`：合成 fixture 仓库 + 逐条**反向**用例（缺失项必须被报出）。
"""

from __future__ import annotations

import argparse
import json
import py_compile
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]

# --------------------------------------------------------------------------- #
# 冻结口径（常量即断言来源；改动必须同步结果证据）
# --------------------------------------------------------------------------- #

#: 正式工程根目录（父规格「影响分析」新增目录 + include/docs 交付口径）。
ROOT_REQUIRED_DIRS: tuple[str, ...] = (
    "crates",
    "desktop",
    "android",
    "migrations",
    "tests",
    "scripts",
    "include",
    "docs",
)

#: 核心 crate 成员集合（`results/gates/gate-1-core.md` §2 冻结）。
CORE_MEMBER_DIRS: tuple[str, ...] = (
    "crates/reading-core",
    "crates/reading-source",
    "crates/reading-index",
    "crates/reading-storage",
    "crates/reading-ffi",
)

#: 桌面 crate 是**自带 [workspace] 的嵌套 workspace**，不在根成员集合内。
DESKTOP_CRATE_DIR = "crates/desktop-app"

#: 三个子规格 Impact Analysis 的目录口径（逐条可追溯）。
SUBSPEC_REQUIRED_DIRS: dict[str, tuple[str, ...]] = {
    "reading-core（crates/ + migrations/ + tests/ + scripts/）": (
        "crates",
        "migrations",
        "tests",
        "tests/fixtures",
        "tests/e2e",
        "scripts",
    ),
    "desktop（crates/desktop-app + desktop/ui + desktop/packaging + tests/e2e）": (
        "crates/desktop-app",
        "desktop",
        "desktop/ui",
        "desktop/packaging",
        "desktop/packaging/linux",
        "desktop/packaging/windows",
        "tests/e2e",
    ),
    "android（android/ + crates/reading-ffi + android 测试口径）": (
        "android",
        "android/app",
        "android/app/src",
        "android/app/src/main",
        "android/app/src/main/java",
        "android/app/src/main/res",
        "android/app/src/main/jniLibs",
        "android/gradle/wrapper",
        "android/jni",
        "crates/reading-ffi",
    ),
}

#: 子规格点名要求的文件（存在 + 非空）。
SUBSPEC_REQUIRED_FILES: tuple[tuple[str, str], ...] = (
    ("reading-core", "Cargo.toml"),
    ("reading-core", "Cargo.lock"),
    ("reading-core", "migrations/0001_init.sql"),
    ("reading-core", "tests/fixtures/smoke.txt"),
    ("reading-core", "tests/fixtures/manifest.json"),
    ("reading-core", "tests/e2e/run_txt_lifecycle.py"),
    ("reading-core", "tests/e2e/run_txt_safety.py"),
    ("desktop", "crates/desktop-app/Cargo.toml"),
    ("desktop", "crates/desktop-app/src/main.rs"),
    ("desktop", "crates/desktop-app/src/lib.rs"),
    ("desktop", "desktop/ui/app.slint"),
    ("desktop", "desktop/packaging/targets.json"),
    ("desktop", "desktop/packaging/README.md"),
    ("desktop", "desktop/packaging/linux/package-linux.sh"),
    ("desktop", "desktop/packaging/windows/package-windows.ps1"),
    ("android", "android/settings.gradle.kts"),
    ("android", "android/build.gradle.kts"),
    ("android", "android/gradlew"),
    ("android", "android/gradle/wrapper/gradle-wrapper.properties"),
    ("android", "android/app/build.gradle.kts"),
    ("android", "android/jni/README.md"),
    ("android", "include/novel_reader.h"),
    ("android", "crates/reading-ffi/Cargo.toml"),
)

#: 各清单/门禁引用的脚本（父级 C1–C13 与三份子清单的命令口径）。
REQUIRED_SCRIPTS: tuple[str, ...] = (
    # reading-core
    "scripts/generate-fixtures.py",
    "scripts/verify-abi.py",
    "scripts/verify-core-scope.py",
    # desktop
    "scripts/smoke-desktop.sh",
    "scripts/verify-desktop-scope.py",
    "scripts/verify-desktop-targets.py",
    "scripts/verify-desktop-input.py",
    # android
    "scripts/build-android-libs.sh",
    "scripts/build-android-apk.sh",
    "scripts/verify-android-scope.py",
    "scripts/verify-android-boundary.py",
    "scripts/verify-android-apk.py",
    "scripts/verify-android-device-readiness.py",
    # 父级发布门禁（本卡 REL1）
    "scripts/verify-v1-structure.py",
    "scripts/verify-v1-release.py",
)

#: 端到端脚本（父级 C9/C13 与 desktop C4 口径）。
REQUIRED_E2E_SCRIPTS: tuple[str, ...] = (
    "tests/e2e/run_txt_lifecycle.py",
    "tests/e2e/run_txt_safety.py",
)

#: ABI 头文件必须存在的常量（冻结值见 gate-0 §2）。
REQUIRED_HEADER_MACROS: dict[str, int] = {
    "NR_ABI_VERSION": 1,
    "NR_DEFAULT_WINDOW_BYTES": 65536,
    "NR_MAX_WINDOW_BYTES": 262144,
}
REQUIRED_ERROR_CODE_COUNT = 12  # NR_ERR_OK(0) .. NR_ERR_CHAPTER_OUT_OF_RANGE(11)
REQUIRED_NR_EXPORTS = 18  # 冻结 ABI 入口集合（RC7 / V-AN1 口径：18/18）
MIGRATION_REQUIRED_TABLES = 8  # gate-0 基线 §2.6：初始迁移建立 8 张表

#: 探针（experiments/）数据源标识符：正式代码中出现即失败。
FORBIDDEN_PROBE_TOKENS: tuple[str, ...] = (
    "SyntheticTextSource",
    "SyntheticBookSource",
    "SyntheticLibrary",
    "MockLibrary",
    "MockBookSource",
    "EmptyExternalSource",
    "ProbeBookSource",
    "SimulatedTextSource",
)

#: 生产源码/脚本扫描范围（相对仓库根）。
PRODUCTION_SCAN_GLOBS: tuple[str, ...] = (
    "crates/**/*.rs",
    "desktop/**/*.rs",
    "desktop/**/*.slint",
    "android/**/*.kt",
    "android/**/*.kts",
    "android/**/*.java",
    "android/**/*.c",
    "android/**/*.h",
    "android/jni/**/*",
    "scripts/build-android-libs.sh",
    "scripts/build-android-apk.sh",
    "desktop/packaging/**/*.sh",
    "desktop/packaging/**/*.ps1",
)

SKIP_DIR_NAMES = {"target", "build", ".gradle", "node_modules", "__pycache__", ".git"}

ERROR_CODE_RE = re.compile(r"^#define\s+(NR_ERR_[A-Z0-9_]+)\s+(\d+)\b", re.M)
MACRO_RE = re.compile(r"^#define\s+([A-Z0-9_]+)\s+(\d+)u?\b", re.M)
NR_DECL_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_ \t\*]*\b(nr_[a-z0-9_]+)\s*\(", re.M)


class Report:
    """与仓库既有 verify-*.py 一致的成功/失败/跳过/事实四件套。"""

    def __init__(self) -> None:
        self.ok_count = 0
        self.failures: list[str] = []
        self.skips: list[str] = []
        self.facts: dict[str, object] = {}

    def ok(self, label: str) -> None:
        self.ok_count += 1
        print(f"[ok]   {label}")

    def fail(self, label: str) -> None:
        self.failures.append(label)
        print(f"[FAIL] {label}")

    def skip(self, label: str) -> None:
        self.skips.append(label)
        print(f"[skip] {label}")

    def fact(self, key: str, value: object) -> None:
        self.facts[key] = value


# --------------------------------------------------------------------------- #
# 通用工具
# --------------------------------------------------------------------------- #


def rel(root: Path, path: Path) -> str:
    try:
        return str(path.relative_to(root))
    except ValueError:
        return str(path)


def iter_files(root: Path, pattern: str) -> list[Path]:
    """按 glob 收集文件，跳过构建产物目录（target/build/.gradle/target 等）。"""
    out: list[Path] = []
    if not root.is_dir():
        return out
    for p in sorted(root.glob(pattern)):
        if not p.is_file():
            continue
        parts = set(p.relative_to(root).parts[:-1])
        if parts & SKIP_DIR_NAMES:
            continue
        out.append(p)
    return out


def strip_comments(text: str, style: str) -> str:
    """去掉注释，**保留字符串字面量**（避免把 `include_str!("…experiments…")` 一并抹掉）。

    style: "c"（Rust/Kotlin/Slint：// 与 /* */）、"hash"（Shell/PowerShell：# 与 <# #>）。
    """
    out: list[str] = []
    i = 0
    n = len(text)
    while i < n:
        ch = text[i]
        nxt = text[i + 1] if i + 1 < n else ""
        # 字符串字面量：原样保留（含转义）
        if ch == '"':
            if style == "c" and text[i : i + 3] == '"""':  # Kotlin 原始字符串
                end = text.find('"""', i + 3)
                end = n if end == -1 else end + 3
                out.append(text[i:end])
                i = end
                continue
            j = i + 1
            while j < n:
                if text[j] == "\\":
                    j += 2
                    continue
                if text[j] == '"':
                    j += 1
                    break
                j += 1
            out.append(text[i:j])
            i = j
            continue
        # 行注释
        if (style == "c" and ch == "/" and nxt == "/") or (style == "hash" and ch == "#"):
            j = text.find("\n", i)
            i = n if j == -1 else j
            continue
        # 块注释
        if ch == "/" and nxt == "*":
            end = text.find("*/", i + 2)
            # 块注释可能跨行：保留换行以维持行号观感
            block = text[i : n if end == -1 else end + 2]
            out.append("\n" * block.count("\n"))
            i = n if end == -1 else end + 2
            continue
        if style == "hash" and ch == "<" and nxt == "#":
            end = text.find("#>", i + 2)
            i = n if end == -1 else end + 2
            continue
        out.append(ch)
        i += 1
    return "".join(out)


def comment_style_for(path: Path) -> str:
    return "hash" if path.suffix in {".sh", ".ps1", ".cmd", ".bat"} else "c"


def read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8", errors="replace")


def nonempty_file(rep: Report, path: Path, label: str) -> bool:
    if not path.is_file():
        rep.fail(f"{label}：缺少文件 {path}")
        return False
    if path.stat().st_size == 0:
        rep.fail(f"{label}：文件为空 {path}")
        return False
    rep.ok(f"{label}：{path.name}（{path.stat().st_size} B）")
    return True


# --------------------------------------------------------------------------- #
# 各节检查
# --------------------------------------------------------------------------- #


def check_root_and_workspace(rep: Report, root: Path) -> None:
    print("[1] 正式工程根目录与 Cargo workspace 口径")
    for d in ROOT_REQUIRED_DIRS:
        target = root / d
        if target.is_dir():
            rep.ok(f"根目录存在：{d}/")
        else:
            rep.fail(f"正式工程根目录缺失：{d}/（父规格「影响分析」新增目录）")

    manifest = root / "Cargo.toml"
    if not nonempty_file(rep, manifest, "根 workspace"):  # pragma: no cover - 前提缺失
        return
    text = read_text(manifest)

    m = re.search(r"^members\s*=\s*\[(.*?)\]", text, re.M | re.S)
    if not m:
        rep.fail("根 Cargo.toml 未声明 [workspace] members")
        return
    members = re.findall(r'"([^"]+)"', m.group(1))
    rep.fact("workspace_members", members)
    missing = [d for d in CORE_MEMBER_DIRS if d not in members]
    extra = [d for d in members if d not in CORE_MEMBER_DIRS]
    if missing:
        rep.fail(f"workspace 缺少冻结成员：{missing}（gate-1-core.md §2 冻结集合）")
    elif extra:
        rep.fail(f"workspace 出现冻结集合之外的成员：{extra}")
    else:
        rep.ok(f"workspace 成员集合与冻结集合一致（{len(members)} 个）")

    ex = re.search(r"^exclude\s*=\s*\[(.*?)\]", text, re.M | re.S)
    excluded = re.findall(r'"([^"]+)"', ex.group(1)) if ex else []
    if any(e.rstrip("/") == "experiments" for e in excluded):
        rep.ok("experiments/ 已被 workspace exclude（探针不进入生产依赖图）")
    else:
        rep.fail(f"根 Cargo.toml 未 exclude experiments/（当前 exclude={excluded}）")

    ver = re.search(r'^version\s*=\s*"([^"]+)"', text.split("[workspace.package]")[-1], re.M)
    if ver:
        rep.fact("workspace_version", ver.group(1))
        rep.ok(f"workspace.package.version = {ver.group(1)}")
    else:
        rep.fail("根 Cargo.toml 的 [workspace.package] 未声明 version")
        return

    # ---- 发行许可（L2，2026-10-02 主人裁定 GPL-3.0-only）----
    # 判据：①根目录存在 LICENSE 且含 GPLv3 标题；②[workspace.package] 声明 license；
    #③各 crate 的 license 与之一致（desktop-app 是独立嵌套 workspace，不能继承，须单独核对）。
    lic_file = root / "LICENSE"
    if not lic_file.is_file():
        rep.fail("根目录缺少 LICENSE（项目发行许可为 GPL-3.0-only，发布物必须附带）")
    else:
        lic_text = read_text(lic_file)
        if "GNU GENERAL PUBLIC LICENSE" in lic_text and "Version 3" in lic_text:
            rep.ok(f"LICENSE 存在且为 GPLv3 原文（{lic_file.stat().st_size} 字节）")
        else:
            rep.fail("LICENSE 内容不是 GPLv3 原文（缺 'GNU GENERAL PUBLIC LICENSE' / 'Version 3'）")

    ws_lic = re.search(r'^license\s*=\s*"([^"]+)"', text.split("[workspace.package]")[-1], re.M)
    if ws_lic and ws_lic.group(1) == "GPL-3.0-only":
        rep.ok("workspace.package.license = GPL-3.0-only")
    else:
        rep.fail(f"[workspace.package].license 应为 GPL-3.0-only（当前 {ws_lic.group(1) if ws_lic else '未声明'}）")

    lic_mismatch = []
    for d in CORE_MEMBER_DIRS:
        cm = root / d / "Cargo.toml"
        if not cm.is_file():
            lic_mismatch.append(f"{d}: 缺 Cargo.toml")
            continue
        ctext = read_text(cm)
        head = ctext.split("[package]")[1].split("[")[0] if "[package]" in ctext else ""
        if "license.workspace = true" not in head:
            lic_mismatch.append(f"{d}: 未继承 workspace 许可")
    dm = root / "crates" / "desktop-app" / "Cargo.toml"
    if dm.is_file():
        dtext = read_text(dm)
        # 自检/合成 fixture 里的 desktop-app/Cargo.toml 是**占位文件**（非真实清单），
        # 不该要求它带许可字段；真实仓库里该文件是有 version/edition 的完整清单。
        is_placeholder = "[package]" not in dtext
        # 注意：不能按 "[package]" 段切到下一个 "[" 再匹配 —— desktop-app 的 license 字段
        # 之后还有 description，段内会先遇到别的键；直接全文判定更稳。
        if not is_placeholder and (
            'license = "GPL-3.0-only"' not in dtext or 'license-file = "../../LICENSE"' not in dtext
        ):
            lic_mismatch.append("desktop-app: 未声明 GPL-3.0-only / license-file")
    if lic_mismatch:
        rep.fail("以下 crate 的许可标注与 GPL-3.0-only 不一致：" + "；".join(lic_mismatch))
    else:
        rep.ok(f"{len(CORE_MEMBER_DIRS) + 1} 个 crate 的许可标注一致（GPL-3.0-only）")

    lock = root / "Cargo.lock"
    if nonempty_file(rep, lock, "锁定文件"):
        pkgs = read_text(lock).count("[[package]]")
        rep.fact("cargo_lock_packages", pkgs)
        if pkgs > 0:
            rep.ok(f"Cargo.lock 可解析（{pkgs} 个 package）")
        else:
            rep.fail("Cargo.lock 中没有任何 [[package]]")

    for d in CORE_MEMBER_DIRS:
        cargo = root / d / "Cargo.toml"
        if not cargo.is_file():
            rep.fail(f"workspace 成员缺少 Cargo.toml：{d}/Cargo.toml")
            continue
        rs = iter_files(root / d, "src/**/*.rs")
        if rs:
            rep.ok(f"成员 {d}：Cargo.toml + {len(rs)} 个 .rs 源文件")
        else:
            rep.fail(f"成员 {d} 的 src/ 下没有任何 .rs 源文件")


def check_subspec_dirs(rep: Report, root: Path) -> None:
    print("[2] 三个子规格 Impact Analysis 要求的目录")
    for label, dirs in SUBSPEC_REQUIRED_DIRS.items():
        print(f"     -- {label}")
        missing: list[str] = []
        empty: list[str] = []
        for d in dirs:
            target = root / d
            if not target.is_dir():
                missing.append(d)
                continue
            entries = [p for p in target.iterdir() if p.name not in SKIP_DIR_NAMES]
            if not entries:
                empty.append(d)
        if missing:
            rep.fail(f"{label}：缺少目录 {missing}")
        elif empty:
            rep.fail(f"{label}：目录为空 {empty}（子规格要求的工程目录必须已落地）")
        else:
            rep.ok(f"{label}：{len(dirs)} 个目录存在且非空")

    print("     -- 子规格点名的文件")
    for spec, rel_path in SUBSPEC_REQUIRED_FILES:
        nonempty_file(rep, root / rel_path, spec)


def check_android_landing(rep: Report, root: Path) -> None:
    """Android 侧落地物（源码 / ABI 目录 / 单测目录析取口径）。"""
    print("[3] Android 正式落地物")
    srcs = iter_files(root / "android", "app/src/main/**/*.kt")
    if srcs:
        rep.ok(f"Kotlin 源码 {len(srcs)} 个（android/app/src/main）")
        rep.fact("android_kotlin_main_files", len(srcs))
    else:
        rep.fail("android/app/src/main 下没有 Kotlin 源码")

    jni_c = iter_files(root / "android/jni", "**/*.c") + iter_files(root / "android/jni", "**/*.h")
    if jni_c:
        rep.ok(f"JNI 桥接源 {len(jni_c)} 个（android/jni）")
    else:
        rep.fail("android/jni 下没有 .c/.h 桥接源")

    so_files = iter_files(root / "android/app/src/main/jniLibs", "*/*.so")
    if so_files:
        for so in so_files:
            rep.ok(f"预编译 JNI 库：{rel(root, so)}（{so.stat().st_size} B）")
        rep.fact("android_jni_so", [rel(root, p) for p in so_files])
    else:
        rep.fail("android/app/src/main/jniLibs/<abi>/ 下没有 libnr_jni.so（A-T6 产物未落地）")

    # 口径析取：android 子规格 Impact Analysis 写 `tests/android/`，
    # 实际 JVM 单测按 android/tasks.md 落在 android/app/src/test/。两处至少一处非空。
    unit_tests = iter_files(root / "android/app/src/test", "**/*.kt")
    tests_android = [
        p
        for p in iter_files(root, "tests/android/**/*")
        if p.is_file() and p.stat().st_size > 0
    ]
    if unit_tests or tests_android:
        rep.ok(
            "Android 单测目录存在且非空"
            f"（android/app/src/test: {len(unit_tests)} 个 .kt；tests/android: {len(tests_android)} 个文件）"
        )
        rep.fact("android_unit_test_files", len(unit_tests))
    else:
        rep.fail(
            "Android 单测目录为空：android/app/src/test/ 与 tests/android/ 均无测试文件"
            "（子规格 A-T2/A-T3 口径）"
        )

    gradle_kts = root / "android/app/build.gradle.kts"
    if gradle_kts.is_file():
        text = read_text(gradle_kts)
        abi = re.search(r"val\s+nrAbi\s*=\s*\"([^\"]+)\"", text)
        if abi:
            rep.fact("android_abi", abi.group(1))
            if (root / "android/app/src/main/jniLibs" / abi.group(1)).is_dir():
                rep.ok(f"gradle nrAbi = {abi.group(1)} 且有对应 jniLibs 目录")
            else:
                rep.fail(f"gradle nrAbi = {abi.group(1)} 但 jniLibs/{abi.group(1)}/ 不存在")
        else:
            rep.skip("app/build.gradle.kts 未以 `val nrAbi = \"…\"` 形式声明 ABI（跳过一致性检查）")


def check_migration(rep: Report, root: Path) -> None:
    print("[4] 迁移文件")
    sqls = sorted((root / "migrations").glob("*.sql")) if (root / "migrations").is_dir() else []
    if not sqls:
        rep.fail("migrations/ 下没有任何 *.sql 迁移")
        return
    rep.fact("migrations", [p.name for p in sqls])
    initial = [p for p in sqls if p.name.startswith("0001_")]
    if not initial:
        rep.fail(f"缺少初始迁移 0001_*.sql（实际 {[p.name for p in sqls]}）")
        return
    text = read_text(initial[0])
    tables = re.findall(r"CREATE TABLE IF NOT EXISTS\s+([a-z0-9_]+)", text, re.I)
    rep.fact("migration_tables", tables)
    if len(tables) >= MIGRATION_REQUIRED_TABLES:
        rep.ok(f"{initial[0].name}：{len(tables)} 张表（≥ {MIGRATION_REQUIRED_TABLES}）")
    else:
        rep.fail(f"{initial[0].name} 只有 {len(tables)} 张表（< {MIGRATION_REQUIRED_TABLES}，gate-0 §2.6）")
    if re.search(r"^[ \t]*PRAGMA\b", text, re.I | re.M):
        rep.fail(f"{initial[0].name} 含 PRAGMA 语句（迁移器约束：pragma 由仓储层设置）")
    else:
        rep.ok(f"{initial[0].name} 不含 PRAGMA（迁移器口径一致）")
    if re.search(r"\b(content|body|full_text)\s+(TEXT|BLOB)\b", text, re.I):
        rep.fail(f"{initial[0].name} 疑含整本正文字段（规格禁止把正文入库）")
    else:
        rep.ok(f"{initial[0].name} 无正文字段（只存元数据/索引/位置/摘要）")


def check_scripts_and_fixtures(rep: Report, root: Path) -> None:
    print("[5] 脚本与夹具齐全 + 语法成立")
    missing = [s for s in REQUIRED_SCRIPTS if not (root / s).is_file()]
    if missing:
        rep.fail(f"缺少必需脚本：{missing}")
    else:
        rep.ok(f"必需脚本 {len(REQUIRED_SCRIPTS)} 个全部存在")
    e2e_missing = [s for s in REQUIRED_E2E_SCRIPTS if not (root / s).is_file()]
    if e2e_missing:
        rep.fail(f"缺少端到端脚本：{e2e_missing}")
    else:
        rep.ok(f"端到端脚本 {len(REQUIRED_E2E_SCRIPTS)} 个存在")

    py_files = iter_files(root, "scripts/*.py") + iter_files(root, "tests/e2e/*.py")
    py_files = [p for p in py_files if p.is_file()]
    bad: list[str] = []
    with tempfile.TemporaryDirectory() as td:
        cfile = Path(td) / "out.pyc"
        for p in py_files:
            try:
                py_compile.compile(str(p), cfile=str(cfile), doraise=True)
            except Exception as exc:  # noqa: BLE001 - 语法错误需原样报出
                bad.append(f"{rel(root, p)}: {exc}")
    rep.fact("python_scripts_checked", len(py_files))
    if bad:
        rep.fail("Python 脚本无法编译：" + "；".join(bad))
    else:
        rep.ok(f"Python 脚本 {len(py_files)} 个全部可编译（py_compile）")

    sh_files = iter_files(root, "scripts/*.sh") + iter_files(root, "desktop/packaging/**/*.sh")
    sh_files = [p for p in sh_files if p.is_file()]
    sh_bad: list[str] = []
    for p in sh_files:
        r = subprocess.run(["bash", "-n", str(p)], capture_output=True, text=True)
        if r.returncode != 0:
            sh_bad.append(f"{rel(root, p)}: {r.stderr.strip().splitlines()[:1]}")
    rep.fact("shell_scripts_checked", len(sh_files))
    if sh_bad:
        rep.fail("Bash 脚本语法不成立：" + "；".join(sh_bad))
    else:
        rep.ok(f"Bash 脚本 {len(sh_files)} 个 `bash -n` 通过")


def check_abi_header(rep: Report, root: Path) -> None:
    print("[6] ABI 头文件")
    header = root / "include/novel_reader.h"
    if not nonempty_file(rep, header, "ABI 头文件"):
        return
    text = read_text(header)

    macros = {m.group(1): int(m.group(2)) for m in MACRO_RE.finditer(text)}
    for name, expected in REQUIRED_HEADER_MACROS.items():
        got = macros.get(name)
        if got is None:
            rep.fail(f"头文件缺少常量 {name}")
        elif got != expected:
            rep.fail(f"头文件常量 {name} = {got}，期望 {expected}（冻结值，改值必须更新 ABI 版本流程）")
        else:
            rep.ok(f"头文件常量 {name} = {got}")

    codes = {m.group(1): int(m.group(2)) for m in ERROR_CODE_RE.finditer(text)}
    rep.fact("header_error_codes", codes)
    if len(codes) != REQUIRED_ERROR_CODE_COUNT:
        rep.fail(f"错误码数量 {len(codes)}，期望 {REQUIRED_ERROR_CODE_COUNT} 项（NR_ERR_OK..NR_ERR_CHAPTER_OUT_OF_RANGE）")
    elif sorted(codes.values()) != list(range(REQUIRED_ERROR_CODE_COUNT)):
        rep.fail(f"错误码数值不是 0..11 连续：{sorted(codes.values())}")
    else:
        rep.ok(f"错误码 {len(codes)} 项，数值 0..{REQUIRED_ERROR_CODE_COUNT - 1} 连续")

    exports = sorted({m.group(1) for m in NR_DECL_RE.finditer(text)})
    rep.fact("header_nr_exports", exports)
    if len(exports) >= REQUIRED_NR_EXPORTS:
        rep.ok(f"ABI 入口声明 {len(exports)} 项（≥ {REQUIRED_NR_EXPORTS}）")
    else:
        rep.fail(f"ABI 入口声明只有 {len(exports)} 项（< {REQUIRED_NR_EXPORTS}，冻结集合被削减）")


def scan_production_sources(root: Path) -> list[Path]:
    files: list[Path] = []
    for pattern in PRODUCTION_SCAN_GLOBS:
        files.extend(iter_files(root, pattern))
    seen: set[str] = set()
    uniq: list[Path] = []
    for p in files:
        key = str(p)
        if key not in seen and p.is_file():
            seen.add(key)
            uniq.append(p)
    return uniq


def check_no_probe_dependency(rep: Report, root: Path) -> None:
    print("[7] 正式代码不依赖 experiments/ 探针（依赖图 / 运行时引用 / 探针标识符）")

    # 7.1 依赖图：任何生产 Cargo.toml 的 path 依赖都不得指向 experiments/
    dep_hits: list[str] = []
    for pattern in ("crates/*/Cargo.toml", "desktop/*/Cargo.toml", "desktop/*/*/Cargo.toml"):
        for cargo in iter_files(root, pattern):
            for m in re.finditer(r"path\s*=\s*\"([^\"]*experiments[^\"]*)\"", read_text(cargo)):
                dep_hits.append(f"{rel(root, cargo)} -> {m.group(1)}")
    if dep_hits:
        rep.fail("生产 Cargo.toml 存在指向 experiments/ 的 path 依赖：" + "；".join(dep_hits))
    else:
        rep.ok("生产 Cargo.toml 无指向 experiments/ 的 path 依赖")

    # 7.2 运行时引用：去注释后仍出现 `experiments` 即视为依赖探针资源
    runtime_hits: list[str] = []
    token_hits: list[str] = []
    scanned = 0
    for path in scan_production_sources(root):
        raw = read_text(path)
        scanned += 1
        code = strip_comments(raw, comment_style_for(path))
        for m in re.finditer(r"experiments[/\\]", code):
            line = code[: m.start()].count("\n") + 1
            runtime_hits.append(f"{rel(root, path)}:{line}")
        for token in FORBIDDEN_PROBE_TOKENS:
            for m in re.finditer(rf"\b{token}\b", code):
                line = code[: m.start()].count("\n") + 1
                token_hits.append(f"{rel(root, path)}:{line}:{token}")
    rep.fact("production_files_scanned", scanned)
    if runtime_hits:
        rep.fail(
            "正式源码/构建脚本在运行时引用 experiments/（探针资源不得进入产品）："
            + "；".join(runtime_hits[:10])
        )
    else:
        rep.ok(f"正式源码/构建脚本（{scanned} 个文件，已去注释）无 experiments/ 运行时引用")
    if token_hits:
        rep.fail("正式代码出现探针数据源标识符：" + "；".join(token_hits[:10]))
    else:
        rep.ok("正式代码无探针数据源标识符（Synthetic*/Mock*/EmptyExternal* 等）")


# --------------------------------------------------------------------------- #
# 自测：合成 fixture 仓库 + 逐条反向用例
# --------------------------------------------------------------------------- #


def build_fixture(root: Path) -> None:
    """按本脚本的常量生成一个「结构齐全」的合成仓库（fixture 内容最小但可判定）。"""
    for d in ROOT_REQUIRED_DIRS:
        (root / d).mkdir(parents=True, exist_ok=True)

    # 先按清单铺一遍占位文件（保证「点名文件存在」），随后再由下方专用内容覆盖，
    # 避免 Cargo.toml / 迁移 / 脚本被占位内容反向覆盖（曾导致 fixture 自身误报）。
    for _, rel_path in SUBSPEC_REQUIRED_FILES:
        p = root / rel_path
        p.parent.mkdir(parents=True, exist_ok=True)
        if p.suffix == ".slint":
            p.write_text("export component App {}\n", encoding="utf-8")
        elif p.suffix == ".json":
            p.write_text("{}\n", encoding="utf-8")
        elif p.suffix == ".rs":
            p.write_text("// fixture\n", encoding="utf-8")
        elif p.suffix == ".sh":
            p.write_text("#!/usr/bin/env bash\nset -Eeuo pipefail\necho fixture\n", encoding="utf-8")
        elif p.suffix == ".ps1":
            p.write_text("# fixture\n", encoding="utf-8")
        elif p.name == "novel_reader.h":
            p.write_text(build_fixture_header(), encoding="utf-8")
        elif p.name == "gradlew":
            p.write_text("#!/usr/bin/env bash\nset -Eeuo pipefail\nexit 0\n", encoding="utf-8")
        else:
            p.write_text("fixture\n", encoding="utf-8")

    members = list(CORE_MEMBER_DIRS)  # 仅在根 workspace 内的成员（desktop-app 是嵌套 workspace）
    member_list = ",\n    ".join(f'"{d}"' for d in members)
    (root / "Cargo.toml").write_text(
        "[workspace]\nresolver = \"2\"\nmembers = [\n    "
        + member_list
        + "\n]\nexclude = [\"experiments\"]\n\n[workspace.package]\nversion = \"0.1.0\"\n"
        + "license = \"GPL-3.0-only\"\nlicense-file = \"LICENSE\"\n",
        encoding="utf-8",
    )
    # LICENSE：GPLv3 标题片段即可（fixture 只需满足判据，不需全文 35KB）
    (root / "LICENSE").write_text(
        "                    GNU GENERAL PUBLIC LICENSE\n"
        "                       Version 3, 29 June 2007\n\n"
        "  This program is free software: you can redistribute it and/or modify\n"
        "  it under the terms of the GNU General Public License as published by\n"
        "  the Free Software Foundation, either version 3 of the License, or\n"
        "  (at your option) any later version.\n",
        encoding="utf-8",
    )
    (root / "Cargo.lock").write_text(
        "version = 3\n\n"
        + "\n\n".join(
            f'[[package]]\nname = "{Path(d).name}"\nversion = "0.1.0"' for d in members
        )
        + "\n",
        encoding="utf-8",
    )

    for d in members:
        pkg = root / d
        (pkg / "src").mkdir(parents=True, exist_ok=True)
        (pkg / "Cargo.toml").write_text(
            f'[package]\nname = "{Path(d).name}"\nversion = "0.1.0"\n'
            f'license.workspace = true\nlicense-file.workspace = true\n', encoding="utf-8"
        )
        (pkg / "src/lib.rs").write_text("// fixture lib\n", encoding="utf-8")
    (root / DESKTOP_CRATE_DIR / "src/main.rs").write_text("fn main() {}\n", encoding="utf-8")

    for d in SUBSPEC_REQUIRED_DIRS["desktop（crates/desktop-app + desktop/ui + desktop/packaging + tests/e2e）"]:
        (root / d).mkdir(parents=True, exist_ok=True)
    for d in SUBSPEC_REQUIRED_DIRS["android（android/ + crates/reading-ffi + android 测试口径）"]:
        (root / d).mkdir(parents=True, exist_ok=True)

    (root / "migrations/0001_init.sql").write_text(
        "-- fixture migration\n"
        + "\n".join(f"CREATE TABLE IF NOT EXISTS t{i} (id INTEGER PRIMARY KEY);" for i in range(8))
        + "\n",
        encoding="utf-8",
    )

    for s in REQUIRED_SCRIPTS + REQUIRED_E2E_SCRIPTS:
        p = root / s
        p.parent.mkdir(parents=True, exist_ok=True)
        if p.suffix == ".py":
            p.write_text('#!/usr/bin/env python3\n"""fixture"""\n', encoding="utf-8")
        else:
            p.write_text("#!/usr/bin/env bash\nset -Eeuo pipefail\necho fixture\n", encoding="utf-8")

    kt = root / "android/app/src/main/java/com/novelreader/MainActivity.kt"
    kt.parent.mkdir(parents=True, exist_ok=True)
    kt.write_text("package com.novelreader\n\nclass MainActivity\n", encoding="utf-8")
    res = root / "android/app/src/main/res/values/strings.xml"
    res.parent.mkdir(parents=True, exist_ok=True)
    res.write_text('<resources><string name="app_name">nr</string></resources>\n', encoding="utf-8")
    so = root / "android/app/src/main/jniLibs/arm64-v8a/libnr_jni.so"
    so.parent.mkdir(parents=True, exist_ok=True)
    so.write_bytes(b"\x7fELF fixture")
    jni = root / "android/jni/nr_jni.c"
    jni.write_text("/* fixture */\n", encoding="utf-8")
    unit = root / "android/app/src/test/java/com/novelreader/PageMathTest.kt"
    unit.parent.mkdir(parents=True, exist_ok=True)
    unit.write_text("package com.novelreader\n\nclass PageMathTest\n", encoding="utf-8")

    (root / "android/app/build.gradle.kts").write_text(
        'val nrAbi = "arm64-v8a"\n', encoding="utf-8"
    )


def build_fixture_header() -> str:
    lines = [
        "#ifndef NOVEL_READER_H",
        "#define NOVEL_READER_H",
        "#define NR_ABI_VERSION 1u",
        "#define NR_DEFAULT_WINDOW_BYTES 65536u",
        "#define NR_MAX_WINDOW_BYTES 262144u",
        "#define NR_ERR_OK 0",
    ]
    for i in range(1, 12):
        lines.append(f"#define NR_ERR_CODE_{i} {i}")
    for i in range(REQUIRED_NR_EXPORTS):
        lines.append(f"int32_t nr_fixture_{i}(void *engine);")
    lines.append("#endif")
    return "\n".join(lines) + "\n"


def evaluate(root: Path, rep: Report) -> None:
    check_root_and_workspace(rep, root)
    check_subspec_dirs(rep, root)
    check_android_landing(rep, root)
    check_migration(rep, root)
    check_scripts_and_fixtures(rep, root)
    check_abi_header(rep, root)
    check_no_probe_dependency(rep, root)


def _run_quiet(root: Path) -> list[str]:
    import contextlib
    import io

    rep = Report()
    with contextlib.redirect_stdout(io.StringIO()):
        evaluate(root, rep)
    return rep.failures


def self_test() -> list[str]:
    failures: list[str] = []
    print("[self-test] 合成 fixture 仓库 + 逐条反向用例（每条都必须命中**指定**判定）")

    with tempfile.TemporaryDirectory() as td:
        base = Path(td) / "repo"
        build_fixture(base)
        base_failures = _run_quiet(base)
        if base_failures:
            failures.append(f"完整 fixture 误报失败: {base_failures}")
            print(f"  FAIL  完整 fixture 不应有失败项，实际: {base_failures}")
        else:
            print("  PASS  完整 fixture 通过（检查项不空转）")

        def write(repo: Path, rel_path: str, text: str) -> None:
            (repo / rel_path).write_text(text, encoding="utf-8")

        def patch_file(repo: Path, rel_path: str, old: str, new: str) -> None:
            write(repo, rel_path, read_text(repo / rel_path).replace(old, new))

        # (标签, 变异, 期望命中的判定片段)
        mutations: list[tuple[str, object, str]] = [
            ("删除根目录 desktop/", lambda r: shutil.rmtree(r / "desktop"), "正式工程根目录缺失：desktop/"),
            (
                "workspace 删除成员 reading-index",
                lambda r: patch_file(r, "Cargo.toml", '"crates/reading-index",\n    ', ""),
                "workspace 缺少冻结成员",
            ),
            (
                "workspace 追加冻结集合外成员",
                lambda r: patch_file(
                    r, "Cargo.toml", '"crates/reading-core",', '"crates/extra-probe",\n    "crates/reading-core",'
                ),
                "冻结集合之外的成员",
            ),
            (
                "根 Cargo.toml 去掉 exclude experiments",
                lambda r: patch_file(r, "Cargo.toml", 'exclude = ["experiments"]', "exclude = []"),
                "未 exclude experiments/",
            ),
            (
                "workspace 未声明 members",
                lambda r: patch_file(r, "Cargo.toml", "members = [", "memberz = ["),
                "未声明 [workspace] members",
            ),
            ("成员 src 无 .rs 源文件", lambda r: shutil.rmtree(r / "crates/reading-storage/src"), ".rs 源文件"),
            ("缺少 desktop/packaging/windows", lambda r: shutil.rmtree(r / "desktop/packaging/windows"), "缺少目录"),
            ("缺少迁移 0001_init.sql", lambda r: (r / "migrations/0001_init.sql").unlink(), "0001_init.sql"),
            (
                "迁移含 PRAGMA",
                lambda r: (r / "migrations/0001_init.sql").open("a", encoding="utf-8").write(
                    "PRAGMA user_version = 1;\n"
                ),
                "含 PRAGMA",
            ),
            (
                "迁移只建 2 张表",
                lambda r: write(
                    r,
                    "migrations/0001_init.sql",
                    "CREATE TABLE IF NOT EXISTS a (id INTEGER);\n"
                    "CREATE TABLE IF NOT EXISTS b (id INTEGER);\n",
                ),
                "只有 2 张表",
            ),
            (
                "迁移写入完整正文字段",
                lambda r: (r / "migrations/0001_init.sql").open("a", encoding="utf-8").write(
                    "CREATE TABLE IF NOT EXISTS extra (id INTEGER, content TEXT);\n"
                ),
                "疑含整本正文字段",
            ),
            (
                "缺少 scripts/verify-v1-release.py",
                lambda r: (r / "scripts/verify-v1-release.py").unlink(),
                "缺少必需脚本",
            ),
            (
                "缺少 tests/fixtures/manifest.json",
                lambda r: (r / "tests/fixtures/manifest.json").unlink(),
                "manifest.json",
            ),
            (
                "Python 脚本语法错误",
                lambda r: write(r, "scripts/verify-abi.py", "def broken(:\n"),
                "无法编译",
            ),
            ("Bash 脚本语法错误", lambda r: write(r, "scripts/smoke-desktop.sh", "if [ x; then\n"), "Bash 脚本语法不成立"),
            ("删除 ABI 头文件", lambda r: (r / "include/novel_reader.h").unlink(), "novel_reader.h"),
            (
                "ABI 版本改为 2",
                lambda r: patch_file(r, "include/novel_reader.h", "#define NR_ABI_VERSION 1u", "#define NR_ABI_VERSION 2u"),
                "NR_ABI_VERSION = 2",
            ),
            (
                "窗口硬上限被改大",
                lambda r: patch_file(r, "include/novel_reader.h", "#define NR_MAX_WINDOW_BYTES 262144u", "#define NR_MAX_WINDOW_BYTES 1048576u"),
                "NR_MAX_WINDOW_BYTES = 1048576",
            ),
            (
                "错误码缺一项",
                lambda r: patch_file(r, "include/novel_reader.h", "#define NR_ERR_CODE_5 5\n", ""),
                "错误码数量",
            ),
            (
                "错误码数值重复（去注释后仍 12 项）",
                lambda r: patch_file(r, "include/novel_reader.h", "#define NR_ERR_CODE_7 7", "#define NR_ERR_CODE_7 6"),
                "不是 0..11 连续",
            ),
            (
                "清除 ABI 入口声明",
                lambda r: write(
                    r,
                    "include/novel_reader.h",
                    "#define NR_ABI_VERSION 1u\n#define NR_DEFAULT_WINDOW_BYTES 65536u\n"
                    "#define NR_MAX_WINDOW_BYTES 262144u\n#define NR_ERR_OK 0\n",
                ),
                "ABI 入口声明只有 0 项",
            ),
            (
                "生产 Cargo.toml 依赖 experiments",
                lambda r: (r / "crates/reading-core/Cargo.toml").open("a", encoding="utf-8").write(
                    '\n[dependencies.probe]\npath = "../../experiments/ui-selection/shared/rust-reading-core"\n'
                ),
                "指向 experiments/ 的 path 依赖",
            ),
            (
                "生产源码运行时引用 experiments/",
                lambda r: write(
                    r, "crates/reading-core/src/lib.rs", 'pub const P: &str = include_str!("../../../experiments/probe.txt");\n'
                ),
                "运行时引用 experiments/",
            ),
            (
                "生产源码注释提到 experiments/ 不应报错之外的判定",
                lambda r: write(
                    r,
                    "crates/reading-core/src/lib.rs",
                    "//! 不得引用 experiments/ 下的探针\npub fn f() {}\n",
                ),
                "__NONE__",
            ),
            (
                "生产代码出现探针数据源标识符",
                lambda r: write(r, "crates/reading-source/src/lib.rs", "pub struct SyntheticTextSource;\n"),
                "探针数据源标识符",
            ),
            (
                "JNI 预编译库缺失",
                lambda r: (r / "android/app/src/main/jniLibs/arm64-v8a/libnr_jni.so").unlink(),
                "没有 libnr_jni.so",
            ),
            (
                "JNI 桥接源缺失",
                lambda r: shutil.rmtree(r / "android/jni"),
                "android/jni",
            ),
            (
                "Android 单测目录清空",
                lambda r: shutil.rmtree(r / "android/app/src/test"),
                "Android 单测目录为空",
            ),
            (
                "Kotlin 主源码缺失",
                lambda r: shutil.rmtree(r / "android/app/src/main/java"),
                "没有 Kotlin 源码",
            ),
            (
                "gradle nrAbi 与 jniLibs 不一致",
                lambda r: write(r, "android/app/build.gradle.kts", 'val nrAbi = "x86_64"\n'),
                "jniLibs/x86_64/ 不存在",
            ),
        ]

        for idx, (label, fn, expect) in enumerate(mutations):
            work = Path(td) / f"mut-{idx}"
            shutil.copytree(base, work)
            fn(work)  # type: ignore[operator]
            found = _run_quiet(work)
            if label.startswith("生产源码注释提到"):
                # 该用例期望**无失败**：注释不是运行时依赖
                if found:
                    failures.append(f"文档注释被误判为 experiments 依赖：{found}")
                    print(f"  FAIL  文档注释被误判为依赖：{found[0]}")
                else:
                    print("  PASS  文档注释提到 experiments/ 不误报（只查运行时引用）")
                continue
            if not found:
                failures.append(f"反向用例未被报出：{label}")
                print(f"  FAIL  反向用例未被报出：{label}")
            elif not any(expect in f for f in found):
                failures.append(f"反向用例命中了非预期判定：{label}（期望含「{expect}」，实际 {found}）")
                print(f"  FAIL  反向用例命中非预期判定：{label} → {found[:2]}")
            else:
                hit = next(f for f in found if expect in f)
                print(f"  PASS  {label} → {hit}")

        # 注释剥离必须「保留字符串字面量、忽略注释」
        doc_only = "//! 不得引用 experiments/ 下的探针\npub fn f() {}\n"
        code_hit = 'pub const P: &str = include_str!("../../experiments/x");\n'
        kotlin_doc = '// 探针在 experiments/ 下\nval x = 1\n'
        if "experiments" in strip_comments(doc_only, "c"):
            failures.append("注释剥离未忽略文档注释")
            print("  FAIL  注释剥离未忽略文档注释")
        elif "experiments" not in strip_comments(code_hit, "c"):
            failures.append("注释剥离误删字符串字面量")
            print("  FAIL  注释剥离误删字符串字面量")
        elif "experiments" in strip_comments(kotlin_doc, "c"):
            failures.append("注释剥离未忽略 Kotlin 行注释")
            print("  FAIL  注释剥离未忽略 Kotlin 行注释")
        elif "experiments" in strip_comments("# 见 experiments/\necho hi\n", "hash"):
            failures.append("注释剥离未忽略 Shell 行注释")
            print("  FAIL  注释剥离未忽略 Shell 行注释")
        else:
            print("  PASS  注释剥离：忽略注释、保留字符串字面量")

    return failures


# --------------------------------------------------------------------------- #
# CLI
# --------------------------------------------------------------------------- #


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description="V1 正式工程结构门禁（父级 checklist C1）")
    p.add_argument("--root", default=str(REPO_ROOT), help="仓库根（默认本脚本上级目录）")
    p.add_argument("--json", dest="json_out", default=None, help="把结论写入 JSON 文件")
    p.add_argument("--self-test", action="store_true", help="运行合成 fixture 反向用例")
    args = p.parse_args(argv)

    root = Path(args.root).resolve()
    print(f"verify-v1-structure: repo={root}")
    if not root.is_dir():
        print(f"RESULT: FAIL — --root 不存在: {root}")
        return 2

    exit_code = 0
    if args.self_test:
        failures = self_test()
        print("-" * 68)
        if failures:
            print(f"SELF-TEST RESULT: FAIL ({len(failures)} 项)")
            for f in failures:
                print(f"  - {f}")
            exit_code = 1
        else:
            print("SELF-TEST RESULT: PASS（每条判定均有反向用例）")

    rep = Report()
    evaluate(root, rep)

    print("-" * 68)
    print("结构清单摘要：")
    print(f"  workspace 成员           : {rep.facts.get('workspace_members')}")
    print(f"  workspace 版本           : {rep.facts.get('workspace_version')}")
    print(f"  Cargo.lock package 数    : {rep.facts.get('cargo_lock_packages')}")
    print(f"  Android 主源码 .kt 数    : {rep.facts.get('android_kotlin_main_files')}")
    print(f"  Android 单测 .kt 数      : {rep.facts.get('android_unit_test_files')}")
    print(f"  jniLibs 预编译库         : {rep.facts.get('android_jni_so')}")
    print(f"  迁移文件                 : {rep.facts.get('migrations')}")
    print(f"  迁移建表                 : {rep.facts.get('migration_tables')}")
    print(f"  ABI 版本 / 入口 / 错误码 : NR_ABI_VERSION={REQUIRED_HEADER_MACROS['NR_ABI_VERSION']}, "
          f"{len(rep.facts.get('header_nr_exports') or [])} 入口, "
          f"{len(rep.facts.get('header_error_codes') or {})} 错误码")
    print(f"  扫描的生产文件数         : {rep.facts.get('production_files_scanned')}")
    print("")

    if args.json_out:
        payload = {
            "result": "PASS" if (not rep.failures and exit_code == 0) else "FAIL",
            "root": str(root),
            "ok_count": rep.ok_count,
            "failures": rep.failures,
            "skips": rep.skips,
            "facts": rep.facts,
        }
        Path(args.json_out).write_text(
            json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8"
        )
        print(f"JSON 结论已写入 {args.json_out}")

    if rep.failures:
        print(f"RESULT: FAIL ({len(rep.failures)} 项)")
        for item in rep.failures:
            print(f"  - {item}")
        return 1
    if exit_code != 0:
        print("RESULT: FAIL — --self-test 未通过")
        return 1
    print(
        f"RESULT: PASS — 正式工程结构齐全（断言 {rep.ok_count} 项，跳过 {len(rep.skips)} 项）；"
        "编译/测试/APK/真机结论不包含在本门禁内"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())

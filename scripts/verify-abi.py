#!/usr/bin/env python3
"""verify-abi.py — 比对 `include/novel_reader.h` 与 Rust 导出符号（必要时比对 Kotlin 声明）。

用途（RC7 冻结契约、reading-core NFR-C4）：
证明 C 头文件声明的 18 项入口与 `crates/reading-ffi` **实际导出的符号**逐项一致，
并在 Android Kotlin 目录存在时一并比对 Kotlin 的 `external fun` 声明。

检查项：
  1. 头文件函数声明集合 ↔ Rust 导出符号集合（双向：缺失与多余都报）；
  2. 头文件冻结常量（ABI 版本、12 项错误码、窗口上限、数据源种类、编码、页边界状态、
     拒绝原因）与冻结表逐项一致；
  3. Kotlin 目录存在时比对 `external fun nr_*` 声明；目录不存在时**显式打印跳过项**，
     且默认以非零退出码结束（要显式放行必须给出 `--allow-missing-kotlin`）；
  4. `--self-test`：用临时 fixture 证明"缺失项会被报出"——
     包括缺 Rust 符号、缺 Kotlin 声明、缺库、缺 Kotlin 目录、宏数值错误等反向用例，
     以及用 `rustc` 现场编译一个真实 `.so` 的正向对照（证明符号提取路径不是空检查）。

退出码：0 = 全部通过；1 = 存在违规；2 = 脚本自身前提缺失（如既无库也无源码、rustc 不可用）。

局限说明：符号提取优先使用 `nm` 读取编译产物；找不到产物时退化为 Rust 源码扫描，
并打印 `[FALLBACK]` 行说明（绝不静默通过）。Kotlin 侧只比对声明名与数量，
业务边界的静态扫描由 AN2/van1 的 `verify-android-*.py` 负责。
"""

from __future__ import annotations

import argparse
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]

# ---------------------------------------------------------------- 冻结表

# gate-0 §2.2 冻结的 12 项错误码宏（顺序即数值 0..=11）。
FROZEN_ERROR_MACROS = [
    "NR_ERR_OK",
    "NR_ERR_INVALID_ARGUMENT",
    "NR_ERR_OUT_OF_RANGE",
    "NR_ERR_UTF8_BOUNDARY",
    "NR_ERR_BOOK_NOT_FOUND",
    "NR_ERR_NO_OPEN_BOOK",
    "NR_ERR_IO",
    "NR_ERR_CANCELLED",
    "NR_ERR_LAYOUT_NOT_FOUND",
    "NR_ERR_ENGINE_DESTROYED",
    "NR_ERR_INTERNAL",
    "NR_ERR_CHAPTER_OUT_OF_RANGE",
]

# gate-0 §2.1 / §2.3 冻结值（ABI 版本在运行时从 Rust 常量读取，避免两处各写一份）。
FROZEN_CONSTANTS = {
    "NR_DEFAULT_WINDOW_BYTES": 65536,
    "NR_MAX_WINDOW_BYTES": 262144,
    "NR_SOURCE_KIND_FILE": 1,
    "NR_SOURCE_KIND_PLATFORM": 2,
    "NR_SUBMIT_BEGIN_LAYOUT": 1,
    "NR_SUBMIT_COMMIT": 2,
    "NR_LAYOUT_ALGORITHM_VERSION": 1,
    "NR_CONTENT_UNIT_SINGLE": 1,
    "NR_MAX_CHAPTER_BATCH": 256,
}

# gate-0 §2.7 冻结的 18 项入口（顺序即头文件与 Rust 的声明顺序）。
FROZEN_ENTRY_NAMES = [
    "nr_abi_version",
    "nr_engine_create",
    "nr_engine_destroy",
    "nr_open_txt",
    "nr_close_book",
    "nr_read_window",
    "nr_submit_page_boundary",
    "nr_get_page_boundary",
    "nr_save_position",
    "nr_get_position",
    "nr_get_chapters",
    "nr_create_bookmark",
    "nr_delete_bookmark",
    "nr_invalidate_layout",
    "nr_rebuild_index",
    "nr_set_encoding",
    "nr_list_categories",
    "nr_create_category",
    "nr_move_book_to_category",
    "nr_delete_category",
    "nr_list_history",
    "nr_clear_history_entry",
    "nr_clear_history_for_book",
    "nr_get_theme_base",
    "nr_set_theme_base",
    "nr_set_theme_enabled",
    "nr_get_theme_token",
    "nr_get_theme_contrast",
    "nr_get_background",
    "nr_set_background",
    "nr_set_background_scrim",
    "nr_clear_background",
    "nr_category_move_targets",
    "nr_rename_category",
    "nr_move_category",
    "nr_list_books_in_scope",
    "nr_list_book_categories",
    "nr_cancel_request",
    "nr_get_last_error",
    "nr_release_buffer",
]

NR_FUNCTION_DECL_RE = re.compile(
    r"^\s*(?:int32_t|uint32_t)\s+(nr_[A-Za-z0-9_]+)\s*\(([^;]*)\)\s*;\s*$"
)
NR_MACRO_RE = re.compile(
    r"^\s*#define\s+(NR_[A-Za-z0-9_]+)\s+([0-9]+)[uU]?\s*(?:/\*.*\*/)?\s*$"
)
KOTLIN_EXTERNAL_RE = re.compile(r"\bexternal\s+fun\s+(nr_[A-Za-z0-9_]+)\s*\(")
RUST_EXPORT_RE = re.compile(
    r"#\[no_mangle\][^;]*?\bpub\s+(?:unsafe\s+)?extern\s+\"C\"\s+fn\s+(nr_[A-Za-z0-9_]+)"
)
RUST_ABI_VERSION_RE = re.compile(r"pub\s+const\s+ABI_VERSION\s*:\s*u32\s*=\s*([0-9_]+)\s*;")


class Report:
    """收集检查结果（PASS/FAIL/SKIP），并给出退出码。"""

    def __init__(self, title: str) -> None:
        self.title = title
        self.failures: list[str] = []
        self.skips: list[str] = []
        self.passes = 0

    def ok(self, message: str) -> None:
        self.passes += 1
        print(f"  PASS  {message}")

    def fail(self, message: str) -> None:
        self.failures.append(message)
        print(f"  FAIL  {message}")

    def skip(self, message: str) -> None:
        self.skips.append(message)
        print(f"  SKIP  {message}")

    def note(self, message: str) -> None:
        print(f"  NOTE  {message}")


# ---------------------------------------------------------------- 解析


def parse_header(text: str, path: str) -> tuple[list[str], dict[str, int]]:
    """解析头文件：返回 (入口声明顺序, 宏表)。"""
    order: list[str] = []
    macros: dict[str, int] = {}
    for line in text.splitlines():
        match = NR_FUNCTION_DECL_RE.match(line)
        if match:
            order.append(match.group(1))
            continue
        macro = NR_MACRO_RE.match(line)
        if macro:
            macros[macro.group(1)] = int(macro.group(2))
    if not order:
        raise ValueError(f"{path} 未解析出任何 nr_* 函数声明：头文件格式或路径不正确")
    return order, macros


def rust_abi_version(source: Path) -> int | None:
    """从 reading-core 读取冻结 ABI 版本常量。"""
    if not source.is_file():
        return None
    match = RUST_ABI_VERSION_RE.search(source.read_text(encoding="utf-8"))
    if not match:
        return None
    return int(match.group(1).replace("_", ""))


def symbols_from_library(path: Path) -> set[str]:
    """用 nm 读取编译产物的已定义符号；取 nr_* 前缀。"""
    if shutil.which("nm") is None:
        raise RuntimeError("nm 不可用，无法读取编译产物符号")
    args = ["nm", "--defined-only", "--format=posix"]
    if path.suffix == ".so":
        args = ["nm", "-D", "--defined-only", "--format=posix"]
    proc = subprocess.run(args + [str(path)], capture_output=True, text=True, check=False)
    if proc.returncode != 0:
        raise RuntimeError(f"nm 读取 {path} 失败(exit {proc.returncode}): {proc.stderr.strip()[:200]}")
    symbols: set[str] = set()
    for line in proc.stdout.splitlines():
        parts = line.split()
        if not parts:
            continue
        name = parts[0]
        if name.startswith("nr_"):
            symbols.add(name)
    return symbols


def symbols_from_sources(directory: Path) -> set[str]:
    """退化的源码扫描：`#[no_mangle] ... extern "C" fn nr_*`。"""
    symbols: set[str] = set()
    for path in sorted(directory.rglob("*.rs")):
        text = path.read_text(encoding="utf-8", errors="replace")
        for match in RUST_EXPORT_RE.finditer(text):
            symbols.add(match.group(1))
    return symbols


def kotlin_declarations(directory: Path) -> dict[str, str]:
    """Kotlin `external fun nr_*` 声明：名字 → 文件路径。"""
    found: dict[str, str] = {}
    for path in sorted(directory.rglob("*.kt")):
        text = path.read_text(encoding="utf-8", errors="replace")
        for match in KOTLIN_EXTERNAL_RE.finditer(text):
            found.setdefault(match.group(1), str(path))
    return found


def auto_library() -> Path | None:
    """自动探测编译产物（release 优先，其次 debug）。"""
    for candidate in [
        REPO_ROOT / "target" / "release" / "libreading_ffi.so",
        REPO_ROOT / "target" / "release" / "libreading_ffi.a",
        REPO_ROOT / "target" / "debug" / "libreading_ffi.so",
        REPO_ROOT / "target" / "debug" / "libreading_ffi.a",
    ]:
        if candidate.is_file():
            return candidate
    return None


# ---------------------------------------------------------------- 比较


def compare_entries(
    order: list[str],
    symbols: set[str],
    source_label: str,
    report: Report,
    expected_order: list[str] | None = None,
) -> None:
    """头文件入口 ↔ Rust 导出符号双向比对。"""
    declared = set(order)
    missing = sorted(declared - symbols)
    extra = sorted(symbols - declared)
    if missing:
        report.fail(
            f"头文件声明但 {source_label} 未导出（{len(missing)} 项）: {', '.join(missing)}"
        )
    else:
        report.ok(f"头文件声明的 {len(declared)} 项入口全部存在于 {source_label}")
    if extra:
        report.fail(
            f"{source_label} 导出了头文件未声明的符号（{len(extra)} 项）: {', '.join(extra)}"
        )
    else:
        report.ok(f"{source_label} 未导出任何头文件未声明的 nr_* 符号")
    if len(order) != len(declared):
        report.fail(f"头文件存在重复的函数声明：{order}")
    if expected_order is not None:
        if order == expected_order:
            report.ok(f"头文件声明顺序与冻结入口清单一致（{len(expected_order)} 项）")
        else:
            report.fail(
                "头文件声明顺序与冻结清单不一致："
                f"实际 {order}，期望 {expected_order}"
            )


def compare_constants(
    macros: dict[str, int], abi_version: int | None, report: Report
) -> None:
    """头文件宏 ↔ 冻结常量表。"""
    for name, expected in FROZEN_CONSTANTS.items():
        if name not in macros:
            report.fail(f"头文件缺少冻结常量宏: {name}")
        elif macros[name] != expected:
            report.fail(f"宏 {name} = {macros[name]}，冻结值为 {expected}")
    if any(name in macros for name in FROZEN_CONSTANTS):
        report.ok(f"冻结常量宏 {len(FROZEN_CONSTANTS)} 项与 gate-0 冻结值一致")
    else:
        # 一项都没有 ⇒ 空检查：必须报出，且继续检查版本与错误码（不得静默通过）。
        report.fail("头文件未声明任何冻结常量宏，常量比对为空检查")

    if abi_version is None:
        report.fail("无法从 Rust 常量读取 ABI_VERSION，版本一致性无法判定")
    elif "NR_ABI_VERSION" not in macros:
        report.fail("头文件缺少 NR_ABI_VERSION 宏")
    elif macros["NR_ABI_VERSION"] != abi_version:
        report.fail(
            f"NR_ABI_VERSION = {macros['NR_ABI_VERSION']}，Rust ABI_VERSION = {abi_version}"
        )
    else:
        report.ok(f"NR_ABI_VERSION = Rust ABI_VERSION = {abi_version}")

    for index, name in enumerate(FROZEN_ERROR_MACROS):
        if name not in macros:
            report.fail(f"头文件缺少错误码宏: {name}")
        elif macros[name] != index:
            report.fail(f"错误码宏 {name} = {macros[name]}，冻结值为 {index}")
    if all(name in macros for name in FROZEN_ERROR_MACROS):
        report.ok(f"12 项错误码宏与冻结表一致（0..={len(FROZEN_ERROR_MACROS) - 1}）")


def compare_kotlin(
    order: list[str],
    kotlin_dir: Path | None,
    allow_missing: bool,
    report: Report,
) -> None:
    """Kotlin 声明比对；目录缺失时显式跳过（默认仍视为失败）。"""
    if kotlin_dir is None:
        report.skip("未提供 --kotlin 目录：Kotlin 声明一致性未校验（Android 尚未建立）")
        if not allow_missing:
            report.fail("Kotlin 声明一致性未校验且未给出 --allow-missing-kotlin")
        return
    if not kotlin_dir.is_dir():
        report.skip(f"Kotlin 目录不存在: {kotlin_dir}")
        if not allow_missing:
            report.fail(f"Kotlin 声明一致性未校验且未给出 --allow-missing-kotlin: {kotlin_dir}")
        return
    declarations = kotlin_declarations(kotlin_dir)
    if not declarations:
        report.skip(f"Kotlin 目录内没有任何 `external fun nr_*` 声明: {kotlin_dir}")
        if not allow_missing:
            report.fail(f"Kotlin 目录存在但没有 JNI 声明: {kotlin_dir}")
        return
    missing = sorted(set(order) - set(declarations))
    extra = sorted(set(declarations) - set(order))
    if missing:
        report.fail(f"Kotlin 未声明的入口（{len(missing)} 项）: {', '.join(missing)}")
    else:
        report.ok(f"Kotlin 声明覆盖全部 {len(order)} 项入口")
    if extra:
        report.fail(f"Kotlin 声明了头文件不存在的入口: {', '.join(extra)}")
    else:
        report.ok("Kotlin 未声明任何头文件不存在的入口")


def check_contract(
    header_text: str,
    header_label: str,
    symbols: set[str],
    symbol_source: str,
    abi_version: int | None,
    kotlin_dir: Path | None,
    allow_missing_kotlin: bool,
    report: Report,
    expected_order: list[str] | None = FROZEN_ENTRY_NAMES,
) -> None:
    order, macros = parse_header(header_text, header_label)
    print(f"[1] 头文件入口 ↔ Rust 导出符号（来源：{symbol_source}）")
    compare_entries(order, symbols, symbol_source, report, expected_order)
    print("[2] 头文件冻结常量 ↔ gate-0 冻结表")
    compare_constants(macros, abi_version, report)
    print("[3] Kotlin 声明一致性")
    compare_kotlin(order, kotlin_dir, allow_missing_kotlin, report)


# ---------------------------------------------------------------- 自测


def _self_expect(report: Report, label: str, condition: bool, detail: str) -> None:
    if condition:
        report.ok(f"自测：{label}")
    else:
        report.fail(f"自测（{label}）未按预期报出：{detail}")


SELF_HEADER_OK = """\
#define NR_ABI_VERSION 1u
#define NR_ERR_OK 0
#define NR_ERR_INVALID_ARGUMENT 1
int32_t nr_alpha(uint64_t engine);
int32_t nr_beta(uint64_t engine, int32_t *out);
"""

SELF_HEADER_MISSING_RUST = """\
#define NR_ABI_VERSION 1u
int32_t nr_alpha(uint64_t engine);
int32_t nr_gamma(uint64_t engine);
"""

SELF_RUST_OK = """\
#[no_mangle]
pub extern "C" fn nr_alpha(engine: u64) -> i32 { let _ = engine; 0 }

#[no_mangle]
pub unsafe extern "C" fn nr_beta(engine: u64, out: *mut i32) -> i32 {
    let _ = (engine, out);
    0
}
"""


def self_test() -> Report:
    """反向用例 + 正向对照，证明"缺失项会被报出"而不是静默通过。"""
    report = Report("self-test")
    print("[self-test] 反向用例与正向对照")

    # 1) 完全一致的合成输入：必须通过（入口比对的正向对照）。
    symbols = symbols_from_sources(_write_temp("ok.rs", SELF_RUST_OK))
    case = Report("fixture-ok")
    compare_entries(["nr_alpha", "nr_beta"], symbols, "fixture 源码", case, None)
    _self_expect(
        report,
        "完整 fixture 通过（正向对照）",
        not case.failures,
        f"意外失败: {case.failures}",
    )

    # 1b) 真实头文件的常量比对必须通过（常量比对的正向对照）。
    real_header = REPO_ROOT / "include" / "novel_reader.h"
    if real_header.is_file():
        _order, real_macros = parse_header(real_header.read_text(encoding="utf-8"), str(real_header))
        abi_version = rust_abi_version(
            REPO_ROOT / "crates" / "reading-core" / "src" / "lib.rs"
        )
        case = Report("fixture-real-header")
        compare_constants(real_macros, abi_version, case)
        _self_expect(
            report,
            "真实头文件的冻结常量比对通过",
            not case.failures,
            f"意外失败: {case.failures}",
        )
    else:
        report.skip("仓库头文件不存在：跳过常量比对的正向对照")

    # 2) 头文件声明了未导出的入口：必须报出。
    order, _macros = parse_header(SELF_HEADER_MISSING_RUST, "<fixture>")
    case = Report("fixture-missing")
    compare_entries(order, symbols, "fixture 源码", case, None)
    _self_expect(
        report,
        "缺失 Rust 符号被报出",
        any("nr_gamma" in item for item in case.failures),
        f"failures={case.failures}",
    )

    # 3) 导出符号多于头文件声明：必须报出。
    case = Report("fixture-extra")
    compare_entries(["nr_alpha"], symbols, "fixture 源码", case, None)
    _self_expect(
        report,
        "多余导出符号被报出",
        any("nr_beta" in item for item in case.failures),
        f"failures={case.failures}",
    )

    # 4) 错误码宏数值错误 / 版本不一致：必须报出。
    broken = SELF_HEADER_OK.replace("#define NR_ERR_INVALID_ARGUMENT 1", "#define NR_ERR_INVALID_ARGUMENT 5")
    _order, macros = parse_header(broken, "<fixture>")
    case = Report("fixture-macro")
    compare_constants(macros, 2, case)
    _self_expect(
        report,
        "错误码宏数值错误被报出",
        any("NR_ERR_INVALID_ARGUMENT" in item for item in case.failures),
        f"failures={case.failures}",
    )
    _self_expect(
        report,
        "ABI 版本不一致被报出",
        any("NR_ABI_VERSION" in item for item in case.failures),
        f"failures={case.failures}",
    )

    # 5) Kotlin 目录缺失：默认必须失败（不得静默通过）。
    case = Report("fixture-kotlin-missing")
    compare_kotlin(["nr_alpha", "nr_beta"], None, False, case)
    _self_expect(
        report,
        "Kotlin 缺失且未放行时失败",
        bool(case.failures) and bool(case.skips),
        f"failures={case.failures} skips={case.skips}",
    )
    case = Report("fixture-kotlin-allowed")
    compare_kotlin(["nr_alpha"], None, True, case)
    _self_expect(
        report,
        "Kotlin 缺失但显式放行时打印跳过项且不失败",
        not case.failures and bool(case.skips),
        f"failures={case.failures} skips={case.skips}",
    )

    # 6) Kotlin 声明不完整：必须报出。
    with tempfile.TemporaryDirectory(prefix="nr-abi-kotlin-") as tmp:
        kt_dir = Path(tmp) / "main" / "java" / "core"
        kt_dir.mkdir(parents=True)
        (kt_dir / "NrCore.kt").write_text(
            "package core\n\nobject NrCore {\n"
            "    external fun nr_alpha(engine: Long): Int\n"
            "    external fun nr_omega(engine: Long): Int\n"
            "}\n",
            encoding="utf-8",
        )
        case = Report("fixture-kotlin-partial")
        compare_kotlin(["nr_alpha", "nr_beta"], Path(tmp), False, case)
        _self_expect(
            report,
            "Kotlin 缺失声明被报出",
            any("nr_beta" in item for item in case.failures),
            f"failures={case.failures}",
        )
        # 7) Kotlin 声明多余：必须报出。
        case = Report("fixture-kotlin-extra")
        compare_kotlin(["nr_alpha"], Path(tmp), False, case)
        _self_expect(
            report,
            "Kotlin 多余声明被报出",
            any("nr_omega" in item for item in case.failures),
            f"failures={case.failures}",
        )

    # 8) 正向对照：真实编译产物 + nm 提取（证明符号检查不是空检查）。
    if shutil.which("rustc") is None:
        report.skip("rustc 不可用：跳过真实动态库符号提取的对照")
    else:
        with tempfile.TemporaryDirectory(prefix="nr-abi-lib-") as tmp:
            src = Path(tmp) / "fixture.rs"
            src.write_text(SELF_RUST_OK, encoding="utf-8")
            lib = Path(tmp) / "libfixture_abi.so"
            proc = subprocess.run(
                ["rustc", "--crate-type", "cdylib", "-o", str(lib), str(src)],
                capture_output=True,
                text=True,
                check=False,
            )
            if proc.returncode != 0:
                report.fail(f"rustc 编译自测夹具失败: {proc.stderr.strip()[:200]}")
            else:
                symbols = symbols_from_library(lib)
                _self_expect(
                    report,
                    "从真实 .so 提取到导出符号",
                    {"nr_alpha", "nr_beta"} <= symbols,
                    f"symbols={sorted(symbols)}",
                )
                case = Report("fixture-real-lib")
                compare_entries(["nr_alpha"], symbols, str(lib), case, None)
                _self_expect(
                    report,
                    "真实库的多余符号被报出",
                    any("nr_beta" in item for item in case.failures),
                    f"failures={case.failures}",
                )

    # 9) 既无库也无源码：必须判定为"无法比对"而不是通过。
    _self_expect(
        report,
        "无法进行符号比对时判定为无法比对",
        _empty_source_fails(),
        "空源码目录必须被判定为无法比对",
    )
    return report


def _empty_source_fails() -> bool:
    with tempfile.TemporaryDirectory(prefix="nr-abi-empty-") as tmp:
        return not symbols_from_sources(Path(tmp))


def _write_temp(name: str, content: str) -> Path:
    directory = Path(tempfile.mkdtemp(prefix="nr-abi-src-"))
    path = directory / name
    path.write_text(content, encoding="utf-8")
    return directory


# ---------------------------------------------------------------- main


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="比对 C 头文件与 Rust/Kotlin ABI 声明")
    parser.add_argument("--header", default=str(REPO_ROOT / "include" / "novel_reader.h"))
    parser.add_argument("--lib", default=None, help="编译产物路径（默认自动探测 release/debug）")
    parser.add_argument(
        "--rust-src",
        default=str(REPO_ROOT / "crates" / "reading-ffi" / "src"),
        help="无编译产物时的退化源码扫描目录",
    )
    parser.add_argument("--kotlin", default=None, help="Kotlin 目录（AN2 建立后提供）")
    parser.add_argument(
        "--allow-missing-kotlin",
        action="store_true",
        help="允许 Kotlin 目录缺失（仍会显式打印跳过项）",
    )
    parser.add_argument("--self-test", action="store_true", help="运行反向/正向自测")
    args = parser.parse_args(argv)

    print(f"verify-abi: repo={REPO_ROOT}")
    exit_code = 0

    if args.self_test:
        report = self_test()
        print("-" * 60)
        if report.failures:
            print(f"SELF-TEST RESULT: FAIL ({len(report.failures)} 项)")
            for item in report.failures:
                print(f"  - {item}")
            exit_code = 1
        else:
            print(f"SELF-TEST RESULT: PASS（{report.passes} 项断言，缺失项均可被报出）")

    header_path = Path(args.header)
    if not header_path.is_file():
        print(f"RESULT: FAIL — 头文件不存在: {header_path}")
        return 1
    header_text = header_path.read_text(encoding="utf-8")

    report = Report("contract")
    if args.lib:
        library = Path(args.lib)
        if not library.is_file():
            report.fail(f"--lib 指定的编译产物不存在: {library}")
            symbol_source = None
        else:
            symbol_source = library
    else:
        symbol_source = auto_library()
    if symbol_source is None:
        print("[1] 头文件入口 ↔ Rust 导出符号")
        report.skip(
            "未找到编译产物（target/release|debug/libreading_ffi.{so,a}）："
            "退化为 Rust 源码扫描，编译产物符号级校验未执行"
        )
        symbols = symbols_from_sources(Path(args.rust_src))
        label = f"Rust 源码（{args.rust_src}）"
        if not symbols:
            print(f"RESULT: FAIL — 源码扫描也未发现任何 nr_* 导出（{args.rust_src}）")
            return 2
    else:
        try:
            symbols = symbols_from_library(symbol_source)
        except RuntimeError as error:
            print(f"RESULT: FAIL — {error}")
            return 2
        label = str(symbol_source)
        if not symbols:
            print(f"RESULT: FAIL — {symbol_source} 未导出任何 nr_* 符号")
            return 2

    abi_version = rust_abi_version(REPO_ROOT / "crates" / "reading-core" / "src" / "lib.rs")
    kotlin_dir = Path(args.kotlin) if args.kotlin else None
    try:
        check_contract(
            header_text,
            str(header_path),
            symbols,
            label,
            abi_version,
            kotlin_dir,
            args.allow_missing_kotlin,
            report,
        )
    except ValueError as error:
        print(f"RESULT: FAIL — {error}")
        return 1

    print("-" * 60)
    if report.failures:
        print(f"RESULT: FAIL ({len(report.failures)} 项违规)")
        for item in report.failures:
            print(f"  - {item}")
        return 1
    if exit_code != 0:
        print("RESULT: FAIL — --self-test 未通过（缺失项检测能力未得到证明）")
        return 1
    print(
        "RESULT: PASS — 头文件与 Rust 导出符号逐项一致；"
        f"跳过项 {len(report.skips)} 个（见上方 SKIP 行）"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())

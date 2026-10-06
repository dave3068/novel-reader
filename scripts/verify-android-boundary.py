#!/usr/bin/env python3
"""verify-android-boundary.py — AN2 Kotlin 薄封装 ↔ C ABI ↔ JNI 导出 的边界门禁。

用途（子规格 android/spec.md AR2 / 检查项 C2；`android/jni/README.md` 契约）：

  证明 Android 薄封装（`android/app/src/main/java/**/core/`）在**边界**上与冻结契约一致，
  且没有把阅读业务搬进 Kotlin：

  [1] Kotlin `external fun` 声明 ↔ `include/novel_reader.h` 的 18 项入口
      （集合、数量、顺序、唯一承载文件 `core/NrCore.kt`，且不得有第二个声明处）；
  [2] JNI 符号转义 ↔ C 层导出名（`Java_com_novelreader_android_core_NrCore_nr_1abi_1version`；
      方法名里的 `_` 必须转义成 `_1`），并在编译产物存在时用 `nm -D` 比对真实导出符号
      （文本校验 + 二进制校验双证）；
  [3] 错误码：Kotlin 常量数值 ↔ 头文件宏（12 项 `NR_ERR_*`），且每个错误码都进入业务映射；
      Kotlin 侧**不得解析错误文本**（头文件 §2：错误文本只用于展示）；
  [4] 句柄：句柄/偏移一律 `Long`（不得退化为 `Int`）、创建失败 `0` 视为失败、
      销毁只触发一次且销毁后不再触碰 native、`System.loadLibrary` 只允许出现在 `NrCore.kt`、
      引擎/ABI 承载层（`core/**`）不得加 Kotlin 侧锁（否则头文件 §7 的跨线程取消会被阻塞）——
      `reader/**` 的 ViewModel 状态串行化锁不是本规则的客体（AR4 明确要求 Mutex 串行化），
      它受 [6b] 的行为型规则约束；
  [5] 缓冲释放：四个缓冲区入口以 `outBuffer: LongArray` 收尾；归还只经 `NrBufferHandle`
      （`NrCore.nr_release_buffer` 只在一个文件出现）；有「恰好一次」与空凭据 no-op 保护；
      结构体内嵌槽位与 `outBuffer` 必须交叉断言一致；
  [6] 取消：`nr_cancel_request` 声明、`requestId` 非 0 前置校验、取消是部分成功（`NR_ERR_CANCELLED`）；
  [6b] 取消可达性（头文件 §7 的行为型可证形式）：`reader/**` 的取消入口（`fun cancel*`）体内
      不得出现取 `reader/**` 串行化 Mutex 门（`gate.withLock` / `gate.lock`）的调用；
      「取消入口存在」与「串行化门存在」缺一即 FAIL —— 规则不得因客体缺失而空转通过；
  [7] Kotlin 业务边界：`core/` 不得引用 Compose/Activity 状态，也不得引用 `experiments/` 探针。

退出码：0 = 全部通过；1 = 存在违规；2 = 脚本自身前提缺失（头文件 / Kotlin 目录缺失）。

局限说明：本脚本是静态检查 + 二进制符号比对，不加载 `.so`、不运行 JVM
（JVM 行为由 `./gradlew :app:testDebugUnitTest` 单测取证）。
`--self-test` 以 fixture 证明「缺失/多余/未转义/错值/文本解析/加锁/取消取门」都会被报出，
即上述检查不是空检查。
"""

from __future__ import annotations

import argparse
import re
import shutil
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]

# ---- 冻结标识（android/jni/README.md §1；改名即 UnsatisfiedLinkError） ----
KOTLIN_PACKAGE = "com.novelreader.android.core"
KOTLIN_CLASS = "NrCore"
JNI_LIBRARY = "nr_jni"
JNI_CLASS_PREFIX = "Java_" + KOTLIN_PACKAGE.replace(".", "_") + "_" + KOTLIN_CLASS + "_"
CARRIER_SUFFIX = "com/novelreader/android/core/NrCore.kt"
CORE_DIR_PREFIX = "com/novelreader/android/core/"
READER_DIR_PREFIX = "com/novelreader/android/reader/"
READER_VIEWMODEL_SUFFIX = READER_DIR_PREFIX + "ReaderViewModel.kt"

# ---- 头文件冻结的 18 项入口（顺序即 include/novel_reader.h 的声明顺序） ----
FROZEN_ENTRIES = [
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

# 四个「产出缓冲区 + 归还槽位」的入口（README §4）。
BUFFER_ENTRIES = ["nr_read_window", "nr_get_position", "nr_get_chapters", "nr_get_last_error"]

# 头文件冻结的 12 项错误码（数值即 0..=11）。
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

# 头文件 §2：错误文本只用于展示 —— Kotlin 侧不得出现文本匹配原语。
FORBIDDEN_TEXT_PARSING = (".contains(", ".startsWith(", ".endsWith(", ".matches(", "Regex(")

# 头文件 §7：取消标志与引擎锁分离存放；作用域 = **引擎/ABI 承载层 `core/**`**。
# 只有这一层包裹 native 调用，在它上面加 Kotlin 侧锁才会把跨线程取消阻塞住。
# `reader/**`（ViewModel）按 android/design.md + AR4「打开/切章/翻页/重排/保存位置串行化」
# 必须持有 Mutex 门，该锁不属于本规则客体；它自身受 [6b] 行为型规则约束（取消入口不得取该门）。
FORBIDDEN_LOCKS = ("Mutex", "synchronized", "ReentrantLock", "@Synchronized")

# [6b] 行为型规则：「reader/** 的串行化门」声明形式与「取消入口」函数名形式。
GATE_DECL_RE = re.compile(r"\bval\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?::\s*Mutex\s*)?=\s*Mutex\s*\(\s*\)")
CANCEL_ENTRY_RE = re.compile(r"\bfun\s+(cancel[A-Za-z0-9_]*)\s*\(")

# core/ 不得持有 Activity / Compose / ViewModel 状态（业务边界）。
FORBIDDEN_PLATFORM_STATE = (
    "androidx.compose",
    "android.app.Activity",
    "ComponentActivity",
    "ViewModel",
)

# 64 位退化的检查范围：只扫 ABI 绑定层（JNI 声明 / 测试接缝 / 引擎 API）。
# 解码器辅助函数里的 ByteArray 下标是数组索引（Int 正确），不属于书籍坐标空间。
BOUNDARY_FILES = (
    "com/novelreader/android/core/NrCore.kt",
    "com/novelreader/android/core/NrAbi.kt",
    "com/novelreader/android/core/NrEngine.kt",
)

# 64 位字段名：绑定层里出现 `name: Int` 就是 32 位退化。
INT_DEGRADATION_NAMES = (
    "engine",
    "bookId",
    "contentUnitId",
    "offset",
    "length",
    "requestId",
    "layoutId",
    "startByte",
    "endByte",
    "totalBytes",
    "maxWindowBytes",
    "bookmarkId",
    "ptr",
    "capacity",
)

HEADER_FUNC_RE = re.compile(r"^\s*(?:int32_t|uint32_t)\s+(nr_[A-Za-z0-9_]+)\s*\(", re.M)
HEADER_MACRO_RE = re.compile(r"^\s*#define\s+(NR_[A-Za-z0-9_]+)\s+([0-9]+)[uU]?\s*(?:/\*.*\*/)?\s*$", re.M)
KOTLIN_EXTERNAL_RE = re.compile(r"\bexternal\s+fun\s+(nr_[A-Za-z0-9_]+)\s*\(")
KOTLIN_CONST_RE = re.compile(r"^\s*const\s+val\s+(NR_[A-Za-z0-9_]+)\s*=\s*([0-9]+)\s*[lL]?\s*$", re.M)
JNI_MACRO_RE = re.compile(r"#define\s+NR_JNI\(name\)\s+([A-Za-z0-9_]+)##name")
JNI_WRAPPER_RE = re.compile(r"\bNR_JNI\(([A-Za-z0-9_]+)\)")
JNI_SYMBOL_RE = re.compile(r"\bJava_[A-Za-z0-9_]+")


class Report:
    """收集 PASS/FAIL/SKIP 并给出退出码。"""

    def __init__(self) -> None:
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


# ------------------------------------------------------------------ 解析


def parse_header_entries(text: str) -> list[str]:
    return [match.group(1) for match in HEADER_FUNC_RE.finditer(text)]


def parse_header_macros(text: str) -> dict[str, int]:
    return {m.group(1): int(m.group(2)) for m in HEADER_MACRO_RE.finditer(text)}


def parse_kotlin_files(directory: Path) -> dict[str, str]:
    files: dict[str, str] = {}
    for path in sorted(directory.rglob("*.kt")):
        files[str(path.relative_to(directory))] = path.read_text(encoding="utf-8", errors="replace")
    return files


def parse_kotlin_externals(files: dict[str, str]) -> list[tuple[str, str]]:
    found: list[tuple[str, str]] = []
    for rel, text in sorted(files.items()):
        for match in KOTLIN_EXTERNAL_RE.finditer(text):
            found.append((match.group(1), rel))
    return found


def parse_kotlin_consts(files: dict[str, str]) -> dict[str, tuple[int, str]]:
    consts: dict[str, tuple[int, str]] = {}
    for rel, text in sorted(files.items()):
        for match in KOTLIN_CONST_RE.finditer(text):
            consts.setdefault(match.group(1), (int(match.group(2)), rel))
    return consts


def jni_prefix(c_text: str) -> str | None:
    match = JNI_MACRO_RE.search(c_text)
    return match.group(1) if match else None


def jni_wrappers(c_text: str) -> list[str]:
    return [name for name in JNI_WRAPPER_RE.findall(c_text) if name != "name"]


def escape_jni(name: str) -> str:
    """JNI 短名转义：方法名里的 `_` → `_1`。"""
    return name.replace("_", "_1")


def exported_java_symbols(so_path: Path) -> set[str]:
    """用 nm 读取编译产物中的 `Java_*` 导出符号。"""
    if shutil.which("nm") is None:
        raise RuntimeError("nm 不可用")
    proc = subprocess.run(
        ["nm", "-D", "--defined-only", "--format=posix", str(so_path)],
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"nm 读取 {so_path} 失败：{proc.stderr.strip()[:200]}")
    symbols: set[str] = set()
    for line in proc.stdout.splitlines():
        parts = line.split()
        if parts and parts[0].startswith("Java_"):
            symbols.add(parts[0])
    return symbols


def strip_kotlin_comments(text: str) -> str:
    """剥离 Kotlin 注释（`//...` 与可嵌套的 `/* ... */`），保留代码与字符串字面量。

    注释不是可执行代码：文档里提到某 API 名称不应被判成违规；但字符串字面量可能是
    真正被执行的模式串，因此保留。
    """
    out: list[str] = []
    i = 0
    n = len(text)
    while i < n:
        char = text[i]
        nxt = text[i + 1] if i + 1 < n else ""
        if char == "/" and nxt == "/":
            while i < n and text[i] != "\n":
                i += 1
            continue
        if char == "/" and nxt == "*":
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
        out.append(char)
        i += 1
    return "".join(out)


def code_only(files: dict[str, str]) -> dict[str, str]:
    """剥离注释后的 Kotlin 文件集（用于词法规则扫描）。"""
    return {rel: strip_kotlin_comments(text) for rel, text in files.items()}


def kotlin_function_body(text: str, signature: str) -> str | None:
    """按花括号配平截取 Kotlin 函数体（含签名），找不到返回 None。"""
    index = text.find(signature)
    if index < 0:
        return None
    depth = 0
    started = False
    out: list[str] = []
    for char in text[index:]:
        out.append(char)
        if char == "{":
            depth += 1
            started = True
        elif char == "}":
            depth -= 1
            if started and depth == 0:
                break
    return "".join(out) if started else None


# ------------------------------------------------------------------ 检查


def check_kotlin_declarations(
    header_entries: list[str],
    declarations: list[tuple[str, str]],
    report: Report,
) -> None:
    print("[1] Kotlin `external fun` 声明 ↔ 头文件 18 项入口")
    text = f"{len(declarations)} 项"
    if not declarations:
        report.fail("Kotlin 目录内没有任何 `external fun nr_*` 声明（薄封装未实现）")
        return
    names = [name for name, _ in declarations]
    carriers = sorted({rel for _, rel in declarations})
    if carriers != [CARRIER_SUFFIX]:
        report.fail(
            f"`external fun` 声明必须只在 `{CARRIER_SUFFIX}`：实际出现在 {carriers}"
        )
    else:
        report.ok(f"18 项 JNI 声明唯一承载于 `{CARRIER_SUFFIX}`")

    missing = [name for name in header_entries if name not in set(names)]
    extra = [name for name in names if name not in set(header_entries)]
    if missing:
        report.fail(f"Kotlin 未声明头文件入口（{len(missing)} 项）: {', '.join(missing)}")
    if extra:
        report.fail(f"Kotlin 声明了头文件不存在的入口（{len(extra)} 项）: {', '.join(extra)}")
    if not missing and not extra:
        report.ok(f"Kotlin 声明集合与头文件一致（{len(names)} 项）")

    if len(names) != len(set(names)):
        report.fail(f"Kotlin 存在重复声明：{sorted({n for n in names if names.count(n) > 1})}")
    else:
        report.ok("Kotlin 无重复声明")

    if names == header_entries:
        report.ok(f"Kotlin 声明顺序与头文件一致（{text}）")
    else:
        report.fail(f"Kotlin 声明顺序与头文件不一致：实际 {names}，期望 {header_entries}")


def check_jni_escape(entries: list[str], c_text: str, report: Report) -> set[str]:
    print("[2] JNI 符号转义（文本层）")
    expected = {escape_jni(name) for name in entries}
    prefix = jni_prefix(c_text)
    if prefix is None:
        report.fail("`android/jni/nr_jni.c` 缺少 `#define NR_JNI(name) ...##name` 符号前缀宏")
        return set()
    if prefix != JNI_CLASS_PREFIX:
        report.fail(
            f"JNI 符号前缀 = `{prefix}`，冻结值为 `{JNI_CLASS_PREFIX}`"
            f"（包名/类名/库名见 android/jni/README.md §1）"
        )
    else:
        report.ok(f"JNI 符号前缀冻结：`{prefix}`")

    wrappers = jni_wrappers(c_text)
    if len(wrappers) != len(set(wrappers)):
        report.fail(f"C 层存在重复的 NR_JNI 定义：{sorted({w for w in wrappers if wrappers.count(w) > 1})}")
    missing = sorted(expected - set(wrappers))
    extra = sorted(set(wrappers) - expected)
    if missing:
        report.fail(f"C 层缺少转义名定义（{len(missing)} 项）: {', '.join(missing)}")
    if extra:
        report.fail(f"C 层出现头文件之外的 JNI 定义（{len(extra)} 项）: {', '.join(extra)}")
    if not missing and not extra:
        report.ok(f"C 层 {len(wrappers)} 项定义全部使用转义名（`_` → `_1`）")

    unescaped = []
    for name in entries:
        raw_symbol = prefix + name
        if name != escape_jni(name) and re.search(rf"\b{re.escape(raw_symbol)}\b", c_text):
            unescaped.append(raw_symbol)
    if unescaped:
        report.fail(
            "C 层出现未转义的 JNI 符号（真机/宿主机实测会 UnsatisfiedLinkError）："
            + ", ".join(unescaped)
        )
    else:
        report.ok("C 层无未转义的 JNI 短名（方法名下划线全部转义）")
    return expected


def check_exported_symbols(entries: list[str], so_path: Path | None, report: Report) -> None:
    print("[2b] 编译产物导出符号（nm -D，二进制层）")
    if so_path is None or not so_path.is_file():
        report.skip(
            f"编译产物不存在（{so_path}）：文本层转义校验已执行，二进制符号比对跳过"
            "（可由 scripts/build-android-libs.sh 重建）"
        )
        return
    try:
        symbols = exported_java_symbols(so_path)
    except RuntimeError as error:
        report.skip(f"无法读取编译产物符号：{error}")
        return
    expected = {JNI_CLASS_PREFIX + escape_jni(name) for name in entries}
    if not symbols:
        report.fail(f"{so_path} 未导出任何 `Java_*` 符号（JNI 层未编入）")
        return
    missing = sorted(expected - symbols)
    extra = sorted(symbols - expected)
    if missing:
        report.fail(f"{so_path} 缺少 JNI 导出（{len(missing)} 项）: {', '.join(missing)}")
    if extra:
        report.fail(f"{so_path} 导出了契约之外的 JNI 符号：{', '.join(extra)}")
    if not missing and not extra:
        report.ok(f"{so_path.name} 真实导出 {len(symbols)} 项 JNI 符号，与 Kotlin 声明逐项一致")


def check_error_codes(
    header_macros: dict[str, int],
    kotlin_consts: dict[str, tuple[int, str]],
    files: dict[str, str],
    report: Report,
) -> None:
    print("[3] 错误码常量与业务映射")
    for name in FROZEN_ERROR_MACROS:
        if name not in header_macros:
            report.fail(f"头文件缺少错误码宏 {name}")
            continue
        if name not in kotlin_consts:
            report.fail(f"Kotlin 缺少错误码常量 {name}")
        elif kotlin_consts[name][0] != header_macros[name]:
            report.fail(
                f"错误码常量 {name} = {kotlin_consts[name][0]}（{kotlin_consts[name][1]}），"
                f"头文件冻结值 = {header_macros[name]}"
            )
    if all(name in kotlin_consts and name in header_macros for name in FROZEN_ERROR_MACROS):
        mismatched = [
            name
            for name in FROZEN_ERROR_MACROS
            if kotlin_consts[name][0] != header_macros[name]
        ]
        if not mismatched:
            report.ok(f"12 项错误码常量与头文件数值一致（0..=11）")

    core_text = "\n".join(files.values())
    unmapped = [name for name in FROZEN_ERROR_MACROS if core_text.count(name) < 2]
    if unmapped:
        report.fail(
            f"错误码未进入业务映射（只出现 1 次，说明没有 when 分支覆盖）：{', '.join(unmapped)}"
        )
    else:
        report.ok("12 项错误码全部进入 `NrErrorKind.of` 的业务映射")

    if re.search(r"fun\s+of\s*\(\s*rawCode\s*:\s*Int\s*\)\s*:\s*NrErrorKind", core_text):
        report.ok("业务映射入口为 `fun of(rawCode: Int): NrErrorKind`（只吃错误码数值）")
    else:
        report.fail("缺少 `fun of(rawCode: Int): NrErrorKind`：错误码 → 业务分类入口不符合契约")

    code_files = code_only(files)
    hits = [(rel, token) for rel, text in code_files.items() for token in FORBIDDEN_TEXT_PARSING if token in text]
    if hits:
        for rel, token in hits[:10]:
            report.fail(f"{rel} 出现错误文本解析原语 `{token}`；头文件 §2 禁止用文本判断业务分支")
    else:
        report.ok("core/ 无错误文本匹配原语（`contains/startsWith/endsWith/matches/Regex` 均未出现）")


def check_handle_semantics(files: dict[str, str], report: Report) -> None:
    print("[4] 句柄与销毁语义")
    code_files = code_only(files)
    core_text = "\n".join(code_files.values())

    create = re.search(r"external\s+fun\s+nr_engine_create\s*\(([^)]*)\)\s*:\s*(\w+)", core_text)
    if create is None:
        report.fail("缺少 `nr_engine_create` 声明，句柄契约无法校验")
    elif create.group(2) != "Long":
        report.fail(f"`nr_engine_create` 返回 `{create.group(2)}`，句柄必须是 `Long`（0 = 失败）")
    else:
        report.ok("`nr_engine_create` 返回 `Long`（0 = 失败；与错误码语义区分）")

    degraded: list[str] = []
    for rel in BOUNDARY_FILES:
        text = code_files.get(rel, "")
        for name in INT_DEGRADATION_NAMES:
            if re.search(rf"\b{name}\s*:\s*Int\b", text):
                degraded.append(f"{rel}:{name}")
    if degraded:
        report.fail(
            "绑定层 64 位字段退化为 Int（头文件 §4 禁止 32 位截断）：" + ", ".join(sorted(degraded))
        )
    else:
        report.ok(
            f"绑定层 {len(BOUNDARY_FILES)} 个文件的句柄/偏移/长度字段全部为 `Long`（无 32 位退化）"
        )

    load_library = [
        (rel, text.count(f'System.loadLibrary("{JNI_LIBRARY}")')) for rel, text in code_files.items()
    ]
    total = sum(count for _, count in load_library)
    carriers = sorted(rel for rel, count in load_library if count > 0)
    if total != 1 or carriers != [CARRIER_SUFFIX]:
        report.fail(
            f'`System.loadLibrary("{JNI_LIBRARY}")` 必须恰好出现 1 次且只在 `{CARRIER_SUFFIX}`：'
            f"实际 {total} 次，位置 {carriers}"
        )
    else:
        report.ok(f'自包含 `.so` 只经 `System.loadLibrary("{JNI_LIBRARY}")` 加载（无绝对路径绕过）')

    engine_rel = next((rel for rel in files if rel.endswith("core/NrEngine.kt")), None)
    engine_text = code_files.get(engine_rel, "") if engine_rel else ""
    destroy_body = kotlin_function_body(engine_text, "fun destroy(): Int {")
    if destroy_body is None:
        report.fail("未找到 `fun destroy(): Int`：句柄销毁语义无法校验")
    else:
        flag = destroy_body.find("isDestroyed = true")
        native_call = destroy_body.find("abi.engineDestroy")
        if flag < 0 or native_call < 0:
            report.fail("`destroy()` 必须同时包含销毁标志置位与 native 销毁调用")
        elif flag > native_call:
            report.fail("`destroy()` 必须先置销毁标志再调用 native（否则并发下有二重释放窗口）")
        else:
            report.ok("`destroy()` 先置销毁标志再调用 native（销毁只触发一次）")
        if "NR_ERR_ENGINE_DESTROYED" in destroy_body and "if (isDestroyed)" in destroy_body:
            report.ok("重复销毁不触碰 native，直接返回 `NR_ERR_ENGINE_DESTROYED`")
        else:
            report.fail("重复销毁必须短路返回 `NR_ERR_ENGINE_DESTROYED`（不得再次调用 native）")

    guard = kotlin_function_body(engine_text, "private fun requireAlive() {")
    if guard is None or "NR_ERR_ENGINE_DESTROYED" not in guard:
        report.fail("缺少销毁后的调用守卫（requireAlive 抛 `NR_ERR_ENGINE_DESTROYED`）")
    else:
        report.ok("销毁后所有入口由 `requireAlive()` 短路（不触碰 native）")

    # 作用域 = 引擎/ABI 承载层 core/**（只有这一层包裹 native 调用）；
    # reader/** 的 ViewModel 串行化锁不属于本规则客体，见 [6b] 行为型规则。
    core_code_files = {
        rel: text for rel, text in code_files.items() if rel.startswith(CORE_DIR_PREFIX)
    }
    if not core_code_files:
        report.fail(
            f"未找到 `{CORE_DIR_PREFIX}` 下的 Kotlin 文件：Kotlin 侧锁规则失去客体（不得空转通过）"
        )
        return
    lock_hits = [
        (rel, token)
        for rel, text in core_code_files.items()
        for token in FORBIDDEN_LOCKS
        if re.search(rf"\b{re.escape(token)}\b", text)
    ]
    if lock_hits:
        for rel, token in lock_hits[:10]:
            report.fail(
                f"{rel} 出现 Kotlin 侧锁 `{token}`：引擎/ABI 承载层（`{CORE_DIR_PREFIX}`）包裹 native 调用，"
                "头文件 §7 要求取消标志与引擎锁分离，这一层加锁会让跨线程取消（长任务运行期间）被阻塞"
            )
    else:
        report.ok(
            f"core/（{len(core_code_files)} 个文件）无 Kotlin 侧锁"
            "（取消可在长任务运行期间从另一线程生效）"
        )


def check_buffer_release(
    declarations: list[tuple[str, str]],
    files: dict[str, str],
    report: Report,
) -> None:
    print("[5] 缓冲区归还契约")
    carrier_text = files.get(CARRIER_SUFFIX, "")
    bad = []
    for name in BUFFER_ENTRIES:
        match = re.search(rf"external\s+fun\s+{name}\s*\(([^)]*)\)\s*:", carrier_text, re.S)
        if match is None:
            bad.append(name)
            continue
        if not re.search(r"outBuffer\s*:\s*LongArray\s*,?\s*$", match.group(1).strip()):
            bad.append(name)
    if bad:
        report.fail(
            "四个缓冲区入口的末位必须是 `outBuffer: LongArray`（归还槽位）：" + ", ".join(bad)
        )
    else:
        report.ok(f"{len(BUFFER_ENTRIES)} 个缓冲区入口均以 `outBuffer: LongArray` 收尾")

    if re.search(r"external\s+fun\s+nr_release_buffer\s*\(\s*ptr\s*:\s*Long\s*,\s*len\s*:\s*Long\s*,\s*capacity\s*:\s*Long\s*\)\s*:\s*Int", carrier_text):
        report.ok("`nr_release_buffer(ptr, len, capacity): Int` 声明与头文件一致")
    else:
        report.fail("`nr_release_buffer` 声明必须为 `(Long, Long, Long): Int`")

    seam_rel = "com/novelreader/android/core/NrAbi.kt"
    allowed_release_files = {seam_rel, "com/novelreader/android/core/NrBuffers.kt"}
    release_files = sorted(rel for rel, text in files.items() if "NrCore.nr_release_buffer(" in text)
    unexpected = [rel for rel in release_files if rel not in allowed_release_files]
    if unexpected:
        report.fail(
            f"`NrCore.nr_release_buffer(...)` 只允许出现在接缝（{seam_rel}）与归还器（NrBuffers.kt）："
            f"实际额外出现于 {unexpected}"
        )
    elif "com/novelreader/android/core/NrBuffers.kt" not in release_files:
        report.fail("`NrBufferHandle` 未调用 `NrCore.nr_release_buffer`：归还路径缺失")
    else:
        report.ok("归还只有一个持有者路径（JNI 透传在 NrAbi.kt，归还路径只在 NrBuffers.kt）")

    buffers_rel = next((rel for rel in files if rel.endswith("core/NrBuffers.kt")), None)
    buffers_text = files.get(buffers_rel, "") if buffers_rel else ""
    release_body = kotlin_function_body(buffers_text, "fun release(releaser: NrReleaser = NrReleaser.JNI): Int {")
    if release_body is None:
        report.fail("未找到 `NrBufferHandle.release(...)`：归还语义无法校验")
        return
    if "if (released)" in release_body and "NR_ERR_INVALID_ARGUMENT" in release_body:
        report.ok("归还前检查 `released` 标志：重复归还返回 `NR_ERR_INVALID_ARGUMENT` 且不再触碰 native")
    else:
        report.fail("归还函数必须检查 `released` 并拒绝重复归还（`NR_ERR_INVALID_ARGUMENT`）")
    if "isEmpty" in release_body and "NR_ERR_OK" in release_body:
        report.ok("`(0,0,0)` 空凭据按安全 no-op 返回 `NR_ERR_OK`（零长度结果不登记缓冲区）")
    else:
        report.fail("空凭据 `(0,0,0)` 必须是安全 no-op（返回 `NR_ERR_OK`）")

    if "requireSameSlots" in buffers_text and "requireLenMatches" in buffers_text:
        report.ok("提供结构体槽位 / 长度一致性断言（`requireSameSlots`、`requireLenMatches`）")
    else:
        report.fail("缺少结构体槽位 / 长度一致性断言：无法证明内嵌槽位与 outBuffer 是同一份值")

    engine_rel = next((rel for rel in files if rel.endswith("core/NrEngine.kt")), None)
    engine_text = files.get(engine_rel, "") if engine_rel else ""
    same_slots = engine_text.count("requireSameSlots(")
    len_matches = engine_text.count("requireLenMatches(")
    if same_slots >= 3 and len_matches >= 1:
        report.ok(f"引擎在 {same_slots} 处交叉断言结构体槽位、{len_matches} 处断言长度一致")
    else:
        report.fail(
            "引擎必须对 `nr_get_position` / `nr_get_chapters` / `nr_get_last_error` 的结构体槽位"
            f"逐一交叉断言（实际 {same_slots} 处），并至少一处长度断言（实际 {len_matches} 处）"
        )


def check_cancel(files: dict[str, str], report: Report) -> None:
    print("[6] 取消（协作式）契约")
    carrier_text = files.get(CARRIER_SUFFIX, "")
    if re.search(r"external\s+fun\s+nr_cancel_request\s*\(\s*engine\s*:\s*Long\s*,\s*requestId\s*:\s*Long\s*\)", carrier_text):
        report.ok("`nr_cancel_request(engine: Long, requestId: Long)` 声明存在")
    else:
        report.fail("`nr_cancel_request` 声明缺失或签名不符")

    engine_rel = next((rel for rel in files if rel.endswith("core/NrEngine.kt")), None)
    engine_text = files.get(engine_rel, "") if engine_rel else ""
    cancel_body = kotlin_function_body(engine_text, "fun cancelRequest(requestId: Long) {")
    if cancel_body is None:
        report.fail("未找到 `fun cancelRequest(requestId: Long)`")
    elif "requestId == 0L" in cancel_body and "NR_ERR_INVALID_ARGUMENT" in cancel_body:
        report.ok("`cancelRequest` 前置校验 `requestId != 0`（请求号 0 永不合法）")
    else:
        report.fail("`cancelRequest` 必须前置拒绝 `requestId == 0`（头文件 §7）")

    if "NR_ERR_CANCELLED" in engine_text and "partial" in engine_text:
        report.ok("取消按部分成功处理（`NR_ERR_CANCELLED` + `partial` 结果结构体回写）")
    else:
        report.fail("取消必须按部分成功处理（`NR_ERR_CANCELLED` 携带 partial 结果，不得当普通失败丢弃）")


def check_reader_cancel_reachability(files: dict[str, str], report: Report) -> None:
    """[6b] `reader/**` 的取消入口不得取状态串行化门（头文件 §7 的行为型可证形式）。

    语义：`reader/**` 的 ViewModel 按 AR4 用同一把 Mutex 门把「打开/切章/翻页/重排/保存位置」
    串行化；§7 要求的「取消可从另一线程在长任务运行期间生效」由此成为**可机器证明**的性质：
    只要每个取消入口（`fun cancel*`）体内都不取该门，长任务持门期间取消路径就不会被阻塞。

    规则不空转：`reader/**` 缺文件、缺串行化门、或缺取消入口，一律判 FAIL ——
    否则删除取消入口或删除门都能让本规则"静静地通过"。
    """
    print("[6b] reader/ 取消入口不得取串行化门（头文件 §7 行为型反例）")
    code_files = code_only(files)
    reader_files = {
        rel: text for rel, text in code_files.items() if rel.startswith(READER_DIR_PREFIX)
    }
    if not reader_files:
        report.fail(
            f"未找到 `{READER_DIR_PREFIX}` 下的 Kotlin 文件：取消可达性规则失去客体（不得空转通过）"
        )
        return

    gates: dict[str, str] = {}
    for rel, text in sorted(reader_files.items()):
        for match in GATE_DECL_RE.finditer(text):
            gates.setdefault(match.group(1), rel)
    if not gates:
        report.fail(
            f"`{READER_DIR_PREFIX}` 未声明状态串行化门（`val <gate> = Mutex()`）："
            "AR4 的串行化与 §7 的取消可达性都无从证明"
        )
        return

    entries: list[tuple[str, str, str]] = []
    for rel, text in sorted(reader_files.items()):
        for match in CANCEL_ENTRY_RE.finditer(text):
            body = kotlin_function_body(text, match.group(0))
            if body is None:
                continue
            entries.append((match.group(1), rel, body))
    if not entries:
        report.fail(
            "`reader/**` 未找到取消入口 `fun cancel*`：无法证明「取消可在长任务运行期间"
            "从另一线程生效」（头文件 §7）；规则不得因客体缺失而空转通过"
        )
        return

    violations: list[str] = []
    for name, rel, body in entries:
        for gate in gates:
            if re.search(rf"\b{re.escape(gate)}\s*\.\s*(?:withLock|lock)\b", body):
                violations.append(f"{rel}::{name}")
    if violations:
        for item in violations[:10]:
            report.fail(
                f"{item} 的取消入口体内取了状态串行化门（`<gate>.withLock` / `<gate>.lock`）："
                "长任务持门期间该取消入口会被阻塞，违反头文件 §7（跨线程取消）"
            )
        return
    report.ok(
        f"reader/ 的 {len(entries)} 个取消入口（{', '.join(name for name, _, _ in entries)}）"
        f"均不取状态串行化门 `{'/'.join(sorted(gates))}`：长任务持门期间仍可从另一线程取消"
    )


def check_kotlin_scope(files: dict[str, str], report: Report) -> None:
    print("[7] Kotlin 业务边界（core/ 不持有平台状态、不引用探针）")
    core_files = {rel: text for rel, text in files.items() if rel.startswith(CORE_DIR_PREFIX)}
    if not core_files:
        report.fail(f"未找到 core/ 目录下的 Kotlin 文件（{CORE_DIR_PREFIX}）")
        return
    code_files = code_only(core_files)
    hits = [
        (rel, token)
        for rel, text in code_files.items()
        for token in FORBIDDEN_PLATFORM_STATE
        if token in text
    ]
    if hits:
        for rel, token in hits[:10]:
            report.fail(f"{rel} 引用平台状态 `{token}`：核心层只接受值类型参数，不持有 UI 状态")
    else:
        report.ok(f"{len(core_files)} 个 core/ 文件均无 Compose/Activity/ViewModel 依赖")

    probe_hits = [rel for rel, text in code_files.items() if "experiments/" in text]
    if probe_hits:
        report.fail(f"Kotlin 引用了验证探针路径 `experiments/`：{probe_hits}")
    else:
        report.ok("Kotlin 未引用 `experiments/` 探针")


# ------------------------------------------------------------------ 自测


def self_test() -> Report:
    """fixture 反向用例：证明上述检查会报出违规，而不是空检查。"""
    report = Report()
    print("[self-test] fixture 反向用例（每条规则都必须能报出违规）")
    header_entries = ["nr_alpha", "nr_beta"]
    full_macros = {name: index for index, name in enumerate(FROZEN_ERROR_MACROS)}
    full_consts = {name: (index, CARRIER_SUFFIX) for index, name in enumerate(FROZEN_ERROR_MACROS)}
    mapped_text = " ".join(list(full_consts) * 2) + " fun of(rawCode: Int): NrErrorKind"

    good_c = (
        "#define NR_JNI(name) Java_com_novelreader_android_core_NrCore_##name\n"
        "JNIEXPORT jint JNICALL NR_JNI(nr_1alpha)(JNIEnv *env, jobject self) {}\n"
        "JNIEXPORT jint JNICALL NR_JNI(nr_1beta)(JNIEnv *env, jobject self) {}\n"
    )
    good_core = (
        'object NrCore { init { System.loadLibrary("nr_jni") } '
        "external fun nr_engine_create(maxWindowBytes: Long): Long "
        "external fun nr_engine_destroy(engine: Long): Int }"
    )
    good_engine = (
        "class NrEngine { var isDestroyed = false\n"
        "    fun destroy(): Int {\n        if (isDestroyed) return NrErrors.NR_ERR_ENGINE_DESTROYED\n"
        "        isDestroyed = true\n        return abi.engineDestroy(handle)\n    }\n"
        "    private fun requireAlive() {\n        if (isDestroyed) throw NrException(NR_ERR_ENGINE_DESTROYED)\n    } }"
    )
    good_buffers = (
        "class NrBufferHandle {\n"
        "    fun release(releaser: NrReleaser = NrReleaser.JNI): Int {\n"
        "        if (released) return NR_ERR_INVALID_ARGUMENT\n"
        "        if (isEmpty) { released = true; return NR_ERR_OK }\n"
        "        return releaser.release(ptr, len, capacity)\n    }\n"
        "    companion object {\n"
        "        fun requireSameSlots(handle: NrBufferHandle, struct: Long) {}\n"
        "        fun requireLenMatches(handle: NrBufferHandle, expected: Long) {}\n    }\n"
        "    fun passthrough() { NrCore.nr_release_buffer(ptr, len, capacity) } }\n"
    )

    case = Report()
    check_kotlin_declarations(
        header_entries,
        [("nr_alpha", CARRIER_SUFFIX), ("nr_beta", CARRIER_SUFFIX)],
        case,
    )
    _expect(report, "完整声明 fixture 通过（正向对照）", not case.failures, case.failures)

    case = Report()
    check_kotlin_declarations(header_entries, [("nr_alpha", CARRIER_SUFFIX)], case)
    _expect(report, "缺失 Kotlin 声明被报出", any("未声明" in item for item in case.failures), case.failures)

    case = Report()
    check_kotlin_declarations(
        header_entries,
        [("nr_alpha", CARRIER_SUFFIX), ("nr_gamma", CARRIER_SUFFIX)],
        case,
    )
    _expect(report, "多余 Kotlin 声明被报出", any("不存在" in item for item in case.failures), case.failures)

    case = Report()
    check_kotlin_declarations(
        header_entries,
        [("nr_alpha", CARRIER_SUFFIX), ("nr_beta", "com/novelreader/android/core/Other.kt")],
        case,
    )
    _expect(report, "第二处 external 声明被报出", any("必须只在" in item for item in case.failures), case.failures)

    case = Report()
    check_jni_escape(header_entries, good_c, case)
    _expect(report, "转义名 fixture 通过（正向对照）", not case.failures, case.failures)

    case = Report()
    check_jni_escape(
        header_entries,
        good_c
        + "JNIEXPORT jint JNICALL "
        + "Java_com_novelreader_android_core_NrCore_nr_alpha(JNIEnv *env, jobject self) {}\n",
        case,
    )
    _expect(
        report,
        "未转义 JNI 符号被报出",
        any("未转义" in item for item in case.failures),
        case.failures,
    )

    missing_io_text = (
        " ".join([name for name in full_consts if name != "NR_ERR_IO"] * 2)
        + " fun of(rawCode: Int): NrErrorKind"
    )
    case = Report()
    check_error_codes(
        full_macros,
        {name: value for name, value in full_consts.items() if name != "NR_ERR_IO"},
        {"a.kt": missing_io_text},
        case,
    )
    _expect(
        report,
        "缺失错误码常量与未映射错误码被报出",
        any("缺少错误码常量" in item for item in case.failures)
        and any("业务映射" in item for item in case.failures),
        case.failures,
    )

    broken_consts = dict(full_consts)
    broken_consts["NR_ERR_OK"] = (7, CARRIER_SUFFIX)
    case = Report()
    check_error_codes(full_macros, broken_consts, {"a.kt": mapped_text}, case)
    _expect(
        report,
        "错误码数值不一致被报出",
        any("冻结值" in item for item in case.failures),
        case.failures,
    )

    case = Report()
    check_error_codes(
        full_macros,
        full_consts,
        {"a.kt": mapped_text + '\nval x = message.contains("boom")'},
        case,
    )
    _expect(report, "错误文本解析被报出", any("contains" in item for item in case.failures), case.failures)

    case = Report()
    check_handle_semantics(
        {
            CARRIER_SUFFIX: good_core,
            "com/novelreader/android/core/NrEngine.kt": good_engine + "\n    fun bad(engine: Int) = Unit\n",
        },
        case,
    )
    _expect(
        report,
        "句柄退化为 Int 被报出",
        any("退化为 Int" in item for item in case.failures),
        case.failures,
    )

    case = Report()
    check_handle_semantics(
        {
            CARRIER_SUFFIX: good_core,
            "com/novelreader/android/core/NrEngine.kt": (
                "class NrEngine { var isDestroyed = false\n"
                "    fun destroy(): Int {\n        val code = abi.engineDestroy(handle)\n"
                "        isDestroyed = true\n        return code\n    }\n"
                "    private fun requireAlive() {\n        if (isDestroyed) throw NrException(NR_ERR_ENGINE_DESTROYED)\n    }\n"
                "    private val mutex = Mutex() }\n"
            ),
        },
        case,
    )
    _expect(
        report,
        "加锁与销毁顺序错误被报出",
        any("Kotlin 侧锁" in item for item in case.failures)
        and any("先置销毁标志" in item for item in case.failures),
        case.failures,
    )

    case = Report()
    check_handle_semantics(
        {
            CARRIER_SUFFIX: good_core,
            "com/novelreader/android/core/NrEngine.kt": good_engine,
        },
        case,
    )
    _expect(report, "完整句柄 fixture 通过（正向对照）", not case.failures, case.failures)

    # [4] 收窄后：core/ 内的锁仍被抓（越界样本不得因收窄而漏掉）
    case = Report()
    check_handle_semantics(
        {
            CARRIER_SUFFIX: good_core,
            "com/novelreader/android/core/NrEngine.kt": good_engine
            + "\n    private val guard = Mutex()\n",
        },
        case,
    )
    _expect(
        report,
        "core/ 内的 Kotlin 侧锁仍被报出（收窄后越界样本仍可抓）",
        any("Kotlin 侧锁" in item for item in case.failures),
        case.failures,
    )
    _expect(
        report,
        "core/ 锁违规文案点明作用域为引擎/ABI 承载层",
        any("引擎/ABI 承载层" in item for item in case.failures),
        case.failures,
    )

    # [4] 收窄后：reader/ 的串行化 Mutex 不再误报（AR4 要求）
    case = Report()
    check_handle_semantics(
        {
            CARRIER_SUFFIX: good_core,
            "com/novelreader/android/core/NrEngine.kt": good_engine,
            READER_VIEWMODEL_SUFFIX: (
                "import kotlinx.coroutines.sync.Mutex\n"
                "class ReaderViewModel {\n"
                "    private val gate = Mutex()\n"
                "    private fun notACancelEntry() { gate.withLock { } }\n"
                "}\n"
            ),
        },
        case,
    )
    _expect(
        report,
        "reader/ 的串行化 Mutex 不再误报（作用域与规则文案一致）",
        not any("Kotlin 侧锁" in item for item in case.failures),
        case.failures,
    )

    good_reader = (
        "import kotlinx.coroutines.sync.Mutex\n"
        "import kotlinx.coroutines.sync.withLock\n"
        "class ReaderViewModel {\n"
        "    private val gate = Mutex()\n"
        "    private fun launchSerial(block: () -> Unit) { gate.withLock { block() } }\n"
        "    fun cancelIndexRebuild() {\n"
        "        work.launch { engine.cancelRequest(id) }\n"
        "    }\n"
        "    fun cancelInFlightWork(): Int {\n"
        "        val active = synchronized(inFlight) { inFlight.filter { it.isActive } }\n"
        "        return active.size\n"
        "    }\n"
        "}\n"
    )

    case = Report()
    check_reader_cancel_reachability({READER_VIEWMODEL_SUFFIX: good_reader}, case)
    _expect(report, "[6b] 取消入口 fixture 通过（正向对照）", not case.failures, case.failures)

    case = Report()
    check_reader_cancel_reachability(
        {
            READER_VIEWMODEL_SUFFIX: good_reader.replace(
                "    fun cancelIndexRebuild() {\n",
                "    fun cancelIndexRebuild() {\n        gate.withLock { }\n",
            )
        },
        case,
    )
    _expect(
        report,
        "[6b] 取消入口取串行化门被报出",
        any("取了状态串行化门" in item for item in case.failures),
        case.failures,
    )

    case = Report()
    check_reader_cancel_reachability(
        {
            READER_VIEWMODEL_SUFFIX: good_reader.replace("fun cancelIndexRebuild", "fun rebuildIndex").replace(
                "fun cancelInFlightWork", "fun hostStopped"
            )
        },
        case,
    )
    _expect(
        report,
        "[6b] 取消入口缺失被报出（规则不得空转）",
        any("未找到取消入口" in item for item in case.failures),
        case.failures,
    )

    case = Report()
    check_reader_cancel_reachability(
        {
            READER_VIEWMODEL_SUFFIX: good_reader.replace(
                "private val gate = Mutex()", "private val nothing = 1"
            )
        },
        case,
    )
    _expect(
        report,
        "[6b] 串行化门缺失被报出（规则不得空转）",
        any("未声明状态串行化门" in item for item in case.failures),
        case.failures,
    )

    case = Report()
    check_reader_cancel_reachability({"com/novelreader/android/core/NrCore.kt": "object NrCore"}, case)
    _expect(
        report,
        "[6b] reader/ 目录缺失被报出（规则不得空转）",
        any("取消可达性规则失去客体" in item for item in case.failures),
        case.failures,
    )

    good_carrier = good_core + (
        "\n    external fun nr_read_window(engine: Long, chapterIndex: Int, offset: Long, length: Long,\n"
        "                                outWindow: ByteArray, outBuffer: LongArray,): ByteArray?\n"
        "    external fun nr_get_position(engine: Long, bookId: Long, outResult: ByteArray,\n"
        "                                 outBuffer: LongArray): ByteArray?\n"
        "    external fun nr_get_chapters(engine: Long, bookId: Long, startSequence: Int, maxCount: Int,\n"
        "                                 outBatch: ByteArray, outBuffer: LongArray): ByteArray?\n"
        "    external fun nr_get_last_error(engine: Long, outInfo: ByteArray, outBuffer: LongArray): ByteArray?\n"
        "    external fun nr_release_buffer(ptr: Long, len: Long, capacity: Long): Int\n"
    )
    buffer_declarations = [(name, CARRIER_SUFFIX) for name in BUFFER_ENTRIES + ["nr_release_buffer"]]
    good_engine_buffer = (
        "class NrEngine {\n"
        "    fun a() { NrBufferHandle.requireSameSlots(x, 1L, 2L, 3L) }\n"
        "    fun b() { NrBufferHandle.requireSameSlots(x, 1L, 2L, 3L) }\n"
        "    fun c() { NrBufferHandle.requireSameSlots(x, 1L, 2L, 3L) }\n"
        "    fun d() { NrBufferHandle.requireLenMatches(x, 4L) } }\n"
    )

    case = Report()
    check_buffer_release(
        buffer_declarations,
        {
            CARRIER_SUFFIX: good_carrier,
            "com/novelreader/android/core/NrBuffers.kt": good_buffers,
            "com/novelreader/android/core/NrEngine.kt": good_engine_buffer,
            "com/novelreader/android/core/NrDtos.kt": "val x = 1",
        },
        case,
    )
    _expect(
        report,
        "归还路径 fixture 通过（正向对照）",
        not case.failures,
        case.failures,
    )

    case = Report()
    check_buffer_release(
        buffer_declarations,
        {
            CARRIER_SUFFIX: good_carrier,
            "com/novelreader/android/core/NrBuffers.kt": (
                "class NrBufferHandle {\n"
                "    fun release(releaser: NrReleaser = NrReleaser.JNI): Int {\n"
                "        return releaser.release(ptr, len, capacity)\n    }\n"
                "    fun passthrough() { NrCore.nr_release_buffer(ptr, len, capacity) } }\n"
            ),
            "com/novelreader/android/core/NrEngine.kt": good_engine_buffer,
        },
        case,
    )
    _expect(
        report,
        "缺少归还保护与一致性断言被报出",
        any("归还函数必须检查" in item for item in case.failures)
        and any("一致性断言" in item for item in case.failures),
        case.failures,
    )

    case = Report()
    check_cancel({CARRIER_SUFFIX: "external fun nr_cancel_request(engine: Long, requestId: Long): Int"}, case)
    _expect(report, "缺少 cancelRequest 实现被报出", any("cancelRequest" in item for item in case.failures), case.failures)

    case = Report()
    check_kotlin_scope(
        {"com/novelreader/android/core/A.kt": "import androidx.compose.runtime.Composable\n// experiments/ 探针"},
        case,
    )
    _expect(
        report,
        "平台状态被报出（注释里的探针引用不算违规）",
        any("androidx.compose" in item for item in case.failures),
        case.failures,
    )
    _expect(
        report,
        "注释中的 experiments/ 引用不误报",
        not any("experiments/" in item for item in case.failures),
        case.failures,
    )
    return report


def _expect(report: Report, label: str, condition: bool, detail: object) -> None:
    if condition:
        report.ok(f"自测：{label}")
    else:
        report.fail(f"自测（{label}）未按预期报出：{detail}")


# ------------------------------------------------------------------ main


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="校验 Kotlin 薄封装与 C ABI / JNI 导出的一致性")
    parser.add_argument("--header", default=str(REPO_ROOT / "include" / "novel_reader.h"))
    parser.add_argument("--kotlin", default=str(REPO_ROOT / "android" / "app" / "src" / "main" / "java"))
    parser.add_argument("--jni", default=str(REPO_ROOT / "android" / "jni" / "nr_jni.c"))
    parser.add_argument(
        "--so",
        default=str(REPO_ROOT / "android" / "app" / "src" / "main" / "jniLibs" / "arm64-v8a" / "libnr_jni.so"),
    )
    parser.add_argument("--self-test", action="store_true", help="运行 fixture 反向用例")
    args = parser.parse_args(argv)

    print(f"verify-android-boundary: repo={REPO_ROOT}")
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
            print(f"SELF-TEST RESULT: PASS（{report.passes} 项断言，违规均可被报出）")

    header_path = Path(args.header)
    kotlin_dir = Path(args.kotlin)
    jni_path = Path(args.jni)
    if not header_path.is_file():
        print(f"RESULT: FAIL — 头文件不存在: {header_path}")
        return 2
    if not kotlin_dir.is_dir():
        print(f"RESULT: FAIL — Kotlin 目录不存在: {kotlin_dir}")
        return 2
    if not jni_path.is_file():
        print(f"RESULT: FAIL — JNI 适配层不存在: {jni_path}")
        return 2

    header_text = header_path.read_text(encoding="utf-8")
    c_text = jni_path.read_text(encoding="utf-8")
    files = parse_kotlin_files(kotlin_dir)
    if not files:
        print(f"RESULT: FAIL — Kotlin 目录内没有任何 .kt 文件: {kotlin_dir}")
        return 2

    header_entries = parse_header_entries(header_text)
    if not header_entries:
        print(f"RESULT: FAIL — 头文件未解析出任何 nr_* 声明: {header_path}")
        return 2

    report = Report()
    declarations = parse_kotlin_externals(files)
    check_kotlin_declarations(header_entries, declarations, report)
    check_jni_escape(header_entries, c_text, report)
    check_exported_symbols(header_entries, Path(args.so), report)
    check_error_codes(parse_header_macros(header_text), parse_kotlin_consts(files), files, report)
    check_handle_semantics(files, report)
    check_buffer_release(declarations, files, report)
    check_cancel(files, report)
    check_reader_cancel_reachability(files, report)
    check_kotlin_scope(files, report)

    print("-" * 60)
    if report.failures:
        print(f"RESULT: FAIL ({len(report.failures)} 项违规)")
        for item in report.failures:
            print(f"  - {item}")
        return 1
    if exit_code != 0:
        print("RESULT: FAIL — --self-test 未通过（违规检测能力未得到证明）")
        return 1
    print(
        f"RESULT: PASS — Kotlin 声明 / JNI 转义 / 错误码 / 句柄 / 缓冲归还 / 取消 / 取消可达性 "
        f"与冻结契约一致；断言 {report.passes} 项，跳过项 {len(report.skips)} 个（见上方 SKIP 行）"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())

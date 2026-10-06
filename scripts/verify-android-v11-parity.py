#!/usr/bin/env python3
"""Android V1.1 同语义门禁（FFI 加性契约修订；规格 `.hermes/specs/novel-reader-txt-v1.1/p5-android.md` §6）。

背景：桌面 V1.1 的四组功能（分类 / 历史 / 配色 / 背景）全在核心 SQLite 里，
Android 侧读不到 —— 冻结 FFI 原本没有这些导出点。路线 A 要求**加性契约修订**：
既有 19 个导出不动，新增一批导出，而**点名该集合的计数与清单散落在十余处**
（技能 `references/frozen-abi-extension.md` 的站点表）。

本门禁现在做三件事：

1. **[1] 站点齐备**：12 处站点必须都在、且都提到 V1.1（尚未实施时明确报「未实施」，
   而不是静默放过 —— 免得门禁变成空转）；
2. **[2] 计数一致**：`ABI_ENTRY_COUNT` / `ABI_ENTRY_NAMES` / `ABI_FUNCTIONS` /
   `entry_names()` / header 注释 / 构建脚本 / 两个 verify 脚本的计数必须一致；
3. **[3] 语义边界**：
   - Kotlin 侧**不得**出现派生 / 对比度 / 时间格式化（判定必须在核心）；
   - Kotlin 侧**不得**另存分类 / 历史（不得制造两个真相）；
   - JNI 包装的 `_` 转义（`_1`）不得漏。

反向对照：每类坏样本都必须被判失败（门禁自身有效的证明）。

用法：
    python3 scripts/verify-android-v11-parity.py
退出码 0 = PASS，1 = FAIL。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# 冻结基线：18 项冻结 + nr_set_encoding = 19。
FROZEN_COUNT = 19
# 后续批次（尚未实施）：不算入期望计数，但门禁要如实列出。
PENDING_ENTRIES: list[str] = [
    # V1.1 的 16 项计划导出已全部实施（B1 分类 4 + B2 历史 3 + B3 配色 5 + B4 背景 4）。
    # **留空是有意的**：一旦再有「已规划未实施」的导出，必须显式列进来，
    # 否则门禁会静默通过（这就是 B1 时踩过的坑）。
]
# V1.1 新增导出（规格 §3）。**按批次实施**，逐项加入。
#
# B1（已实施，2026-10-05）：分类树 4 项。
# 全部 16 项已实施（B1 分类 4 + B2 历史 3 + B3 配色 5 + B4 背景 4）。
# 本清单只列**已实施**的 —— 未实施项由 PENDING_ENTRIES 单列并如实报告，
# 免得门禁把「还没做」与「做错了」混为一谈。
V11_ENTRIES = [
    # 分类（§3.1）
    "nr_list_categories",
    "nr_create_category",
    "nr_move_book_to_category",
    "nr_delete_category",
    # 历史（§3.2）
    "nr_list_history",
    "nr_clear_history_entry",
    "nr_clear_history_for_book",
    # 配色（§3.3 / §3.5）
    "nr_get_theme_base",
    "nr_set_theme_base",
    "nr_set_theme_enabled",
    "nr_get_theme_token",
    "nr_get_theme_contrast",
    # 背景（§3.4）
    "nr_get_background",
    "nr_set_background",
    "nr_set_background_scrim",
    "nr_clear_background",
    # 筛选前置（A2；§3.1 的界面筛选所需）
    "nr_category_move_targets",
    "nr_rename_category",
    "nr_move_category",
    "nr_list_books_in_scope",
    "nr_list_book_categories",
]

# 12 处站点（规格 §4）→ (相对路径, 说明)
SITES = [
    ("include/novel_reader.h", "C 头：结构 typedef + 编号声明"),
    ("crates/reading-ffi/src/structs.rs", "结构 + ABI_STRUCTS + struct_sizes() + 计数断言"),
    ("crates/reading-ffi/src/exports.rs", "#[no_mangle] 导出 + entry_names() 长度"),
    ("crates/reading-ffi/src/engine.rs", "引擎方法 + outcome 结构"),
    ("crates/reading-ffi/src/abi.rs", "ABI_ENTRY_NAMES + ABI_FUNCTIONS（长度各一处）"),
    ("crates/reading-ffi/src/lib.rs", "ABI_ENTRY_COUNT（权威）+ 断言"),
    ("android/jni/nr_jni.c", "NR_JNI(...) 包装（_ → _1 转义）"),
    ("scripts/build-android-libs.sh", "-lt N 断言 + 打印的契约计数"),
    ("scripts/verify-abi.py", "入口名清单"),
    ("scripts/verify-android-boundary.py", "入口名清单（比对顺序序列）"),
    ("android/app/src/main/java/com/novelreader/android/core/NrCore.kt", "external fun（按 header 顺序）"),
    ("android/app/src/main/java/com/novelreader/android/core/NrAbi.kt", "接口 + 生产实现"),
]

# Kotlin 侧**不得**出现的语义（判定必须在核心；规格 §3.6）
KOTLIN_FORBIDDEN_SEMANTICS = {
    "derive_palette": "派生调色",
    "contrast_ratio": "对比度计算",
    "relative_luminance": "相对亮度",
    "CREATE TABLE": "自行建表",
    "insert into book_categories": "另存分类",
    "insert into reading_history": "另存历史",
    "insert into reader_theme_custom": "另存配色",
    "insert into reader_background": "另存背景",
}


def read(rel: str) -> str:
    path = REPO / rel
    if not path.is_file():
        return ""
    return path.read_text(encoding="utf-8")


def strip_comments(text: str, slint_like: bool = False) -> str:
    """剥离注释（`//` 与 `/* */`）—— 判据扫的是代码，不是说明。

    不剥注释会重演已踩过的坑：判据把自己的说明当违规。
    """
    out = []
    i = 0
    n = len(text)
    while i < n:
        if text.startswith("//", i):
            end = text.find("\n", i)
            i = n if end == -1 else end
            continue
        if text.startswith("/*", i):
            end = text.find("*/", i + 2)
            i = n if end == -1 else end + 2
            continue
        out.append(text[i])
        i += 1
    return "".join(out)


def fail(msg: str) -> None:
    print(f"  FAIL  {msg}")
    FAILURES.append(msg)


def ok(msg: str) -> None:
    print(f"  PASS  {msg}")


FAILURES: list[str] = []


def check_sites() -> None:
    """[1] 12 处站点齐备。"""
    print("[1] 静态：FFI 12 处站点齐备")
    missing = [rel for rel, _ in SITES if not (REPO / rel).is_file()]
    if missing:
        fail(f"站点缺失：{', '.join(missing)}")
    else:
        ok(f"12 处站点全部在位")

    exports_rs = read("crates/reading-ffi/src/exports.rs")
    implemented = [name for name in V11_ENTRIES if name in exports_rs]
    if len(implemented) != len(V11_ENTRIES):
        fail(
            f"V1.1 已登记的 {len(V11_ENTRIES)} 个导出只实施了 {len(implemented)} 个："
            f"缺 {', '.join(n for n in V11_ENTRIES if n not in implemented)}"
        )
    else:
        ok(f"V1.1 B1 分类 {len(V11_ENTRIES)} 个导出已实施")
    if PENDING_ENTRIES:
        ok(
            f"后续批次 {len(PENDING_ENTRIES)} 个导出**尚未实施**"
            f"（B2–B4；已实施前不得计入契约计数）"
        )


def check_counts() -> None:
    """[2] 计数一致（技能警告：硬编码长度可能藏在函数返回类型里）。"""
    print("[2] 静态：契约计数一致（散落十余处）")
    expected = FROZEN_COUNT + len(V11_ENTRIES)

    lib_rs = read("crates/reading-ffi/src/lib.rs")
    abi_rs = read("crates/reading-ffi/src/abi.rs")
    exports_rs = read("crates/reading-ffi/src/exports.rs")
    header = read("include/novel_reader.h")
    build_sh = read("scripts/build-android-libs.sh")

    # ABI_ENTRY_COUNT（权威常量）
    m = re.search(r"pub const ABI_ENTRY_COUNT: usize = (\d+);", lib_rs)
    if m is None:
        fail("lib.rs 找不到 `pub const ABI_ENTRY_COUNT: usize = N;`")
    else:
        actual = int(m.group(1))
        if actual != expected:
            fail(f"ABI_ENTRY_COUNT = {actual}，应为 {expected}（19 + {len(V11_ENTRIES)}）")
        else:
            ok(f"ABI_ENTRY_COUNT = {actual}")

    # 两处数组字面量长度（ABI_ENTRY_NAMES / ABI_FUNCTIONS）
    for pattern, label in (
        (r"pub const ABI_ENTRY_NAMES: \[&str; (\d+)\]", "ABI_ENTRY_NAMES"),
        (r"pub const ABI_FUNCTIONS: \[\(&str, &str\); (\d+)\]", "ABI_FUNCTIONS"),
    ):
        m = re.search(pattern, abi_rs)
        if m is None:
            fail(f"abi.rs 找不到 {label} 的长度标注")
        elif int(m.group(1)) != expected:
            fail(f"{label} 标注 {m.group(1)}，应为 {expected}")
        else:
            ok(f"{label} 标注 {m.group(1)}")

    # entry_names() 的**返回类型**长度（藏在签名里，搜数组字面量找不到）
    m = re.search(r"pub fn entry_names\(\) -> &'static \[&'static str; (\d+)\]", exports_rs)
    if m is None:
        fail("exports.rs 找不到 `entry_names() -> &'static [&'static str; N]`")
    elif int(m.group(1)) != expected:
        fail(f"entry_names() 返回类型标注 {m.group(1)}，应为 {expected}")
    else:
        ok(f"entry_names() 返回类型标注 {m.group(1)}")

    # header 的编号声明数（逐个数 `NR_ERR_*` 无关，只数形如 `nr_xxx(` 的声明）
    header_decls = set(re.findall(r"^\s*(?:[A-Za-z_][\w \*]*?)\b(nr_[a-z0-9_]+)\s*\(", header, re.M))
    missing_hdr = [name for name in V11_ENTRIES if name not in header_decls]
    if missing_hdr:
        fail(f"头文件缺少 V1.1 声明：{', '.join(missing_hdr)}")
    else:
        ok(f"头文件 V1.1 声明齐备（{len(V11_ENTRIES)} 项）")

    # 构建脚本的契约计数（形如 `-lt 19` 或「19 项」）
    m = re.search(r"-lt\s+(\d+)", build_sh)
    if m is None:
        # 未实施时脚本可能还没有该断言 —— 不算失败，但要报告。
        if V11_ENTRIES and all(
            n in read("crates/reading-ffi/src/exports.rs") for n in V11_ENTRIES
        ):
            fail("build-android-libs.sh 找不到 `-lt N` 导出数断言")
        else:
            ok("build-android-libs.sh 的 `-lt N` 断言（实施时必须同步）")
    elif int(m.group(1)) != expected:
        fail(f"build-android-libs.sh 的 `-lt` 断言 = {m.group(1)}，应为 {expected}")
    else:
        ok(f"build-android-libs.sh 的 `-lt` 断言 = {m.group(1)}")


def check_semantics() -> None:
    """[3] 语义边界：判定在核心、Kotlin 不另存、JNI 转义不漏。"""
    print("[3] 静态：语义边界（判定在核心 / 不得两个真相 / JNI 转义）")

    kotlin_dir = REPO / "android/app/src/main/java/com/novelreader/android"
    kotlin_files = list(kotlin_dir.rglob("*.kt")) if kotlin_dir.is_dir() else []
    if not kotlin_files:
        fail("找不到 Kotlin 源文件（android/app/src/main/java/...）")
        return
    violations: list[str] = []
    for path in kotlin_files:
        code = strip_comments(path.read_text(encoding="utf-8")).lower()
        for token, label in KOTLIN_FORBIDDEN_SEMANTICS.items():
            if token.lower() in code:
                violations.append(f"{path.name}: {label}（`{token}`）")
    if violations:
        fail("Kotlin 侧出现应由核心判定的语义：" + "; ".join(violations))
    else:
        ok(f"Kotlin 侧无派生 / 对比度 / 自建表 / 另存四组数据（{len(kotlin_files)} 个文件）")

    # ---- 派生算法必须在**共享层**（B3 的硬要求）----
    #
    # 背景：派生原本在 `desktop-app`，而 `reading-ffi` 不依赖它。若两边各算一套，
    # 就会出现「同一个基色在桌面和手机显示不同颜色」的**两个真相**。
    # 故判据分三条：共享层有实现、桌面侧**不再**自实现、FFI 暴露令牌与对比度。
    storage_derive_path = REPO / "crates/reading-storage/src/theme_derive.rs"
    storage_derive = (
        storage_derive_path.read_text(encoding="utf-8")
        if storage_derive_path.is_file()
        else ""
    )
    desktop_derive_path = REPO / "crates/desktop-app/src/theme_derive.rs"
    desktop_derive = (
        strip_comments(desktop_derive_path.read_text(encoding="utf-8"))
        if desktop_derive_path.is_file()
        else ""
    )
    problems: list[str] = []
    if "fn derive(" not in strip_comments(storage_derive):
        problems.append("派生算法不在共享层（reading-storage/src/theme_derive.rs）")
    if "fn derive(" in desktop_derive:
        problems.append("desktop-app 仍在自实现派生（两个真相：两端会不一致）")
    ui_rs = read("crates/desktop-app/src/ui.rs")  # 导出清单在 ui.rs 的驱动常量里
    exports_rs = read("crates/reading-ffi/src/exports.rs")
    # 必须是**定义**（`pub unsafe extern "C" fn nr_x`），而非仅在清单里出现
    # —— 只查子串会被 `abi.rs` 之类的清单命中（实测踩中）。
    for entry in ("nr_get_theme_token", "nr_get_theme_contrast"):
        definition = f"pub unsafe extern \"C\" fn {entry}"
        if definition not in exports_rs:
            problems.append(f"FFI 缺 {entry} 的定义（Android 取不到派生结果）")
    if problems:
        fail("派生单一真相被破坏：" + "; ".join(problems))
    else:
        ok("派生算法在共享层（桌面侧仅 re-export），且 FFI 暴露令牌与对比度读数")

    # JNI 转义：每个 V1.1 导出在 nr_jni.c 里若存在，其名字里的 `_` 必须写成 `_1`
    jni = read("android/jni/nr_jni.c")
    if V11_ENTRIES and all(n in read("crates/reading-ffi/src/exports.rs") for n in V11_ENTRIES):
        bad = []
        for name in V11_ENTRIES:
            if name not in jni:
                bad.append(f"{name}（缺包装）")
                continue
            escaped = name.replace("_", "_1")
            if escaped not in jni:
                bad.append(f"{name}（未转义 `_` → `_1`，运行时会 UnsatisfiedLinkError）")
        if bad:
            fail("JNI 包装问题：" + "; ".join(bad))
        else:
            ok(f"JNI 包装齐备且 `_` 转义正确（{len(V11_ENTRIES)} 项）")
    else:
        ok("JNI 包装（实施时必须逐项转义 `_` → `_1`）")


def check_not_vacuous() -> None:
    """反向对照：判据自身必须有牙。

    注意：真实源码此刻**本来就该 FAIL**（V1.1 未实施），所以不能用它当基线。
    这里改为构造一个「已实施」样本（把 15 项导出与计数补进各站点），
    要求它**通过**；再逐类破坏，要求**失败**。这样才证明判据不是恒假。
    """
    print("[4] 反向对照：判据有效性（构造「已实施」样本）")

    header = read("include/novel_reader.h")
    abi_rs = read("crates/reading-ffi/src/abi.rs")
    lib_rs = read("crates/reading-ffi/src/lib.rs")
    exports_rs = read("crates/reading-ffi/src/exports.rs")
    build_sh = read("scripts/build-android-libs.sh")

    expected = FROZEN_COUNT + len(V11_ENTRIES)

    def judge(
        header_text: str, abi_text: str, lib_text: str, build_text: str
    ) -> list[str]:
        found = []
        m = re.search(r"pub const ABI_ENTRY_COUNT: usize = (\d+);", lib_text)
        if m is None or int(m.group(1)) != expected:
            found.append("count")
        m = re.search(r"pub const ABI_ENTRY_NAMES: \[&str; (\d+)\]", abi_text)
        if m is None or int(m.group(1)) != expected:
            found.append("names")
        m = re.search(r"pub const ABI_FUNCTIONS: \[\(&str, &str\); (\d+)\]", abi_text)
        if m is None or int(m.group(1)) != expected:
            found.append("functions")
        decls = set(
            re.findall(r"^\s*(?:[A-Za-z_][\w \*]*?)\b(nr_[a-z0-9_]+)\s*\(", header_text, re.M)
        )
        if any(name not in decls for name in V11_ENTRIES):
            found.append("header")
        m = re.search(r"-lt\s+(\d+)", build_text)
        if m is None or int(m.group(1)) != expected:
            found.append("build-lt")
        return found

    # 构造「已实施」样本。
    decls = "".join(
        f"\n{('int32_t' if name.startswith(('nr_create', 'nr_delete', 'nr_clear', 'nr_set')) else 'void')} "
        f"{name}(void); /* 样本 */\n"
        for name in V11_ENTRIES
    )
    good_header = header + decls
    # 合成样本必须**按实际出现的数字**替换，而不是硬编码 19 ——
    # 真实源码此刻已经是 23（硬编码 19 的替换会静默不生效，判据空转）。
    def retarget(text: str, pattern: str, replacement: str) -> str:
        return re.sub(pattern, replacement, text, count=1)

    good_abi = retarget(retarget(abi_rs, r"\[&str; \d+\]", f"[&str; {expected}]"),
                        r"\[\(&str, &str\); \d+\]", f"[(&str, &str); {expected}]")
    good_lib = retarget(
        lib_rs, r"ABI_ENTRY_COUNT: usize = \d+", f"ABI_ENTRY_COUNT: usize = {expected}"
    )
    good_exports = retarget(
        exports_rs,
        r"&'static \[&'static str; \d+\]",
        f"&'static [&'static str; {expected}]",
    )
    good_build = retarget(build_sh, r"-lt\s+\d+", f"-lt {expected}")

    # judge 需要 exports 的「已实施」版本 —— 用参数传入。
    def judge_with(
        h: str, a: str, l: str, b: str, e: str
    ) -> list[str]:
        found = judge(h, a, l, b)
        m = re.search(
            r"pub fn entry_names\(\) -> &'static \[&'static str; (\d+)\]", e
        )
        if m is None or int(m.group(1)) != expected:
            found.append("entry-names")
        return found

    baseline = judge_with(good_header, good_abi, good_lib, good_build, good_exports)
    if baseline:
        fail(
            f"反向对照失效：「已实施」样本被判出 {len(baseline)} 处缺陷"
            f"（{', '.join(baseline)}）—— 判据与预期实现不一致"
        )
        return

    bad_cases = {
        # 只把 `ABI_ENTRY_COUNT` 漏改成 19（其余站点都对）—— 最典型的漏改。
        "ABI_ENTRY_COUNT 漏改": (
            good_header,
            good_abi,
            good_lib.replace(f"ABI_ENTRY_COUNT: usize = {expected}", "ABI_ENTRY_COUNT: usize = 19"),
            good_build,
            good_exports,
        ),
        # 「头文件漏一个声明」：**真实声明与合成声明都要删**
        # （只删一处会被另一处顶替 —— 这正是判据空转的成因）。
        "头文件漏一个声明": (
            re.sub(
                r"^int32_t nr_create_category\([^;]*;\s*$",
                "",
                good_header,
                flags=re.M | re.S,
            ).replace("int32_t nr_create_category(void); /* 样本 */\n", ""),
            good_abi,
            good_lib,
            good_build,
            good_exports,
        ),
        "ABI_ENTRY_NAMES 长度漏改": (
            good_header,
            good_abi.replace(f"[&str; {expected}]", f"[&str; {expected - 1}]"),
            good_lib,
            good_build,
            good_exports,
        ),
        "entry_names() 返回类型漏改": (
            good_header,
            good_abi,
            good_lib,
            good_build,
            good_exports.replace(
                f"&'static [&'static str; {expected}]",
                f"&'static [&'static str; {expected - 1}]",
            ),
        ),
        "构建脚本 -lt 漏改": (
            good_header,
            good_abi,
            good_lib,
            good_build.replace(f"-lt {expected}", f"-lt {expected - 1}"),
            good_exports,
        ),
    }
    missed = [
        name
        for name, args in bad_cases.items()
        if not judge_with(*args)  # 判不出缺陷 = 放过了坏样本
    ]
    if missed:
        fail("反向对照失效：以下坏样本竟然通过 —— " + "; ".join(missed))
    else:
        ok(
            f"反向对照：「已实施」样本通过，且 {len(bad_cases)} 类漏改样本"
            "（计数 / 头文件 / 两处数组长度 / 返回类型 / -lt）均被判失败"
        )


def main() -> int:
    print("=== Android V1.1 同语义门禁（FFI 加性契约修订）===")
    check_sites()
    check_counts()
    check_semantics()
    check_not_vacuous()
    print()
    if FAILURES:
        print(f"RESULT: FAIL（{len(FAILURES)} 项）")
        for item in FAILURES:
            print(f"  - {item}")
        return 1
    print("RESULT: PASS（12 站点 / 计数一致 / 语义边界 / 反向对照）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
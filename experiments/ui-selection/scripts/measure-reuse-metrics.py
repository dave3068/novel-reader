#!/usr/bin/env python3
"""measure-reuse-metrics.py —— H-R5 复用/平台专属/重复 UI 指标的**实测复算**

用法:
    python3 scripts/measure-reuse-metrics.py
    python3 scripts/measure-reuse-metrics.py --write-results   # 同时回写分页结果里的 metrics

指标口径（全部为真实统计，不是常量）:
  * shared_core_loc      : shared/rust-reading-core/src 的代码行（UI 无关、可复用）
  * shared_ffi_loc       : 头文件行数（跨语言契约）
  * platform_specific_loc: 平台专属代码行 = 桌面 UI(.slint) + 桌面 Rust 胶水
                           + Android Kotlin(main) + JNI C。不含 shared/
  * duplicated_ui_loc    : UI 层中「实现」了共享业务职责的代码行。
                           判定 = 在允许文件之外出现与共享入口/共享类型同名的
                           函数或类型定义（与 verify-hybrid-native-contract.py 同源规则）。
  * rust_business_coverage: business-responsibilities.json 中每个职责的全部
                           rust_symbols 都能在共享 Rust 源码里找到 -> 记 1 分，
                           coverage = 命中职责数 / 职责总数 * 100。

退出码: 0 统计成功
产出: results/raw/reuse-metrics.log, results/normalized/metrics-hybrid-native.json
"""

from __future__ import annotations

import argparse
import datetime
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REPO = ROOT.parent.parent
SHARED_SRC = ROOT / "shared" / "rust-reading-core" / "src"
HEADER = ROOT / "shared" / "ffi" / "include" / "novel_reader_probe.h"
CONTRACTS = ROOT / "shared" / "contracts" / "business-responsibilities.json"
DESKTOP = ROOT / "hybrid-native-probe" / "desktop-slint"
ANDROID = ROOT / "hybrid-native-probe" / "android-native"
RAW = ROOT / "results" / "raw"
NORM = ROOT / "results" / "normalized"

COMMENT_PREFIXES = ("//", "/*", "*", "*/", "#")


def android_verified_flag() -> bool:
    """Android 能力是否已经人工真机验收（记录缺失即视为未验证）。"""
    path = NORM / "android-device-verification.json"
    if not path.exists():
        return False
    try:
        return bool(json.loads(path.read_text(encoding="utf-8")).get("verified"))
    except (json.JSONDecodeError, OSError):
        return False


def code_lines(path: Path) -> tuple[int, int]:
    """返回 (总行数, 代码行数)。代码行 = 非空且非纯注释行。"""
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return (0, 0)
    total = 0
    code = 0
    for line in text.splitlines():
        total += 1
        stripped = line.strip()
        if not stripped:
            continue
        if stripped.startswith(COMMENT_PREFIXES):
            continue
        code += 1
    return (total, code)


def sum_loc(paths) -> tuple[int, int, int]:
    files = 0
    total = 0
    code = 0
    for p in paths:
        t, c = code_lines(p)
        if t == 0:
            continue
        files += 1
        total += t
        code += c
    return (files, total, code)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--write-results", action="store_true",
                        help="把实测指标回写到 results/normalized/pagination-hybrid-native-*.json")
    args = parser.parse_args()

    contract = json.loads(CONTRACTS.read_text(encoding="utf-8"))
    duties = contract["shared_duties"]
    boundary = contract["kotlin_boundary"]
    bridge_files = {str((ANDROID / p).resolve()) for p in boundary.get("jni_bridge_files", [])}

    # ---- 1. 代码量 ----
    shared_files, shared_total, shared_code = sum_loc(sorted(SHARED_SRC.rglob("*.rs")))
    _, header_total, header_code = sum_loc([HEADER])
    desktop_ui = sorted(DESKTOP.glob("ui/*.slint"))
    desktop_rs = sorted((DESKTOP / "src").rglob("*.rs"))
    android_kt = sorted((ANDROID / "app/src/main").rglob("*.kt"))
    android_c = sorted((ANDROID / "jni").rglob("*.c")) + sorted((ANDROID / "jni").rglob("*.cpp"))
    _, ui_total, ui_code = sum_loc(desktop_ui)
    _, drs_total, drs_code = sum_loc(desktop_rs)
    _, akt_total, akt_code = sum_loc(android_kt)
    _, ac_total, ac_code = sum_loc(android_c)
    platform_specific_loc = ui_code + drs_code + akt_code + ac_code

    # ---- 2. Rust 业务覆盖率 ----
    rust_text = "\n".join(p.read_text(encoding="utf-8", errors="replace")
                          for p in sorted(SHARED_SRC.rglob("*.rs")))
    hit_duties, missing = [], []
    for duty in duties:
        symbols = duty.get("rust_symbols", [])
        found = all(re.search(r"\b" + re.escape(s.split("::")[-1]) + r"\b", rust_text)
                    for s in symbols)
        (hit_duties if found else missing).append(duty["id"])
    coverage = (len(hit_duties) / len(duties) * 100.0) if duties else 0.0

    # ---- 3. 重复 UI 业务代码（与契约检查同源规则） ----
    shared_entry_names = "(readWindow|openBook|layoutPage|savePosition|commitPageBoundary|invalidateLayout|cancelRequest|getBookBatch|getChapterBatch)"
    dup_patterns = [
        re.compile(r"fun\s+" + shared_entry_names + r"\s*\("),
        re.compile(r"(class|object)\s+\w*PageBoundary\w*\b"),
        re.compile(r"(generateLibrary|generateBook|generateFixtures|makeAnchor|locateAnchor)\s*\("),
    ]
    duplicated = []
    ui_layer = list(desktop_ui) + list(desktop_rs) + list(android_kt)
    for path in ui_layer:
        if str(path.resolve()) in bridge_files:
            continue
        for lineno, line in enumerate(
                path.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
            if any(rx.search(line) for rx in dup_patterns):
                duplicated.append(f"{path.relative_to(REPO)}:{lineno}: {line.strip()[:100]}")
    duplicated_ui_loc = len(duplicated)

    metrics = {
        "measured_at": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "method": "code_lines excl. blank/comment-only; coverage from business-responsibilities.json",
        "shared_core_loc": shared_code,
        "shared_core_files": shared_files,
        "shared_core_total_lines": shared_total,
        "shared_ffi_header_loc": header_code,
        "desktop_ui_loc": ui_code,
        "desktop_rust_glue_loc": drs_code,
        "android_kotlin_loc": akt_code,
        "android_jni_c_loc": ac_code,
        "platform_specific_loc": platform_specific_loc,
        "duplicated_ui_loc": duplicated_ui_loc,
        "rust_business_coverage": round(coverage, 2),
        "duties_total": len(duties),
        "duties_covered": len(hit_duties),
        "duties_missing": missing,
        "duplicated_ui_samples": duplicated[:10],
        "reuse_ratio_shared_vs_platform": round(shared_code / (shared_code + platform_specific_loc) * 100, 2)
        if (shared_code + platform_specific_loc) else 0.0,
    }

    RAW.mkdir(parents=True, exist_ok=True)
    NORM.mkdir(parents=True, exist_ok=True)
    log = [
        "# H-R5 复用 / 平台专属 / 重复 UI 指标实测",
        f"# measured_at: {metrics['measured_at']}",
        f"# 口径: 代码行 = 非空且非纯注释行",
        "",
        f"shared_core_loc          = {shared_code:>6d}   ({shared_files} 个 .rs，共 {shared_total} 行)",
        f"shared_ffi_header_loc    = {header_code:>6d}",
        f"desktop_ui_loc (.slint)  = {ui_code:>6d}",
        f"desktop_rust_glue_loc    = {drs_code:>6d}",
        f"android_kotlin_loc       = {akt_code:>6d}",
        f"android_jni_c_loc        = {ac_code:>6d}",
        f"platform_specific_loc    = {platform_specific_loc:>6d}   (platform_specific 合计)",
        f"duplicated_ui_loc        = {duplicated_ui_loc:>6d}",
        f"rust_business_coverage   = {coverage:>6.2f}%  ({len(hit_duties)}/{len(duties)} 职责在共享库中全部命中)",
        f"reuse_ratio (shared 占比) = {metrics['reuse_ratio_shared_vs_platform']:.2f}%",
    ]
    if missing:
        log.append(f"未覆盖职责: {', '.join(missing)}")
    if duplicated:
        log.append("重复 UI 业务代码明细:")
        log.extend("  " + s for s in duplicated[:10])
    (RAW / "reuse-metrics.log").write_text("\n".join(log) + "\n", encoding="utf-8")
    (NORM / "metrics-hybrid-native.json").write_text(
        json.dumps(metrics, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    if args.write_results:
        patched = []
        # 覆盖全部 hybrid-native 结果记录（build / pagination / smoke / ui-contract 等）
        targets = [
            q for q in sorted(NORM.glob("*.json"))
            if q.name not in ("metrics-hybrid-native.json",)
            and json.loads(q.read_text(encoding="utf-8")).get("candidate") == "hybrid-native"
        ]
        for path in targets:
            data = json.loads(path.read_text(encoding="utf-8"))
            data.setdefault("metrics", {})
            data["metrics"].update({
                "shared_core_loc": shared_code,
                "rust_business_coverage": round(coverage, 2),
                "platform_specific_loc": platform_specific_loc,
                "duplicated_ui_loc": duplicated_ui_loc,
                "android_verified": android_verified_flag(),
                "android_guide": "docs/ui-selection/android.md",
                "kotlin_business_duplication": bool(duplicated),
                "measured_at": metrics["measured_at"],
            })
            path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
            patched.append(path.name)
        log.append(f"# 已回写 metrics: {', '.join(patched)}")
        (RAW / "reuse-metrics.log").write_text("\n".join(log) + "\n", encoding="utf-8")

    print("\n".join(log))
    return 0


if __name__ == "__main__":
    sys.exit(main())

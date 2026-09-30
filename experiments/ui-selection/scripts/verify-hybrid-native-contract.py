#!/usr/bin/env python3
"""verify-hybrid-native-contract.py —— 双 UI 业务边界与薄 JNI 契约检查（H-C2 / H-C3 静态部分）

用法:
    python3 scripts/verify-hybrid-native-contract.py \
        [--check-android-layout] [--check-jni] [--check-no-business-duplication]

不给参数时三项全做。

退出码:
    0  全部检查通过
    1  存在 FAIL 项
    2  契约文件缺失

产出:
    results/raw/verify-hybrid-native-contract.log
    results/normalized/contract-hybrid-native.json

检查来源：shared/contracts/business-responsibilities.json（kotlin_boundary 段）与 abi.json。
"""

from __future__ import annotations

import argparse
import fnmatch
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent  # experiments/ui-selection
CONTRACTS = ROOT / "shared" / "contracts"
ANDROID = ROOT / "hybrid-native-probe" / "android-native"
HEADER = ROOT / "shared" / "ffi" / "include" / "novel_reader_probe.h"
RAW = ROOT / "results" / "raw"
NORM = ROOT / "results" / "normalized"

ANDROID_REL_PREFIX = "hybrid-native-probe/android-native/"


def log(lines: list[str], text: str) -> None:
    print(text)
    lines.append(text)


def iter_android_sources() -> list[Path]:
    if not ANDROID.exists():
        return []
    out: list[Path] = []
    for pattern in ("**/*.kt", "**/*.java", "**/*.c", "**/*.cpp", "**/*.h"):
        out.extend(p for p in ANDROID.glob(pattern) if p.is_file())
    return sorted(set(out))


def rel(path: Path) -> str:
    try:
        return str(path.relative_to(ROOT.parent.parent))
    except ValueError:
        return str(path)


def scan(rule_id: str, pattern: str, files: list[Path], reason: str) -> list[str]:
    rx = re.compile(pattern)
    hits: list[str] = []
    for path in files:
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        for lineno, line in enumerate(text.splitlines(), 1):
            if rx.search(line):
                hits.append(f"{rel(path)}:{lineno}: {line.strip()[:120]}")
    return hits


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check-android-layout", action="store_true")
    parser.add_argument("--check-jni", action="store_true")
    parser.add_argument("--check-no-business-duplication", action="store_true")
    args = parser.parse_args()
    do_all = not (
        args.check_android_layout or args.check_jni or args.check_no_business_duplication
    )
    check_layout = do_all or args.check_android_layout
    check_jni = do_all or args.check_jni
    check_dup = do_all or args.check_no_business_duplication

    lines: list[str] = []
    failures: list[str] = []
    passes: list[str] = []

    contract_path = CONTRACTS / "business-responsibilities.json"
    abi_path = CONTRACTS / "abi.json"
    if not contract_path.exists() or not abi_path.exists():
        print(f"[FAIL] 契约文件缺失: {contract_path} / {abi_path}", file=sys.stderr)
        return 2

    contract = json.loads(contract_path.read_text(encoding="utf-8"))
    abi = json.loads(abi_path.read_text(encoding="utf-8"))
    boundary = contract["kotlin_boundary"]

    log(lines, f"# verify-hybrid-native-contract.py  android-native={ANDROID}")
    log(lines, f"# 契约: {contract_path.name}  version={contract.get('version')}")

    sources = iter_android_sources()
    log(lines, f"# 扫描android源码文件数: {len(sources)}")
    if not sources:
        log(lines, "[FAIL] android-native 目录下没有任何源码文件")
        failures.append("android-native 源码缺失")

    bridge_files = set(boundary.get("jni_bridge_files", []))
    bridge_paths = {str((ANDROID / p).resolve()) for p in bridge_files}

    def is_bridge(path: Path) -> bool:
        return str(path.resolve()) in bridge_paths

    # ---- 1. 禁止业务复制（任何文件） ----
    if check_dup:
        log(lines, "## check-no-business-duplication")
        for rule in boundary.get("forbidden_anywhere", []):
            hits = scan(rule["id"], rule["pattern"], sources, rule.get("reason", ""))
            if hits:
                failures.append(f"forbidden_anywhere:{rule['id']}")
                log(lines, f"[FAIL] {rule['id']} 命中 {len(hits)} 处 — {rule.get('reason','')}")
                for hit in hits[:8]:
                    log(lines, f"        {hit}")
            else:
                passes.append(f"forbidden_anywhere:{rule['id']}")
                log(lines, f"[PASS] {rule['id']} 未命中（{rule.get('reason','')}）")

        for rule in boundary.get("forbidden_outside_bridge", []):
            outside = [p for p in sources if not is_bridge(p)]
            hits = scan(rule["id"], rule["pattern"], outside, rule.get("reason", ""))
            if hits:
                failures.append(f"forbidden_outside_bridge:{rule['id']}")
                log(lines, f"[FAIL] {rule['id']} 在桥接文件之外命中 {len(hits)} 处")
                for hit in hits[:8]:
                    log(lines, f"        {hit}")
            else:
                passes.append(f"forbidden_outside_bridge:{rule['id']}")
                log(lines, f"[PASS] {rule['id']} 桥接文件之外未命中")

        log(lines, f"kotlin_business_duplication={str(bool(failures)).lower()}")

    # ---- 2. required_evidence ----
    if check_layout or check_jni:
        log(lines, "## required-evidence")
        for item in boundary.get("required_evidence", []):
            glob = item["path_glob"]
            rx = re.compile(item["pattern"])
            matched_files = [
                p
                for p in sources
                if fnmatch.fnmatch(str(p.relative_to(ANDROID)), glob)
                or (glob.startswith("app/src/main/java/") and fnmatch.fnmatch(str(p.relative_to(ANDROID)), glob))
            ]
            hit = None
            for path in matched_files:
                text = path.read_text(encoding="utf-8", errors="replace")
                if rx.search(text):
                    hit = path
                    break
            if hit is None:
                failures.append(f"required_evidence:{item['id']}")
                log(
                    lines,
                    f"[FAIL] {item['id']} 未满足 — glob={glob} 匹配 {len(matched_files)} 个文件，"
                    f"无文件命中 {item['pattern']!r}（{item.get('reason','')}）",
                )
            else:
                passes.append(f"required_evidence:{item['id']}")
                log(lines, f"[PASS] {item['id']} 命中 {rel(hit)}（{item.get('reason','')}）")

    # ---- 3. ABI 基线符号 ----
    log(lines, "## abi-baseline")
    header_text = HEADER.read_text(encoding="utf-8") if HEADER.exists() else ""
    if not header_text:
        failures.append("header-missing")
        log(lines, f"[FAIL] 头文件缺失: {rel(HEADER)}")
    else:
        missing = [s for s in abi["entries"] for s in s["symbols"] if s not in header_text]
        if missing:
            failures.append("header-symbols")
            log(lines, f"[FAIL] 头文件缺少符号: {', '.join(missing)}")
        else:
            passes.append("header-symbols")
            log(
                lines,
                f"[PASS] 头文件包含全部 {abi['symbol_count']} 个基线符号"
                f"（{abi['entry_count']} 个入口）",
            )

    nr_core = ANDROID / "app/src/main/java/com/novelreader/probe/core/NrCore.kt"
    if nr_core.exists():
        core_text = nr_core.read_text(encoding="utf-8")
        # Kotlin 侧方法名为 nativeXxx，逐个映射到 C 符号
        mapping = {
            "nr_engine_create": "nativeCreate",
            "nr_engine_destroy": "nativeDestroy",
            "nr_open_book": "nativeOpenBook",
            "nr_read_window": "nativeReadWindow",
            "nr_get_book_batch": "nativeGetBookBatch",
            "nr_get_chapter_batch": "nativeGetChapterBatch",
            "nr_save_position": "nativeSavePosition",
            "nr_commit_page_boundary": "nativeCommitPageBoundary",
            "nr_invalidate_layout": "nativeInvalidateLayout",
            "nr_subscribe_events": "nativeSubscribeEvents",
            "nr_cancel_request": "nativeCancelRequest",
        }
        absent = [c for c in mapping.values() if c not in core_text]
        release_ok = ("nr_release_buffer" in core_text) or ("releaseBuffer" in core_text)
        if absent or not release_ok:
            failures.append("kotlin-bridge-declarations")
            detail = ", ".join(absent) if absent else ""
            log(
                lines,
                f"[FAIL] NrCore.kt 声明不完整; 缺失={detail or '无'} release_buffer={'ok' if release_ok else 'missing'}",
            )
        else:
            passes.append("kotlin-bridge-declarations")
            log(lines, "[PASS] NrCore.kt 声明了全部 11 个基线入口映射 + 缓冲释放")
    else:
        failures.append("nr-core-missing")
        log(lines, f"[FAIL] 缺少 JNI 桥接文件: {rel(nr_core)}")

    # ---- 4. JNI C 层 ----
    if check_jni:
        log(lines, "## check-jni")
        jni_c = ANDROID / "jni/nr_jni.c"
        if not jni_c.exists():
            failures.append("jni-c-missing")
            log(lines, f"[FAIL] 缺少 JNI C 适配层: {rel(jni_c)}")
        else:
            text = jni_c.read_text(encoding="utf-8", errors="replace")
            if "JNIEXPORT" not in text:
                failures.append("jni-no-export")
                log(lines, "[FAIL] nr_jni.c 未使用 JNIEXPORT")
            else:
                passes.append("jni-export")
                log(lines, f"[PASS] nr_jni.c 使用 JNIEXPORT（{text.count('JNIEXPORT')} 处）")
            calls = [
                sym
                for sym in ("nr_engine_create", "nr_read_window", "nr_commit_page_boundary",
                            "nr_release_buffer", "nr_cancel_request")
                if sym in text
            ]
            if len(calls) < 3:
                failures.append("jni-not-calling-c-abi")
                log(lines, f"[FAIL] nr_jni.c 只调用 {len(calls)} 个 C ABI 符号，疑未复用共享库")
            else:
                passes.append("jni-calls-c-abi")
                log(lines, f"[PASS] nr_jni.c 调用共享 C ABI: {', '.join(calls)}")

    # ---- 5. 输出 ----
    overall = "通过" if not failures else "不通过"
    log(lines, f"## 结果 status={overall} pass={len(passes)} fail={len(failures)}")
    for item in failures:
        log(lines, f"  FAIL {item}")

    RAW.mkdir(parents=True, exist_ok=True)
    NORM.mkdir(parents=True, exist_ok=True)
    (RAW / "verify-hybrid-native-contract.log").write_text("\n".join(lines) + "\n", encoding="utf-8")

    result = {
        "candidate": "hybrid-native",
        "platform": "android",
        "phase": "diagnostic",
        "status": overall,
        "framework": "compose+Slint",
        "command": "python3 scripts/verify-hybrid-native-contract.py"
        + (" --check-android-layout" if args.check_android_layout else "")
        + (" --check-jni" if args.check_jni else "")
        + (" --check-no-business-duplication" if args.check_no_business_duplication else ""),
        "exit_code": 0 if not failures else 1,
        "timestamp": __import__("datetime").datetime.utcnow().strftime("%Y-%m-%dT%H:%M:%SZ"),
        "log_path": "results/raw/verify-hybrid-native-contract.log",
        "environment": {"os": "Deepin 20.9", "glibc": "2.28", "backend": "n/a"},
        "details": {
            "passed": passes,
            "failed": failures,
            "android_source_files": len(sources),
            "android_verified": False,
            "android_guide": "docs/ui-selection/android.md",
        },
    }
    (NORM / "contract-hybrid-native.json").write_text(
        json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())

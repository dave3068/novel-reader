#!/usr/bin/env python3
"""verify-results.py —— 结果契约校验（H-C6 / S-R6）

用法:
    python3 scripts/verify-results.py --phase pagination --candidates hybrid-native [slint ...]

退出码:
    0  所有被请求候选的结果文件齐全、字段合法、hybrid-native 额外指标完整
    1  存在缺失/非法项
    2  参数错误

产出:
    results/raw/verify-results-<phase>.log

校验内容：
  * 结果记录必须含 result.schema.json 的 required 字段，status 只能为
    通过 / 有条件通过 / 不通过 / 未验证；
  * hybrid-native 必须有 metrics.rust_business_coverage / platform_specific_loc /
    duplicated_ui_loc / android_verified / android_guide；
  * hybrid-native 的 android_verified 为布尔值，且 android_guide 指向存在的文件；
    Android 分页状态必须与其一致（verified=true → 通过；否则 → 未验证）。
"""

from __future__ import annotations

import argparse
import datetime
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
NORM = ROOT / "results" / "normalized"
RAW = ROOT / "results" / "raw"
REPO = ROOT.parent.parent

REQUIRED_FIELDS = [
    "candidate",
    "platform",
    "phase",
    "status",
    "framework",
    "command",
    "exit_code",
    "timestamp",
    "environment",
]
VALID_STATUS = ("通过", "有条件通过", "不通过", "未验证")
PLATFORMS = {"hybrid-native": ["desktop", "android"]}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--phase", required=True)
    parser.add_argument("--candidates", nargs="+", required=True)
    args = parser.parse_args()

    lines: list[str] = []
    failures: list[str] = []
    summaries: dict[str, dict] = {}

    def emit(text: str) -> None:
        print(text)
        lines.append(text)

    emit(f"# verify-results.py phase={args.phase} candidates={','.join(args.candidates)}")
    emit(f"# normalized 目录: {NORM}")

    for candidate in args.candidates:
        platforms = PLATFORMS.get(candidate, ["desktop"])
        for platform in platforms:
            path = NORM / f"{args.phase}-{candidate}-{platform}.json"
            if not path.exists():
                failures.append(f"{candidate}/{platform}: 结果文件缺失 {path.name}")
                emit(f"[FAIL] {candidate}/{platform}: 缺少 {path.name}")
                continue

            try:
                data = json.loads(path.read_text(encoding="utf-8"))
            except json.JSONDecodeError as exc:
                failures.append(f"{candidate}/{platform}: JSON 解析失败 {exc}")
                emit(f"[FAIL] {candidate}/{platform}: JSON 解析失败 {exc}")
                continue

            missing = [f for f in REQUIRED_FIELDS if f not in data]
            if missing:
                failures.append(f"{candidate}/{platform}: 缺少字段 {missing}")
                emit(f"[FAIL] {candidate}/{platform}: 缺少必需字段 {missing}")
                continue
            if data["status"] not in VALID_STATUS:
                failures.append(f"{candidate}/{platform}: status 非法 {data['status']!r}")
                emit(f"[FAIL] {candidate}/{platform}: status 非法 {data['status']!r}")
                continue
            if data["candidate"] != candidate:
                failures.append(f"{candidate}/{platform}: candidate 字段不一致 {data['candidate']!r}")
                emit(f"[FAIL] {candidate}/{platform}: candidate 字段为 {data['candidate']!r}")
                continue

            env = data.get("environment") or {}
            for key in ("os", "glibc", "backend"):
                if not env.get(key):
                    failures.append(f"{candidate}/{platform}: environment.{key} 缺失")
                    emit(f"[FAIL] {candidate}/{platform}: environment.{key} 缺失")
                    break

            # hybrid-native 专属指标
            if candidate == "hybrid-native":
                metrics = data.get("metrics") or {}
                need = [
                    "rust_business_coverage",
                    "platform_specific_loc",
                    "duplicated_ui_loc",
                    "android_verified",
                    "android_guide",
                ]
                absent = [k for k in need if k not in metrics]
                if absent:
                    failures.append(f"{candidate}/{platform}: metrics 缺少 {absent}")
                    emit(f"[FAIL] {candidate}/{platform}: metrics 缺少 {absent}")
                    continue
                if not isinstance(metrics["android_verified"], bool):
                    failures.append(
                        f"{candidate}/{platform}: android_verified 必须是布尔值，实际 {metrics['android_verified']!r}"
                    )
                    emit(f"[FAIL] {candidate}/{platform}: android_verified 必须是布尔值")
                    continue
                guide = REPO / metrics["android_guide"]
                if not guide.exists():
                    failures.append(f"{candidate}/{platform}: 操作指南不存在 {metrics['android_guide']}")
                    emit(f"[FAIL] {candidate}/{platform}: 操作指南不存在 {metrics['android_guide']}")
                    continue
                if platform == "android" and args.phase == "pagination":
                    expected = "通过" if metrics["android_verified"] else "未验证"
                    if data["status"] != expected:
                        failures.append(
                            f"{candidate}/android: 分页能力状态必须与 android_verified="
                            f"{metrics['android_verified']} 一致（应为 {expected}），实际 {data['status']!r}"
                        )
                        emit(f"[FAIL] {candidate}/android: 状态应为 {expected}，实际 {data['status']!r}")
                        continue
                emit(
                    f"[PASS] {candidate}/{platform}: status={data['status']} "
                    f"coverage={metrics['rust_business_coverage']} "
                    f"platform_loc={metrics['platform_specific_loc']} "
                    f"dup_ui_loc={metrics['duplicated_ui_loc']} "
                    f"android_verified={metrics['android_verified']} guide={metrics['android_guide']}"
                )
            else:
                emit(f"[PASS] {candidate}/{platform}: status={data['status']}")

            summaries[f"{candidate}/{platform}"] = {
                "status": data["status"],
                "file": path.name,
            }

    overall = "通过" if not failures else "不通过"
    emit(f"## 结果 status={overall} pass={len(summaries)} fail={len(failures)}")
    for item in failures:
        emit(f"  FAIL {item}")

    RAW.mkdir(parents=True, exist_ok=True)
    (RAW / f"verify-results-{args.phase}.log").write_text(
        "\n".join(lines) + "\n", encoding="utf-8"
    )
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())

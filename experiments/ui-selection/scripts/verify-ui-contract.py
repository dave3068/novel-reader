#!/usr/bin/env python3
"""verify-ui-contract.py —— 统一 UI 语义契约检查（H-C4）

用法:
    python3 scripts/verify-ui-contract.py --candidate hybrid-native --platform desktop
    python3 scripts/verify-ui-contract.py --candidate hybrid-native --platform android --allow-unverified

退出码:
    0  契约完整（android 必须带 --allow-unverified，且状态记录为 未验证）
    1  契约不完整或分页门槛未通过
    2  参数错误

产出:
    results/normalized/ui-contract-<candidate>-<platform>.json
    results/raw/verify-ui-contract-<candidate>-<platform>.log
"""

from __future__ import annotations

import argparse
import datetime
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RAW = ROOT / "results" / "raw"
NORM = ROOT / "results" / "normalized"
SLINT_UI = ROOT / "hybrid-native-probe" / "desktop-slint" / "ui" / "app.slint"
ANDROID_UI = ROOT / "hybrid-native-probe" / "android-native"
REPO = ROOT.parent.parent

VALID_STATUS = ("通过", "有条件通过", "不通过", "未验证")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--platform", default="desktop", choices=["desktop", "android"])
    parser.add_argument("--allow-unverified", action="store_true")
    args = parser.parse_args()

    lines: list[str] = []
    failures: list[str] = []
    passes: list[str] = []

    if args.candidate != "hybrid-native":
        print(f"[FAIL] 候选 {args.candidate} 尚未实现统一 UI 契约", file=sys.stderr)
        return 1

    if args.platform == "desktop":
        if not SLINT_UI.exists():
            failures.append("slint-ui-missing")
            lines.append(f"[FAIL] 缺少 {SLINT_UI}")
        else:
            text = SLINT_UI.read_text(encoding="utf-8")
            pages = {"书架": "书架页", "阅读": "阅读页", "配置": "配置页"}
            for needle, label in pages.items():
                if needle in text:
                    passes.append(f"page:{label}")
                    lines.append(f"[PASS] 存在{label}（含「{needle}」）")
                else:
                    failures.append(f"page:{label}")
                    lines.append(f"[FAIL] 缺少{label}")
            switches = len(re.findall(r"if root\.current-page == [0-9]", text))
            if switches >= 3:
                passes.append("page-switching")
                lines.append(f"[PASS] 存在 {switches} 处页面切换条件")
            else:
                failures.append("page-switching")
                lines.append(f"[FAIL] 页面切换条件不足（{switches}），三页面不完整")

            entries = {
                "上一页": "翻页-上一页",
                "下一页": "翻页-下一页",
                "保存位置": "位置保存",
                "菜单": "阅读菜单",
                "搜索": "书架搜索",
                "字号": "字号配置",
                "行距": "行距配置",
                "页边距": "页边距配置",
                "背景模式": "背景配置",
                "日间/夜间": "模式切换",
                "分类目录": "6 层分类目录",
            }
            for needle, label in entries.items():
                if needle in text:
                    passes.append(f"entry:{label}")
                    lines.append(f"[PASS] 存在交互入口：{label}")
                else:
                    failures.append(f"entry:{label}")
                    lines.append(f"[FAIL] 缺少交互入口：{label}")

        # 分页门槛结果
        pagination = NORM / "pagination-hybrid-native-desktop.json"
        if pagination.exists():
            data = json.loads(pagination.read_text(encoding="utf-8"))
            if data.get("status") == "通过":
                passes.append("pagination-gate")
                lines.append("[PASS] 桌面分页门槛结果为 通过")
            else:
                failures.append("pagination-gate")
                lines.append(f"[FAIL] 桌面分页门槛结果为 {data.get('status')!r}")
        else:
            failures.append("pagination-evidence-missing")
            lines.append(f"[FAIL] 缺少分页结果 {pagination.relative_to(REPO)}")

        status = "通过" if not failures else "不通过"
        exit_code = 0 if not failures else 1

    else:  # android
        required = {
            "ui/BookshelfScreen.kt": ("@Composable", "书架屏 @Composable"),
            "ui/ReaderScreen.kt": ("@Composable", "阅读屏 @Composable"),
            "ui/SettingsScreen.kt": ("@Composable", "配置屏 @Composable"),
            "MainActivity.kt": (None, "Activity 与生命周期"),
        }
        found_any = ANDROID_UI.exists()
        if not found_any:
            failures.append("android-native-missing")
            lines.append(f"[FAIL] 缺少 {ANDROID_UI}")

        comoposables = 0
        if found_any:
            kt_files = list(ANDROID_UI.rglob("*.kt"))
            for path in kt_files:
                if "@Composable" in path.read_text(encoding="utf-8", errors="replace"):
                    comoposables += 1
            lines.append(f"# android Kotlin 文件 {len(kt_files)} 个，含 @Composable 的 {comoposables} 个")

        globs = {
            "SAF 文件访问": ("data/*.kt", r"(OpenDocument|OpenDocumentTree|ContentResolver)"),
            "系统返回/导航": ("ui/*.kt", r"(BackHandler|onBackPressed|navigateUp|popBackStack)"),
            "旋转恢复": ("MainActivity.kt", r"(rememberSaveable|onSaveInstanceState|ViewModel|configChanges)"),
            "生命周期/后台恢复": ("MainActivity.kt", r"(LifecycleEventObserver|ON_STOP|ON_START|lifecycle)"),
            "JNI 取消": ("core/*.kt", r"(cancelRequest|nr_cancel_request)"),
        }
        for label, (pattern_glob, rx) in globs.items():
            candidates = list(ANDROID_UI.rglob(pattern_glob)) if found_any else []
            hit = any(re.search(rx, p.read_text(encoding="utf-8", errors="replace")) for p in candidates)
            if hit:
                passes.append(f"android:{label}")
                lines.append(f"[PASS] Android {label} 入口存在")
            else:
                failures.append(f"android:{label}")
                lines.append(f"[FAIL] Android {label} 入口缺失")

        if not found_any or comoposables < 3 or failures:
            status = "不通过"
            exit_code = 1
        else:
            status = "未验证"
            exit_code = 0 if args.allow_unverified else 1
            if not args.allow_unverified:
                lines.append("[FAIL] Android 未实测，必须显式传 --allow-unverified 才允许以 未验证 通过")
                failures.append("android-unverified-needs-flag")

    RAW.mkdir(parents=True, exist_ok=True)
    NORM.mkdir(parents=True, exist_ok=True)
    log_path = RAW / f"verify-ui-contract-{args.candidate}-{args.platform}.log"
    log_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    for line in lines:
        print(line)

    result = {
        "candidate": args.candidate,
        "platform": args.platform,
        "phase": "ui-contract",
        "status": status if status in VALID_STATUS else "不通过",
        "framework": "slint" if args.platform == "desktop" else "compose",
        "framework_version": "1.17.1" if args.platform == "desktop" else "",
        "command": f"python3 scripts/verify-ui-contract.py --candidate {args.candidate} --platform {args.platform}"
        + (" --allow-unverified" if args.allow_unverified else ""),
        "exit_code": exit_code,
        "timestamp": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "log_path": str(log_path.relative_to(REPO)),
        "environment": {"os": "Deepin 20.9", "glibc": "2.28", "backend": "x11"},
        "details": {"passed": passes, "failed": failures},
        "notes": ["Android 当前未实测，状态为 未验证"] if args.platform == "android" else [],
    }
    (NORM / f"ui-contract-{args.candidate}-{args.platform}.json").write_text(
        json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    print(f"# 结果已写入 results/normalized/ui-contract-{args.candidate}-{args.platform}.json")
    return exit_code


if __name__ == "__main__":
    sys.exit(main())

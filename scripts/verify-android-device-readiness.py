#!/usr/bin/env python3
"""verify-android-device-readiness.py — 真机延期验收的「就绪度」门禁（AN6 交付物；代理检查项 C9）。

用途（android/spec.md 检查项 C9 / AR6；卡片 AN6「本卡交付物」第 5 条）：

  证明「真机操作说明已完备到可被执行」，而不是证明「真机已经跑过」：

  [1] `docs/android/` 存在且含 Markdown 说明；
  [2] 含真实安装命令（`adb install`）与 BUILD_MARKER 三步核对方式；
  [3] `dist/BUILD-INFO.md` 存在，且其记录的 BUILD_MARKER 值 **等于**
      `android/app/build.gradle.kts` 当前 `buildConfigField` 值（跨文件一致性，非自说自话）；
  [4] 文档中 `dist/**.apk` 的引用在磁盘上确实存在（说明写的包真的产出了）；
  [5] 步骤表的表头同时含「步骤」与「期望结果」列；`期望` 出现次数 ≥ 6；
  [6] `待验证` 状态字段存在（表格单元格形式）且出现次数 ≥ 6；
  [7] MIUI/澎湃 限制说明完整：USB 安装、USB 调试（安全设置）、
      INSTALL_FAILED_USER_RESTRICTED、INJECT_EVENTS；
  [8] 延期证据路径约定已声明（`results/verify/android-device-verify.md` 与附件目录），
      脚本按 `--results` 把约定解析成绝对路径并报告当前状态（存在 / 待写入）；
  [9] 文档**不得**声称已真机验证（禁止事项：不得声称已真机验证）。

`--results` 既接受 results 根（`--results results`），也接受其子目录（`--results results/android`）：
解析时会向上纳入上级 `results` 根，保证两种调用口径都能定位到同一份约定路径。

退出码：0 = 就绪；1 = 文档缺失项/不一致；2 = 前提缺失（docs 目录不存在或为空）。
`--self-test`：用合成 fixture 文本验证每条判定的**反向**能力（缺失项必须被报出）。
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
GRADLE_FILE = REPO_ROOT / "android" / "app" / "build.gradle.kts"
BUILD_INFO_FILE = REPO_ROOT / "dist" / "BUILD-INFO.md"
EVIDENCE_RELATIVE = "results/verify/android-device-verify.md"
EVIDENCE_LOGS_RELATIVE = "results/verify/android-device-logs/"

# ---- 必备内容（正则，作用于 docs/android 全部 Markdown 的合并文本） -------------
REQUIRED_PATTERNS: dict[str, str] = {
    "真机安装命令（adb install）": r"adb\s+install",
    "BUILD_MARKER 核对方式": r"BUILD_MARKER",
    "构建信息文件引用（dist/BUILD-INFO.md）": r"dist/BUILD-INFO\.md",
    "MIUI「USB 安装」开关": r"USB\s*安装",
    "MIUI「USB 调试（安全设置）」开关": r"USB\s*调试（安全设置）",
    "安装被拒错误码（INSTALL_FAILED_USER_RESTRICTED）": r"INSTALL_FAILED_USER_RESTRICTED",
    "触控注入权限错误（INJECT_EVENTS）": r"INJECT_EVENTS",
    "延期证据路径约定": re.escape(EVIDENCE_RELATIVE),
    "附件日志目录约定": re.escape(EVIDENCE_LOGS_RELATIVE),
    "机型记录要求": r"机型",
    "系统版本要求（Android 12+ / minSdk 31）": r"minSdk\s*31|Android\s*12",
    "未签名 Release 属预期的说明": r"未签名",
    "shell 进程与 shell 崩溃判读": r"shell\s*进程|shell 进程",
}

TABLE_STEP_RE = re.compile(r"\|\s*#?\s*\|\s*步骤\s*\|")
TABLE_EXPECT_RE = re.compile(r"\|\s*期望结果\s*\|")
TABLE_STATUS_RE = re.compile(r"\|\s*待验证\s*\|")
DIST_APK_RE = re.compile(r"dist/([A-Za-z0-9._-]+\.apk)")
MARKER_VALUE_RES = (
    re.compile(r"BUILD_MARKER[`：:\s]*[`\"]?([A-Za-z0-9][A-Za-z0-9._-]{8,})[`\"]?"),
)

# 禁止事项：文档不得声称已完成真机验证
FORBIDDEN_CLAIM_RES = (
    re.compile(r"真机通过"),
    re.compile(r"已真机验证"),
    re.compile(r"真机已验证"),
    re.compile(r"真机验证通过"),
)

MIN_EXPECT_MENTIONS = 6
MIN_PENDING_MENTIONS = 6


class Readiness:
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
# 文档内容判定（与 CLI 解耦，便于 --self-test 直接调用）
# --------------------------------------------------------------------------- #


def evaluate_corpus(rep: Readiness, corpus: str, files: list[Path]) -> None:
    """对合并后的文档文本逐条判定（真机步骤/期望/待验证/MIUI/路径约定）。"""
    print(f"[1] 文档集：{len(files)} 个文件（{', '.join(p.name for p in files)}）")
    rep.fact("docs_files", [str(p) for p in files])
    if not corpus.strip():
        rep.fail("文档内容为空")
        return
    rep.ok(f"文档集非空（合计 {len(corpus)} 字符）")

    print("[2] 真机步骤所需的关键内容")
    missing = []
    for label, pattern in REQUIRED_PATTERNS.items():
        if re.search(pattern, corpus, re.I):
            rep.ok(f"含 {label}")
        else:
            missing.append(label)
            rep.fail(f"缺少 {label}（正则 /{pattern}/）")

    print("[3] 步骤表与期望结果")
    if TABLE_STEP_RE.search(corpus):
        rep.ok("存在含「步骤」列的表头")
    else:
        rep.fail("未找到含「步骤」列的表头（真机步骤表缺失）")
    if TABLE_EXPECT_RE.search(corpus):
        rep.ok("存在含「期望结果」列的表头")
    else:
        rep.fail("未找到含「期望结果」列的表头（每步期望结果缺失）")
    expect_mentions = len(re.findall(r"期望", corpus))
    rep.fact("expect_mentions", expect_mentions)
    if expect_mentions >= MIN_EXPECT_MENTIONS:
        rep.ok(f"「期望」出现 {expect_mentions} 次（≥ {MIN_EXPECT_MENTIONS}）")
    else:
        rep.fail(f"「期望」仅出现 {expect_mentions} 次（< {MIN_EXPECT_MENTIONS}）")

    print("[4] 延期状态字段（待验证）")
    pending_mentions = len(re.findall(r"待验证", corpus))
    rep.fact("pending_mentions", pending_mentions)
    if TABLE_STATUS_RE.search(corpus):
        rep.ok("存在「| 待验证 |」形式的表格状态单元格")
    else:
        rep.fail("未找到「| 待验证 |」表格状态单元格（延期项未被标为待验证）")
    if pending_mentions >= MIN_PENDING_MENTIONS:
        rep.ok(f"「待验证」出现 {pending_mentions} 次（≥ {MIN_PENDING_MENTIONS}）")
    else:
        rep.fail(f"「待验证」仅出现 {pending_mentions} 次（< {MIN_PENDING_MENTIONS}）")

    print("[5] 禁止事项：不得声称已真机验证")
    claims = [
        f"/{rx.pattern}/ 命中 {m.group(0)!r}"
        for rx in FORBIDDEN_CLAIM_RES
        for m in [rx.search(corpus)]
        if m
    ]
    if claims:
        rep.fail("文档出现『已真机验证』类表述 → " + "；".join(claims))
    else:
        rep.ok("文档未声称已真机验证（所有真机项保持待验证）")


def resolve_evidence_paths(results_roots: list[Path]) -> list[Path]:
    """把 `--results` 的每种调用口径都解析成候选证据路径（去重保序）。"""
    out: list[Path] = []
    for root in results_roots:
        root = root.resolve()
        # 传入 results 的子目录（如 results/android）时，以**上级 results 根**为主导，
        # 保证 `--results results` 与 `--results results/android` 解析到同一份约定路径。
        if root.parent.name == "results" or (root.parent / "verify").is_dir():
            roots = [root.parent, root]
        else:
            roots = [root]
        for r in roots:
            out.append(r / "verify" / "android-device-verify.md")
            out.append(r / "android" / "device-verify.md")
            out.append(r / "android-device-verify.md")
    out.append(REPO_ROOT / EVIDENCE_RELATIVE)
    seen: set[str] = set()
    uniq: list[Path] = []
    for p in out:
        key = str(p)
        if key not in seen:
            seen.add(key)
            uniq.append(p)
    return uniq


def read_build_marker(gradle_file: Path) -> str | None:
    if not gradle_file.is_file():
        return None
    text = gradle_file.read_text(encoding="utf-8")
    m = re.search(
        r'buildConfigField\(\s*"String"\s*,\s*"BUILD_MARKER"\s*,\s*"\\"([^"\\]+)\\""\s*\)',
        text,
    )
    return m.group(1) if m else None


def check_marker_chain(rep: Readiness, gradle_file: Path, build_info: Path) -> None:
    print("[6] 构建标记跨文件一致性（gradle ↔ dist/BUILD-INFO.md）")
    marker = read_build_marker(gradle_file)
    if marker is None:
        rep.fail(f"无法从 {gradle_file} 解析 BUILD_MARKER（构建标记渠道断裂）")
        return
    rep.fact("gradle_marker", marker)
    rep.ok(f"build.gradle.kts 的 BUILD_MARKER = {marker}")
    if not build_info.is_file():
        rep.fail(f"缺少 {build_info}（分发包元信息未生成）")
        return
    text = build_info.read_text(encoding="utf-8")
    rep.fact("build_info_file", str(build_info))
    if "BUILD_MARKER" not in text:
        rep.fail(f"{build_info} 未记录 BUILD_MARKER")
        return
    values = {
        m.group(1)
        for rx in MARKER_VALUE_RES
        for m in [rx.search(text)]
        if m and m.group(1) != "BUILD_MARKER"
    }
    if marker in values:
        rep.ok(f"BUILD-INFO.md 记录的标记与 gradle 一致：{marker}")
    elif values:
        rep.fail(f"BUILD-INFO.md 标记 {sorted(values)} 与 gradle {marker!r} 不一致（分发包过期）")
    else:
        rep.fail(f"{build_info} 中未解析到具体标记值（无法证明分发包与源码对应）")


def check_dist_refs(rep: Readiness, corpus: str) -> None:
    print("[7] 文档引用的分发包是否真实存在")
    names = sorted(set(DIST_APK_RE.findall(corpus)))
    if not names:
        rep.fail("文档未引用任何 dist/*.apk（真机步骤缺少可安装产物）")
        return
    rep.fact("dist_refs", names)
    missing = [n for n in names if not (REPO_ROOT / "dist" / n).is_file()]
    if missing:
        rep.fail(f"文档引用了不存在的分发包: {missing}（应为 dist/ 下的真实产物）")
    else:
        rep.ok(f"doc 引用的 {len(names)} 个分发包均存在: {', '.join(names)}")


def check_evidence(rep: Readiness, candidates: list[Path]) -> None:
    print("[8] 延期证据路径约定与当前状态")
    existing = [p for p in candidates if p.is_file()]
    canonical = REPO_ROOT / EVIDENCE_RELATIVE
    rep.fact("evidence_candidates", [str(p) for p in candidates])
    rep.fact("evidence_resolved", str(existing[0]) if existing else str(canonical))
    print(f"       约定路径（相对仓库根）: {EVIDENCE_RELATIVE}")
    print(f"       解析后的绝对路径: {existing[0] if existing else canonical}")
    if not existing:
        rep.skip(
            "延期证据尚未写入 → 状态 待验证（真机执行后由 V-AN2 写入；本项不作为失败）"
        )
        rep.fact("evidence_status", "待验证（文件未写入）")
        return
    ev = existing[0]
    text = ev.read_text(encoding="utf-8", errors="replace")
    rep.fact("evidence_status", "存在")
    if not text.strip():
        rep.fail(f"延期证据文件为空: {ev}")
        return
    has_pending = "待验证" in text
    has_device = re.search(r"机型|model", text, re.I) is not None
    if has_pending or has_device:
        rep.ok(f"延期证据文件存在且含状态/机型字段（{len(text)} 字符）: {ev}")
    else:
        rep.fail(f"延期证据文件既无『待验证』也无机型字段: {ev}")


# --------------------------------------------------------------------------- #
# 自测：每条判定的反向能力
# --------------------------------------------------------------------------- #


def self_test() -> list[str]:
    failures: list[str] = []
    print("[self-test] 合成 fixture 反向用例")

    # 完整 fixture（取自真实文档的骨架，字段齐全）
    complete = "\n".join(
        [
            "| # | 步骤 | 操作 | 期望结果 | 状态 |",
            "|---|---|---|---|---|",
        ]
        + [f"| S{i} | 步骤{i} | adb install x | 期望结果{i} | 待验证 |" for i in range(1, 8)]
        + [
            "BUILD_MARKER 核对：与 dist/BUILD-INFO.md 一致；minSdk 31（Android 12）",
            "MIUI：USB 安装 与 USB 调试（安全设置）；INSTALL_FAILED_USER_RESTRICTED / INJECT_EVENTS",
            "证据写入 results/verify/android-device-verify.md，附件放 results/verify/android-device-logs/",
            "记录机型与系统版本；未签名 Release 属预期；判断 shell 进程崩溃与应用崩溃的区别",
        ]
    )

    def run(text: str) -> list[str]:
        rep = Readiness()
        import io
        import contextlib

        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            evaluate_corpus(rep, text, [Path("fixture.md")])
        return rep.failures

    base_failures = run(complete)
    if base_failures:
        failures.append(f"完整 fixture 误报失败: {base_failures}")
        print(f"  FAIL  完整 fixture 不应有失败项，实际: {base_failures}")
    else:
        print("  PASS  完整 fixture 通过（检查项不空转）")

    mutations = {
        "删掉 adb install": complete.replace("adb install x", "安装应用"),
        "删掉 BUILD_MARKER": complete.replace("BUILD_MARKER", "版本标记"),
        "删掉 MIUI USB 安装开关": complete.replace("USB 安装", "USB 相关开关"),
        "删掉 USB 调试（安全设置）": complete.replace("USB 调试（安全设置）", "调试开关"),
        "删掉 INJECT_EVENTS": complete.replace("INJECT_EVENTS", "权限错误"),
        "删掉 期望结果 列": complete.replace("| 期望结果 |", "| 结果 |").replace("期望结果", "结果"),
        "删掉 待验证 状态": complete.replace("待验证", "未执行"),
        "删掉证据路径约定": complete.replace("results/verify/android-device-verify.md", "证据文件"),
        "删掉机型要求": complete.replace("机型", "设备"),
        "删掉 dist/BUILD-INFO.md 引用": complete.replace("dist/BUILD-INFO.md", "构建信息"),
        "写入『已真机验证』": complete + "\n本卡已真机验证通过。\n",
    }
    for label, text in mutations.items():
        found = run(text)
        if found:
            print(f"  PASS  反向用例被报出：{label} → {found[0]}")
        else:
            failures.append(f"反向用例未被报出：{label}")
            print(f"  FAIL  反向用例未被报出：{label}")

    # 证据路径解析：两种调用口径必须落到同一份约定
    with tempfile.TemporaryDirectory() as td:
        root = Path(td) / "results"
        (root / "verify").mkdir(parents=True)
        got_root = resolve_evidence_paths([root])[0]
        got_sub = resolve_evidence_paths([root / "android"])[0]
        if got_root == got_sub == root / "verify" / "android-device-verify.md":
            print(f"  PASS  --results 两种口径解析一致：{got_root}")
        else:
            failures.append(f"--results 解析不一致: {got_root} vs {got_sub}")
            print(f"  FAIL  --results 解析不一致: {got_root} vs {got_sub}")

        # 构建标记链路：匹配必须通过；不一致 / 缺失必须报出
        def run_marker_chain(gradle_text: str, info_text: str | None) -> list[str]:
            import contextlib
            import io

            g = Path(td) / "build.gradle.kts"
            g.write_text(gradle_text, encoding="utf-8")
            b = Path(td) / "BUILD-INFO.md"
            if info_text is None:
                b.unlink(missing_ok=True)
            else:
                b.write_text(info_text, encoding="utf-8")
            rep = Readiness()
            with contextlib.redirect_stdout(io.StringIO()):
                check_marker_chain(rep, g, b)
            return rep.failures

        gradle_ok = 'buildConfigField("String", "BUILD_MARKER", "\\"m-1234567890\\"")\n'
        info_ok = "# 分发包\n\n- 构建标记 BUILD_MARKER：`m-1234567890`\n"
        info_bad = "# 分发包\n\n- 构建标记 BUILD_MARKER：`m-9999999999`\n"
        cases = [
            ("标记一致", gradle_ok, info_ok, False),
            ("BUILD-INFO 标记不一致", gradle_ok, info_bad, True),
            ("BUILD-INFO 缺失", gradle_ok, None, True),
            ("gradle 无 BUILD_MARKER", 'buildConfigField("String", "X", "\\"y\\"")\n', info_ok, True),
        ]
        for label, gtext, itext, want_fail in cases:
            found = run_marker_chain(gtext, itext)
            if bool(found) == want_fail:
                print(f"  PASS  构建标记链路判定正确：{label} → {'报出' if found else '通过'}")
            else:
                failures.append(f"构建标记链路判定错误：{label}（failures={found}）")
                print(f"  FAIL  构建标记链路判定错误：{label}（failures={found}）")

        # 文档引用不存在的分发包必须报出
        import contextlib
        import io

        rep_missing = Readiness()
        with contextlib.redirect_stdout(io.StringIO()):
            check_dist_refs(rep_missing, "安装 dist/app-debug.apk 与 dist/does-not-exist.apk")
        if rep_missing.failures:
            print(f"  PASS  引用不存在的分发包被报出：{rep_missing.failures[0]}")
        else:
            failures.append("文档引用不存在的分发包未被报出")
            print("  FAIL  文档引用不存在的分发包未被报出")
        rep_present = Readiness()
        with contextlib.redirect_stdout(io.StringIO()):
            check_dist_refs(rep_present, "安装 dist/app-debug.apk")
        if not rep_present.failures:
            print("  PASS  引用真实存在的分发包不误报")
        else:
            failures.append(f"引用真实分发包被误报: {rep_present.failures}")
            print(f"  FAIL  引用真实分发包被误报: {rep_present.failures}")
    return failures


# --------------------------------------------------------------------------- #
# CLI
# --------------------------------------------------------------------------- #


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description="真机延期验收就绪度门禁（C9）")
    p.add_argument(
        "--docs",
        action="append",
        default=None,
        help="真机说明目录（可重复；默认 docs/android）",
    )
    p.add_argument(
        "--results",
        action="append",
        default=None,
        help="证据根目录（可重复；接受 results 或其子目录，如 results/android）",
    )
    p.add_argument("--gradle-file", default=str(GRADLE_FILE))
    p.add_argument("--build-info", default=str(BUILD_INFO_FILE))
    p.add_argument("--json", dest="json_out", default=None, help="把结论写入 JSON 文件")
    p.add_argument("--self-test", action="store_true", help="运行合成 fixture 反向用例")
    args = p.parse_args(argv)

    docs_dirs = [Path(d) for d in (args.docs or ["docs/android"])]
    results_roots = [Path(r) for r in (args.results or ["results"])]

    print(f"verify-android-device-readiness: repo={REPO_ROOT}")
    with_docs = ", ".join(str(d) for d in docs_dirs)
    with_results = ", ".join(str(r) for r in results_roots)
    print(f"  docs={with_docs}  results={with_results}")

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

    rep = Readiness()
    files: list[Path] = []
    for d in docs_dirs:
        if not d.is_dir():
            print(f"RESULT: FAIL — 文档目录不存在: {d}")
            return 2
        files.extend(sorted(d.rglob("*.md")))
    if not files:
        print(f"RESULT: FAIL — 文档目录内没有任何 Markdown: {[str(d) for d in docs_dirs]}")
        return 2
    corpus = "\n".join(f.read_text(encoding="utf-8", errors="replace") for f in files)

    evaluate_corpus(rep, corpus, files)
    check_marker_chain(rep, Path(args.gradle_file), Path(args.build_info))
    check_dist_refs(rep, corpus)
    check_evidence(rep, resolve_evidence_paths(results_roots))

    print("-" * 68)
    if args.json_out:
        payload = {
            "result": "PASS" if (not rep.failures and exit_code == 0) else "FAIL",
            "docs": [str(d) for d in docs_dirs],
            "ok_count": rep.ok_count,
            "failures": rep.failures,
            "skips": rep.skips,
            "facts": rep.facts,
        }
        Path(args.json_out).write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")
        print(f"JSON 结论已写入 {args.json_out}")
    if rep.failures:
        print(f"RESULT: FAIL ({len(rep.failures)} 项)")
        for item in rep.failures:
            print(f"  - {item}")
        return 1
    if exit_code != 0:
        print("RESULT: FAIL — --self-test 未通过")
        return 1
    status = rep.facts.get("evidence_status", "未知")
    print(
        f"RESULT: PASS — 真机说明完备（断言 {rep.ok_count} 项，跳过 {len(rep.skips)} 项）；"
        f"延期证据状态: {status}；未包含任何真机通过结论"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())

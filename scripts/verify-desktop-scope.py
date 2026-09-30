#!/usr/bin/env python3
"""verify-desktop-scope.py — V1 生产桌面（DT1..DT7）的范围与数据源来源检查。

用途（desktop/spec.md 风险表「当前探针模拟内容混入产品 → 用户看到假数据」、
主规格「范围外：通过截图 OCR 或私有 Slint 文本 API 实现分页」）：

  证明**生产桌面路径**里不存在
    (a) Synthetic（合成）书籍 / 模拟书库；
    (b) 空 External 数据源；
    (c) 整本正文复制（无界整文件读取）；
    (d) 截图 OCR 通道；
  并证明 UI 层没有复制核心阅读语义（编码识别 / 章节识别 / 分页边界 / 书签规则），
  以及桌面工程没有把 UI 依赖（slint）混进冻结核心依赖图。

检查项：
  [1] 桌面工程文件集存在（crates/desktop-app/{Cargo.toml,src}、desktop/ui/*.slint），
      且 `desktop/ui/` 是产品唯一 Slint 来源（`crates/` 下不得有 .slint 副本）；
  [2] Synthetic / 模拟书库 / 空 External 数据源标记（注释与字符串剥离后，大小写不敏感）；
  [3] 截图 OCR 通道标记与相关依赖；
  [4] 整本正文复制：生产源码中不得出现无界整文件读取
      （`read_to_string` / `read_to_end` / `fs::read(` 等），除该行显式带
      `scope-allow:` 注释说明理由；也不得 `include_bytes!` / `include_str!` 内联大文件；
  [5] 探针隔离：桌面依赖不得指向 `experiments/`，源码不得引用 `experiments/` 路径；
  [6] UI 层不得实现阅读语义（.slint 中不得出现编码/章节识别/分页/书签算法标记）；
  [7] workspace 归属：桌面包不得是根 workspace 成员，且必须自带嵌套 workspace
      （保证 `scripts/verify-core-scope.py` 的「成员恰为 5 个生产 crate」与
      「核心依赖树无 slint」两条不变量不被桌面 UI 依赖破坏）；
  [8] 正向对照（自检）：同一扫描规则作用在 `experiments/` 探针源码上必须命中，
      以证明第 2 项不是空检查（探针缺失时该项 SKIP 并说明）。

退出码：0 = 全部通过；1 = 存在违规；2 = 脚本自身前提缺失（如桌面工程文件集缺失）。
不变量说明：本脚本只做静态范围检查（不启动 GUI、不跑 cargo），
`cargo check` / `cargo test` 由卡片 DoD 的构建命令另行取证。

局限说明：源码扫描是轻量词法处理（行注释、嵌套块注释、字符串/字符字面量），
不处理 Rust raw string（`r#"..."#`）；若未来引入 raw string 内含违规标识符，需人工复核。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]

DESKTOP_CRATE = REPO_ROOT / "crates" / "desktop-app"
DESKTOP_MANIFEST = DESKTOP_CRATE / "Cargo.toml"
DESKTOP_SRC = DESKTOP_CRATE / "src"
DESKTOP_TESTS = DESKTOP_CRATE / "tests"
UI_DIR = REPO_ROOT / "desktop" / "ui"
ROOT_MANIFEST = REPO_ROOT / "Cargo.toml"
EXPERIMENTS_DIR = REPO_ROOT / "experiments"

# 扫描范围：生产桌面路径（不含 experiments/，探针只用于正向对照）。
SCAN_DIRS = (DESKTOP_SRC, DESKTOP_TESTS, UI_DIR)

# (a)(b) Synthetic / 模拟书库 / 空 External 数据源：大小写不敏感匹配。
FORBIDDEN_SOURCE_TOKENS = (
    "synthetic",
    "mock_library",
    "mocklibrary",
    "mock_library_source",
    "externalsource",
    "externaltextsource",
    "emptyexternalsource",
    "externalsourceprovider",
    "模拟书库",
    "合成文本源",
    "假数据源",
)

# (d) 截图 OCR 通道：大小写不敏感匹配（含图像/OCR 依赖名）。
FORBIDDEN_SCREENSHOT_TOKENS = (
    "ocr",
    "tesseract",
    "leptonica",
    "zbar",
    "screenshot",
    "x11grab",
    "scrot",
    "xwd",
    "截屏",
    "截图",
)
# 说明：`ocr` 采用词边界匹配（见 scan_tokens），避免命中 `ocr_` 之外的无辜子串。
WORD_BOUNDED_TOKENS = {"ocr", "xwd"}

# (c) 无界整文件读取（整本正文复制的载体）。
UNBOUNDED_READ_TOKENS = (
    "read_to_string",
    "read_to_end",
    "fs::read(",
    "include_bytes!",
    "include_str!",
)
# 该行带此注释时视为已说明理由（例如读取 UI 定义文件而非书籍正文）。
SCOPE_ALLOW_MARKER = "scope-allow:"

# (f) UI 层不得出现阅读语义算法标记（.slint 注释剥离后匹配）。
FORBIDDEN_UI_LOGIC_TOKENS = (
    "encoding",
    "encode(",
    "gb18030",
    "gbk",
    "big5",
    "utf-16",
    "utf16",
    "detect_encoding",
    "pagination",
    "page_boundary",
    "pageboundary",
    "bookmark",
    "章节识别",
    "分页边界",
    "编码识别",
    "位置校验",
    "书签规则",
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


def rel(path: Path) -> str:
    try:
        return str(path.relative_to(REPO_ROOT))
    except ValueError:
        return str(path)


def strip_rust_comments_and_literals(text: str) -> str:
    """去除 Rust 源码中的注释与字符串/字符字面量（与 scripts/verify-core-scope.py 同口径）。"""
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


def strip_slint_comments(text: str) -> str:
    """去除 Slint 源码中的注释（`//...` 与 `/* ... */`），保留代码结构。"""
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
            i += 2
            while i < n and not (text[i] == "*" and i + 1 < n and text[i + 1] == "/"):
                i += 1
            i += 2
            continue
        out.append(ch)
        i += 1
    return "".join(out)


def strip_by_suffix(path: Path, text: str) -> str:
    if path.suffix == ".rs":
        return strip_rust_comments_and_literals(text)
    if path.suffix == ".slint":
        return strip_slint_comments(text)
    if path.suffix == ".toml":
        return re.sub(r"#[^\n]*", "", text)
    return text


def scanned_files() -> list[Path]:
    files: list[Path] = []
    for directory in SCAN_DIRS:
        if directory.is_dir():
            files.extend(sorted(p for p in directory.rglob("*") if p.is_file()))
    if DESKTOP_MANIFEST.is_file():
        files.append(DESKTOP_MANIFEST)
    return sorted(files)


def scan_tokens(files: list[Path], tokens: tuple[str, ...]) -> list[tuple[Path, int, str]]:
    hits: list[tuple[Path, int, str]] = []
    for path in files:
        text = path.read_text(encoding="utf-8", errors="replace")
        code = strip_by_suffix(path, text)
        for lineno, line in enumerate(code.splitlines(), start=1):
            lowered = line.lower()
            for token in tokens:
                if token in WORD_BOUNDED_TOKENS:
                    if re.search(rf"\b{re.escape(token)}\b", lowered):
                        hits.append((path, lineno, token))
                elif token in lowered:
                    hits.append((path, lineno, token))
    return hits


def check_desktop_fileset() -> None:
    print("[1] 桌面工程文件集与 Slint 唯一定义处")
    missing: list[str] = []
    if not DESKTOP_MANIFEST.is_file():
        missing.append(rel(DESKTOP_MANIFEST))
    if not DESKTOP_SRC.is_dir():
        missing.append(rel(DESKTOP_SRC))
    slint_files = sorted(UI_DIR.glob("*.slint")) if UI_DIR.is_dir() else []
    if not slint_files:
        missing.append(f"{rel(UI_DIR)}/*.slint")
    if missing:
        fail(f"桌面工程文件集缺失: {missing}")
    else:
        ok(f"桌面工程存在：{rel(DESKTOP_MANIFEST)} + {len(slint_files)} 个 .slint（{rel(UI_DIR)}）")

    strays = sorted(p for p in (REPO_ROOT / "crates").rglob("*.slint"))
    if strays:
        for path in strays:
            fail(f"`crates/` 下不得出现 .slint 副本（唯一来源是 desktop/ui/）: {rel(path)}")
    else:
        ok("`crates/` 下无 .slint 副本（UI 定义唯一来源为 desktop/ui/）")


def check_synthetic_and_external() -> None:
    print("[2] Synthetic / 模拟书库 / 空 External 数据源标记")
    files = scanned_files()
    if not files:
        fail("未找到可扫描的桌面文件集，检查范围无效")
        return
    hits = scan_tokens(files, FORBIDDEN_SOURCE_TOKENS)
    if hits:
        for path, lineno, token in hits[:20]:
            fail(f"{rel(path)}:{lineno} 出现禁用数据源标记 `{token}`")
    else:
        ok(f"{len(files)} 个桌面文件均无 Synthetic/模拟书库/空 External 标记")


def check_screenshot_ocr() -> None:
    print("[3] 截图 OCR 通道")
    files = scanned_files()
    hits = scan_tokens(files, FORBIDDEN_SCREENSHOT_TOKENS)
    if hits:
        for path, lineno, token in hits[:20]:
            fail(f"{rel(path)}:{lineno} 出现截图/OCR 标记 `{token}`")
    else:
        ok(f"{len(files)} 个桌面文件均无截图/OCR 标记")


def check_whole_book_copy() -> None:
    print("[4] 整本正文复制（无界整文件读取）")
    if not DESKTOP_SRC.is_dir():
        fail("桌面源码目录缺失，检查范围无效")
        return
    files = sorted(p for p in DESKTOP_SRC.rglob("*.rs") if p.is_file())
    hits: list[tuple[Path, int, str]] = []
    for path in files:
        raw = path.read_text(encoding="utf-8", errors="replace")
        code = strip_rust_comments_and_literals(raw)
        for lineno, line in enumerate(code.splitlines(), start=1):
            for token in UNBOUNDED_READ_TOKENS:
                if token in line:
                    hits.append((path, lineno, token))
    if hits:
        allowed: list[tuple[Path, int, str]] = []
        still: list[tuple[Path, int, str]] = []
        for path, lineno, token in hits:
            raw_lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
            source_line = raw_lines[lineno - 1] if 0 <= lineno - 1 < len(raw_lines) else ""
            if SCOPE_ALLOW_MARKER in source_line:
                allowed.append((path, lineno, token))
            else:
                still.append((path, lineno, token))
        for path, lineno, token in allowed:
            note(f"{rel(path)}:{lineno} 的 `{token}` 带 `{SCOPE_ALLOW_MARKER}` 说明，已豁免")
        for path, lineno, token in still[:20]:
            fail(
                f"{rel(path)}:{lineno} 出现无界整文件读取 `{token}`；"
                f"桌面端不得复制整本正文（需要时用 `{SCOPE_ALLOW_MARKER}` 说明理由）"
            )
        if not still:
            ok(f"{len(files)} 个桌面源码文件无未说明的整文件读取")
    else:
        ok(f"{len(files)} 个桌面源码文件无无界整文件读取（无整本正文复制）")


def check_probe_isolation() -> None:
    print("[5] 探针隔离（experiments/ 不得进入生产桌面）")
    if not DESKTOP_MANIFEST.is_file():
        fail(f"缺少桌面 manifest: {rel(DESKTOP_MANIFEST)}")
        return
    text = DESKTOP_MANIFEST.read_text(encoding="utf-8")
    if EXPERIMENTS_DIR.name in text:
        fail(f"{rel(DESKTOP_MANIFEST)} 出现 `{EXPERIMENTS_DIR.name}` 引用（探针不得进入生产依赖）")
    else:
        ok(f"{rel(DESKTOP_MANIFEST)} 未引用 `{EXPERIMENTS_DIR.name}/`")

    hits: list[tuple[Path, int, str]] = []
    for path in scanned_files():
        if path.suffix not in {".rs", ".slint", ".toml"}:
            continue
        code = strip_by_suffix(path, path.read_text(encoding="utf-8", errors="replace"))
        for lineno, line in enumerate(code.splitlines(), start=1):
            if EXPERIMENTS_DIR.name in line:
                hits.append((path, lineno, EXPERIMENTS_DIR.name))
    if hits:
        for path, lineno, token in hits[:20]:
            fail(f"{rel(path)}:{lineno} 引用验证探针路径 `{token}/`")
    else:
        ok("桌面源码/UI/manifest 均不引用 `experiments/`")


def check_ui_has_no_reading_logic() -> None:
    print("[6] UI 层不得实现阅读业务语义（.slint）")
    slint_files = sorted(UI_DIR.glob("*.slint")) + sorted(
        DESKTOP_CRATE.rglob("*.slint")
    )
    slint_files = sorted(set(slint_files))
    if not slint_files:
        fail("未找到 .slint 文件，检查范围无效")
        return
    hits = scan_tokens(slint_files, FORBIDDEN_UI_LOGIC_TOKENS)
    if hits:
        for path, lineno, token in hits[:20]:
            fail(
                f"{rel(path)}:{lineno} 出现阅读语义标记 `{token}`；"
                "编码识别/章节识别/分页边界/书签规则属于冻结核心，UI 只做状态投影"
            )
    else:
        ok(f"{len(slint_files)} 个 .slint 文件均无编码/分页/书签算法标记")


def check_workspace_membership() -> None:
    print("[7] workspace 归属（桌面 UI 依赖不得进入冻结核心依赖图）")
    if not ROOT_MANIFEST.is_file():
        fail(f"缺少根 manifest: {rel(ROOT_MANIFEST)}")
        return
    root_text = ROOT_MANIFEST.read_text(encoding="utf-8")
    workspace_section = root_text.split("[workspace", 1)[-1]
    match = re.search(r"members\s*=\s*\[([^\]]*)\]", workspace_section, re.S)
    members = re.findall(r'"([^"]+)"', match.group(1)) if match else []
    member_names = sorted(Path(m).name for m in members)
    if "desktop-app" in member_names:
        fail(
            "根 workspace 不得包含 `desktop-app`：桌面依赖 slint，进入根依赖图会破坏 "
            "scripts/verify-core-scope.py 的成员集合与「核心依赖树无 UI」不变量"
        )
    else:
        ok(f"根 workspace 成员 ({member_names}) 不含 desktop-app")

    if not DESKTOP_MANIFEST.is_file():
        fail(f"缺少桌面 manifest: {rel(DESKTOP_MANIFEST)}")
        return
    desktop_text = DESKTOP_MANIFEST.read_text(encoding="utf-8")
    if re.search(r"^\s*\[workspace\]", desktop_text, re.M):
        ok("desktop-app 自带嵌套 `[workspace]`（与根 workspace 解耦）")
    else:
        fail(
            f"{rel(DESKTOP_MANIFEST)} 必须自带 `[workspace]`：否则该包会试图加入根 workspace，"
            "导致根成员集合变化"
        )


def check_detector_positive_control() -> None:
    print("[8] 正向对照：扫描规则在 experiments/ 探针上必须命中（自检非空检查）")
    if not EXPERIMENTS_DIR.is_dir():
        note(f"`{EXPERIMENTS_DIR.name}/` 不存在，跳过正向对照（无法证明扫描灵敏度）")
        return
    probe_files = sorted(EXPERIMENTS_DIR.rglob("*.rs"))
    if not probe_files:
        note(f"`{EXPERIMENTS_DIR.name}/` 无 Rust 源码，跳过正向对照")
        return
    hits = scan_tokens(probe_files, FORBIDDEN_SOURCE_TOKENS)
    if hits:
        token = hits[0][2]
        ok(f"扫描规则在 {len(hits)} 处命中探针源码（如 `{token}`），证明检测非空")
    else:
        fail(
            "扫描规则在探针源码上零命中，说明第 2 项为空检查，"
            "无法证明生产桌面路径无 Synthetic/空 External 数据源"
        )

    ocr_hits = scan_tokens(probe_files, FORBIDDEN_SCREENSHOT_TOKENS)
    if ocr_hits:
        ok(f"截图/OCR 规则在探针源码命中 {len(ocr_hits)} 处，证明第 3 项非空检查")
    else:
        note(
            "截图/OCR 规则在探针源码零命中（探针未使用截图/OCR 通道）；"
            "该项灵敏度依赖规则文本本身，人工复核规则清单"
        )


def main() -> int:
    print(f"verify-desktop-scope: repo={REPO_ROOT}")
    check_desktop_fileset()
    check_synthetic_and_external()
    check_screenshot_ocr()
    check_whole_book_copy()
    check_probe_isolation()
    check_ui_has_no_reading_logic()
    check_workspace_membership()
    check_detector_positive_control()
    print("-" * 60)
    if FAILURES:
        print(f"RESULT: FAIL ({len(FAILURES)} 项违规)")
        for item in FAILURES:
            print(f"  - {item}")
        return 1
    if NOTES:
        print(f"RESULT: PASS（{len(NOTES)} 项 SKIP，已在上面逐条说明）"
              " — 生产桌面无 Synthetic 书籍/空 External/整本复制/截图 OCR")
    else:
        print("RESULT: PASS — 生产桌面无 Synthetic 书籍/空 External/整本复制/截图 OCR")
    return 0


if __name__ == "__main__":
    sys.exit(main())

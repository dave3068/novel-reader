#!/usr/bin/env python3
"""run_txt_lifecycle.py — DT4 阅读闭环（章节 / 进度 / 书签 / 设置 / 重启恢复）端到端验证。

口径（卡片 `t_1f76a9a6` 的完成定义）：

- 直接运行**真实 Release 二进制** `crates/desktop-app/target/release/desktop-app`
  （真实 X11 窗口 + 真实 SQLite + 真实 TXT 文件），不 mock 任何一层；
- 阶段：导入 → 阅读（建立章节目录）→ 翻页（用户入口，含进度保存）→ 设置（全部全局设置项）
  → 章节跳转 → 书签（新增 / 跳转 / 重命名 / 删除）→ 保存 → **重启（全新进程）→ 恢复**；
- 每次运行都是独立进程，跨进程的可观察事实（页起点 / 章节区间 / 书签偏移 / 设置值）全部来自
  stdout 的结构化行与**直接读取的 SQLite 表**，不是自报结论；
- 断言「重启恢复位置正确」用两个独立事实交叉验证：恢复行的 `offset` == 上一进程保存的页起点，
  且新进程的 `NR-PAGE start/end/lines/layout` 与上一进程末页逐项相等；
- 断言「书签增删改跳有效」用真实表数据（`bookmarks` 行数与 `title` / `byte_offset`）；
- 断言「正文不入库」用夹具正文切片在全库文本列中做包含性检查。

用法：
    DISPLAY=:0 python3 tests/e2e/run_txt_lifecycle.py --platform desktop --fixture tests/fixtures/smoke.txt
退出码：0 = 全部通过；1 = 存在失败；2 = 前提缺失（无二进制 / 无显示 / 无夹具）。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_BINARY = REPO_ROOT / "crates/desktop-app/target/release/desktop-app"
PLATFORMS = ("desktop",)

FAILURES: list[str] = []
CHECKS: list[dict] = []
LAUNCHES: list[dict] = []
STAGES: list[dict] = []


def stage(index: int, name: str, detail: str = "") -> None:
    STAGES.append({"stage": index, "name": name, "detail": detail})
    print()
    print(f"[{index}] {name}{('：' + detail) if detail else ''}")


def record(name: str, ok: bool, detail: str) -> None:
    CHECKS.append({"check": name, "ok": bool(ok), "detail": detail})
    if ok:
        print(f"  PASS  {name}: {detail}")
    else:
        FAILURES.append(f"{name}: {detail}")
        print(f"  FAIL  {name}: {detail}")


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(65536), b""):
            digest.update(chunk)
    return digest.hexdigest()


PAGE_RE = re.compile(
    r"^NR-PAGE book=(?P<book>-?\d+) layout=(?P<layout>\d+) req=(?P<req>\d+) "
    r"page_index=(?P<page_index>-?\d+) start=(?P<start>\d+) end=(?P<end>\d+) "
    r"lines=(?P<lines>\d+) lines_per_page=(?P<lines_per_page>\d+) pitch=(?P<pitch>\d+) "
    r"font=(?P<font>\d+) viewport=(?P<vw>\d+)x(?P<vh>\d+) menu=(?P<menu>\d+) "
    r"eof=(?P<eof>\d+) turns=(?P<turns>\d+) reflows=(?P<reflows>\d+) "
    r"measured=(?P<measured>\d+) stale=(?P<stale>\d+) cached=(?P<cached>\d+) "
    r"total=(?P<total>\d+)$"
)
STATUS_RE = re.compile(r"status=(?P<status>.*)$")
STATE_RE = re.compile(r"^NR-STATE (?P<fields>.*)$")
TOC_INDEX_RE = re.compile(
    r"^NR-TOC-INDEX book=(?P<book>-?\d+) origin=(?P<origin>\w+) chapters=(?P<chapters>\d+) "
    r"persisted=(?P<persisted>\d+) index_version=(?P<index_version>-?\d+)$"
)
TOC_RE = re.compile(
    r"^NR-TOC book=(?P<book>-?\d+) origin=(?P<origin>\w+) chapters=(?P<chapters>\d+) "
    r"virtual=(?P<virtual>\d) index_version=(?P<index_version>\d+) "
    r"scanned_bytes=(?P<scanned_bytes>\d+) windows=(?P<windows>\d+) bytes_read=(?P<bytes_read>\d+) "
    r"max_window=(?P<max_window>\d+) current=(?P<current>\d+) window_first=(?P<window_first>\d+) "
    r"total=(?P<total>\d+)$"
)
POS_RE = re.compile(
    r"^NR-POS book=(?P<book>-?\d+) chapter=(?P<chapter>\d+) offset=(?P<offset>\d+) "
    r"restored=(?P<restored>\d) fallback=(?P<fallback>\d) anchor=(?P<anchor>\S+) "
    r"distance=(?P<distance>\d+) windows=(?P<windows>\d+) bytes_read=(?P<bytes_read>\d+) "
    r"radius=(?P<radius>\d+)( saved_offset=(?P<saved_offset>\d+))?$"
)
PROGRESS_RE = re.compile(
    r"^NR-PROGRESS op=save inserted=(?P<inserted>\d+) changed=(?P<changed>\d+) "
    r"book=(?P<book>-?\d+) chapter=(?P<chapter>\d+) offset=(?P<offset>\d+) "
    r"quote_chars=(?P<quote_chars>\d+)$"
)
PROGRESS_SKIP_RE = re.compile(r"^NR-PROGRESS op=save skipped=(?P<skipped>\d+)( .*)?$")
MARK_RE = re.compile(
    r"^NR-MARK op=(?P<op>\w+) id=(?P<id>-?\d+) count=(?P<count>\d+) "
    r"chapter=(?P<chapter>-?\d+) offset=(?P<offset>-?\d+) title=(?P<title>.*)$"
)
CHAPTER_RE = re.compile(
    r"^NR-CHAPTER op=jump sequence=(?P<sequence>\d+) chapters=(?P<chapters>\d+) "
    r"target=(?P<target>\d+)$"
)
SETTINGS_RE = re.compile(r"^NR-SETTINGS (?P<fields>.*)$")


def parse_kv(text: str) -> dict:
    fields: dict[str, str] = {}
    for token in text.split():
        if "=" in token:
            key, value = token.split("=", 1)
            fields[key] = value
    return fields


def parse_pos_line(line: str) -> dict:
    """解析 `NR-POS` 行。

    完整格式（锚点恢复报告）走 `POS_RE`；**无进度**时应用只输出短格式
    `NR-POS book=1 restored=0 chapter=0 offset=0 fallback=0 anchor=none`
    （字段更少且顺序不同），此处按 KV 兜底解析，避免把「无进度」误判为「无证据」。
    """
    if (match := POS_RE.match(line)):
        return match.groupdict()
    return parse_kv(line[len("NR-POS") :])


def run_app(binary: Path, db: Path, extra_env: dict, display: str, timeout: int = 120) -> dict:
    env = dict(os.environ)
    env["DISPLAY"] = display
    env["NR_DB_PATH"] = str(db)
    env["NR_STATE_LOG"] = "1"
    env.setdefault("NR_EXIT_AFTER_MS", "2400")
    for key in (
        "NR_IMPORT_PATH",
        "NR_RELINK_BOOK",
        "NR_RELINK_PATH",
        "NR_READER_BOOK",
        "NR_READER_TURNS",
        "NR_READER_FONT",
        "NR_READER_MENU",
        "NR_START_SCREEN",
        "NR_TURNS_UI",
        "NR_CHAPTER_JUMP",
        "NR_MARK_ADD",
        "NR_MARK_JUMP",
        "NR_MARK_RENAME",
        "NR_MARK_DELETE",
        "NR_SETTINGS_SET",
        "NR_SAVE_PROGRESS",
        "NR_PANEL",
    ):
        env.pop(key, None)
    env.update({key: str(value) for key, value in extra_env.items() if value is not None})
    proc = subprocess.run(
        [str(binary)],
        env=env,
        capture_output=True,
        text=True,
        timeout=timeout,
        cwd=str(REPO_ROOT),
    )
    stdout_lines = proc.stdout.splitlines()
    entry = {
        "env": {key: str(value) for key, value in extra_env.items() if value is not None},
        "exit_code": proc.returncode,
        "pages": [m.groupdict() for line in stdout_lines if (m := PAGE_RE.match(line))],
        "toc_index": [
            m.groupdict() for line in stdout_lines if (m := TOC_INDEX_RE.match(line))
        ],
        "toc": [m.groupdict() for line in stdout_lines if (m := TOC_RE.match(line))],
        "pos": [
            parse_pos_line(line) for line in stdout_lines if line.startswith("NR-POS")
        ],
        "progress": [
            m.groupdict() for line in stdout_lines if (m := PROGRESS_RE.match(line))
        ],
        "progress_skipped": sum(
            1 for line in stdout_lines if PROGRESS_SKIP_RE.match(line)
        ),
        "marks": [m.groupdict() for line in stdout_lines if (m := MARK_RE.match(line))],
        "chapter_jumps": [
            m.groupdict() for line in stdout_lines if (m := CHAPTER_RE.match(line))
        ],
        "settings": [
            parse_kv(m.group("fields")) for line in stdout_lines if (m := SETTINGS_RE.match(line))
        ],
        "states": [
            parse_kv(m.group("fields")) for line in stdout_lines if (m := STATE_RE.match(line))
        ],
        "statuses": [
            STATUS_RE.search(line).groupdict()["status"]
            for line in stdout_lines
            if line.startswith("NR-STATE")
        ],
        "stdout_tail": proc.stdout[-3000:],
        "stderr_tail": proc.stderr[-2000:],
        "lines": stdout_lines,
    }
    entry["page"] = entry["pages"][-1] if entry["pages"] else None
    entry["pos_last"] = entry["pos"][-1] if entry["pos"] else None
    entry["progress_last"] = entry["progress"][-1] if entry["progress"] else None
    entry["state"] = entry["states"][-1] if entry["states"] else {}
    entry["status"] = entry["statuses"][-1] if entry["statuses"] else ""
    LAUNCHES.append({key: value for key, value in entry.items() if key != "lines"})
    return entry


def db_connect(path: Path) -> sqlite3.Connection:
    conn = sqlite3.connect(str(path))
    conn.row_factory = sqlite3.Row
    return conn


def as_int(page: dict, key: str) -> int:
    return int(page[key])


def fixture_chapter_titles(fixture: Path) -> list[str]:
    """夹具里的章节标题（用于与核心扫描结果逐条比对）。"""
    titles: list[str] = []
    for line in fixture.read_text(encoding="utf-8").splitlines():
        if re.match(r"^第[一二三四五六七八九十百千\d]+章\s", line):
            titles.append(line)
    return titles


def fixture_paragraph_slices(fixture: Path, length: int = 30, count: int = 8) -> list[str]:
    """夹具正文（非章节标题行）的切片，用于「正文不入库」的包含性检查。"""
    slices: list[str] = []
    for line in fixture.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if len(stripped) < length or re.match(r"^第[一二三四五六七八九十百千\d]+章\s", stripped):
            continue
        slices.append(stripped[:length])
        if len(slices) >= count:
            break
    return slices


def all_text_cells(db: Path) -> list[str]:
    """全库文本列内容（只用于「正文不入库」的包含性检查）。"""
    cells: list[str] = []
    conn = db_connect(db)
    try:
        tables = [
            row["name"]
            for row in conn.execute(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'"
            )
        ]
        for table in tables:
            for row in conn.execute(f"SELECT * FROM {table}"):  # noqa: S608 - 表名来自 sqlite_master
                for value in tuple(row):
                    if isinstance(value, str) and value.strip():
                        cells.append(value)
    finally:
        conn.close()
    return cells


def main() -> int:
    parser = argparse.ArgumentParser(description="DT4 桌面阅读闭环（章节/进度/书签/设置/恢复）E2E")
    parser.add_argument("--platform", default="desktop", choices=PLATFORMS)
    parser.add_argument("--fixture", default="tests/fixtures/smoke.txt", help="真实 TXT 夹具路径")
    parser.add_argument("--binary", default=str(DEFAULT_BINARY), help="desktop-app Release 二进制")
    parser.add_argument("--display", default=os.environ.get("DISPLAY", ":0"))
    parser.add_argument("--json", action="store_true", help="输出机器可读汇总")
    args = parser.parse_args()

    binary = Path(args.binary).resolve()
    fixture = (REPO_ROOT / args.fixture).resolve() if not Path(args.fixture).is_absolute() else Path(args.fixture)
    if not binary.is_file():
        print(f"前提缺失：找不到 Release 二进制 {binary}", file=sys.stderr)
        print("先执行：cd crates/desktop-app && cargo build --release", file=sys.stderr)
        return 2
    if not fixture.is_file():
        print(f"前提缺失：找不到夹具 {fixture}", file=sys.stderr)
        return 2
    if not args.display:
        print("前提缺失：无 X11 DISPLAY", file=sys.stderr)
        return 2
    display = args.display

    expected_titles = fixture_chapter_titles(fixture)
    body_slices = fixture_paragraph_slices(fixture)
    root = Path(tempfile.mkdtemp(prefix="nr-dt4-e2e-"))
    db = root / "library.db"
    before = {"sha256": sha256_of(fixture), "size": fixture.stat().st_size}

    print(f"仓库根目录：{REPO_ROOT}")
    print(f"平台：{args.platform}（唯一实现的桌面平台口径）")
    print(f"二进制：{binary}")
    print(f"夹具：{fixture}（{before['size']} 字节，{len(expected_titles)} 章）")
    print(f"数据库：{db}")
    print(f"DISPLAY：{display}")

    try:
        # ---- 阶段 1：真实导入 ----
        stage(1, "导入", "NR_IMPORT_PATH → books/book_sources（真实 TXT + 真实 SQLite）")
        imported = run_app(binary, db, {"NR_IMPORT_PATH": fixture}, display)
        record("1.导入退出码 0", imported["exit_code"] == 0, f"exit={imported['exit_code']}")
        record(
            "1.导入提示出现在状态日志",
            any("已导入" in status for status in imported["statuses"]),
            " | ".join(imported["statuses"])[-160:],
        )
        conn = db_connect(db)
        books = conn.execute("SELECT id, display_name, status, source_locator FROM books").fetchall()
        record("1.书籍条数 = 1", len(books) == 1, f"books={len(books)}")
        if not books:
            print("导入未建立书籍记录，后续阶段无法进行", file=sys.stderr)
            return 1
        book_id = int(books[0]["id"])
        record("1.来源状态 ready", books[0]["status"] == "ready", str(books[0]["status"]))
        record(
            "1.定位符指向夹具",
            books[0]["source_locator"] == str(fixture),
            str(books[0]["source_locator"]),
        )
        counts = {
            table: conn.execute(f"SELECT COUNT(*) AS n FROM {table}").fetchone()["n"]
            for table in ("chapters", "reading_progress", "bookmarks", "reader_settings")
        }
        record(
            "1.导入后尚未建立目录/进度/书签/设置",
            counts == {
                "chapters": 0,
                "reading_progress": 0,
                "bookmarks": 0,
                "reader_settings": 0,
            },
            str(counts),
        )
        conn.close()

        # ---- 阶段 2：阅读（建立章节目录）----
        stage(2, "阅读", "NR_READER_BOOK → 真实分页 + 核心章节扫描 + chapters 表缓存")
        first = run_app(
            binary,
            db,
            {"NR_START_SCREEN": "reader", "NR_READER_BOOK": book_id},
            display,
        )
        record("2.退出码 0", first["exit_code"] == 0, f"exit={first['exit_code']}")
        page0 = first["page"]
        record("2.输出首屏结构行", page0 is not None, first["stdout_tail"][-200:])
        if page0 is None:
            return 1
        record(
            "2.首屏从内容起点开始（尚无进度）",
            as_int(page0, "start") == 0 and as_int(page0, "page_index") == 0,
            f"start={page0['start']} page_index={page0['page_index']}",
        )
        record(
            "2.无进度时恢复行为 restored=0",
            (first["pos_last"] or {}).get("restored") == "0",
            str(first["pos_last"]),
        )
        toc_index = first["toc_index"][-1] if first["toc_index"] else None
        record(
            "2.章节目录由真实扫描建立（origin=scanned）",
            toc_index is not None and toc_index["origin"] == "scanned",
            str(toc_index),
        )
        record(
            f"2.章节数 = 夹具章节数（{len(expected_titles)}）",
            toc_index is not None and int(toc_index["chapters"]) == len(expected_titles),
            f"toc={toc_index}",
        )
        conn = db_connect(db)
        rows = conn.execute(
            "SELECT sequence, title, start_byte, end_byte FROM chapters WHERE book_id = ? "
            "ORDER BY sequence",
            (book_id,),
        ).fetchall()
        db_titles = [row["title"] for row in rows]
        record(
            "2.chapters 表标题与夹具章节逐条一致",
            db_titles == expected_titles,
            f"db={db_titles[:3]}… 期望={expected_titles[:3]}…",
        )
        record(
            "2.章节区间单调且覆盖全文",
            all(
                rows[i]["end_byte"] == rows[i + 1]["start_byte"] for i in range(len(rows) - 1)
            )
            and rows[0]["start_byte"] == 0
            and rows[-1]["end_byte"] == before["size"],
            f"{rows[0]['start_byte']}..{rows[-1]['end_byte']} 共 {len(rows)} 章",
        )
        chapter_starts = {row["sequence"]: row["start_byte"] for row in rows}
        conn.close()

        # ---- 阶段 3：翻页（用户入口，含进度保存）----
        stage(3, "翻页", "NR_TURNS_UI=2 / -1（用户入口：翻页 + 保存进度）")
        forward = run_app(
            binary,
            db,
            {"NR_START_SCREEN": "reader", "NR_READER_BOOK": book_id, "NR_TURNS_UI": 2},
            display,
        )
        page_f2 = forward["page"]
        record("3.正向翻页退出码 0", forward["exit_code"] == 0, f"exit={forward['exit_code']}")
        record(
            "3.翻 2 页后停在第 3 页",
            page_f2 is not None and as_int(page_f2, "page_index") == 2,
            f"page_index={(page_f2 or {}).get('page_index')}",
        )
        progress_f = forward["progress_last"]
        record(
            "3.翻页后保存进度（changed=1）",
            progress_f is not None and progress_f["changed"] == "1",
            str(progress_f),
        )
        conn = db_connect(db)
        saved_row = conn.execute(
            "SELECT byte_offset, chapter_index, nearby_fingerprint, last_page_hint "
            "FROM reading_progress WHERE book_id = ?",
            (book_id,),
        ).fetchone()
        record(
            "3.库中进度偏移 = 该页起点（位置以 ContentPosition 为准）",
            saved_row is not None
            and page_f2 is not None
            and saved_row["byte_offset"] == as_int(page_f2, "start"),
            f"db={dict(saved_row) if saved_row else None} page_start={page_f2['start']}",
        )
        record(
            "3.进度不含页码身份（last_page_hint 为空）",
            saved_row is not None and saved_row["last_page_hint"] is None,
            f"last_page_hint={saved_row['last_page_hint'] if saved_row else None}",
        )
        record(
            "3.进度带锚点指纹",
            saved_row is not None
            and (saved_row["nearby_fingerprint"] or "") != "",
            f"quote={(saved_row['nearby_fingerprint'] or '')[:12]}…",
        )
        conn.close()

        backward = run_app(
            binary,
            db,
            {"NR_START_SCREEN": "reader", "NR_READER_BOOK": book_id, "NR_TURNS_UI": -1},
            display,
        )
        page_b1 = backward["page"]
        record("3.反向翻页退出码 0", backward["exit_code"] == 0, f"exit={backward['exit_code']}")
        record(
            "3.重启后从保存位置前进：恢复点击中上一页",
            page_b1 is not None
            and backward["pos_last"] is not None
            and backward["pos_last"]["restored"] == "1"
            and backward["pos_last"]["offset"] == page_f2["start"]
            and as_int(page_b1, "page_index") == 1,
            f"pos={backward['pos_last']} page_index={(page_b1 or {}).get('page_index')}",
        )

        # ---- 阶段 4：全局设置（全部设置项，含持久化）----
        stage(4, "设置", "NR_SETTINGS_SET → 全部全局设置项 + 落库 + 重排")
        settings_spec = (
            "font-size=26,line-spacing=6,theme=dark,interaction-mode=gesture,"
            "paragraph-spacing=8,first-line-indent=8,margin-x=24,margin-y=20"
        )
        settings_run = run_app(
            binary,
            db,
            {
                "NR_START_SCREEN": "reader",
                "NR_READER_BOOK": book_id,
                "NR_SETTINGS_SET": settings_spec,
            },
            display,
        )
        record("4.设置退出码 0", settings_run["exit_code"] == 0, f"exit={settings_run['exit_code']}")
        settings_log = settings_run["settings"][-1] if settings_run["settings"] else {}
        expected_settings = {
            "font_size_milli": "26000",
            "line_spacing_milli": "6000",
            "paragraph_spacing_milli": "8000",
            "first_line_indent_milli": "8000",
            "margin_x_milli": "24000",
            "margin_y_milli": "20000",
            "theme": "dark",
            "interaction": "gesture",
            "dark": "1",
        }
        record(
            "4.设置生效（NR-SETTINGS 全部字段）",
            all(settings_log.get(key) == value for key, value in expected_settings.items()),
            str({key: settings_log.get(key) for key in expected_settings}),
        )
        conn = db_connect(db)
        stored = conn.execute(
            "SELECT font_size_milli, line_spacing_milli, paragraph_spacing_milli, "
            "first_line_indent_milli, margin_x_milli, margin_y_milli, theme FROM reader_settings"
        ).fetchall()
        record(
            "4.设置落库（reader_settings 单行 = 全局设置）",
            len(stored) == 1
            and stored[0]["font_size_milli"] == 26000
            and stored[0]["line_spacing_milli"] == 6000
            and stored[0]["paragraph_spacing_milli"] == 8000
            and stored[0]["first_line_indent_milli"] == 8000
            and stored[0]["margin_x_milli"] == 24000
            and stored[0]["margin_y_milli"] == 20000
            and stored[0]["theme"] == "dark",
            str(dict(stored[0]) if stored else None),
        )
        mode_row = conn.execute(
            "SELECT value FROM app_meta WHERE key = 'ui.interaction_mode'"
        ).fetchone()
        record(
            "4.交互模式（手势/按钮）落库",
            mode_row is not None and mode_row["value"] == "gesture",
            str(dict(mode_row) if mode_row else None),
        )
        book_columns = conn.execute(
            "SELECT COUNT(*) AS n FROM pragma_table_info('reader_settings') WHERE name LIKE '%book%'"
        ).fetchone()["n"]
        record("4.设置表无按书字段（V1 只有全局设置）", book_columns == 0, f"book_columns={book_columns}")
        conn.close()
        page_s = settings_run["page"]
        record(
            "4.字号变化触发重排且正文区仍铺满",
            page_s is not None
            and as_int(page_s, "layout") != as_int(page0, "layout")
            and as_int(page_s, "lines") * as_int(page_s, "pitch") <= as_int(page_s, "vh")
            and as_int(page_s, "lines") * as_int(page_s, "pitch") + as_int(page_s, "pitch")
            > as_int(page_s, "vh"),
            f"layout {page0['layout']}→{(page_s or {}).get('layout')} "
            f"lines×pitch={(page_s or {}).get('lines')}×{(page_s or {}).get('pitch')} vh={(page_s or {}).get('vh')}",
        )
        record(
            "4.界面投影：手势模式 + 暗黑主题",
            settings_run["state"].get("gesture") == "1"
            and settings_run["state"].get("theme_dark") == "1",
            str({k: settings_run["state"].get(k) for k in ("gesture", "theme_dark")}),
        )

        # ---- 阶段 5：章节跳转 ----
        stage(5, "章节跳转", "NR_CHAPTER_JUMP=4 → 核心章节起始字节（不用页码）")
        target_sequence = 4
        expected_target = chapter_starts[target_sequence]
        jump_run = run_app(
            binary,
            db,
            {
                "NR_START_SCREEN": "reader",
                "NR_READER_BOOK": book_id,
                "NR_CHAPTER_JUMP": target_sequence,
            },
            display,
        )
        jump_log = jump_run["chapter_jumps"][-1] if jump_run["chapter_jumps"] else None
        record(
            "5.章节跳转落到核心章节起始字节",
            jump_log is not None and int(jump_log["target"]) == expected_target,
            f"log={jump_log} chapters.start_byte={expected_target}",
        )
        page_j = jump_run["page"]
        record(
            "5.跳转后页面从该章节起点开始",
            page_j is not None and as_int(page_j, "start") == expected_target,
            f"page_start={(page_j or {}).get('start')} target={expected_target}",
        )
        record(
            "5.当前章序号投影正确",
            jump_run["state"].get("chapter") == str(target_sequence),
            f"chapter={jump_run['state'].get('chapter')}",
        )
        record(
            "5.跳转页落在该章节区间内",
            page_j is not None
            and expected_target <= as_int(page_j, "start")
            and as_int(page_j, "end") <= chapter_starts[target_sequence + 1],
            f"{page_j['start']}..{page_j['end']} 章节={expected_target}.."
            f"{chapter_starts[target_sequence + 1]}",
        )

        # ---- 阶段 6：书签（新增 / 跳转 / 重命名 / 删除）----
        stage(6, "书签", "NR_MARK_ADD / MARK_JUMP / MARK_RENAME / MARK_DELETE（位置以 ContentPosition 为准）")
        add_run = run_app(
            binary,
            db,
            {
                "NR_START_SCREEN": "reader",
                "NR_READER_BOOK": book_id,
                "NR_MARK_ADD": "关键情节",
            },
            display,
        )
        add_log = add_run["marks"][-1] if add_run["marks"] else None
        record(
            "6.新增书签",
            add_log is not None and add_log["op"] == "add" and add_log["id"].isdigit(),
            str(add_log),
        )
        if add_log is None:
            return 1
        mark_id = int(add_log["id"])
        page_a = add_run["page"]
        conn = db_connect(db)
        mark_row = conn.execute(
            "SELECT id, book_id, title, chapter_index, byte_offset, nearby_fingerprint "
            "FROM bookmarks WHERE id = ?",
            (mark_id,),
        ).fetchone()
        record(
            "6.书签落库位置 = 当前页起点",
            mark_row is not None
            and page_a is not None
            and mark_row["byte_offset"] == as_int(page_a, "start")
            and mark_row["book_id"] == book_id,
            f"db={dict(mark_row) if mark_row else None}",
        )
        conn.close()

        # 先翻走再跳回，证明跳转真的由书签位置驱动。
        away = run_app(
            binary,
            db,
            {"NR_START_SCREEN": "reader", "NR_READER_BOOK": book_id, "NR_TURNS_UI": 2},
            display,
        )
        jump_mark = run_app(
            binary,
            db,
            {
                "NR_START_SCREEN": "reader",
                "NR_READER_BOOK": book_id,
                "NR_MARK_JUMP": mark_id,
            },
            display,
        )
        mark_jump_log = next(
            (item for item in jump_mark["marks"] if item["op"] == "jump"), None
        )
        page_m = jump_mark["page"]
        record(
            "6.书签跳转回到书签位置（与离开页不同）",
            mark_jump_log is not None
            and page_m is not None
            and away["page"] is not None
            and as_int(page_m, "start") == mark_row["byte_offset"]
            and as_int(page_m, "start") != as_int(away["page"], "start"),
            f"jump={mark_jump_log} page_start={(page_m or {}).get('start')} "
            f"离开页={(away['page'] or {}).get('start')}",
        )

        rename_run = run_app(
            binary,
            db,
            {
                "NR_START_SCREEN": "reader",
                "NR_READER_BOOK": book_id,
                "NR_MARK_RENAME": f"{mark_id}:伏笔",
            },
            display,
        )
        conn = db_connect(db)
        renamed = conn.execute(
            "SELECT title FROM bookmarks WHERE id = ?", (mark_id,)
        ).fetchone()
        record(
            "6.书签重命名落库",
            renamed is not None and renamed["title"] == "伏笔",
            f"title={renamed['title'] if renamed else None} exit={rename_run['exit_code']}",
        )
        conn.close()

        # 另建一个书签（跨重启保留证据）
        keep_run = run_app(
            binary,
            db,
            {
                "NR_START_SCREEN": "reader",
                "NR_READER_BOOK": book_id,
                "NR_MARK_ADD": "保留书签",
            },
            display,
        )
        keep_log = keep_run["marks"][-1] if keep_run["marks"] else None
        keep_id = int(keep_log["id"]) if keep_log and keep_log["id"].isdigit() else None
        record(
            "6.第二个书签新增（count 递增）",
            keep_log is not None and keep_log["op"] == "add" and keep_log["count"] == "2",
            str(keep_log),
        )

        delete_run = run_app(
            binary,
            db,
            {
                "NR_START_SCREEN": "reader",
                "NR_READER_BOOK": book_id,
                "NR_MARK_DELETE": mark_id,
            },
            display,
        )
        conn = db_connect(db)
        remaining = conn.execute(
            "SELECT id, title, byte_offset FROM bookmarks WHERE book_id = ? ORDER BY id",
            (book_id,),
        ).fetchall()
        record(
            "6.书签删除生效（库中只剩另一个书签）",
            len(remaining) == 1
            and (keep_id is None or remaining[0]["id"] == keep_id)
            and all(row["id"] != mark_id for row in remaining),
            f"remaining={[dict(row) for row in remaining]} exit={delete_run['exit_code']}",
        )
        record(
            "6.删除提示出现在日志",
            any("已删除书签" in status for status in delete_run["statuses"]),
            " | ".join(delete_run["statuses"])[-120:],
        )
        conn.close()

        # ---- 阶段 7：保存（用户入口翻页 + 进度保存）----
        stage(7, "保存", "NR_TURNS_UI=3 → 逐页保存进度（位置身份 = ContentPosition）")
        save_run = run_app(
            binary,
            db,
            {"NR_START_SCREEN": "reader", "NR_READER_BOOK": book_id, "NR_TURNS_UI": 3},
            display,
        )
        page_p = save_run["page"]
        record("7.退出码 0", save_run["exit_code"] == 0, f"exit={save_run['exit_code']}")
        record(
            "7.翻页 3 次且逐次保存进度",
            page_p is not None
            and as_int(page_p, "turns") >= 3
            and save_run["progress_last"] is not None
            and save_run["progress_last"]["changed"] == "1",
            f"turns={(page_p or {}).get('turns')} progress={save_run['progress_last']}",
        )
        conn = db_connect(db)
        final_row = conn.execute(
            "SELECT byte_offset, chapter_index FROM reading_progress WHERE book_id = ?",
            (book_id,),
        ).fetchone()
        record(
            "7.库中进度 = 末页起点",
            final_row is not None
            and page_p is not None
            and final_row["byte_offset"] == as_int(page_p, "start"),
            f"db={dict(final_row) if final_row else None} page_start={(page_p or {}).get('start')}",
        )
        conn.close()
        if page_p is None:
            return 1

        # ---- 阶段 8：重启（全新进程）→ 恢复 ----
        stage(8, "重启恢复", "全新进程、无任何驱动：读进度 → 核心锚点恢复 → 以该偏移重新测量")
        restore_run = run_app(
            binary,
            db,
            {"NR_START_SCREEN": "reader", "NR_READER_BOOK": book_id},
            display,
        )
        pos = restore_run["pos_last"]
        page_r = restore_run["page"]
        record("8.退出码 0", restore_run["exit_code"] == 0, f"exit={restore_run['exit_code']}")
        record(
            "8.恢复的位置 = 上一进程保存的偏移（锚点命中，未回退章节开头）",
            pos is not None
            and pos["restored"] == "1"
            and pos["fallback"] == "0"
            and pos["offset"] == str(as_int(page_p, "start")),
            f"pos={pos} 上一进程页起点={page_p['start']}",
        )
        record(
            "8.恢复后页面与上一进程末页逐项相同",
            page_r is not None
            and as_int(page_r, "start") == as_int(page_p, "start")
            and as_int(page_r, "end") == as_int(page_p, "end")
            and as_int(page_r, "lines") == as_int(page_p, "lines")
            and page_r["layout"] == page_p["layout"],
            f"restart start/end/lines={(page_r or {}).get('start')}/{(page_r or {}).get('end')}/"
            f"{(page_r or {}).get('lines')} 原={page_p['start']}/{page_p['end']}/{page_p['lines']}",
        )
        settings_after = restore_run["settings"][-1] if restore_run["settings"] else {}
        record(
            "8.全局设置跨重启保留（origin=stored）",
            settings_after.get("origin") == "stored"
            and settings_after.get("font_size_milli") == "26000"
            and settings_after.get("line_spacing_milli") == "6000"
            and settings_after.get("theme") == "dark"
            and settings_after.get("interaction") == "gesture",
            str(settings_after),
        )
        conn = db_connect(db)
        marks_after = conn.execute(
            "SELECT COUNT(*) AS n FROM bookmarks WHERE book_id = ?", (book_id,)
        ).fetchone()["n"]
        conn.close()
        record("8.书签跨重启保留", marks_after == 1, f"bookmarks={marks_after}")
        record(
            "8.无用户动作时不重复写进度（无多余 save）",
            not restore_run["progress"],
            f"progress_lines={restore_run['progress']} skipped={restore_run['progress_skipped']}",
        )
        record(
            "8.目录缓存命中（origin=cache，不重复扫描）",
            restore_run["toc_index"] and restore_run["toc_index"][-1]["origin"] == "cache",
            str(restore_run["toc_index"][-1] if restore_run["toc_index"] else None),
        )

        # ---- 阶段 9：只读性与数据边界 ----
        stage(9, "只读性与数据边界", "原文件不被修改；正文不入库")
        after = {"sha256": sha256_of(fixture), "size": fixture.stat().st_size}
        record("9.原文件哈希不变", after["sha256"] == before["sha256"], after["sha256"])
        record("9.原文件大小不变", after["size"] == before["size"], str(after["size"]))
        cells = all_text_cells(db)
        leaked = [
            sliced
            for sliced in body_slices
            if any(sliced in cell for cell in cells)
        ]
        record(
            "9.正文未入库（正文切片在全库文本列中零命中）",
            not leaked,
            f"检查切片 {len(body_slices)} 条；命中={leaked[:2]}",
        )
        conn = db_connect(db)
        tables = [
            row["name"]
            for row in conn.execute(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'"
            )
        ]
        forbidden_columns = []
        for table in tables:
            for column in conn.execute(f"SELECT name FROM pragma_table_info('{table}')"):
                if column["name"].lower() in {"content", "body", "text", "fulltext", "page_text"}:
                    forbidden_columns.append(f"{table}.{column['name']}")
        conn.close()
        record(
            "9.库中无正文字段（无 content/body/text 列）",
            not forbidden_columns,
            f"tables={sorted(tables)} 违规列={forbidden_columns}",
        )
    finally:
        shutil.rmtree(root, ignore_errors=True)

    print()
    print(f"阶段：{len(STAGES)} 个（" + " → ".join(item["name"] for item in STAGES) + "）")
    print(
        f"断言：{sum(1 for check in CHECKS if check['ok'])}/{len(CHECKS)} 通过；失败 {len(FAILURES)}"
    )
    print(f"真实启动次数：{len(LAUNCHES)}（每次都是独立进程）")
    if args.json:
        print(
            json.dumps(
                {
                    "platform": args.platform,
                    "stages": STAGES,
                    "checks": CHECKS,
                    "failures": FAILURES,
                    "launches": len(LAUNCHES),
                },
                ensure_ascii=False,
                indent=2,
            )
        )
    if FAILURES:
        print("RESULT: FAIL")
        for failure in FAILURES:
            print(f"  - {failure}")
        return 1
    print(
        "RESULT: PASS — 导入 → 阅读 → 翻页 → 设置 → 章节跳转 → 书签（增删改跳）→ 保存 → "
        "重启恢复全链路通过：恢复位置与保存位置一致、页面逐项相同、原文件只读且正文不入库"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

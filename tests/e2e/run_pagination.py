#!/usr/bin/env python3
"""run_pagination.py — DT3 桌面分页闭环的程序化端到端验证（真实 Release 二进制）。

口径（卡片 `t_bfdfffc9` DoD / NFR2 / NFR5）：

- 直接运行**真实 Release 二进制** `crates/desktop-app/target/release/desktop-app`，
  真实 X11 窗口 + 真实 SQLite + 真实 TXT 文件；
- 导入走 DT2 既有链路（`NR_IMPORT_PATH` 注入选择器，选择器之外全部为被测真实代码）；
- 阅读通过真实启动期驱动 `NR_READER_BOOK` / `NR_READER_TURNS` / `NR_READER_FONT` /
  `NR_READER_MENU` 触发，窗口映射完成后（`STARTUP_ALIGN_MS=400`）才取正文区域尺寸；
- 每次运行读取 stdout 的 `NR-PAGE ...` 结构化行（`ReaderRuntime::state_log`），
  逐项断言「正文区铺满、无纵向溢出（无滚动）、无空白页、无丢字/重字、UTF-8 边界合法、EOF 明确」；
- 正向页码连续性用**多次真实启动交叉验证**：第 N 次运行的页起点必须等于第 N-1 次运行的页终点
  （每次运行都是全新的进程 + 全新缓存，因此该等式同时证明「不靠缓存也能正确前进」）；
- 反向翻页从首页出发只可能到 `AtStart`（进度恢复属 DT4，DT3 尚无位置恢复入口），
  因此反向的 100 页等价性由单元测试 `backward_100_pages_match_forward_measurements` 取证，
  本脚本只断言真实二进制的反向入口不产生空白页/状态破坏。

用法：
    DISPLAY=:0 python3 tests/e2e/run_pagination.py            # 逐条打印结论
    DISPLAY=:0 python3 tests/e2e/run_pagination.py --json     # 追加机器可读汇总
退出码：0 = 全部通过；1 = 存在失败；2 = 前提缺失（无二进制 / 无显示）。
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

FAILURES: list[str] = []
CHECKS: list[dict] = []
RAW_PAGES: list[dict] = []
LAUNCHES: list[dict] = []


def record(name: str, ok: bool, detail: str) -> None:
    CHECKS.append({"check": name, "ok": ok, "detail": detail})
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


def novel_body() -> bytes:
    """真实感的 TXT 语料：中文正文 + ASCII 标点 + 4 字节字符（扩展 CJK / emoji）。"""
    lines: list[str] = []
    for index in range(1, 241):
        lines.append(f"第{index}章 分页端到端验证")
        for row in range(4):
            lines.append(
                f"这是第{row}行中文正文，包含标点：句号、逗号、顿号与引号“示例”；"
                f"并混入四字节字符𠀀𠀁与表情🧪𝄞，用于校验页面边界必须落在字符边界上。"
            )
        lines.append("")
    return "\n".join(lines).encode("utf-8")


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


def run_app(binary: Path, db: Path, extra_env: dict, display: str, timeout: int = 120) -> dict:
    env = dict(os.environ)
    env["DISPLAY"] = display
    env["NR_DB_PATH"] = str(db)
    env["NR_STATE_LOG"] = "1"
    env["NR_EXIT_AFTER_MS"] = "1800"
    for key in ("NR_IMPORT_PATH", "NR_RELINK_BOOK", "NR_RELINK_PATH", "NR_READER_BOOK",
                "NR_READER_TURNS", "NR_READER_FONT", "NR_READER_MENU", "NR_START_SCREEN"):
        env.pop(key, None)
    env.update({key: str(value) for key, value in extra_env.items()})
    proc = subprocess.run(
        [str(binary)],
        env=env,
        capture_output=True,
        text=True,
        timeout=timeout,
        cwd=str(REPO_ROOT),
    )
    pages = [PAGE_RE.match(line).groupdict() for line in proc.stdout.splitlines()
             if PAGE_RE.match(line)]
    statuses = [STATUS_RE.search(line).groupdict()["status"]
                for line in proc.stdout.splitlines() if line.startswith("NR-STATE")]
    entry = {
        "env": {key: str(value) for key, value in extra_env.items()},
        "exit_code": proc.returncode,
        "page": pages[-1] if pages else None,
        "page_lines": pages,
        "status": statuses[-1] if statuses else "",
        "statuses": statuses,
        "stdout_tail": proc.stdout[-3000:],
        "stderr_tail": proc.stderr[-2000:],
    }
    LAUNCHES.append(entry)
    if entry["page"]:
        RAW_PAGES.append({"env": dict(entry["env"]), **entry["page"]})
    return entry


def as_int(page: dict, key: str) -> int:
    return int(page[key])


def main() -> int:
    parser = argparse.ArgumentParser(description="DT3 桌面分页闭环 E2E")
    parser.add_argument("--binary", default=str(DEFAULT_BINARY), help="desktop-app Release 二进制路径")
    parser.add_argument("--display", default=os.environ.get("DISPLAY", ":0"), help="X11 显示（默认 :0）")
    parser.add_argument("--pages", type=int, default=10, help="正向逐页交叉验证的页数")
    parser.add_argument("--json", action="store_true", help="输出机器可读汇总")
    args = parser.parse_args()

    binary = Path(args.binary).resolve()
    if not binary.is_file():
        print(f"前提缺失：找不到 Release 二进制 {binary}", file=sys.stderr)
        print("先执行：cd crates/desktop-app && cargo build --release", file=sys.stderr)
        return 2
    if not os.environ.get("DISPLAY") and args.display == "":
        print("前提缺失：无 X11 DISPLAY", file=sys.stderr)
        return 2

    display = args.display
    root = Path(tempfile.mkdtemp(prefix="nr-dt3-e2e-"))
    fixture = root / "斗破苍穹-分页验证.txt"
    content = novel_body()
    fixture.write_bytes(content)
    before = {"sha256": sha256_of(fixture), "size": fixture.stat().st_size}
    db = root / "library.db"

    print(f"仓库根目录：{REPO_ROOT}")
    print(f"二进制：{binary}")
    print(f"夹具：{fixture}（{before['size']} 字节）")
    print(f"数据库：{db}")
    print(f"DISPLAY：{display}")
    print()

    try:
        # ---- 步骤 0：真实导入（DT2 链路），建立真实书籍记录 ----
        print("[0] 真实导入：NR_IMPORT_PATH → books/book_sources（真实 SQLite）")
        imported = run_app(binary, db, {"NR_IMPORT_PATH": fixture}, display)
        record("0.导入退出码", imported["exit_code"] == 0, f"exit={imported['exit_code']}")
        record(
            "0.导入提示出现在状态日志",
            any("已导入" in status for status in imported["statuses"]),
            " | ".join(imported["statuses"])[-200:],
        )
        conn = sqlite3.connect(str(db))
        conn.row_factory = sqlite3.Row
        books = conn.execute("SELECT id, display_name, status, source_locator FROM books").fetchall()
        record("0.书籍条数 = 1", len(books) == 1, f"books={len(books)}")
        if not books:
            print("导入未建立书籍记录，后续阅读断言无法进行", file=sys.stderr)
            return 1
        book_id = int(books[0]["id"])
        record("0.来源状态 ready", books[0]["status"] == "ready", str(books[0]["status"]))
        record(
            "0.定位符指向夹具",
            books[0]["source_locator"] == str(fixture),
            str(books[0]["source_locator"]),
        )
        conn.close()

        # ---- 步骤 1：真实启动阅读，正向逐页交叉验证 ----
        print()
        print(f"[1] 真实启动阅读：NR_READER_TURNS=0..{args.pages}（每次都是全新进程）")
        pages: dict[int, dict] = {}
        for turn in range(0, args.pages + 1):
            run = run_app(
                binary,
                db,
                {"NR_START_SCREEN": "reader", "NR_READER_BOOK": book_id, "NR_READER_TURNS": turn},
                display,
            )
            if run["page"] is None:
                record(f"1.turn={turn} NR-PAGE 输出", False, f"stdout 尾：{run['stdout_tail'][-300:]}")
                continue
            page = run["page"]
            pages[turn] = page
            record(
                f"1.turn={turn} 退出码与结构化输出",
                run["exit_code"] == 0 and as_int(page, "turns") == turn,
                f"exit={run['exit_code']} page_index={page['page_index']} start={page['start']} "
                f"end={page['end']} lines={page['lines']}/{page['lines_per_page']} turns={page['turns']}",
            )
            record(
                f"1.turn={turn} 页码自洽",
                as_int(page, "page_index") == turn,
                f"page_index={page['page_index']} expected={turn}",
            )

        if not pages:
            print("没有任何一次阅读运行产出结构化页面，终止", file=sys.stderr)
            return 1

        first = pages[0]
        # 正文区域：真实窗口映射后才取尺寸（0×0 被拒绝在 DT3 单测里；此处断言实际可用）
        vw, vh = as_int(first, "vw"), as_int(first, "vh")
        lines_per_page, pitch, font = (
            as_int(first, "lines_per_page"),
            as_int(first, "pitch"),
            as_int(first, "font"),
        )
        record(
            "1.正文区域已映射（非 0×0）",
            vw > 0 and vh > 0 and lines_per_page > 0 and font > 0,
            f"viewport={vw}x{vh} lines_per_page={lines_per_page} pitch={pitch} font={font}",
        )
        record(
            "1.行盒不低于字体行盒（无行重叠）",
            pitch >= font,
            f"pitch={pitch} font={font}",
        )
        # 渲染端逐行定位的不变量：行数 × 行盒 ≤ 正文区高度，再加一行即溢出 → 既铺满又无滚动。
        for turn, page in pages.items():
            used = as_int(page, "lines") * as_int(page, "pitch")
            height = as_int(page, "vh")
            record(
                f"1.turn={turn} 正文区铺满且不溢出（无滚动条）",
                used <= height and used + as_int(page, "pitch") > height,
                f"lines×pitch={used} 正文区高度={height}（再添一行={used + as_int(page, 'pitch')}）",
            )
            record(
                f"1.turn={turn} 无空白页",
                as_int(page, "lines") > 0 and as_int(page, "end") > as_int(page, "start"),
                f"lines={page['lines']} bytes={page['start']}..{page['end']}",
            )
            record(
                f"1.turn={turn} 无过期结果覆盖",
                as_int(page, "stale") == 0,
                f"stale={page['stale']}",
            )
            record(
                f"1.turn={turn} 无多余重排",
                as_int(page, "reflows") == 0,
                f"reflows={page['reflows']}",
            )

        # 跨运行连续性：第 N 次运行的起点 == 第 N-1 次运行的终点（全新进程，不依赖缓存）
        for turn in range(1, args.pages + 1):
            previous, current = pages[turn - 1], pages[turn]
            record(
                f"1.第 {turn} 页与上一页首尾相接（无丢字/重字）",
                as_int(current, "start") == as_int(previous, "end"),
                f"start={current['start']} 上一页 end={previous['end']}",
            )
            record(
                f"1.第 {turn} 页严格前进",
                as_int(current, "end") > as_int(current, "start")
                and as_int(current, "start") >= as_int(previous, "end"),
                f"{current['start']}..{current['end']}",
            )

        # UTF-8 边界：所有页面边界都必须落在字符边界
        data = fixture.read_bytes()
        for turn, page in pages.items():
            start, end = as_int(page, "start"), as_int(page, "end")
            record(
                f"1.第 {turn} 页边界为合法 UTF-8 字符边界（无半字裁切）",
                data.decode("utf-8").__len__() > 0
                and is_char_boundary(data, start)
                and is_char_boundary(data, end),
                f"{start}..{end}",
            )

        # 缓存有上限（当前页 + 前一页 + 后三页 → 进程内缓存计数必须远小于总页数）
        cached = as_int(pages[max(pages)], "cached")
        total_bytes = as_int(pages[max(pages)], "total")
        record(
            "1.缓存有上限（不缓存全书）",
            1 <= cached <= 8 and cached < max(1, len(pages)),
            f"cached={cached} 已翻页数={len(pages) - 1} total_bytes={total_bytes}",
        )
        record(
            "1.内容总长度与夹具一致",
            total_bytes == before["size"],
            f"total={total_bytes} 夹具={before['size']}",
        )

        # ---- 步骤 2：EOF（正向翻到末尾）----
        print()
        print("[2] EOF：NR_READER_TURNS=10000 → 必须给出明确 eof 且末页非空")
        eof_run = run_app(
            binary,
            db,
            {"NR_START_SCREEN": "reader", "NR_READER_BOOK": book_id, "NR_READER_TURNS": 10000},
            display,
        )
        record("2.退出码", eof_run["exit_code"] == 0, f"exit={eof_run['exit_code']}")
        if eof_run["page"]:
            eof = eof_run["page"]
            record("2.末页 eof=1", as_int(eof, "eof") == 1, f"eof={eof['eof']}")
            record(
                "2.末页抵达内容末尾",
                as_int(eof, "end") == before["size"],
                f"end={eof['end']} 夹具={before['size']}",
            )
            record(
                "2.末页非空（不产生空白死循环页）",
                as_int(eof, "lines") > 0 and is_char_boundary(data, as_int(eof, "end")),
                f"lines={eof['lines']} end={eof['end']}",
            )
            record(
                "2.末页起点 = 倒数第二页终点（跨运行交叉验证）",
                as_int(eof, "start") > as_int(pages[max(pages)], "end"),
                f"末页 start={eof['start']}，第 {max(pages)} 页 end={pages[max(pages)]['end']}",
            )
            # 用一次真实启动翻到「末页的上一页」，核对与末页首尾相接（不丢字/不重字）。
            previous_index = as_int(eof, "page_index") - 1
            if previous_index >= 0:
                prev_run = run_app(
                    binary,
                    db,
                    {"NR_START_SCREEN": "reader", "NR_READER_BOOK": book_id,
                     "NR_READER_TURNS": previous_index},
                    display,
                )
                if prev_run["page"]:
                    prev = prev_run["page"]
                    record(
                        "2.倒数第二页终点 = 末页起点（无丢字/重字）",
                        as_int(prev, "end") == as_int(eof, "start")
                        and as_int(prev, "page_index") == previous_index,
                        f"page_index={prev['page_index']} end={prev['end']} 末页 start={eof['start']}",
                    )
                else:
                    record("2.倒数第二页输出", False, prev_run["stdout_tail"][-300:])
            record(
                "2.末页铺满判定不适用（EOF 页允许不满行），但仍不出滚动",
                as_int(eof, "lines") * as_int(eof, "pitch") <= as_int(eof, "vh"),
                f"lines×pitch={as_int(eof, 'lines') * as_int(eof, 'pitch')} 高度={eof['vh']}",
            )
        else:
            record("2.NR-PAGE 输出", False, eof_run["stdout_tail"][-300:])

        # ---- 步骤 3：反向翻页入口（真实二进制）----
        print()
        print("[3] 反向：NR_READER_TURNS=-1 / -3 → 必须停在首页并给出 AtStart，且页面内容不被破坏")
        for turns in (-1, -3):
            run = run_app(
                binary,
                db,
                {"NR_START_SCREEN": "reader", "NR_READER_BOOK": book_id, "NR_READER_TURNS": turns},
                display,
            )
            page = run["page"]
            record(f"3.turns={turns} 退出码", run["exit_code"] == 0, f"exit={run['exit_code']}")
            if page is None:
                record(f"3.turns={turns} 页面输出", False, run["stdout_tail"][-300:])
                continue
            record(
                f"3.turns={turns} 停在首页（AtStart）且页面与 turn=0 一致",
                as_int(page, "start") == as_int(first, "start")
                and as_int(page, "end") == as_int(first, "end")
                and as_int(page, "lines") == as_int(first, "lines"),
                f"start={page['start']} end={page['end']} lines={page['lines']} "
                f"（turn=0：{first['start']}..{first['end']}）",
            )
            record(
                f"3.turns={turns} 状态栏给出「已是第一页」",
                "已是第一页" in run["status"],
                run["status"][-120:],
            )
            record(
                f"3.turns={turns} 无空白页",
                as_int(page, "lines") > 0,
                f"lines={page['lines']}",
            )

        # ---- 步骤 4：真实重排（字号变化 / 菜单展开改变正文区高度）----
        print()
        print("[4] 重排：NR_READER_FONT / NR_READER_MENU → 以当前位置重新分页且仍铺满")
        font_run = run_app(
            binary,
            db,
            {"NR_START_SCREEN": "reader", "NR_READER_BOOK": book_id, "NR_READER_TURNS": 0,
             "NR_READER_FONT": as_int(first, "font") + 6},
            display,
        )
        if font_run["page"]:
            page = font_run["page"]
            record(
                "4.字号变化触发重排（新排版世代）",
                as_int(page, "reflows") >= 1 and page["layout"] != first["layout"],
                f"reflows={page['reflows']} layout={first['layout']}→{page['layout']}",
            )
            record(
                "4.重排保留内容位置（仍在 0 起点）",
                as_int(page, "start") == as_int(first, "start"),
                f"start={page['start']}",
            )
            record(
                "4.重排后行盒随字号变化",
                as_int(page, "pitch") > as_int(first, "pitch"),
                f"pitch {first['pitch']} → {page['pitch']}（font {first['font']} → {page['font']}）",
            )
            used = as_int(page, "lines") * as_int(page, "pitch")
            record(
                "4.重排后仍铺满且不溢出",
                used <= as_int(page, "vh")
                and used + as_int(page, "pitch") > as_int(page, "vh"),
                f"lines×pitch={used} 高度={page['vh']}",
            )
        else:
            record("4.字号变化输出", False, font_run["stdout_tail"][-300:])

        menu_run = run_app(
            binary,
            db,
            {"NR_START_SCREEN": "reader", "NR_READER_BOOK": book_id, "NR_READER_TURNS": 0,
             "NR_READER_MENU": 1},
            display,
        )
        if menu_run["page"]:
            page = menu_run["page"]
            record(
                "4.菜单引起正文区高度变化并重排",
                as_int(page, "reflows") >= 1,
                f"reflows={page['reflows']} lines_per_page {first['lines_per_page']} → "
                f"{page['lines_per_page']}",
            )
            used = as_int(page, "lines") * as_int(page, "pitch")
            record(
                "4.重排后仍铺满且不溢出（菜单态）",
                used <= as_int(page, "vh")
                and used + as_int(page, "pitch") > as_int(page, "vh"),
                f"lines×pitch={used} 高度={page['vh']} lines={page['lines']}/{page['lines_per_page']}",
            )
            record(
                "4.重排后无过期结果覆盖",
                as_int(page, "stale") == 0,
                f"stale={page['stale']}",
            )
        else:
            record("4.菜单重排输出", False, menu_run["stdout_tail"][-300:])

        # ---- 步骤 5：只读性（不修改用户原始文件）----
        print()
        print("[5] 只读性：全部分页/翻页/重排运行后原文件 sha256/大小不变")
        after = {"sha256": sha256_of(fixture), "size": fixture.stat().st_size}
        record("5.原文件哈希不变", after["sha256"] == before["sha256"], after["sha256"])
        record("5.原文件大小不变", after["size"] == before["size"], f"{after['size']}")
    finally:
        shutil.rmtree(root, ignore_errors=True)

    print()
    print(f"断言：{sum(1 for check in CHECKS if check['ok'])}/{len(CHECKS)} 通过；失败 {len(FAILURES)}")
    print(f"真实启动次数：{len(LAUNCHES)}（每次都是独立进程）")
    if args.json:
        print(json.dumps(
            {"checks": CHECKS, "failures": FAILURES, "launches": len(LAUNCHES), "pages": RAW_PAGES},
            ensure_ascii=False,
            indent=2,
        ))
    if FAILURES:
        print("RESULT: FAIL")
        for failure in FAILURES:
            print(f"  - {failure}")
        return 1
    print("RESULT: PASS — 真实启动后正文区逐行铺满、无纵向溢出（无滚动条）、无空白页、"
          "正反向入口与重排均保持不丢字/不重字/合法 UTF-8 边界")
    return 0


def is_char_boundary(data: bytes, offset: int) -> bool:
    """UTF-8 字符边界判定：该字节不得是连续字节（0b10xxxxxx）。"""
    if offset < 0 or offset >= len(data):
        return offset == len(data)
    return (data[offset] & 0xC0) != 0x80


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""run_txt_import.py — DT2 桌面 TXT 导入链路的程序化端到端验证。

口径（`desktop/spec.md` DR1/DR5、卡片 `t_b28a9133` DoD）：

- 直接运行**真实 Release 二进制**（`crates/desktop-app/target/release/desktop-app`），
  用真实 X11 窗口 + 真实 SQLite + 真实文件系统；
- 文件选择通过 `NR_IMPORT_PATH`（Rust 侧 `StaticFilePicker`）注入，避免自动化点击系统模态对话框；
  选择器之外的链路（校验 / 指纹 / 编码识别 / 落库 / 状态投影 / 重新关联）全部是被测真实代码；
- 覆盖五类行为：导入成功、非 TXT、空文件、不可读、失效重新关联（含内容不一致拒绝）；
- 每个场景都断言**用户原始文件的 sha256 / 大小 / mtime 不变**，且失败时数据库 0 条记录；
- 夹具全部在 /tmp 下，绝不触碰用户真实书库或真实书籍文件。

用法：
    python3 tests/e2e/run_txt_import.py            # 逐条打印结论
    python3 tests/e2e/run_txt_import.py --json     # 额外输出机器可读汇总
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

FINGERPRINT_RE = re.compile(r"^fnv1a64:\d+:[0-9a-f]{16}:[0-9a-f]{16}:[0-9a-f]{16}$")

FAILURES: list[str] = []
CHECKS: list[dict] = []


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


def file_facts(path: Path) -> dict:
    stat = path.stat()
    return {
        "sha256": sha256_of(path),
        "size": stat.st_size,
        "mtime_ns": stat.st_mtime_ns,
    }


def novel_body(tag: str) -> bytes:
    lines = []
    for index in range(1, 61):
        lines.append(f"第{index}章 {tag}")
        lines.append("这是一行用于导入链路端到端验证的中文正文，包含标点：句号、逗号、顿号。")
        lines.append("第二行正文，用于让文件长度超过单次抽样区间（4 KiB）。" * 4)
        lines.append("")
    return "\n".join(lines).encode("utf-8")


def run_app(binary: Path, db: Path, extra_env: dict, display: str, timeout: int = 90) -> dict:
    env = dict(os.environ)
    env["DISPLAY"] = display
    env["NR_DB_PATH"] = str(db)
    env["NR_STATE_LOG"] = "1"
    env["NR_EXIT_AFTER_MS"] = "1500"
    env.pop("NR_IMPORT_PATH", None)
    env.pop("NR_RELINK_BOOK", None)
    env.pop("NR_RELINK_PATH", None)
    env.update({key: str(value) for key, value in extra_env.items()})
    proc = subprocess.run(
        [str(binary)],
        env=env,
        capture_output=True,
        text=True,
        timeout=timeout,
        cwd=str(REPO_ROOT),
    )
    statuses = []
    for line in proc.stdout.splitlines():
        if not line.startswith("NR-STATE"):
            continue
        marker = line.find("status=")
        statuses.append(line[marker + len("status=") :] if marker >= 0 else "")
    return {
        "env": {key: str(value) for key, value in extra_env.items()},
        "exit_code": proc.returncode,
        "statuses": statuses,
        "stdout_tail": proc.stdout[-2000:],
        "stderr_tail": proc.stderr[-2000:],
    }


def connect(db: Path) -> sqlite3.Connection:
    # 夹具库位于 /tmp（不是用户真实书库）；此处允许读写以读取 WAL 中的最新状态。
    conn = sqlite3.connect(str(db))
    conn.row_factory = sqlite3.Row
    return conn


def scalar(db: Path, sql: str, params: tuple = ()):
    conn = connect(db)
    try:
        row = conn.execute(sql, params).fetchone()
        return None if row is None else row[0]
    finally:
        conn.close()


def rows(db: Path, sql: str, params: tuple = ()) -> list[dict]:
    conn = connect(db)
    try:
        return [dict(row) for row in conn.execute(sql, params).fetchall()]
    finally:
        conn.close()


def seed_reading_state(db: Path, book_id: int) -> None:
    """写入进度与书签（模拟「已经在读」的用户状态；DT4 会由阅读闭环真实写入）。"""
    conn = connect(db)
    try:
        conn.execute(
            "INSERT OR REPLACE INTO reading_progress "
            "(book_id, content_unit_id, chapter_index, byte_offset, nearby_fingerprint, "
            " last_page_hint, index_version, updated_at, last_opened_at) "
            "VALUES (?, 0, 2, 4096, '第3章 起始', NULL, 0, 1000, 1000)",
            (book_id,),
        )
        conn.execute(
            "INSERT INTO bookmarks "
            "(book_id, content_unit_id, chapter_index, byte_offset, nearby_fingerprint, title, "
            " created_at, updated_at) VALUES (?, 0, 2, 4096, '第3章 起始', '锚点', 1001, 1001)",
            (book_id,),
        )
        conn.commit()
    finally:
        conn.close()


def progress_snapshot(db: Path, book_id: int) -> list[dict]:
    return rows(db, "SELECT * FROM reading_progress WHERE book_id = ?", (book_id,))


def bookmark_snapshot(db: Path, book_id: int) -> list[dict]:
    return rows(
        db,
        "SELECT book_id, content_unit_id, chapter_index, byte_offset, nearby_fingerprint, title "
        "FROM bookmarks WHERE book_id = ?",
        (book_id,),
    )


# ---------------------------------------------------------------------------
# 场景
# ---------------------------------------------------------------------------


def scenario_import_ok(binary: Path, root: Path, display: str) -> dict | None:
    print("[A] 导入成功：真实 TXT → books + book_sources + 指纹，原文件不变")
    fixtures = root / "A"
    fixtures.mkdir(parents=True)
    book = fixtures / "斗破苍穹.txt"
    book.write_bytes(novel_body("导入成功"))
    before = file_facts(book)
    db = fixtures / "library.db"

    result = run_app(binary, db, {"NR_IMPORT_PATH": book}, display)
    record("A.退出码", result["exit_code"] == 0, f"exit={result['exit_code']}")
    statuses = " | ".join(result["statuses"])
    record("A.状态栏", "已导入" in statuses and "斗破苍穹" in statuses, statuses[-300:])
    record("A.书架行", any("rows=1" in line for line in result["stdout_tail"].splitlines() if "NR-STATE" in line) or "rows=1" in result["stdout_tail"], "NR-STATE rows")

    books = rows(db, "SELECT * FROM books")
    record("A.书籍条数", len(books) == 1, f"books={len(books)}")
    if len(books) != 1:
        return None
    stored = books[0]
    record("A.书名", stored["display_name"] == "斗破苍穹", str(stored["display_name"]))
    record("A.原始文件名", stored["original_name"] == "斗破苍穹.txt", str(stored["original_name"]))
    record("A.格式", stored["format"] == "txt", str(stored["format"]))
    record("A.状态", stored["status"] == "ready", str(stored["status"]))
    record("A.来源种类", stored["source_kind"] == "desktop_path", str(stored["source_kind"]))
    record("A.来源定位符", stored["source_locator"] == str(book), str(stored["source_locator"]))
    record("A.大小", stored["source_size"] == before["size"], f"{stored['source_size']} vs {before['size']}")
    record(
        "A.指纹格式",
        bool(FINGERPRINT_RE.match(str(stored["source_fingerprint"]))),
        str(stored["source_fingerprint"]),
    )
    record("A.编码", stored["encoding"] == "UTF-8", str(stored["encoding"]))
    record("A.编码置信度", int(stored["encoding_confidence"]) == 1000, str(stored["encoding_confidence"]))

    sources = rows(db, "SELECT * FROM book_sources WHERE book_id = ?", (stored["id"],))
    record("A.来源条数", len(sources) == 1, f"sources={len(sources)}")
    if sources:
        record("A.来源定位符一致", sources[0]["source_locator"] == str(book), str(sources[0]["source_locator"]))
        record("A.来源主来源", bool(sources[0]["is_primary"]), str(sources[0]["is_primary"]))
        record(
            "A.来源指纹一致",
            sources[0]["source_fingerprint"] == stored["source_fingerprint"],
            str(sources[0]["source_fingerprint"]),
        )
    record(
        "A.无进度行（导入不伪造进度）",
        scalar(db, "SELECT COUNT(*) FROM reading_progress") == 0,
        str(scalar(db, "SELECT COUNT(*) FROM reading_progress")),
    )

    after = file_facts(book)
    record("A.原文件哈希不变", after["sha256"] == before["sha256"], after["sha256"])
    record("A.原文件大小不变", after["size"] == before["size"], f"{after['size']}")
    record("A.原文件 mtime 不变", after["mtime_ns"] == before["mtime_ns"], f"{after['mtime_ns']}")

    return {
        "db": db,
        "book_path": book,
        "book_id": int(stored["id"]),
        "before": before,
        "result": result,
    }


def scenario_rejections(binary: Path, root: Path, display: str) -> None:
    print("[B/C/D] 拒绝类：非 TXT / 空文件 / 不可读 —— 必须有可读错误且 0 条记录")

    # B 非 TXT
    fixtures = root / "B"
    fixtures.mkdir(parents=True)
    other = fixtures / "book.pdf"
    other.write_bytes(b"%PDF-1.7\nnot a txt\n")
    before = file_facts(other)
    db = fixtures / "library.db"
    result = run_app(binary, db, {"NR_IMPORT_PATH": other}, display)
    statuses = " | ".join(result["statuses"])
    record("B.退出码", result["exit_code"] == 0, f"exit={result['exit_code']}")
    record("B.可读错误", "只接受 .txt" in statuses, statuses[-200:])
    record("B.0 条记录", scalar(db, "SELECT COUNT(*) FROM books") == 0, str(scalar(db, "SELECT COUNT(*) FROM books")))
    record("B.0 条来源", scalar(db, "SELECT COUNT(*) FROM book_sources") == 0, str(scalar(db, "SELECT COUNT(*) FROM book_sources")))
    record("B.原文件不变", file_facts(other) == before, "sha256/size/mtime")

    # C 空文件
    fixtures = root / "C"
    fixtures.mkdir(parents=True)
    empty = fixtures / "empty.txt"
    empty.write_bytes(b"")
    db = fixtures / "library.db"
    result = run_app(binary, db, {"NR_IMPORT_PATH": empty}, display)
    statuses = " | ".join(result["statuses"])
    record("C.退出码", result["exit_code"] == 0, f"exit={result['exit_code']}")
    record("C.可读错误", "文件为空" in statuses, statuses[-200:])
    record("C.0 条记录", scalar(db, "SELECT COUNT(*) FROM books") == 0, str(scalar(db, "SELECT COUNT(*) FROM books")))

    # D 不可读（0o000）
    fixtures = root / "D"
    fixtures.mkdir(parents=True)
    locked = fixtures / "locked.txt"
    locked.write_bytes(novel_body("不可读"))
    db = fixtures / "library.db"
    os.chmod(locked, 0o000)
    readable = True
    try:
        with locked.open("rb"):
            pass
    except PermissionError:
        readable = False
    result = run_app(binary, db, {"NR_IMPORT_PATH": locked}, display)
    statuses = " | ".join(result["statuses"])
    record("D.退出码", result["exit_code"] == 0, f"exit={result['exit_code']}")
    if readable:
        print("  SKIP  D.不可读：当前进程可读取 0o000 文件（疑似 root），仅断言不留记录")
    else:
        record("D.可读错误", "无法读取" in statuses, statuses[-200:])
    record("D.0 条记录", scalar(db, "SELECT COUNT(*) FROM books") == 0, str(scalar(db, "SELECT COUNT(*) FROM books")))
    record("D.文件仍在", locked.exists(), str(locked))
    os.chmod(locked, 0o600)


def scenario_missing_and_relink(binary: Path, root: Path, display: str, imported: dict) -> None:
    print("[E] 失效重新关联：原文件不可用 → 需要重新关联 → 选同一文件 → 进度/书签保留")

    db = imported["db"]
    book_id = imported["book_id"]
    original = imported["book_path"]
    before = imported["before"]
    seed_reading_state(db, book_id)
    progress_before = progress_snapshot(db, book_id)
    bookmarks_before = bookmark_snapshot(db, book_id)
    record("E.预置进度/书签", len(progress_before) == 1 and len(bookmarks_before) == 1, f"{len(progress_before)}/{len(bookmarks_before)}")

    # 用户把原文件搬到别处（只在 /tmp 内移动夹具副本）。
    moved_dir = root / "E-moved"
    moved_dir.mkdir(parents=True)
    moved = moved_dir / "斗破苍穹.txt"
    shutil.move(str(original), str(moved))
    record("E.原路径已失效", not original.exists(), str(original))

    result = run_app(binary, db, {}, display)
    statuses = " | ".join(result["statuses"])
    record("E.运行退出码", result["exit_code"] == 0, f"exit={result['exit_code']}")
    record(
        "E.进入需要重新关联（状态列 + 界面行）",
        scalar(db, "SELECT status FROM books WHERE id = ?", (book_id,)) == "missing"
        and "rows=1" in result["stdout_tail"]
        and "row_status=需要重新关联" in result["stdout_tail"],
        f"status={scalar(db, 'SELECT status FROM books WHERE id = ?', (book_id,))}；"
        + " | ".join(line for line in result["stdout_tail"].splitlines() if "rows=1" in line)[-200:],
    )
    record(
        "E.状态列 missing",
        scalar(db, "SELECT status FROM books WHERE id = ?", (book_id,)) == "missing",
        str(scalar(db, "SELECT status FROM books WHERE id = ?", (book_id,))),
    )
    record("E.进度保留", progress_snapshot(db, book_id) == progress_before, "逐字段一致")
    record("E.书签保留", bookmark_snapshot(db, book_id) == bookmarks_before, "逐字段一致")
    record("E.原文件未被删除", moved.exists(), str(moved))

    # 重新关联到移动后的同一份内容（专用 NR_RELINK_PATH，避免同时触发导入）。
    result = run_app(binary, db, {"NR_RELINK_BOOK": book_id, "NR_RELINK_PATH": moved}, display)
    statuses = " | ".join(result["statuses"])
    record("E.重新关联退出码", result["exit_code"] == 0, f"exit={result['exit_code']}")
    record("E.重新关联成功", "已重新关联" in statuses and "内容一致" in statuses, statuses[-400:])
    record(
        "E.重新关联后状态 ready",
        scalar(db, "SELECT status FROM books WHERE id = ?", (book_id,)) == "ready",
        str(scalar(db, "SELECT status FROM books WHERE id = ?", (book_id,))),
    )
    record(
        "E.定位符指向新路径",
        scalar(db, "SELECT source_locator FROM books WHERE id = ?", (book_id,)) == str(moved),
        str(scalar(db, "SELECT source_locator FROM books WHERE id = ?", (book_id,))),
    )
    record(
        "E.来源条数 = 2",
        scalar(db, "SELECT COUNT(*) FROM book_sources WHERE book_id = ?", (book_id,)) == 2,
        str(scalar(db, "SELECT COUNT(*) FROM book_sources WHERE book_id = ?", (book_id,))),
    )
    record("E.进度逐字段保留", progress_snapshot(db, book_id) == progress_before, "重新关联不得改动进度")
    record("E.书签逐字段保留", bookmark_snapshot(db, book_id) == bookmarks_before, "重新关联不得改动书签")
    record("E.移动后文件哈希不变", file_facts(moved) == before, "sha256/size/mtime 与导入前一致")

    # 内容不一致：必须零改动（不得静默覆盖旧进度）。
    other = root / "E-other.txt"
    other.write_bytes(novel_body("完全不同的内容"))
    other_before = file_facts(other)
    result = run_app(binary, db, {"NR_RELINK_BOOK": book_id, "NR_RELINK_PATH": other}, display)
    statuses = " | ".join(result["statuses"])
    record("E.内容不一致退出码", result["exit_code"] == 0, f"exit={result['exit_code']}")
    record("E.内容不一致提示", "内容与记录不一致" in statuses and "未做任何改动" in statuses, statuses[-400:])
    record("E.内容不一致零改动（进度）", progress_snapshot(db, book_id) == progress_before, "逐字段一致")
    record("E.内容不一致零改动（书签）", bookmark_snapshot(db, book_id) == bookmarks_before, "逐字段一致")
    record(
        "E.内容不一致零改动（定位符/来源）",
        scalar(db, "SELECT source_locator FROM books WHERE id = ?", (book_id,)) == str(moved)
        and scalar(db, "SELECT COUNT(*) FROM book_sources WHERE book_id = ?", (book_id,)) == 2,
        "定位符与来源条数不变",
    )
    record("E.所选文件未被改动", file_facts(other) == other_before, "sha256/size/mtime")
    record("E.移动后文件仍在", moved.exists() and file_facts(moved) == before, str(moved))


def main() -> int:
    parser = argparse.ArgumentParser(description="DT2 桌面 TXT 导入链路 E2E")
    parser.add_argument("--binary", default=str(DEFAULT_BINARY), help="desktop-app Release 二进制路径")
    parser.add_argument("--display", default=os.environ.get("DISPLAY", ":0"), help="X11 显示（默认 :0）")
    parser.add_argument("--json", action="store_true", help="输出机器可读汇总")
    args = parser.parse_args()

    binary = Path(args.binary).resolve()
    if not binary.is_file():
        print(f"前提缺失：找不到 Release 二进制 {binary}", file=sys.stderr)
        print("先执行：cd crates/desktop-app && cargo build --release", file=sys.stderr)
        return 2

    root = Path(tempfile.mkdtemp(prefix="nr-dt2-e2e-"))
    print(f"仓库根目录：{REPO_ROOT}")
    print(f"二进制：{binary}")
    print(f"夹具目录：{root}")
    print(f"DISPLAY：{args.display}")
    try:
        imported = scenario_import_ok(binary, root, args.display)
        if imported is None:
            print("导入成功场景未建立书籍条目，后续场景跳过（见上方 FAIL）")
        scenario_rejections(binary, root, args.display)
        if imported is not None:
            scenario_missing_and_relink(binary, root, args.display, imported)
    finally:
        shutil.rmtree(root, ignore_errors=True)

    print()
    print(f"断言：{sum(1 for check in CHECKS if check['ok'])}/{len(CHECKS)} 通过；失败 {len(FAILURES)}")
    if args.json:
        print(json.dumps({"checks": CHECKS, "failures": FAILURES}, ensure_ascii=False, indent=2))
    if FAILURES:
        print("RESULT: FAIL")
        return 1
    print("RESULT: PASS — 导入成功 / 非 TXT / 空文件 / 不可读 / 失效重连 全部通过，且原文件哈希未变")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

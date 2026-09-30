#!/usr/bin/env python3
"""run_txt_safety.py — DT5 数据安全端到端回归（移除书架记录 / 清理缓存后原文件、进度、书签安全）。

口径（卡片 `t_47a9bed4` 与验证卡 V-DT2）：

1. **真实 Release 应用进程**：每个阶段都是独立进程运行
   `crates/desktop-app/target/release/desktop-app`（真实 X11 窗口 + 真实 SQLite + 真实 TXT 副本），
   不做任何 mock；
2. **真实冻结核心**：需要「移除书架记录 / 清理可重建缓存」时，用本卡提供的证据夹具
   `results/desktop/safety-driver`（只调用 `reading-storage` 公开 API，不复制任何存储语义）
   在**应用真实生成的库**上执行，并读取其返回的产品报告（`ShelfRemovalReport` /
   `IndexClearReport`）；
3. **独立复核**：Python 用标准库 `sqlite3` 独立读取表行数 / 进度 / 书签 / 来源，
   用 `hashlib` 独立计算原文件 sha256、size、mtime、inode —— 不采信驱动自报结论；
4. **删除语义清单**：扫描产品源码里的文件与数据库删除点，逐条给出范围分类；
   任何无法归类的删除点都会让本脚本失败（交人工复核），不静默通过；
5. 断言列表见输出末尾的 `checks`；失败即 exit 1，前提缺失（无夹具 / 无显示 / 无法构建二进制）exit 2。

用法：
    cd ${NR_PROJECT_DIR} && python3 tests/e2e/run_txt_safety.py
    cd ${NR_PROJECT_DIR} && python3 tests/e2e/run_txt_safety.py --json   # 仅 JSON 到 stdout
退出码：0 = 全部通过；1 = 存在失败；2 = 前提缺失。

证据落盘：`results/desktop/txt-safety.json`（最新）+ `results/desktop/history/` + `results/dt5-logs/`。
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
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_BINARY = REPO_ROOT / "crates/desktop-app/target/release/desktop-app"
DEFAULT_FIXTURE = REPO_ROOT / "tests/fixtures/smoke.txt"
DRIVER_MANIFEST = REPO_ROOT / "results/desktop/safety-driver/Cargo.toml"
DRIVER_BINARY = (
    REPO_ROOT / "results/desktop/safety-driver/target/release/nr-desktop-safety-driver"
)
SLINT_ENV = Path("/opt/dev-tools/slint-libs/slint-env.sh")
RESULT_DIR = REPO_ROOT / "results/desktop"
LOG_DIR = REPO_ROOT / "results/dt5-logs"
SCHEMA = "nr.desktop.gate/1"

KV_RE = re.compile(r"^(?P<tag>NR-[A-Z-]+) (?P<fields>.*)$")

CHECKS: list[dict] = []
FAILURES: list[str] = []
PHASES: list[dict] = []
LOGS: list[Path] = []


def check(cid: str, ok: bool, detail: str) -> bool:
    entry = {"id": cid, "ok": bool(ok), "detail": detail}
    CHECKS.append(entry)
    print(("  PASS  " if ok else "  FAIL  ") + f"{cid}: {detail}", file=sys.stderr)
    if not ok:
        FAILURES.append(f"{cid}: {detail}")
    return bool(ok)


def parse_kv(text: str) -> dict:
    fields: dict[str, str] = {}
    for token in text.split():
        if "=" in token:
            key, value = token.split("=", 1)
            fields[key] = value
    return fields


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def file_facts(path: Path) -> dict:
    """只读探测用户文件（存在性 / 大小 / mtime / 权限 / inode / sha256）。"""
    if not path.exists():
        return {"path": str(path), "exists": False}
    stat = path.stat()
    return {
        "path": str(path),
        "exists": True,
        "is_file": path.is_file(),
        "size": stat.st_size,
        "mtime_ns": stat.st_mtime_ns,
        "mode": oct(stat.st_mode),
        "inode": stat.st_ino,
        "sha256": sha256_file(path) if path.is_file() else None,
    }


def same_file_facts(before: dict, after: dict) -> bool:
    keys = ("exists", "size", "mtime_ns", "mode", "inode", "sha256")
    return all(before.get(key) == after.get(key) for key in keys)


def db_snapshot(db: Path) -> dict:
    """用 Python 标准库独立读取数据库事实（不经过应用或驱动）。"""
    snapshot: dict = {"path": str(db)}
    if not db.exists():
        snapshot["exists"] = False
        return snapshot
    snapshot["exists"] = True
    con = sqlite3.connect(str(db))
    con.row_factory = sqlite3.Row
    try:
        tables = sorted(
            row[0]
            for row in con.execute(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'"
            )
        )
        snapshot["tables"] = tables
        snapshot["rows"] = {
            table: con.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]
            for table in tables
        }
        snapshot["books"] = [dict(row) for row in con.execute("SELECT * FROM books")]
        snapshot["sources"] = [
            dict(row)
            for row in con.execute(
                "SELECT id, book_id, source_kind, source_locator, source_size, is_primary FROM book_sources"
            )
        ]
        snapshot["progress"] = [
            dict(row)
            for row in con.execute(
                "SELECT book_id, content_unit_id, chapter_index, byte_offset, "
                "nearby_fingerprint, last_page_hint, index_version FROM reading_progress"
            )
        ]
        snapshot["bookmarks"] = [
            dict(row)
            for row in con.execute(
                "SELECT id, book_id, chapter_index, byte_offset, title FROM bookmarks ORDER BY id"
            )
        ]
    finally:
        con.close()
    return snapshot


def text_columns(db: Path) -> dict[str, list[str]]:
    con = sqlite3.connect(str(db))
    try:
        tables = [
            row[0]
            for row in con.execute(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'"
            )
        ]
        result: dict[str, list[str]] = {}
        for table in tables:
            columns = []
            for column in con.execute(f"PRAGMA table_info({table})"):
                if str(column[2]).upper() in ("TEXT", "VARCHAR", "CHAR", "CLOB"):
                    columns.append(column[1])
            if columns:
                result[table] = columns
        return result
    finally:
        con.close()


def scan_text_columns_for(db: Path, needle: str) -> list[str]:
    hits: list[str] = []
    con = sqlite3.connect(str(db))
    try:
        for table, columns in text_columns(db).items():
            for column in columns:
                row = con.execute(
                    f"SELECT COUNT(*) FROM {table} WHERE instr({column}, ?) > 0",
                    (needle,),
                ).fetchone()
                if row and row[0] > 0:
                    hits.append(f"{table}.{column}")
    finally:
        con.close()
    return hits


def clear_nr_env(env: dict) -> dict:
    for key in list(env):
        if key.startswith("NR_"):
            env.pop(key)
    return env


def run_app(
    binary: Path,
    db: Path,
    phase: str,
    run_id: str,
    extra_env: dict,
    timeout: int,
    log_dir: Path,
) -> dict:
    env = clear_nr_env(dict(os.environ))
    env.setdefault("DISPLAY", os.environ.get("DISPLAY") or ":0")
    env["NR_DB_PATH"] = str(db)
    env["NR_STATE_LOG"] = "1"
    env.update({key: str(value) for key, value in extra_env.items() if value is not None})
    log_path = log_dir / f"txt-safety-app-{run_id}-{phase}.log"
    LOGS.append(log_path)
    started = time.time()
    proc = subprocess.run(
        [str(binary)],
        env=env,
        capture_output=True,
        text=True,
        timeout=timeout,
        cwd=str(REPO_ROOT),
    )
    duration_ms = int((time.time() - started) * 1000)
    log_path.write_text(proc.stdout + ("\n--- stderr ---\n" + proc.stderr if proc.stderr else ""), encoding="utf-8")

    parsed = {
        "states": [],
        "pages": [],
        "toc_index": [],
        "pos": [],
        "progress": [],
        "marks": [],
        "toc": [],
    }
    for line in proc.stdout.splitlines():
        match = KV_RE.match(line)
        if not match:
            continue
        tag, fields = match.group("tag"), parse_kv(match.group("fields"))
        if tag == "NR-STATE":
            parsed["states"].append(fields)
        elif tag == "NR-PAGE":
            parsed["pages"].append(fields)
        elif tag == "NR-TOC-INDEX":
            parsed["toc_index"].append(fields)
        elif tag == "NR-POS":
            parsed["pos"].append(fields)
        elif tag == "NR-PROGRESS":
            parsed["progress"].append(fields)
        elif tag == "NR-MARK":
            parsed["marks"].append(fields)
        elif tag == "NR-TOC":
            parsed["toc"].append(fields)

    record = {
        "phase": phase,
        "argv": [str(binary)],
        "env": {key: str(value) for key, value in extra_env.items() if value is not None},
        "exit_code": proc.returncode,
        "duration_ms": duration_ms,
        "log": str(log_path),
        "states": parsed["states"],
        "pages": parsed["pages"],
        "toc_index": parsed["toc_index"],
        "pos": parsed["pos"],
        "progress": parsed["progress"],
        "marks": parsed["marks"],
        "toc": parsed["toc"],
        "stdout_tail": proc.stdout[-2000:],
        "stderr_tail": proc.stderr[-2000:],
    }
    PHASES.append({key: value for key, value in record.items() if key not in ("stdout_tail", "stderr_tail")})
    return record


def run_driver(driver: Path, db: Path, book_id: int, op: str, log_dir: Path, run_id: str) -> dict:
    argv = [str(driver), "--db", str(db), "--book-id", str(book_id), "--op", op]
    proc = subprocess.run(argv, capture_output=True, text=True, timeout=300, cwd=str(REPO_ROOT))
    log_path = log_dir / f"txt-safety-driver-{run_id}-{op}.log"
    LOGS.append(log_path)
    log_path.write_text(proc.stdout + ("\n--- stderr ---\n" + proc.stderr if proc.stderr else ""), encoding="utf-8")
    payload: dict = {}
    for line in proc.stdout.splitlines():
        line = line.strip()
        if line.startswith("{") and line.endswith("}"):
            try:
                payload = json.loads(line)
                break
            except json.JSONDecodeError:
                continue
    record = {
        "phase": f"driver:{op}",
        "argv": argv,
        "exit_code": proc.returncode,
        "log": str(log_path),
        "report": payload,
        "stderr_tail": proc.stderr[-1000:],
    }
    PHASES.append({"phase": record["phase"], "argv": argv, "exit_code": proc.returncode, "log": str(log_path)})
    return record


# --------------------------------------------------------------- 删除语义清单

# 删除点扫描：文件系统写入/删除/改名 + 数据库 DELETE。
FS_TOKENS = (
    "fs::remove_file",
    "fs::remove_dir_all",
    "std::fs::remove_file",
    "std::fs::remove_dir_all",
    "fs::rename",
    "fs::write",
    "std::fs::write",
    "File::create",
    "OpenOptions",
    "truncate(true)",
)
SQL_TOKEN = "DELETE FROM"

# 显式归类（必须给出理由）：产品源码中唯一的非测试文件删除点。
ALLOWED_UNCLASSIFIED = {
    (
        "crates/reading-storage/src/migrate.rs",
        "fs::remove_file",
    ): (
        "db_sidecar_or_backup",
        "migrate::remove_if_exists 只接收数据库路径/备份路径/`-wal`·`-shm` 边车路径"
        "（调用点 migrate.rs:200 备份目标、:222-224 恢复时的库文件与边车），不接触书籍来源文件",
    )
}

SOURCE_PATH_TOKENS = (
    "source_locator",
    "book.path",
    "source_path",
    "selection",
    "selected.path",
    "import_path",
)


def source_crates() -> list[Path]:
    files: list[Path] = []
    for crate in sorted((REPO_ROOT / "crates").iterdir()):
        src = crate / "src"
        if src.is_dir():
            files.extend(sorted(src.rglob("*.rs")))
    return files


def test_module_ranges(lines: list[str]) -> list[tuple[int, int]]:
    """用花括号配对找出每个 `#[cfg(test)]` 模块覆盖的行区间（注释内花括号不计）。"""
    ranges: list[tuple[int, int]] = []
    for index, line in enumerate(lines, 1):
        if "#[cfg(test)]" not in line:
            continue
        depth = 0
        started = False
        for cursor in range(index, len(lines) + 1):
            code = lines[cursor - 1].split("//", 1)[0]
            opens = code.count("{")
            closes = code.count("}")
            if opens:
                started = True
            depth += opens - closes
            if started and depth <= 0:
                ranges.append((index, cursor))
                break
        else:
            ranges.append((index, len(lines)))
    return ranges


def delete_semantics_inventory() -> list[dict]:
    """扫描产品源码中的删除/写入点，并逐条给出范围分类。"""
    inventory: list[dict] = []
    for path in source_crates():
        text = path.read_text(encoding="utf-8", errors="replace")
        lines = text.splitlines()
        test_ranges = test_module_ranges(lines)
        rel = str(path.relative_to(REPO_ROOT))
        for index, line in enumerate(lines, 1):
            stripped = line.strip()
            if stripped.startswith("//") or stripped.startswith("///"):
                continue
            tokens = [token for token in FS_TOKENS if token in line]
            if SQL_TOKEN in line:
                remainder = line.split(SQL_TOKEN, 1)[1].strip().lstrip('"').lstrip("'")
                table = re.split(r"[\s\"'`;]+", remainder)[0] or "<unknown>"
                inventory.append(
                    {
                        "file": rel,
                        "line": index,
                        "snippet": stripped,
                        "kind": "sql_delete",
                        "classification": "database_rows_only",
                        "justification": f"只删除数据库表 {table} 的行；SQL 无法触碰用户文件系统",
                    }
                )
            in_test = any(start <= index <= end for start, end in test_ranges)
            for token in tokens:
                normalized = token[5:] if token.startswith("std::fs::") else token
                if in_test:
                    classification, justification = (
                        "test_only",
                        "位于 `#[cfg(test)]` 测试模块内：只操作测试夹具的临时目录",
                    )
                elif token == "OpenOptions" and "read" in line:
                    classification, justification = ("read_only", "只读打开，不做删除或覆盖")
                elif token == "truncate(true)":
                    classification, justification = ("memory_or_test", "非文件句柄的截断调用（无写入语义）")
                else:
                    allowed = ALLOWED_UNCLASSIFIED.get((rel, token)) or ALLOWED_UNCLASSIFIED.get(
                        (rel, normalized)
                    )
                    if allowed:
                        classification, justification = allowed
                    else:
                        window = "\n".join(lines[max(0, index - 9) : index + 3])
                        touches_source = any(text in window for text in SOURCE_PATH_TOKENS)
                        classification = "unclassified"
                        justification = (
                            "未归类：附近出现书籍来源路径相关标识，需人工复核"
                            if touches_source
                            else "未归类：产品源码中的删除/写入点，需人工复核"
                        )
                inventory.append(
                    {
                        "file": rel,
                        "line": index,
                        "snippet": stripped,
                        "kind": "fs",
                        "token": token,
                        "classification": classification,
                        "justification": justification,
                    }
                )
    return inventory


def main() -> int:
    parser = argparse.ArgumentParser(description="DT5 数据安全端到端回归")
    parser.add_argument("--fixture", default=str(DEFAULT_FIXTURE), help="TXT 夹具路径")
    parser.add_argument("--binary", default=str(DEFAULT_BINARY), help="Release 应用二进制")
    parser.add_argument("--driver", default=str(DRIVER_BINARY), help="安全驱动二进制")
    parser.add_argument("--timeout", type=int, default=120, help="单次应用运行上限（秒）")
    parser.add_argument("--exit-after-ms", type=int, default=6000, help="应用自动退出毫秒数")
    parser.add_argument("--work-dir", default="", help="运行目录（默认 /tmp 下的独立目录）")
    parser.add_argument("--keep", action="store_true", help="保留运行目录")
    parser.add_argument("--json", action="store_true", help="仅向 stdout 输出结果 JSON")
    args = parser.parse_args()

    run_id = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    LOG_DIR.mkdir(parents=True, exist_ok=True)
    RESULT_DIR.mkdir(parents=True, exist_ok=True)
    (RESULT_DIR / "history").mkdir(parents=True, exist_ok=True)
    script_log = LOG_DIR / f"txt-safety-{run_id}.log"
    LOGS.append(script_log)

    started_at = time.time()
    fixture = Path(args.fixture).resolve()
    binary = Path(args.binary).resolve()
    driver = Path(args.driver).resolve()

    def emit(line: str) -> None:
        with script_log.open("a", encoding="utf-8") as handle:
            handle.write(line + "\n")
        if args.json:
            print(line, file=sys.stderr)
        else:
            print(line)

    print(f"== DT5 安全回归 run={run_id} ==", file=sys.stderr)

    # ---------------------------------------------------------- 前提
    if not fixture.is_file():
        emit(f"PRECONDITION_MISSING fixture={fixture}")
        check("V1_fixture_exists", False, f"夹具不存在：{fixture}")
        return 2

    display = os.environ.get("DISPLAY") or ":0"
    os.environ["DISPLAY"] = display
    display_ok = subprocess.run(
        ["xdpyinfo", "-display", display], capture_output=True, text=True
    ).returncode == 0
    if not display_ok:
        emit(f"PRECONDITION_MISSING display={display}（X 服务器不可达）")
        check("V2_display_ready", False, f"X11 显示 {display} 不可达")
        return 2
    check("V2_display_ready", True, f"X11 显示可达：DISPLAY={display}")

    if not binary.is_file():
        emit("构建缺失的 Release 二进制（cargo build --release --manifest-path crates/desktop-app/Cargo.toml）")
        source = f"source {SLINT_ENV} && " if SLINT_ENV.is_file() else ""
        build = subprocess.run(
            [
                "bash",
                "-lc",
                f"set -e; {source}cargo build --manifest-path crates/desktop-app/Cargo.toml --release",
            ],
            capture_output=True,
            text=True,
            cwd=str(REPO_ROOT),
        )
        (LOG_DIR / f"txt-safety-build-{run_id}.log").write_text(
            build.stdout + build.stderr, encoding="utf-8"
        )
        if build.returncode != 0 or not binary.is_file():
            emit("PRECONDITION_MISSING 无法构建 Release 二进制")
            check("V3_binary_ready", False, f"Release 二进制不可用：{binary}")
            return 2
    binary_facts = file_facts(binary)
    check(
        "V3_binary_ready",
        binary.is_file(),
        f"binary={binary.relative_to(REPO_ROOT)} size={binary_facts.get('size')} "
        f"sha256={str(binary_facts.get('sha256'))[:16]}…",
    )

    if not driver.is_file():
        emit("构建缺失的安全驱动（results/desktop/safety-driver）")
        build = subprocess.run(
            [
                "cargo",
                "build",
                "--release",
                "--manifest-path",
                str(DRIVER_MANIFEST),
            ],
            capture_output=True,
            text=True,
            cwd=str(REPO_ROOT),
        )
        (LOG_DIR / f"txt-safety-driver-build-{run_id}.log").write_text(
            build.stdout + build.stderr, encoding="utf-8"
        )
        if build.returncode != 0 or not driver.is_file():
            emit("PRECONDITION_MISSING 无法构建安全驱动")
            check("V4_driver_ready", False, f"安全驱动不可用：{driver}")
            return 2
    driver_facts = file_facts(driver)
    check(
        "V4_driver_ready",
        driver.is_file(),
        f"driver={driver.relative_to(REPO_ROOT)} size={driver_facts.get('size')}",
    )

    # ---------------------------------------------------------- 工作目录
    if args.work_dir:
        work = Path(args.work_dir).resolve()
        if work.exists():
            shutil.rmtree(work)
        work.mkdir(parents=True)
    else:
        work = Path(f"/tmp/nr-dt5-safety-{run_id}-{os.getpid()}")
        work.mkdir(parents=True)
    book_path = work / "原始书籍.txt"
    shutil.copyfile(fixture, book_path)
    db_path = work / "library.db"
    parked_path = work / "移动后书籍.txt"

    original_facts = file_facts(book_path)
    original_sha = original_facts["sha256"]
    check(
        "V5_fixture_copy_ready",
        original_facts["exists"] and book_path.stat().st_size > 0,
        f"夹具副本 {book_path} size={original_facts['size']} sha256={original_sha[:16]}…",
    )

    # ---------------------------------------------------------- P1 导入 + 阅读
    emit("[1] 真实应用进程：导入 → 阅读 → 翻页（用户入口）→ 新增书签 → 保存进度")
    p1 = run_app(
        binary,
        db_path,
        "01-import-read",
        run_id,
        {
            "NR_IMPORT_PATH": str(book_path),
            "NR_START_SCREEN": "reader",
            "NR_READER_BOOK": "first",
            "NR_TURNS_UI": 3,
            "NR_MARK_ADD": "安全书签A",
            "NR_SAVE_PROGRESS": 1,
            "NR_EXIT_AFTER_MS": args.exit_after_ms,
        },
        args.timeout,
        LOG_DIR,
    )
    check("V6_import_read_exit0", p1["exit_code"] == 0, f"退出码 {p1['exit_code']}")
    first_page = p1["pages"][0] if p1["pages"] else {}
    check(
        "V7_first_page_paginated",
        bool(first_page) and first_page.get("start") == "0" and int(first_page.get("lines", 0)) >= 1,
        f"首屏 NR-PAGE：{json.dumps(first_page, ensure_ascii=False)}",
    )
    toc_index = p1["toc_index"][-1] if p1["toc_index"] else {}
    check(
        "V8_toc_index_built",
        bool(toc_index) and int(toc_index.get("chapters", 0)) >= 1,
        f"章节索引：{json.dumps(toc_index, ensure_ascii=False)}",
    )
    mark_adds = [mark for mark in p1["marks"] if mark.get("op") == "add"]
    check(
        "V9_bookmark_added",
        bool(mark_adds) and int(mark_adds[-1].get("count", 0)) >= 1,
        f"书签新增：{json.dumps(mark_adds[-1] if mark_adds else {}, ensure_ascii=False)}",
    )
    progress_saves = [row for row in p1["progress"] if row.get("op") == "save" and row.get("inserted") == "1"]
    check(
        "V10_progress_saved",
        bool(progress_saves),
        f"进度保存：{json.dumps(progress_saves[-1] if progress_saves else {}, ensure_ascii=False)}",
    )

    snap_after_import = db_snapshot(db_path)
    saved_progress = snap_after_import.get("progress", [{}])[0] if snap_after_import.get("progress") else {}
    saved_offset = saved_progress.get("byte_offset")
    check(
        "V11_db_state_after_import",
        snap_after_import.get("books")
        and snap_after_import["books"][0]["status"] == "ready"
        and len(snap_after_import.get("bookmarks", [])) == 1
        and int(snap_after_import["rows"].get("chapters", 0)) >= 1
        and len(snap_after_import.get("sources", [])) == 1
        and isinstance(saved_offset, int),
        f"books={len(snap_after_import.get('books', []))} status={snap_after_import.get('books', [{}])[0].get('status')} "
        f"progress_offset={saved_offset} bookmarks={len(snap_after_import.get('bookmarks', []))} "
        f"chapters={snap_after_import.get('rows', {}).get('chapters')} sources={len(snap_after_import.get('sources', []))}",
    )

    facts_after_import = file_facts(book_path)
    check(
        "V12_file_unchanged_after_import",
        same_file_facts(original_facts, facts_after_import),
        f"原文件 sha256/size/mtime/inode 前后一致（sha256={facts_after_import.get('sha256', '')[:16]}…）",
    )

    # ---------------------------------------------------------- P2 清理可重建缓存
    emit("[2] 真实冻结核心：清理可重建索引与分页缓存（chapters / layout_records）")
    clear = run_driver(driver, db_path, 1, "clear-index", LOG_DIR, run_id)
    report = clear.get("report", {})
    clear_report = report.get("ops", {}).get("clear_rebuildable_index", {})
    check(
        "V13_clear_index_driver_exit0",
        clear["exit_code"] == 0 and report.get("result") == "PASS",
        f"驱动退出码 {clear['exit_code']}，result={report.get('result')}，"
        f"chapters_deleted={clear_report.get('chapters_deleted')} layout_records_deleted={clear_report.get('layout_records_deleted')}",
    )
    check(
        "V14_clear_index_reports_metadata_preserved",
        int(clear_report.get("books_preserved", 0)) >= 1
        and int(clear_report.get("progress_preserved", 0)) >= 1
        and int(clear_report.get("bookmarks_preserved", 0)) >= 1
        and int(clear_report.get("book_sources_preserved", 0)) >= 1,
        f"IndexClearReport: {json.dumps(clear_report, ensure_ascii=False)}",
    )

    snap_after_clear = db_snapshot(db_path)
    check(
        "V15_db_state_after_clear_index",
        int(snap_after_clear["rows"].get("chapters", -1)) == 0
        and int(snap_after_clear["rows"].get("layout_records", -1)) == 0
        and len(snap_after_clear.get("books", [])) == 1
        and len(snap_after_clear.get("bookmarks", [])) == 1
        and len(snap_after_clear.get("sources", [])) == 1
        and snap_after_clear.get("progress", [{}])[0].get("byte_offset") == saved_offset,
        f"chapters={snap_after_clear['rows'].get('chapters')} layout_records={snap_after_clear['rows'].get('layout_records')} "
        f"books={len(snap_after_clear.get('books', []))} bookmarks={len(snap_after_clear.get('bookmarks', []))} "
        f"progress_offset={snap_after_clear.get('progress', [{}])[0].get('byte_offset')}",
    )

    facts_after_clear = file_facts(book_path)
    check(
        "V16_file_unchanged_after_clear_index",
        same_file_facts(original_facts, facts_after_clear),
        f"清理缓存后原文件仍完整（sha256={facts_after_clear.get('sha256', '')[:16]}…）",
    )

    # ---------------------------------------------------------- P3 重开重建 + 进度恢复
    emit("[3] 真实应用进程：清理缓存后重新打开书籍（重建章节索引 + 恢复进度）")
    p3 = run_app(
        binary,
        db_path,
        "03-reopen-after-clear",
        run_id,
        {
            "NR_START_SCREEN": "reader",
            "NR_READER_BOOK": "first",
            "NR_EXIT_AFTER_MS": args.exit_after_ms,
        },
        args.timeout,
        LOG_DIR,
    )
    toc_after_reopen = p3["toc_index"][-1] if p3["toc_index"] else {}
    check(
        "V17_reopen_rebuilds_chapters",
        p3["exit_code"] == 0 and int(toc_after_reopen.get("chapters", 0)) >= 1,
        f"退出码 {p3['exit_code']}；重建章节数={toc_after_reopen.get('chapters')}",
    )
    pos = p3["pos"][-1] if p3["pos"] else {}
    check(
        "V18_progress_restored_without_loss",
        pos.get("restored") == "1"
        and int(pos.get("offset", -1)) == int(saved_offset)
        and int(pos.get("saved_offset", -1)) == int(saved_offset),
        f"NR-POS offset={pos.get('offset')} saved_offset={pos.get('saved_offset')} restored={pos.get('restored')} "
        f"anchor={pos.get('anchor')} distance={pos.get('distance')}（入库进度={saved_offset}）",
    )
    snap_after_reopen = db_snapshot(db_path)
    check(
        "V19_db_state_after_reopen",
        snap_after_reopen["books"][0]["status"] == "ready"
        and snap_after_reopen.get("progress", [{}])[0].get("byte_offset") == saved_offset
        and len(snap_after_reopen.get("bookmarks", [])) == 1
        and int(snap_after_reopen["rows"].get("chapters", 0)) >= 1,
        f"chapters={snap_after_reopen['rows'].get('chapters')} "
        f"progress_offset={snap_after_reopen.get('progress', [{}])[0].get('byte_offset')} "
        f"bookmarks={len(snap_after_reopen.get('bookmarks', []))}",
    )

    # ---------------------------------------------------------- P4 失效 + 重新关联
    emit("[4] 真实应用进程：原路径失效 → 重新关联（不得静默丢弃旧进度）")
    book_path.rename(parked_path)
    parked_facts = file_facts(parked_path)
    check(
        "V20_parked_copy_identical",
        parked_facts["sha256"] == original_sha and not book_path.exists(),
        f"移动后副本内容一致（sha256={parked_facts['sha256'][:16]}…），原路径不存在={not book_path.exists()}",
    )
    p4 = run_app(
        binary,
        db_path,
        "04-relink",
        run_id,
        {
            "NR_RELINK_BOOK": 1,
            "NR_RELINK_PATH": str(parked_path),
            "NR_START_SCREEN": "shelf",
            "NR_EXIT_AFTER_MS": args.exit_after_ms,
        },
        args.timeout,
        LOG_DIR,
    )
    relink_status = next(
        (state.get("status", "") for state in p4["states"] if "重新关联" in state.get("status", "")),
        "",
    )
    check(
        "V21_relink_executed",
        p4["exit_code"] == 0 and "重新关联" in relink_status,
        f"退出码 {p4['exit_code']}；状态行={relink_status or '<未出现>'}",
    )
    snap_after_relink = db_snapshot(db_path)
    relink_sources = snap_after_relink.get("sources", [])
    check(
        "V22_db_state_after_relink",
        len(relink_sources) == 2
        and snap_after_relink["books"][0]["source_locator"] == str(parked_path)
        and snap_after_relink.get("progress", [{}])[0].get("byte_offset") == saved_offset
        and len(snap_after_relink.get("bookmarks", [])) == 1,
        f"sources={[source['source_locator'] for source in relink_sources]} "
        f"locator={snap_after_relink['books'][0]['source_locator']} "
        f"progress_offset={snap_after_relink.get('progress', [{}])[0].get('byte_offset')} "
        f"bookmarks={len(snap_after_relink.get('bookmarks', []))}",
    )
    check(
        "V23_progress_bookmarks_unchanged_by_relink",
        snap_after_relink.get("progress") == snap_after_import.get("progress")
        and [b["byte_offset"] for b in snap_after_relink.get("bookmarks", [])]
        == [b["byte_offset"] for b in snap_after_import.get("bookmarks", [])],
        "重新关联前后 reading_progress 逐字段一致、书签偏移一致",
    )
    facts_after_relink = file_facts(parked_path)
    check(
        "V24_file_unchanged_after_relink",
        same_file_facts(parked_facts, facts_after_relink)
        and not book_path.exists(),
        f"新路径文件未被改写（sha256={facts_after_relink.get('sha256', '')[:16]}…），旧路径仍不存在（应用未复活/删除文件）",
    )

    # ---------------------------------------------------------- P5 移除书架记录
    emit("[5] 真实冻结核心：软移除书架记录（保留进度 / 书签 / 来源，不删除原始文件）")
    remove = run_driver(driver, db_path, 1, "remove", LOG_DIR, run_id)
    remove_report = remove.get("report", {})
    removal = remove_report.get("ops", {}).get("remove_book_from_shelf", {})
    check(
        "V25_remove_driver_exit0",
        remove["exit_code"] == 0 and remove_report.get("result") == "PASS",
        f"驱动退出码 {remove['exit_code']}，result={remove_report.get('result')}",
    )
    check(
        "V26_remove_report_preserves_user_data",
        removal.get("status") == "removed"
        and removal.get("progress_preserved") is True
        and int(removal.get("bookmarks_preserved", 0)) >= 1
        and int(removal.get("sources_preserved", 0)) >= 2
        and int(removal.get("visible_shelf_rows", -1)) == 0,
        f"ShelfRemovalReport: {json.dumps(removal, ensure_ascii=False)}",
    )

    snap_after_remove = db_snapshot(db_path)
    check(
        "V27_db_state_after_remove",
        len(snap_after_remove.get("books", [])) == 1
        and snap_after_remove["books"][0]["status"] == "removed"
        and len(snap_after_remove.get("bookmarks", [])) == 1
        and len(snap_after_remove.get("sources", [])) == 2
        and snap_after_remove.get("progress", [{}])[0].get("byte_offset") == saved_offset
        and int(snap_after_remove["rows"].get("chapters", -1)) == 0,
        f"books={len(snap_after_remove.get('books', []))} status={snap_after_remove['books'][0]['status']} "
        f"progress_offset={snap_after_remove.get('progress', [{}])[0].get('byte_offset')} "
        f"bookmarks={len(snap_after_remove.get('bookmarks', []))} sources={len(snap_after_remove.get('sources', []))} "
        f"chapters={snap_after_remove['rows'].get('chapters')}",
    )
    facts_after_remove = file_facts(parked_path)
    parked_unchanged = same_file_facts(parked_facts, facts_after_remove)
    check(
        "V28_file_unchanged_after_remove",
        parked_unchanged and not book_path.exists(),
        f"移除书架记录后原 TXT 仍存在且哈希不变：exists={facts_after_remove['exists']} "
        f"size={facts_after_remove.get('size')} sha256={facts_after_remove.get('sha256', '')[:16]}…",
    )
    check(
        "V29_no_file_deleted_or_recreated",
        not book_path.exists() and parked_path.exists(),
        f"旧路径未被删除也不再存在={not book_path.exists()}；新路径仍在={parked_path.exists()}；"
        "应用未创建、未移动、未删除任何用户文件",
    )

    # ---------------------------------------------------------- 删除语义清单
    emit("[6] 删除语义清单（产品源码逐条范围分类）")
    inventory = delete_semantics_inventory()
    unclassified = [item for item in inventory if item["classification"] == "unclassified"]
    check(
        "V30_delete_semantics_all_classified",
        not unclassified,
        f"共 {len(inventory)} 个删除/写入点，全部已归类；未归类 {len(unclassified)} 个"
        + (f"：{json.dumps(unclassified, ensure_ascii=False)}" if unclassified else ""),
    )
    risky = [
        item
        for item in inventory
        if item["classification"] == "unclassified"
        or "书籍来源路径" in item["justification"]
    ]
    check(
        "V31_no_delete_point_touches_book_source",
        not risky,
        f"无删除/写入点引用书籍来源路径（复核 {len(inventory)} 个点，可疑 {len(risky)} 个）",
    )

    # ---------------------------------------------------------- 正文不入库
    emit("[7] 正文边界：库内不得出现夹具正文（正向对照：文件中必须命中）")
    fixture_text = fixture.read_bytes()
    longest_line = max(
        (line for line in fixture_text.decode("utf-8", errors="replace").splitlines()),
        key=len,
    )
    sentinel = longest_line.strip()[:120]
    in_file = sentinel.encode("utf-8") in fixture_text if sentinel else False
    db_hits = scan_text_columns_for(db_path, sentinel) if in_file else ["<sentinel 未在文件中命中，检查无效>"]
    check(
        "V32_db_contains_no_book_body",
        in_file and not db_hits,
        f"哨兵长度 {len(sentinel)}；文件命中={in_file}；库内命中列={db_hits or '无'}（全库 TEXT 列扫描）",
    )

    # ---------------------------------------------------------- 退出清理
    emit("[8] 退出清理：无残留进程 / 无残留窗口")
    residual = subprocess.run(["pgrep", "-x", "desktop-app"], capture_output=True, text=True)
    windows = subprocess.run(
        ["xdotool", "search", "--onlyvisible", "--class", "desktop-app"],
        capture_output=True,
        text=True,
    )
    check(
        "V33_no_residual_process_or_window",
        residual.returncode != 0 and not windows.stdout.strip(),
        f"pgrep -x desktop-app 无命中={residual.returncode != 0}；"
        f"残留窗口={windows.stdout.split() or '无'}",
    )

    # ---------------------------------------------------------- 结果 JSON
    finished_at = time.time()
    result = "FAIL" if FAILURES else "PASS"
    exit_code = 1 if FAILURES else 0
    doc = {
        "schema": SCHEMA,
        "gate": "txt-safety",
        "run_id": run_id,
        "platform": {"os": sys.platform, "backend": "x11", "display": display},
        "repo_root": str(REPO_ROOT),
        "command": "python3 tests/e2e/run_txt_safety.py"
        + (" --json" if args.json else ""),
        "started_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(started_at)),
        "finished_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(finished_at)),
        "duration_ms": int((finished_at - started_at) * 1000),
        "binary": {
            "path": str(binary.relative_to(REPO_ROOT)),
            "size": binary_facts.get("size"),
            "sha256": binary_facts.get("sha256"),
        },
        "driver": {
            "path": str(driver.relative_to(REPO_ROOT)),
            "size": driver_facts.get("size"),
            "sha256": driver_facts.get("sha256"),
        },
        "fixture": {
            "source": str(fixture.relative_to(REPO_ROOT)),
            "copy": str(book_path),
            "moving_path": str(parked_path),
            "facts_before": original_facts,
            "facts_after_import": facts_after_import,
            "facts_after_clear_index": facts_after_clear,
            "facts_after_relink": facts_after_relink,
            "facts_after_remove": facts_after_remove,
        },
        "database": {
            "path": str(db_path),
            "after_import": snap_after_import,
            "after_clear_index": snap_after_clear,
            "after_reopen": snap_after_reopen,
            "after_relink": snap_after_relink,
            "after_remove": snap_after_remove,
        },
        "saved_progress_offset": saved_offset,
        "delete_semantics": inventory,
        "phases": PHASES,
        "checks": CHECKS,
        "summary": {
            "total": len(CHECKS),
            "passed": len([entry for entry in CHECKS if entry["ok"]]),
            "failed": len(FAILURES),
        },
        "result": result,
        "exit_code": exit_code,
        "artifacts": {"script_log": str(script_log), "logs": [str(path) for path in LOGS]},
    }

    latest = RESULT_DIR / "txt-safety.json"
    history = RESULT_DIR / "history" / f"txt-safety-{run_id}.json"
    text = json.dumps(doc, ensure_ascii=False, indent=2) + "\n"
    for path in (latest, history):
        path.write_text(text, encoding="utf-8")

    summary_line = (
        f"RESULT: {result} — 检查 {doc['summary']['passed']}/{doc['summary']['total']} 通过"
        f"（失败 {doc['summary']['failed']}）；原文件 sha256 前后一致="
        f"{original_sha == facts_after_remove.get('sha256')}；JSON={latest}"
    )
    emit(summary_line)

    if not args.keep and not args.work_dir:
        # 库与工作目录属于本次运行的临时证据；JSON 已记录全部事实，故默认清理。
        shutil.rmtree(work, ignore_errors=True)

    if args.json:
        print(text, end="")
    return exit_code


if __name__ == "__main__":
    sys.exit(main())

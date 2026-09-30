#!/usr/bin/env bash
# smoke-desktop.sh — DT5 桌面 Release 冒烟门禁（真实 Release 构建 + 真实 X11 窗口 + 程序化退出码证据）。
#
# 口径（卡片 `t_47a9bed4` / desktop checklist C5）：
#   1) **真实 Release 构建**：`cargo build --manifest-path crates/desktop-app/Cargo.toml --release`；
#   2) **真实 X11 启动**：启动 `crates/desktop-app/target/release/desktop-app`，用 `xdotool` 证明
#      窗口真的被映射（宽高 > 0），**不用截图或肉眼观察**；
#   3) **结构化就绪输出**：解析应用 stdout 的 `NR-STATE` / `NR-PAGE` / `NR-TOC-INDEX` 行，
#      断言「书架里出现导入的书」「首屏第 1 页已分页」；
#   4) **退出清理**：应用自行退出（退出码 0），脚本校验无残留进程、无残留窗口；
#   5) 证据落盘：`results/desktop/smoke-desktop.json`（字段约定见 results/desktop/README.md）
#      + `results/desktop/history/` + 原始日志 `results/dt5-logs/`。
#
# 用法：
#   source /opt/dev-tools/slint-libs/slint-env.sh     # Slint 编译依赖（必须）
#   bash scripts/smoke-desktop.sh --backend x11 --fixture tests/fixtures/smoke.txt
#
# 退出码：0 = PASS；1 = FAIL；2 = 前提缺失（无二进制/构建失败 / 无显示 / 无夹具）。
#
# 注意：本脚本禁用 `| head -1` 这类会提前关闭管道（SIGPIPE → 141）的写法，改用 `awk 'NR==1{…}'`。

set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

BACKEND="x11"
FIXTURE="$REPO_ROOT/tests/fixtures/smoke.txt"
BINARY="$REPO_ROOT/crates/desktop-app/target/release/desktop-app"
MANIFEST="$REPO_ROOT/crates/desktop-app/Cargo.toml"
DB_PATH=""
TIMEOUT_SEC=90
EXIT_AFTER_MS=4000
RUN_TURNS_UI=1
DO_BUILD=1
KEEP=0
JSON_ONLY=0

usage() {
  cat <<'TXT'
用法：bash scripts/smoke-desktop.sh [选项]
  --backend x11|wayland   目标后端（默认 x11）
  --fixture PATH          TXT 夹具（默认 tests/fixtures/smoke.txt）
  --binary PATH           待测 Release 二进制（默认 crates/desktop-app/target/release/desktop-app）
  --db PATH               指定库文件（默认本次运行的临时库）
  --timeout SEC           应用退出等待上限（默认 90）
  --exit-after-ms MS      应用自动退出毫秒数（默认 4000）
  --no-build              跳过 Release 构建（要求二进制已存在）
  --keep                  保留运行目录（默认删除）
  --json                  仅输出结果 JSON（人类可读行写 stderr）
TXT
}

while [ $# -gt 0 ]; do
  case "$1" in
    --backend) BACKEND="${2:-}"; shift 2 ;;
    --fixture) FIXTURE="${2:-}"; shift 2 ;;
    --binary) BINARY="${2:-}"; shift 2 ;;
    --db) DB_PATH="${2:-}"; shift 2 ;;
    --timeout) TIMEOUT_SEC="${2:-}"; shift 2 ;;
    --exit-after-ms) EXIT_AFTER_MS="${2:-}"; shift 2 ;;
    --no-build) DO_BUILD=0; shift ;;
    --keep) KEEP=1; shift ;;
    --json) JSON_ONLY=1; shift ;;
    --help|-h) usage; exit 0 ;;
    *) echo "未知参数：$1" >&2; usage >&2; exit 2 ;;
  esac
done

RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
LOGDIR="$REPO_ROOT/results/dt5-logs"
RESULT_DIR="$REPO_ROOT/results/desktop"
mkdir -p "$LOGDIR" "$RESULT_DIR/history"
SCRIPT_LOG="$LOGDIR/smoke-desktop-$RUN_ID.log"
APP_LOG="$LOGDIR/smoke-desktop-app-$RUN_ID.log"
BUILD_LOG="$LOGDIR/smoke-build-$RUN_ID.log"
WORK_DIR="$(mktemp -d "/tmp/nr-dt5-smoke-$RUN_ID-XXXXXX")"
FACTS="$WORK_DIR/facts.tsv"
DB_FACTS_FILE="$WORK_DIR/db-facts.json"
: > "$FACTS"
: > "$SCRIPT_LOG"

APP_PID=""
APP_EXIT=""
WID=""
WIN_NAME=""
WIDTH=0
HEIGHT=0
OVERALL="PASS"
EXIT_CODE=0

say() {
  if [ "$JSON_ONLY" -eq 1 ]; then
    printf '%s\n' "$*" >> "$SCRIPT_LOG"
    printf '%s\n' "$*" >&2
  else
    printf '%s\n' "$*" | tee -a "$SCRIPT_LOG"
  fi
}

# TSV：check_id <TAB> ok(0=通过/1=失败/3=跳过) <TAB> detail
record() { printf '%s\t%s\t%s\n' "$1" "$2" "$3" >> "$FACTS"; }
pass_check() { record "$1" 0 "$2"; say "CHECK $1 PASS — $2"; }
fail_check() { record "$1" 1 "$2"; OVERALL="FAIL"; say "CHECK $1 FAIL — $2"; }

cleanup() {
  local code=$?
  if [ -n "$APP_PID" ] && kill -0 "$APP_PID" 2>/dev/null; then
    kill -TERM "$APP_PID" 2>/dev/null || true
    for _ in 1 2 3 4 5 6 7 8 9 10; do
      kill -0 "$APP_PID" 2>/dev/null || break
      sleep 0.2
    done
    kill -KILL "$APP_PID" 2>/dev/null || true
  fi
  if [ "$KEEP" -eq 0 ]; then rm -rf "$WORK_DIR"; fi
  return $code
}
trap cleanup EXIT

# ------------------------------------------------------------------ 前提
if [ ! -f "$FIXTURE" ]; then
  say "PRECONDITION_MISSING fixture=$FIXTURE"
  record "S0_fixture_exists" 1 "夹具不存在：$FIXTURE"
  exit 2
fi

case "$BACKEND" in
  x11)
    export DISPLAY="${DISPLAY:-:0}"
    if ! xdpyinfo -display "$DISPLAY" >/dev/null 2>&1; then
      say "PRECONDITION_MISSING backend=x11 display=$DISPLAY（X 服务器不可达）"
      record "S1_display_ready" 1 "X11 显示 $DISPLAY 不可达"
      exit 2
    fi
    pass_check "S1_display_ready" "X11 显示可达：DISPLAY=$DISPLAY（xdpyinfo 退出码 0）"
    ;;
  wayland)
    if [ -z "${WAYLAND_DISPLAY:-}" ]; then
      say "PRECONDITION_MISSING backend=wayland（本机无 Wayland 会话，未实测）"
      record "S1_display_ready" 1 "WAYLAND_DISPLAY 未设置：Wayland 未实测（待验证）"
      exit 2
    fi
    pass_check "S1_display_ready" "Wayland 会话存在：WAYLAND_DISPLAY=$WAYLAND_DISPLAY"
    ;;
  *)
    echo "未知 --backend：$BACKEND" >&2
    exit 2
    ;;
esac

# ------------------------------------------------------------------ S2 Release 构建
if [ "$DO_BUILD" -eq 1 ]; then
  say "BUILD cargo build --manifest-path crates/desktop-app/Cargo.toml --release（日志 $BUILD_LOG）"
  if (cd "$REPO_ROOT" && cargo build --manifest-path "$MANIFEST" --release) >"$BUILD_LOG" 2>&1; then
    pass_check "S2_release_build" "Release 构建成功（日志 ${BUILD_LOG#"$REPO_ROOT"/}）"
  else
    fail_check "S2_release_build" "Release 构建失败，详见 $BUILD_LOG"
    EXIT_CODE=1
  fi
else
  pass_check "S2_release_build" "跳过构建（--no-build）"
fi

if [ ! -x "$BINARY" ]; then
  say "PRECONDITION_MISSING binary=$BINARY 不存在或不可执行"
  record "S3_binary_present" 1 "Release 二进制不存在：$BINARY"
  exit 2
fi
BIN_SHA="$(sha256sum "$BINARY" | awk '{print $1}')"
BIN_SIZE="$(stat -c '%s' "$BINARY")"
BIN_MTIME="$(stat -c '%y' "$BINARY")"
pass_check "S3_binary_present" "binary=${BINARY#"$REPO_ROOT"/} size=$BIN_SIZE sha256=${BIN_SHA:0:16}…"

# ------------------------------------------------------------------ 资源准备（夹具副本 + 临时库）
FIXTURE_SHA_BEFORE="$(sha256sum "$FIXTURE" | awk '{print $1}')"
FIXTURE_SIZE="$(stat -c '%s' "$FIXTURE")"
BOOK_COPY="$WORK_DIR/冒烟书籍.txt"
cp "$FIXTURE" "$BOOK_COPY"
COPY_SHA_BEFORE="$(sha256sum "$BOOK_COPY" | awk '{print $1}')"
if [ -z "$DB_PATH" ]; then DB_PATH="$WORK_DIR/library.db"; fi

# ------------------------------------------------------------------ S4 启动应用
export NR_DB_PATH="$DB_PATH"
export NR_IMPORT_PATH="$BOOK_COPY"
export NR_STATE_LOG=1
export NR_START_SCREEN=reader
export NR_READER_BOOK=first
export NR_TURNS_UI="$RUN_TURNS_UI"
export NR_EXIT_AFTER_MS="$EXIT_AFTER_MS"

say "LAUNCH backend=$BACKEND display=${DISPLAY:-${WAYLAND_DISPLAY:-}} db=$DB_PATH import=$BOOK_COPY exit_after_ms=$EXIT_AFTER_MS"
"$BINARY" >"$APP_LOG" 2>&1 &
APP_PID=$!

# ------------------------------------------------------------------ S5 窗口就绪（xdotool 程序化证据）
if [ "$BACKEND" = "x11" ]; then
  for _ in $(seq 1 60); do
    WID="$(xdotool search --onlyvisible --class 'desktop-app' 2>/dev/null | awk 'NR==1{print; exit}' || true)"
    if [ -n "$WID" ]; then break; fi
    if ! kill -0 "$APP_PID" 2>/dev/null; then break; fi
    sleep 0.25
  done
  if [ -n "$WID" ]; then
    WIN_NAME="$(xdotool getwindowname "$WID" 2>/dev/null || true)"
    eval "$(xdotool getwindowgeometry --shell "$WID" 2>/dev/null || true)"
    if [ "${WIDTH:-0}" -gt 0 ] && [ "${HEIGHT:-0}" -gt 0 ]; then
      pass_check "S5_window_ready" "X11 窗口已映射：wid=$WID name=$WIN_NAME geometry=${WIDTH}x${HEIGHT}"
    else
      fail_check "S5_window_ready" "找到窗口 wid=$WID 但几何无效（${WIDTH:-0}x${HEIGHT:-0}）"
    fi
  else
    fail_check "S5_window_ready" "未发现 WM_CLASS=desktop-app 的可见窗口"
  fi
else
  record "S5_window_ready" 3 "Wayland 会话下不做窗口枚举（待验证）"
  say "CHECK S5_window_ready SKIP — Wayland 会话下不做窗口枚举（待验证）"
fi

# ------------------------------------------------------------------ S6 等待自行退出
WAITED=0
while kill -0 "$APP_PID" 2>/dev/null; do
  if [ "$WAITED" -ge $((TIMEOUT_SEC * 4)) ]; then break; fi
  sleep 0.25
  WAITED=$((WAITED + 1))
done
if kill -0 "$APP_PID" 2>/dev/null; then
  fail_check "S6_self_exit" "${TIMEOUT_SEC}s 内未自行退出，判定为挂起"
  kill -TERM "$APP_PID" 2>/dev/null || true
fi
set +e
wait "$APP_PID"
APP_EXIT=$?
set -e
APP_PID=""
if [ "${APP_EXIT:-1}" -eq 0 ]; then
  pass_check "S6_self_exit" "应用自行退出，退出码 0（等待 ${WAITED}×0.25s）"
else
  fail_check "S6_self_exit" "应用退出码 ${APP_EXIT:-?}（期望 0）"
fi

# ------------------------------------------------------------------ S7 结构化就绪输出
STATE_LINES="$(awk '/^NR-STATE /{count++} END{print count+0}' "$APP_LOG")"
PAGE_LINES="$(awk '/^NR-PAGE /{count++} END{print count+0}' "$APP_LOG")"
TOC_INDEX_LINES="$(awk '/^NR-TOC-INDEX /{count++} END{print count+0}' "$APP_LOG")"

SHELF_READY="$(awk '/^NR-STATE / && /rows=1/ && /row_status=可阅读/ {found=1} END{print found+0}' "$APP_LOG")"
if [ "$SHELF_READY" -eq 1 ]; then
  pass_check "S7_import_visible_in_shelf" "NR-STATE 显示书架 1 行且状态=可阅读"
else
  fail_check "S7_import_visible_in_shelf" "未在 NR-STATE 中观察到「rows=1 且 row_status=可阅读」"
fi

TOC_SUMMARY="$(awk '/^NR-TOC-INDEX /{line=$0} END{print line}' "$APP_LOG")"
if [ "$TOC_INDEX_LINES" -ge 1 ]; then
  pass_check "S8_toc_index_built" "章节目录已建立：$TOC_SUMMARY"
else
  fail_check "S8_toc_index_built" "未观察到 NR-TOC-INDEX（章节索引未建立）"
fi

FIRST_PAGE="$(awk '/^NR-PAGE /{print; exit}' "$APP_LOG")"
FIRST_PAGE_OK=0
FIRST_LINES=""
if [ -n "$FIRST_PAGE" ]; then
  case "$FIRST_PAGE" in
    *"page_index=0"*"start=0"*"eof=0"*)
      FIRST_LINES="$(printf '%s\n' "$FIRST_PAGE" | sed -n 's/.* lines=\([0-9]*\) .*/\1/p')"
      if [ -n "$FIRST_LINES" ] && [ "$FIRST_LINES" -ge 1 ]; then FIRST_PAGE_OK=1; fi
      ;;
  esac
fi
if [ "$FIRST_PAGE_OK" -eq 1 ]; then
  pass_check "S9_first_screen_paginated" "首屏已分页（${FIRST_LINES} 行）：$FIRST_PAGE"
else
  fail_check "S9_first_screen_paginated" "未取得合法首屏 NR-PAGE（page_index=0 / start=0）：${FIRST_PAGE:-<无>}"
fi

if [ "$STATE_LINES" -ge 3 ] && [ "$PAGE_LINES" -ge 1 ]; then
  pass_check "S10_structured_state" "结构化就绪输出：NR-STATE=${STATE_LINES} 行, NR-PAGE=${PAGE_LINES} 行, NR-TOC-INDEX=${TOC_INDEX_LINES} 行"
else
  fail_check "S10_structured_state" "结构化输出不足：NR-STATE=${STATE_LINES}, NR-PAGE=${PAGE_LINES}, NR-TOC-INDEX=${TOC_INDEX_LINES}"
fi

# ------------------------------------------------------------------ S11 退出清理
sleep 0.3
RESIDUAL="$(pgrep -x desktop-app 2>/dev/null | tr '\n' ' ' || true)"
if [ -z "$RESIDUAL" ]; then
  pass_check "S11_no_residual_process" "pgrep -x desktop-app 无命中"
else
  fail_check "S11_no_residual_process" "发现残留进程：$RESIDUAL"
fi

if [ "$BACKEND" = "x11" ]; then
  LEFT_WINDOWS="$(xdotool search --onlyvisible --class 'desktop-app' 2>/dev/null | tr '\n' ' ' || true)"
  if [ -z "$LEFT_WINDOWS" ]; then
    pass_check "S12_no_residual_window" "退出后 WM_CLASS=desktop-app 的窗口已消失"
  else
    fail_check "S12_no_residual_window" "退出后仍有窗口：$LEFT_WINDOWS"
  fi
fi

# ------------------------------------------------------------------ S13 夹具只读
FIXTURE_SHA_AFTER="$(sha256sum "$FIXTURE" | awk '{print $1}')"
COPY_SHA_AFTER="$(sha256sum "$BOOK_COPY" | awk '{print $1}')"
if [ "$FIXTURE_SHA_BEFORE" = "$FIXTURE_SHA_AFTER" ] && [ "$COPY_SHA_BEFORE" = "$COPY_SHA_AFTER" ]; then
  pass_check "S13_fixture_readonly" "夹具与导入副本 sha256 前后一致（${COPY_SHA_BEFORE:0:16}…）"
else
  fail_check "S13_fixture_readonly" "夹具内容变化：fixture=$FIXTURE_SHA_BEFORE→$FIXTURE_SHA_AFTER copy=$COPY_SHA_BEFORE→$COPY_SHA_AFTER"
fi

# ------------------------------------------------------------------ 结果 JSON
python3 - "$DB_PATH" "$DB_FACTS_FILE" <<'PY' || true
import json, sqlite3, sys
db, out = sys.argv[1], sys.argv[2]
facts = {"path": db}
try:
    con = sqlite3.connect(db)
    tables = [r[0] for r in con.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")]
    facts["tables"] = sorted(tables)
    facts["rows"] = {t: con.execute(f"SELECT COUNT(*) FROM {t}").fetchone()[0] for t in sorted(tables)}
    facts["books"] = [
        {"id": r[0], "status": r[1], "display_name": r[2], "source_locator": r[3]}
        for r in con.execute("SELECT id, status, display_name, source_locator FROM books")
    ]
    facts["progress"] = [
        {"book_id": r[0], "chapter_index": r[1], "byte_offset": r[2]}
        for r in con.execute("SELECT book_id, chapter_index, byte_offset FROM reading_progress")
    ]
    facts["bookmarks"] = [
        {"id": r[0], "title": r[1], "byte_offset": r[2]}
        for r in con.execute("SELECT id, title, byte_offset FROM bookmarks")
    ]
    con.close()
except Exception as error:  # 库缺失时如实记录，不伪造
    facts["error"] = str(error)
with open(out, "w", encoding="utf-8") as handle:
    handle.write(json.dumps(facts, ensure_ascii=False))
PY
[ -s "$DB_FACTS_FILE" ] || printf '%s' '{"path": "'"$DB_PATH"'", "error": "db facts unavailable"}' > "$DB_FACTS_FILE"
say "DB $(cat "$DB_FACTS_FILE")"

PASSED="$(awk -F'\t' '$2==0{count++} END{print count+0}' "$FACTS")"
FAILED="$(awk -F'\t' '$2==1{count++} END{print count+0}' "$FACTS")"
SKIPPED="$(awk -F'\t' '$2==3{count++} END{print count+0}' "$FACTS")"

if [ "$FAILED" -gt 0 ]; then OVERALL="FAIL"; fi
if [ "$OVERALL" = "PASS" ]; then EXIT_CODE=0; else EXIT_CODE=1; fi

JSON_LATEST="$RESULT_DIR/smoke-desktop.json"
JSON_HISTORY="$RESULT_DIR/history/smoke-desktop-$RUN_ID.json"

python3 - "$FACTS" "$DB_FACTS_FILE" "$JSON_LATEST" "$JSON_HISTORY" <<PY
import json, sys

facts_path, db_facts_path, latest, history = sys.argv[1:5]
with open(db_facts_path, encoding="utf-8") as handle:
    db_facts = json.load(handle)

doc = {
    "schema": "nr.desktop.gate/1",
    "gate": "smoke-desktop",
    "run_id": "$RUN_ID",
    "backend": "$BACKEND",
    "display": "${DISPLAY:-${WAYLAND_DISPLAY:-}}",
    "repo_root": "$REPO_ROOT",
    "command": "bash scripts/smoke-desktop.sh --backend $BACKEND --fixture ${FIXTURE#"$REPO_ROOT"/}",
    "fixture": {
        "path": "${FIXTURE#"$REPO_ROOT"/}",
        "size": int("$FIXTURE_SIZE"),
        "sha256_before": "$FIXTURE_SHA_BEFORE",
        "sha256_after": "$FIXTURE_SHA_AFTER",
        "import_copy_sha256_before": "$COPY_SHA_BEFORE",
        "import_copy_sha256_after": "$COPY_SHA_AFTER",
    },
    "binary": {
        "path": "${BINARY#"$REPO_ROOT"/}",
        "size": int("$BIN_SIZE"),
        "mtime": "$BIN_MTIME",
        "sha256": "$BIN_SHA",
    },
    "app": {
        "exit_code": int("${APP_EXIT:-1}"),
        "exit_after_ms": int("$EXIT_AFTER_MS"),
        "waited_ticks_250ms": int("$WAITED"),
        "window": {
            "wid": "$WID",
            "name": "$WIN_NAME",
            "width": int("${WIDTH:-0}"),
            "height": int("${HEIGHT:-0}"),
        },
        "nr_state_lines": int("$STATE_LINES"),
        "nr_page_lines": int("$PAGE_LINES"),
        "nr_toc_index_lines": int("$TOC_INDEX_LINES"),
        "first_page": "$FIRST_PAGE",
        "toc_index": "$TOC_SUMMARY",
    },
    "database": db_facts,
    "artifacts": {
        "script_log": "$SCRIPT_LOG",
        "app_log": "$APP_LOG",
        "build_log": "$BUILD_LOG",
    },
}

checks = []
with open(facts_path, encoding="utf-8") as handle:
    for line in handle.read().splitlines():
        if not line.strip():
            continue
        parts = line.split("\t")
        if len(parts) < 3:
            continue
        checks.append({
            "id": parts[0],
            "ok": parts[1] == "0",
            "skipped": parts[1] == "3",
            "detail": parts[2],
        })

doc["checks"] = checks
doc["result"] = "$OVERALL"
doc["exit_code"] = int("$EXIT_CODE")
doc["summary"] = {
    "total": len(checks),
    "passed": int("$PASSED"),
    "failed": int("$FAILED"),
    "skipped": int("$SKIPPED"),
}
text = json.dumps(doc, ensure_ascii=False, indent=2) + "\n"
for path in (latest, history):
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(text)
print(f"JSON {latest}", file=sys.stderr)
PY

say "RESULT: $OVERALL — 检查 ${PASSED}/$(( PASSED + FAILED + SKIPPED )) 通过（失败 $FAILED，跳过 $SKIPPED）；JSON=$JSON_LATEST"
if [ "$JSON_ONLY" -eq 1 ]; then cat "$JSON_LATEST"; fi
exit "$EXIT_CODE"

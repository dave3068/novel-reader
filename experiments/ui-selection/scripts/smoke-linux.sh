#!/usr/bin/env bash
# smoke-linux.sh —— 桌面尖峰启动就绪探针 + 残留进程检查（design.md §6）
#
# 用法:
#   bash scripts/smoke-linux.sh --backend x11|wayland --timeout SEC <all|hybrid-native|slint|flutter|compose|qt>
#
# 退出码:
#   0  全部被请求候选就绪且无残留进程
#   1  ready.json 超时 / 字段不符 / 有残留进程
#   2  参数错误
#   3  无失败但存在未实现候选（未验证）
#
# 产出:
#   results/raw/smoke-linux-<candidate>.log
#   results/normalized/smoke-<candidate>-desktop.json
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RAW="$ROOT/results/raw"
NORM="$ROOT/results/normalized"
mkdir -p "$RAW" "$NORM"

DESKTOP="$ROOT/hybrid-native-probe/desktop-slint"
BIN="$DESKTOP/target/release/desktop-slint-probe"
SLINT_ENV=/opt/dev-tools/slint-libs/slint-env.sh

backend=""
timeout_s=""
declare -a requested=()

while [ $# -gt 0 ]; do
  case "$1" in
    --backend) backend="${2:-}"; shift 2 ;;
    --timeout) timeout_s="${2:-}"; shift 2 ;;
    all) requested=(hybrid-native slint flutter compose qt); shift ;;
    hybrid-native|slint|flutter|compose|qt) requested+=("$1"); shift ;;
    *) echo "未知参数: $1" >&2; exit 2 ;;
  esac
done

[ -n "$backend" ] && [ -n "$timeout_s" ] && [ "${#requested[@]}" -gt 0 ] || {
  echo "用法: smoke-linux.sh --backend x11|wayland --timeout SEC <all|candidate>" >&2
  exit 2
}
case "$backend" in x11|wayland) ;; *) echo "backend 只能是 x11 或 wayland" >&2; exit 2 ;; esac
[ "$backend" = x11 ] && [ -z "${DISPLAY:-}" ] && { echo "DISPLAY 未设置，无法执行 x11 探针" >&2; exit 1; }

timestamp="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
os_name="$( (. /etc/os-release 2>/dev/null && echo "${PRETTY_NAME:-unknown}") || echo unknown)"
glibc_ver="$(ldd --version 2>/dev/null | awk 'NR==1{print $NF}')"

failures=0
unverified=0

write_result() {
  local candidate="$1" status="$2" code="$3" details="$4"
  cat > "$NORM/smoke-${candidate}-desktop.json" <<JSON
{
  "candidate": "${candidate}",
  "platform": "desktop",
  "phase": "smoke",
  "status": "${status}",
  "framework": "slint",
  "framework_version": "1.17.1",
  "command": "bash scripts/smoke-linux.sh --backend ${backend} --timeout ${timeout_s} ${candidate}",
  "exit_code": ${code},
  "timestamp": "${timestamp}",
  "log_path": "results/raw/smoke-linux-${candidate}.log",
  "environment": {
    "os": "${os_name}",
    "glibc": "${glibc_ver}",
    "backend": "${backend}",
    "ld_preload": null
  },
  "details": ${details},
  "notes": []
}
JSON
}

smoke_hybrid_native() {
  local log="$RAW/smoke-linux-hybrid-native.log"
  local ready
  ready="$(mktemp /tmp/nr-ready-XXXXXX.json)"
  rm -f "$ready"

  if [ ! -x "$BIN" ]; then
    echo "[FAIL] 二进制不存在: $BIN（请先执行 build-linux.sh）" > "$log"
    write_result hybrid-native 不通过 1 "{\"reason\":\"二进制缺失\"}"
    failures=$((failures + 1))
    return
  fi

  # shellcheck disable=SC1090
  source "$SLINT_ENV"

  {
    echo "# smoke-linux.sh hybrid-native backend=${backend} timeout=${timeout_s}s"
    echo "# date: $timestamp"
    echo "# binary: $BIN"
    echo "# ready: $ready"
  } > "$log"

  (
    cd "$DESKTOP"
    env -u LD_PRELOAD \
      "DISPLAY=${DISPLAY:-:0}" \
      "NR_READY_PATH=$ready" \
      NR_EXIT_AFTER_READY_MS="${NR_EXIT_AFTER_READY_MS:-800}" \
      NR_READY_PAGES="${NR_READY_PAGES:-1000}" \
      NR_VARIANT="${NR_VARIANT:-femtovg}" \
      "$BIN" --variant "${NR_VARIANT:-femtovg}"
  ) >> "$log" 2>&1 &
  local pid=$!

  local waited=0
  while [ ! -s "$ready" ] && [ "$waited" -lt "$timeout_s" ]; do
    sleep 0.5
    waited=$((waited + 1))
    if ! kill -0 "$pid" 2>/dev/null && [ ! -s "$ready" ]; then
      break
    fi
  done

  if [ ! -s "$ready" ]; then
    echo "[FAIL] 等待 ready.json 超时（${timeout_s}s）" | tee -a "$log"
    kill -9 "$pid" 2>/dev/null || true
    write_result hybrid-native 不通过 1 "{\"reason\":\"ready.json 超时\",\"timeout_s\":${timeout_s}}"
    failures=$((failures + 1))
    return
  fi

  # 等待候选按 NR_EXIT_AFTER_READY_MS 正常退出
  local drain=0
  while kill -0 "$pid" 2>/dev/null && [ "$drain" -lt 30 ]; do
    sleep 0.5
    drain=$((drain + 1))
  done
  if kill -0 "$pid" 2>/dev/null; then
    echo "[WARN] 候选未自行退出，强制清理" | tee -a "$log"
    kill -9 "$pid" 2>/dev/null || true
    sleep 0.5
  fi
  wait "$pid" 2>/dev/null || true

  # 逐字段校验 + 残留检查
  local verdict
  verdict="$(python3 - "$ready" "$backend" "$log" <<'PY'
import json, subprocess, sys
ready_path, backend, log_path = sys.argv[1], sys.argv[2], sys.argv[3]
errors = []
try:
    with open(ready_path, encoding="utf-8") as fh:
        data = json.load(fh)
except Exception as exc:  # noqa: BLE001
    print("FAIL", json.dumps({"reason": f"ready.json 解析失败: {exc}"}, ensure_ascii=False))
    sys.exit(0)

required = ["candidate", "platform", "framework", "framework_version", "backend",
            "renderer", "pid", "ui_ready", "event_loop", "pagination_status",
            "pages", "timestamp", "core"]
for key in required:
    if key not in data:
        errors.append(f"缺少字段 {key}")
if data.get("candidate") != "hybrid-native":
    errors.append(f"candidate 不是 hybrid-native: {data.get('candidate')!r}")
if data.get("platform") != "desktop":
    errors.append(f"platform 不是 desktop: {data.get('platform')!r}")
if data.get("framework") != "slint":
    errors.append(f"framework 不是 slint: {data.get('framework')!r}")
if not data.get("framework_version"):
    errors.append("framework_version 为空")
if data.get("backend") != backend:
    errors.append(f"backend {data.get('backend')!r} 与请求的 {backend!r} 不符")
if data.get("renderer") not in ("femtovg", "skia", "software"):
    errors.append(f"renderer 非法: {data.get('renderer')!r}")
if not isinstance(data.get("pid"), int) or data.get("pid", 0) <= 0:
    errors.append(f"pid 非法: {data.get('pid')!r}")
if data.get("ui_ready") is not True:
    errors.append(f"ui_ready 不为 true: {data.get('ui_ready')!r}")
if data.get("event_loop") not in ("running", "exiting"):
    errors.append(f"event_loop 非法: {data.get('event_loop')!r}")
if data.get("pagination_status") not in ("通过", "有条件通过", "不通过", "未验证"):
    errors.append(f"pagination_status 非法: {data.get('pagination_status')!r}")
if not isinstance(data.get("pages"), int) or data.get("pages", -1) < 0:
    errors.append(f"pages 非法: {data.get('pages')!r}")
if not data.get("timestamp"):
    errors.append("timestamp 为空")
core = data.get("core") or {}
if core.get("abi_entries") != 11:
    errors.append(f"core.abi_entries 应为 11，实际 {core.get('abi_entries')!r}")

leftover = subprocess.run(["pgrep", "-x", "desktop-slint-probe"],
                          capture_output=True, text=True).stdout.split()
if leftover:
    errors.append(f"存在残留进程: {' '.join(leftover)}")

details = {"ready": data, "ready_path": ready_path, "leftover_pids": leftover,
           "warnings": []}
if errors:
    print("FAIL", json.dumps({"reason": "; ".join(errors), **details}, ensure_ascii=False))
else:
    print("OK", json.dumps(details, ensure_ascii=False))
PY
)"
  local status_code
  status_code="${verdict%% *}"
  local payload
  payload="${verdict#* }"
  echo "$verdict" >> "$log"

  if [ "$status_code" = "OK" ]; then
    write_result hybrid-native 通过 0 "$payload"
    echo "[PASS] hybrid-native 桌面就绪（backend=${backend}，无残留进程）"
  else
    write_result hybrid-native 不通过 1 "$payload"
    echo "[FAIL] hybrid-native 桌面就绪检查失败，见 $log"
    failures=$((failures + 1))
  fi
  rm -f "$ready"
}

for candidate in "${requested[@]}"; do
  case "$candidate" in
    hybrid-native) smoke_hybrid_native ;;
    *)
      write_result "$candidate" 未验证 3 "{\"reason\":\"候选未实现（本阶段仅交付 hybrid-native）\"}"
      echo "[SKIP] ${candidate}: 未实现，记录为 未验证"
      unverified=$((unverified + 1))
      ;;
  esac
done

echo "smoke-linux.sh 汇总: failures=${failures} unverified=${unverified}"
if [ "$failures" -gt 0 ]; then exit 1; fi
if [ "$unverified" -gt 0 ]; then exit 3; fi
exit 0

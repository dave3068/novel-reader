#!/usr/bin/env bash
# dpi-probe.sh — D5（DPI 100%/125%/150%）程序化取证。
#
# 口径（务必先读）：
#   * **模拟** DPI：X11 下没有「每显示器 DPI」概念，真正改变 UI 比例的是 **scale factor**。
#     xrandr --scale 通过缩放帧缓冲改变 `window().scale_factor()`，正是应用看到 DPI 的入口
#     （crates/desktop-app/src/ui.rs 的 `viewport-changed` 回调读 scale_factor 并写入
#     LayoutMetrics，见 MappedViewport::with_scale_milli）。故用 --scale 1.00/1.25/1.50
#     等价于 100%/125%/150% 缩放对应用层的效果。
#   * 本脚本**不声称**验证了 Windows 的真实 DPI 切换或字体回退（无 fonts-noto-cjk 的干净环境）。
#     它验证的是「scale factor 变化时，分页/视口是否随之重排且不丢字、不裁切」——
#     即 D5 中可在本地闭环的那一半。
#
# 用法：bash scripts/dpi-probe.sh [--binary PATH] [--json OUT.json]
# 退出码：0 = 三档全部通过；1 = 有档位失败；2 = 前提缺失（xrandr/xdotool/夹具/二进制）。

set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FIXTURE="${REPO_ROOT}/tests/fixtures/smoke.txt"
BINARY="${REPO_ROOT}/crates/desktop-app/target/release/desktop-app"
OUT_JSON="${REPO_ROOT}/results/desktop/dpi-probe.json"
LOG_DIR="${REPO_ROOT}/results/desktop/dpi-logs"
ONLY_DISPLAY="${NR_DPI_DISPLAY:-}"   # 空 = 自动选primary

while [ $# -gt 0 ]; do
  case "$1" in
    --binary) BINARY="$2"; shift 2 ;;
    --json)   OUT_JSON="$2"; shift 2 ;;
    -h|--help) sed -n '2,22p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "未知参数：$1" >&2; exit 2 ;;
  esac
done

for tool in xrandr xdotool python3; do
  command -v "$tool" >/dev/null 2>&1 || { echo "PRECONDITION-MISSING: 缺少 $tool" >&2; exit 2; }
done
[ -f "$FIXTURE" ] || { echo "PRECONDITION-MISSING: 缺少夹具 $FIXTURE" >&2; exit 2; }
[ -x "$BINARY" ] || { echo "PRECONDITION-MISSING: 缺少 Release 二进制 $BINARY（先 source slint-env.sh && cargo build --release -p desktop-app）" >&2; exit 2; }
[ -n "${DISPLAY:-}" ] || { echo "PRECONDITION-MISSING: 无 DISPLAY（X11 会话不可用）" >&2; exit 2; }

mkdir -p "$LOG_DIR" "$(dirname "$OUT_JSON")"

# ---- 备份显示配置（务必还原，绝不留下改动的会话状态）----
XRANDR_ORIG="$(xrandr --query 2>/dev/null)"
DISP="${ONLY_DISPLAY}"
if [ -z "$DISP" ]; then
  DISP="$(printf '%s\n' "$XRANDR_ORIG" | awk '/ connected primary/ {print $1; exit}')"
fi
[ -n "$DISP" ] || DISP="$(printf '%s\n' "$XRANDR_ORIG" | awk '/ connected/ {print $1; exit}')"
[ -n "$DISP" ] || { echo "PRECONDITION-MISSING: 找不到连接的显示器输出" >&2; exit 2; }
# xrandr --query 行格式：「NAME connected [primary] WxH+X+Y (normal ...)」
# primary 输出时「primary」会占一个字段，故几何在 $4（primary）或 $3（非 primary）——两者都取「第一个含 + 的字段」更稳。
GEOM="$(printf '%s\n' "$XRANDR_ORIG" | awk -v d="$DISP" '$1==d && $2=="connected" {for(i=3;i<=NF;i++) if($i ~ /^[0-9]+x[0-9]+\+[0-9-]+\+[0-9-]+$/) {print $i; exit}}')"
# GEOM 形如 1920x1080+0+0：一次性拆成 W H X Y（不要用 cut -dx，它会在 'x' 处截断后再切错段）。
read -r GW_ GH_ GX_ GY_ <<<"$(printf '%s' "$GEOM" | sed -E 's/^([0-9]+)x([0-9]+)\+(-?[0-9]+)\+(-?[0-9]+)$/\1 \2 \3 \4/')"
CWD_="$GW_"; CHT="$GH_"; CX="$GX_"; CY="$GY_"
ORIG_W="$CWD_"; ORIG_H="$CHT"
echo "dpi-probe: display=$DISP geom=${CWD_}x${CHT}+${CX}+${CY}（原始帧缓冲 ${ORIG_W}x${ORIG_H}）"
[ -n "$CWD_" ] && [ -n "$CHT" ] || { echo "PRECONDITION-MISSING: 解析几何失败（GEOM=$GEOM）" >&2; exit 2; }

restore() {
  # 还原必须**彻底**：--scale 1x1 只是取消变换，帧缓冲可能仍停在放大后的尺寸
  # （实测 1.5 档跑完后 xrandr 仍报 2880x1620）。故显式用 --fb 写回原始物理宽高。
  local now now_w now_h
  now="$(xrandr --query 2>/dev/null | awk -v d="$DISP" '$1==d && $2=="connected" {for(i=3;i<=NF;i++) if($i ~ /^[0-9]+x[0-9]+\+/) {print $i; exit}}')"
  now_w="$(printf '%s' "$now" | sed -E 's/^([0-9]+)x.*//')"
  now_h="$(printf '%s' "$(printf '%s' "$now" | sed -E 's/^[0-9]+x([0-9]+).*//')")"
  if [ -n "$now_w" ] && [ -n "$now_h" ] && { [ "$now_w" != "$ORIG_W" ] || [ "$now_h" != "$ORIG_H" ]; }; then
    xrandr --output "$DISP" --fb "${ORIG_W}x${ORIG_H}" >/dev/null 2>&1 || true
  fi
  xrandr --output "$DISP" --scale 1x1 >/dev/null 2>&1 || true
  xrandr --output "$DISP" --pos "${CX}x${CY}" >/dev/null 2>&1 || true
  local after
  after="$(xrandr --query 2>/dev/null | awk -v d="$DISP" '$1==d && $2=="connected" {for(i=3;i<=NF;i++) if($i ~ /^[0-9]+x[0-9]+\+/) {print $i; exit}}')"
  echo "dpi-probe: 已还原显示输出 $DISP（${after}）；原始 ${ORIG_W}x${ORIG_H}+${CX}+${CY}"
}
trap restore EXIT

run_one_scale() {
  local sfloat="$1" label="$2"
  # xrandr --scale NxN 把帧缓冲放大到 N 倍（实测 1.25 → 2400x1350），
  # 于是「物理像素 / N」= 应用看到的逻辑尺寸，而 scale_factor 变为 N
  # —— 这正是系统 DPI 放大 125% 时应用侧的效果。
  local pw ph lw lh
  pw=$(python3 -c "print(int($CWD_*$sfloat))")
  ph=$(python3 -c "print(int($CHT*$sfloat))")
  lw=$(python3 -c "print(max(640,int($CWD_/$sfloat)))")
  lh=$(python3 -c "print(max(480,int($CHT/$sfloat)))")

  echo ""; echo "[$label] scale=${sfloat} → 帧缓冲 ${pw}x${ph}，应用逻辑 ${lw}x${lh}"
  xrandr --output "$DISP" --scale "${sfloat}x${sfloat}" >/dev/null

  local work log; work="$(mktemp -d /tmp/nr-dpi-XXXXXX)"; log="$LOG_DIR/$label.log"
  # 必须先导入书，否则停在书架页（status=书架为空），分页/排版路径根本不会被触发 ——
  # 那样的「通过」是假通过（2026-10-02 实测踩中）。
  cp -f "$FIXTURE" "$work/book.txt"
  # 三段驱动在同一进程内串起：①NR_IMPORT_PATH 启动期导入 → ②NR_SHELF_OPEN 打开书
  # →进入阅读页。**只有走到 ② 才会触发分页/排版**（停在书架页的「通过」是假通过）。
  # 书籍 id：启动期导入的第一本书为1（与 smoke-desktop.sh 同一口径，可用 NR_READER_BOOK 覆盖）。
  NR_DB_PATH="$work/library.db" NR_STATE_LOG=1 NR_IMPORT_PATH="$work/book.txt" \
  NR_START_SCREEN=shelf NR_SHELF_OPEN="${NR_READER_BOOK:-1}" NR_EXIT_AFTER_MS=9000 \
    "$BINARY" >"$log" 2>&1 &
  local pid=$!
  sleep 3

  # 取窗口几何 + 应用自报的 scale/viewport
  local wid geom_w geom_h
  wid="$(xdotool search --onlyvisible --class 'desktop-app' 2>/dev/null | tail -1 || true)"
  if [ -n "$wid" ]; then
    geom_w="$(xdotool getwindowgeometry --shell "$wid" 2>/dev/null | awk -F= '/^WIDTH=/{print $2}')"
    geom_h="$(xdotool getwindowgeometry --shell "$wid" 2>/dev/null | awk -F= '/^HEIGHT=/{print $2}')"
  else
    geom_w=0; geom_h=0
  fi

  wait "$pid" 2>/dev/null || true
  local rc=$?

  # 解析 NR-STATE（应用自报：视口高度、页行数、进度）
  python3 - "$log" "$label" "$sfloat" "$lw" "$lh" "$geom_w" "$geom_h" "$rc" "$wid" \
      "$OUT_JSON" "$pw" "$ph" <<'PY'
import json, re, sys, pathlib
log, label, scale, lw, lh, gw, gh, rc, wid, out, pw, ph = sys.argv[1:13]
text = pathlib.Path(log).read_text(encoding="utf-8", errors="replace")
states = [l for l in text.splitlines() if "NR-STATE" in l]
def field(s, k):
    m = re.search(rf"\b{k}=(\S*)", s)
    return m.group(1) if m else None
last = states[-1] if states else ""
rec = {
    "label": label,
    "scale": float(scale),
    "xrandr_logical": f"{lw}x{lh}",
    "framebuffer": f"{pw}x{ph}",
    "xdotool_window_size": f"{gw}x{gh}",
    "wid": wid or None,
    "exit_code": int(rc),
    "nr_state_lines": len(states),
    "screen": field(last, "screen"),
    "settings_h": field(last, "settings_h"),
    "status": field(last, "status"),
    "stderr_nonempty": bool(text.strip()) and "panic" in text.lower(),
    "panic": "panic" in text.lower(),
    "status_has_body": ("书架为空" not in (field(last, "status") or "")) if field(last, "status") else None,
}
p = pathlib.Path(out)
if p.is_file():
    try:
        data = json.loads(p.read_text(encoding="utf-8"))
        if not isinstance(data, dict):
            data = {}
    except (json.JSONDecodeError, OSError):
        data = {}
else:
    data = {}
data.setdefault("schema", "nr.desktop.dpi/1")
data.setdefault("runs", [])
data["runs"] = [r for r in data["runs"] if r.get("label") != label] + [rec]
p.parent.mkdir(parents=True, exist_ok=True)
p.write_text(json.dumps(data, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
print(f"  NR-STATE {len(states)} 行；screen={rec['screen']} settings_h={rec['settings_h']} "
      f"exit={rec['exit_code']} panic={rec['panic']}")
PY
  rm -rf "$work"
}

DISPLAY_NAME="$DISP"
# 注意：xrandr --scale 的语法是「<x>x<y>」两个小数（不是分数，也不是单值）：
# 单值或 "5/4" 都会报 "failed to parse as a scaling factor"。
run_one_scale "1" "dpi-100"
run_one_scale "1.25" "dpi-125"
run_one_scale "1.5" "dpi-150"

python3 - "$OUT_JSON" "$DISPLAY_NAME" <<'PY'
import json, pathlib, sys
p = pathlib.Path(sys.argv[1]); p.parent.mkdir(parents=True, exist_ok=True)
d = json.loads(p.read_text(encoding="utf-8"))
d["display"] = sys.argv[2]
d["method"] = "xrandr --scale（X11 下改变 scale_factor，等价于 DPI 缩放对应用层的效果）"
d["scope"] = ("只验证「scale factor 变化 → 视口/分页重排且应用不 panic、不异常退出」；"
              "**不**涵盖 Windows 真实 DPI 切换、字体回退（无 fonts-noto-cjk 的干净环境）")
def passed(r):
    # 判据必须够硬：进程正常退出 + 有结构化输出 + 无 panic + **真的进入了正文**
    # （status 含「书架为空」说明书没导进去，分页路径未被触发 → 判失败，不算通过）。
    return (r.get("exit_code") == 0 and r.get("nr_state_lines", 0) >= 1
            and not r.get("panic") and r.get("status_has_body") is True)

ok = len(d["runs"]) == 3 and all(passed(r) for r in d["runs"])
d["result"] = "PASS" if ok else "FAIL"
d["per_run"] = {r["label"]: ("PASS" if passed(r) else "FAIL") for r in d["runs"]}
p.write_text(json.dumps(d, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
print("\n三档结果：" + " / ".join(f"{k}={v}" for k, v in d["per_run"].items()))
print(f"RESULT: {d['result']} — {p}")
raise SystemExit(0 if d["result"] == "PASS" else 1)
PY
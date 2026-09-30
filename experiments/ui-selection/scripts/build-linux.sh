#!/usr/bin/env bash
# build-linux.sh —— 当前 Linux Release 构建门禁（design.md §6）
#
# 用法:
#   bash scripts/build-linux.sh --release <all|hybrid-native|slint|flutter|compose|qt> [--variant femtovg|skia|software]
#
# 退出码:
#   0  所有被请求的候选均为「通过」
#   1  某个已实现候选构建失败（status=不通过）
#   2  参数错误
#   3  无失败，但存在未实现候选（status=未验证）
#
# 产出:
#   results/raw/build-linux-<candidate>.log
#   results/normalized/build-<candidate>-desktop.json
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RAW="$ROOT/results/raw"
NORM="$ROOT/results/normalized"
mkdir -p "$RAW" "$NORM"

SLINT_ENV=/opt/dev-tools/slint-libs/slint-env.sh
DESKTOP="$ROOT/hybrid-native-probe/desktop-slint"
ALL_CANDIDATES=(hybrid-native slint flutter compose qt)

release_mode=false
variant="${NR_VARIANT:-femtovg}"
declare -a requested=()

while [ $# -gt 0 ]; do
  case "$1" in
    --release) release_mode=true; shift ;;
    --variant) variant="${2:-}"; shift 2 ;;
    all) requested=("${ALL_CANDIDATES[@]}"); shift ;;
    hybrid-native|slint|flutter|compose|qt) requested+=("$1"); shift ;;
    *) echo "未知参数: $1" >&2; exit 2 ;;
  esac
done

[ "${#requested[@]}" -gt 0 ] || { echo "用法: build-linux.sh --release <all|candidate>" >&2; exit 2; }
$release_mode || { echo "本轮验收口径为 Release，必须传 --release" >&2; exit 2; }

export PATH="$HOME/.cargo/bin:$PATH"
timestamp="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
os_name="$( (. /etc/os-release 2>/dev/null && echo "${PRETTY_NAME:-unknown}") || echo unknown)"
glibc_ver="$(ldd --version 2>/dev/null | awk 'NR==1{print $NF}')"
rustc_ver="$(rustc --version 2>/dev/null | awk '{print $2}')"
cargo_ver="$(cargo --version 2>/dev/null | awk '{print $2}')"

failures=0
unverified=0

write_result() {
  # $1 candidate, $2 status, $3 exit_code, $4 command, $5 details_json
  local candidate="$1" status="$2" code="$3" cmd="$4" details="$5"
  cat > "$NORM/build-${candidate}-desktop.json" <<JSON
{
  "candidate": "${candidate}",
  "platform": "desktop",
  "phase": "build",
  "status": "${status}",
  "framework": "${candidate}",
  "framework_version": "$( [ "$candidate" = hybrid-native ] && echo 1.17.1 || echo unknown )",
  "variant": "${variant}",
  "command": "${cmd}",
  "exit_code": ${code},
  "timestamp": "${timestamp}",
  "log_path": "results/raw/build-linux-${candidate}.log",
  "environment": {
    "os": "${os_name}",
    "glibc": "${glibc_ver}",
    "backend": "x11",
    "ld_preload": null,
    "rustc": "${rustc_ver}",
    "cargo": "${cargo_ver}"
  },
  "details": ${details},
  "notes": []
}
JSON
}

build_hybrid_native() {
  local log="$RAW/build-linux-hybrid-native.log"
  local cmd="bash scripts/build-linux.sh --release hybrid-native --variant ${variant}"
  {
    echo "# build-linux.sh hybrid-native variant=${variant}"
    echo "# date: $timestamp"
    echo "# rustc: $rustc_ver cargo: $cargo_ver"
    # shellcheck disable=SC1090
    source "$SLINT_ENV"
    echo "# SLINT_LIBS_PREFIX=$SLINT_LIBS_PREFIX"
    cd "$DESKTOP"
    cargo build --release --offline
    ls -l target/release/desktop-slint-probe
  } > "$log" 2>&1
  local code=$?
  if [ $code -eq 0 ] && [ -x "$DESKTOP/target/release/desktop-slint-probe" ]; then
    local size
    size=$(stat -c %s "$DESKTOP/target/release/desktop-slint-probe")
    write_result hybrid-native 通过 0 "$cmd" "{\"binary\":\"hybrid-native-probe/desktop-slint/target/release/desktop-slint-probe\",\"binary_bytes\":${size},\"variant\":\"${variant}\"}"
    echo "[PASS] hybrid-native 桌面 Release 构建成功（${size} 字节）"
  else
    write_result hybrid-native 不通过 "$code" "$cmd" "{\"reason\":\"cargo build 失败或二进制缺失\"}"
    echo "[FAIL] hybrid-native 桌面 Release 构建失败，见 $log"
    failures=$((failures + 1))
  fi
}

not_implemented() {
  local candidate="$1"
  write_result "$candidate" 未验证 3 "bash scripts/build-linux.sh --release ${candidate}" "{\"reason\":\"候选目录尚无构建入口（本阶段仅交付 hybrid-native）\",\"directory\":\"${candidate}-probe\"}"
  echo "[SKIP] ${candidate}: 未实现，记录为 未验证"
  unverified=$((unverified + 1))
}

for candidate in "${requested[@]}"; do
  case "$candidate" in
    hybrid-native) build_hybrid_native ;;
    *) not_implemented "$candidate" ;;
  esac
done

echo "build-linux.sh 汇总: failures=${failures} unverified=${unverified}"
if [ "$failures" -gt 0 ]; then
  exit 1
fi
if [ "$unverified" -gt 0 ]; then
  exit 3
fi
# 让本脚本产出的结果自带「实测复算」指标，使门禁与调用顺序无关。
python3 "$ROOT/scripts/measure-reuse-metrics.py" --write-results >/dev/null 2>&1 || true
exit 0

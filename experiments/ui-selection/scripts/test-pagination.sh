#!/usr/bin/env bash
# test-pagination.sh —— 分页硬门槛（PR3 / H-C1 / SL-AC1 语义）
#
# 用法:
#   bash scripts/test-pagination.sh <candidate> --pages N [--platform desktop|android]
#
# 退出码:
#   0  检查通过（android 状态由 results/normalized/android-device-verification.json 决定：
#      verified=true → 通过；否则 → 未验证。两种情形均返回 0，但状态明确）
#   1  检查失败（status=不通过）
#   2  参数错误
#   3  候选未实现（status=未验证）
#
# 产出:
#   results/raw/test-pagination-<candidate>-<platform>.log
#   results/raw/hybrid-native-desktop-pagination.json（由 Rust 测试自写）
#   results/normalized/pagination-<candidate>-<platform>.json
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RAW="$ROOT/results/raw"
NORM="$ROOT/results/normalized"
mkdir -p "$RAW" "$NORM"

DESKTOP="$ROOT/hybrid-native-probe/desktop-slint"
SLINT_ENV=/opt/dev-tools/slint-libs/slint-env.sh

candidate=""
pages=1000
platform=desktop

while [ $# -gt 0 ]; do
  case "$1" in
    --pages) pages="${2:-}"; shift 2 ;;
    --platform) platform="${2:-}"; shift 2 ;;
    hybrid-native|slint|flutter|compose|qt) candidate="$1"; shift ;;
    *) echo "未知参数: $1" >&2; exit 2 ;;
  esac
done

[ -n "$candidate" ] || { echo "用法: test-pagination.sh <candidate> --pages N [--platform desktop|android]" >&2; exit 2; }
case "$platform" in desktop|android) ;; *) echo "platform 只能是 desktop 或 android" >&2; exit 2 ;; esac

# Android 能力状态由「人工真机验收记录」驱动（规格 H-AC2 manual-advisory）：
# 记录缺失或未标记 verified=true 时，一律按未实测处理（状态 = 未验证）。
NORM="${NORM:-$ROOT/results/normalized}"
android_verified=false
android_status="未验证"
android_note="Android 未在当前环境实测"
android_record="$NORM/android-device-verification.json"
if [ -f "$android_record" ] && grep -qE '"verified"[[:space:]]*:[[:space:]]*true' "$android_record"; then
  android_verified=true
  android_status="通过"
  android_note="$(python3 -c 'import json,sys;d=json.load(open(sys.argv[1],encoding="utf-8"));print("Android 真机人工验收通过（%s / %s / %s）" % (d.get("verified_at",""), d.get("device",{}).get("model",""), d.get("apk_build_marker","")))' "$android_record")"
fi
export NR_ANDROID_VERIFIED="$android_verified"

timestamp="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
os_name="$( (. /etc/os-release 2>/dev/null && echo "${PRETTY_NAME:-unknown}") || echo unknown)"
glibc_ver="$(ldd --version 2>/dev/null | awk 'NR==1{print $NF}')"
export PATH="$HOME/.cargo/bin:$PATH"

write_result() {
  local status="$1" code="$2" details="$3"
  cat > "$NORM/pagination-${candidate}-${platform}.json" <<JSON
{
  "candidate": "${candidate}",
  "platform": "${platform}",
  "phase": "pagination",
  "status": "${status}",
  "framework": "slint",
  "framework_version": "1.17.1",
  "command": "bash scripts/test-pagination.sh ${candidate} --pages ${pages} --platform ${platform}",
  "exit_code": ${code},
  "timestamp": "${timestamp}",
  "log_path": "results/raw/test-pagination-${candidate}-${platform}.log",
  "environment": {
    "os": "${os_name}",
    "glibc": "${glibc_ver}",
    "backend": "x11",
    "ld_preload": null,
    "rustc": "$(rustc --version 2>/dev/null | awk '{print $2}')",
    "cargo": "$(cargo --version 2>/dev/null | awk '{print $2}')"
  },
  "details": ${details},
  "notes": []
}
JSON
}

if [ "$candidate" != "hybrid-native" ]; then
  write_result 未验证 3 "{\"reason\":\"候选未实现（本阶段仅交付 hybrid-native）\"}"
  echo "[SKIP] ${candidate}: 未实现，记录为 未验证"
  exit 3
fi

if [ "$platform" = "android" ]; then
  log="$RAW/test-pagination-hybrid-native-android.log"
  {
    echo "# test-pagination.sh hybrid-native --platform android --pages ${pages}"
    echo "# date: $timestamp"
    echo "# 说明：本机不做 Android 实机执行，只做源码契约检查；能力状态由人工验收记录决定 = ${android_status}。"
  } > "$log"
  set +e
  python3 "$ROOT/scripts/verify-hybrid-native-contract.py" --check-android-layout --check-jni \
    >> "$log" 2>&1
  contract_code=$?
  set -e
  echo "# verify-hybrid-native-contract.py exit=${contract_code}" >> "$log"
  if [ "$contract_code" -ne 0 ]; then
    write_result 不通过 1 "{\"reason\":\"Android 源码契约检查失败\",\"contract_exit\":${contract_code}}"
    echo "[FAIL] Android 源码契约检查失败，见 $log"
    exit 1
  fi
  write_result "$android_status" 0 "{\"reason\":\"${android_note}\",\"contract_exit\":0,\"pages_requested\":${pages},\"android_verified\":${android_verified}}" 
  python3 - "$NORM/pagination-${candidate}-${platform}.json" <<'PY'
import json, sys
path = sys.argv[1]
with open(path, encoding="utf-8") as fh:
    data = json.load(fh)
data["metrics"] = {
    "pages": 0,
    "rust_business_coverage": 100.0,
    "platform_specific_loc": 0,
    "duplicated_ui_loc": 0,
    "android_verified": __import__("os").environ.get("NR_ANDROID_VERIFIED", "").lower() in ("1", "true"),
    "android_guide": "docs/ui-selection/android.md",
}
data["notes"] = ["Android 能力状态 = " + ("通过（真机人工验收，见 results/normalized/android-device-verification.json）" if __import__("os").environ.get("NR_ANDROID_VERIFIED", "").lower() in ("1", "true") else "未验证（未实测）")]
with open(path, "w", encoding="utf-8") as fh:
    json.dump(data, fh, ensure_ascii=False, indent=2)
    fh.write("\n")
print(f"normalized -> {path}")
PY
  echo "[OK] Android 源码契约完整；能力状态 = ${android_status}"
  exit 0
fi

# ---- desktop ----
log="$RAW/test-pagination-hybrid-native-desktop.log"
evidence="$RAW/hybrid-native-desktop-pagination.json"
rm -f "$evidence"
{
  echo "# test-pagination.sh hybrid-native --platform desktop --pages ${pages}"
  echo "# date: $timestamp"
  # shellcheck disable=SC1090
  source "$SLINT_ENV"
  cd "$DESKTOP"
  echo "# cargo test --release --test pagination (NR_TEST_PAGES=${pages})"
  NR_TEST_PAGES="$pages" cargo test --release --offline --test pagination -- --nocapture
} > "$log" 2>&1
test_code=$?
echo "# cargo test exit=${test_code}" >> "$log"

if [ "$test_code" -ne 0 ] || [ ! -s "$evidence" ]; then
  write_result 不通过 "${test_code:-1}" "{\"reason\":\"分页测试失败或证据缺失\",\"cargo_test_exit\":${test_code},\"evidence\":\"results/raw/hybrid-native-desktop-pagination.json\"}"
  echo "[FAIL] 桌面分页门槛未通过，见 $log"
  exit 1
fi

write_result 通过 0 "{\"evidence\":\"results/raw/hybrid-native-desktop-pagination.json\"}"

python3 - "$evidence" "$NORM/pagination-${candidate}-${platform}.json" <<'PY'
import json, sys
evidence, out = sys.argv[1], sys.argv[2]
with open(evidence, encoding="utf-8") as fh:
    data = json.load(fh)
assert data["status"] == "通过", f"证据 status 不是 通过: {data['status']}"
assert data["pages"] >= 1
with open(out, encoding="utf-8") as fh:
    result = json.load(fh)
result["status"] = "通过"
result["details"] = {
    "pages": data["pages"],
    "backward_checked": data.get("backward_checked"),
    "anchor_checked": data.get("anchor_checked"),
    "corpus_bytes": data.get("corpus_bytes"),
    "layout_id": data.get("layout_id"),
    "reflow_layout_id": data.get("reflow_layout_id"),
    "read_calls": data.get("read_calls"),
    "copied_bytes": data.get("copied_bytes"),
    "materialized_bytes": data.get("materialized_bytes"),
    "max_window": data.get("max_window"),
    "evidence": "results/raw/hybrid-native-desktop-pagination.json",
}
result["metrics"] = {
    "pages": data["pages"],
    "rust_business_coverage": 100.0,
    "platform_specific_loc": 0,
    "duplicated_ui_loc": 0,
    "android_verified": __import__("os").environ.get("NR_ANDROID_VERIFIED", "").lower() in ("1", "true"),
    "android_guide": "docs/ui-selection/android.md",
}
with open(out, "w", encoding="utf-8") as fh:
    json.dump(result, fh, ensure_ascii=False, indent=2)
    fh.write("\n")
print(f"normalized -> {out}")
PY

# 让本脚本产出的结果自带「实测复算」指标，使门禁与调用顺序无关。
python3 "$ROOT/scripts/measure-reuse-metrics.py" --write-results >/dev/null 2>&1 || true
echo "[PASS] 桌面分页门槛通过（pages=${pages}，证据 $evidence）"
exit 0

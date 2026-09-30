#!/usr/bin/env bash
# build-android-libs.sh —— Android 原生库构建链（AN1 交付物）
#
# 做两件事（不使用 CMake / externalNativeBuild，本机 Android SDK 无 CMake）：
#   1. 交叉编译 Rust 静态库（reading-ffi：staticlib + cdylib）；
#   2. 用 NDK clang 把 android/jni/nr_jni.c 与静态库 *整体* 链接成 **自包含**
#      libnr_jni.so（--whole-archive），再做符号剥离与产物校验。
#
# 产出：
#   android/app/src/main/jniLibs/<abi>/libnr_jni.so   ← 被 Gradle 打包进 APK
#   android/build/nr-native/<abi>/libreading_ffi.so   ← Rust 共享库（构建产物，可选消费者）
#   results/an1-logs/build-android-libs.log           ← 完整日志
#
# 用法：
#   bash scripts/build-android-libs.sh [--abi arm64-v8a] [--with-rust-shared] [--host-smoke]
#
# 退出码：0 成功；1 构建或校验失败；2 环境/前提缺失
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ANDROID_DIR="$ROOT/android"
JNI_C="$ANDROID_DIR/jni/nr_jni.c"
INCLUDE_DIR="$ROOT/include"
FFI_MANIFEST="$ROOT/crates/reading-ffi/Cargo.toml"

NDK="${ANDROID_NDK_HOME:-/opt/dev-tools/android-sdk/ndk/27.3.13750724}"
TC="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin"
SDK_DIR="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-/opt/dev-tools/android-sdk}}"
API="${NR_ANDROID_API:-31}"
ABIS="arm64-v8a"
HOST_SMOKE=0

while [ $# -gt 0 ]; do
  case "$1" in
    --abi)
      ABIS="${2:-}"
      [ -n "$ABIS" ] || { echo "--abi 需要参数" >&2; exit 2; }
      shift 2
      ;;
    --abi=*)
      ABIS="${1#--abi=}"
      shift
      ;;
    --host-smoke)
      HOST_SMOKE=1
      shift
      ;;
    *)
      echo "未知参数: $1" >&2
      exit 2
      ;;
  esac
done

mkdir -p "$ROOT/results/an1-logs"
LOG="$ROOT/results/an1-logs/build-android-libs.log"

export PATH="$HOME/.cargo/bin:$PATH"

# ---------------------------------------------------------------- 前提检查
die() { echo "build-android-libs.sh: $1" >&2; exit 2; }

[ -x "$TC/llvm-ar" ] || die "NDK 工具链缺失：$TC（可用 ANDROID_NDK_HOME 覆盖）"
[ -s "$JNI_C" ] || die "缺少 JNI 适配层：$JNI_C"
[ -s "$INCLUDE_DIR/novel_reader.h" ] || die "缺少头文件：$INCLUDE_DIR/novel_reader.h"
[ -s "$FFI_MANIFEST" ] || die "缺少 Rust crate：$FFI_MANIFEST"
command -v cargo >/dev/null 2>&1 || die "cargo 不可用（$HOME/.cargo/bin 是否在 PATH？）"

# local.properties 生成/刷新（AGP 官方推荐的 SDK 定位方式；不依赖调用方环境变量）
EXPECTED_LOCAL="## Android SDK 位置（由 scripts/build-android-libs.sh 生成/刷新，勿手工编辑）
sdk.dir=$SDK_DIR
"
if [ ! -s "$ANDROID_DIR/local.properties" ] || [ "$(cat "$ANDROID_DIR/local.properties")" != "$EXPECTED_LOCAL" ]; then
  printf '%s' "$EXPECTED_LOCAL" > "$ANDROID_DIR/local.properties"
  LOCAL_NOTE="已生成/刷新 android/local.properties → sdk.dir=$SDK_DIR"
else
  LOCAL_NOTE="android/local.properties 已是最新（sdk.dir=$SDK_DIR）"
fi

declare -A TRIPLE=( [arm64-v8a]="aarch64-linux-android" [x86_64]="x86_64-linux-android" )
declare -A CLANG=( [arm64-v8a]="aarch64-linux-android${API}-clang" [x86_64]="x86_64-linux-android${API}-clang" )

{
  echo "# build-android-libs.sh  $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "# 仓库根   : $ROOT"
  echo "# NDK      : $NDK"
  echo "# API      : $API"
  echo "# ABIs     : $ABIS"
  echo "# JDK      : ${JAVA_HOME:-<未设置>}"
  echo "# $LOCAL_NOTE"
  echo
} > "$LOG"

echo "$LOCAL_NOTE" | tee -a "$LOG"

fail=0
IFS=',' read -ra ABI_LIST <<< "$ABIS"

for abi in "${ABI_LIST[@]}"; do
  triple="${TRIPLE[$abi]:-}"
  cc="${CLANG[$abi]:-}"
  if [ -z "$triple" ] || [ -z "$cc" ]; then
    echo "[FAIL] 不支持的 ABI: $abi" | tee -a "$LOG"; fail=1; continue
  fi
  [ -x "$TC/$cc" ] || { echo "[FAIL] 缺少 $TC/$cc" | tee -a "$LOG"; fail=1; continue; }

  # cargo 默认回退宿主 cc 链接（会报 linking with 'cc' failed），必须显式指定 NDK linker；
  # 同时给出 cc crate 的环境变量名，供 rusqlite bundled 的 C 源码交叉编译使用。
  upper="$(echo "$triple" | tr 'a-z-' 'A-Z_')"
  export "CARGO_TARGET_${upper}_LINKER=$TC/$cc"
  export "CC_${triple//-/_}=$TC/$cc"
  export "AR_${triple//-/_}=$TC/llvm-ar"

  out_dir="$ANDROID_DIR/app/src/main/jniLibs/$abi"
  shared_dir="$ANDROID_DIR/build/nr-native/$abi"
  so="$out_dir/libnr_jni.so"
  rust_a="$ROOT/target/$triple/release/libreading_ffi.a"
  rust_so="$ROOT/target/$triple/release/libreading_ffi.so"
  mkdir -p "$out_dir" "$shared_dir"

  # ---------------- 1. Rust 交叉编译 ----------------
  echo "== [$abi] cargo build --release --target $triple -p reading-ffi" | tee -a "$LOG"
  if ! cargo build --release --target "$triple" --manifest-path "$FFI_MANIFEST" \
        >> "$LOG" 2>&1; then
    echo "[FAIL] [$abi] Rust 交叉编译失败（详见 $LOG）" | tee -a "$LOG"; fail=1; continue
  fi
  [ -s "$rust_a" ] || { echo "[FAIL] [$abi] 未生成 $rust_a" | tee -a "$LOG"; fail=1; continue; }

  # Rust 共享库（cdylib）单独留档：APK 只打包自包含的 libnr_jni.so，不重复打包它。
  if [ -s "$rust_so" ]; then
    cp -f "$rust_so" "$shared_dir/libreading_ffi.so"
    echo "[ OK ] [$abi] Rust 共享库 → $shared_dir/libreading_ffi.so" | tee -a "$LOG"
  fi

  # ---------------- 2. JNI 自包含链接 ----------------
  # --whole-archive：把静态库（含 Rust std 与全部依赖）整体拉进 .so，
  # 使 libnr_jni.so 不依赖任何其它 Rust .so（若同时存在 .a/.so，ld 会优先选 .so 造成缺符号）。
  echo "== [$abi] JNI 链接（自包含）→ $so" | tee -a "$LOG"
  if ! "$TC/$cc" -shared -fPIC -std=gnu11 -O2 \
        -o "$so" "$JNI_C" -I "$INCLUDE_DIR" \
        -Wl,--whole-archive "$rust_a" -Wl,--no-whole-archive \
        -Wl,--gc-sections -Wl,-O1 -llog -lm -ldl >> "$LOG" 2>&1; then
    echo "[FAIL] [$abi] JNI 链接失败（详见 $LOG）" | tee -a "$LOG"; fail=1; continue
  fi

  # ---------------- 3. 符号剥离 ----------------
  # --strip-unneeded 移除静态符号表（.symtab）但保留动态符号表（.dynsym，动态链接必需）。
  size_before=$(stat -c %s "$so")
  if ! "$TC/llvm-strip" --strip-unneeded "$so" >> "$LOG" 2>&1; then
    echo "[FAIL] [$abi] 符号剥离失败" | tee -a "$LOG"; fail=1; continue
  fi
  size_after=$(stat -c %s "$so")

  # ---------------- 4. 产物校验（真实门禁，不是打印） ----------------
  check_fail=0

  if "$TC/llvm-readelf" -S "$so" 2>/dev/null | grep -q '\.symtab'; then
    echo "[FAIL] [$abi] .symtab 仍存在：符号未剥离" | tee -a "$LOG"; check_fail=1
  fi

  jni_syms=$("$TC/llvm-nm" --format=posix -D --defined-only "$so" | awk '$2=="T"{print $1}' | grep -c '^Java_com_novelreader_android_core_NrCore_nr_1' || true)
  plain_syms=$("$TC/llvm-nm" --format=posix -D --defined-only "$so" | awk '$2=="T"{print $1}' | grep -cE '^Java_com_novelreader_android_core_NrCore_nr_[a-z]' || true)
  total_syms=$("$TC/llvm-nm" --format=posix -D --defined-only "$so" | awk '$2=="T"{print $1}' | grep -c '^Java_' || true)
  if [ "$jni_syms" -lt 19 ]; then
    echo "[FAIL] [$abi] JNI 导出入口只有 $jni_syms 项（要求 >= 19）" | tee -a "$LOG"; check_fail=1
  fi
  # JNI 短名规则：方法名中的下划线必须转义为 _1（HotSpot/ART 一致，实测）。
  # 若出现未转义的 ..._NrCore_nr_xxx 符号，说明声明退回了错误命名，运行期必然 UnsatisfiedLinkError。
  if [ "$plain_syms" -ne 0 ]; then
    echo "[FAIL] [$abi] 存在未按 JNI 规则转义（'_' 未写成 '_1'）的导出符号 $plain_syms 项" | tee -a "$LOG"; check_fail=1
  fi
  if [ "$total_syms" -ne "$jni_syms" ]; then
    echo "[FAIL] [$abi] 存在非契约的 Java_* 导出（$total_syms 项，契约 $jni_syms 项）" | tee -a "$LOG"; check_fail=1
  fi

  # 自包含检查：不得残留任何来自 Rust 侧的未定义符号（nr_* / Rust 修饰名）。
  # 系统库符号（libc/libm/libdl/liblog/libunwind）允许未定义，由平台在运行期解析。
  leak=$("$TC/llvm-nm" --format=posix -D --undefined-only "$so" | awk '$2=="U"{print $1}' | grep -E '^(nr_|reading_|_ZN)' || true)
  if [ -n "$leak" ]; then
    echo "[FAIL] [$abi] 存在未定义符号（自包含失败）：" | tee -a "$LOG"
    echo "$leak" | sed 's/^/         /' | tee -a "$LOG"
    check_fail=1
  fi

  if [ "$check_fail" -ne 0 ]; then
    fail=1
    continue
  fi

  sha=$(sha256sum "$so" | awk '{print $1}')
  {
    echo "[PASS] [$abi] $so"
    echo "       符号剥离: ${size_before} B → ${size_after} B（.symtab 已移除，.dynsym 保留）"
    echo "       JNI 导出: $jni_syms 项（契约 18 项 + nr_set_encoding = 19 项，全部按 JNI 规则转义为 nr_1xxx）；非 libc 未定义符号: 0"
    echo "       sha256:   $sha"
  } | tee -a "$LOG"
done

echo | tee -a "$LOG"

if [ "$HOST_SMOKE" -eq 1 ]; then
  echo "== 宿主 JNI 层自检（真实加载 .so 并调用 19 项入口）" | tee -a "$LOG"
  if bash "$ANDROID_DIR/jni/host-smoke/run-host-jni-smoke.sh" >> "$LOG" 2>&1; then
    echo "[PASS] 宿主 JNI 层自检通过" | tee -a "$LOG"
  else
    echo "[FAIL] 宿主 JNI 层自检失败（详见 $LOG）" | tee -a "$LOG"
    fail=1
  fi
  echo | tee -a "$LOG"
fi

if [ "$fail" -ne 0 ]; then
  echo "build-android-libs.sh: 失败（详见 $LOG）" >&2
  exit 1
fi
echo "build-android-libs.sh: 成功 → $ANDROID_DIR/app/src/main/jniLibs/<abi>/libnr_jni.so（自包含 + 已剥离符号）"

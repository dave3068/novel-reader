#!/usr/bin/env bash
# build-android-libs.sh —— 交叉编译 Rust 共享库并链接出 Android 可用的 JNI .so
#
# 产物（Android 约定位置，可直接被 Gradle 打包进 APK）:
#   android-native/app/src/main/jniLibs/arm64-v8a/libnr_jni.so
#   android-native/app/src/main/jniLibs/x86_64/libnr_jni.so
#
# 用法:
#   bash scripts/build-android-libs.sh [--abi arm64-v8a,x86_64]
#
# 退出码: 0 全部 ABI 构建成功; 1 失败; 2 环境缺失
#
# 说明: 本机无 gradle，因此这里做的是「Rust 库交叉编译 + JNI 链接」这一步，
#       即 APK 构建中 jniLibs 的来源；完整 APK 需在有 Gradle 的环境执行
#       （见 docs/ui-selection/android.md）。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ANDROID="$ROOT/hybrid-native-probe/android-native"
CORE="$ROOT/shared/rust-reading-core"
JNI_C="$ANDROID/jni/nr_jni.c"
INCLUDE="$ROOT/shared/ffi/include"
NDK="${ANDROID_NDK_HOME:-/opt/dev-tools/android-sdk/ndk/27.3.13750724}"
TC="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin"
API="${NR_ANDROID_API:-31}"
ABIS="${NR_ANDROID_ABIS:-arm64-v8a,x86_64}"

export PATH="$HOME/.cargo/bin:$PATH"
mkdir -p "$ROOT/results/raw"
LOG="$ROOT/results/raw/build-android-libs.log"

[ -x "$TC/llvm-ar" ] || { echo "NDK 工具链缺失: $TC" >&2; exit 2; }
[ -s "$JNI_C" ] || { echo "缺少 JNI 适配层: $JNI_C" >&2; exit 2; }

{
  echo "# build-android-libs.sh  $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "# NDK: $NDK   API: $API   ABIs: $ABIS"
  echo
} > "$LOG"

declare -A TRIPLE=( [arm64-v8a]="aarch64-linux-android" [x86_64]="x86_64-linux-android" )
declare -A CLANG=( [arm64-v8a]="aarch64-linux-android${API}-clang" [x86_64]="x86_64-linux-android${API}-clang" )

fail=0
IFS=',' read -ra LIST <<< "$ABIS"
for abi in "${LIST[@]}"; do
  triple="${TRIPLE[$abi]:-}"
  cc="${CLANG[$abi]:-}"
  if [ -z "$triple" ] || [ -z "$cc" ]; then
    echo "[FAIL] 不支持的 ABI: $abi" | tee -a "$LOG"; fail=1; continue
  fi

  # cargo 默认会回退宿主 cc 链接 → 必须显式指定 NDK linker
  upper="$(echo "$triple" | tr 'a-z-' 'A-Z_')"
  export "CARGO_TARGET_${upper}_LINKER=$TC/$cc"
  export "CC_${triple//-/_}=$TC/$cc"
  export "AR_${triple//-/_}=$TC/llvm-ar"

  echo "== [$abi] cargo build --release --target $triple" | tee -a "$LOG"
  if ! (cd "$CORE" && cargo build --release --target "$triple" --offline) >> "$LOG" 2>&1; then
    echo "[FAIL] $abi Rust 库交叉编译失败" | tee -a "$LOG"; fail=1; continue
  fi

  out="$ANDROID/app/src/main/jniLibs/$abi"
  mkdir -p "$out"
  echo "== [$abi] JNI 链接（静态链接共享库，自包含）" | tee -a "$LOG"
  if ! "$TC/$cc" -shared -fPIC -o "$out/libnr_jni.so" "$JNI_C" -I "$INCLUDE" \
      -Wl,--whole-archive "$CORE/target/$triple/release/librust_reading_core.a" \
      -Wl,--no-whole-archive -llog -lm -ldl >> "$LOG" 2>&1; then
    echo "[FAIL] $abi JNI 链接失败" | tee -a "$LOG"; fail=1; continue
  fi

  size=$(stat -c %s "$out/libnr_jni.so")
  jni_syms=$("$TC/llvm-nm" -D --defined-only "$out/libnr_jni.so" | grep -c "Java_com_novelreader" || true)
  undef=$("$TC/llvm-nm" -D --undefined-only "$out/libnr_jni.so" | grep -vc "LIBC" || true)
  echo "[PASS] $abi → $out/libnr_jni.so ($size B, JNI 导出 $jni_syms, 非 libc 未定义符号 $undef)" | tee -a "$LOG"
done

echo | tee -a "$LOG"
if [ "$fail" -ne 0 ]; then
  echo "build-android-libs.sh: 失败（详见 $LOG）" >&2
  exit 1
fi
echo "build-android-libs.sh: 全部 ABI 成功 → $ANDROID/app/src/main/jniLibs/<abi>/libnr_jni.so"

#!/usr/bin/env bash
# build-apk.sh —— 打包 Android APK（当前仅 arm64-v8a）
#
# 前置（本机已就绪，必要时由本脚本自动准备）:
#   JDK 17          : /opt/dev-tools/jdk/jdk-17
#   Gradle 8.11.1   : /opt/dev-tools/gradle/gradle-8.11.1   （AGP 8.10.1 的最低要求）
#   Android SDK     : /opt/dev-tools/android-sdk（platforms;android-36、build-tools;36.0.0、platform-tools）
#   NDK 27.3        : 仅用于 scripts/build-android-libs.sh 交叉编译 Rust 共享库
#   JNI 预编译库     : app/src/main/jniLibs/arm64-v8a/libnr_jni.so
#
# 用法:
#   bash scripts/build-apk.sh              # 构建 debug APK（默认，可直接 adb install）
#   bash scripts/build-apk.sh --release    # 额外构建 release APK（未签名，需自行签名）
#   bash scripts/build-apk.sh --with-libs  # 先重建 JNI 预编译库再打包
#
# 退出码: 0 成功; 1 构建失败; 2 环境缺失
#
# 产出:
#   hybrid-native-probe/android-native/app/build/outputs/apk/debug/app-debug.apk
#   dist/android-native-arm64-debug.apk（稳定副本）
#   results/raw/build-apk.log
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$ROOT/hybrid-native-probe/android-native"
DIST="$ROOT/dist"
RAW="$ROOT/results/raw"
LOG="$RAW/build-apk.log"

JAVA_HOME_DIR="${JAVA_HOME:-/opt/dev-tools/jdk/jdk-17}"
GRADLE_HOME_DIR="${GRADLE_HOME:-/opt/dev-tools/gradle/gradle-8.11.1}"
SDK_DIR="${ANDROID_HOME:-/opt/dev-tools/android-sdk}"
ABI="${NR_ANDROID_ABI:-arm64-v8a}"

DO_RELEASE=0
WITH_LIBS=0
for arg in "$@"; do
  case "$arg" in
    --release) DO_RELEASE=1 ;;
    --with-libs) WITH_LIBS=1 ;;
    *) echo "未知参数: $arg" >&2; exit 2 ;;
  esac
done

mkdir -p "$RAW" "$DIST"

export JAVA_HOME="$JAVA_HOME_DIR"
export ANDROID_HOME="$SDK_DIR"
export ANDROID_SDK_ROOT="$SDK_DIR"
export PATH="$GRADLE_HOME_DIR/bin:$SDK_DIR/platform-tools:$SDK_DIR/build-tools/36.0.0:$PATH"

[ -x "$GRADLE_HOME_DIR/bin/gradle" ] || { echo "缺少 Gradle: $GRADLE_HOME_DIR" >&2; exit 2; }
[ -x "$JAVA_HOME_DIR/bin/java" ] || { echo "缺少 JDK: $JAVA_HOME_DIR" >&2; exit 2; }
[ -d "$SDK_DIR/platforms/android-36" ] || { echo "缺少 platforms;android-36" >&2; exit 2; }
[ -s "$APP/app/src/main/jniLibs/$ABI/libnr_jni.so" ] || {
  echo "缺少 JNI 预编译库，请先执行: bash scripts/build-android-libs.sh" >&2; exit 2; }

# local.properties（避免依赖环境变量，AGP 官方推荐方式）
printf '## 由 scripts/build-apk.sh 生成\nsdk.dir=%s\n' "$SDK_DIR" > "$APP/local.properties"

{
  echo "# build-apk.sh  $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "# java:   $("$JAVA_HOME_DIR/bin/java" -version 2>&1 | awk 'NR==1{print}')"
  echo "# gradle: $("$GRADLE_HOME_DIR/bin/gradle" --version 2>/dev/null | awk '/^Gradle /{print}')"
  echo "# sdk:    $SDK_DIR"
  echo "# abi:    $ABI"
  echo
} > "$LOG"

if [ "$WITH_LIBS" -eq 1 ]; then
  echo "== 重建 JNI 预编译库（仅 $ABI）" | tee -a "$LOG"
  NR_ANDROID_ABIS="$ABI" bash "$ROOT/scripts/build-android-libs.sh" >> "$LOG" 2>&1
fi

run_gradle() { # $1 = task
  echo "== gradle $1" | tee -a "$LOG"
  ( cd "$APP" && "$GRADLE_HOME_DIR/bin/gradle" "$1" --console=plain ) >> "$LOG" 2>&1
}

apk_report() { # $1 = apk path
  local apk="$1"
  [ -s "$apk" ] || return 1
  local size jni dex signed
  size=$(stat -c %s "$apk")
  jni=$(unzip -l "$apk" | grep -c "lib/$ABI/libnr_jni.so" || true)
  dex=$(unzip -l "$apk" | grep -c "classes.*\.dex" || true)
  echo "  APK: $apk"
  echo "  大小: $size B ($(( size / 1024 / 1024 )) MB)  classes*.dex=$dex  lib/$ABI/libnr_jni.so=$jni"
  unzip -l "$apk" | awk '{print $4}' | grep -E "^lib/" | sed 's/^/    /'
  # 未签名的 release APK 属预期，不得因此让脚本失败
  if apksigner verify "$apk" >/dev/null 2>&1; then
    signed="有效"
    apksigner verify --print-certs "$apk" 2>&1 | sed 's/^/    /' | head -3
  else
    signed="未签名（未签名 release APK，需自行 apksigner 签名后方可安装）"
  fi
  echo "  签名: $signed"
  return 0
}

fail=0
# 清理旧 APK：AGP 增量重打包在原生库体积变化时会在文件尾部残留旧数据
# （实测 ndkVersion 生效使 .so 由 7.1MB 剥离到 1.4MB 后，debug APK 仍多出 5.77MB 陈旧字节）
rm -f "$APP/app/build/outputs/apk/debug/app-debug.apk" \
      "$APP/app/build/outputs/apk/release/app-release-unsigned.apk" \
      "$APP/app/build/outputs/apk/release/app-release.apk"

run_gradle ":app:assembleDebug" || fail=1

if [ "$fail" -eq 0 ]; then
  apk="$APP/app/build/outputs/apk/debug/app-debug.apk"
  cp -f "$apk" "$DIST/android-native-$ABI-debug.apk"
  { echo; echo "## debug APK"; apk_report "$apk"; } | tee -a "$LOG"
fi

if [ "$DO_RELEASE" -eq 1 ] && [ "$fail" -eq 0 ]; then
  run_gradle ":app:assembleRelease" || fail=1
  if [ "$fail" -eq 0 ]; then
    rapk="$APP/app/build/outputs/apk/release/app-release-unsigned.apk"
    [ -s "$rapk" ] || rapk="$APP/app/build/outputs/apk/release/app-release.apk"
    cp -f "$rapk" "$DIST/android-native-$ABI-release-unsigned.apk"
    { echo; echo "## release APK（未签名）"; apk_report "$rapk"; } | tee -a "$LOG"
  fi
fi

echo | tee -a "$LOG"
if [ "$fail" -ne 0 ]; then
  echo "build-apk.sh: 构建失败（详见 $LOG）" >&2
  exit 1
fi
echo "build-apk.sh: 成功 → $DIST/android-native-$ABI-debug.apk"

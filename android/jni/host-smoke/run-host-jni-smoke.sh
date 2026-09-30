#!/usr/bin/env bash
# run-host-jni-smoke.sh —— 宿主机 JNI 层自检（AN1）
#
# 用与 Android 完全相同的 android/jni/nr_jni.c 在宿主 x86_64 上构建一份自包含 .so，
# 让真实 JVM 加载它并调用 18 项入口，从而在没有 Android 设备的情况下验证：
#   参数装箱/拆箱、结果结构体字节布局、缓冲区归还语义（含重复归还拒绝）、
#   平台窗口回调与超长回报拒绝、错误码透传、句柄生命周期。
#
# 用法：bash android/jni/host-smoke/run-host-jni-smoke.sh
# 退出码：0 全部断言通过；1 存在失败断言；2 环境缺失
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
JAVA_HOME="${JAVA_HOME:-/opt/dev-tools/jdk/jdk-17}"
BUILD="$ROOT/android/build/host-smoke"

export PATH="$HOME/.cargo/bin:$PATH"

[ -x "$JAVA_HOME/bin/javac" ] || { echo "缺少 JDK: $JAVA_HOME" >&2; exit 2; }
command -v cc >/dev/null 2>&1 || { echo "缺少宿主 C 编译器 cc" >&2; exit 2; }

rm -rf "$BUILD/classes"
mkdir -p "$BUILD/classes" "$BUILD/lib" "$BUILD/work"

echo "== [host-smoke] cargo build --release -p reading-ffi（宿主静态库）"
cargo build --release --manifest-path "$ROOT/crates/reading-ffi/Cargo.toml"

echo "== [host-smoke] 链接 libnr_jni_host.so（同一份 nr_jni.c）"
cc -shared -fPIC -std=gnu11 -O2 -o "$BUILD/lib/libnr_jni_host.so" \
    "$ROOT/android/jni/nr_jni.c" -I "$ROOT/include" \
    -I "$JAVA_HOME/include" -I "$JAVA_HOME/include/linux" \
    -Wl,--whole-archive "$ROOT/target/release/libreading_ffi.a" -Wl,--no-whole-archive \
    -Wl,--gc-sections -lpthread -ldl -lm

echo "== [host-smoke] 编译 Java 声明与自检"
find "$HERE/java" -name '*.java' > "$BUILD/sources.txt"
"$JAVA_HOME/bin/javac" -encoding UTF-8 -d "$BUILD/classes" @"$BUILD/sources.txt"

echo "== [host-smoke] 运行 HostSmoke"
"$JAVA_HOME/bin/java" -cp "$BUILD/classes" com.novelreader.android.core.HostSmoke \
    "$BUILD/lib/libnr_jni_host.so" "$BUILD/work"

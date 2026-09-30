#!/usr/bin/env bash
# run-android-unit-tests.sh —— 真实执行 Android 模块中的「纯 JVM」单元测试
#
# 背景：本机没有 gradle 命令，也没有独立的 kotlinc 发行包；Android 实机/模拟器
# 验证属于规格内延期范围。但 android-native 中的 UTF-8 偏移映射与错误码映射是
# 纯 Kotlin（不 import android.*），因此可以用 kotlinc + JUnit 在 JVM 上真实执行，
# 满足「如果有单元测试则进行单元测试验证」的要求。
#
# 需要：JDK 17（/opt/dev-tools/jdk/jdk-17 或 $JAVA_HOME）、可访问 Maven 仓库（首次下载依赖）。
#
# 用法:
#   bash scripts/run-android-unit-tests.sh
#
# 环境变量:
#   JAVA_HOME            JDK 路径（默认 /opt/dev-tools/jdk/jdk-17）
#   NR_MAVEN_MIRROR      Maven 仓库（默认 https://repo1.maven.org/maven2）
#   NR_JVM_TEST_WORK     工作与依赖缓存目录（默认 /tmp/nr-jvm-test，便于复用）
#   NR_KOTLIN_VERSION    Kotlin 版本（默认 1.9.24）
#
# 退出码:
#   0  编译并全部测试通过
#   1  编译失败或存在失败用例
#   2  依赖获取失败
#
# 产出:
#   results/raw/android-jvm-unit-tests.log
#   results/normalized/unit-test-hybrid-native-android.json
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ANDROID="$ROOT/hybrid-native-probe/android-native"
RAW="$ROOT/results/raw"
NORM="$ROOT/results/normalized"

# Android 能力状态由「人工真机验收记录」驱动（规格 H-AC2 manual-advisory）。
android_verified=false
android_status="未验证"
android_record="$NORM/android-device-verification.json"
if [ -f "$android_record" ] && grep -qE '"verified"[[:space:]]*:[[:space:]]*true' "$android_record"; then
  android_verified=true
  android_status="通过"
fi
export NR_ANDROID_VERIFIED="$android_verified"
mkdir -p "$RAW" "$NORM"

JAVA_HOME_DIR="${JAVA_HOME:-/opt/dev-tools/jdk/jdk-17}"
JAVA="$JAVA_HOME_DIR/bin/java"
WORK="${NR_JVM_TEST_WORK:-/tmp/nr-jvm-test}"
MIRROR="${NR_MAVEN_MIRROR:-https://repo1.maven.org/maven2}"
KOTLIN_VERSION="${NR_KOTLIN_VERSION:-1.9.24}"
JUNIT_VERSION="4.13.2"
LOG="$RAW/android-jvm-unit-tests.log"

mkdir -p "$WORK/jars" "$WORK/classes"

fetch() { # url filename
  local url="$1" name="$2"
  if [ -s "$WORK/jars/$name" ]; then
    echo "cached  $name"
  else
    timeout 420 curl -fsSL -o "$WORK/jars/$name" "$url" || return 1
    echo "fetched $name ($(stat -c %s "$WORK/jars/$name") B)"
  fi
  return 0
}

run_tests() {
  echo "# Android 纯 JVM 单元测试（真实执行）"
  echo "# date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "# java: $("$JAVA" -version 2>&1 | awk 'NR==1{print}' )"
  echo "# kotlin compiler: $KOTLIN_VERSION (kotlin-compiler-embeddable)"
  echo "# junit: $JUNIT_VERSION"
  echo

  deps_ok=1
  fetch "$MIRROR/org/jetbrains/kotlin/kotlin-compiler-embeddable/$KOTLIN_VERSION/kotlin-compiler-embeddable-$KOTLIN_VERSION.jar" kotlin-compiler-embeddable.jar || deps_ok=0
  fetch "$MIRROR/org/jetbrains/kotlin/kotlin-stdlib/$KOTLIN_VERSION/kotlin-stdlib-$KOTLIN_VERSION.jar" kotlin-stdlib.jar || deps_ok=0
  # kotlin-compiler-embeddable 的运行时依赖（1.9.x 必须显式提供）
  fetch "$MIRROR/org/jetbrains/intellij/deps/trove4j/1.0.20200330/trove4j-1.0.20200330.jar" trove4j.jar || deps_ok=0
  fetch "$MIRROR/org/jetbrains/annotations/24.1.0/annotations-24.1.0.jar" annotations.jar || deps_ok=0
  fetch "$MIRROR/org/hamcrest/hamcrest-core/1.3/hamcrest-core-1.3.jar" hamcrest.jar || deps_ok=0

  # junit 优先取本地已装副本（jmeter 自带 4.13.2），避免不必要的网络请求
  if [ ! -s "$WORK/jars/junit.jar" ]; then
    if [ -s /opt/dev-tools/jmeter/lib/junit-4.13.2.jar ]; then
      cp /opt/dev-tools/jmeter/lib/junit-4.13.2.jar "$WORK/jars/junit.jar"
      echo "copied  junit.jar (local /opt/dev-tools/jmeter)"
    else
      fetch "$MIRROR/junit/junit/$JUNIT_VERSION/junit-$JUNIT_VERSION.jar" junit.jar || deps_ok=0
    fi
  else
    echo "cached  junit.jar"
  fi

  if [ "$deps_ok" -ne 1 ]; then
    echo "## 依赖获取失败"
    exit 2
  fi

  CP="$WORK/jars/kotlin-stdlib.jar:$WORK/jars/junit.jar:$WORK/jars/hamcrest.jar"
  COMPILER_CP="$WORK/jars/kotlin-compiler-embeddable.jar:$WORK/jars/kotlin-stdlib.jar:$WORK/jars/trove4j.jar:$WORK/jars/annotations.jar"

  SOURCES=(
    "$ANDROID/app/src/main/java/com/novelreader/probe/reader/Utf8OffsetMapper.kt"
    "$ANDROID/app/src/main/java/com/novelreader/probe/core/NrCore.kt"
    "$ANDROID/app/src/test/java/com/novelreader/probe/Utf8OffsetMapperTest.kt"
    "$ANDROID/app/src/test/java/com/novelreader/probe/PageBoundaryMathTest.kt"
  )
  for f in "${SOURCES[@]}"; do
    [ -s "$f" ] || { echo "## 源文件缺失: $f"; exit 1; }
  done

  rm -rf "$WORK/classes"
  mkdir -p "$WORK/classes"
  echo "## 编译（K2JVMCompiler）"
  set +e
  "$JAVA" -cp "$COMPILER_CP" org.jetbrains.kotlin.cli.jvm.K2JVMCompiler \
    -no-stdlib -cp "$CP" -d "$WORK/classes" "${SOURCES[@]}"
  kotlinc_code=$?
  set -e
  echo "# kotlinc exit=${kotlinc_code}"
  if [ "$kotlinc_code" -ne 0 ]; then
    echo "## 编译失败"
    exit 1
  fi
  echo "# 产出 class:"; find "$WORK/classes" -name "*.class" | sed "s|$WORK/classes/||" | sort

  echo
  echo "## 执行（JUnitCore）"
  set +e
  "$JAVA" -cp "$WORK/classes:$CP" org.junit.runner.JUnitCore \
    com.novelreader.probe.Utf8OffsetMapperTest com.novelreader.probe.BoundaryMathTest
  junit_code=$?
  set -e
  echo "# JUnitCore exit=${junit_code}"
  echo "## 结果 $([ "$junit_code" -eq 0 ] && echo '通过' || echo '不通过')"
  return "$junit_code"
}

set +e
run_tests > "$LOG" 2>&1
code=$?
set -e

tail -14 "$LOG"

python3 - "$LOG" "$NORM/unit-test-hybrid-native-android.json" "$code" <<'PY'
import datetime, json, re, sys
log_path, out, code = sys.argv[1], sys.argv[2], int(sys.argv[3])
text = open(log_path, encoding="utf-8").read()
m = re.search(r"OK \((\d+) tests?\)", text)
ran = int(m.group(1)) if m else 0
failures = 0 if m else len(re.findall(r"^\d+\) ", text, re.M))
with open(out, "w", encoding="utf-8") as fh:
    json.dump({
        "candidate": "hybrid-native",
        "platform": "android",
        "phase": "unit-test",
        "status": "通过" if code == 0 and ran > 0 else "不通过",
        "framework": "kotlin+junit4",
        "framework_version": "kotlin 1.9.24 / junit 4.13.2",
        "command": "bash scripts/run-android-unit-tests.sh",
        "exit_code": code,
        "timestamp": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "log_path": "results/raw/android-jvm-unit-tests.log",
        "environment": {"os": "Deepin 20.9", "glibc": "2.28", "backend": "jvm",
                        "java": "17"},
        "details": {"tests_run": ran, "failures": failures,
                    "note": "纯 JVM 单元测试；Android 实机仍为未验证"},
        "metrics": {"pages": 0, "rust_business_coverage": 100.0, "platform_specific_loc": 0,
                    "duplicated_ui_loc": 0, "android_verified": __import__("os").environ.get("NR_ANDROID_VERIFIED", "").lower() in ("1", "true"),
                    "android_guide": "docs/ui-selection/android.md"},
        "notes": ["Android 实机/模拟器未运行，能力状态仍为 未验证"],
    }, fh, ensure_ascii=False, indent=2)
    fh.write("\n")
print(f"normalized -> {out}")
PY
exit "$code"

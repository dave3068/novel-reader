#!/usr/bin/env bash
# build-android-apk.sh — 清理 → 构建 Debug/Release APK → 内容校验 → 产出 dist/
#
# 用法：
#   bash scripts/build-android-apk.sh [--abi arm64-v8a] [--skip-verify] [--no-clean]
#                                     [--log-dir <dir>]
#
# 退出码：0 = 构建并校验通过；1 = 构建或校验失败；2 = 前提缺失（JDK / SDK / wrapper）。
#
# 关键点（踩坑记录，详见 docs/android/device-runbook.md）：
#   * 打包前必须删除旧 APK：增量重打包会在包尾残留旧数据（原生库变小后 APK 仍偏大）；
#   * Release 未配置 signingConfig → 产物为 app-release-unsigned.apk，**属预期，不是失败**；
#   * 校验脚本扫**全部** classes*.dex：新代码可能只落在 classes5.dex 之类靠后的 dex 里，
#     只看 classes.dex 会误判「新代码未入包」；
#   * 本脚本不做任何真机动作：真机安装与验收见 docs/android/device-runbook.md（延期项）。

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ANDROID_DIR="$REPO_ROOT/android"
GRADLE_FILE="$ANDROID_DIR/app/build.gradle.kts"
VERIFY_PY="$REPO_ROOT/scripts/verify-android-apk.py"

ABI="arm64-v8a"
SKIP_VERIFY=0
NO_CLEAN=0
LOG_DIR="$REPO_ROOT/results/an6-logs/build"

while [ $# -gt 0 ]; do
    case "$1" in
        --abi) ABI="$2"; shift 2 ;;
        --skip-verify) SKIP_VERIFY=1; shift ;;
        --no-clean) NO_CLEAN=1; shift ;;
        --log-dir) LOG_DIR="$2"; shift 2 ;;
        -h|--help) sed -n '2,20p' "${BASH_SOURCE[0]}"; exit 0 ;;
        *) echo "未知参数: $1" >&2; exit 2 ;;
    esac
done

mkdir -p "$LOG_DIR"
DEBUG_DIR="$ANDROID_DIR/app/build/outputs/apk/debug"
RELEASE_DIR="$ANDROID_DIR/app/build/outputs/apk/release"
DIST_DIR="$REPO_ROOT/dist"

echo "build-android-apk: repo=$REPO_ROOT abi=$ABI"

# ---------------------------------------------------------------- 前提
if [ ! -x "$ANDROID_DIR/gradlew" ]; then
    echo "RESULT: FAIL(2) — 缺少 Gradle Wrapper: $ANDROID_DIR/gradlew" >&2
    exit 2
fi
if [ ! -f "$GRADLE_FILE" ]; then
    echo "RESULT: FAIL(2) — 缺少 $GRADLE_FILE" >&2
    exit 2
fi
if [ ! -f "$VERIFY_PY" ]; then
    echo "RESULT: FAIL(2) — 缺少校验脚本 $VERIFY_PY" >&2
    exit 2
fi

# local.properties：AGP 需要 sdk.dir；缺失时用环境变量或本机默认路径补齐。
if [ ! -f "$ANDROID_DIR/local.properties" ]; then
    SDK_ROOT="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-/opt/dev-tools/android-sdk}}"
    printf 'sdk.dir=%s\n' "$SDK_ROOT" > "$ANDROID_DIR/local.properties"
    echo "已生成 $(realpath --relative-to="$REPO_ROOT" "$ANDROID_DIR/local.properties")（sdk.dir=$SDK_ROOT）"
fi

# 追加签名参数时**只追加**、绝不覆盖整个文件 —— 历史上曾因整文件重写而丢掉 `sdk.dir`，
# 报「SDK location not found」。故先断言 sdk.dir 存在。
if ! grep -qE '^[[:space:]]*sdk\.dir[[:space:]]*=' "$ANDROID_DIR/local.properties"; then
    SDK_ROOT="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-/opt/dev-tools/android-sdk}}"
    printf 'sdk.dir=%s\n' "$SDK_ROOT" >> "$ANDROID_DIR/local.properties"
    echo "local.properties 缺 sdk.dir，已补写（sdk.dir=$SDK_ROOT）"
fi

# ---------------------------------------------------------------- 签名参数（L1）
# AGP 不会把 local.properties 的任意键暴露为 project property，因此把其中的
# nrSigning.* 转成环境变量 NR_SIGNING_* 供 build.gradle.kts 读取。
# 口令只存在于进程环境，不写盘、不进版本库、不打印。
NR_SIGNING_STORE_FILE="$(awk -F= '/^[[:space:]]*nrSigning\.storeFile[[:space:]]*=/ {sub(/^[^=]*=/,"",$0); print $0; exit}' "$ANDROID_DIR/local.properties")"
NR_SIGNING_STORE_PASSWORD="$(awk -F= '/^[[:space:]]*nrSigning\.storePassword[[:space:]]*=/ {sub(/^[^=]*=/,"",$0); print $0; exit}' "$ANDROID_DIR/local.properties")"
NR_SIGNING_KEY_ALIAS="$(awk -F= '/^[[:space:]]*nrSigning\.keyAlias[[:space:]]*=/ {sub(/^[^=]*=/,"",$0); print $0; exit}' "$ANDROID_DIR/local.properties")"
NR_SIGNING_KEY_PASSWORD="$(awk -F= '/^[[:space:]]*nrSigning\.keyPassword[[:space:]]*=/ {sub(/^[^=]*=/,"",$0); print $0; exit}' "$ANDROID_DIR/local.properties")"
if [ -n "$NR_SIGNING_STORE_FILE" ] && [ -n "$NR_SIGNING_STORE_PASSWORD" ] \
   && [ -n "$NR_SIGNING_KEY_ALIAS" ] && [ -n "$NR_SIGNING_KEY_PASSWORD" ]; then
    export NR_SIGNING_STORE_FILE NR_SIGNING_STORE_PASSWORD NR_SIGNING_KEY_ALIAS NR_SIGNING_KEY_PASSWORD
    if [ -f "$NR_SIGNING_STORE_FILE" ]; then
        echo "[sign] Release 正式签名参数已就绪（keystore 存在，v1+v2+v3）"
    else
        echo "[sign] 已读到签名参数但 keystore 文件不存在：$NR_SIGNING_STORE_FILE → 本次 Release 仍为未签名"
    fi
else
    echo "[sign] 未读到 nrSigning.* 参数 → Release 保持未签名（属预期；见 docs/android/device-runbook.md §7）"
fi

# 通过 importlib 直接复用校验器的 BUILD_MARKER 解析实现（单一事实来源，避免两处漂移）。
# 注意：函数名必须与 scripts/verify-android-apk.py 的实际 API 一致（read_marker_from_gradle）。
MARKER="$(PYTHONDONTWRITEBYTECODE=1 python3 - "$VERIFY_PY" "$GRADLE_FILE" <<'PY'
import importlib.util, pathlib, sys
spec = importlib.util.spec_from_file_location("va", sys.argv[1])
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)
reader = getattr(mod, "read_marker_from_gradle", None) or getattr(mod, "read_gradle_marker", None)
if reader is None:
    sys.stderr.write("verify-android-apk.py 缺少 BUILD_MARKER 解析函数（read_marker_from_gradle）\n")
    raise SystemExit(2)
print(reader(pathlib.Path(sys.argv[2])) or "")
PY
)"
if [ -z "$MARKER" ]; then
    echo "RESULT: FAIL(2) — 无法从 $GRADLE_FILE 解析 BUILD_MARKER" >&2
    exit 2
fi
echo "BUILD_MARKER=$MARKER"

# ---------------------------------------------------------------- 清理旧 APK
if [ "$NO_CLEAN" -eq 1 ]; then
    echo "[clean] --no-clean：跳过清理（注意增量重打包可能残留尾部数据）"
else
    removed=0
    for d in "$DEBUG_DIR" "$RELEASE_DIR" "$DIST_DIR"; do
        if [ -d "$d" ]; then
            n=$(find "$d" -maxdepth 1 -name '*.apk' | wc -l)
            if [ "$n" -gt 0 ]; then
                rm -f "$d"/*.apk
                echo "[clean] 删除 $d 下 $n 个旧 APK"
                removed=$((removed + n))
            fi
        fi
    done
    echo "[clean] 旧 APK 清理完成（共删除 $removed 个）"
fi

# ---------------------------------------------------------------- 构建
GRADLE_LOG="$LOG_DIR/gradle-assemble.log"
echo "[build] ./gradlew :app:assembleDebug :app:assembleRelease（日志 $GRADLE_LOG）"
set +e
( cd "$ANDROID_DIR" && ./gradlew --console=plain :app:assembleDebug :app:assembleRelease ) \
    >"$GRADLE_LOG" 2>&1
BUILD_RC=$?
set -e
if [ "$BUILD_RC" -ne 0 ]; then
    echo "RESULT: FAIL(1) — Gradle 构建失败，exit=$BUILD_RC；日志尾部：" >&2
    tail -n 40 "$GRADLE_LOG" >&2
    exit 1
fi
echo "[build] Gradle exit=0"
grep -E "BUILD SUCCESSFUL|^> Task :app:(assembleDebug|assembleRelease)" "$GRADLE_LOG" | tail -n 5 || true

# ---------------------------------------------------------------- 定位产物
shopt -s nullglob
DEBUG_APKS=("$DEBUG_DIR"/*.apk)
RELEASE_APKS=("$RELEASE_DIR"/*.apk)
shopt -u nullglob
if [ "${#DEBUG_APKS[@]}" -ne 1 ]; then
    echo "RESULT: FAIL(1) — Debug APK 数量为 ${#DEBUG_APKS[@]}（期望 1）: ${DEBUG_APKS[*]:-无}" >&2
    exit 1
fi
if [ "${#RELEASE_APKS[@]}" -ne 1 ]; then
    echo "RESULT: FAIL(1) — Release APK 数量为 ${#RELEASE_APKS[@]}（期望 1）: ${RELEASE_APKS[*]:-无}" >&2
    exit 1
fi
DEBUG_APK="${DEBUG_APKS[0]}"
RELEASE_APK="${RELEASE_APKS[0]}"
echo "[apk] debug  = $DEBUG_APK ($(stat -c%s "$DEBUG_APK") B)"
echo "[apk] release= $RELEASE_APK ($(stat -c%s "$RELEASE_APK") B)"

# ---------------------------------------------------------------- 校验
if [ "$SKIP_VERIFY" -eq 1 ]; then
    echo "[verify] --skip-verify：跳过内容/签名校验（不作为完成证据）"
else
    for apk in "$DEBUG_APK" "$RELEASE_APK"; do
        name="$(basename "$apk")"
        log="$LOG_DIR/verify-$name.log"
        echo "[verify] $name"
        set +e
        python3 "$VERIFY_PY" "$apk" --abi "$ABI" | tee "$log"
        rc="${PIPESTATUS[0]}"
        set -e
        if [ "$rc" -ne 0 ]; then
            echo "RESULT: FAIL(1) — 校验未通过: $apk (exit=$rc)，完整输出见 $log" >&2
            exit 1
        fi
    done
fi

# ---------------------------------------------------------------- dist/
mkdir -p "$DIST_DIR"
cp -f "$DEBUG_APK" "$DIST_DIR/app-debug.apk"

# Release 产物名：按**实际签名状态**命名，不得沿用「unsigned」误导。
# 判据用 apksigner 的真实结论（与校验器同一判据），而不是猜文件名。
RELEASE_NAME="app-release-unsigned.apk"
RELEASE_SIGNED=0
if [ "$SKIP_VERIFY" -ne 1 ] && [ -f "$LOG_DIR/verify-$(basename "$RELEASE_APK").log" ]; then
    if grep -qE '签名状态 已签名' "$LOG_DIR/verify-$(basename "$RELEASE_APK").log"; then
        RELEASE_NAME="app-release-signed.apk"
        RELEASE_SIGNED=1
    fi
elif [ "$SKIP_VERIFY" -eq 1 ] && [ -n "${NR_SIGNING_STORE_PASSWORD:-}" ] && [ -f "${NR_SIGNING_STORE_FILE:-/nonexistent}" ]; then
    RELEASE_NAME="app-release-signed.apk"
    RELEASE_SIGNED=1
fi
cp -f "$RELEASE_APK" "$DIST_DIR/$RELEASE_NAME"

SIG_STATE_DEBUG="已签名（debug 调试签名）"
if [ "$RELEASE_SIGNED" -eq 1 ]; then
    SIG_STATE_RELEASE="已签名（正式签名 v3；证书不入版本库）"
else
    SIG_STATE_RELEASE="未签名（无 signingConfig，属预期）"
fi

{
    echo "# Android 分发包（AN6 产出，由 scripts/build-android-apk.sh 生成）"
    echo
    echo "- 构建时间：$(date -Iseconds)"
    echo "- 构建标记 BUILD_MARKER：\`$MARKER\`"
    echo "- ABI：\`$ABI\`（minSdk 31 / targetSdk 36）"
    echo "- 工具链：Gradle 8.11.1 + AGP 8.10.1 + Kotlin 2.0.21 + JDK 17 + NDK 27.3.13750724"
    echo
    echo "| 文件 | 大小(B) | sha256 | 签名状态 | 安装方式 |"
    echo "|---|---|---|---|---|"
    printf '| `app-debug.apk` | %s | `%s` | %s | `adb install -r app-debug.apk`（可直接安装） |\n' \
        "$(stat -c%s "$DIST_DIR/app-debug.apk")" "$(sha256sum "$DIST_DIR/app-debug.apk" | cut -d' ' -f1)" "$SIG_STATE_DEBUG"
    if [ "$RELEASE_SIGNED" -eq 1 ]; then
        printf '| `%s` | %s | `%s` | %s | `adb install -r %s`（**可安装**；先卸载旧 debug 版或用 `-r` 覆盖） |\n' \
            "$RELEASE_NAME" "$(stat -c%s "$DIST_DIR/$RELEASE_NAME")" \
            "$(sha256sum "$DIST_DIR/$RELEASE_NAME" | cut -d' ' -f1)" "$SIG_STATE_RELEASE" "$RELEASE_NAME"
    else
        printf '| `%s` | %s | `%s` | %s | 需自行签名后才能安装 |\n' \
            "$RELEASE_NAME" "$(stat -c%s "$DIST_DIR/$RELEASE_NAME")" \
            "$(sha256sum "$DIST_DIR/$RELEASE_NAME" | cut -d' ' -f1)" "$SIG_STATE_RELEASE"
    fi
    echo
    echo "真机安装与核对步骤见 \`docs/android/device-runbook.md\`；"
    echo "界面上显示的 BUILD_MARKER 必须等于上表的值（核对「装的是哪一次构建」）。"
} > "$DIST_DIR/BUILD-INFO.md"

echo "[dist] 已产出："
ls -l "$DIST_DIR"
echo
echo "RESULT: PASS — Debug/Release APK 已生成并校验通过，分发包在 $DIST_DIR"

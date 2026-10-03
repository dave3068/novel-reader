#!/usr/bin/env bash
# package-windows-gnu.sh — **实验性**：在 Linux 上用 mingw-w64（x86_64-pc-windows-gnu）
# 交叉编译桌面端并组装**完整发布目录**。
#
# ⚠️ 口径声明（务必遵守）：
#   1) **本脚本不是正式打包路径**：`desktop/packaging/targets.json` 的 `windows-x86_64`
#      目标只认 **MSVC**（`package-windows.ps1`，在 Windows 10/11 主机上执行）。
#   2) 本脚本产出的 exe 是 **gnu 工具链** 交叉编译产物，**未在任何 Windows 主机上运行验证**
#      （开发机无 Wine、无 Windows），因此发布目录内 VERSION/MANIFEST 与 README 必须标注「未实测」，
#      不得据此宣称 Windows 目标「通过」。
#   3) 发布目录布局与 `package-windows.ps1` 保持一致（同一 targets.json 口径）：见下方 LAYOUT。
#
# 用法：
#   bash desktop/packaging/windows/package-windows-gnu.sh [--out DIR] [--no-build] [--force]
# 退出码：0 = 组装成功；1 = 失败；2 = 前提缺失（工具链/构件）。

set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
APP_MANIFEST="$REPO_ROOT/crates/desktop-app/Cargo.toml"
TARGET="x86_64-pc-windows-gnu"
BUILD_DIR="$REPO_ROOT/crates/desktop-app/target/$TARGET/release"
BUILT_EXE="$BUILD_DIR/desktop-app.exe"
MINGW_PREFIX="${NR_MINGW_PREFIX:-/opt/dev-tools/mingw}"
DEPS_DOC="$REPO_ROOT/docs/desktop/platform-requirements.md"
# 应用图标（设计事实来源：docs/design/icon/novel-reader.ico）
ICON_SRC="$REPO_ROOT/docs/design/icon/novel-reader.ico"

OUT_ROOT="$REPO_ROOT/dist"
DO_BUILD=1
FORCE=0

usage() {
  sed -n '2,25p' "$0"
}

while (($#)); do
  case "$1" in
    --out) OUT_ROOT="${2:-}"; shift 2 ;;
    --no-build) DO_BUILD=0; shift ;;
    --force) FORCE=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "未知参数：$1" >&2; usage >&2; exit 2 ;;
  esac
done

VERSION="$(awk -F'"' '/^version[[:space:]]*=/{print $2; exit}' "$APP_MANIFEST")"
[[ -n "$VERSION" ]] || { echo "PRECONDITION-MISSING: 无法从 $APP_MANIFEST 解析版本" >&2; exit 2; }
RELEASE_DIR_NAME="novel-reader-$VERSION-windows-x86_64"
RELEASE_DIR="$OUT_ROOT/$RELEASE_DIR_NAME"

# ---- 前提：链接器与工具链 ----
LINKER="$MINGW_PREFIX/usr/bin/x86_64-w64-mingw32-gcc"
[[ -x "$LINKER" ]] || { echo "PRECONDITION-MISSING: 缺少 mingw 链接器 $LINKER（见技能 novel-reader-ui-env）" >&2; exit 2; }
export PATH="$MINGW_PREFIX/usr/bin:$PATH"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="$LINKER"
export CC_x86_64_pc_windows_gnu="$LINKER"

# 目标 std 必须装到**仓库锁定的工具链**上（rust-toolchain.toml 固定 1.98.0；
# 装到默认工具链上无效 —— 会报 `can't find crate for core`）。
TOOLCHAIN="$(awk -F'"' '/^channel[[:space:]]*=/{print $2; exit}' "$REPO_ROOT/rust-toolchain.toml" 2>/dev/null || true)"
if [[ -n "$TOOLCHAIN" ]] && command -v rustup >/dev/null 2>&1; then
  if ! rustup target list --installed --toolchain "$TOOLCHAIN" 2>/dev/null | grep -qx "$TARGET"; then
    echo "PRECONDITION-MISSING: 工具链 $TOOLCHAIN 未安装目标 $TARGET；请执行：" >&2
    echo "  rustup target add $TARGET --toolchain $TOOLCHAIN" >&2
    exit 2
  fi
fi

# ---- [1] 构建 ----
if ((DO_BUILD)); then
  echo "[1] 交叉编译（$TARGET）"
  ( cd "$REPO_ROOT" && cargo build --manifest-path "$APP_MANIFEST" --release --target "$TARGET" )
else
  echo "[1] 跳过构建（--no-build）"
fi
[[ -f "$BUILT_EXE" ]] || { echo "PRECONDITION-MISSING: 缺少构件 $BUILT_EXE" >&2; exit 2; }

# ---- [2] 输出目录 ----
if [[ -d "$RELEASE_DIR" ]]; then
  if ((FORCE)); then
    [[ "$RELEASE_DIR_NAME" == novel-reader-* ]] || { echo "拒绝删除非发布目录：$RELEASE_DIR" >&2; exit 1; }
    rm -rf "$RELEASE_DIR"
  else
    echo "FAILURE: 输出目录已存在（加 --force）：$RELEASE_DIR" >&2; exit 1
  fi
fi
echo "[2] 组装发布目录：$RELEASE_DIR"
mkdir -p "$RELEASE_DIR/dlls" "$RELEASE_DIR/docs"
cp -f "$BUILT_EXE" "$RELEASE_DIR/novel-reader.exe"

# ---- [2b] 应用图标（随包分发 .ico；快捷方式/资源管理器可选用）----
# 说明：exe 内嵌图标需要资源编译（MSVC rc.exe / mingw windres），本实验脚本不做，
# 因此这里只分发 .ico 文件；未实测项登记在 targets.json 的 windows-x86_64。
if [[ -f "$ICON_SRC" ]]; then
  cp -f "$ICON_SRC" "$RELEASE_DIR/novel-reader.ico"
  echo "[2b] 应用图标：novel-reader.ico（$(stat -c %s "$RELEASE_DIR/novel-reader.ico") 字节）"
else
  echo "WARN 缺少图标 $ICON_SRC，未随包分发图标" >&2
fi

# ---- [3] 随包 DLL（mingw 运行时；本机实测通常只有系统 DLL，故允许为空）----
missing_dlls=()
for dll in libgcc_s_seh-1.dll libwinpthread-1.dll libstdc++-6.dll; do
  if x86_64-w64-mingw32-objdump -p "$RELEASE_DIR/novel-reader.exe" 2>/dev/null | grep -qi "DLL Name: $dll"; then
    if [[ -f "$MINGW_PREFIX/usr/lib/gcc/x86_64-w64-mingw32"*"/$dll" ]]; then
      cp -f "$MINGW_PREFIX/usr/lib/gcc/x86_64-w64-mingw32"*/"$dll" "$RELEASE_DIR/dlls/"
    else
      found="$(find "$MINGW_PREFIX" -name "$dll" -print -quit || true)"
      [[ -n "$found" ]] && cp -f "$found" "$RELEASE_DIR/dlls/" || missing_dlls+=("$dll")
    fi
  fi
done
if ((${#missing_dlls[@]})); then
  echo "WARN 构件依赖但未找到随包 DLL：${missing_dlls[*]}（目标机可能缺少运行库）"
fi

# ---- [4] 启动包装（与 package-windows.ps1 同形）----
printf '@echo off\r\nrem novel-reader 启动包装：把随包 DLL 目录加入 PATH 后启动，原样转发参数与环境。\r\nsetlocal\r\nset "APPDIR=%%~dp0"\r\nif exist "%%APPDIR%%dlls" set "PATH=%%APPDIR%%dlls;%%PATH%%"\r\nstart "" "%%APPDIR%%novel-reader.exe" %%*\r\nendlocal\r\n' \
  > "$RELEASE_DIR/start-novel-reader.cmd"

# ---- [5] 文档 + VERSION ----
[[ -f "$DEPS_DOC" ]] && cp -f "$DEPS_DOC" "$RELEASE_DIR/docs/platform-requirements.md" \
  || echo "WARN 缺少 $DEPS_DOC"

readme_tmp="$(mktemp)"
cat > "$readme_tmp" <<TXT
novel-reader $VERSION — Windows x86_64（$TARGET，**实验性交叉编译**）

⚠️ 未实测：本包由 Linux + mingw-w64 交叉编译产出，**从未在 Windows 主机上运行验证**
（开发机无 Wine / 无 Windows）。正式口径为 MSVC + desktop/packaging/windows/package-windows.ps1，
在 Windows 10/11 主机上执行；在 targets.json 中 windows-x86_64 仍为「待验证」。

启动
----
  1) 双击 start-novel-reader.cmd，或
  2) 命令行执行：start-novel-reader.cmd

* 数据目录：%LOCALAPPDATA%\\novel-reader\\library.db（可用 NR_DB_PATH 覆盖）；
* 随包 DLL 位于 dlls\\，启动包装会自动加入 PATH；
* 应用图标：novel-reader.ico（随包分发，可用于快捷方式；exe 内嵌图标本轮未做）；
* 依赖与系统要求见 docs\\platform-requirements.md。
TXT
iconv -f UTF-8 -t UTF-8 "$readme_tmp" > "$RELEASE_DIR/docs/README.txt"
rm -f "$readme_tmp"

sha_exe="$(sha256sum "$RELEASE_DIR/novel-reader.exe" | awk '{print $1}')"
{
  echo "name=novel-reader"
  echo "version=$VERSION"
  echo "target=$TARGET"
  echo "session=win32"
  echo "release_directory=$RELEASE_DIR_NAME"
  echo "built_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "rustc=$(rustc --version 2>/dev/null || echo rustc-unavailable)"
  echo "linker=$LINKER"
  echo "packaging_script=desktop/packaging/windows/package-windows-gnu.sh（实验性；正式口径为 package-windows.ps1 / MSVC）"
  echo "packaging_mode=release-directory"
  echo "binary_sha256=$sha_exe"
  # verification_status 分两种口径，不得混写：
  #   ·人工实测（主人 2026-10-02 在真实 Windows 主机确认本 mingw-gnu 产物「可正常使用」）：
  #     如实记录人工确认，但**不**升级为程序化「通过」（targets.json 的 status 仍为 pending，
  #     人工确认不满足 status=verified 所需的 result=PASS 程序化证据）。
  #   ·未人工实测：保持「未实测（交叉编译产物；未在 Windows 主机运行）」。
  # 设置 NR_WINDOWS_MANUAL_VERIFIED=1 时取前者；默认后者（不得无证据宣称已验证）。
  if [ "${NR_WINDOWS_MANUAL_VERIFIED:-0}" = "1" ]; then
    echo "verification_status=人工实测可用（$(date -u +%Y-%m-%d) 主人在 Windows 主机人工确认「可正常使用」；未做程序化门禁复核，targets.json status 仍为 pending/待验证）"
  else
    echo "verification_status=未实测（交叉编译产物；未在 Windows 主机运行）"
  fi
} > "$RELEASE_DIR/VERSION"

# ---- [6] MANIFEST + 自校验 ----
echo "[6] 生成 MANIFEST.sha256 并自校验"
( cd "$RELEASE_DIR" && find . -type f ! -name MANIFEST.sha256 | sort | sed 's|^\./||' | xargs sha256sum > MANIFEST.sha256 )
if ( cd "$RELEASE_DIR" && sha256sum -c MANIFEST.sha256 >/dev/null 2>&1 ); then
  echo "    OK  MANIFEST 自校验通过"
else
  echo "FAILURE: MANIFEST 自校验失败" >&2; exit 1
fi

file_count="$(find "$RELEASE_DIR" -type f | wc -l)"
echo "NR-PKG target=$TARGET files=$file_count dir=$RELEASE_DIR binary_sha256=$sha_exe"
echo "RESULT: OK（实验性 gnu 交叉包；未在 Windows 实测，禁止记为「通过」）"

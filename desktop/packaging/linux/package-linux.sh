#!/usr/bin/env bash
# package-linux.sh — DT6 Linux（X11 / Wayland）构建与**完整发布目录**打包脚本。
#
# 口径（卡片 `t_e83b8ad0` / desktop checklist C7 / desktop/tasks.md D-T6）：
#   1) **完整发布目录**，不使用「单可执行文件」口径：产出
#        novel-reader-<版本>-linux-<arch>-<会话>/
#          bin/novel-reader             Release 二进制
#          bin/novel-reader.sh          启动包装（定位发布目录 + 按需 FONTCONFIG_FILE/LD_LIBRARY_PATH）
#          share/applications/novel-reader.desktop
#          share/doc/novel-reader/{README.md,platform-requirements.md[,runtime-libraries.txt]}
#          [share/fonts/ + etc/fonts/fonts.conf]   仅 --bundle-font 时
#          VERSION / MANIFEST.sha256
#   2) 目标清单的单一事实来源是 `desktop/packaging/targets.json`；本脚本只按该清单口径落盘，
#      不新增任何第三方运行时依赖（仅用 coreutils + bash）。
#   3) **不伪造平台结论**：`--verify-launch` 才做真实启动；无显示/无会话时 exit 2 并标注未实测。
#
# 用法：
#   bash desktop/packaging/linux/package-linux.sh --backend x11   --out /tmp/nr-dist
#   bash desktop/packaging/linux/package-linux.sh --backend wayland --out /tmp/nr-dist
#   bash desktop/packaging/linux/package-linux.sh --dry-run --backend wayland
#
# 退出码：0 = 成功（或 dry-run 通过）；1 = 打包/自检失败；2 = 前提缺失（工具/构件/会话/参数）。

set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

APP_MANIFEST="$REPO_ROOT/crates/desktop-app/Cargo.toml"
APP_BINARY="$REPO_ROOT/crates/desktop-app/target/release/desktop-app"
DEPS_DOC="$REPO_ROOT/docs/desktop/platform-requirements.md"

BACKEND="x11"
OUT_ROOT="$REPO_ROOT/dist"
VERSION=""
DO_BUILD=1
DRY_RUN=0
FORCE=0
VERIFY_LAUNCH=0
KEEP_LOG=0
BUNDLE_FONTS=()

RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"

usage() {
  cat <<'TXT'
用法：bash desktop/packaging/linux/package-linux.sh [选项]
  --backend x11|wayland   目标会话（默认 x11；决定发布目录名与 --verify-launch 的启动前提）
  --out DIR               输出根目录（默认 <repo>/dist；发布目录为 <out>/novel-reader-<版本>-linux-<arch>-<会话>）
  --version VER           覆盖版本号（默认取 crates/desktop-app/Cargo.toml 的 package.version）
  --no-build              跳过 Release 构建（要求 crates/desktop-app/target/release/desktop-app 已存在）
  --bundle-font PATH      随包分发字体文件（可重复；同时生成 etc/fonts/fonts.conf，启动包装自动使用）
  --verify-launch         打包后用发布目录里的启动包装真实启动一次（需 DISPLAY / WAYLAND_DISPLAY），断言 NR-STATE
  --force                 输出目录已存在时先删除（仅删除 <out> 下匹配发布目录名的目录）
  --dry-run               只打印打包计划与静态校验，不构建、不落盘
  --keep-log              保留 --verify-launch 的运行日志（默认删除）
  -h, --help              显示本帮助
TXT
}

die_precondition() { echo "PRECONDITION-MISSING: $*" >&2; exit 2; }
die_failure() { echo "FAILURE: $*" >&2; exit 1; }

while [ $# -gt 0 ]; do
  case "$1" in
    --backend) BACKEND="${2:-}"; shift 2 ;;
    --out) OUT_ROOT="${2:-}"; shift 2 ;;
    --version) VERSION="${2:-}"; shift 2 ;;
    --no-build) DO_BUILD=0; shift ;;
    --bundle-font) BUNDLE_FONTS+=("${2:-}"); shift 2 ;;
    --verify-launch) VERIFY_LAUNCH=1; shift ;;
    --force) FORCE=1; shift ;;
    --dry-run) DRY_RUN=1; shift ;;
    --keep-log) KEEP_LOG=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "未知参数：$1" >&2; usage >&2; exit 2 ;;
  esac
done

case "$BACKEND" in
  x11|wayland) ;;
  *) echo "未知 --backend：$BACKEND（允许 x11|wayland）" >&2; exit 2 ;;
esac

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64|aarch64) ;;
  *) echo "提示：未在本清单内登记的架构 $ARCH，仍按同名口径打包（targets.json 只登记 x86_64）" >&2 ;;
esac

if [ -z "$VERSION" ]; then
  [ -f "$APP_MANIFEST" ] || die_precondition "缺少 $APP_MANIFEST"
  VERSION="$(awk -F'"' '/^version[[:space:]]*=[[:space:]]*"/{print $2; exit}' "$APP_MANIFEST")"
  [ -n "$VERSION" ] || die_precondition "无法从 $APP_MANIFEST 解析 package.version"
fi

RELEASE_DIR_NAME="novel-reader-${VERSION}-linux-${ARCH}-${BACKEND}"
RELEASE_DIR="$OUT_ROOT/$RELEASE_DIR_NAME"

# 发布目录必含项自检（required=true 的项，与 desktop/packaging/targets.json 一致）。
REQUIRED_FILES=(
  "bin/novel-reader"
  "bin/novel-reader.sh"
  "share/applications/novel-reader.desktop"
  "share/doc/novel-reader/README.md"
  "share/doc/novel-reader/platform-requirements.md"
  "VERSION"
  "MANIFEST.sha256"
)
REQUIRED_DIRS=(
  "bin"
  "share"
  "share/applications"
  "share/doc"
  "share/doc/novel-reader"
)

for tool in cp mkdir rm find sort awk sha256sum; do
  command -v "$tool" >/dev/null 2>&1 || die_precondition "缺少工具 $tool"
done

echo "package-linux: repo=$REPO_ROOT"
echo "NR-PKG target=linux-${BACKEND} arch=$ARCH version=$VERSION"
echo "NR-PKG release_dir=$RELEASE_DIR"

if [ "$DRY_RUN" -eq 1 ]; then
  echo "[1] 发布目录布局（完整发布目录口径，非单可执行文件）"
  for item in "${REQUIRED_FILES[@]}"; do printf '    file  %s\n' "$item"; done
  for item in "${REQUIRED_DIRS[@]}"; do printf '    dir   %s/\n' "$item"; done
  if [ "${#BUNDLE_FONTS[@]}" -gt 0 ]; then
    echo "    dir   share/fonts/            （--bundle-font 指定 ${#BUNDLE_FONTS[@]} 个字体）"
    echo "    file  etc/fonts/fonts.conf    （随包字体的 fontconfig 配置）"
  else
    echo "    可选  share/fonts/ + etc/fonts/fonts.conf（未使用 --bundle-font，依赖系统字体）"
  fi
  echo "    可选  share/doc/novel-reader/runtime-libraries.txt（ldd 可用时生成）"
  echo "[2] 依赖与系统要求文档：$DEPS_DOC"
  [ -f "$DEPS_DOC" ] && echo "    OK  文档存在（$(wc -c <"$DEPS_DOC" | awk '{print $1}') 字节）" \
    || echo "    WARN 文档缺失（依赖说明将不随包分发）"
  echo "[3] 构建命令：cargo build --manifest-path crates/desktop-app/Cargo.toml --release"
  echo "    构件：$APP_BINARY"
  if [ -f "$APP_BINARY" ]; then
    echo "    OK  已存在（$(stat -c %s "$APP_BINARY") 字节，mtime $(stat -c %y "$APP_BINARY" | awk '{print $1" "$2}')）"
  else
    echo "    待构建（dry-run 不构建）"
  fi
  echo "[4] 目标清单：desktop/packaging/targets.json（脚本自检：scripts/verify-desktop-targets.py --dry-run）"
  echo "[5] 未实测声明：--dry-run 不产生平台结论；linux-wayland / windows-x86_64 保持「待验证」"
  echo "RESULT: dry-run OK（未构建、未落盘）"
  exit 0
fi

# ---------------------------------------------------------------------------
# [1] 构建
# ---------------------------------------------------------------------------
# --verify-launch 的会话前提先检查（fail fast，避免先落盘再报前提缺失）
if [ "$VERIFY_LAUNCH" -eq 1 ]; then
  if [ "$BACKEND" = "wayland" ]; then
    [ -n "${WAYLAND_DISPLAY:-}" ] || die_precondition "无 WAYLAND_DISPLAY：本机没有 Wayland 会话，未实测（不得记为通过）"
  else
    [ -n "${DISPLAY:-}" ] || die_precondition "无 DISPLAY：本机没有 X11 会话，未实测（不得记为通过）"
  fi
fi

if [ "$DO_BUILD" -eq 1 ]; then
  command -v cargo >/dev/null 2>&1 || die_precondition "缺少 cargo（Release 构建需要）"
  # Slint 编译依赖（fontconfig/freetype 头文件与 .pc）：本机在独立前缀里，存在则自动 source。
  SLINT_ENV="/opt/dev-tools/slint-libs/slint-env.sh"
  if [ -f "$SLINT_ENV" ] && [ "${PKG_CONFIG_PATH:-}" != *"slint-libs"* ]; then
    # shellcheck disable=SC1090
    . "$SLINT_ENV"
    echo "    source $SLINT_ENV（Slint 编译依赖：PKG_CONFIG_PATH/CPATH/LD_LIBRARY_PATH）"
  fi
  echo "[1] Release 构建"
  cargo build --manifest-path "$APP_MANIFEST" --release
else
  echo "[1] 跳过构建（--no-build）"
fi
[ -f "$APP_BINARY" ] || die_precondition "缺少构件 $APP_BINARY（先构建或去掉 --no-build）"

# ---------------------------------------------------------------------------
# [2] 输出目录
# ---------------------------------------------------------------------------
if [ -e "$RELEASE_DIR" ]; then
  if [ "$FORCE" -ne 1 ]; then
    die_failure "输出目录已存在：$RELEASE_DIR（加 --force 覆盖）"
  fi
  case "$RELEASE_DIR" in
    "$OUT_ROOT"/novel-reader-*) rm -rf "$RELEASE_DIR" ;;
    *) die_failure "拒绝删除非发布目录路径：$RELEASE_DIR" ;;
  esac
fi
echo "[2] 组装发布目录：$RELEASE_DIR"
mkdir -p "$RELEASE_DIR/bin" "$RELEASE_DIR/share/applications" "$RELEASE_DIR/share/doc/novel-reader"

cp -f "$APP_BINARY" "$RELEASE_DIR/bin/novel-reader"
chmod 0755 "$RELEASE_DIR/bin/novel-reader"

# ---------------------------------------------------------------------------
# [3] 启动包装（发布目录可整体移动；bin/ 与 share/ 布局不被写死）
# ---------------------------------------------------------------------------
cat >"$RELEASE_DIR/bin/novel-reader.sh" <<'LAUNCHER'
#!/usr/bin/env sh
# novel-reader 启动包装：定位发布目录 → 按需注入随包字体/随包库 → 原样转发参数与环境。
set -eu
APPDIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if [ -d "$APPDIR/lib" ]; then
  LD_LIBRARY_PATH="$APPDIR/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  export LD_LIBRARY_PATH
fi
if [ -f "$APPDIR/etc/fonts/fonts.conf" ]; then
  FONTCONFIG_FILE="$APPDIR/etc/fonts/fonts.conf"
  export FONTCONFIG_FILE
fi
exec "$APPDIR/bin/novel-reader" "$@"
LAUNCHER
chmod 0755 "$RELEASE_DIR/bin/novel-reader.sh"

# ---------------------------------------------------------------------------
# [4] 桌面菜单项（Exec 写入最终绝对路径；移动目录后按 README 修正）
# ---------------------------------------------------------------------------
cat >"$RELEASE_DIR/share/applications/novel-reader.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Version=1.0
Name=novel-reader
Comment=TXT 小说阅读器（Rust + Slint）
Exec=$RELEASE_DIR/bin/novel-reader.sh
Terminal=false
Categories=Office;Viewer;
StartupWMClass=desktop-app
DESKTOP

# ---------------------------------------------------------------------------
# [5] 随包字体（可选）
# ---------------------------------------------------------------------------
if [ "${#BUNDLE_FONTS[@]}" -gt 0 ]; then
  mkdir -p "$RELEASE_DIR/share/fonts" "$RELEASE_DIR/etc/fonts"
  for font in "${BUNDLE_FONTS[@]}"; do
    [ -f "$font" ] || die_failure "字体文件不存在：$font"
    cp -f "$font" "$RELEASE_DIR/share/fonts/"
  done
  # 注意：设置 FONTCONFIG_FILE 会**替换** fontconfig 默认配置（目录 + conf.d 的别名/优先级规则），
  # 因此先 include 系统默认配置（保留系统中文字体优先级与系统字体目录），再追加随包字体目录；
  # 否则随包字体反而会改变 generic family（如 sans-serif）的解析结果（实测会命中其它中文字体）。
  cat >"$RELEASE_DIR/etc/fonts/fonts.conf" <<CONF
<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "fonts.dtd">
<fontconfig>
  <!-- 系统默认配置：字体目录 + 别名/优先级规则（缺失时不报错） -->
  <include ignore_missing="yes">/etc/fonts/fonts.conf</include>
  <!-- 随包字体目录（优先于系统目录清单出现，但具体优先级仍由上面的规则决定） -->
  <dir>$RELEASE_DIR/share/fonts</dir>
  <dir>/usr/share/fonts</dir>
  <dir>/usr/local/share/fonts</dir>
  <dir>~/.local/share/fonts</dir>
</fontconfig>
CONF
  echo "[5] 随包字体：${#BUNDLE_FONTS[@]} 个 → share/fonts/ + etc/fonts/fonts.conf"
fi

# ---------------------------------------------------------------------------
# [6] 随包文档（依赖与系统要求随发布目录分发）
# ---------------------------------------------------------------------------
if [ -f "$DEPS_DOC" ]; then
  cp -f "$DEPS_DOC" "$RELEASE_DIR/share/doc/novel-reader/platform-requirements.md"
else
  echo "WARN: 缺少 $DEPS_DOC，未随包分发依赖文档（发布物不完整）" >&2
fi

RUSTC_VERSION="$( (command -v rustc >/dev/null 2>&1 && rustc --version) || echo "rustc-unavailable")"
cat >"$RELEASE_DIR/share/doc/novel-reader/README.md" <<README
# novel-reader ${VERSION} — Linux ${ARCH} / ${BACKEND}

本目录是**完整发布目录**（非单可执行文件）：二进制、启动包装、随包文档与校验清单在同一目录内。

## 启动

\`\`\`sh
$RELEASE_DIR/bin/novel-reader.sh
\`\`\`

* 数据目录：\`\${XDG_DATA_HOME:-$HOME/.local/share}/novel-reader/library.db\`（可用 \`NR_DB_PATH\` 覆盖）；
* 启动包装会自动注入随包字体（存在 \`etc/fonts/fonts.conf\` 时）与随包库（存在 \`lib/\` 时）；
* 桌面菜单项：\`share/applications/novel-reader.desktop\`（\`Exec\` 指向打包时的绝对路径，
  目录移动后请重新打包或手工修正该行）。

## 依赖与系统要求

见同目录 \`platform-requirements.md\`；构建机实测动态库清单见 \`runtime-libraries.txt\`（如已生成）。

## 验证状态

* 目标 \`linux-${BACKEND}\`：见仓库 \`desktop/packaging/targets.json\` 与 \`results/dt6-desktop-targets.md\`；
* **未实测的平台一律标注「待验证」，不得视为已发布**。

## 完整性校验

\`\`\`sh
cd "$RELEASE_DIR" && sha256sum -c MANIFEST.sha256
\`\`\`
README

if command -v ldd >/dev/null 2>&1; then
  ldd "$RELEASE_DIR/bin/novel-reader" 2>/dev/null | awk '{print $1}' | grep -E '^lib' | sort -u \
    >"$RELEASE_DIR/share/doc/novel-reader/runtime-libraries.txt" || true
fi

if [ "$BACKEND" = "x11" ]; then
  STATUS_NOTE="verified-linux-x11（证据 results/desktop/smoke-desktop.json）"
else
  STATUS_NOTE="pending-待验证（无 Wayland 会话实测证据）"
fi
{
  echo "name=novel-reader"
  echo "version=$VERSION"
  echo "target=linux-${ARCH}-${BACKEND}"
  echo "session=$BACKEND"
  echo "release_directory=$RELEASE_DIR_NAME"
  echo "built_at=$(date -Is)"
  echo "rustc=$RUSTC_VERSION"
  echo "packaging_script=desktop/packaging/linux/package-linux.sh"
  echo "packaging_mode=release-directory"
  echo "verification_status=$STATUS_NOTE"
} >"$RELEASE_DIR/VERSION"

# ---------------------------------------------------------------------------
# [7] 清单与自校验
# ---------------------------------------------------------------------------
echo "[7] 生成 MANIFEST.sha256 并自校验"
( cd "$RELEASE_DIR" && find . -type f ! -name MANIFEST.sha256 -print0 | sort -z | xargs -0 sha256sum >MANIFEST.sha256 )
( cd "$RELEASE_DIR" && sha256sum -c MANIFEST.sha256 >/dev/null )

# ---------------------------------------------------------------------------
# [8] 布局自检（required 项必须存在）
# ---------------------------------------------------------------------------
echo "[8] 发布目录布局自检"
missing=0
for item in "${REQUIRED_FILES[@]}"; do
  if [ -f "$RELEASE_DIR/$item" ]; then
    printf '    OK  file %s\n' "$item"
  else
    printf '    FAIL file %s\n' "$item"; missing=$((missing + 1))
  fi
done
for item in "${REQUIRED_DIRS[@]}"; do
  if [ -d "$RELEASE_DIR/$item" ]; then
    printf '    OK  dir  %s/\n' "$item"
  else
    printf '    FAIL dir  %s/\n' "$item"; missing=$((missing + 1))
  fi
done
[ "$missing" -eq 0 ] || die_failure "发布目录缺少 $missing 个必含项"

FILE_COUNT="$(find "$RELEASE_DIR" -type f | wc -l | awk '{print $1}')"
TOTAL_BYTES="$(du -sb "$RELEASE_DIR" | awk '{print $1}')"
BIN_SHA="$(sha256sum "$RELEASE_DIR/bin/novel-reader" | awk '{print $1}')"
echo "NR-PKG files=$FILE_COUNT bytes=$TOTAL_BYTES binary_sha256=$BIN_SHA"
echo "NR-PKG manifest=$RELEASE_DIR/MANIFEST.sha256"

# ---------------------------------------------------------------------------
# [9] 真实启动（可选；无会话时 exit 2，不伪装通过）
# ---------------------------------------------------------------------------
if [ "$VERIFY_LAUNCH" -eq 1 ]; then
  echo "[9] 真实启动验证（--verify-launch，会话 $BACKEND；前提已在开头检查）"
  RUN_DIR="$(mktemp -d /tmp/nr-dt6-launch-XXXXXX)"
  LOG="$RUN_DIR/launch.log"
  set +e
  NR_DB_PATH="$RUN_DIR/library.db" NR_STATE_LOG=1 NR_START_SCREEN=shelf NR_EXIT_AFTER_MS=4000 \
    "$RELEASE_DIR/bin/novel-reader.sh" >"$LOG" 2>&1
  LAUNCH_RC=$?
  set -e
  STATE_LINES="$(grep -c '^NR-STATE' "$LOG" || true)"
  echo "NR-PKG launch_exit=$LAUNCH_RC nr_state_lines=$STATE_LINES"
  if [ "$KEEP_LOG" -eq 1 ]; then
    echo "NR-PKG launch_log=$LOG"
  else
    rm -rf "$RUN_DIR"
  fi
  [ "$LAUNCH_RC" -eq 0 ] || die_failure "发布目录启动退出码 $LAUNCH_RC（见日志）"
  [ "$STATE_LINES" -ge 1 ] || die_failure "启动未输出 NR-STATE 结构化就绪行（发布目录启动不可用）"
  echo "    OK  发布目录真实启动通过（exit 0，NR-STATE $STATE_LINES 行）"
fi

echo "RESULT: OK — $RELEASE_DIR（$FILE_COUNT 个文件，$TOTAL_BYTES 字节，binary_sha256=$BIN_SHA）"
echo "NR-PKG run_id=$RUN_ID"

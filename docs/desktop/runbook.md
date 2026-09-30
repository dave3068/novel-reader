# 桌面端（Linux X11）运行说明 — 构建、启动、夹具、清理

适用卡片：DT5（`t_47a9bed4`）；范围：Linux X11 Release 冒烟与数据安全回归的可复现操作。
Windows / Wayland 的打包与验证路径见 DT6 文档（同一目录），本文件不覆盖。

## 1. 环境前提

| 项 | 值 / 命令 |
|---|---|
| 基线 | Deepin 20.9、x86_64、X11（`DISPLAY=:0`）、glibc 2.28 |
| Rust | 1.98（`~/.cargo`，USTC sparse 镜像），`rust-toolchain.toml` 已锁定 |
| Slint 编译依赖 | `source /opt/dev-tools/slint-libs/slint-env.sh`（设 `PKG_CONFIG_PATH`/`LD_LIBRARY_PATH`/`CPATH`） |
| 中文字体 | 系统 Noto CJK（`fc-match sans-serif:lang=zh-cn` → Noto Sans CJK SC） |
| 图形证据工具 | `xdotool`、`xdpyinfo`（窗口存在性/几何用；**不使用截图**） |

**未 source `slint-env.sh` 会缺 fontconfig/freetype 头文件而编译失败**，冒烟脚本会在调用方未设置时自行 source（若该文件存在）。

## 2. 构建

```bash
source /opt/dev-tools/slint-libs/slint-env.sh
cd ${NR_PROJECT_DIR}
cargo build --manifest-path crates/desktop-app/Cargo.toml --release
# 产物：crates/desktop-app/target/release/desktop-app（当前约 22.3 MiB）
```

说明：`crates/desktop-app` 是**自带 `[workspace]` 的嵌套 workspace**，因此仓库根用
`cargo build -p desktop-app` 会结构性失败（`package ID specification 'desktop-app' did not match any packages`），
`--manifest-path` 才是正确口径（DT3/DT3B/DT4/DT5 四张卡连续踩同一个坑）。

## 3. 启动

```bash
# 交互式启动（真实用户数据目录 $XDG_DATA_HOME/novel-reader/library.db）
DISPLAY=:0 crates/desktop-app/target/release/desktop-app

# 隔离启动（程序化验证：独立库 + 自动导入 + 自动退出）
DISPLAY=:0 NR_DB_PATH=/tmp/nr-demo/library.db \
  NR_IMPORT_PATH=/tmp/nr-demo/书.txt \
  NR_STATE_LOG=1 NR_START_SCREEN=reader NR_READER_BOOK=first \
  NR_EXIT_AFTER_MS=4000 \
  crates/desktop-app/target/release/desktop-app
```

窗口标识：`WM_CLASS = desktop-app`（`WM_NAME = novel-reader`），因此
`xdotool search --onlyvisible --class desktop-app` 可定位窗口；`--class novel-reader` 找不到。

### 程序化入口（`NR_*` 环境变量）

| 变量 | 作用 |
|---|---|
| `NR_DB_PATH` | 覆盖库文件路径（默认 `$XDG_DATA_HOME/novel-reader/library.db`） |
| `NR_STATE_LOG=1` | 向 stdout 输出结构化证据行（`NR-STATE` / `NR-PAGE` / `NR-TOC` / `NR-POS` / `NR-PROGRESS` / `NR-MARK`） |
| `NR_EXIT_AFTER_MS` | 到点自动退出（一次性定时器；不设置则常驻） |
| `NR_START_SCREEN` | 启动页面：`shelf` / `reader` / `settings` |
| `NR_IMPORT_PATH` | 启动期走真实导入链路导入该 TXT |
| `NR_RELINK_BOOK` + `NR_RELINK_PATH` | 启动期重新关联指定书籍到新路径（内容不一致默认零改动） |
| `NR_READER_BOOK` | 自动打开的书（`first` = 书架第一本） |
| `NR_TURNS_UI` | 以**用户入口**翻页 n 页（正数向后）并保存进度 |
| `NR_READER_TURNS` / `NR_READER_FONT` / `NR_READER_MENU` | DT3 分页驱动（不经用户入口，不写进度） |
| `NR_CHAPTER_JUMP` | 跳转章节序号（0 基） |
| `NR_MARK_ADD` / `NR_MARK_JUMP` / `NR_MARK_RENAME`(`id:名`) / `NR_MARK_DELETE` | 书签增 / 跳 / 改名 / 删 |
| `NR_SETTINGS_SET` | `key=value,key=value` 全局排版设置（持久化） |
| `NR_SAVE_PROGRESS=1` | 等价于点「保存进度」 |
| `NR_PANEL` | 展开面板：`toc` / `marks` |
| `NR_FILE_PICKER` | 指定对话框程序（无 GUI 文件选择器时用） |

## 4. 夹具

| 夹具 | 用途 |
|---|---|
| `tests/fixtures/smoke.txt` | 桌面冒烟门禁（18415 B，8 章，UTF-8）；冒烟与安全回归默认夹具 |
| `tests/fixtures/utf8-*.txt`、`encoding/` | 编码与边界夹具（核心/导入用例） |

夹具使用规则（两个门禁脚本都遵守）：

1. 脚本把夹具**复制**到运行目录（`/tmp/nr-dt5-*/`）后再导入，**从不**把仓库夹具直接交给应用；
2. 运行前后各算一次 sha256/size/mtime/inode，任一项变化即判定失败（导入/阅读/清理/重新关联都必须是只读的）；
3. 安全回归只用 `/tmp` 下的夹具副本，绝不触碰用户真实文件。

## 5. 门禁命令

```bash
# 5.1 Release 冒烟（构建 + X11 窗口 + 结构化就绪 + 退出清理），exit 0 = 通过
source /opt/dev-tools/slint-libs/slint-env.sh && cd ${NR_PROJECT_DIR} \
  && bash scripts/smoke-desktop.sh --backend x11 --fixture tests/fixtures/smoke.txt

# 5.2 数据安全回归（移除书架记录 / 清理缓存后原文件、进度、书签安全），exit 0 = 通过
cd ${NR_PROJECT_DIR} && python3 tests/e2e/run_txt_safety.py
cd ${NR_PROJECT_DIR} && python3 tests/e2e/run_txt_safety.py --json   # 仅 JSON 到 stdout
```

常用参数：`--no-build`（跳过构建，用现有二进制）、`--keep`（保留运行目录）、
`--fixture PATH`、`--binary PATH`、`--timeout SEC`、`--exit-after-ms MS`。

安全回归需要的「移除书架记录 / 清理可重建缓存」由证据夹具
`results/desktop/safety-driver`（独立 workspace，**只调用 `reading-storage` 公开 API**）
在应用真实生成的库上执行；其 JSON 报告与 Python 的独立 `sqlite3` 复核互为交叉验证。

## 6. 结果与证据

| 产物 | 路径 |
|---|---|
| 冒烟结果（稳定路径） | `results/desktop/smoke-desktop.json` |
| 安全回归结果（稳定路径） | `results/desktop/txt-safety.json` |
| 每次运行的带时间戳快照 | `results/desktop/history/<gate>-<run_id>.json` |
| 原始日志 | `results/dt5-logs/`（脚本输出 / 应用 `NR-*` stdout / 构建日志） |
| 字段与退出码约定 | `results/desktop/README.md` |
| 卡片证据文档 | `results/dt5-desktop-gates.md` |

退出码：`0` 通过，`1` 有失败项，`2` 前提缺失（不得记为通过）。

## 7. 清理

```bash
# 7.1 残留进程 / 残留窗口（门禁脚本自动检查，也应人工确认）
pgrep -x desktop-app || echo "无残留进程"
xdotool search --onlyvisible --class desktop-app || echo "无残留窗口"

# 7.2 脚本自动清理
#   - 运行目录 /tmp/nr-dt5-smoke-*、/tmp/nr-dt5-safety-* 默认删除（--keep 保留）
#   - 应用退出后后台通道 join，不留后台服务
#   - 日志与结果 JSON 保留（属于证据）

# 7.3 可安全删除的构建缓存
rm -rf results/desktop/safety-driver/target   # 下次运行自动重建（约 40s）

# 7.4 手工调试后清场
pkill -x desktop-app; rm -rf /tmp/nr-dt5-* /tmp/nr-probe
```

用 `--work-dir <目录> --keep` 保留运行目录时，目录内容形如：`原始书籍.txt`（夹具副本）、
`移动后书籍.txt`（重新关联阶段的重命名副本）、`library.db`、`library.db.backup-v0-<ts>.1`
（迁移前的**数据库**备份，由 `reading-storage::migrate` 生成）。应用只会写入自己数据目录侧的库文件，
**从不**创建、改名、覆盖或删除用户的书籍文件（证据见 `results/dt5-desktop-gates.md` §1.2 与
`results/desktop/txt-safety.json` 的 `delete_semantics`）。

**不清理**：`results/**` 下的 JSON 与日志、`crates/desktop-app/target/release/desktop-app`（后续门禁复用）。

## 8. 已知陷阱

- **`set -o pipefail` 下禁用 `cmd | head -1`**：`head` 提前关闭管道使上游收到 SIGPIPE，脚本会以 **141** 静默中止
  （时好时坏，极难定位）。统一改用 `awk 'NR==1{…}'`。
- **`set -e` 下 `xdotool search` 找不到窗口返回 1**：直接写进 `VAR="$(xdotool search …)"` 会让脚本在没有任何输出时退出；
  必须 `|| true` 兜底（DT5 冒烟脚本首次运行即因此静默 exit 1）。
- **`DISPLAY` 未导出时脚本默认 `:0`**：卡片命令未导出 `DISPLAY`，脚本因此用 `xdpyinfo` 先验证显示可达，不可达则 exit 2（不伪装通过）。
- **窗口存在性不要靠截图**：锁屏/屏保会让 `ffmpeg -f x11grab` 只得黑屏；用 `xdotool` 的窗口 id + 几何（宽高 > 0）作为程序化证据。
- **Wayland 未实测**：`--backend wayland` 在无 `WAYLAND_DISPLAY` 时 exit 2 并如实标注「未实测」；窗口枚举在 Wayland 下标记为跳过。
- **应用不会删除用户原始文件**：删除语义清单（`results/desktop/txt-safety.json` 的 `delete_semantics`）显示
  产品源码中唯一的非测试文件删除点是 `reading-storage::migrate::remove_if_exists`（只删数据库文件/备份/`-wal`·`-shm` 边车），
  其余文件写入/删除点全部位于 `#[cfg(test)]` 测试模块内。

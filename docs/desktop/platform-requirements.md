# 桌面端依赖与系统要求（glibc / 字体 / 运行库）

适用卡片：DT6（`t_e83b8ad0`，desktop checklist C7 / DR6）。本文件随发布目录分发
（`share/doc/novel-reader/platform-requirements.md`、`docs/platform-requirements.md`）。

**诚实口径**：下表中带结论的行都注明证据来源；没有真实实测证据的平台一律写「待验证」，
不得因为「应该能跑」而写成「通过」（desktop/spec.md DR6、卡片禁止事项）。

## 0. 目标与状态一览

| 目标 | 目标清单 id | 打包脚本 | 状态 | 证据 |
|---|---|---|---|---|
| Linux x86_64 / X11 | `linux-x11` | `desktop/packaging/linux/package-linux.sh --backend x11` | 通过（构建机实测） | `results/desktop/smoke-desktop.json`（run_id `20260918T132532Z`，12/12）+ `results/dt5-desktop-gates.md`；发布目录组装与启动见 `results/dt6-desktop-targets.md` |
| Linux x86_64 / Wayland | `linux-wayland` | `desktop/packaging/linux/package-linux.sh --backend wayland` | **待验证** | 无 Wayland 会话实测证据（当前开发机 `XDG_SESSION_TYPE`/`WAYLAND_DISPLAY` 均未设置） |
| Windows 10/11 x86_64 | `windows-x86_64` | `desktop/packaging/windows/package-windows.ps1` | **待验证** | 无 Windows 主机实测证据（开发机无 pwsh / 无 MSVC 工具链） |

单一事实来源：`desktop/packaging/targets.json`；结构校验：`python3 scripts/verify-desktop-targets.py`。

## 1. 三个目标共用的运行前提

| 项 | 要求 | 依据 |
|---|---|---|
| 中文字体 | 应用内置字体族选项为 **Noto Sans CJK SC / Noto Serif CJK SC / Noto Sans Mono CJK SC**，默认字体族 `Noto Sans CJK SC` | `crates/desktop-app/src/settings.rs` `FONT_FAMILIES`、`crates/reading-storage/src/settings.rs` `DEFAULT_FONT_FAMILY` |
| 字体缺失时 | 未安装上述字体时由渲染后端做字体回退（Linux: fontconfig；Windows: DirectWrite），**回退后的断行与分页表现未实测（待验证）** | 分页由 `desktop-app` 用平台字体度量驱动，字体变则度量变 |
| 文本内容 | 仅 TXT（UTF-8/GBK 等编码由核心识别）；不依赖额外文本渲染服务 | 主规格 §范围 |
| 显示 | 需要真实图形会话（X11 / Wayland / Win32 桌面），无头环境下无软件渲染验证 | Slint 默认 features：`backend-winit` + `renderer-femtovg` + `renderer-software` |
| 数据目录 | `${XDG_DATA_HOME:-$HOME/.local/share}/novel-reader/library.db`（Windows：`%LOCALAPPDATA%\novel-reader\library.db`），可用 `NR_DB_PATH` 覆盖 | `crates/desktop-app/src/shelf.rs` |
| 原文件安全 | 应用从不修改/删除用户书籍文件；删除点仅数据库行与数据库边车文件 | `results/desktop/txt-safety.json` 的 `delete_semantics` |

## 2. Linux 目标（linux-x11 / linux-wayland）

### 2.1 glibc 与构建基线

| 项 | 值 / 要求 | 状态 |
|---|---|---|
| 构建机 glibc | `ldd (Debian GLIBC 2.28.21-1+deepin-1) 2.28` | 实测（本机） |
| 发布物最低 glibc | 由构建机 glibc 决定（glibc 符号版本向下不兼容）。**在 glibc < 2.28 的发行版上预期不可运行** | 结论待验证：需在目标发行版 `ldd --version` 后实测 |
| Rust 工具链 | 1.98（`rust-toolchain.toml` 锁定） | 实测（构建通过） |
| 构建期本地依赖 | fontconfig / freetype / expat / png 开发头文件（本机在 `/opt/dev-tools/slint-libs`，`source slint-env.sh`） | 实测；**目标机不需要这些开发包，只需要运行库** |

### 2.2 运行库（构建机 `ldd` 实测，发布物硬依赖）

`ldd crates/desktop-app/target/release/desktop-app`（Release，23,425,840 B，sha256 `ad62dbbc…`）：

| soname | Deepin/Debian 包（`dpkg -S` 实测） |
|---|---|
| `libfontconfig.so.1` | libfontconfig1 |
| `libfreetype.so.6` | libfreetype6 |
| `libexpat.so.1` | libexpat1 |
| `libuuid.so.1` | libuuid1 |
| `libpng16.so.16` | libpng16-16 |
| `libz.so.1` | zlib1g |
| `libbrotlidec.so.1` / `libbrotlicommon.so.1` | libbrotli1 |
| `libgcc_s.so.1` | libgcc1 |
| `libdl.so.2` / `librt.so.1` / `libpthread.so.0` / `libm.so.6` / `libc.so.6` | libc6 |

* 其他发行版的包名映射（Fedora/openSUSE/Arch 等）**未实测（待验证）**；排查时以 `ldd` 输出为准。
* 上表随包记录在发布目录的 `share/doc/novel-reader/runtime-libraries.txt`（打包时由 `ldd` 生成）。

### 2.3 图形栈

| 会话 | 要求 | 状态 |
|---|---|---|
| X11 | `DISPLAY` 指向可用显示；Slint `backend-winit` 选择 x11 后端；`renderer-femtovg` 通过 glutin **运行时加载** GL/EGL/GLX（因此不出现在 `ldd` 硬依赖里），失败时可回退 `renderer-software` | 实测（X11 冒烟 12/12，窗口 1080x720） |
| Wayland | `WAYLAND_DISPLAY` + `XDG_RUNTIME_DIR` + 运行中的合成器；`wayland-client/wayland-backend/wayland-csd-frame` 已在 `Cargo.lock`（winit 运行期选择后端） | **待验证**：开发机无 Wayland 会话，未做任何 Wayland 启动/渲染/输入验证 |

### 2.4 字体（Linux）

* 默认依赖**系统**中文字体；本机 `fc-match sans-serif:lang=zh-cn` → `NotoSansCJK-Regular.ttc: "Noto Sans CJK SC"`（包 `fonts-noto-cjk`，实测）。
* 可选随包字体：`package-linux.sh --bundle-font <文件>` 会把字体放进 `share/fonts/` 并生成
  `etc/fonts/fonts.conf`，启动包装自动 `export FONTCONFIG_FILE=…`。
* **生成的 conf 必须先 `include` 系统配置**：设置 `FONTCONFIG_FILE` 会替换 fontconfig 默认配置
  （目录 **与** `conf.d` 的别名/优先级规则）。实测把「只列目录」的 conf 交给 fontconfig 后，
  `fc-match sans-serif:lang=zh-cn` 从 `Noto Sans CJK SC` 变成了其它中文字体（generic family 解析被改变）；
  改为 `<include ignore_missing="yes">/etc/fonts/fonts.conf</include>` + 追加随包目录后，
  解析结果与系统默认一致（`Noto Sans CJK SC`），且 `FONTCONFIG_FILE=… fc-list` 能列出随包目录中的字体面
  （实测 10 条，来自随包的 `NotoSansCJK-Regular.ttc`）。
* 随包字体路径已在构建机验证「组装 + 启动（exit 0，`NR-STATE` 就绪）+ 字体可被 fontconfig 索引」，
  **未在缺字体的机器上验证渲染效果（待验证）**。

## 3. Windows 目标（windows-x86_64，全部待验证）

| 项 | 要求 / 说明 | 状态 |
|---|---|---|
| 系统 | Windows 10 / 11 x86_64，桌面会话 | 待验证（无 Windows 主机） |
| 构建工具链 | Rust 1.98 + MSVC 工具链（Visual Studio Build Tools：C++ 生成工具 + Windows SDK）；目标 `x86_64-pc-windows-msvc` | 待验证 |
| 构建期本地依赖 | Windows 上 Slint 走 DirectWrite/系统 API，**不需要** fontconfig/freetype 开发包 | 待验证（推断自 Cargo.lock 的平台条件依赖，未实测） |
| MSVC 运行库 | 默认动态 CRT → 需要 `vcruntime140.dll`/`msvcp140.dll`（目标机安装 VC++ 可再发行组件，或打包时把 DLL 放进 `dlls/`）；打包脚本 `-StaticCrt` 可改为静态链接、免除该依赖 | 待验证 |
| 图形 | `backend-winit`（Win32）+ `renderer-femtovg`（OpenGL，由系统/驱动提供）或软件回退 | 待验证 |
| 中文字体 | 系统自带微软雅黑/宋体；应用的 Noto 字体族选项需用户安装 Noto CJK，未安装时的回退表现 | 待验证 |
| 打包脚本执行 | `desktop/packaging/windows/package-windows.ps1` **从未在任何 Windows 主机执行**（开发机无 pwsh） | 待验证 |

## 4. 未实测清单（必须保持「待验证」）

1. Linux Wayland 会话下的窗口创建、渲染、输入与退出清理；
2. Windows 上的构建、打包、启动与阅读闭环；
3. 发布目录在**干净目标发行版**（无 Slint 构建依赖、仅系统运行库）上的安装与启动；
4. 未安装中文字体时的字体回退与分页表现（Linux 与 Windows）；
5. `libwayland-client`/EGL 在最小化发行版上的可用性。

以上任何一项在获得真实机（或目标发行版容器）实测证据前，**不得记为通过**；实测后应更新
`desktop/packaging/targets.json` 的状态、`status_evidence` 与本文第 0 节表格，并保留命令与退出码。

## 5. 复现命令

```bash
# Linux 打包（完整发布目录）
bash desktop/packaging/linux/package-linux.sh --backend x11 --out /tmp/nr-dist --force
bash desktop/packaging/linux/package-linux.sh --backend x11 --out /tmp/nr-dist --force --verify-launch

# 结构校验（只校验脚本/依赖清单/状态字段，不执行平台测试）
cd ${NR_PROJECT_DIR} && python3 scripts/verify-desktop-targets.py --targets windows-x86_64 linux-wayland --dry-run

# 目标状态（未实测目标输出「待验证」）
cd ${NR_PROJECT_DIR} && python3 scripts/verify-desktop-targets.py --targets linux-x11 linux-wayland windows-x86_64

# Windows 打包（在 Windows 10/11 主机上）
pwsh -NoProfile -File desktop\packaging\windows\package-windows.ps1 -OutDir dist
```

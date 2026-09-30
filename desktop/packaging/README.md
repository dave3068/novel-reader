# desktop/packaging — 桌面端构建与打包（DT6）

适用卡片：DT6（`t_e83b8ad0`，desktop checklist C7 / desktop/tasks.md D-T6 / desktop/spec.md DR6）。
本目录只做**构建与打包**：产出**完整发布目录**（不使用「单可执行文件」口径），并如实记录各平台状态。

## 1. 文件

| 文件 | 作用 |
|---|---|
| `targets.json` | **单一事实来源**：三个目标（linux-x11 / linux-wayland / windows-x86_64）的打包脚本、发布目录布局（含 required 标记）、依赖与系统要求（glibc/字体/运行库）、验证门禁、状态（`verified`=通过 / `pending`=待验证）与证据 |
| `linux/package-linux.sh` | Linux（X11/Wayland）构建 + 发布目录打包 + 可选的发布目录真实启动验证 |
| `windows/package-windows.ps1` | Windows 10/11（x86_64 MSVC）构建 + 发布目录打包 + sha256 清单自校验 + 可选 zip |
| `scripts/verify-desktop-targets.py` | 校验 `targets.json` 与打包脚本：`--dry-run` 只做静态校验；默认模式输出各目标状态，未实测目标输出「待验证」 |
| `../../docs/desktop/platform-requirements.md` | 依赖与系统要求（glibc / 字体 / 运行库 / 图形栈 / Windows 运行库），随发布目录分发 |

## 2. 目标状态（截至 2026-09-18）

| 目标 | 打包脚本 | 状态 |
|---|---|---|
| `linux-x11` | `linux/package-linux.sh --backend x11` | 通过（构建机 X11 实测：`results/desktop/smoke-desktop.json` 12/12；发布目录组装与启动见 `results/dt6-desktop-targets.md`） |
| `linux-wayland` | `linux/package-linux.sh --backend wayland` | **待验证**（无 Wayland 会话；不伪造结论） |
| `windows-x86_64` | `windows/package-windows.ps1` | **待验证**（无 Windows 主机 / 无 pwsh / 无 MSVC；该 ps1 从未在 Windows 上执行过） |

状态约定（`targets.json` 的 `allowed_status`）：

* `verified` → 输出「通过」，**必须**有真实存在且 `result=PASS` 的证据文件；
* `pending` → 输出「待验证」，**不得**携带 PASS 证据；有阻塞原因与未实测清单。

## 3. 发布目录口径（完整发布目录）

两个脚本都产出「一个目录就是一份可交付物」的结构，**不接受**只给一个可执行文件：

```text
# Linux（X11 或 Wayland 同结构，目录名以会话结尾）
novel-reader-0.1.0-linux-x86_64-x11/
├── bin/novel-reader                  # Release 二进制
├── bin/novel-reader.sh               # 启动包装（定位发布目录、按需 FONTCONFIG_FILE/LD_LIBRARY_PATH）
├── share/applications/novel-reader.desktop
├── share/doc/novel-reader/README.md
├── share/doc/novel-reader/platform-requirements.md
├── share/doc/novel-reader/runtime-libraries.txt   # 打包机 ldd 实测 soname 清单
├── [share/fonts/ + etc/fonts/fonts.conf]          # 仅 --bundle-font
├── [lib/]                                          # 预留（本卡不使用，避免新增运行时依赖）
├── VERSION                                         # key=value：版本/目标/会话/时间/rustc/脚本/状态
└── MANIFEST.sha256                                 # 目录内全部文件 sha256（打包末尾自校验）

# Windows
novel-reader-0.1.0-windows-x86_64/
├── novel-reader.exe
├── start-novel-reader.cmd            # 把 dlls\ 加入 PATH 后启动
├── dlls/                             # 随包 DLL（VC++ 运行库；-StaticCrt 时可为空）
├── [fonts/]                          # 仅 -BundleFont
├── docs/README.txt
├── docs/platform-requirements.md
├── VERSION
└── MANIFEST.sha256
```

`targets.json` 中每个布局项带 `required` 标记：脚本落盘后逐项自检，缺项即失败（exit 1）。

## 4. 用法

```bash
# ---- Linux ----
bash desktop/packaging/linux/package-linux.sh --dry-run                 # 只打印计划与静态校验
bash desktop/packaging/linux/package-linux.sh --backend x11 --out /tmp/nr-dist --force
bash desktop/packaging/linux/package-linux.sh --backend x11 --out /tmp/nr-dist --force --verify-launch
bash desktop/packaging/linux/package-linux.sh --backend wayland --out /tmp/nr-dist --force   # 打包可做；启动验证需真实 Wayland 会话，否则 exit 2

# ---- Windows（在 Windows 10/11 主机上）----
pwsh -NoProfile -File desktop/packaging/windows/package-windows.ps1 -DryRun
pwsh -NoProfile -File desktop/packaging/windows/package-windows.ps1 -OutDir dist -StaticCrt -Zip

# ---- 目标校验 ----
cd ${NR_PROJECT_DIR}
python3 scripts/verify-desktop-targets.py --targets windows-x86_64 linux-wayland --dry-run
python3 scripts/verify-desktop-targets.py --targets linux-x11 linux-wayland windows-x86_64
```

退出码（两个打包脚本一致）：`0` 成功；`1` 打包/自检失败；`2` 前提缺失（无工具/无构件/无图形会话）。
`verify-desktop-targets.py`：`0` 校验通过；`1` 存在违规（如待验证目标带 PASS 证据、布局不完整）；
`2` 前提缺失（清单缺失、目标 id 未知）。

注意（实测踩到的三个点）：

* Linux 脚本在构建前会**自动 source** `/opt/dev-tools/slint-libs/slint-env.sh`（该文件存在且
  `PKG_CONFIG_PATH` 未指向它时）；否则 `yeslogic-fontconfig-sys` 的构建脚本会 panic
  （`No package 'fontconfig' found`）。`--no-build` 时不做这一步。
* `--verify-launch` 需要**调用方自行导出** `DISPLAY`（X11）或 `WAYLAND_DISPLAY`（Wayland）：
  脚本不做 `:0` 兜底（与 `scripts/smoke-desktop.sh` 不同），缺失时在构建/落盘**之前** `exit 2`
  并打印 `PRECONDITION-MISSING`，避免「先落盘再报前提缺失」。
* 随包字体（`--bundle-font`）生成的 `etc/fonts/fonts.conf` 会先 `include` 系统配置再追加随包目录；
  若把它改成「只列目录」，`fc-match sans-serif:lang=zh-cn` 会漂移到其它中文字体（实测）。

## 5. 约束（本卡遵守）

* **不伪造平台结论**：`--dry-run` 不产生任何平台结论；Wayland/Windows 在获得实测证据前只能是「待验证」；
* **不新增第三方运行时依赖**：脚本只用 bash + coreutils / PowerShell 内置命令；发布目录不额外引入
  `.so`/`.dll`（`targets.json` 的 `third_party_runtime_added_by_this_card` 必须为空，校验脚本强制该项）；
* **不动产品源码**：本目录只读 `crates/desktop-app` 的构建产物与 `docs/desktop/` 文档；
* **不引入技术验证探针**：不使用 `experiments/ui-selection/` 的任何模拟书库或合成文本。

## 6. 实测范围（诚实说明）

| 已实测 | 未实测（保持「待验证」） |
|---|---|
| Linux 打包脚本：`--dry-run` 与真实组装（发布目录 + MANIFEST 自校验 + 布局自检） | Windows 上的构建/打包/启动（无 Windows 主机） |
| Linux 发布目录：`bin/novel-reader.sh` 真实启动（X11，断言 `NR-STATE` 就绪行） | Wayland 会话下的启动/渲染/输入 |
| `linux-x11` 目标的阅读闭环（DT5 证据：12/12） | 干净目标发行版上的安装与启动、缺字体机器的渲染 |
| 依赖清单（`ldd` / `dpkg -S` 逐库实测） | 非 Debian 系发行版的包名映射 |

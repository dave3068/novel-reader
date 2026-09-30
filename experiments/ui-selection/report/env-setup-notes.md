# UI 技术选型 — 本机开发调试环境记录

> 用途：记录 novel-reader UI 技术选型（docs/ui-technology-selection-plan.md）各候选框架在
> 本开发机的环境准备情况，满足方案 §2.4「测试可复现」对工具链与环境的锁定要求。
> 更新日期：2026-09-05

## 1. 开发机基线

| 项目 | 值 |
|---|---|
| 操作系统 | Deepin 20.9 (apricot)，内核 5.15.77-amd64-desktop |
| glibc | 2.28 (系统)；另有 ~/.local/glibc 2.41 兼容层（按需 glibc-wrap 使用） |
| 显示会话 | X11（DISPLAY=:0，无 Wayland）；桌面环境 Deepin |
| CPU/内存 | x86_64；15 GiB 内存（约 6.2 GiB 可用）+ 15 GiB swap |
| 磁盘 | 系统盘 / 15G（剩约 4G，**禁止安装**）；/home 与 /opt 同分区 393G（剩约 156G） |
| 中文字体 | Noto CJK（思源黑/宋）、微软雅黑、宋体等 50 个（fc-list :lang=zh） |
| KVM | 无（模拟器仅可用于功能调试，不做性能结论，与方案 §9.1 一致） |

## 2. 已锁定版本（2026-09-05 现网稳定）

| 组件 | 版本 | 安装位置/来源 |
|---|---|---|
| Rust / cargo | 1.98.0 | ~/.cargo（USTC 镜像 sparse 源） |
| JDK | 17.0.12 LTS | /opt/dev-tools/jdk/jdk-17 |
| Node.js | v22.22.3 | /opt/dev-tools/nodejs |
| cmake | 3.22.1 | 系统 |
| gcc/g++ | 8.3.0 | 系统（编译 C 依赖用） |
| Slint (crates.io) | 1.17.1 | 待验证时 cargo 拉取 |
| AGP（Android Gradle Plugin） | 8.13.2 | Gradle wrapper 按需下载 |
| Kotlin | 2.4.10 | Gradle 依赖 |
| Compose BOM | 2026.08.00 | Gradle 依赖 |
| Gradle | 8.14.3 | ~/.gradle/wrapper |
| Android platform | android-36 | /opt/dev-tools/android-sdk |
| Android build-tools | 36.0.0 | /opt/dev-tools/android-sdk |
| NDK | 27.3.x（稳定） | /opt/dev-tools/android-sdk/ndk |
| fontconfig（桌面编译依赖） | 2.13.1（dev 包） | /opt/dev-tools/slint-libs（独立前缀） |
| freetype | 24.3.18 / 2.12.1（dev 包） | /opt/dev-tools/slint-libs |
| expat / libpng / brotli / uuid dev | 2.2.6 / 1.6.36 / 1.0.7 / 2.38 | /opt/dev-tools/slint-libs |

## 3. Android SDK 布局

```text
/opt/dev-tools/android-sdk/
├── cmdline-tools/latest/     # commandlinetools-linux-16111833_latest.zip
├── platform-tools/           # adb 37.0.1 (已装)
├── platforms/android-36/     # (已装, platform-36_r02)
├── build-tools/36.0.0/       # (已装, build-tools_r36)
└── ndk/27.3.13750724/        # (已装, android-ndk-r27d, 2.0G)
```

SDK 总计约 2.5G，全部位于 /opt（/home 分区），**未占用系统盘 /**。

环境变量（已写入 ~/.bashrc 尾部，见 §5）：

```bash
export ANDROID_HOME=/opt/dev-tools/android-sdk
export ANDROID_SDK_ROOT=/opt/dev-tools/android-sdk
_ndk_latest=$(ls -d "$ANDROID_HOME"/ndk/* 2>/dev/null | sort -V | tail -1)
export PATH="$ANDROID_HOME/platform-tools:$ANDROID_HOME/cmdline-tools/latest/bin:$ANDROID_HOME/build-tools/36.0.0${_ndk_latest:+:$_ndk_latest/toolchains/llvm/prebuilt/linux-x86_64/bin}:$PATH"
unset _ndk_latest
```

> 安装要点：cmdline-tools 的 `sdkmanager` 已退化为转发 android-cli（Go 程序）。
> android-cli 偶发通过 IPv6 连接 dl.google.com 卡死（本机 IPv6 不通）。规避：
> `export JAVA_TOOL_OPTIONS=-Djava.net.preferIPv4Stack=true`（android-cli 读 JVM 变量，
> 实测有效），并加超时重试循环。IPv4 直连 dl.google.com 约 0.27s。

## 4. Slint 桌面编译依赖（独立前缀，不污染系统）

系统 glibc 2.28 无 fontconfig/freetype 开发头文件；apt 源为 Deepin 20.9 仓库，
dev 包与 glibc 2.28 完全兼容。以 **apt 下载 dev 包 → dpkg-deb 解压到独立前缀** 方式安装，
不写入系统目录、不占用 / 分区。

```text
/opt/dev-tools/slint-libs/
├── slint-env.sh             # source 后启用 PKG_CONFIG_PATH/LD_LIBRARY_PATH/CPATH
├── src/                     # （源码备用，下载失败已清理）
└── usr/
    ├── include/             # fontconfig.h, freetype2/, expat.h, png.h, uuid/
    ├── lib/x86_64-linux-gnu/
    │   ├── *.so*            # dev 链接 + 系统运行时副本
    │   └── pkgconfig/       # fontconfig/freetype2/expat/libpng/brotli/uuid .pc（prefix 已改写）
    └── share/
```

启用方式（仅当前 shell）：

```bash
source /opt/dev-tools/slint-libs/slint-env.sh
pkg-config --modversion fontconfig freetype2   # → 2.13.1 / 24.3.18
```

> 注意：fontconfig.pc 中 sysconfdir=/etc、confdir=/etc/fonts —— 运行时读取系统字体配置
> （/etc/fonts/fonts.conf），中文字体发现依赖系统已装的 Noto CJK 等（fc-list 已确认 50 个）。
> freetype 的 .pc 版本号为 24.3.18（包版本 2.12.1，ABI 兼容）。

## 5. ~/.bashrc 追加的环境变量段

（将在 Android SDK 安装完成后追加，此处为待写入内容备份）

```bash
# ---- novel-reader UI 验证环境 ----
export ANDROID_HOME=/opt/dev-tools/android-sdk
export ANDROID_SDK_ROOT=/opt/dev-tools/android-sdk
export PATH="$ANDROID_HOME/platform-tools:$ANDROID_HOME/cmdline-tools/latest/bin:$PATH"
# Slint 桌面编译依赖（按需 source，不全局 export）
# source /opt/dev-tools/slint-libs/slint-env.sh
```

## 6. 候选框架环境就绪状态

| 候选 | 桌面 | Android | 备注 |
|---|---|---|---|
| A: Slint + Android 原生（首要） | ✅ 冒烟通过 | ✅ SDK 就绪 | 分页闭环验证前置 |
| B: 三平台 Slint | ✅ 同 A 桌面 | ✅ 复用 A SDK | |
| C: Flutter | 未装 | 未装 | 后续 |
| D: Compose MP | ✅ JVM 已有 | ✅ SDK 就绪 | |
| E: Qt Quick | 未装 | 未装 | 后续 |

## 7. 验证冒烟结果（2026-09-05 全部实测通过）

### 7.1 Rust 工具链
- `rustc 1.98.0` / `cargo 1.98.0` 正常
- Android 交叉编译 targets 已装：`aarch64-linux-android`、`x86_64-linux-android`

### 7.2 Android SDK/NDK
- `adb version` → Android Debug Bridge version 1.0.41 (37.0.1)
- `sdkmanager` → 1.0.16261425 (Android CLI)
- PATH 含 aapt2 / apksigner / zipalign（build-tools/36.0.0）
- NDK 交叉编译冒烟：`aarch64-linux-android31-clang -shared` 编译 smoke.c → ELF ARM aarch64 .so ✅
- 同源 x86_64 交叉编译 ✅（模拟器用）

### 7.3 Slint 桌面编译 + 运行
- `cargo build`（debug）2m29s 成功，产物 target/debug/desktop-slint-probe（338MB 含调试符号）
- `cargo build --release` 4m50s 成功，产物 **16.9MB**（stripped + LTO，符合轻量化预期）
- 运行验证：`timeout 20 <bin>` 退出码 0，无崩溃无报错
- 窗口创建证据（xdotool 实测）：`Slint 中文冒烟测试`，Position 60,720，Geometry 480x360
- fontconfig 中文匹配：`fc-match "sans-serif:lang=zh-cn"` → `Noto Sans CJK SC` ✅
  （Slint femtovg 后端经 fontconfig 回退，简体/繁体/emoji 依赖系统已装 Noto CJK 等 50 个中文字体）
- 注意：本机当时处于锁屏状态，全屏截图呈黑屏系锁屏层所致，非应用问题；窗口本身运行正常。

### 7.4 冒烟工程位置
```text
experiments/ui-selection/hybrid-native-probe/desktop-slint/
├── Cargo.toml            # slint = "=1.17.1"
└── src/main.rs           # 中文冒烟窗口(标题/正文含简繁中文)
```

构建命令（需先 source slint-env.sh 提供 fontconfig/freetype 头与 .pc）：

```bash
source /opt/dev-tools/slint-libs/slint-env.sh
cd experiments/ui-selection/hybrid-native-probe/desktop-slint
cargo build --release && ./target/release/desktop-slint-probe
```

## 8. 遗留事项
- Slint 离屏渲染（软件渲染器输出 PNG）因需自定义 Platform 样板过多，本轮未做；改用窗口运行 + fontconfig 匹配验证替代。技术验证阶段如需无头截图，再补 MinimalSoftwareWindow 方案。
- 未装模拟器系统镜像（无 KVM，且方案 §9.1 规定模拟器仅功能调试用，性能结论须物理设备）。
- novel-reader 目录非 git 仓库（纯文档），未做版本控制。

## 9. 已创建目录骨架

```text
experiments/ui-selection/          # 方案 §6 定义
├── shared/rust-reading-core/  shared/fixtures/  shared/ffi/
├── hybrid-native-probe/desktop-slint/  hybrid-native-probe/android-native/
├── slint-probe/  flutter-probe/  compose-probe/  qt-probe/
├── scripts/  results/raw/  results/normalized/  results/screenshots/  report/
```

> 目录为空，待技术验证阶段按方案填充。

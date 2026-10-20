# Novel Reader

跨平台、本地离线的 TXT 小说阅读器。项目使用 Rust 实现共享阅读核心；桌面端采用 Rust + Slint，Android 端采用 Kotlin + Jetpack Compose，并通过 JNI/C ABI 调用 Rust 核心。

## 当前范围

- 阅读普通 TXT 文件，按需读取内容，不把整本书载入内存。
- 自动识别常见文本编码，并支持手动选择编码。
- 整页分页阅读、章节导航、阅读位置恢复与书签。
- 本地书架、阅读设置及 SQLite 持久化。
- 桌面目标为 Linux、Windows；Android 目标为 Android 12（API 31）及以上。

当前版本**不支持** ZIP、EPUB、MOBI、PDF、在线书源、账号/云同步、正文全文搜索或连续滚动。应用管理书架记录和缓存时不会修改或删除用户原始 TXT 文件。

> 本仓库仍有平台验收项待验证。不要仅根据目标平台或成功构建推断平台已完整通过；详见 [`docs/desktop/platform-requirements.md`](docs/desktop/platform-requirements.md)、[`docs/android/device-runbook.md`](docs/android/device-runbook.md) 和 [`docs/release/known-limitations.md`](docs/release/known-limitations.md)。

## 项目结构

```text
crates/
  reading-core/       阅读会话、位置与分页协调
  reading-source/     TXT 数据源、有限窗口读取、编码与指纹
  reading-index/      章节、稀疏索引、分页边界与锚点
  reading-storage/    SQLite 仓储、迁移与持久化
  reading-ffi/        Android 使用的稳定 C ABI/JNI 接口
  desktop-app/        桌面应用层（独立嵌套 Cargo workspace）
desktop/ui/            Slint 界面
desktop/packaging/     Linux/Windows 打包
android/               Kotlin/Compose 应用、JNI 桥接与 Gradle 工程
include/               C ABI 头文件
migrations/            SQLite schema 迁移
tests/                 测试夹具与端到端验证脚本
scripts/               构建、校验与冒烟脚本
docs/                  设计、开发、运行与发布说明
experiments/           技术验证资料，不属于产品构建
```

根 Cargo workspace 只包含五个共享核心 crate。桌面应用因依赖 Slint，单独位于 `crates/desktop-app` 嵌套 workspace；不要把它加入根 workspace。`experiments/` 也不得加入产品依赖图。

## 开发环境

- Rust 工具链由 [`rust-toolchain.toml`](rust-toolchain.toml) 固定为 1.98.0。
- Android 工程使用 Gradle Wrapper；当前基线为 JDK 17、Android API 31+，首发 ABI 为 `arm64-v8a`。
- Linux 桌面构建需要 Slint 使用的 fontconfig/freetype 等开发依赖；本项目开发机运行说明见 [`docs/desktop/runbook.md`](docs/desktop/runbook.md)。
- 所有 Rust 外部依赖版本须精确锁定；提交时更新相应 `Cargo.lock`。不要将密钥、本机配置、生成的 JNI 库或构建产物加入版本库。

## 构建与测试

在仓库根目录执行。

### Rust 共享核心

```bash
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

### 桌面端

`crates/desktop-app` 是独立 workspace，须显式指定 manifest：

```bash
# Linux 构建前按本机环境加载 Slint 依赖（路径可能因开发机而异）
source /opt/dev-tools/slint-libs/slint-env.sh
cargo test --manifest-path crates/desktop-app/Cargo.toml --locked
cargo build --manifest-path crates/desktop-app/Cargo.toml --release --locked
```

Linux X11 冒烟（需要可用的图形会话及相关工具）：

```bash
bash scripts/smoke-desktop.sh --backend x11 --fixture tests/fixtures/smoke.txt
```

更多运行、隔离数据目录和安全回归说明见 [`docs/desktop/runbook.md`](docs/desktop/runbook.md)。

### Android

```bash
bash scripts/build-android-libs.sh --abi arm64-v8a
bash scripts/build-android-apk.sh --abi arm64-v8a
cd android
./gradlew :app:testDebugUnitTest
```

原生库脚本会生成 `android/app/src/main/jniLibs/` 下的构建产物；不要手工编辑或提交这些 `.so`。APK 构建及签名配置见 [`docs/android/device-runbook.md`](docs/android/device-runbook.md)。构建成功不等于真机验收通过。

### 项目校验脚本

```bash
python3 scripts/verify-core-scope.py
python3 scripts/verify-v1-structure.py
python3 scripts/verify-abi.py
```

如改动涉及平台边界、发布包或对应契约，应进一步运行相关的 `scripts/verify-*.py`、端到端测试或平台门禁；以脚本当前用法和对应 runbook 为准。

## 重要设计约束

- 阅读进度、章节与书签使用稳定的内容位置语义，不以动态页码作为持久位置。
- TXT 正文必须按有限窗口读取；不得为了获得总页数而无条件全量载入或分页。
- 阅读、导入、重新关联与缓存清理都不得修改/删除用户原始文件。
- `reading-core` 的 ABI、错误码、结构布局和窗口读取契约属于兼容性边界；修改前检查 `include/novel_reader.h`、`crates/reading-ffi` 及配套契约测试。
- 设计决策、已知限制与实测状态以 `docs/` 中对应文档为准；未实测事项必须标记为“待验证”。

## 文档导航

- [V1 实际开发方案](docs/novel-reader-v1-development-plan.md)
- [总体方案](docs/novel-reader-solution.md)
- [桌面运行说明](docs/desktop/runbook.md)
- [桌面平台要求与验收状态](docs/desktop/platform-requirements.md)
- [Android 真机 runbook](docs/android/device-runbook.md)
- [发布文档索引](docs/release/README.md)
- [已知限制与延期验收项](docs/release/known-limitations.md)

## 许可

本项目以 GPL-3.0-only 发布，详见 [`LICENSE`](LICENSE)。第三方依赖及发布许可注意事项记录在 [`docs/release/known-limitations.md`](docs/release/known-limitations.md)。

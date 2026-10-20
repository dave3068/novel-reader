# AGENTS.md

本文件为在本仓库工作的编码代理提供项目级约束。开始修改前先阅读相关实现和 `docs/` 中对应设计/runbook；不要仅凭旧计划文档推断当前代码状态。

## 项目结构与 Cargo 边界

- 根 Cargo workspace 的正式成员固定为 `reading-core`、`reading-ffi`、`reading-index`、`reading-source`、`reading-storage`。修改成员、排除项或核心依赖边界前，先核对 `scripts/verify-core-scope.py` 和核心门禁设计。
- `crates/desktop-app` 有自己的 `[workspace]`，是嵌套 workspace。桌面 Cargo 命令必须使用 `--manifest-path crates/desktop-app/Cargo.toml`；不要为了方便把桌面 crate 加进根 workspace。
- `experiments/` 是技术探针，明确排除在正式产品依赖图和构建之外。不要从探针引入模拟书库、合成生产数据源或空的 External 数据源。
- 分层职责：共享阅读语义与存储在 Rust 核心；桌面为 Rust/Slint；Android 为 Kotlin/Compose，通过 `reading-ffi` 和 `include/novel_reader.h` 的稳定 C ABI 接入。避免在 UI 层复制核心编码、章节、位置或分页算法。

## 必须保持的行为与兼容性

- TXT 按有限窗口读取，不将全文读入内存；不要以强制完整分页来伪造总页数或牺牲大文件性能。
- 阅读位置以内容位置/字节偏移等稳定锚点表达。页码会受布局影响，不可替代持久化位置。
- 用户原始 TXT 只读：任何导入、阅读、删除书架记录、重关联和缓存清理行为都不得修改或删除原文件。
- ABI、错误码、结构布局、读取窗口上限及取消/缓冲区所有权语义均为跨语言契约。涉及修改时同时检查 `include/novel_reader.h`、`crates/reading-ffi`、Android JNI/Kotlin 桥接及 ABI 测试/校验脚本；不要未经明确需求做破坏性调整。
- 数据库 schema 通过 `migrations/` 的递增迁移演进。不得重写既有迁移来改变已发布数据库语义；迁移应保留用户数据并有迁移测试。
- 依赖必须精确锁定版本（Rust 使用 `=X.Y.Z`），保持 Cargo/Gradle 锁文件同步；除非任务明确要求，不新增网络、遥测或后台服务依赖。
- 许可基线为 GPL-3.0-only。新增依赖或资源时检查其许可并更新必要的许可/发布文档。

## 修改方式

- 先理解现有调用路径与测试，不做无关重构，不删除或弱化门禁来使改动“通过”。
- 优先沿用项目已有错误类型、取消模型、仓储 API 和设置/位置语义；跨平台行为尽量在核心层定义并测试。
- UI 排版测量必须与最终渲染使用一致的字体、尺寸、行距及边距口径，避免页边界与显示内容不一致。
- 新行为添加针对性测试，特别是编码边界、超大文件有限读取、位置恢复、并发/取消、ABI 边界、迁移及原文件安全。
- Android JNI 生成库属于构建产物。通过仓库脚本重建，不能手工替换或静默提交二进制。
- 本机设置、签名材料、私有密钥、真实设备/用户数据和 `results/` 里的本地验证记录不得写入公开文档或提交。不要覆盖 `android/local.properties` 中与 `sdk.dir` 无关的签名键。

## 常用验证

按改动范围选择验证，不要宣称未运行的检查已通过。

```bash
# 根 workspace（五个共享核心 crate）
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings

# 桌面嵌套 workspace
cargo test --manifest-path crates/desktop-app/Cargo.toml --locked
cargo build --manifest-path crates/desktop-app/Cargo.toml --release --locked

# Android
(cd android && ./gradlew :app:testDebugUnitTest)
bash scripts/build-android-libs.sh --abi arm64-v8a
bash scripts/build-android-apk.sh --abi arm64-v8a

# 项目边界/结构/ABI
python3 scripts/verify-core-scope.py
python3 scripts/verify-v1-structure.py
python3 scripts/verify-abi.py
```

- Linux 桌面构建可能需要先加载本机 Slint 原生依赖环境，具体见 `docs/desktop/runbook.md`；不要把某台开发机的绝对路径当作通用依赖。
- `cargo test --workspace` 不会自动包含桌面嵌套 workspace，桌面测试需单独运行。
- X11 桌面冒烟需真实可用的图形会话：`bash scripts/smoke-desktop.sh --backend x11 --fixture tests/fixtures/smoke.txt`。无图形环境、无真机或无目标系统时应如实记录未验证，而不是跳过后宣称通过。
- 对 Android 平台行为，单元测试和 APK 校验不替代物理设备验收；遵循 `docs/android/device-runbook.md`。
- 修改发布/平台清单后，运行对应 `verify-*.py` 或打包校验，并保持延期项状态与证据一致。

## 文档与状态准确性

- `docs/novel-reader-v1-development-plan.md` 是开发基线，可能包含早期计划；如与当前代码或较新的裁定/验收文档冲突，先核实对应实现、`docs/release/known-limitations.md`、平台状态清单和 runbook，并在必要时同步文档。
- 只把真实执行且有证据的结果描述为“通过”。构建通过不等于设备、系统、渲染、输入或发布验收通过；未实测项目使用“待验证”。
- 不在 README、提交说明或测试输出中加入本机路径之外的私有信息、设备标识、签名指纹或密钥。
- 改变用户可见行为、构建/运行方式、支持范围、数据安全或 ABI 约定时，更新对应 `docs/` 与根 `README.md`。

# 父级验证清单

## 环境

- 计划工程根目录：`${NR_PROJECT_DIR}`。
- Rust：1.98.0；Linux 桌面验证基线：Deepin 20.9、x86_64、X11。
- Android：JDK 17、SDK 36、NDK 27.3.13750724；真机验证不属于当前执行阶段。
- 原型证据目录：`experiments/ui-selection/`；正式实现不得把原型占位数据带入发布包。

## 必需程序化检查

- [ ] C1 (P1, PR2, PR5, PR6) — `file:` `python3 scripts/verify-v1-structure.py`; expect exit 0，正式 workspace、三个子规格要求的目录、迁移、脚本和 ABI 头文件存在，且正式代码不依赖 `experiments/` 的运行时模拟源。
- [ ] C2 (P1, PR2, PR5) — `test:` `source /opt/dev-tools/slint-libs/slint-env.sh && cargo test --workspace --all-targets`; expect exit 0，Rust 核心、索引、存储和 ABI 契约测试全部通过（桌面 crate 加入 workspace 后必须保留 `slint-env` 前缀，否则构建缺 fontconfig/freetype）。
- [ ] C3 (P1, PR5) — `test:` `cargo test -p reading-core --release -- bounded_window position pagination`; expect exit 0，100MB/1GB 有限窗口、UTF-8 边界、正反向分页和锚点恢复通过。
- [ ] C4 (P2, PR1, PR3) — `cmd:` `source /opt/dev-tools/slint-libs/slint-env.sh && cargo build --release -p desktop-app`; expect exit 0，桌面 Release 构建成功。
- [ ] C5 (P2, PR1, PR3, PR5) — `cmd:` `bash scripts/smoke-desktop.sh --backend x11 --fixture tests/fixtures/smoke.txt`; expect exit 0，真实 TXT 导入、首屏、翻页、设置重排和退出清理有结构化结果。
- [ ] C6 (P3, PR2, PR4) — `cmd:` `cd android && ./gradlew :app:testDebugUnitTest :app:assembleDebug`; expect exit 0，Android JVM 单测通过且 Debug APK 生成。
- [ ] C7 (P3, PR2, PR4) — `cmd:` `bash scripts/build-android-libs.sh --abi arm64-v8a`; expect exit 0，Rust Android 共享库与自包含 JNI 库生成并放入预期 ABI 目录。
- [ ] C8 (P3, PR4, PR6) — `cmd:` `python3 scripts/verify-android-apk.py android/app/build/outputs/apk/debug/app-debug.apk`; expect exit 0，APK 包名、minSdk/targetSdk、arm64 JNI 库、DEX 和签名/内容状态符合规格。
- [ ] C9 (P4, PR1, PR5) — `cmd:` `python3 tests/e2e/run_txt_lifecycle.py --platform desktop --fixture tests/fixtures/smoke.txt`; expect exit 0，导入→阅读→保存→重启→恢复位置→清理缓存流程通过，原始 TXT 仍存在。
- [ ] C10 (P5, PR2, PR4) — `grep:` `python3 scripts/verify-android-boundary.py`; expect exit 0，Kotlin 不包含章节/位置/分页业务复制，JNI 入口、错误码、释放和取消契约完整。
- [ ] C11 (P6, PR6) — `cmd:` `python3 scripts/verify-v1-release.py --desktop --android`; expect exit 0，发布物、版本、依赖许可证、构建标识和已知未验证项完整。

## 回归检查

- [ ] C12 (P6) — `file:` `git diff --exit-code -- docs/novel-reader-solution.md docs/novel-reader-v1-development-plan.md`; expect exit 0，正式开发没有反向改写既有方案文档。
- [ ] C13 (P6, PR5) — `test:` `python3 tests/e2e/run_txt_safety.py`; expect exit 0，移除书架记录和清理缓存不删除原始 TXT、进度与书签。

## 延期/人工建议检查

- [ ] M1 (manual-advisory, PAC2) — action: 在 Linux 桌面实际选择 TXT，完成目录、翻页、字号、暗黑模式、书签和系统关闭操作；expected: 与 C5/C9 的程序化结果一致；programmatic fallback: C5、C9；final reminder: 交付后请您进行一次视觉和操作复核。
- [ ] M2 (manual-advisory, PAC3) — action: 后续在 Android 12+ 真机安装 APK，验证 SAF、旋转、后台恢复、系统返回和 JNI 取消；expected: 补充真实设备结果；programmatic fallback: C6、C7、C8、C10；final reminder: 当前不把真机项计为已通过。
- [ ] M3 (manual-advisory, PAC2/PAC6) — action: 后续在 Windows 10/11 和 Linux Wayland 干净环境运行发布物；expected: 补充跨平台发布结果；programmatic fallback: C4、C11；final reminder: 完成前不宣称三平台发布完成。

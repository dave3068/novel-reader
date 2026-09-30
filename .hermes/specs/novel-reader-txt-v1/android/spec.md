# android：Android TXT 阅读器

## 背景与目标

在 `reading-core` 正式核心之上建立 Kotlin + Jetpack Compose Android 应用，支持 Android 12+ 的 TXT 导入和阅读。按照当前执行策略，Android 先完成源码、JNI/ABI、JVM 单元测试、Rust 交叉编译、APK 构建和内容校验；真机调试及旋转、后台恢复、权限失效和性能结果等待后续设备条件具备。

## 范围

### 范围内

- Kotlin/Compose 书架、阅读、设置和状态页面。
- Android SAF `OpenDocument`、持久 URI 权限、TXT 元数据读取和重新关联入口。
- JNI/C ABI 调用 Rust 核心；有限窗口正文读取；Android 文本测量和页末位置提交。
- ViewModel 状态串行化、重排请求合并、错误处理和取消入口。
- JVM 单元测试、Rust Android 交叉编译、JNI 自包含 `.so`、Debug/Release APK 和内容校验。
- 为后续真机调试提供构建标记、测试标签和操作指南。

### 范围外

- 当前阶段不要求 Android 真机安装、完整 UI instrumentation、旋转/后台/进程回收的真实设备证据。
- ZIP/EPUB/其他格式、网络、同步、连续滚动和背景图片。
- Kotlin 复制 Rust 的章节、位置、分页、缓存失效或书签业务。
- 使用整本正文 ByteArray 或长期持有 Activity 引用。

## Requirements

### AR1 — Android TXT 导入

Android SHALL 使用系统 SAF 选择 TXT，保存持久 URI 权限和来源信息，并将有限窗口读取交给共享核心协调；非 TXT、空文件和权限失败必须有明确状态。

### AR2 — JNI 共享核心边界

Android SHALL 通过稳定 JNI/C ABI 使用 Rust 的位置、分页、章节、进度、书签和错误语义，明确句柄、64 位偏移、缓冲区释放、请求取消和结果版本。

### AR3 — Android 中文分页

Android SHALL 使用统一像素测量口径和实际渲染行高，向共享核心提交合法 UTF-8 页末位置；不得因为 Compose 默认字距/行高造成半字、滚动正文或页面裁切。

### AR4 — Compose 状态与生命周期

Android SHALL 通过 ViewModel 管理阅读状态，打开、切章、翻页、重排和保存位置串行化；字号拖动产生的重排请求必须合并；Activity/配置变更不应使过期结果覆盖当前页面。

### AR5 — 编译与自动化测试

Android SHALL 完成 JVM 单元测试、Rust Android arm64 交叉编译、JNI 链接、Debug APK 构建和 APK 内容/签名校验。当前不把未执行的真机项目标记为通过。

### AR6 — 真机延期边界

在没有明确 Android 12+ 真机调试条件时，Android SHALL 将真实 SAF、旋转、后台恢复、系统返回、JNI 取消和功耗结果列为 `待验证`，不得用 APK 构建成功替代。

## Non-Functional Requirements

- NFR-A1：minSdk 31，首发 ABI 为 arm64-v8a；x86_64 只作为可选模拟器/调试目标。
- NFR-A2：Android 正文窗口不超过核心硬上限；SAF 不支持随机访问时使用可取消的应用私有缓存复制。
- NFR-A3：targetSdk 36 时正确处理 edge-to-edge、状态栏和导航栏内边距。
- NFR-A4：Compose 正文显式控制 `letterSpacing`、字号和行高；正文不使用 `verticalScroll` 伪装分页。
- NFR-A5：APK 通过 `aapt2`、内容清单、JNI 库和 `apksigner` 校验；增量打包前清理旧 APK。

## Impact Analysis

- Affected code：`android/`、`crates/reading-ffi/`、`scripts/build-android-libs.sh`、`scripts/build-android-apk.sh`、`tests/android/`。
- Migration：参考 `experiments/ui-selection/hybrid-native-probe/android-native/`，但必须替换探针的模拟书籍和空 External 数据源。
- API：Android Kotlin 声明与 Rust C ABI/JNI 版本同步；无网络 API。
- Current limitation：当前真机自动化和生命周期证据延期，不阻塞 Android 编译交付，也不允许宣称 Android 全面通过。

## Acceptance Criteria

- AC-A1 (AR1)：SAF 代码、权限持久化、TXT 元数据和错误状态的 JVM/静态检查通过。
- AC-A2 (AR2)：ABI 头文件、JNI 声明、Rust 导出和边界检查一致，Kotlin 无阅读业务复制。
- AC-A3 (AR3)：Android 测量单测和共享分页契约通过，字距/行高/像素口径固定。
- AC-A4 (AR4)：ViewModel 并发/重排合并测试通过，过期结果不会覆盖当前状态。
- AC-A5 (AR5)：JVM 单测、Rust arm64 交叉编译、JNI `.so`、APK 构建和内容校验通过。
- AC-A6 (AR6)：真机延期项、命令、预期结果和当前状态完整记录。

## Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| SAF Provider 不支持 seek | Medium | 大文件无法跳页 | 检测随机访问；不支持时复制私有缓存；复制可取消 |
| Compose 测量与渲染口径不同 | High | 半字/滚动条/裁切 | 像素统一、显式 0 字距、回传实际行高、整行安全余量 |
| JNI 句柄/缓冲区生命周期错误 | Medium | 崩溃/泄漏 | 不透明句柄、release 契约、错误码和 JVM/ABI 测试 |
| 真机验证延期被误报完成 | High | 发布质量失真 | 使用 `待验证` 状态；构建门禁与真机门禁分离 |
| Android 与桌面业务分叉 | Medium | 进度/书签不一致 | Rust 核心语义共享；Kotlin 仅编排和平台适配 |

## Open Questions and Deferred Decisions

- Blocking：无。Android 代码、单测和编译路径可以先行。
- Deferred：真机型号、instrumentation 开关、后台/旋转/进程回收验收、最终 AAB/商店发布。
- Accepted risk：真机结果缺失期间，只能交付“可构建/可测试 APK”，不能交付“Android 全面通过”结论。

## References

- `docs/novel-reader-v1-development-plan.md`
- `docs/ui-selection/android.md`
- `experiments/ui-selection/hybrid-native-probe/android-native/`
- [父级设计](../design.md)

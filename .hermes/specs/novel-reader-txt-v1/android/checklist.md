# android 验证清单

## Environment

- JDK 17；Android SDK platform/build-tools 36；NDK 27.3.13750724。
- Android 构建目录：`android/`；首发 ABI：`arm64-v8a`。
- 当前不要求真机；真机项全部属于后续 `manual-advisory`。

## Programmatic Required Checks

- [ ] C1 (A-T1, AR5) — `cmd:` `cd android && ./gradlew :app:assembleDebug`; expect exit 0，Debug APK 生成。
- [ ] C2 (A-T2, AR2) — `cmd:` `python3 scripts/verify-android-boundary.py --header include/novel_reader.h --kotlin android/app/src/main/java`; expect exit 0，JNI 声明/导出、错误码、句柄、释放、取消和 Kotlin 业务边界一致。
- [ ] C3 (A-T3, AR3, AR4) — `test:` `cd android && ./gradlew :app:testDebugUnitTest`; expect exit 0，页边界数学、UTF-8 偏移、ViewModel 串行化/重排合并和设置状态测试通过。
- [ ] C4 (A-T4, AR1) — `test:` `cd android && ./gradlew :app:testDebugUnitTest --tests '*Saf*'`; expect exit 0，URI 元数据、权限状态、窗口读取和私有缓存回退测试通过。
- [ ] C5 (A-T5, AR3) — `test:` `cd android && ./gradlew :app:testDebugUnitTest --tests '*Pagination*'`; expect exit 0，像素字号、实际行高、字距、合法页末和无滚动伪分页契约通过。
- [ ] C6 (A-T6, AR5) — `cmd:` `bash scripts/build-android-libs.sh --abi arm64-v8a`; expect exit 0，Rust arm64 共享库和自包含 `libnr_jni.so` 生成。
- [ ] C7 (A-T6, AR5) — `cmd:` `cd android && ./gradlew :app:testDebugUnitTest :app:assembleDebug :app:assembleRelease`; expect exit 0，Android 单测、Debug 和 Release 构建全部通过。
- [ ] C8 (A-T6, AR5) — `cmd:` `python3 scripts/verify-android-apk.py android/app/build/outputs/apk/debug/app-debug.apk --abi arm64-v8a`; expect exit 0，包名、minSdk/targetSdk、JNI 库、DEX、构建标记和签名/内容状态符合规格。
- [ ] C9 (A-T7, AR6) — `file:` `python3 scripts/verify-android-device-readiness.py --docs docs/android --results results/android`; expect exit 0，真机操作步骤、预期结果、当前 `待验证` 状态和构建标记完整。

## Regression Checks

- [ ] C10 — `cmd:` `cd android && ./gradlew lintDebug`; expect exit 0 或仅允许已登记的非阻断告警，不得有 Kotlin 编译错误。
- [ ] C11 — `grep:` `python3 scripts/verify-android-scope.py`; expect exit 0，Kotlin 不包含章节/位置/分页/书签业务复制，不传整本正文。

## Advisory Manual Checks

- [ ] M1 (manual-advisory, AC-A1/AC-A3) — action: 后续在 Android 12+ 真机选择 UTF-8/GB18030/UTF-16 TXT 并翻页；expected: 中文无乱码、无滚动正文、无半字裁切；programmatic fallback: C3、C5、C8；final reminder: 真机条件具备后请您执行。
- [ ] M2 (manual-advisory, AC-A4/AC-A6) — action: 后续在真机横竖屏、Home/恢复、系统返回、权限失效和进程重建；expected: 位置与状态正确恢复；programmatic fallback: C3、C9；final reminder: 当前不把这些项目计为已通过。
- [ ] M3 (manual-advisory, AC-A2) — action: 后续在真机执行批次读取中途退出页面，观察 JNI 取消和资源释放；expected: 无应用崩溃、无旧结果覆盖；programmatic fallback: C2、C3、C11；final reminder: 设备验证完成后补写证据。

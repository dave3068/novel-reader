# 候选 D：Compose Multiplatform UI 验证原型

## 目标
使用 Compose Multiplatform Desktop + Rust FFI 验证 `TextMeasurer`/`TextLayoutResult` 中文分页、JVM/Skia 桌面运行和条件完整原型。

## 范围
### 范围内
- `experiments/ui-selection/compose-probe/` Desktop 分页尖峰、FFI 桥、测试和条件完整原型。
- JVM、Skia、桥接复制与线程切换的可观测信息。
### 范围外
- Android/Windows 实机、安装器、裁剪后运行时包体结论和正式架构。

## 需求
### CM-R1 — Compose 公共 API 分页
原型 SHALL 使用稳定文字布局 API 产生合法 UTF-8 页末位置并完成 1,000 页、反向和重排判定。
### CM-R2 — 稳定 Rust 边界
原型 SHALL 通过固定 C ABI 的明确 JNI/JNA/其他稳定桥接实现有限窗口、释放、回调、取消和指标；桥接选择须记录理由。
### CM-R3 — 条件完整原型
分页门槛通过后 SHALL 实现统一三页面和 UI 探针；失败时停止并留证。
### CM-R4 — 当前 Linux 启动
Desktop Release 构建 SHALL 使用锁定 JDK/Gradle，在当前 Deepin/X11 写出就绪 JSON 并正常退出。

## 验收标准
- CM-AC1：分页结果无非法边界并有明确能力状态。
- CM-AC2：桥接生命周期、线程、取消和复制测试通过。
- CM-AC3：Release 构建包含其实际运行所需 JVM/Skia 依赖口径说明，X11 可启动。
- CM-AC4：完整原型受分页门禁控制。

## 风险
| 风险 | 缓解 |
|---|---|
| Gradle/Kotlin/Compose/JDK 版本耦合 | Wrapper 与插件版本全部锁定，使用 JDK 17 基线 |
| JNI/JNA 引入隐式复制或崩溃 | 统一 C ABI、边界/寿命压力测试和复制计数 |
| Deepin 旧系统与 Skia 不兼容 | 先最小启动尖峰；保存 JVM/native 崩溃证据 |

## 开放与延期
- 桥接实现由 CM-T1 尖峰在 JNI/JNA/稳定 C ABI 封装中选择并记录，不影响对外 ABI。

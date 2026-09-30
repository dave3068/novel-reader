# 候选 C：Flutter UI 验证原型

## 目标
使用 Flutter + `dart:ffi` 在统一契约下验证 `TextPainter`/`Paragraph` 中文页末定位、Rust 双向调用和条件完整三页面原型。

## 范围
### 范围内
- `experiments/ui-selection/flutter-probe/` Linux 分页尖峰、测试与条件完整原型。
- Dart/Rust 字符串复制、回调、取消和线程切换测量。
### 范围外
- Android/Windows 实机、商店包、正式插件架构和性能评分。

## 需求
### FL-R1 — TextPainter/Paragraph 分页
原型 SHALL 使用公开 API 返回合法 UTF-8 页末位置，并通过正向 1,000 页、反向和重排判定。
### FL-R2 — Rust FFI 边界
原型 SHALL 通过共同 C ABI 按窗口获取正文，正确释放缓冲、处理回调/取消并记录复制字节。
### FL-R3 — 条件完整原型
分页门槛通过后 SHALL 实现统一三页面、虚拟化书架和稳定 UI 状态探针；失败时停止并留证。
### FL-R4 — 当前 Linux 启动
Linux Release 构建 SHALL 在当前 Deepin/X11 写出就绪 JSON并正常退出。

## 验收标准
- FL-AC1：分页判定工具输出合法能力状态及可复现证据。
- FL-AC2：FFI 不复制整本正文，生命周期、线程和取消测试通过。
- FL-AC3：Release 构建和 X11 启动通过。
- FL-AC4：完整原型严格受分页门禁控制。

## 风险
| 风险 | 缓解 |
|---|---|
| Dart 字符索引到 UTF-8 字节偏移映射错误 | 独立映射表和复杂 Unicode 属性测试 |
| Flutter Linux 依赖与 Deepin 兼容性 | 锁 SDK；预检 GTK/Clang/CMake/Ninja；保存诊断 |
| 引擎空闲调度 | 本规格只植入帧计数探针，不作功耗结论 |

## 开放与延期
- Flutter 版本在共享预检中锁定；Android 分 ABI 和 Windows 包体延期。

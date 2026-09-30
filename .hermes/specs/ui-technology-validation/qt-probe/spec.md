# 候选 E：Qt Quick/QML UI 验证原型

## 目标
使用 Qt Quick/QML + C++ 薄桥 + Rust C ABI 验证 `QTextLayout` 中文分页、桥接安全边界和条件完整原型。

## 范围
### 范围内
- `experiments/ui-selection/qt-probe/` CMake 工程、QML、分页尖峰、C++/Rust 桥测试和条件完整原型。
- 开源 Qt 模块与动态链接依赖清单，供后续许可证审查。
### 范围外
- 商业 Qt 模块、静态链接发布、Android/Windows 实机、安装包和最终许可结论。

## 需求
### QT-R1 — QTextLayout 分页
原型 SHALL 使用稳定公共 API 返回合法 UTF-8 页末位置，完成 1,000 页、反向和重排判定。
### QT-R2 — 薄且安全的桥接层
C++ 层 SHALL 只负责 Qt/C ABI 适配，正文有限窗口传输，生命周期、错误、回调、取消和复制量可测。
### QT-R3 — 条件完整原型
分页门槛通过后 SHALL 实现统一三页面和 QML 状态探针；失败时停止并留证。
### QT-R4 — 当前 Linux 启动
Release 构建 SHALL 在当前 Deepin/X11 使用动态 Qt 库启动、写出就绪 JSON 并正常退出。
### QT-R5 — 许可输入清单
原型 SHALL 只选明确记录的开源 Qt 模块，输出动态链接模块、插件和许可文件清单，但不在本规格作最终法律结论。

## 验收标准
- QT-AC1：分页能力有合法结果与证据。
- QT-AC2：C++ 桥无跨边界悬空内存，测试覆盖回调/取消/释放。
- QT-AC3：Release X11 可启动，Qt/QML/插件依赖清单完整。
- QT-AC4：完整原型严格受分页门禁控制。

## 风险
| 风险 | 缓解 |
|---|---|
| Qt 6 与 Deepin 20.9 系统库不兼容 | 用户目录锁定 Qt；最小启动先行；不覆盖系统 Qt |
| C++ 桥引入所有权缺陷 | RAII 包装、sanitizer 开发检查、Release ABI smoke |
| LGPL 动态链接义务遗漏 | 限定模块并生成依赖/许可清单，最终法务判断延期 |

## 开放与延期
- Qt 稳定版本及安装来源在共享预检中锁定；最终许可证选择延期。

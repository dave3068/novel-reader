# 候选 B：三平台 Slint UI 验证原型

## 目标
使用 Rust + Slint 在统一契约下验证中文分页与 Rust 直接/共同 C ABI 集成，并比较 Winit+FemtoVG、Winit+Skia；软件渲染器仅产生中文能力诊断。

## 范围
### 范围内
- `experiments/ui-selection/slint-probe/` 分页尖峰、测试、条件完整三页面。
- FemtoVG 与 Skia 的独立 Release 构建/启动证据；软件渲染器诊断结果。
### 范围外
- 自定义私有排版 API、正式产品代码、Android/Windows 实机运行、性能评分。

## 需求
### SL-R1 — 公共 API 分页
原型 SHALL 通过 Slint 稳定公共接口得到合法 UTF-8 页末位置并完成 1,000 页正向、反向与重排检查。
### SL-R2 — 等价 Rust 边界
原型 SHALL 保留共同 C ABI 基准；可另记 Rust 直接调用，但不得用其替代基准结果。
### SL-R3 — 多渲染器证据
FemtoVG 与 Skia SHALL 独立构建、启动和记录；软件渲染器 SHALL 记录中文渲染诊断而不默认作为完整原型后端。
### SL-R4 — 条件完整原型
分页门槛通过后 SHALL 实现共享设计中的三页面与 UI 状态探针；未通过则输出淘汰证据并停止该阶段。
### SL-R5 — 当前 Linux 启动
分页尖峰 SHALL 在当前 Deepin/X11 Release 模式显式写出就绪 JSON 并正常退出。

## 验收标准
- SL-AC1：分页测试无丢字、重字、非法边界、死循环，重排结果可锚定。
- SL-AC2：共同 C ABI 调用、释放、回调切回 UI 线程和复制字节指标通过。
- SL-AC3：FemtoVG/Skia 构建及启动证据分离；软件渲染诊断有明确状态与原因。
- SL-AC4：候选判定和是否进入完整原型严格遵循门禁。

## 风险
| 风险 | 缓解 |
|---|---|
| Slint 文本接口无法稳定映射页末 | 尖峰先行；不得私有补丁；失败即记录不通过 |
| 软件渲染器中文限制 | 只诊断，完整原型使用通过门槛的 GPU 后端 |
| Skia 在旧 Linux 工具链构建困难 | 隔离依赖和日志；不能启动则子规格不完成 |

## 开放与延期
- 阻塞：无；具体 Slint 稳定版本在共享工具链任务锁定。
- 延期：Android/Windows 实测与发布包评估。

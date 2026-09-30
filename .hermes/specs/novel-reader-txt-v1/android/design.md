# android 设计细节

## 架构

```text
Compose Screen
  → ViewModel（StateFlow + Mutex + request/layout id）
  → Kotlin thin core wrapper
  → JNI `libnr_jni.so`
  → Rust reading-core
```

Compose 只显示状态，不保存第二份章节/分页业务。所有打开书籍、翻页、重排、保存位置操作在 ViewModel 入口串行化；拖动字号时先同步 UI 状态，再合并后台重排。

## SAF 数据源

使用 `OpenDocument` 获取 URI 和持久权限。若 Provider 支持可靠定位，按窗口读取；否则复制到应用私有缓存后读取。Rust 负责坐标长度、边界和位置语义，Kotlin 负责实际 `ContentResolver` 字节访问。正式版不得保留验证原型 `ExternalTextSource` 返回空字节的行为。

## 排版

Android 侧统一用像素参与测量：`fontSizePx = fontSizeSp × density`。Compose 正文显式指定与测量一致的 `letterSpacing`、`fontSize` 和 `lineHeight`；测量得到的实际行高回传渲染。正文区域不加 `verticalScroll`，整页结束由合法 UTF-8 字节偏移决定。

## 当前验证边界

JVM 单测、Rust Android 交叉编译、JNI 链接和 APK 内容可以在当前开发机完成。真机安装/交互、旋转、后台、权限失效、进程回收和功耗必须等设备条件具备后执行，并保持独立结果文件，不通过构建结果推断。

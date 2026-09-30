# V1 设计与子规格边界

## 1. 依赖关系

```text
reading-core
  ├── Rust 直接 API ── desktop / Slint
  └── C ABI + JNI ─── android / Compose

reading-core
  └── SQLite schema + migrations

desktop / android
  └── 平台数据源、文本测量、交互和生命周期
```

实施顺序为 `reading-core → desktop`，Android 在核心 API 冻结后可并行，但资源优先保障 desktop。Android 的真实设备验证不阻塞当前桌面开发，却阻塞最终 Android/V1 全平台发布结论。

## 2. 核心边界

### Rust 共享核心

负责 `TextSource`、编码状态、章节区间、`ContentPosition`、`PageBoundary`、锚点、布局签名、索引任务、SQLite 仓储、错误码、取消和缓存失效。正文按有限窗口读取，永不返回整本文件。

### 桌面端

Rust 应用层直接调用共享 API；Slint 只负责页面、状态投影、鼠标/键盘、窗口尺寸和可见文本渲染。文件选择使用桌面平台适配器。Slint 文本实际渲染参数必须与页面测量参数一致。

### Android 端

Kotlin/Compose 负责 Activity、ViewModel、SAF、权限、系统返回、edge-to-edge、Android 文本测量和 UI 语义。Rust 通过自包含 `libnr_jni.so` 作为正式核心。SAF 不支持随机读取时复制到私有缓存；不得保留原型的空 `ExternalTextSource` 行为。

## 3. 关键数据流

### 导入 TXT

```text
文件选择
  → 读取元数据/权限
  → 生成来源指纹
  → 创建 books 记录
  → 有限窗口识别编码
  → 建立首个可读页面
  → 后台章节索引
```

### 翻页

```text
ContentPosition + LayoutSignature
  → bounded read
  → 平台文本测量
  → 合法 UTF-8 end byte
  → commit page boundary
  → 页面缓存/进度保存
```

### 重排

```text
设置变化
  → 新 LayoutSignature
  → 失效旧分页边界
  → 以当前位置/锚点重新测量
  → request_id/layout_id 校验
  → 发布新页面
```

## 4. 跨语言规则

- 所有 ID、文件长度和字节偏移使用 64 位；跨 JSON/Java/Kotlin 时使用字符串或保证不发生精度截断。
- 字符串统一 UTF-8 + 长度；不得把语言专属对象跨边界传递。
- 返回缓冲区的所有权和释放函数写入 ABI 头文件，并由 Kotlin/C/Rust 测试覆盖。
- JNI 错误使用稳定错误码与详情；不能用错误文本作为业务分支。
- 每个异步请求带 request id；每次排版带 layout id；过期结果必须丢弃。
- 同一个阅读会话的打开、切章、翻页、重排、保存位置串行化；重排请求合并。

## 5. V1 数据与版本

普通 TXT 是一个 `content_unit`；章节只是区间。布局缓存由 `layout_signature + content_index_version` 标识。编码或原始文件指纹变化只使可重建索引失效，不删除书籍、进度和书签。

正式迁移脚本按 `migrations/0001_*.sql`、`0002_*.sql` 递增，升级前备份 SQLite。原型数据库和模拟书库不迁入用户环境。

## 6. 不采用的方案

- 不采用三平台统一 UI 来换取表面代码复用；当前验证表明 Android 原生平台能力和桌面 Slint 的组合更适合本项目。
- 不在 Rust 核心中把最终字形光栅化；两端字体、密度和文本布局不同，核心只统一位置和边界语义。
- 不以截图 OCR、整本正文复制或私有排版 API 实现分页。
- 不在 V1 引入 ZIP，以免同时引入多内容单元、排序、解压缓存和跨单元位置映射。

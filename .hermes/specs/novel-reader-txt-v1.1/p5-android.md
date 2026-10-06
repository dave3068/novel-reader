# 规格：V1.1-P5 Android 同语义入口（FFI 加性契约修订）

> 状态：**已批准，待实施**
> 基线：`novel-reader` v0.3.0 + V1.1 桌面四组功能（P1–P4 已完成）
> 主人裁定路线：**A —— 先写规格 + 导出清单 + 门禁**，据真实工作量再决定是否继续实施。
> 日期：2026-10-05

---

## 1. 背景与裁定

### 1.1 为什么 P5 不是「补几个入口」

桌面侧 V1.1 的四组功能（分类树 / 阅读历史 / 派生配色 / 全局背景图）全部落在**核心 SQLite**里
（`book_categories` / `reading_history` / `reader_theme_custom` / `reader_background`）。
Android 侧**读不到这些表** —— 冻结 FFI 的 19 个导出里没有任何书架列表、设置读写、分类、历史、
主题或背景的入口。

宿主侧现有的 `ReaderSettingsStore` / `HostBookCatalog`（Kotlin 侧 KV/SharedPreferences）是
**已记录的权宜之计**，与核心 SQLite **不同源**。

### 1.2 路线 B（Kotlin host 存储）被否决的理由

若在 Kotlin 侧另做一套分类/历史/主题/背景：

- 用户在桌面建的分类，Android 看不到；反之亦然 → **两个真相**；
- 技能明确记载 host store 是「等一张卡做 FFI 统一」的权宜之计；
- 违反规格「不得制造两个真相」。

### 1.3 路线 A 的性质：**加性契约修订**

按 `references/frozen-abi-extension.md`：

- 既有 19 个导出**保持名称、语义、顺序**不变；
- `NR_ABI_VERSION` **不升**（这是加法，不是行为变更）；
- 成本在**所有点名该集合的计数与清单**（12 处站点）。

---

## 2. 范围

### 2.1 本期交付（纯文档 + 门禁，本轮）

| 项 | 内容 |
|---|---|
S1 | 本规格（含导出清单与工作量勘察） |
| S2 | `docs/design/novel-reader-v1.1-android-design.md`（Android 侧四组功能的界面设计） |
| S3 | `verify-android-v11-parity.py` 静态门禁：核对 12 处站点齐备且计数一致 |
| S4 | 变异测试证据（门禁在坏样本上必须失败） |

### 2.2 后续实施（状态更新于 2026-10-06）

| 项 | 内容 | 状态 |
|---|---|---|
| T1 | FFI 导出点（§3 清单）—— 分 4 批，每批一个功能 | ✅ **完成**：19 → **35** 项（B1 分类 4 + B2 历史 3 + B3 配色 5 + B4 背景 4） |
| T2 | Kotlin 侧 `NrCore` / `NrAbi` / `NrEngine` 接线 + 测试替身同步 | ✅ **完成**：`NrCore` / `NrAbi` / **`NrEngine` 门面 16 个方法** / 两个测试替身 / `NrJniBoundaryTest` / **DTO 小端解析**全部就位 |
| T3 | Compose 界面（书架分类侧栏、历史面板、设置页配色区、阅读页背景） | ⚠️ **三块已交付 + 前置已闭合**：配色区 ✅（§2.2.5）、最近阅读面板 ✅（§2.2.8）、阅读背景图 ✅（§2.2.9）；**分类侧栏的筛选前置已由 A1+A2 打通**（§2.2.6 / §2.2.10），侧栏界面本身待做 |
| T4 | JVM 测试 + 门禁登记 + APK 出包证据 | ✅ **完成**：门禁登记 ✅、APK 出包 ✅（16 项逐符号双端验证）、JVM 测试 ✅（**264 项**全绿，含 11 项布局 + **15 项端口级**） |

### 2.2.1 实施中发现的规格偏差（已修）

§3.6 要求「时间标签**由核心格式化后返回字符串**，Kotlin 不得自己格式化」。
B2 首版 `nr_list_history` **只回 `opened_at`（Unix 秒）**、未回格式化标签 ——
若照此做 T3，Android 就必须自己实现「今天 / 本年 / 跨年」判定，正是规格禁止的语义下沉。

**修法**（与 B3 的派生下沉同一手法）：

- `time_label` / `file_label` 从 `desktop-app/src/history_view.rs` **下沉**到
  `reading-storage/src/history_view.rs`（桌面侧改为 `pub use`）——
  依赖方向是 `ffi → storage`，留在桌面侧 Android 只能复制一份逻辑；
- `nr_history` 加 `label_offset` / `label_len` 两字段（**64 → 72 字节**，仍 8 字节对齐），
  时间标签作为字节池的**第三段**逐行紧随文件名之后；
- 契约测试断言标签已是**成品文本**（长度 ∈ {5, 11, 16} 且含时分数字），
  而非原始时间戳 —— 这是「界面不得自己格式化」的机械守卫；
- 顺带消除重复：引擎层原先自己写了一份「取末段文件名」，现复用共享层 `file_label`。

### 2.2.2 T2 / T4 进展（2026-10-06）

**DTO 小端解析已完成**（`core/NrDtos.kt` 追加，712 行）：

| 类型 | 尺寸 | 说明 |
|---|---|---|
| `NrCategoryEntry` / `NrCategoryBatch` / `NrCategoryCodec` | 48 / 32 | 扁平列表（含 `parentId`/`depth`），树形由 UI 侧按 `parentId` 组装 |
| `NrHistoryEntry` / `NrHistoryBatch` / `NrHistoryCodec` | 72 / 32 | 每行**三段**（书名 · 文件名 · **时间标签**） |
| `NrThemeBase` / `NrThemeContrastResult` | 56 / 24 | 四基色定长槽（全 0 = 未设置）+ 整数比值 |
| `NrBackgroundEntry` / `NrBackgroundBatch` / `NrBackgroundCodec` | 48 / 32 | 相对文件名（R15） |

共同纪律（与既有 `NrChapterCodec` 一致）：

- 布局只用 `NrStructs.buffer(...)` 校验尺寸后按**显式小端偏移**取值，`u32` 一律解成 `Long` 不截断；
- 字节池切片（`NrBatchText`）**越界一律按 `NR_ERR_INTERNAL` 拒绝**，不静默截成乱码；
- `NrHistoryCodec.timeLabel` 直接取核心给的**成品文本** —— 界面不得再格式化（§3.6）。

**JVM 测试**：新增 `NrV11CodecTest`（**11 项**）钉布局与边界，含「标签不得是时间戳数字串」
与「相对文件名不得含分隔符」两条语义守卫。变异测试：把 `labelOffset` 读错偏移、
把历史行尺寸 72→64 **均被判据抓出**（还原后全绿）。

Android 套件由 238 → **249 项**（失败 0）。

### 2.2.3 T2 收口：`NrEngine` 门面与端口级测试（2026-10-06）

`core/NrEngine.kt` 新增 **16 个门面方法**（每批 4 个），与既有 `getChapters` 同一套
「批次解码 + 归还凭据核对」写法：

| 功能 | 门面方法 |
|---|---|
| 分类 | `listCategories` / `createCategory` / `moveBookToCategory` / `deleteCategory` |
| 历史 | `listHistory` / `clearHistoryEntry` / `clearHistoryForBook` |
| 配色 | `getThemeBase` / `setThemeBase` / `setThemeEnabled` / `getThemeToken` / `getThemeContrast` |
| 背景 | `getBackground` / `setBackground` / `setBackgroundScrim` / `clearBackground` |

新增三个包装类 `NrCategoriesRead` / `NrHistoryRead`（含 `rows` 三段文本）/ `NrBackgroundRead`。

**发现并修掉一个既有缺陷**（端口级测试才暴露）：

两个测试替身（`FakeAbi` / `ScriptedReaderAbi`）的 V1.1 读入口**在成功路径返回 `null`**，
而 `null` 在门面语义里表示**失败**（`abi.listX(...) ?: throw`）——
即先前 B1–B4 的替身**只能模拟失败、无法模拟成功**，任何走门面的调用都会抛异常。
因当时只有布局级测试（直接喂字节给 Codec，不经过门面），这一缺陷无人发现。
修法：成功路径返回**字节池 + 写回归还槽**（与 `getChapters` 一致）。

**新增 `NrEngineV11Test`（15 项）** 覆盖：参数透传（去向 / 源路径 / 目标目录）、
失败码映射（读失败必须抛错**而非静默当空**）、归还凭据、销毁后不触碰 native，
以及两条规格守卫：`timeLabel` 原样呈现（不再格式化）、`clearHistoryEntry` 对不存在的
id **如实回报 false**。

**变异测试**（两条均被抓出，还原后全绿）：

1. 分类读的**成功路径返回 `null`**（即上述既往缺陷）→ 红；
2. 历史批次把 `count` / `total` **写反** → 红。

Android 套件 249 → **264 项**（失败 0）。

### 2.2.4 T3 前置勘察：两处架构缺口（**待主人裁定**）

动手写 Compose 界面之前，先核验「界面要读的数据能否从 ABI 取到」，结果发现两处缺口。

**缺口 1：分类 → 书籍 的筛选在 Android 上不可行 —— ✅ 已解决（见 §2.2.10）**

实测证据：

- ABI 35 项导出中**没有任何「列书籍」或「读书籍分类」的入口**——
  按 `list_book` / `get_book` / `book_category` / `category_of` 逐个查均为 0 命中；
  分类相关只有 `nr_list_categories` / `nr_create_category` / `nr_move_book_to_category` / `nr_delete_category`。
- `nr_move_book_to_category` 是**只写**，无对应读回。
- Android 书架书目来自**宿主目录**（`HostBookCatalog`，Kotlin 侧 SAF 持久化），
  **不是**核心；其 `list()` 返回的 `HostBookEntry` **不含 `bookId`**，
  而核心 `books.category_id` 以 **`book_id`** 为键（`ReaderViewModel:717-725` 只在打开时回填）。

⇒ 侧栏可以画树、显示计数、做增删改，但**点击分类无法过滤书目**。

**缺口 2：历史面板「打开该书」不可行（同源）—— ✅ 已解决（见 §2.2.6）**

- `nr_list_history` 回的是 `book_id`；而界面打开书籍需要 `sourceIdentity`（SAF URI）。
- `HostBookCatalog.rememberBookId(sourceIdentity, bookId)` 是**只写**，
  `list()` 不回 `bookId` ⇒ **反向映射也拿不到**。

⇒ 历史面板能列出流水（含核心格式化的时间标签），但点条目**无法回到那本书**。

**候选方向**

| 方向 | 内容 | 代价 | 影响 |
|---|---|---|---|
| **A1**（缺口 2，小） | 宿主目录 `HostBookEntry` 增 `bookId` 字段 + 回读方法 | 很小（Kotlin 单侧） | 历史「打开该书」立即可行 |
| **A2**（缺口 1，中） | ABI 新增 1 项读导出（`nr_list_books` 回 `book_id`+`category_id`，或 `nr_get_book_category(book_id)`） | 12 站点 × 1 项，约 1 轮 | 分类筛选可行，且两端同源 |
| **B** | 先做**无缺口**的两块：设置页配色区 + 阅读页背景 | 约 1 轮 | 不阻塞；分类 / 历史留待裁定 |

**在下的建议**：先做 **B**（把自足的两块交付掉），同时按 **A1 + A2** 补缺口 ——
因为分类与历史是 V1.1 四组里的两组，缺了它们 T3 只完成一半；且 A1 本身是既有宿主缺口，
分类与历史都用得上。若主人希望**一次把 ABI 补到能支撑完整界面**，A2 是唯一路径（不可绕过）。

### 2.2.5 T3 首块：设置页「自定义配色」区（2026-10-06）

**已交付**（无架构缺口的一块）：

| 层 | 内容 |
|---|---|
| 模型 | `ThemeState`（4 基色 + enabled + 对比度 × 100 + `contrastLabel`）+ `ThemeBase`（索引常量与四个显示名） |
| ViewModel | `refreshTheme()`（进设置页即读回）、`onThemeEnabledChange`、`onThemeBaseChange` |
| 界面 | `SettingsScreen` 新增「自定义配色」区：启用开关 + 4 个基色输入框 + 对比度读数 + 未设置提示 |
| 测试标签 | `SETTINGS_THEME_ENABLED` / `SETTINGS_THEME_CONTRAST` / `SETTINGS_THEME_EMPTY_HINT` / `settingsThemeBase(which)` |

**关键口径：界面只显示核心认可的值。**

- 写入后**无论成功与否都回读核心**（`refreshTheme`），于是「被核心拒绝的非法色值」
  不会留在输入框里 —— 界面与核心不会不一致；
- 读失败**如实回显错误码**，不静默显示成「未设置」；
- 对比度由核心给整数 × 100，界面只做显示换算（`454` ⇒ `4.54 : 1`）；
  低于建议值时**保留用户基色**，只加一句「已保留你的选择」（R12）。

**新增 `ThemeSettingsTest`（11 项）**：纯状态 3 项 + 门面接线 8 项，含
「被拒写入后界面仍显示核心的值」「读失败不得静默成空」「低对比度保留基色」
「销毁后进设置页不崩」四条规格守卫。

**变异测试**：把「写入后回读核心」改成**乐观自更新** → 判据变红（还原后全绿）。

Android 套件 264 → **275 项**（失败 0）。

### 2.2.6 A1 已实施：历史 → 打开书籍（缺口 2 闭合）

**根因**：`SafSourceRecord.bookId` **本来就存在**（打开成功时回填），但

- `HostBookCatalog.list()` 返回的 `HostBookEntry` **不回传 `bookId`**；
- 目录合并到书架时把 `bookId` **写死 `0L`**。

两处叠加 ⇒ 历史条目（只带 `book_id`）永远映射不回可打开的来源。

**改动**（均为 Kotlin 单侧，无 ABI 变更）：

| 位置 | 内容 |
|---|---|
| `HostBookEntry` | 增 `bookId: Long? = null`（带默认值，现有构造点不受影响） |
| `HostBookCatalog` | 增 `byBookId(bookId)`，**接口默认实现**遍历 `list()` ⇒ 只实现 `rememberBookId` 的宿主无需改动 |
| `SafHost.list()` | 透传 `record.bookId` |
| `ReaderViewModel` | 目录合并改为 `bookId = entry.bookId ?: 0L`（不再写死 0） |
| `ReaderViewModel` | 新增 `openHistoryBook(bookId)`：反查命中则**复用书架的打开路径**（含平台窗口来源重解析），不另写一套 |

**三条口径**（`HistoryOpenTest` 5 项钉住）：

1. 映射命中 ⇒ 走书架**同一条**打开路径；
2. 映射缺失 ⇒ **如实回报**「该书的来源不在本机」，**绝不试探性打开**（打开错书比明确失败更糟）；
3. 目录未接线 ⇒ 同样如实回报，不退化成静默无操作；
4. `bookId` 为 `null`（本次安装未打开过）**不算**命中。

**变异测试**（两条均被抓出）：① 目录合并改回写死 `0L` → 红；② 映射缺失时改为试探性打开目录第一本 → 红。

### 2.2.7 又一处规格与现实不符：S2 设计稿缺失 —— ✅ 已补（见 §2.2.17）

§2.1 把 `docs/design/novel-reader-v1.1-android-design.md`（S2：Android 侧四组功能的界面设计）
列为已交付项，**但该文件不存在**（`docs/design/` 下仅有桌面两份设计稿）。

影响：T3 的界面工作在**没有 Android 设计稿**的情况下进行。目前的做法是以规格 §3 与
**桌面设计稿**（`novel-reader-v1.1-desktop-design.md` 的 T1–T4 / B1–B4 画框）作语义参照，
按 Android 平台惯例落地。请主人裁定是否补写 S2。

### 2.2.8 T3 第二块：最近阅读面板（2026-10-06）

| 层 | 内容 |
|---|---|
| 路由 | 新增 `ReaderRoute.HISTORY`；`backRouteFromHistory` 复用「有书回阅读、无书回书架」口径 |
| 模型 | `HistoryRow`（含 `timeLabel` / `fileLabel` / `positionLabel`）+ `HistoryState`（含 `total` 与空态文案） |
| ViewModel | `openHistory` / `refreshHistory` / `onClearHistoryEntry` / `onClearHistoryForBook` / `onClearAllHistory` |
| 界面 | `HistoryScreen`：行（书名 · **核心格式化的时间标签** · 文件名 · 位置）+ 逐条「清除」+ 底部「清空全部」（**二次确认**） |
| 入口 | 书架顶栏新增「最近阅读」按钮（规格 R9：从书架其他菜单项进入，不放入分类侧栏） |
| 标签 | 8 项（面板 / 返回 / 空态 / 总数 / 清空 / 行 / 逐条清除） |

**口径**：

- **排序由核心给**（`opened_at DESC`），界面不重排；
- **时间标签由核心格式化**，界面原样显示、不截断（§3.6）；
- 点条目 ⇒ `openHistoryBook(bookId)`（§2.2.6 的映射；缺失时**如实回报**，不试探性打开）；
- 清除只删**流水**，不动书籍 / 进度 / 书签；
- **`onClearAllHistory` 的诚实口径**：ABI 没有「清空全部」入口，故按当前行里出现的书
  逐本调 `nr_clear_history_for_book`，回显报告**核心返回值之和**，而不是界面行数。

**`HistoryPanelTest`（8 项）**：行内容与总数、**保持核心顺序**、空态、读失败不得静默成空、
清除不存在的 id 如实回报、逐本清除与**真实删除总数**、无行时不产生空转原生调用。

**变异测试**（两条均被抓出）：① 界面按 id **重排** → 红；② 清空全部**按界面行数谎报** → 红。

> ⚠️ 第 ② 条**初版没抓住**，反而暴露了测试自身的弱点：替身让核心每本回 2 条（合计 4），
> 而界面恰好也是 4 行 —— **两个数相等，谎报的实现照样通过**。改成「核心 3+3=6 vs 界面 4 行」
> 这种**可区分**的组合后才抓住。教训：跨边界断言的两个数必须**可证不等**。

Android 套件 280 → **288 项**（失败 0）。

### 2.2.9 T3 第三块：阅读背景图（2026-10-06）

| 层 | 内容 |
|---|---|
| 模型 | `BackgroundState`（相对文件名 / 大小 / 遮罩 / `scrimAlpha` / `fitLabel` / `scrimLabel`）+ `BackgroundFit`（三态与显示名） |
| ViewModel | `refreshBackground`（进设置页读回）、`onBackgroundPickRequest`、`applyPickedBackground`、`onBackgroundScrimChange`、`onBackgroundClear`、`reportBackgroundFailure` |
| 宿主 | `SafHost.materializeBackgroundSourceAsync`：把选中的 SAF URI **物化**成本地文件（复用既有流式复制 + 原子转正），目标是**独立**的 `background-src` 缓存目录；`MainActivity` 新增图片选择器（`image` 类型） |
| 界面 | 设置页「阅读背景」区：信息行 / 空态提示 / 缩放口径三选 / 遮罩滑杆 / 「选择图片…」/「移除背景」 |
| 目录 | 背景落盘目录**由宿主决定并传入**（`filesDir/backgrounds`）—— 核心不猜平台数据目录 |

**关键判断：不在 ViewModel / 核心触碰 `content://`。** 选中的 URI 先由**宿主**物化成本地路径，
再交给 `nr_set_background`；核心随后**复制**到数据目录并只存相对名（R15）。

**缺口 3（新发现，非阻塞）**：核心**没有**「只改 `fit_mode`」的导出
（存储层有 `background::set_fit_mode`，但未在 ABI 暴露）。故缩放口径只能在
`nr_set_background` 那一次调用里确定 —— 界面据此把它做成「**选图时生效**」的三选，
并在文案里写明；**不给一个按了不改存储的开关**（那是不诚实的控件）。

**`BackgroundSettingsTest`（12 项）**：读回（相对名 / alpha / 显示换算）、空态、读失败不静默、
选图透传（路径 + 目录 + **fit/scrim**）、被拒时原值不变、调遮罩不换图、移除透传目录、
**缺目录 / 缺选图入口时如实回报且不触碰 native**、宿主失败可见且清除暂存。

**变异测试（3/3 抓住）**：① 口径不随图传（改用核心默认值）→ 红；
② 缺目录时**静默无操作** → 红（**初版没抓住**，因测试只覆盖「选图」未覆盖「移除」；补测后抓住）。

> ⚠️ 本轮另踩一个**编译期**陷阱：在 KDoc 里写 `OpenDocument("image/*")` ——
> 其中的 `/*` 会**吞掉后面的注释**（Kotlin 块注释不嵌套），报错是远处一句
> 「Unclosed comment」。同一坑在本轮踩了**两次**（一个在参数列表 KDoc、一个在方法 KDoc）。
> 规避：注释里不要出现 `/*` 字面量。

Android 套件 288 → **300 项**（失败 0）。

### 2.2.10 A2 已实施：书籍分类映射（缺口 1 闭合）

**问题**：界面要把书架条目按分类过滤，但 35 项导出里既没有「列书籍」也没有「读书籍分类」；
而核心 `books.category_id` 以 **`book_id`** 为键，Android 书架的键是 `sourceIdentity`。

**解法**（新增 1 项导出，**不改既有 35 项**）：

| 层 | 内容 |
|---|---|
| ABI | **`nr_list_book_categories(engine, out_batch)`** → 只回**在架**书籍的 `(book_id, category_id)`；`category_id = -1` = 未分类（None 哨兵口径） |
| 结构 | `nr_book_category`（16 字节，2 × i64）+ `nr_book_category_batch`（32 字节）；行内**无字符串**，字节池就是行本身 |
| 存储 | `books::list_book_categories` —— 单列一个查询，**不给 `BookRecord` 加字段**（否则所有既有 SELECT 与构造点都要改） |
| Kotlin | DTO（含 `UNCATEGORIZED = -1` 常量）+ `NrAbi`/`NrCore`/门面 `listBookCategories()` |
| 契约计数 | **35 → 36**；结构数 25 → 27 |

**筛选链路至此闭合**（两端相合，不把书目搬进核心）：

```
书架条目.sourceIdentity ──(A1：宿主目录 bookId)──▶ bookId
bookId ──(A2：nr_list_book_categories)──▶ categoryId
```

**测试**：Rust 契约 2 项（未分类 = -1；移动后逐本反映，含「两本都要出现」）+
JVM 4 项（尺寸 / 解码与哨兵 / 越界拒绝 / 门面映射与失败传播）。

Android 套件 300 → **304 项**（失败 0）；**八项门禁全部 PASS**。

### 2.2.11 本轮在下踩的坑（如实记录）

1. **`rindex('\n}')` 又一次命中文件最后一个 `}`** —— 新方法被插进 `FakeReleaser` 而非 `FakeAbi`。
   这与更早那次「误删 FakeReleaser」是**同一根因**：以「最后一个 `}`」作锚点在有多个类的文件里必然失手。
   修法：显式锚定**方法前一行**或类内的唯一签名。
2. **臆造 API** —— 契约测试里写了 `LongArrayLike` 与 `NrCategoryEntry::PARENT_NONE`，
   前者是 Kotlin 侧概念、后者是 Kotlin 的类名；Rust 侧应写 `[0i64; 1]` 与 `-1i64`。
   （存在性自检也曾被自己的插入文本骗过 —— 检查必须针对**插入前**的状态。）
3. **注释里的 `/*`** —— 见 §2.2.9。

### 2.2.12 桌面侧「分类筛选」补完（C11 闭合，2026-10-06）

**发现**：准备做 Android 侧栏时，在下先核对「桌面是怎么筛的」，结果发现**桌面从未筛过**：

| 证据 | 内容 |
|---|---|
`crates/desktop-app/src/shelf.rs` | `rows` 由 `records.iter()`（**全量**）直接映射；函数内唯一的 `filter` 用于「待重新关联」与移出书架，**与分类无关** |
`filtered_count` | = `rows.len()` —— 与全量**恒等**，名不符实 |
全 crate 检索 | 唯一「按分类」的查询在 `category_tree.rs:227/240`，用途是**侧栏计数**，不筛书目 |
`checklist.md` | **C11 本就是未勾选项** —— 规格自己的清单已承认该项未完成 |

⇒ 在下先前「P1 桌面分类树已完成」的说法**过于宽松**：分类树/维护做完了，但 **C11 的「只显示该分类书籍」一直没做**。
这也意味着缺口 1（Android 无筛选入口）的另一半真相是：**两端都没有筛选**。

**修法**（桌面侧，闭合 C11）：

- `shelf::load_shelf_in` 增加作用域过滤：`None` ⇒ 不过滤（根「全部书籍」）；
  `Some(-1)` ⇒ 仅未分类；`Some(id)` ⇒ 该分类 **含子孙**（与侧栏「合计计数」同一口径）；
- 过滤在**核心侧**完成（界面只渲染结果），故桌面 / Android 口径天然一致；
- `filtered_count` 由此变为**真实过滤后**的行数。

**测试**（`shelf::tests`，4 项）：根不过滤 4 本；选「科幻」= 2 本（**含子分类**）；选子分类 = 1 本；
选未分类 = 1 本且**不与全量相等**；选中不存在的分类 = **空**（不退化成「显示全部」）。

**变异测试**：把作用域匹配改成常量 `None`（忽略选择）⇒ **3 项变红**，还原全绿。

桌面库 170 → **174 项**；工作区 **297 项**；八项门禁全部 PASS。

### 2.2.13 A3：作用域书籍导出（子树展开留在核心）

**动因**：Android 侧栏要在选中分类时过滤书目。核心已给「分类树（含 parentId）」与
「bookId → categoryId」，但**子树展开**若放到 Kotlin，就是把核心语义复制到界面
（与「Rust 是唯一语义来源」相悖，且两端口径可能各自演化）。

**做法**：新增 1 项导出，把展开留在核心。

| 层 | 内容 |
|---|---|
| ABI | **`nr_list_books_in_scope(engine, scope_kind, category_id, out_batch)`**；`scope_kind`：`0` 全部 / `1` 未分类 / `2` 分类（**含子孙**） |
| 结构 | `nr_id_list`（32 字节 = u32 × 2 + `nr_buffer`；字节池为小端 i64 数组） |
| 存储 | `categories::books_in_scope` + `SCOPE_*` 常量；复用既有 `descendant_ids` |
| Kotlin | `NrIdList` + `NrIdListCodec` + `NrScopeKind` + 门面 `listBooksInScope()` |
| 契约计数 | **36 → 37**；结构数 27 → 28 |

**测试**：Rust 契约 3 项（含子孙展开方向、未分类、**未知作用域必须报错**）+
JVM 4 项（参数透传、全部作用域、失败不得静默成空、字节数与 count 必须恰好自洽）。

**踩坑（重要）**：`descendant_ids` **已包含自身**（BFS 起点即 `id`）——
在下按直觉又 `push(category_id)`，导致该分类被查两次、书籍重复（`[1,1,2,3,3]`）。
桌面侧有同一处多余 `push`，因收进 `HashSet` 而去重、侥幸正确；两处均已加警示注释。

Android 套件 304 → **308 项**；`.so` 重建为 **37 项** JNI 导出；**八项门禁全部 PASS**。

### 2.2.14 T3 收口：分类侧栏 + A4（改名 / 移动导出）

**缺口 4（新发现）**：`nr_rename_category` 与 `nr_move_category` **都不存在**
（B1「分类 4 项」只有 list / create / move_book / delete），而规格 R2（改名）与 R4（移动分类）
都要求它们。故补 **A4**（2 项导出）：

| 项 | 签名 | 语义 |
|---|---|---|
| `nr_rename_category` | `(engine, category_id, name, name_len)` | 空名 / 坏 UTF-8 拒绝；重名由核心拒绝并保留原值 |
| `nr_move_category` | `(engine, category_id, new_parent_id)` | `-1` = 根层；**移入自身或子孙必须拒绝**（R5，防环） |

契约计数 **37 → 39**；无新结构。Rust 契约测试新增（含**未知作用域必须报错**那条之外，
本轮另加改名/移动的链接与计数断言）。

**侧栏界面（Android）**：

| 层 | 内容 |
|---|---|
| 装配 | `CategoryTreeAssembly` —— **纯函数**：扁平表 → 深度优先序；**祖先折叠 ⇒ 子孙不下发**；孤儿视为根层（脏数据不吞树） |
| 模型 | `CategoryRowView`（`depth`/计数全来自核心）+ `CategoryState`（含 `collapsed` 纯视图状态） |
| 界面 | `CategorySidebar`：全部书籍 / 未分类 / 分类树（按 `depth` 缩进、有子才给箭头）+ 新建 / 重命名对话框 |
| 过滤 | `visibleBookIds`：由核心 `nr_list_books_in_scope` 给出；书架按集合成员判断过滤 |

**分层说明（与桌面的差异）**：桌面把折叠集合传给核心投影（`category_tree::project`），
Android 在客户端装配。**分层不同、语义相同** —— 折叠规则由单测钉住（`CategorySidebarTest`），
任何一侧改动都要同步。核心仍是层级/计数/归属的唯一来源。

**本轮测试抓出的两个真实缺陷**（都不是测试自身问题）：

1. **`onRenameCategory` 被拒分支漏了重新读回** —— 与 `onMoveCategory` 不一致；
   被拒后界面可能停在核心并未接受的树上。已补 `refreshCategories()`。
2. **作用域过滤被写在 `if (added)` 内** —— 书架已填充后再切换分类时目录无新条目，
   `added` 为 false ⇒ **选中分类却不筛选**。已改为无条件执行。

**`CategorySidebarTest`（17 项）**：纯装配 4 项（顺序/层级/折叠含中间层/孤儿/选中唯一）+
门面 9 项（作用域透传、全部不过滤、读失败不得静默、折叠不碰语义、改名/移动透传与被拒回读、
空名本地拒绝）+ 书架过滤 4 项（只显示核心给出的书、无 bookId 不猜、全部恢复）。

**变异测试（2/2 抓住）**：① 过滤退回 `if (added)` 内 → 红；② 折叠不再隐藏子孙 → 红。

Android 套件 308 → **325 项**；`.so` 两侧重建为 **39 项**；**八项门禁全部 PASS**；工作区 299 项。

### 2.2.15 已知的偶发测试（并行执行）

`reading-ffi` 的 `cancel::tests::clear_engine_removes_only_that_engine` **偶发失败**
（断言进程级引擎注册表计数；并行运行的其他用例会创建/销毁引擎而扰动它）。

- **判定**：单跑通过、`--test-threads=1` 通过、连跑 4 次全量无失败 ⇒ 属**用例自身耦合**，
  非被测代码缺陷；
- **纪律**：不得把「重跑到绿」当作通过 —— 报告里必须说明这次全绿是在一次偶发红之后取得的；
- **待办**：该用例应改为**自持有其断言的状态**（或单独串行化），本规格不改测试实现。

### 2.2.16 A5 + 分类移动 / 删除界面（R3/R4/R5 收口）

**A5（1 项导出）**：`nr_category_move_targets(engine, source_id, out_batch)`

- 返回「未分类」（`id = -1`）+ **全部分类**，并把 `source_id` **自身与全部子孙标
  `selectable = 0`** —— **环判定在核心完成**，界面不遍历父子链；
- 刻意**不隐藏**不可选项而只标记：让用户看见「为什么不能选」，比整条消失更好解释
  （与桌面 `category_ops::move_targets` 同一口径）；
- 结构 `nr_category_choice`（40 字节）+ `nr_category_choice_batch`（32 字节）；
- **逻辑落在共享存储层** `categories::move_targets`（桌面目前在应用层实现，日后可复用此函数）；
- 契约计数 **39 → 40**；结构数 28 → 30。

**界面**：分类行的「⋯」菜单 → 重命名 / 移动到… / 删除；
目标选择与删除去向**共用一个对话框**（语义不同）：

- 移动：从核心给的目标里选（不可选项**置灰仍显示**）；
- 删除：先选去向（移入另一分类 / 一并移出书架 / 移回根下未分类），
  **未选去向时确认按钮不发请求** —— 且界面这一层就不该发（硬约束）。

**`CategorySidebarTest` 17 → 23 项**（新增 6 项）：目标来自核心且不可选项仍可见、
未选去向**本地拒绝且不触碰核心**、去向与目标如实透传并回报移走本数、成功后清空目标、
被拒后回读真实树、清空目标。

**变异测试**：把「未选去向」的本地拦截去掉（放行执行）⇒ 判据变红，还原全绿。

Android 套件 325 → **331 项**；`.so` 两侧重建为 **40 项**；**八项门禁全部 PASS**。

### 2.2.17 桌面侧复用共享层 + 修掉一处**真实潜伏缺陷** + 补 S2

**① 消除重复实现**：桌面 `category_ops::{move_targets, move_target_ids}` 原本各自
`project_tree_with_uncategorized` + `descendant_ids` 算一遍。改为**共用**
`categories::move_targets`（A5 落在共享层的函数）⇒ 桌面与 Android **同源**。

**② 修掉一个真实潜伏缺陷（错位）**：

- 界面（`ui.rs` 移动态）用**名称列表的 0 基下标**去查 id 列表 ⇒ 两列表**必须逐下标对齐**；
- 旧 `move_target_ids` 把被阻断项**剔除**，而名称列表**包含**它们（只标不可选）
  ⇒ **错位**：夹具下「历史」在下标 3，旧 id 列表只有 2 项 ⇒ `rows.get(3)` 取不到
  ⇒ **根本选不中**（且当被阻断行在前时，会写到**错误**的分类 id 上）。
- 修法：id 列表同样包含全部行（未分类 `None`，其余给 id），**长度与顺序由结构保证**。
- **回归验证**：把旧实现（剔除）临时还原 ⇒ 新断言变红，证明缺陷真实存在。

**③ 门禁判据同步（两次自身失误）**：

- 判据 [10] 原查「`category_ops` 里有没有 `descendant_ids`」—— 逻辑下沉后**误报**。
  改为检查三件事：存储层确按子孙集标记；桌面**消费**共享层；桌面**不得**再出现
  `descendant_ids`（出现即又抄一份）。
- 第一次改完仍**空转**：判据命中了在下写在**注释**里的 `categories::move_targets`。
  加 `_strip_rust_comments` 后有效（三条变异：重新遍历 → 红；两处都不复用 → 红；还原 → 绿）。

**④ 补 S2 设计稿**：`docs/design/novel-reader-v1.1-android-design.md` 已补写
（如实标注为**实施后补写**，非驱动设计的原始稿），含尺寸常量、组件与测试标签、
布局骨架、逐屏规格（A-C1…A-B1）、与桌面稿的对应表、以及**已知分层差异**说明。

桌面库 174 项、工作区 **301 项**、Android **331 项**；八项门禁全部 PASS。

### 2.3 明确不做

- **不改**既有 19 个导出的名称、语义、顺序；
- **不升** `NR_ABI_VERSION`；
- **不做** Android 侧的全文搜索 / 连续滚动 / ZIP / EPUB（V1 之外的规划项）；
- **不删改**用户原始 TXT（Android 侧 SAF 本就只读）。

---

## 3. FFI 导出清单（§3 是本规格的核心）

设计原则（对齐桌面侧语义）：

- **批量返回**：一次导出换回整段数据（JSON 或定长数组），不搞逐条查询 ——
  技能记载「Android 每次 JNI 往返都是成本」。
- **语义判定在 Rust**：派生、对比度、循环防护、计数都在核心侧；Kotlin 只渲染。
- **字符串 UTF-8**：内容窗口按内容编码，ABI 字符串一律 UTF-8（技能硬约束）。
- **不返回绝对路径**：与桌面侧一致（分类、背景都只回安全相对名）。

### 3.1 分类树（4 项）

| 导出 | 方向 | 说明 |
|---|---|---|
| `nr_list_categories` | 读 | 返回全部分类（含 `parent_id` / `depth` / 直属计数 / 合计计数 / `sort_order`）。形状：**定长数组**，一次取回。 |
| `nr_create_category` | 写 | `(parent_id, name_utf8, len)` → `out_category_id`。同父同名由核心拒绝（同桌面）。 |
| `nr_move_book_to_category` | 写 | `(book_id, category_id_or_neg1)`。分类树移动同理复用 `nr_move_category`。 |
| `nr_delete_category` | 写 | `(category_id, destination, target_id)`。**`destination` 必选** —— 未选（`0`）必须返回 `NR_ERR_INVALID_ARGUMENT` 且**零副作用**（与桌面 `DeleteChoice::None` 同语义）。 |

> 循环防护（自身 + 全部子孙不可选）在核心 `categories.rs` 已有；导出点只做调用，
> **不在 Kotlin 侧重算**（技能：判定必须可测且单一来源）。

### 3.2 阅读历史（3 项）

| 导出 | 方向 | 说明 |
|---|---|---|
| `nr_list_history` | 读 | `(book_id_or_0, limit, grouped)` → 流水或按书聚合。**时间倒序由核心排**（技能：排序不得下沉）。 |
| `nr_clear_history_entry` | 写 | `(entry_id)` |
| `nr_clear_history_for_book` | 写 | `(book_id)` → `out_removed_count`。 |

> **打开书籍写流水不需要新导出**：Android 的 `open_txt` 已在 ABI 内，
> 核心侧在打开时写流水（与桌面同一路径），故「每次打开一条」自动成立。

### 3.3 自定义配色（3 项）

| 导出 | 方向 | 说明 |
|---|---|---|
| `nr_get_theme_base` | 读 | → 四个基色（`#RRGGBB`，未设置回空串）+ `enabled`。 |
| `nr_set_theme_base` | 写 | `(which, hex_utf8, len)`，`which` ∈ `0..4`（fg/bg/primary/accent）。**只存基色**（40 令牌不落库）。 |
| `nr_set_theme_enabled` | 写 | `(enabled)`。关闭时**保留基色**。 |

> **派生与对比度不需要新导出**：核心侧派生，Android 只取令牌值。
> 故需 `nr_get_theme_token`（`which_token, dark`）—— 见 §3.5。

### 3.4 全局背景图（4 项）

| 导出 | 方向 | 说明 |
|---|---|---|
| `nr_set_background` | 写 | `(src_path_utf8, len, fit_mode, scrim_permille)`。核心**复制**到应用数据目录并生成安全文件名（原图只读）。 |
| `nr_get_background` | 读 | → 相对文件名 / 大小 / `fit_mode` / `scrim_permille`。**不回绝对路径**（Android 侧由 app-private 目录拼接）。 |
| `nr_set_background_scrim` | 写 | `(permille)` |
| `nr_clear_background` | 写 | → 回清除的文件名（Android 侧据 app-private 目录删除；**只删核心生成的文件**）。 |

### 3.5 令牌读取（1 项，配合 §3.3）

| 导出 | 方向 | 说明 |
|---|---|---|
| `nr_get_theme_token` | 读 | `(token_index, dark)` → `out_rgba`。**对比度**另由 `nr_get_theme_contrast` 返回（`out_ratio_permille` + `out_passes`）。 |

合计 **15 项新导出**（4 + 3 + 4 + 2 + 2），加上既有 19 项 = **34 项**。

> **实际实现为 16 项 / 共 35 项**（2026-10-06）：§3.5 的「令牌读取」在实现时拆成两个导出 ——
> `nr_get_theme_token`（取令牌 RGBA）+ `nr_get_theme_contrast`（取对比度读数与是否达标）。
> 拆开比「一个导出带两个 out 参数」更清晰，Kotlin 侧也不必解复用载荷。
> `ABI_ENTRY_COUNT` 的**权威值现为 35**。

### 3.6 与桌面侧的语义对等（必须逐条成立）

| 语义 | 桌面 | Android（必须一致） |
|---|---|---|
同父同名 | `categories::create_category` 拒绝 | 同一核心函数 |
循环移动 | `move_category` 拒绝自身+子孙 | 同一核心函数 |
删除未选去向 | `DeleteChoice::None` 零副作用 | `destination=0` → `NR_ERR_INVALID_ARGUMENT`，零副作用 |
历史排序 | `ORDER BY opened_at DESC` | 同一 SQL |
时间标签 | `history_view::time_label`（今天/本年/跨年） | **由核心格式化后返回字符串**（Kotlin 不得自己格式化） |
40 令牌不落库 | `reader_theme_custom` 只 4 基色 | 同一张表 |
对比度不足保留基色 | `ContrastReport.warning` | 同一函数 |
背景只删生成的文件 | `background_ops::clear` | 同一函数 + Android 侧拼 app-private 路径 |

---

## 4. 工作量勘察（12 处站点，§4 是本轮调研结论）

技能记载「计数活在 10+ 处独立位置」。本轮实测：

| # | 站点 | 实测 | 新增 15 项需要的改动 |
|---|---|---|---|
| 1 | `include/novel_reader.h` | 439 行 | 15 个 `typedef` + 15 个编号声明；改「18+1=19」注释 |
| 2 | `structs.rs` | 690 行 | 新结构的 `#[repr(C)]` + `ABI_STRUCTS` + `struct_sizes()` + **计数断言** |
| 3 | `exports.rs` | 834 行 | 15 个 `#[no_mangle]`；`entry_names()` **数组长度**；文档注释里的计数 |
| 4 | `engine.rs` | — | 15 个引擎方法 + outcome 结构 |
| 5 | `abi.rs` | 509 行 | `ABI_ENTRY_NAMES`（长度 + 15 项）+ `ABI_FUNCTIONS`（长度 + 15 个签名元组） |
| 6 | `lib.rs` | 71 行 | `ABI_ENTRY_COUNT`（**权威**）+ 其断言 |
| 7 | `android/jni/nr_jni.c` | 808 行 | 15 个 `NR_JNI(...)` 包装；**每个 `_` 要转义 `_1`**（否则运行时 `UnsatisfiedLinkError`） |
| 8 | `scripts/build-android-libs.sh` | 225 行 | `-lt N` 断言 + 打印的契约计数 |
| 9 | `scripts/verify-abi.py` | 659 行 | 入口名清单 |
| 10 | `scripts/verify-android-boundary.py` | 1223 行 | 入口名清单（**比对顺序序列**） |
| 11 | Kotlin `core/` | — | `NrCore` external **按 header 顺序**、`NrAbi` 接口 + 生产实现、`NrEngine` 门面、DTO（含显式小端偏移） |
| 12 | 每个 `NrAbi` 测试替身 | — | 接口变更会让编译失败，需逐个补实现 |

**实测确认的计数点**（技能警告「硬编码长度可能藏在函数返回类型里」）：

- `crates/reading-ffi/src/lib.rs:43` `pub const ABI_ENTRY_COUNT: usize = 19;`
- `lib.rs:58` `assert_eq!(ABI_ENTRY_COUNT, 19, "18 项冻结 + nr_set_encoding");`
- `include/novel_reader.h:360` 注释「冻结 18 项 + nr_set_encoding，共 19 项」
- `lib.rs:3` / `lib.rs:18` 文档注释各一处

**估算**：15 项 × 12 站点，按技能「加一个入口要几轮」的经验，约 **6–9 轮**；
若减到「只做分类 + 历史」（7 项导出），约 **3–5 轮**。

---

## 5. 实施分期（若主人决定继续）

| 批次 | 内容 | 轮次估 |
|---|---|---|
B1 | 分类树 4 项（端到端：FFI → Kotlin → 界面 → 测试 → 门禁） | 2–3 |
| B2 | 阅读历史 3 项（+「打开写流水」零改动验证） | 1–2 |
| B3 | 自定义配色 5 项（含派生/对比度在核心侧调用） | 2–3 |
| B4 | 背景图 4 项 | 1–2 |

每批结束须交付：Rust 契约测试（走真实 `extern "C"`）+ JVM 测试（端口调用）+ 门禁登记 + 变异证据。

---

## 6. 验收条件（写成可执行的命令）

- [x] `python3 scripts/verify-android-v11-parity.py` → PASS（12 站点齐备 + 计数一致）
- [x] 变异：`python3 scripts/verify-android-v11-parity.py` 在「删某站点新增」样本上 FAIL
      （含「派生单一真相」三条判据的变异对照）
- [x] 每批实现后：`python3 scripts/verify-abi.py --allow-missing-kotlin` PASS
- [x] 每批实现后：`python3 scripts/verify-android-boundary.py` PASS（顺序 + JNI 转义）
- [x] 每批实现后：`cd android && gradle --offline :app:testDebugUnitTest` PASS（238 项）
- [x] 出包：`bash scripts/build-android-libs.sh --abi arm64-v8a` 打印「JNI 导出: **35** 项」
- [x] 符号扫描：16 项 V1.1 导出在 `.so`（转义名）与 `classes*.dex`（Kotlin 原名）
      双端 **16/16** 命中

> **口径修正（2026-10-06）**：原第 7 条写「DEX 含**JNI 符号**」是错的 ——
> 转义名（`nr_1list_1categories`）只存在于 `.so` 的符号表；`classes*.dex` 里是
> Kotlin `external fun` 的**原名**（`nr_list_categories`）。两端必须分别扫；
> 扫错层会得出「导出没进包」的假结论（本项实测踩中过一次）。

---

## 7. 风险与已知限制

| 风险 | 缓解 |
|---|---|
JNI 名字转义漏掉（`_` → `_1`） | 构建脚本已 grep 该错误；每批跑 `build-android-libs.sh` |
计数点遗漏（十+处） | 门禁 §3.6 + 技能要求「先 grep 邻近导出名与字面计数」 |
Kotlin 侧自算语义（派生/对比度/时间格式） | 门禁扫 Kotlin 不得出现派生与格式化逻辑 |
两套数据 | 本规格路线 A 明确禁止 Kotlin 侧另存分类/历史 |
出包时 `local.properties` 被整写导致 Release 静默变未签名 | 只改 `sdk.dir` 单键（技能硬约束） |

**已知限制**（如实记录，不假装覆盖）：

- `FitMode::Tile`（平铺）在两端都退化为「原始尺寸居中」—— Slint 不支持 repeat 平铺，
  Compose 侧同样没有内置 repeat。本期**不实现平铺**。
- 时间标签按本地时区格式化（SQLite `localtime`），与桌面一致。

---

## 8. 开放问题

| # | 问题 | 待主人裁定 |
|---|---|---|
Q1 | 是否继续 A 的实施（B1–B4，约 6–9 轮），还是先只做分类 + 历史（3–5 轮）？ | ✔ |
| Q2 | 派生配色在 Android 侧是否需要「8 色带预览」？还是只要令牌值就够（省 1 项导出）？ | ✔ |
| Q3 | 背景图在 Android 侧是否允许「从相册选」（需要 `READ_MEDIA_IMAGES` 权限）？还是仅「从应用私有目录选」？ | ✔ |

---

## 9. 可追溯性

| 规格条目 | 任务 | 检查项 |
|---|---|---|
R1（§3.1 分类） | T1-B1 | C-P5-1 |
R2（§3.2 历史） | T1-B2 | C-P5-2 |
R3（§3.3+§3.5 配色） | T1-B3 | C-P5-3 |
R4（§3.4 背景） | T1-B4 | C-P5-4 |
R5（§3.6 语义对等） | T1 全部 | C-P5-5 |
R6（§4 站点勘察） | S3 门禁 | C-P5-6 |
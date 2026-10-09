# novel-reader Android 端 · V1.1 新功能设计规格（书架分类 / 阅读历史 / 自定义配色 / 背景图）

> **文档性质（请先读）**：本文档是**实施后补写**的 —— 原规格 §2.1 把本文件列为「已批准、待实施」
> 的交付项，但实际缺失（见 `p5-android.md` §2.2.7）。因此本文**不是**驱动设计的原始稿，
> 而是对**已实现**界面的事后记录与口径固化。
>
> 它仍然有价值：(a) 补上规格要求的交付项；(b) 作为后续 Android UI 改动的参照；
> (c) 把「为什么这样实现」的口径写下来，避免下次改动时重新推演。
> **语义参照**来自 `novel-reader-v1.1-desktop-design.md`（桌面稿 T1–T4 / B1–B4 / C1–C7 / H1–H3）
> 与 `p5-android.md` §3；平台惯例按 Android/Compose 落地。
>
> **2026-10-09 改版**：书架分类导航改为「文件管理器式」（窄屏下钻 / 宽屏保留左右布局），
> 过滤口径由「含子孙」改为**直属**；书籍「移动到…」入口补齐。相关章节（§0/§1/§2/§3/§4 A-C1·A-C6·A-C7/§5/§6）已同步。

---

## 0. 本次口径

1. **核心是唯一语义来源**：层级（`depth`）、直接/合计计数、分类归属（`nr_list_book_categories`）、
   环判定、配色派生与对比度、时间标签格式化**全部由 Rust 计算**；Kotlin 只解析 ABI 载荷并呈现。
2. **界面只显示核心认可的值**：写入后一律**回读**；被核心拒绝时不保留乐观状态。
3. **不给无反应的控件**：缺能力（宿主未提供目录 / 选图入口）时**如实回报**，不静默无操作。
4. **未选去向不得执行删除**：界面这一层就不发请求（硬约束，核心也会拒绝）。

---

## 1. 设计原则（Android 侧）

| 原则 | 落地 |
|---|---|
| 层级由核心给，界面只缩进 | `CategoryRowView.depth` → `indentLevel`；界面不计算层级 |
| 直属口径：每本书只出现一次 | 可见集合由核心映射逐本判定；界面不遍历父子链、不派生语义 |
| 不可选**不隐藏、只置灰** | 移动/删除目标里「自身与子孙」仍显示，文案写明「不可选：自身或子孙」 |
| 折叠是纯视图状态 | `CategoryState.collapsed`；只影响下发哪些行，不触碰核心 |
| 文本不二次加工 | 历史时间标签原样显示（核心已格式化）；界面不截断、不重排 |
| 只回相对名 | 背景图只显示核心给的相对文件名；绝对路径由界面用 app-private 目录拼 |

---

## 2. 令牌与组件

### 2.1 尺寸常量

| 常量 | 值 | 用途 |
|---|---|---|
| `SIDEBAR_WIDTH` | `190.dp` | 分类侧栏宽度（**仅宽屏**） |
| `ShelfLayout.COMPACT_MAX_WIDTH_DP` | `600` | 自适应断点：< 600dp = 窄屏（下钻列表） |
| 侧栏行内缩进 | `8 + depth * 12` dp | 层级缩进 |
| 行内边距 | 竖直 `8–10.dp` | 侧栏行 |

### 2.2 新组件

| 组件 | 文件 | 说明 |
|---|---|---|
| `CategorySidebar` | `ui/CategorySidebar.kt` | 侧栏根（**宽屏**）：全部书籍 / 未分类 / 分类树 / 新建 |
| `SidebarFixedRow` | 同上 | 「全部书籍」「未分类」两行（入口，非真实分类） |
| `CategoryTreeRow` | 同上 | 分类行：箭头（**有子才给**）+ 名称 + 计数 + 「⋯」菜单 |
| `CategoryDrill` | `ui/CategoryDrill.kt` | 窄屏下钻列表：返回行 + 目录行 + 书籍行 +「新建分类」（当前目录） |
| `CategoryOpsDialogs` | `ui/CategoryOpsDialogs.kt` | 分类操作对话框共享宿主（两布局**共用一套**：新建子分类/重命名/移动/删除） |
| `ShelfRow`（含菜单） | `ui/ShelfScreen.kt` | 书籍行：「⋯」→ 移动到…（`BOOK_MOVE_DIALOG`）/ 移除（带确认） |
| `HistoryScreen` | `ui/HistoryScreen.kt` | 最近阅读面板 |
| 设置页配色区 | `ui/SettingsScreen.kt` | 启用开关 + 4 基色输入 + 对比度读数 |
| 设置页背景区 | `ui/SettingsScreen.kt` | 信息行 + 缩放口径三选 + 遮罩滑杆 + 选择/移除 |

### 2.3 测试标签（`ui/ReaderTestTags.kt`）

侧栏（宽屏）：`SIDEBAR` / `SIDEBAR_ALL` / `SIDEBAR_UNCATEGORIZED` / `sidebarCategory(id)` /
`sidebarToggle(id)` / `sidebarMenu(id)` / `SIDEBAR_MENU_{CREATE,RENAME,MOVE,DELETE}` /
`SIDEBAR_TARGET_DIALOG` / `sidebarTarget(id)` / `sidebarDeleteDestination(kind)` /
`SIDEBAR_CONFIRM` / `SIDEBAR_CREATE` / `SIDEBAR_NAME_{FIELD,CONFIRM}`

下钻（窄屏）：`DRILL` / `DRILL_BACK` / `drillDir(id)` / `drillDirMenu(id)` / `DRILL_CREATE` /
`DRILL_EMPTY_HINT`

书籍操作（两布局共用）：`shelfBookMenu(sourceIdentity)` / `BOOK_MENU_MOVE` /
`BOOK_MOVE_DIALOG` / `bookMoveTarget(id)` / `BOOK_MOVE_CONFIRM`

历史：`HISTORY_SCREEN` / `SHELF_OPEN_HISTORY_BUTTON` / `HISTORY_BACK_BUTTON` /
`HISTORY_EMPTY_STATE` / `HISTORY_TOTAL` / `HISTORY_CLEAR_ALL_BUTTON` /
`historyRow(id)` / `historyClearEntry(id)`

配色：`SETTINGS_THEME_ENABLED` / `SETTINGS_THEME_CONTRAST` / `SETTINGS_THEME_EMPTY_HINT` /
`settingsThemeBase(which)`

背景：`SETTINGS_BACKGROUND_INFO` / `SETTINGS_BACKGROUND_EMPTY_HINT` /
`SETTINGS_BACKGROUND_SCRIM` / `settingsBackgroundFit(mode)` /
`SETTINGS_BACKGROUND_PICK` / `SETTINGS_BACKGROUND_CLEAR`

---

## 3. 布局骨架

### 3.1 书架页（三页共用顶栏）

```
Column(fillMaxSize)
├─ NrTopBar(书架 / 继续阅读 · 最近阅读 · 设置)
├─ HorizontalDivider
├─ BoxWithConstraints(weight 1f)        ← 自适应：ShelfLayout.isCompact（< 600dp = 窄屏）
│   ├─ 窄屏：CategoryDrill              ← 2026-10-09：单列下钻（目录在上、书籍在下）
│   │        ├─ ← 返回行（含面包屑；系统返回键同义）
│   │        ├─ LazyColumn[目录行…, 书籍行…]
│   │        └─「新建分类」（建在当前目录）
│   └─ 宽屏：Row[CategorySidebar(190dp) + Column(loading / empty / 列表)]
├─ HorizontalDivider
├─ 状态文本
└─ OutlinedButton(导入 TXT)
```

### 3.2 侧栏纵向预算（宽屏；窄屏不渲染侧栏）

```
全部书籍       ← 恒在，选中态高亮
未分类         ← 恒在（-1 哨兵），选中态高亮
──────────
分类树（LazyColumn, weight 1f）   ← 空时显示「还没有分类」
──────────
新建分类       ← 建在**根层**（子分类由分类行菜单建，见下）
```

> **未分类不是一条分类记录**，只是根下过滤入口 —— 故它不出现在树里，也不可重命名/删除。
> 窄屏（下钻列表）的「新建分类」建在**当前目录**下，入口在列表底部（`DRILL_CREATE`）。

---

## 4. 逐屏规格

### A-C1 书架 · 分类导航与过滤（2026-10-09 改版：文件管理器式）

- **自适应**：窄屏（< 600dp，竖屏手机）用**下钻列表**（`CategoryDrill`，见 A-C6）；
  宽屏（横屏 / 平板）保留左右布局（`CategorySidebar` + 书籍列表）。
- **直属口径**：每层 = 直属子目录 + 直属书籍；根层（`-1`）= 顶层目录 + 未分类书籍；
  **每本书只出现一次**（与桌面旧口径「含子孙」的差异见 §5）。
- 可见集合由核心映射 `nr_list_book_categories` **逐本判定**；界面不遍历父子链、不派生语义。
- **归属不可判定**的书（宿主目录里没有 `bookId`，或映射缺失）**只在根层显示** ——
  否则它们在所有分类下都不可见。「全部书籍」（宽屏侧栏）不过滤。
- 取可见集合失败 ⇒ **明确报错**且**保持原状**（不得退化成「显示全部 / 空书架」）。

### A-C2 分类行 · 层级 / 选中 / 折叠 / 菜单

- 按 `depth` 缩进；**只有真有子分类的行才显示 ▾/▸**（不给假箭头）。
- 折叠只影响下发哪些行；**任一祖先折叠 ⇒ 该行不下发**。
- 计数显示：`direct` 与 `total` 不同时显示 `direct/total`，相同时只显示 `direct`。
- 行尾「⋯」打开操作菜单：**新建子分类 / 重命名 / 移动到… / 删除**（窄屏目录行同款，共用 `CategoryOpsDialogs`）。

### A-C3 新建 / 重命名分类

- 「新建分类」建在**当前选中分类**下（无选中 = 根层；窄屏 = 当前目录）；行菜单的「新建子分类」建在该行下。
- 单行输入；空名**本地拒绝**（不发请求）。
- 重名 / 非法层级由**核心拒绝并保留原值**；被拒后**重新读回**真实树
  （不留乐观状态 —— 与移动/删除同一条纪律）。

### A-C4 删除分类 · 去向选择

- 三种去向：**移入另一分类 / 一并移出书架 / 移回根下未分类**。
- **未选去向时确认按钮不发请求**；文案写明「原文件始终不受影响」。
- 选择「移入另一分类」时才展示目标列表（不可选项置灰仍显示）。
- 成功后回报**核心返回的移走本数**，并清空目标列表、重算可见集合。

### A-C5 移动到分类 / 移动分类

- 目标列表由核心 `nr_category_move_targets` 给出：含「未分类」+ 全部分类。
- **自身与全部子孙 `selectable = false`**（核心判定，界面不遍历父子链）—— 置灰并写明原因。
- 移动分类被拒（成环）⇒ 如实回显并**重新读回**。

### A-C6 窄屏下钻导航（`CategoryDrill`；2026-10-09 新增）

- 同一列表：当前目录的**子目录在上、书籍在下**；点目录行进入下一层（`levelChildren` 纯装配）。
- 「← 返回××」行 + **系统返回键**逐级回退；顶部分类 ⇒ 根层；根层 ⇒ 交给系统（可退出应用）。
- 窄屏没有「全部书籍」位置：启动或从宽屏旋转过来时**归一化到根层**（`-1`）。
- 目录行「⋯」与侧栏共用 `CategoryOpsDialogs`；底部「新建分类」建在**当前目录**下。
- 空层提示：根层且无目录 ⇒「书架为空」；其余 ⇒「（此目录下暂无书籍）」。

### A-C7 书籍移动（「移动到…」；2026-10-09 新增）

- 书籍行「⋯」→「移动到…」→ 目标弹框（「根下（未分类）」+ 全部分类，按 `depth` 缩进、标注「（当前）」）。
- 未登记的书（从未打开过、无核心 `bookId`）：菜单项**置灰并写明原因**，不发请求；VM 亦做防御性本地拒绝。
- 写入成功后回显「已把《书名》移入「目标名」」，并用同一读回路径刷新列表与侧栏（计数/内容自洽）。

### A-H1 最近阅读（`HistoryScreen`）

- 从**书架顶栏**进入（规格 R9：不放入分类侧栏）。
- 行：书名 · **核心格式化的时间标签** · 文件名 · 位置（`第 N 章 · 偏移 M 字节`）。
- **排序由核心给**（时间倒序），界面不重排。
- 点行 ⇒ 按 `book_id` 反查来源打开；映射缺失 ⇒ 如实回报「来源不在本机」，**不试探性打开**。

### A-H2 清空历史

- 逐条「清除」+ 底部「清空全部」（**二次确认**，文案写明不删书、不删进度、不删书签）。
- ABI 无「清空全部」入口 ⇒ 按当前行里出现的书**逐本**调 `nr_clear_history_for_book`，
  回显报告**核心返回值之和**（不是界面行数）。

### A-T1 设置 · 自定义配色

- 启用开关 + 4 个基色输入（正文 / 背景 / 主色 / 强调）+ 对比度读数 + 未设置提示。
- **只选基色**，其余语义色由核心派生；不分别设置明暗。
- 写入后**无论成败都回读** ⇒ 被拒的非法色值**不会留在输入框**。
- 对比度由核心给整数 ×100，界面只做显示换算；低于建议值时**保留用户基色**，只加提示。

### A-B1 设置 · 阅读背景

- 全局一张，作用于**阅读正文区**，不参与分页。
- 信息行（相对文件名 · 大小）+ 缩放口径三选 + 遮罩滑杆 + 「选择图片…」/「移除背景」。
- **缩放口径在选图时生效**：核心无「只改 fit」的导出，故做成「选图时使用的口径」
  并在文案写明 —— **不给按了不改存储的开关**。
- 选图流程：宿主拉起系统选图器 → 把 URI **物化**为本地路径 → 交给核心
  `nr_set_background`（核心复制到 app-private 目录并只存相对名）。
  **ViewModel 与核心都不触碰 `content://`。**
- 移除背景只删核心生成的文件；**用户原图永不动**。

---

## 5. 与桌面设计稿的对应（可追溯）

| 桌面稿 | Android 落地 | 差异说明 |
|---|---|---|
| C1 分类导航与过滤 | `CategorySidebar`（宽屏）+ `CategoryDrill`（窄屏） | **口径差异（2026-10-09）**：桌面侧栏为「含子孙」筛选；Android 改为**直属口径**（文件管理器式：目录在上、书籍在下、每本书只出现一次） |
| C2 分类行 | `CategoryTreeRow` + 下钻目录行 | 桌面行内菜单；Android 宽屏「⋯」、窄屏目录行「⋯」（共用 `CategoryOpsDialogs`） |
| C3 新建/重命名 | 命名对话框 | 同 |
| C4 删除去向 | 去向 + 目标共用对话框 | 同口径（未选不发请求） |
| C5 移动目标 | `nr_category_move_targets` | **共用同一核心判定** |
| C5b 移动到分类（**书籍**） | 书籍行「⋯」→ 移动到… | 2026-10-09 Android 补齐（此前仅桌面有）；写入共用 `nr_move_book_to_category` |
| H1–H3 历史 | `HistoryScreen` | 同口径（核心排序 + 核心时间标签） |
| T1–T4 配色 | 设置页配色区 | 同口径（只存基色、核心派生） |
| B1–B4 背景 | 设置页背景区 | 缩放口径改为「选图时确定」（核心无单独入口） |

**已知的分层差异（非语义差异）**：桌面把「折叠集合」传给核心投影
（`category_tree::project`），Android 在客户端装配（`CategoryTreeAssembly`）。
折叠规则由两端单测分别钉住，**核心仍是层级/计数/归属的唯一来源**。

---

## 6. 实现状态与证据

| 能力 | 关键文件 | 证据 |
|---|---|---|
| 分类导航（宽屏侧栏 + 窄屏下钻） | `ui/CategorySidebar.kt`、`ui/CategoryDrill.kt`、`reader/CategoryTreeAssembly` | `CategorySidebarTest`（24 项）+ `CategoryDrillTest`（11 项）；门禁 `verify-android-scope.py [14]`（含变异证明） |
| 书籍移动（移动到…） | `ui/ShelfScreen.kt` → `ReaderViewModel.onMoveBookToCategory` | `CategoryDrillTest`（移书 2 项）+ 入口可达性门禁 `[14]` |
| 历史面板 | `ui/HistoryScreen.kt` | `HistoryPanelTest`（8 项） |
| 历史打开链路 | `ReaderViewModel.openHistoryBook` | `HistoryOpenTest`（5 项） |
| 配色区 | `ui/SettingsScreen.kt`、`ThemeState` | `ThemeSettingsTest`（11 项） |
| 背景区 | `ui/SettingsScreen.kt`、`BackgroundState` | `BackgroundSettingsTest`（12 项） |
| ABI 契约 | `include/novel_reader.h`（40 项） | `abi_contract` / `abi_layout` / 八项门禁 |

界面可达性由 `verify-android-scope.py` 与 `verify-android-v11-parity.py` 等门禁覆盖；
所有新增交互均带 `testTag` + `contentDescription`（程序化驱动与无障碍可达）。

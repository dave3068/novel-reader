package com.novelreader.android.reader

import com.novelreader.android.core.NrException
import com.novelreader.android.core.NrErrorKind
import com.novelreader.android.core.NrAbiConstants

/**
 * 三个页面（子规格 A-T3：书架 / 阅读 / 设置）。
 *
 * 路由本身是**平台状态**（Compose 侧导航），不含任何阅读语义。
 */
enum class ReaderRoute {
    /** 书架页。 */
    SHELF,

    /** 阅读页。 */
    READER,

    /** 设置页。 */
    SETTINGS,

    /** 最近阅读（历史，V1.1）。 */
    HISTORY,
}

/**
 * 排版设置：用户可见单位（sp / 倍数 / dp）。
 *
 * 换算到像素只发生在 [TextLayoutSpec] 构造处（`sp × spToPx`、`dp × dpToPx`），
 * 并且**同一份像素值**同时用于测量与渲染（AR3 / 子规格 design.md §排版）。
 *
 * 段距与首行缩进**固定为 0**：桌面端已验证「测量侧叠加段距/缩进、渲染侧不叠加」会让
 * 每页少排若干行（见 novel-reader-ui-env 技能与开发方案 §8.3）；Android 在两侧渲染能力
 * 对齐前不暴露这两个设置，宁可少一个控件，也不提供「改了不生效」的假控件。
 */
data class TypographySettings(
    val fontSizeSp: Float = DEFAULT_FONT_SIZE_SP,
    val lineSpacingMultiplier: Float = DEFAULT_LINE_SPACING_MULTIPLIER,
    val horizontalMarginDp: Float = DEFAULT_HORIZONTAL_MARGIN_DP,
    val verticalMarginDp: Float = DEFAULT_VERTICAL_MARGIN_DP,
    val fontFamily: String = DEFAULT_FONT_FAMILY,
    val darkTheme: Boolean = false,
) {
    companion object {
        const val DEFAULT_FONT_SIZE_SP = 18f
        const val MIN_FONT_SIZE_SP = 12f
        const val MAX_FONT_SIZE_SP = 34f

        const val DEFAULT_LINE_SPACING_MULTIPLIER = 1.35f
        const val MIN_LINE_SPACING_MULTIPLIER = 1.0f
        const val MAX_LINE_SPACING_MULTIPLIER = 2.0f

        const val DEFAULT_HORIZONTAL_MARGIN_DP = 20f
        const val MIN_HORIZONTAL_MARGIN_DP = 0f
        const val MAX_HORIZONTAL_MARGIN_DP = 64f

        const val DEFAULT_VERTICAL_MARGIN_DP = 16f
        const val MIN_VERTICAL_MARGIN_DP = 0f
        const val MAX_VERTICAL_MARGIN_DP = 64f

        const val DEFAULT_FONT_FAMILY = "sans-serif"
        const val DEFAULT_FONT_WEIGHT = 400

        /** 段距（sp）——固定 0，见类注释。 */
        const val PARAGRAPH_SPACING_SP = 0f

        /** 首行缩进（sp）——固定 0，见类注释。 */
        const val FIRST_LINE_INDENT_SP = 0f
    }
}

/**
 * 书架条目。
 *
 * **AN3 的会话范围**：条目来自本次会话里真实打开成功的书籍（[ReaderViewModel] 从
 * `nr_open_txt` 的回执登记）。来源标识在 AN3 是本地文件路径 / 私有缓存副本路径；
 * AN4（SAF）会把同一字段换成持久 URI，本字段自始至终是不透明字符串，UI 不解析它。
 */
/**
 * 宿主把「一个 SAF 来源」解析为可打开目标。
 *
 * 平台窗口来源的描述符**不跨会话存活**（进程重启、Activity 重建、引擎重建后都失效），
 * 因此书架再次打开时必须由宿主重新建立窗口会话；本地文件 / 私有缓存副本路径则不需要。
 */
fun interface HostSourceResolver {

    /** 在后台线程调用；失败以异常携带可展示原因。 */
    fun resolve(sourceIdentity: String): HostOpenTarget
}

/** 宿主解析结果（与 `NrOpenRequest` 的 kind/totalBytes 口径一致）。 */
data class HostOpenTarget(
    val kind: Int,
    val identity: String,
    val totalBytes: Long,
    val displayName: String?,
)

data class ShelfBook(
    val sourceIdentity: String,
    val displayName: String,
    val bookId: Long,
    val totalBytes: Long,
    val chapterCount: Int,
    /**
     * 打开本条目所用的数据源种类（[com.novelreader.android.core.NrAbiConstants]）。
     *
     * 为 `NR_SOURCE_KIND_PLATFORM` 时，sourceIdentity 是 SAF URI，**不能**当文件路径再打开
     * （真机实测：把 content:// 当路径打开会得到 `NR_ERR_6 (IO) 打开书籍（数据源读取失败）`）。
     */
    val sourceKind: Int = NrAbiConstants.NR_SOURCE_KIND_FILE,
    val lastChapterIndex: Int? = null,
    val lastByteOffset: Long? = null,
) {
    /** 是否有可用的阅读进度（来自核心 `nr_get_position`）。 */
    val hasProgress: Boolean
        get() = lastChapterIndex != null && lastByteOffset != null
}

/** 书架状态（真实状态，空书架就是空列表）。 */
data class ShelfState(
    val books: List<ShelfBook> = emptyList(),
    val selectedBookId: Long? = null,
    val loading: Boolean = false,
) {
    val isEmpty: Boolean
        get() = books.isEmpty() && !loading
}

/** 章节条目（由核心章节批次解码而来；标题与区间都由核心裁定）。 */
data class ChapterEntry(
    val sequence: Int,
    val title: String,
    val startByte: Long,
    val endByte: Long,
)

/**
 * 页锚点：章序号 + 起始字节。
 *
 * 导航状态（「上一页」的落点），**不是**进度身份 —— 进度由核心的
 * `reading_progress`（chapter_index + original_byte_offset + 指纹）表达。
 */
data class PageAnchor(
    val chapterIndex: Int,
    val startByte: Long,
)

/** 一页正文的**展示事实**（测量结果 + 核心登记回执）。 */
data class PageState(
    val chapterIndex: Int,
    val chapterTitle: String,
    val text: String,
    val startByte: Long,
    val endByte: Long,
    val lineCount: Int,
    /** 实测行高（像素），渲染端据此设置 `lineHeight`（AR3）。 */
    val lineHeightPx: Float,
    /** 本次测量使用的字号（像素）。 */
    val fontSizePx: Float,
    /** 会话内页码（1 起；属于导航状态，不是进度身份）。 */
    val pageOrdinal: Int,
    /** 核心当前排版世代已登记的边界数量（`nr_boundary_result.page_count`）。 */
    val registeredPageCount: Long,
    val reachedContentEnd: Boolean,
)

/** 打开书籍后的阅读状态。 */
data class ReaderState(
    val bookId: Long? = null,
    val displayName: String = "",
    val totalBytes: Long = 0,
    val contentIndexVersion: Long = 0,
    /** 核心判定的文本编码展示名（如 `GB18030`）；空串 = 未知。 */
    val encodingName: String = "",
    /** 编码识别可信度（0..1000 千分比）。 */
    val encodingConfidence: Int = 0,
    /** true = 低可信度，核心要求用户确认编码（规格 CR2「低可信度编码」场景）。 */
    val needsEncodingConfirmation: Boolean = false,
    val chapters: List<ChapterEntry> = emptyList(),
    val page: PageState? = null,
    val layoutId: Long = 0,
    val requestId: Long = 0,
    val canGoPreviousPage: Boolean = false,
    /** 已预取并登记的后续页数（受 [ReaderViewModel.MAX_FORWARD_CACHE] 约束）。 */
    val forwardCacheCount: Int = 0,
    val indexRebuilding: Boolean = false,
    val lastSavedByteOffset: Long? = null,
) {
    /**
     * 阅读进度千分比（当前页起始字节 / 总字节；总长未知时为 0）。
     *
     * 用途：代替**页码**做界面读数 —— 页码只属某个排版签名（切章/重开/改字号即失真，
     * 真机实测恒显示「第 1 页」），而字节位置与排版无关，始终可算。
     */
    val progressPermille: Int
        get() {
            val current = page ?: return 0
            if (totalBytes <= 0L) return 0
            return ((current.startByte * 1000L) / totalBytes).toInt().coerceIn(0, 1000)
        }

    /** 进度展示文本（界面用；例：`35.2%`）。 */
    val progressText: String
        get() = formatPermille(progressPermille)

    val isOpen: Boolean
        get() = bookId != null
}

/**
 * UI 失败状态：只携带错误码数值与**分类**。
 *
 * 提示文案由 [messageFor] 依据分类给出，绝不解析跨边界错误文本
 * （头文件 §2「错误文本只用于展示，禁止用文本判断业务分支」）。
 */
data class ReaderFailure(
    val rawCode: Int,
    val kind: NrErrorKind,
    val detail: String? = null,
)

/** 应用状态（Compose 只显示它，不保存第二份阅读业务）。 */

/**
 * 缩放口径（与头文件 `nr_background.fit_mode` 一致）。
 *
 * ⚠️ 核心**没有**「只改 `fit_mode`」的导出（存储层有 `set_fit_mode`，未在 ABI 暴露），
 * 故该项只能在**选图时**确定；界面把它做成只读并写明缘由。
 */
object BackgroundFit {
    const val COVER = 0
    const val TILE = 1
    const val CENTER = 2

    /** 三项显示名（顺序与取值一致）。 */
    val LABELS = listOf("覆盖铺满", "原始尺寸平铺", "原始尺寸居中")

    /** 取值 → 显示名（越界回「未知」）。 */
    fun labelOf(value: Long): String = LABELS.getOrNull(value.toInt()) ?: "未知"
}

/**
 * 阅读背景状态（V1.1）。
 *
 * **只存相对文件名**（规格 R15）—— 绝对路径由界面用 app-private 目录拼，
 * 核心不猜平台数据目录。`fileName` 为空 = 未设置。
 */
data class BackgroundState(
    val fileName: String = "",
    val fileSize: Long = 0L,
    val scrimPermille: Long = DEFAULT_SCRIM_PERMILLE,
    val fitMode: Long = BackgroundFit.COVER.toLong(),
    /** 遮罩 alpha（0–255）—— **核心已从千分比换算**，界面不再换算。 */
    val scrimAlpha: Int = 0,
) {
    companion object {
        /** 与核心 `background::DEFAULT_SCRIM_PERMILLE` 同值。 */
        const val DEFAULT_SCRIM_PERMILLE = 400L
    }

    val hasImage: Boolean
        get() = fileName.isNotEmpty()

    /** 缩放口径显示名。 */
    val fitLabel: String
        get() = BackgroundFit.labelOf(fitMode)

    /** 遮罩强度显示（千分比 → 百分比）。 */
    val scrimLabel: String
        get() = "${(scrimPermille / 10)}%"

    /** 文件大小显示（字节 → KB/MB；仅在已设置时有意义）。 */
    val sizeLabel: String
        get() = when {
            !hasImage -> "—"
            fileSize < 1024 -> "$fileSize B"
            fileSize < 1024 * 1024 -> "${fileSize / 1024} KB"
            else -> "${fileSize / (1024 * 1024)} MB"
        }
}


/**
 * 移动 / 删除目标的一行（V1.1）。
 *
 * [selectable] 由**核心**给出（自身与子孙为 false）—— 界面据此置灰并解释原因，
 * **不自行遍历父子链**判断可达性。
 */
data class CategoryTargetView(
    val id: Long,
    val name: String,
    val depth: Int,
    val selectable: Boolean,
    val totalCount: Long,
) {
    /** 缩进（层级由核心给）。 */
    val indentLevel: Int
        get() = depth

    /** 显示文本（不可选时明确标注原因，而不是让用户猜）。 */
    val label: String
        get() = if (selectable) "$name（$totalCount 本）" else "$name（不可选：自身或子孙）"
}

/** 删除去向（与核心 `nr_delete_category` 的 `destination` 一致）。 */
object DeleteDestination {
    /** 未选 —— **不得执行删除**（核心会拒绝）。 */
    const val NONE = 0
    /** 移入另一分类（需 [CategoryState] 的目标 id）。 */
    const val MOVE_TO = 1
    /** 一并移出书架。 */
    const val REMOVE_FROM_SHELF = 2
    /** 移回根下未分类。 */
    const val TO_UNCATEGORIZED = 3
}

/**
 * 侧栏树的一行（V1.1）。
 *
 * 除 [expanded] / [selected] 是纯视图状态外，其余字段**全部来自核心**
 * （`depth` 由核心给 —— 界面不自己算层级）。
 */
data class CategoryRowView(
    val id: Long,
    val name: String,
    /** 层级（核心给）：`0` = 根层分类。 */
    val depth: Int,
    /** 直接归属本分类的在架书籍数（核心给）。 */
    val directCount: Long,
    /** 含子孙的在架书籍数（核心给）—— 徽标信息；选中后的列表口径为**直属**（见 [CategoryState]）。 */
    val totalCount: Long,
    val hasChildren: Boolean,
    val expanded: Boolean,
    val selected: Boolean,
) {
    /** 侧栏缩进（每层 1 级）。 */
    val indentLevel: Int
        get() = depth

    /** 「科幻 · 3/12」这类显示文本（直接/合计）。 */
    val countLabel: String
        get() = if (totalCount == directCount) "$directCount" else "$directCount/$totalCount"
}

/**
 * 书架分类（V1.1；2026-10-09 起为「文件管理器式」**直属口径**）。
 *
 * [selectedId]：`null` = 「全部书籍」（不过滤；宽屏侧栏专用）；`-1` = 根层
 * （窄屏下钻起点：目录 = 顶层分类、书籍 = 未分类）；`> 0` = 某个分类
 * （可见书籍 = **直属**该分类的书）。
 *
 * 直属归属与可见集合都来自核心映射 [bookCategories]（`nr_list_book_categories`）
 * —— 界面不遍历父子链、不自行派生分类语义。
 */
data class CategoryState(
    val rows: List<CategoryRowView> = emptyList(),
    /** 完整分类节点（核心给；下钻层级、路径与移动目标都由它装配）。 */
    val nodes: List<CategoryTreeAssembly.Node> = emptyList(),
    /** 书籍 → 分类映射（核心 `nr_list_book_categories`；`-1` = 未分类）。 */
    val bookCategories: Map<Long, Long> = emptyMap(),
    val selectedId: Long? = null,
    /** 已折叠的分类 id（只影响**下发哪些行**，不影响核心的任何计算）。 */
    val collapsed: Set<Long> = emptySet(),
    val rootTotalCount: Long = 0,
    val uncategorizedCount: Long = 0,
    val loading: Boolean = false,
) {
    val hasCategories: Boolean
        get() = rows.isNotEmpty()
}

/**
 * 侧栏树的纯装配（V1.1）。
 *
 * # 为什么在客户端
 *
 * 核心 `nr_list_categories` 返回的是**扁平列表**（带 `parentId` 与 `depth`，DTO 注释即写明
 * 「树形由界面按 parentId 组装」）。本对象只做**视图装配**：按父子链把扁平行排成深度优先序，
 * 并剔除祖先被折叠的行 —— 不派生任何核心语义（层级、计数、归属判断全部来自核心）。
 *
 * ⚠️ 桌面侧在同一件事上选择了「核心投影」（`category_tree::project` 接收折叠集合）。
 * 两者**分层不同、语义必须相同**：折叠规则（祖先折叠 ⇒ 子孙不显示；根行恒显示）在此以
 * 单测钉住，任何一侧改动都要同步。
 */
object CategoryTreeAssembly {

    /** 未分类哨兵（与核心 `nr_list_categories` / `nr_list_books_in_scope` 一致）。 */
    const val UNCATEGORIZED = -1L

    /**
     * 一条核心条目中装配所必需的部分。
     *
     * 用独立类型而不是直接吃核心 DTO：界面模型不依赖 ABI 结构体，
     * 核心换布局时这层不用动。
     */
    data class Node(
        val id: Long,
        val parentId: Long,
        val name: String,
        val depth: Int,
        val directCount: Long,
        val totalCount: Long,
    )

    /**
     * 深度优先展开成可下发的行。
     *
     * - 兄弟之间**保持核心给的顺序**（不在客户端重排，避免与桌面分叉）；
     * - 某行只要**任一祖先被折叠**就不下发（自身折叠只影响其子孙，不影响自己）；
     * - 父链缺失的孤儿节点视为根层（不让一个脏数据把整棵树吞掉）。
     */
    fun flatten(
        nodes: List<Node>,
        collapsed: Set<Long>,
        selectedId: Long?,
        rootTotalCount: Long = 0,
        uncategorizedCount: Long = 0,
    ): List<CategoryRowView> {
        val byParent = nodes.groupBy { node ->
            if (nodes.any { it.id == node.parentId }) node.parentId else ROOT
        }
        val out = mutableListOf<CategoryRowView>()
        // 显式栈（避免深树递归栈溢出）；children 逆序入栈以保持原顺序。
        val stack = ArrayDeque<Pair<Node, Boolean>>()
        (byParent[ROOT] ?: emptyList()).asReversed().forEach { stack.addLast(it to false) }
        while (stack.isNotEmpty()) {
            val (node, hidden) = stack.removeLast()
            val children = byParent[node.id] ?: emptyList()
            if (!hidden) {
                out += CategoryRowView(
                    id = node.id,
                    name = node.name,
                    depth = node.depth,
                    directCount = node.directCount,
                    totalCount = node.totalCount,
                    hasChildren = children.isNotEmpty(),
                    expanded = node.id !in collapsed,
                    selected = selectedId == node.id,
                )
            }
            val childHidden = hidden || node.id in collapsed
            children.asReversed().forEach { child -> stack.addLast(child to childHidden) }
        }
        return out
    }

    /**
     * 下钻：某位置的**直属子目录**行（保持核心给出的兄弟顺序）。
     *
     * [location]：`null` / [UNCATEGORIZED] = 根层（无真实父节点的节点，含孤儿容错 ——
     * 与 [flatten] 同一规则）；其他 = 该分类的直属子分类。空列表 = 没有子目录。
     */
    fun levelChildren(nodes: List<Node>, location: Long?): List<CategoryRowView> {
        if (nodes.isEmpty()) return emptyList()
        val byParent = nodes.groupBy { node ->
            if (nodes.any { it.id == node.parentId }) node.parentId else ROOT
        }
        val atRoot = location == null || location == UNCATEGORIZED
        val children = if (atRoot) byParent[ROOT] ?: emptyList() else byParent[location] ?: emptyList()
        return children.map { node ->
            CategoryRowView(
                id = node.id,
                name = node.name,
                depth = node.depth,
                directCount = node.directCount,
                totalCount = node.totalCount,
                hasChildren = byParent[node.id]?.isNotEmpty() == true,
                expanded = true,
                selected = false,
            )
        }
    }

    /**
     * 从根到 [id] 的节点路径（含 [id] 自身；[id] 不存在时为空列表）。
     *
     * 面包屑与「返回上一级」文案都由它派生。经父链上溯；核心已防环，这里仍有
     * 防御性上限（同节点不重复入路径、最多 [MAX_PATH_DEPTH] 层）。
     */
    fun pathTo(nodes: List<Node>, id: Long): List<Node> {
        if (nodes.isEmpty()) return emptyList()
        val byId = nodes.associateBy { it.id }
        val path = ArrayDeque<Node>()
        val seen = HashSet<Long>()
        var current = byId[id] ?: return emptyList()
        while (seen.add(current.id)) {
            path.addFirst(current)
            if (path.size >= MAX_PATH_DEPTH) break
            current = byId[current.parentId] ?: break
        }
        return path.toList()
    }

    /** [id] 的**真实**父节点 id（没有 / 父节点缺失 ⇒ `null`，调用方映射为根层）。 */
    fun parentOf(nodes: List<Node>, id: Long): Long? {
        val node = nodes.firstOrNull { it.id == id } ?: return null
        return if (nodes.any { it.id == node.parentId }) node.parentId else null
    }

    /** 父链起点（`parentId` 无处可挂时归到这里）。 */
    private const val ROOT = Long.MIN_VALUE

    /** 路径深度上限（防御性；核心已保证无环）。 */
    private const val MAX_PATH_DEPTH = 64
}
/**
 * 阅读历史的一行（V1.1）。
 *
 * [timeLabel] **由核心格式化**（今天 `HH:MM` / 本年 `MM-DD HH:MM` / 跨年 `yyyy-MM-dd HH:MM`）
 * —— 界面原样显示，**不得**再格式化或截断（规格 §3.6）。
 *
 * [bookId] 用于「点条目回到那本书」；来源映射由宿主目录提供（见 `openHistoryBook`）。
 */
data class HistoryRow(
    val entryId: Long,
    val bookId: Long,
    val title: String,
    /** 来源**末段文件名**（核心已取末段，不含路径分隔符）。 */
    val fileLabel: String,
    /** 核心格式化的时间标签。 */
    val timeLabel: String,
    /** 进入方式码（`0`=书架打开 `1`=阅读页打开 `2`=进度恢复 `3`=书签跳转 `4`=章节跳转）。 */
    val entryKind: Long,
    val chapterIndex: Long,
    val byteOffset: Long,
) {
    /** 位置描述（章序号从 1 起；与桌面 `position_label` 同口径）。 */
    val positionLabel: String
        get() = "第 ${chapterIndex + 1} 章 · 偏移 $byteOffset 字节"
}

/**
 * 历史状态（V1.1）。
 *
 * [total] 是核心给的**全部流水条数**（不受 `limit` 影响），界面据此显示「共 N 条」。
 */
data class HistoryState(
    val rows: List<HistoryRow> = emptyList(),
    val total: Int = 0,
    val loading: Boolean = false,
) {
    val isEmpty: Boolean
        get() = rows.isEmpty() && !loading

    /** 空态标题（与核心侧固定文案一致）。 */
    val emptyTitle: String
        get() = "还没有阅读记录"

    /** 空态说明。 */
    val emptyDesc: String
        get() = "打开一本书后会在这里留下一条流水。"
}

/**
 * 自定义配色状态（V1.1）。
 *
 * **只存 4 个基色 + 启用位** —— 40 个语义令牌不落库（规格 R11），由核心派生；
 * 界面若要显示某个令牌的颜色，走 `NrEngine.getThemeToken(下标, dark)`。
 *
 * 对比度由核心以**整数 × 100** 回传（`454` ⇒ `4.54 : 1`），界面只做显示换算。
 */
data class ThemeState(
    val enabled: Boolean = false,
    val foreground: String = "",
    val background: String = "",
    val primary: String = "",
    val accent: String = "",
    /** 对比度读数（整数 × 100）；`0` = 尚未读取。 */
    val contrastX100: Int = 0,
    /** 是否达到可读性建议（不足时核心**已保留用户基色**，这里只提示）。 */
    val contrastPasses: Boolean = true,
) {
    /** 按 `which` 取基色（`0`=正文 `1`=背景 `2`=主色 `3`=强调）。 */
    fun baseOf(which: Int): String = when (which) {
        0 -> foreground
        1 -> background
        2 -> primary
        3 -> accent
        else -> ""
    }

    /** 对比度显示文本（如 `4.54 : 1`）；未读取时为占位。 */
    val contrastLabel: String
        get() = if (contrastX100 <= 0) {
            "尚未读取"
        } else {
            "${contrastX100 / 100}.${(contrastX100 % 100).toString().padStart(2, '0')} : 1"
        }

    /** 未设置任何基色（界面据此显示「未设置」）。 */
    val hasAnyBase: Boolean
        get() = foreground.isNotEmpty() || background.isNotEmpty() ||
            primary.isNotEmpty() || accent.isNotEmpty()
}


/** 配色读写失败的文案（界面只回显核心给的错误码，不复述业务判断）。 */
object NrStatusForTheme {
    fun readFailed(error: NrException): String = "读取配色失败（错误码 ${error.rawCode}）"

    fun writeFailed(error: NrException): String = "配色写入被拒（错误码 ${error.rawCode}），已保留原值"
}

/** 基色索引（与头文件 `nr_set_theme_base` 的 `which` 一致）。 */
object ThemeBase {
    const val FOREGROUND = 0
    const val BACKGROUND = 1
    const val PRIMARY = 2
    const val ACCENT = 3

    /** 四行的显示名（顺序与索引一致）。 */
    val LABELS = listOf("正文色", "背景色", "主色", "强调色")
}

data class ReaderUiState(
    val route: ReaderRoute = ReaderRoute.SHELF,
    val shelf: ShelfState = ShelfState(),
    val reader: ReaderState = ReaderState(),
    val settings: TypographySettings = TypographySettings(),
    /** 自定义配色（V1.1；只存基色，令牌由核心派生）。 */
    val theme: ThemeState = ThemeState(),
    /** 最近阅读（V1.1；时间标签由核心格式化）。 */
    val history: HistoryState = HistoryState(),
    /** 阅读背景图（V1.1；只存相对文件名）。 */
    val background: BackgroundState = BackgroundState(),
    /** 书架分类侧栏（V1.1；层级与计数全部来自核心）。 */
    val categories: CategoryState = CategoryState(),
    /** 当前的移动 / 删除目标选项（核心给；空 = 未在选目标）。 */
    val categoryTargets: List<CategoryTargetView> = emptyList(),
    /**
     * 当前作用域下**可见的核心 bookId 集合**（V1.1）。
     *
     * `null` = 不过滤（根「全部书籍」）。非空集合 = 只显示这些书；
     * 由核心 `nr_list_books_in_scope` 给出（子树展开在核心完成）。
     */
    val visibleBookIds: Set<Long>? = null,
    /** 正文区域尺寸（像素，不含状态栏/导航栏内边距，含边距设置）。 */
    val viewportWidthPx: Int = 0,
    val viewportHeightPx: Int = 0,
    /** sp → px 换算因子（含系统字体缩放）。 */
    val spToPx: Float = 1f,
    /** dp → px 换算因子。 */
    val dpToPx: Float = 1f,
    val abiVersion: Int? = null,
    val status: String = ReaderStatus.READY,
    val failure: ReaderFailure? = null,
) {
    /** 视口是否已就绪（未就绪时不得建立排版世代：零视口是非法签名）。 */
    val hasViewport: Boolean
        get() = viewportWidthPx > 0 && viewportHeightPx > 0
}

/** 界面状态文案（失败提示与动作回显）。 */
object ReaderStatus {
    const val READY = "就绪"
    const val OPENING = "正在打开书籍…"
    const val LAYOUT = "正在排版…"
    const val REFLOW = "排版设置已更新"
    const val NO_BOOK = "尚未打开书籍"
    const val EMPTY_SHELF = "书架为空：请先导入 TXT 文件"
    const val CONTENT_END = "已到内容末尾"
    const val SAVED = "阅读位置已保存"
    const val INDEX_REBUILDING = "正在重建章节索引…"
    const val INDEX_CANCELLED = "索引重建已取消（部分结果不落库）"

    /** 历史条目无法打开：宿主目录未接线（单测/纯文件场景）。 */
    const val HISTORY_NO_CATALOG = "无法从历史打开：宿主未提供书籍目录"

    /** 历史条目无法打开：该书来源不在本机（bookId ↔ 来源映射缺失）。 */
    const val HISTORY_SOURCE_MISSING = "无法从历史打开：该书的来源不在本机"

    /** 历史读取失败。 */
    fun historyReadFailed(code: Int): String = "读取阅读历史失败（错误码 $code）"

    /** 清除流水的结果回显（`false` = 该条已不存在）。 */
    fun historyCleared(removed: Boolean): String =
        if (removed) "已清除该条流水" else "该条流水已不存在"

    /** 宿主未提供背景图目录（单测 / 纯文件场景）。 */
    const val BACKGROUND_NO_DIR = "背景功能不可用：宿主未提供背景图目录"

    /** 宿主未提供选图入口。 */
    const val BACKGROUND_NO_PICKER = "背景功能不可用：宿主未提供选图入口"

    /** 读取背景失败。 */
    fun backgroundReadFailed(code: Int): String = "读取背景设置失败（错误码 $code）"

    /** 背景写入被拒（如格式不支持、遮罩越界）—— 原设置保持不变。 */
    fun backgroundWriteFailed(code: Int): String = "背景设置被拒（错误码 $code），原设置未改动"

    /**
     * 宿主生命周期取消（旋转 / 退到后台）的回显：取消的是在途任务，**不是**引擎句柄。
     *
     * 文案必须体现「句柄保留」，否则真机上会被误读成引擎已销毁（那时任何操作都会
     * `NR_ERR_ENGINE_DESTROYED`）。
     */
    fun hostCancelled(reason: String, count: Int): String =
        "$reason：已取消 $count 个在途任务（引擎句柄保留）"

    /** 统一的失败前缀（错误码数值 + 分类，不含跨边界文本）。 */
    fun failure(rawCode: Int, kind: NrErrorKind): String = "失败：NR_ERR_$rawCode（${kind.name}）"
}

/**
 * 错误分类 → 用户提示。
 *
 * 只依据 [NrErrorKind]（由错误码数值映射而来）；[NrErrorKind.UNKNOWN] 绝不当成成功。
 */
fun messageFor(kind: NrErrorKind): String = when (kind) {
    NrErrorKind.NONE -> "成功"
    NrErrorKind.CALLER -> "参数不合法（内部调用错误）"
    NrErrorKind.RANGE -> "偏移或章节越界"
    NrErrorKind.ENCODING -> "偏移不在合法字符边界"
    NrErrorKind.NOT_FOUND -> "书籍不存在，请重新导入"
    NrErrorKind.SESSION -> "尚未打开书籍"
    NrErrorKind.IO -> "数据源读取失败"
    NrErrorKind.CANCELLED -> "请求已取消"
    NrErrorKind.LAYOUT -> "排版世代已过期，正在重新排版"
    NrErrorKind.ENGINE_STATE -> "阅读引擎已销毁"
    NrErrorKind.INTERNAL -> "内部错误"
    NrErrorKind.CHAPTER -> "章节与偏移不一致"
    NrErrorKind.UNKNOWN -> "未知错误码（不得当作成功）"
}

/** 千分比 → 展示文本（整数运算，避免 Locale 差异；例：`352` → `35.2%`）。 */
internal fun formatPermille(permille: Int): String {
    val clamped = permille.coerceIn(0, 1000)
    return "${clamped / 10}.${clamped % 10}%"
}

package com.novelreader.android.reader

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
data class ReaderUiState(
    val route: ReaderRoute = ReaderRoute.SHELF,
    val shelf: ShelfState = ShelfState(),
    val reader: ReaderState = ReaderState(),
    val settings: TypographySettings = TypographySettings(),
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

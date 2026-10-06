package com.novelreader.android.reader

/**
 * 阅读设置的持久化快照。
 *
 * 只覆盖**排版与外观**（重启后必须记住的部分）；阅读进度由核心 `reading_progress` 负责，
 * 不在此处重复存储。
 */
data class ReaderSettingsSnapshot(
    val fontSizeSp: Float,
    val lineSpacingMultiplier: Float,
    val horizontalMarginDp: Float,
    val verticalMarginDp: Float,
    val darkTheme: Boolean,
)

/**
 * 阅读设置持久化端口。
 *
 * 真机缺陷：退出应用后字号 / 夜间模式被重置 —— 因为设置此前只存在于内存状态。
 * 契约：**不抛异常**（持久化失败只影响「记住设置」，绝不影响阅读主链路）；
 * [load] 返回 `null` 表示无记录或记录不可用，调用方保持默认值。
 */
interface ReaderSettingsStore {

    fun load(): ReaderSettingsSnapshot?

    fun save(snapshot: ReaderSettingsSnapshot)
}

/**
 * 宿主侧已登记的书籍条目（用于重启后重建书架，使书架不再被清空）。
 *
 * [sourceIdentity] 必须是**可直接打开**的标识：平台窗口来源为 SAF URI，
 * 私有缓存来源为缓存文件路径。
 */
data class HostBookEntry(
    val sourceIdentity: String,
    val displayName: String,
    val sourceKind: Int,
    /**
     * 核心 `book_id`（打开成功时由 [HostBookCatalog.rememberBookId] 回填）。
     *
     * `null` = 本次安装尚未打开过该书 —— 此时**无法**把核心侧的历史条目
     * （只带 `book_id`）映射回可打开的来源。
     */
    val bookId: Long? = null,
)

/**
 * 书架目录端口。
 *
 * 真机缺陷：退出应用后书架被清空 —— 书籍来源本就由宿主持久化（SAF 记录），
 * 但没有任何一方在启动时把它读回书架。
 *
 * [rememberBookId] 在打开成功后回填核心 `book_id`，使宿主记录可关联核心进度。
 */
interface HostBookCatalog {

    /** 已登记且**可打开**的书籍（不可打开的失效来源由实现方过滤，避免「点开必失败」）。 */
    fun list(): List<HostBookEntry>

    /** 打开成功后回填核心 book_id（幂等；未知来源应被忽略）。 */
    fun rememberBookId(sourceIdentity: String, bookId: Long)

    /**
     * 按核心 `book_id` 反查可打开来源（V1.1：阅读历史条目只带 `book_id`）。
     *
     * 默认实现遍历 [list]；只需覆盖 `rememberBookId` 的实现无需额外改动。
     * 找不到返回 `null` —— 调用方应**如实回报**，不得猜测性打开别的书。
     */
    fun byBookId(bookId: Long): HostBookEntry? =
        list().firstOrNull { entry -> entry.bookId == bookId }
}

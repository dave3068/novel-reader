package com.novelreader.android.core

/**
 * 头文件冻结常量的 Kotlin 镜像（名字与 `include/novel_reader.h` **逐字相同**，
 * 便于跨语言 grep 与 `scripts/verify-android-boundary.py` 逐项比对数值）。
 *
 * 这些常量只表达 ABI 事实（种类、阶段、状态、拒绝原因），不含任何阅读业务判定。
 */
object NrAbiConstants {

    /** `NR_ABI_VERSION`。 */
    const val NR_ABI_VERSION = 1

    /** 单次读取默认窗口（64 KiB）。 */
    const val NR_DEFAULT_WINDOW_BYTES = 65536L

    /** 单次读取窗口硬上限（256 KiB）。 */
    const val NR_MAX_WINDOW_BYTES = 262144L

    /** 单次章节批次最大条目数（禁止一次取走整库章节）。 */
    const val NR_MAX_CHAPTER_BATCH = 256

    /** V1 全局唯一内容单元标识。 */
    const val NR_CONTENT_UNIT_SINGLE = 1L

    /** 排版算法版本（参与排版签名；变化会失效分页缓存）。 */
    const val NR_LAYOUT_ALGORITHM_VERSION = 1

    /** 数据源种类：本地文件。 */
    const val NR_SOURCE_KIND_FILE = 1

    /** 数据源种类：平台窗口（需要 [NrSourceProvider]）。 */
    const val NR_SOURCE_KIND_PLATFORM = 2

    /** 页边界提交阶段：登记排版签名。 */
    const val NR_SUBMIT_BEGIN_LAYOUT = 1

    /** 页边界提交阶段：提交测量结果。 */
    const val NR_SUBMIT_COMMIT = 2

    /** 编码稳定数值。 */
    const val NR_ENCODING_UTF8 = 1L
    const val NR_ENCODING_UTF8_BOM = 2L
    const val NR_ENCODING_UTF16LE = 3L
    const val NR_ENCODING_UTF16BE = 4L
    const val NR_ENCODING_GB18030 = 5L
    const val NR_ENCODING_GBK = 6L
    const val NR_ENCODING_BIG5 = 7L

    /** 页边界状态（`nr_boundary_result.status`）。 */
    const val NR_BOUNDARY_COMMITTED = 0L
    const val NR_BOUNDARY_DUPLICATE_IDENTICAL = 1L
    const val NR_BOUNDARY_REJECTED = 2L
    const val NR_BOUNDARY_STALE_LAYOUT_ID = 3L
    const val NR_BOUNDARY_STALE_REQUEST_ID = 4L
    const val NR_BOUNDARY_STALE_CONTENT_INDEX_VERSION = 5L
    const val NR_BOUNDARY_NOT_FOUND = 6L
    const val NR_BOUNDARY_LAYOUT_READY = 100L

    /** 页边界拒绝原因（`nr_boundary_result.rejection`，仅 REJECTED 时非 0）。 */
    const val NR_REJECT_REVERSED_INTERVAL = 1L
    const val NR_REJECT_EMPTY_INTERVAL = 2L
    const val NR_REJECT_OUT_OF_RANGE = 3L
    const val NR_REJECT_ILLEGAL_CHAR_BOUNDARY = 4L
    const val NR_REJECT_GAP_BEFORE_PREVIOUS = 5L
    const val NR_REJECT_OVERLAP_WITH_PREVIOUS = 6L
    const val NR_REJECT_NOT_ADVANCING = 7L
    const val NR_REJECT_CONFLICT_WITH_CACHED = 8L
    const val NR_REJECT_TOTAL_LENGTH_MISMATCH = 9L

    /** `outBuffer: LongArray` 的槽位数：(ptr, len, capacity)。 */
    const val NR_BUFFER_SLOTS = 3

    /** `layoutFields: LongArray` 的槽位数（与 `nr_layout_signature` 字段顺序一致）。 */
    const val NR_LAYOUT_FIELDS_SLOTS = 13
}

/**
 * JNI 绑定接口：与 [NrCore] 的 18 项 `external fun` **同签名、同顺序**的实例化视图。
 *
 * 它存在的唯一理由是让 [NrEngine] 的薄封装能在 JVM 单测里被替换 —— 宿主机无法加载
 * 面向 arm64 的 `libnr_jni.so`，而生命周期 / 错误映射 / 缓冲区归还语义必须被真实断言。
 * 生产实现在 [NrJniAbi] 中是一比一透传，**不含任何逻辑**（不解析错误文本、不做坐标运算、
 * 不做业务判定）。
 */
internal interface NrAbi {
    fun abiVersion(): Int
    fun engineCreate(maxWindowBytes: Long, dbPath: ByteArray?, provider: NrSourceProvider?): Long
    fun engineDestroy(engine: Long): Int
    fun openTxt(
        engine: Long,
        kind: Int,
        totalBytes: Long,
        identity: ByteArray,
        displayName: ByteArray?,
        outResult: ByteArray,
    ): Int

    fun closeBook(engine: Long, bookId: Long): Int
    fun readWindow(
        engine: Long,
        chapterIndex: Int,
        offset: Long,
        length: Long,
        outWindow: ByteArray,
        outBuffer: LongArray,
    ): ByteArray?

    fun submitPageBoundary(
        engine: Long,
        phase: Int,
        contentIndexVersion: Long,
        layoutId: Long,
        requestId: Long,
        totalBytes: Long,
        startByte: Long,
        endByte: Long,
        layoutFields: LongArray?,
        fontFamily: ByteArray?,
        outResult: ByteArray,
    ): Int

    fun getPageBoundary(engine: Long, layoutId: Long, chapterIndex: Int, startByte: Long, outResult: ByteArray): Int
    fun savePosition(
        engine: Long,
        chapterIndex: Int,
        bookId: Long,
        contentUnitId: Long,
        originalByteOffset: Long,
        fingerprint: ByteArray?,
        outChanged: IntArray?,
    ): Int

    fun getPosition(engine: Long, bookId: Long, outResult: ByteArray, outBuffer: LongArray): ByteArray?
    fun getChapters(
        engine: Long,
        bookId: Long,
        startSequence: Int,
        maxCount: Int,
        outBatch: ByteArray,
        outBuffer: LongArray,
    ): ByteArray?

    fun createBookmark(
        engine: Long,
        chapterIndex: Int,
        bookId: Long,
        contentUnitId: Long,
        byteOffset: Long,
        title: ByteArray?,
        outBookmarkId: LongArray,
    ): Int

    fun deleteBookmark(engine: Long, bookmarkId: Long, outDeleted: IntArray?): Int
    fun invalidateLayout(engine: Long, layoutId: Long, outRemoved: LongArray?): Int
    fun rebuildIndex(engine: Long, bookId: Long, requestId: Long, outResult: ByteArray): Int

    /** 19. 手动指定内容编码（0 = 清除覆盖）。 */
    fun setEncoding(engine: Long, bookId: Long, encodingCode: Int, outResult: ByteArray): Int

    // ---- 分类树（V1.1；加性契约修订）----

    /** 列出分类树（一次性返回整棵树 + 名称字节池）。 */
    fun listCategories(engine: Long, outBatch: ByteArray, outBuffer: LongArray): ByteArray?

    /** 新建分类；`parentId = -1` 表示根下。同父同名由核心拒绝。 */
    fun createCategory(engine: Long, parentId: Long, name: ByteArray, outCategoryId: LongArray): Int

    /** 移动书籍到分类；`categoryId = -1` 表示未分类（根）。 */
    fun moveBookToCategory(engine: Long, bookId: Long, categoryId: Long): Int

    /**
     * 删除分类。`destination` **必须显式给出**：`1`=移入 [targetId]、`2`=移出书架、
     * `3`=移回根；传 `0`（未选去向）核心拒绝且**零副作用**（与桌面同语义）。
     */
    fun deleteCategory(engine: Long, categoryId: Long, destination: Int, targetId: Long, outMovedCount: IntArray): Int

    // ---- 阅读历史（V1.1；加性契约修订）----

    /**
     * 列出阅读历史。[bookId] > 0 限定单书；[limit] > 0 限量；[grouped] != 0 走按书聚合。
     *
     * **排序在核心完成**，调用方不得自行排序。
     */
    fun listHistory(
        engine: Long,
        bookId: Long,
        limit: Int,
        grouped: Int,
        outBatch: ByteArray,
        outBuffer: LongArray,
    ): ByteArray?

    /** 清除一条阅读流水；`outRemoved` 写回 1 = 已清除、0 = 该 id 不存在。 */
    fun clearHistoryEntry(engine: Long, entryId: Long, outRemoved: IntArray): Int

    /** 清除某本书的全部阅读流水；**只删流水**（不动书籍 / 进度 / 书签）。 */
    fun clearHistoryForBook(engine: Long, bookId: Long, outRemovedCount: IntArray): Int

    // ---- 自定义配色（V1.1；加性契约修订）----

    /** 读回基色；四项各 12 字节，**全 0 = 该项未设置**。 */
    fun getThemeBase(engine: Long, outBase: ByteArray): Int

    /**
     * 写入一个基色。[which]：`0`=正文 `1`=背景 `2`=主色 `3`=强调；
     * [value] 长度 0 = **清除该项**；非法值被拒且**保留原值**。
     */
    fun setThemeBase(engine: Long, which: Int, value: ByteArray): Int

    /** 切换启用；关闭时**保留基色**。 */
    fun setThemeEnabled(engine: Long, enabled: Int): Int

    /** 取一个派生令牌的 RGBA（4 字节）；派生**全在核心**（两端一致）。 */
    fun getThemeToken(engine: Long, which: Int, dark: Int, outRgba: ByteArray): Int

    /** 取对比度读数（比值 × 100）；`passes = 0` 时核心**已保留用户基色**。 */
    fun getThemeContrast(engine: Long, outResult: ByteArray): Int

    // ---- 全局背景图（V1.1；加性契约修订）----

    /**
     * 读回背景。**只回相对文件名**（R15）—— 调用方拼自己的 app-private 前缀。
     *
     * `outBuffer` 承载核心分配的字节缓冲，用完必须走 `releaseBuffer` 归还。
     */
    fun getBackground(engine: Long, outBatch: ByteArray, outBuffer: LongArray): ByteArray?

    /**
     * 设置背景：核心**复制** [srcPath] 到 [destDir]（原图只读），只把相对名写库。
     *
     * [destDir] 由调用方给；[fitMode] < 0 或 [scrimPermille] < 0 = 用默认值。
     */
    fun setBackground(
        engine: Long,
        srcPath: ByteArray,
        destDir: ByteArray,
        fitMode: Int,
        scrimPermille: Int,
        outBatch: ByteArray,
        outBuffer: LongArray,
    ): ByteArray?

    /** 只调遮罩强度（**不换图**）；[scrimPermille] 必须在 `0..=1000`。 */
    fun setBackgroundScrim(
        engine: Long,
        scrimPermille: Int,
        outBatch: ByteArray,
        outBuffer: LongArray,
    ): ByteArray?

    /** 清除背景：**只删核心生成的文件**；**不动用户原图**。 */
    fun clearBackground(
        engine: Long,
        destDir: ByteArray,
        outBatch: ByteArray,
        outBuffer: LongArray,
    ): ByteArray?

    /**
     * 合法的移动 / 删除目标（`sourceId` 自身与全部子孙 `selectable = false`）。
     *
     * 环判定在**核心**完成 —— 界面不得自行遍历父子链。
     */
    fun categoryMoveTargets(
        engine: Long,
        sourceId: Long,
        outBatch: ByteArray,
        outBuffer: LongArray,
    ): ByteArray?

    /** 重命名分类；空名 / 坏 UTF-8 由核心拒绝并保留原值。 */
    fun renameCategory(engine: Long, categoryId: Long, name: ByteArray): Int

    /** 移动分类到新父分类（`-1` = 根层）；移入自身或子孙必须拒绝（规格 R5）。 */
    fun moveCategory(engine: Long, categoryId: Long, newParentId: Long): Int

    /**
     * 列出某作用域下的在架书籍 id（`scopeKind`：0 全部 / 1 未分类 / 2 分类含子孙）。
     *
     * 子树展开在**核心**完成 —— 界面只做集合成员判断，不遍历父子链。
     */
    fun listBooksInScope(
        engine: Long,
        scopeKind: Int,
        categoryId: Long,
        outBatch: ByteArray,
        outBuffer: LongArray,
    ): ByteArray?

    /**
     * 列出书籍 → 分类映射（只含在架书籍）；`-1` 表示未分类。
     *
     * 界面筛选用：宿主目录给 `sourceIdentity → bookId`，本入口给
     * `bookId → categoryId`，两者相合即可按分类过滤书架。
     */
    fun listBookCategories(engine: Long, outBatch: ByteArray, outBuffer: LongArray): ByteArray?


    fun cancelRequest(engine: Long, requestId: Long): Int
    fun getLastError(engine: Long, outInfo: ByteArray, outBuffer: LongArray): ByteArray?
    fun releaseBuffer(ptr: Long, len: Long, capacity: Long): Int
}

/** 生产实现：一比一透传到 [NrCore] 的 40 项 `external fun`（零逻辑）。 */
internal object NrJniAbi : NrAbi {

    override fun abiVersion(): Int = NrCore.nr_abi_version()

    override fun engineCreate(maxWindowBytes: Long, dbPath: ByteArray?, provider: NrSourceProvider?): Long =
        NrCore.nr_engine_create(maxWindowBytes, dbPath, provider)

    override fun engineDestroy(engine: Long): Int = NrCore.nr_engine_destroy(engine)

    override fun openTxt(
        engine: Long,
        kind: Int,
        totalBytes: Long,
        identity: ByteArray,
        displayName: ByteArray?,
        outResult: ByteArray,
    ): Int = NrCore.nr_open_txt(engine, kind, totalBytes, identity, displayName, outResult)

    override fun closeBook(engine: Long, bookId: Long): Int = NrCore.nr_close_book(engine, bookId)

    override fun readWindow(
        engine: Long,
        chapterIndex: Int,
        offset: Long,
        length: Long,
        outWindow: ByteArray,
        outBuffer: LongArray,
    ): ByteArray? = NrCore.nr_read_window(engine, chapterIndex, offset, length, outWindow, outBuffer)

    override fun submitPageBoundary(
        engine: Long,
        phase: Int,
        contentIndexVersion: Long,
        layoutId: Long,
        requestId: Long,
        totalBytes: Long,
        startByte: Long,
        endByte: Long,
        layoutFields: LongArray?,
        fontFamily: ByteArray?,
        outResult: ByteArray,
    ): Int = NrCore.nr_submit_page_boundary(
        engine,
        phase,
        contentIndexVersion,
        layoutId,
        requestId,
        totalBytes,
        startByte,
        endByte,
        layoutFields,
        fontFamily,
        outResult,
    )

    override fun getPageBoundary(
        engine: Long,
        layoutId: Long,
        chapterIndex: Int,
        startByte: Long,
        outResult: ByteArray,
    ): Int = NrCore.nr_get_page_boundary(engine, layoutId, chapterIndex, startByte, outResult)

    override fun savePosition(
        engine: Long,
        chapterIndex: Int,
        bookId: Long,
        contentUnitId: Long,
        originalByteOffset: Long,
        fingerprint: ByteArray?,
        outChanged: IntArray?,
    ): Int = NrCore.nr_save_position(
        engine,
        chapterIndex,
        bookId,
        contentUnitId,
        originalByteOffset,
        fingerprint,
        outChanged,
    )

    override fun getPosition(engine: Long, bookId: Long, outResult: ByteArray, outBuffer: LongArray): ByteArray? =
        NrCore.nr_get_position(engine, bookId, outResult, outBuffer)

    override fun getChapters(
        engine: Long,
        bookId: Long,
        startSequence: Int,
        maxCount: Int,
        outBatch: ByteArray,
        outBuffer: LongArray,
    ): ByteArray? = NrCore.nr_get_chapters(engine, bookId, startSequence, maxCount, outBatch, outBuffer)

    override fun createBookmark(
        engine: Long,
        chapterIndex: Int,
        bookId: Long,
        contentUnitId: Long,
        byteOffset: Long,
        title: ByteArray?,
        outBookmarkId: LongArray,
    ): Int = NrCore.nr_create_bookmark(engine, chapterIndex, bookId, contentUnitId, byteOffset, title, outBookmarkId)

    override fun deleteBookmark(engine: Long, bookmarkId: Long, outDeleted: IntArray?): Int =
        NrCore.nr_delete_bookmark(engine, bookmarkId, outDeleted)

    override fun invalidateLayout(engine: Long, layoutId: Long, outRemoved: LongArray?): Int =
        NrCore.nr_invalidate_layout(engine, layoutId, outRemoved)

    override fun setEncoding(engine: Long, bookId: Long, encodingCode: Int, outResult: ByteArray): Int =
        NrCore.nr_set_encoding(engine, bookId, encodingCode, outResult)

    override fun listCategories(engine: Long, outBatch: ByteArray, outBuffer: LongArray): ByteArray? =
        NrCore.nr_list_categories(engine, outBatch, outBuffer)

    override fun createCategory(engine: Long, parentId: Long, name: ByteArray, outCategoryId: LongArray): Int =
        NrCore.nr_create_category(engine, parentId, name, outCategoryId)

    override fun moveBookToCategory(engine: Long, bookId: Long, categoryId: Long): Int =
        NrCore.nr_move_book_to_category(engine, bookId, categoryId)

    override fun deleteCategory(
        engine: Long,
        categoryId: Long,
        destination: Int,
        targetId: Long,
        outMovedCount: IntArray,
    ): Int = NrCore.nr_delete_category(engine, categoryId, destination, targetId, outMovedCount)

    override fun listHistory(
        engine: Long,
        bookId: Long,
        limit: Int,
        grouped: Int,
        outBatch: ByteArray,
        outBuffer: LongArray,
    ): ByteArray? = NrCore.nr_list_history(engine, bookId, limit, grouped, outBatch, outBuffer)

    override fun clearHistoryEntry(engine: Long, entryId: Long, outRemoved: IntArray): Int =
        NrCore.nr_clear_history_entry(engine, entryId, outRemoved)

    override fun clearHistoryForBook(engine: Long, bookId: Long, outRemovedCount: IntArray): Int =
        NrCore.nr_clear_history_for_book(engine, bookId, outRemovedCount)

    override fun getThemeBase(engine: Long, outBase: ByteArray): Int =
        NrCore.nr_get_theme_base(engine, outBase)

    override fun setThemeBase(engine: Long, which: Int, value: ByteArray): Int =
        NrCore.nr_set_theme_base(engine, which, value)

    override fun setThemeEnabled(engine: Long, enabled: Int): Int =
        NrCore.nr_set_theme_enabled(engine, enabled)

    override fun getThemeToken(engine: Long, which: Int, dark: Int, outRgba: ByteArray): Int =
        NrCore.nr_get_theme_token(engine, which, dark, outRgba)

    override fun getThemeContrast(engine: Long, outResult: ByteArray): Int =
        NrCore.nr_get_theme_contrast(engine, outResult)

    override fun getBackground(engine: Long, outBatch: ByteArray, outBuffer: LongArray): ByteArray? =
        NrCore.nr_get_background(engine, outBatch, outBuffer)

    override fun setBackground(
        engine: Long,
        srcPath: ByteArray,
        destDir: ByteArray,
        fitMode: Int,
        scrimPermille: Int,
        outBatch: ByteArray,
        outBuffer: LongArray,
    ): ByteArray? = NrCore.nr_set_background(
        engine,
        srcPath,
        destDir,
        fitMode,
        scrimPermille,
        outBatch,
        outBuffer,
    )

    override fun setBackgroundScrim(
        engine: Long,
        scrimPermille: Int,
        outBatch: ByteArray,
        outBuffer: LongArray,
    ): ByteArray? = NrCore.nr_set_background_scrim(engine, scrimPermille, outBatch, outBuffer)

    override fun clearBackground(
        engine: Long,
        destDir: ByteArray,
        outBatch: ByteArray,
        outBuffer: LongArray,
    ): ByteArray? = NrCore.nr_clear_background(engine, destDir, outBatch, outBuffer)

    override fun listBookCategories(
        engine: Long,
        outBatch: ByteArray,
        outBuffer: LongArray,
    ): ByteArray? = NrCore.nr_list_book_categories(engine, outBatch, outBuffer)

    override fun categoryMoveTargets(
        engine: Long,
        sourceId: Long,
        outBatch: ByteArray,
        outBuffer: LongArray,
    ): ByteArray? = NrCore.nr_category_move_targets(engine, sourceId, outBatch, outBuffer)

    override fun renameCategory(engine: Long, categoryId: Long, name: ByteArray): Int =
        NrCore.nr_rename_category(engine, categoryId, name)

    override fun moveCategory(engine: Long, categoryId: Long, newParentId: Long): Int =
        NrCore.nr_move_category(engine, categoryId, newParentId)

    override fun listBooksInScope(
        engine: Long,
        scopeKind: Int,
        categoryId: Long,
        outBatch: ByteArray,
        outBuffer: LongArray,
    ): ByteArray? =
        NrCore.nr_list_books_in_scope(engine, scopeKind, categoryId, outBatch, outBuffer)

    override fun rebuildIndex(engine: Long, bookId: Long, requestId: Long, outResult: ByteArray): Int =
        NrCore.nr_rebuild_index(engine, bookId, requestId, outResult)

    override fun cancelRequest(engine: Long, requestId: Long): Int = NrCore.nr_cancel_request(engine, requestId)

    override fun getLastError(engine: Long, outInfo: ByteArray, outBuffer: LongArray): ByteArray? =
        NrCore.nr_get_last_error(engine, outInfo, outBuffer)

    override fun releaseBuffer(ptr: Long, len: Long, capacity: Long): Int =
        NrCore.nr_release_buffer(ptr, len, capacity)
}

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
    fun cancelRequest(engine: Long, requestId: Long): Int
    fun getLastError(engine: Long, outInfo: ByteArray, outBuffer: LongArray): ByteArray?
    fun releaseBuffer(ptr: Long, len: Long, capacity: Long): Int
}

/** 生产实现：一比一透传到 [NrCore] 的 18 项 `external fun`（零逻辑）。 */
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

    override fun rebuildIndex(engine: Long, bookId: Long, requestId: Long, outResult: ByteArray): Int =
        NrCore.nr_rebuild_index(engine, bookId, requestId, outResult)

    override fun cancelRequest(engine: Long, requestId: Long): Int = NrCore.nr_cancel_request(engine, requestId)

    override fun getLastError(engine: Long, outInfo: ByteArray, outBuffer: LongArray): ByteArray? =
        NrCore.nr_get_last_error(engine, outInfo, outBuffer)

    override fun releaseBuffer(ptr: Long, len: Long, capacity: Long): Int =
        NrCore.nr_release_buffer(ptr, len, capacity)
}

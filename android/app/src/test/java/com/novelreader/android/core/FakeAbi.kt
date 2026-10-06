package com.novelreader.android.core

import java.nio.ByteBuffer
import java.nio.ByteOrder

/** `nr_history_batch` 结构大小（与 `abi_layout` 的 EXPECTED_SIZES 一致）。 */
private const val NR_HISTORY_BATCH_SIZE = 32
/** `nr_background` 结构大小（与 `abi_layout` 的 EXPECTED_SIZES 一致）。 */
private const val NR_BACKGROUND_SIZE = 48
/** `nr_background_batch` 结构大小（与 `abi_layout` 的 EXPECTED_SIZES 一致）。 */
private const val NR_BACKGROUND_BATCH_SIZE = 32
/** `nr_theme_base` 结构大小（与 `abi_layout` 的 EXPECTED_SIZES 一致）。 */
private const val NR_THEME_BASE_SIZE = 56
/** `nr_theme_contrast_result` 结构大小（与 `abi_layout` 的 EXPECTED_SIZES 一致）。 */
private const val NR_THEME_CONTRAST_SIZE = 24

/** `nr_category_batch` 结构大小（与 `abi_layout` 的 EXPECTED_SIZES 一致）。 */
private const val NR_CATEGORY_BATCH_SIZE = 32

/**
 * 单测用的假 JNI 绑定：默认行为**模仿** `android/jni/nr_jni.c` 的真实语义
 * （结构体字节布局、`(ptr,len,capacity)` 槽位、空结果 = `(0,0,0)`、取消写回出参……），
 * 并提供旋钮用于构造负例（返回 null、槽位不一致、长度不一致）。
 *
 * 生产路径不使用本类（[NrJniAbi]）。
 */
internal class FakeAbi : NrAbi {

    companion object {
        const val HANDLE = 4242L
        const val PTR_BASE = 0x0000_7F00_0000_0000L
    }

    /** 调用序列（用于断言「销毁后不再触碰 native」等）。 */
    val calls = mutableListOf<String>()

    // ---- 返回值旋钮 ----
    var abiVersionResult = NrAbiConstants.NR_ABI_VERSION
    var engineCreateResult = HANDLE
    var engineDestroyResult = NrErrors.NR_ERR_OK
    var openTxtCode = NrErrors.NR_ERR_OK
    var closeBookCode = NrErrors.NR_ERR_OK
    var savePositionCode = NrErrors.NR_ERR_OK
    var rebuildIndexCode = NrErrors.NR_ERR_OK
    var cancelRequestCode = NrErrors.NR_ERR_OK

    /** 四个缓冲区入口的返回数组；置 null 表示调用失败（与 C 层一致）。 */
    var readWindowBytes: ByteArray? = ByteArray(0)
    var getPositionBytes: ByteArray? = ByteArray(0)
    var getChaptersBytes: ByteArray? = ByteArray(0)
    var lastErrorMessage: ByteArray = ByteArray(0)
    var getLastErrorReturnsNull = false
    var lastErrorCode = NrErrors.NR_ERR_OK

    // ---- 负例旋钮 ----
    /** 覆写写入 `outBuffer` 的槽位（默认按返回数组长度生成）。 */
    var slotsOverride: LongArray? = null

    /** 只覆写结构体内嵌槽位、不改 `outBuffer`（用于构造「槽位不一致」）。 */
    var structSlotsOnly: LongArray? = null

    /** 覆写 `nr_window.len`（用于构造「长度自述不一致」）。 */
    var windowLenOverride: Long? = null

    /** 章节批次的 `count` / `total_chapters`（默认从返回字节推导为 0）。 */
    var chapterCount = 0L
    var chapterTotal = 0L

    private var nextPtr = PTR_BASE

    private fun allocPtr(): Long {
        val ptr = nextPtr
        nextPtr += 4096
        return ptr
    }

    /** 与 C 层一致：空结果不登记缓冲区 ⇒ 凭据为 `(0,0,0)`。 */
    private fun slotsFor(bytes: ByteArray): LongArray {
        slotsOverride?.let { return it }
        return if (bytes.isEmpty()) {
            longArrayOf(0L, 0L, 0L)
        } else {
            val ptr = allocPtr()
            longArrayOf(ptr, bytes.size.toLong(), bytes.size.toLong())
        }
    }

    private fun copyInto(buffer: ByteBuffer, target: ByteArray) {
        val size = minOf(target.size, buffer.capacity())
        System.arraycopy(buffer.array(), 0, target, 0, size)
        // 目标数组比结构体大时，其余字节保持 0（调用方预分配即正确尺寸）。
    }

    private fun writeSlots(outBuffer: LongArray, slots: LongArray) {
        outBuffer[0] = slots[0]
        outBuffer[1] = slots[1]
        outBuffer[2] = slots[2]
    }

    private fun structSlots(fallback: LongArray): LongArray = structSlotsOnly ?: fallback

    private fun writer(size: Int): ByteBuffer =
        ByteBuffer.allocate(size).order(ByteOrder.LITTLE_ENDIAN)

    // ------------------------------------------------------------------ NrAbi

    override fun abiVersion(): Int {
        calls.add("abiVersion")
        return abiVersionResult
    }

    override fun engineCreate(maxWindowBytes: Long, dbPath: ByteArray?, provider: NrSourceProvider?): Long {
        calls.add("engineCreate")
        return engineCreateResult
    }

    override fun engineDestroy(engine: Long): Int {
        calls.add("engineDestroy")
        return engineDestroyResult
    }

    override fun openTxt(
        engine: Long,
        kind: Int,
        totalBytes: Long,
        identity: ByteArray,
        displayName: ByteArray?,
        outResult: ByteArray,
    ): Int {
        calls.add("openTxt")
        if (openTxtCode == NrErrors.NR_ERR_OK) {
            val buf = writer(NrOpenResult.STRUCT_SIZE)
            buf.putLong(0, 77L)
            buf.putLong(8, 4096L)
            buf.putLong(16, 5L)
            buf.putInt(24, NrAbiConstants.NR_CONTENT_UNIT_SINGLE.toInt())
            buf.putInt(28, NrAbiConstants.NR_ENCODING_UTF8.toInt())
            buf.putInt(32, 1000)
            buf.putInt(36, 990)
            buf.putInt(40, 0)
            buf.putInt(44, 3)
            buf.putInt(48, 1)
            copyInto(buf, outResult)
        }
        return openTxtCode
    }

    override fun closeBook(engine: Long, bookId: Long): Int {
        calls.add("closeBook")
        return closeBookCode
    }

    override fun readWindow(
        engine: Long,
        chapterIndex: Int,
        offset: Long,
        length: Long,
        outWindow: ByteArray,
        outBuffer: LongArray,
    ): ByteArray? {
        calls.add("readWindow")
        val bytes = readWindowBytes ?: return null
        val slots = slotsFor(bytes)
        val len = windowLenOverride ?: bytes.size.toLong()
        val buf = writer(NrWindow.STRUCT_SIZE)
        buf.putLong(0, offset)
        buf.putLong(8, offset + len)
        buf.putLong(16, 1L)
        buf.putLong(24, 4096L)
        buf.putInt(32, len.toInt())
        buf.putInt(36, 0)
        buf.putInt(40, 0)
        buf.putInt(44, if (offset + len >= 4096L) 1 else 0)
        copyInto(buf, outWindow)
        writeSlots(outBuffer, slots)
        return bytes
    }

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
    ): Int {
        calls.add("submitPageBoundary")
        val buf = writer(NrBoundaryResult.STRUCT_SIZE)
        val status = if (phase == NrAbiConstants.NR_SUBMIT_BEGIN_LAYOUT) {
            NrAbiConstants.NR_BOUNDARY_LAYOUT_READY
        } else {
            NrAbiConstants.NR_BOUNDARY_COMMITTED
        }
        buf.putInt(0, status.toInt())
        buf.putInt(4, 0)
        buf.putInt(8, NrErrors.NR_ERR_OK)
        buf.putInt(12, 0)
        buf.putLong(16, layoutId)
        buf.putLong(24, requestId)
        buf.putLong(32, 0L)
        buf.putLong(40, 0L)
        buf.putLong(48, startByte)
        buf.putLong(56, endByte)
        buf.putLong(64, totalBytes)
        buf.putLong(72, 1L)
        copyInto(buf, outResult)
        return NrErrors.NR_ERR_OK
    }

    override fun getPageBoundary(
        engine: Long,
        layoutId: Long,
        chapterIndex: Int,
        startByte: Long,
        outResult: ByteArray,
    ): Int {
        calls.add("getPageBoundary")
        val buf = writer(NrBoundaryResult.STRUCT_SIZE)
        buf.putInt(0, NrAbiConstants.NR_BOUNDARY_COMMITTED.toInt())
        buf.putInt(8, NrErrors.NR_ERR_OK)
        buf.putLong(16, layoutId)
        buf.putLong(48, startByte)
        buf.putLong(56, startByte + 256L)
        copyInto(buf, outResult)
        return NrErrors.NR_ERR_OK
    }

    override fun savePosition(
        engine: Long,
        chapterIndex: Int,
        bookId: Long,
        contentUnitId: Long,
        originalByteOffset: Long,
        fingerprint: ByteArray?,
        outChanged: IntArray?,
    ): Int {
        calls.add("savePosition")
        if (savePositionCode == NrErrors.NR_ERR_OK && outChanged != null) {
            outChanged[0] = 1
        }
        return savePositionCode
    }

    override fun getPosition(engine: Long, bookId: Long, outResult: ByteArray, outBuffer: LongArray): ByteArray? {
        calls.add("getPosition")
        val bytes = getPositionBytes ?: return null
        val slots = slotsFor(bytes)
        val struct = structSlots(slots)
        val buf = writer(NrPositionResult.STRUCT_SIZE)
        buf.putInt(0, NrPositionResult.STRUCT_SIZE)
        buf.putInt(4, 1)
        buf.putInt(8, 2)
        buf.putInt(12, 1)
        buf.putInt(16, 0)
        buf.putInt(20, 0)
        buf.putLong(24, bookId)
        buf.putLong(32, NrAbiConstants.NR_CONTENT_UNIT_SINGLE)
        buf.putLong(40, 512L)
        buf.putLong(48, 1_700_000_000L)
        buf.putLong(56, struct[0])
        buf.putLong(64, struct[1])
        buf.putLong(72, struct[2])
        copyInto(buf, outResult)
        writeSlots(outBuffer, slots)
        return bytes
    }

    override fun getChapters(
        engine: Long,
        bookId: Long,
        startSequence: Int,
        maxCount: Int,
        outBatch: ByteArray,
        outBuffer: LongArray,
    ): ByteArray? {
        calls.add("getChapters")
        val bytes = getChaptersBytes ?: return null
        val slots = slotsFor(bytes)
        val struct = structSlots(slots)
        val buf = writer(NrChapterBatch.STRUCT_SIZE)
        buf.putInt(0, chapterCount.toInt())
        buf.putInt(4, 7)
        buf.putInt(8, chapterTotal.toInt())
        buf.putInt(12, 0)
        buf.putLong(16, struct[0])
        buf.putLong(24, struct[1])
        buf.putLong(32, struct[2])
        copyInto(buf, outBatch)
        writeSlots(outBuffer, slots)
        return bytes
    }

    override fun createBookmark(
        engine: Long,
        chapterIndex: Int,
        bookId: Long,
        contentUnitId: Long,
        byteOffset: Long,
        title: ByteArray?,
        outBookmarkId: LongArray,
    ): Int {
        calls.add("createBookmark")
        outBookmarkId[0] = 99L
        return NrErrors.NR_ERR_OK
    }

    override fun deleteBookmark(engine: Long, bookmarkId: Long, outDeleted: IntArray?): Int {
        calls.add("deleteBookmark")
        if (outDeleted != null) {
            outDeleted[0] = 1
        }
        return NrErrors.NR_ERR_OK
    }

    override fun invalidateLayout(engine: Long, layoutId: Long, outRemoved: LongArray?): Int {
        calls.add("invalidateLayout")
        if (outRemoved != null) {
            outRemoved[0] = 12L
        }
        return NrErrors.NR_ERR_OK
    }

    override fun setEncoding(engine: Long, bookId: Long, encodingCode: Int, outResult: ByteArray): Int =
        NrErrors.NR_ERR_OK

    override fun rebuildIndex(engine: Long, bookId: Long, requestId: Long, outResult: ByteArray): Int {
        calls.add("rebuildIndex")
        if (rebuildIndexCode == NrErrors.NR_ERR_OK || rebuildIndexCode == NrErrors.NR_ERR_CANCELLED) {
            val buf = writer(NrRebuildResult.STRUCT_SIZE)
            buf.putInt(0, NrRebuildResult.STRUCT_SIZE)
            buf.putInt(4, if (rebuildIndexCode == NrErrors.NR_ERR_CANCELLED) 1 else 0)
            buf.putInt(8, 12)
            buf.putInt(12, 0)
            buf.putLong(16, 4096L)
            buf.putLong(24, 8192L)
            buf.putLong(32, 9L)
            copyInto(buf, outResult)
        }
        return rebuildIndexCode
    }

    override fun cancelRequest(engine: Long, requestId: Long): Int {
        calls.add("cancelRequest")
        return cancelRequestCode
    }

    override fun getLastError(engine: Long, outInfo: ByteArray, outBuffer: LongArray): ByteArray? {
        calls.add("getLastError")
        if (getLastErrorReturnsNull) {
            return null
        }
        val bytes = lastErrorMessage
        val slots = slotsFor(bytes)
        val buf = writer(NrErrorInfo.STRUCT_SIZE)
        buf.putInt(0, lastErrorCode)
        buf.putInt(4, if (bytes.isEmpty()) 0 else 1)
        buf.putLong(8, slots[0])
        buf.putLong(16, slots[1])
        buf.putLong(24, slots[2])
        copyInto(buf, outInfo)
        writeSlots(outBuffer, slots)
        return bytes
    }

    override fun releaseBuffer(ptr: Long, len: Long, capacity: Long): Int {
        calls.add("releaseBuffer")
        return NrErrors.NR_ERR_OK
    }

    // ---- V1.1 四组功能（替身实现：记录调用、可编排载荷）----
    //
    // ⚠️ **成功路径必须返回字节池 + 写回归还槽**，不能返回 `null` ——
    // `null` 在门面语义里表示**失败**（`abi.listX(...) ?: throw`）。
    // 早期版本的替身在成功时返回 `null`，导致每次门面调用都抛异常；
    // 因当时没有端口级 JVM 测试而未被发现（本轮补测时实测踩中）。

    /** 可编排的返回值码（四组功能共用；非 0 时**读**操作返回 `null` = 失败）。 */
    var v11Code: Int = NrErrors.NR_ERR_OK

    // ---------------- 分类 ----------------

    /** 可编排的分类字节池（`null` = 读失败）。 */
    var categoriesBytes: ByteArray? = ByteArray(0)

    /** 可编排的分类条目数。 */
    var categoryCount: Int = 0

    /** 删除时记录收到的去向（供测试断言「未选去向」传的是 0）。 */
    var lastDeleteDestination: Int? = null

    /** 新建分类时写回的分类 id。 */
    var createdCategoryId: Long = 42L

    /** 删除分类时写回的「被移动书籍数」。 */
    var deletedMovedCount: Int = 0

    override fun listCategories(engine: Long, outBatch: ByteArray, outBuffer: LongArray): ByteArray? {
        calls.add("listCategories")
        if (v11Code != NrErrors.NR_ERR_OK) return null
        val bytes = categoriesBytes ?: return null
        val slots = slotsFor(bytes)
        val struct = structSlots(slots)
        val buf = writer(NR_CATEGORY_BATCH_SIZE)
        buf.putInt(0, categoryCount)
        buf.putInt(4, 0)
        buf.putLong(8, struct[0])
        buf.putLong(16, struct[1])
        buf.putLong(24, struct[2])
        copyInto(buf, outBatch)
        writeSlots(outBuffer, slots)
        return bytes
    }

    override fun createCategory(
        engine: Long,
        parentId: Long,
        name: ByteArray,
        outCategoryId: LongArray,
    ): Int {
        calls.add("createCategory")
        if (v11Code == NrErrors.NR_ERR_OK && outCategoryId.isNotEmpty()) {
            outCategoryId[0] = createdCategoryId
        }
        return v11Code
    }

    override fun moveBookToCategory(engine: Long, bookId: Long, categoryId: Long): Int {
        calls.add("moveBookToCategory")
        return v11Code
    }

    override fun deleteCategory(
        engine: Long,
        categoryId: Long,
        destination: Int,
        targetId: Long,
        outMovedCount: IntArray,
    ): Int {
        calls.add("deleteCategory")
        lastDeleteDestination = destination
        if (v11Code == NrErrors.NR_ERR_OK && outMovedCount.isNotEmpty()) {
            outMovedCount[0] = deletedMovedCount
        }
        return v11Code
    }

    // ---------------- 阅读历史 ----------------

    /** 可编排的历史字节池（`null` = 读失败）。 */
    var historyBytes: ByteArray? = ByteArray(0)

    /** 可编排的流水条数。 */
    var historyCount: Int = 0

    /** 可编排的「全部流水条数」（不受 limit 影响）。 */
    var historyTotal: Int = 0

    /** 清除一条流水时写回的「是否已存在」。 */
    var historyEntryRemoved: Int = 1

    /** 清除单书流水时写回的条数。 */
    var historyClearedCount: Int = 0

    override fun listHistory(
        engine: Long,
        bookId: Long,
        limit: Int,
        grouped: Int,
        outBatch: ByteArray,
        outBuffer: LongArray,
    ): ByteArray? {
        calls.add("listHistory")
        if (v11Code != NrErrors.NR_ERR_OK) return null
        val bytes = historyBytes ?: return null
        val slots = slotsFor(bytes)
        val struct = structSlots(slots)
        val buf = writer(NR_HISTORY_BATCH_SIZE)
        buf.putInt(0, historyCount)
        buf.putInt(4, historyTotal)
        buf.putLong(8, struct[0])
        buf.putLong(16, struct[1])
        buf.putLong(24, struct[2])
        copyInto(buf, outBatch)
        writeSlots(outBuffer, slots)
        return bytes
    }

    override fun clearHistoryEntry(engine: Long, entryId: Long, outRemoved: IntArray): Int {
        calls.add("clearHistoryEntry")
        if (v11Code == NrErrors.NR_ERR_OK && outRemoved.isNotEmpty()) {
            outRemoved[0] = historyEntryRemoved
        }
        return v11Code
    }

    override fun clearHistoryForBook(engine: Long, bookId: Long, outRemovedCount: IntArray): Int {
        calls.add("clearHistoryForBook")
        if (v11Code == NrErrors.NR_ERR_OK && outRemovedCount.isNotEmpty()) {
            outRemovedCount[0] = historyClearedCount
        }
        return v11Code
    }

    // ---------------- 配色 ----------------

    // ---- 书籍分类映射（V1.1；筛选前置）----

    /** 可编排的映射载荷（`bookId → categoryId` 编码为 16 字节行）。 */
    var bookCategoriesBytes: ByteArray? = ByteArray(0)

    override fun listBookCategories(
        engine: Long,
        outBatch: ByteArray,
        outBuffer: LongArray,
    ): ByteArray? {
        calls.add("listBookCategories")
        if (v11Code != NrErrors.NR_ERR_OK) return null
        val bytes = bookCategoriesBytes ?: return null
        val slots = slotsFor(bytes)
        val struct = structSlots(slots)
        val buf = writer(NrBookCategoryBatch.STRUCT_SIZE)
        buf.putInt(0, bytes.size / NrBookCategoryEntry.STRUCT_SIZE)
        buf.putInt(4, 0)
        buf.putLong(8, struct[0])
        buf.putLong(16, struct[1])
        buf.putLong(24, struct[2])
        copyInto(buf, outBatch)
        writeSlots(outBuffer, slots)
        return bytes
    }




    // ---- 移动 / 删除目标（V1.1）----

    /** 可编排的目标（id, 深度, 可选, 计数, 名称）。 */
    var moveTargets: List<Triple<Long, Int, Pair<Boolean, String>>> = emptyList()
    var lastMoveTargetsSource: Long? = null

    override fun categoryMoveTargets(
        engine: Long,
        sourceId: Long,
        outBatch: ByteArray,
        outBuffer: LongArray,
    ): ByteArray? {
        calls.add("categoryMoveTargets")
        lastMoveTargetsSource = sourceId
        if (v11Code != NrErrors.NR_ERR_OK) return null
        val rowSize = NrCategoryChoiceEntry.STRUCT_SIZE
        val headerSize = rowSize * moveTargets.size
        val names = moveTargets.map { it.third.second.toByteArray(Charsets.UTF_8) }
        val bytes = ByteArray(headerSize + names.sumOf { it.size })
        val buf = java.nio.ByteBuffer.wrap(bytes).order(java.nio.ByteOrder.LITTLE_ENDIAN)
        var cursor = headerSize
        moveTargets.forEachIndexed { index, row ->
            val base = index * rowSize
            buf.putLong(base, row.first)
            buf.putInt(base + 8, row.second)
            buf.putInt(base + 12, if (row.third.first) 1 else 0)
            buf.putLong(base + 16, 0L)
            buf.putInt(base + 24, cursor)
            buf.putInt(base + 28, names[index].size)
            cursor += names[index].size
        }
        var at = headerSize
        names.forEach { it.copyInto(bytes, at); at += it.size }

        val slots = slotsFor(bytes)
        val struct = structSlots(slots)
        val header = writer(NrCategoryChoiceBatch.STRUCT_SIZE)
        header.putInt(0, moveTargets.size)
        header.putInt(4, 0)
        header.putLong(8, struct[0])
        header.putLong(16, struct[1])
        header.putLong(24, struct[2])
        copyInto(header, outBatch)
        writeSlots(outBuffer, slots)
        return bytes
    }

    // ---- 分类改名 / 移动（V1.1）----

    var lastRename: Pair<Long, String>? = null
    var lastCategoryMove: Pair<Long, Long>? = null

    override fun renameCategory(engine: Long, categoryId: Long, name: ByteArray): Int {
        calls.add("renameCategory")
        lastRename = categoryId to String(name, Charsets.UTF_8)
        return themeWriteCode
    }

    override fun moveCategory(engine: Long, categoryId: Long, newParentId: Long): Int {
        calls.add("moveCategory")
        lastCategoryMove = categoryId to newParentId
        return themeWriteCode
    }

    // ---- 作用域书籍 id（V1.1）----

    /** 可编排的 id 载荷（小端 i64）。 */
    var scopeIds: List<Long> = emptyList()
    var lastScopeRequest: Pair<Int, Long>? = null

    override fun listBooksInScope(
        engine: Long,
        scopeKind: Int,
        categoryId: Long,
        outBatch: ByteArray,
        outBuffer: LongArray,
    ): ByteArray? {
        calls.add("listBooksInScope")
        lastScopeRequest = scopeKind to categoryId
        if (v11Code != NrErrors.NR_ERR_OK) return null
        val bytes = ByteArray(scopeIds.size * 8)
        val buf = java.nio.ByteBuffer.wrap(bytes).order(java.nio.ByteOrder.LITTLE_ENDIAN)
        scopeIds.forEachIndexed { index, id -> buf.putLong(index * 8, id) }
        val slots = slotsFor(bytes)
        val struct = structSlots(slots)
        val header = writer(NrIdList.STRUCT_SIZE)
        header.putInt(0, scopeIds.size)
        header.putInt(4, 0)
        header.putLong(8, struct[0])
        header.putLong(16, struct[1])
        header.putLong(24, struct[2])
        copyInto(header, outBatch)
        writeSlots(outBuffer, slots)
        return bytes
    }
    /** 写操作的独立返回码（默认成功）—— 用于断言「被拒时界面仍显示核心认可的值」。 */
    var themeWriteCode: Int = NrErrors.NR_ERR_OK

    /** 可编排的基色结构体（56 字节，直接给定）。 */
    var themeBaseBytes: ByteArray? = ByteArray(NR_THEME_BASE_SIZE)

    /** 取令牌时写回的 RGBA（4 字节）。 */
    var themeTokenRgba: ByteArray = byteArrayOf(0x22, 0x22, 0x22, 0xFF.toByte())

    /** 可编排的对比度读数（24 字节，直接给定）。 */
    var themeContrastBytes: ByteArray? = ByteArray(NR_THEME_CONTRAST_SIZE)

    /** 记录写入的基色（供断言 which / 值）。 */
    var lastThemeBaseWrite: Pair<Int, String>? = null

    /** 记录启用开关（供断言）。 */
    var lastThemeEnabled: Int? = null

    override fun getThemeBase(engine: Long, outBase: ByteArray): Int {
        calls.add("getThemeBase")
        if (v11Code != NrErrors.NR_ERR_OK) return v11Code
        themeBaseBytes?.let { it.copyInto(outBase, endIndex = minOf(it.size, outBase.size)) }
        return NrErrors.NR_ERR_OK
    }

    override fun setThemeBase(engine: Long, which: Int, value: ByteArray): Int {
        calls.add("setThemeBase")
        lastThemeBaseWrite = which to String(value, Charsets.UTF_8)
        return themeWriteCode
    }

    override fun setThemeEnabled(engine: Long, enabled: Int): Int {
        calls.add("setThemeEnabled")
        lastThemeEnabled = enabled
        return themeWriteCode
    }

    override fun getThemeToken(engine: Long, which: Int, dark: Int, outRgba: ByteArray): Int {
        calls.add("getThemeToken")
        if (v11Code != NrErrors.NR_ERR_OK) return v11Code
        themeTokenRgba.copyInto(outRgba, endIndex = minOf(themeTokenRgba.size, outRgba.size))
        return NrErrors.NR_ERR_OK
    }

    override fun getThemeContrast(engine: Long, outResult: ByteArray): Int {
        calls.add("getThemeContrast")
        if (v11Code != NrErrors.NR_ERR_OK) return v11Code
        themeContrastBytes?.let { it.copyInto(outResult, endIndex = minOf(it.size, outResult.size)) }
        return NrErrors.NR_ERR_OK
    }

    // ---------------- 背景 ----------------

    /** 可编排的背景字节池（`null` = 读失败）。 */
    var backgroundBytes: ByteArray? = ByteArray(0)

    /** 可编排的背景条目数（0 = 未设置）。 */
    var backgroundCount: Int = 0

    /** 记录最后一次设置的源路径 / 目标目录（供断言参数透传）。 */
    var lastBackgroundSet: Pair<String, String>? = null

    override fun getBackground(engine: Long, outBatch: ByteArray, outBuffer: LongArray): ByteArray? =
        backgroundResult("getBackground", outBatch, outBuffer)

    override fun setBackground(
        engine: Long,
        srcPath: ByteArray,
        destDir: ByteArray,
        fitMode: Int,
        scrimPermille: Int,
        outBatch: ByteArray,
        outBuffer: LongArray,
    ): ByteArray? {
        lastBackgroundSet = String(srcPath, Charsets.UTF_8) to String(destDir, Charsets.UTF_8)
        return backgroundResult("setBackground", outBatch, outBuffer)
    }

    override fun setBackgroundScrim(
        engine: Long,
        scrimPermille: Int,
        outBatch: ByteArray,
        outBuffer: LongArray,
    ): ByteArray? = backgroundResult("setBackgroundScrim", outBatch, outBuffer)

    override fun clearBackground(
        engine: Long,
        destDir: ByteArray,
        outBatch: ByteArray,
        outBuffer: LongArray,
    ): ByteArray? = backgroundResult("clearBackground", outBatch, outBuffer)

    /** 背景四个入口共用：写批次头 + 归还槽 + 返回字节池。 */
    private fun backgroundResult(call: String, outBatch: ByteArray, outBuffer: LongArray): ByteArray? {
        calls.add(call)
        if (v11Code != NrErrors.NR_ERR_OK) return null
        val bytes = backgroundBytes ?: return null
        val slots = slotsFor(bytes)
        val struct = structSlots(slots)
        val buf = writer(NR_BACKGROUND_BATCH_SIZE)
        buf.putInt(0, backgroundCount)
        buf.putInt(4, 0)
        buf.putLong(8, struct[0])
        buf.putLong(16, struct[1])
        buf.putLong(24, struct[2])
        copyInto(buf, outBatch)
        writeSlots(outBuffer, slots)
        return bytes
    }

}

internal class FakeReleaser(
    var code: Int = NrErrors.NR_ERR_OK,
) : NrReleaser {

    val calls = mutableListOf<Triple<Long, Long, Long>>()

    override fun release(ptr: Long, len: Long, capacity: Long): Int {
        calls.add(Triple(ptr, len, capacity))
        return code
    }
}

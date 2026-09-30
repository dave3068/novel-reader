package com.novelreader.android.core

import java.nio.ByteBuffer
import java.nio.ByteOrder

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
}

/** 记录归还调用的假归还器（单测专用）。 */
internal class FakeReleaser(
    var code: Int = NrErrors.NR_ERR_OK,
) : NrReleaser {

    val calls = mutableListOf<Triple<Long, Long, Long>>()

    override fun release(ptr: Long, len: Long, capacity: Long): Int {
        calls.add(Triple(ptr, len, capacity))
        return code
    }
}

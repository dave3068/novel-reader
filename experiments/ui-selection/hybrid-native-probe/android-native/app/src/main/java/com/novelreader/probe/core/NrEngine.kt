package com.novelreader.probe.core

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** Lifecycle and coroutine adapter around the shared C ABI. */
class NrEngine(private val maxWindowBytes: Long = 256L * 1024L) : AutoCloseable {
    private var handle: Long = NrCore.nativeCreate(maxWindowBytes)

    init {
        if (handle == 0L) throw NrException(NrErrorCodes.NR_ERR_INTERNAL, "nr_engine_create failed")
    }

    private fun ensureHandle(): Long = handle.takeIf { it != 0L }
        ?: throw NrException(NrErrorCodes.NR_ERR_ENGINE_DESTROYED, "engine is closed")

    private fun check(code: Int) {
        if (code != NrErrorCodes.NR_ERR_OK) {
            throw NrException(code, "shared core error code $code")
        }
    }

    private fun checkSubscription(value: Long): Long {
        if (value < 0L) throw NrException((-value).toInt(), "event subscription failed")
        return value
    }

    suspend fun openBook(bookId: Long, path: String? = null, totalBytes: Long = 0L) =
        withContext(Dispatchers.IO) {
            val bytes = path?.toByteArray(Charsets.UTF_8)
            check(NrCore.nativeOpenBook(ensureHandle(), bookId, bytes, totalBytes))
        }

    suspend fun readWindow(chapterIndex: Int, offset: Long, length: Int): ByteArray =
        withContext(Dispatchers.IO) {
            NrCore.nativeReadWindow(ensureHandle(), chapterIndex, offset, length)
                ?: throw NrException(NrErrorCodes.NR_ERR_INTERNAL, "read window failed")
        }

    /**
     * 打开**外部内容**（例如 SAF 选中的文件）：共享库只登记「单章、长度 = totalBytes」
     * 的坐标空间，字节由平台侧按窗口提供；页边界/位置/锚点语义仍由共享库裁定。
     */
    suspend fun openExternalBook(bookId: Long, totalBytes: Long) =
        withContext(Dispatchers.IO) {
            check(NrCore.nativeOpenExternalBook(ensureHandle(), bookId, totalBytes))
        }

    suspend fun getBookBatch(start: Long, count: Int): ByteArray =
        withContext(Dispatchers.IO) {
            NrCore.nativeGetBookBatch(ensureHandle(), start, count)
                ?: throw NrException(NrErrorCodes.NR_ERR_INTERNAL, "book batch failed")
        }

    suspend fun getChapterBatch(bookId: Long, start: Int, count: Int): ByteArray =
        withContext(Dispatchers.IO) {
            NrCore.nativeGetChapterBatch(ensureHandle(), bookId, start, count)
                ?: throw NrException(NrErrorCodes.NR_ERR_INTERNAL, "chapter batch failed")
        }

    suspend fun savePosition(bookId: Long, chapterIndex: Int, byteOffset: Long) =
        withContext(Dispatchers.IO) {
            check(NrCore.nativeSavePosition(ensureHandle(), bookId, chapterIndex, byteOffset))
        }

    suspend fun commitPageBoundary(
        chapterIndex: Int,
        startByte: Long,
        endByte: Long,
        layoutId: Long,
        lineCount: Int
    ) = withContext(Dispatchers.IO) {
        check(NrCore.nativeCommitPageBoundary(ensureHandle(), chapterIndex, startByte, endByte, layoutId, lineCount))
    }

    suspend fun invalidateLayout(layoutId: Long) = withContext(Dispatchers.IO) {
        check(NrCore.nativeInvalidateLayout(ensureHandle(), layoutId))
    }

    suspend fun subscribeEvents(): Long = withContext(Dispatchers.IO) {
        checkSubscription(NrCore.nativeSubscribeEvents(ensureHandle()))
    }

    suspend fun cancelRequest(requestId: Long) = withContext(Dispatchers.IO) {
        check(NrCore.nativeCancelRequest(ensureHandle(), requestId))
    }

    override fun close() {
        val old = handle
        handle = 0L
        if (old != 0L) NrCore.nativeDestroy(old)
    }
}

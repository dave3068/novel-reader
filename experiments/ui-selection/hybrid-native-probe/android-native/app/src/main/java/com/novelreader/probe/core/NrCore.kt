package com.novelreader.probe.core

/** Thin JNI declarations. Business semantics remain in rust-reading-core. */
object NrCore {
    init {
        System.loadLibrary("nr_jni")
    }

    external fun nativeCreate(maxWindowBytes: Long): Long
    external fun nativeDestroy(handle: Long)
    external fun nativeOpenBook(handle: Long, bookId: Long, pathUtf8: ByteArray?, totalBytes: Long): Int
    external fun nativeOpenExternalBook(handle: Long, bookId: Long, totalBytes: Long): Int
    external fun nativeReadWindow(handle: Long, chapterIndex: Int, offset: Long, length: Int): ByteArray?
    external fun nativeReleaseBuffer(): Int
    external fun nativeGetBookBatch(handle: Long, start: Long, count: Int): ByteArray?
    external fun nativeGetChapterBatch(handle: Long, bookId: Long, start: Int, count: Int): ByteArray?
    external fun nativeSavePosition(handle: Long, bookId: Long, chapterIndex: Int, byteOffset: Long): Int
    external fun nativeCommitPageBoundary(
        handle: Long,
        chapterIndex: Int,
        startByte: Long,
        endByte: Long,
        layoutId: Long,
        lineCount: Int
    ): Int
    external fun nativeInvalidateLayout(handle: Long, layoutId: Long): Int
    external fun nativeSubscribeEvents(handle: Long): Long
    external fun nativeCancelRequest(handle: Long, requestId: Long): Int

    /** Explicit ownership marker for the C/Rust output buffer contract. */
    fun releaseBuffer(): Int = nativeReleaseBuffer()
}

class NrException(val code: Int, message: String) : RuntimeException(message)

object NrErrorCodes {
    const val NR_ERR_OK = 0
    const val NR_ERR_INVALID_ARGUMENT = 1
    const val NR_ERR_OUT_OF_RANGE = 2
    const val NR_ERR_UTF8_BOUNDARY = 3
    const val NR_ERR_BOOK_NOT_FOUND = 4
    const val NR_ERR_NO_OPEN_BOOK = 5
    const val NR_ERR_IO = 6
    const val NR_ERR_CANCELLED = 7
    const val NR_ERR_LAYOUT_NOT_FOUND = 8
    const val NR_ERR_ENGINE_DESTROYED = 9
    const val NR_ERR_INTERNAL = 10
    const val NR_ERR_CHAPTER_OUT_OF_RANGE = 11
}

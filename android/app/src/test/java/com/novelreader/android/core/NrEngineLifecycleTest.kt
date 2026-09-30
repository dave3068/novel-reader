package com.novelreader.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

/** 句柄生命周期（创建 / 销毁 / 销毁后拒绝）、取消、以及 `null` 返回的错误码来源。 */
class NrEngineLifecycleTest {

    private val releaser = FakeReleaser()

    private fun engine(abi: FakeAbi): NrEngine =
        NrEngine.createWith(abi, NrAbiConstants.NR_DEFAULT_WINDOW_BYTES, null, null, releaser)

    @Test
    fun createUsesInjectedHandleAndWindowLimit() {
        val abi = FakeAbi()
        val engine = engine(abi)
        assertEquals(FakeAbi.HANDLE, engine.handle)
        assertTrue(engine.handle != 0L)
        assertFalse(engine.isDestroyed)
        assertEquals(listOf("engineCreate"), abi.calls)
    }

    @Test
    fun zeroHandleMeansCreateFailure() {
        val abi = FakeAbi().apply { engineCreateResult = 0L }
        val error = assertThrows(NrException::class.java) { engine(abi) }
        assertEquals(NrErrors.NR_ERR_INTERNAL, error.rawCode)
        assertEquals(NrErrorKind.INTERNAL, error.kind)
    }

    @Test
    fun destroyIsIdempotentAndReachesNativeOnce() {
        val abi = FakeAbi()
        val engine = engine(abi)

        assertEquals(NrErrors.NR_ERR_OK, engine.destroy())
        assertTrue(engine.isDestroyed)
        assertEquals(NrErrors.NR_ERR_ENGINE_DESTROYED, engine.destroy())
        assertEquals(1, abi.calls.count { it == "engineDestroy" })
    }

    @Test
    fun destroyFailureStillClosesTheHandle() {
        val abi = FakeAbi().apply { engineDestroyResult = NrErrors.NR_ERR_INVALID_ARGUMENT }
        val engine = engine(abi)

        assertEquals(NrErrors.NR_ERR_INVALID_ARGUMENT, engine.destroy())
        assertTrue(engine.isDestroyed)
        // 已置位：不再二次调用 native（宁可让后续调用失败，也不冒 use-after-free 风险）
        assertEquals(NrErrors.NR_ERR_ENGINE_DESTROYED, engine.destroy())
        assertEquals(1, abi.calls.count { it == "engineDestroy" })
    }

    @Test
    fun closeIsSilentAndIdempotent() {
        val abi = FakeAbi()
        val engine = engine(abi)

        engine.close()
        engine.close()
        assertTrue(engine.isDestroyed)
        assertEquals(1, abi.calls.count { it == "engineDestroy" })
    }

    @Test
    fun callsAfterDestroyNeverReachNative() {
        val abi = FakeAbi()
        val engine = engine(abi)
        engine.destroy()
        val before = abi.calls.size

        val attempts: List<() -> Unit> = listOf(
            { engine.openTxt(NrOpenRequest(NrAbiConstants.NR_SOURCE_KIND_FILE, 16L, ByteArray(4))) },
            { engine.closeBook(77L) },
            { engine.readWindow(0, 0L, 16L) },
            { engine.submitPageBoundary(NrPageBoundarySubmission(1, 1L, 0L, 0L, 16L, 0L, 16L)) },
            { engine.getPageBoundary(1L, 0, 0L) },
            { engine.savePosition(NrSavePositionRequest(0, 77L, 4L)) },
            { engine.getPosition(77L) },
            { engine.getChapters(77L, 0, 8) },
            { engine.createBookmark(NrBookmarkRequest(0, 77L, 4L)) },
            { engine.deleteBookmark(1L) },
            { engine.invalidateLayout(1L) },
            { engine.rebuildIndex(77L, 1L) },
            { engine.cancelRequest(1L) },
            { engine.lastError() },
        )
        for (attempt in attempts) {
            val error = assertThrows(NrException::class.java) { attempt() }
            assertEquals(NrErrors.NR_ERR_ENGINE_DESTROYED, error.rawCode)
            assertEquals(NrErrorKind.ENGINE_STATE, error.kind)
        }
        assertEquals(before, abi.calls.size)
    }

    @Test
    fun cancelRequestRejectsZeroRequestIdWithoutNative() {
        val abi = FakeAbi()
        val engine = engine(abi)

        val error = assertThrows(NrException::class.java) { engine.cancelRequest(0L) }
        assertEquals(NrErrors.NR_ERR_INVALID_ARGUMENT, error.rawCode)
        assertEquals(0, abi.calls.count { it == "cancelRequest" })

        engine.cancelRequest(7L)
        assertEquals(1, abi.calls.count { it == "cancelRequest" })
    }

    @Test
    fun cancelRequestPropagatesNativeCode() {
        val abi = FakeAbi().apply { cancelRequestCode = NrErrors.NR_ERR_NO_OPEN_BOOK }
        val error = assertThrows(NrException::class.java) { engine(abi).cancelRequest(3L) }
        assertEquals(NrErrors.NR_ERR_NO_OPEN_BOOK, error.rawCode)
        assertEquals(NrErrorKind.SESSION, error.kind)
    }

    @Test
    fun failedBufferCallFallsBackToLastErrorCode() {
        val abi = FakeAbi().apply {
            readWindowBytes = null
            lastErrorCode = NrErrors.NR_ERR_UTF8_BOUNDARY
        }
        val error = assertThrows(NrException::class.java) { engine(abi).readWindow(0, 12L, 16L) }
        assertEquals(NrErrors.NR_ERR_UTF8_BOUNDARY, error.rawCode)
        assertEquals(NrErrorKind.ENCODING, error.kind)
    }

    @Test
    fun failedBufferCallWithoutRecordedErrorIsInternalNotSuccess() {
        val abi = FakeAbi().apply {
            getChaptersBytes = null
            lastErrorCode = NrErrors.NR_ERR_OK
        }
        val error = assertThrows(NrException::class.java) { engine(abi).getChapters(77L, 0, 8) }
        assertEquals(NrErrors.NR_ERR_INTERNAL, error.rawCode)
        assertEquals(NrErrorKind.INTERNAL, error.kind)
    }

    @Test
    fun zeroLengthWindowIsEmptyArrayNotFailure() {
        val abi = FakeAbi().apply { readWindowBytes = ByteArray(0) }
        val result = engine(abi).readWindow(0, 100L, 0L)

        assertEquals(0, result.bytes.size)
        assertEquals(0L, result.window.len)
        assertTrue(result.buffer.isEmpty)
        // 空结果是安全 no-op 归还：不触碰 native
        assertEquals(NrErrors.NR_ERR_OK, result.buffer.release(releaser))
        assertTrue(releaser.calls.isEmpty())
    }

    @Test
    fun readWindowKeeps64BitOffsetsAndReleasesOnce() {
        val abi = FakeAbi().apply { readWindowBytes = ByteArray(4) }
        val result = engine(abi).readWindow(0, 9_007_199_254_740_993L, 4L)

        assertEquals(9_007_199_254_740_993L, result.window.offset)
        assertEquals(9_007_199_254_740_997L, result.window.endByte)
        assertEquals(4L, result.window.len)
        assertEquals(4L, result.buffer.len)
        assertEquals(4, result.bytes.size)

        val bufferReleaser = FakeReleaser()
        assertEquals(NrErrors.NR_ERR_OK, result.buffer.release(bufferReleaser))
        assertEquals(1, bufferReleaser.calls.size)
        assertEquals(NrErrors.NR_ERR_INVALID_ARGUMENT, result.buffer.release(bufferReleaser))
        assertEquals(1, bufferReleaser.calls.size)
    }

    @Test
    fun windowLengthMismatchIsRejected() {
        val abi = FakeAbi().apply {
            readWindowBytes = ByteArray(4)
            windowLenOverride = 8L
        }
        val error = assertThrows(NrException::class.java) { engine(abi).readWindow(0, 0L, 4L) }
        assertEquals(NrErrors.NR_ERR_INTERNAL, error.rawCode)
    }

    @Test
    fun positionStructSlotsMustMatchOutBuffer() {
        val abi = FakeAbi().apply {
            getPositionBytes = ByteArray(8)
            structSlotsOnly = longArrayOf(0xDEADL, 8L, 8L)
        }
        val error = assertThrows(NrException::class.java) { engine(abi).getPosition(77L) }
        assertEquals(NrErrors.NR_ERR_INTERNAL, error.rawCode)
    }

    @Test
    fun chapterBatchSlotsMustMatchOutBuffer() {
        val abi = FakeAbi().apply {
            getChaptersBytes = ByteArray(0)
            structSlotsOnly = longArrayOf(0x1L, 0L, 0L)
        }
        val error = assertThrows(NrException::class.java) { engine(abi).getChapters(77L, 0, 8) }
        assertEquals(NrErrors.NR_ERR_INTERNAL, error.rawCode)
    }

    @Test
    fun lastErrorReleasesMessageBufferImmediately() {
        val abi = FakeAbi().apply {
            lastErrorCode = NrErrors.NR_ERR_IO
            lastErrorMessage = "boom".toByteArray(Charsets.UTF_8)
        }
        val info = engine(abi).lastError()

        assertEquals(NrErrors.NR_ERR_IO, info.info.code)
        assertEquals(1L, info.info.hasMessage)
        assertEquals("boom", String(info.message, Charsets.UTF_8))
        assertEquals(NrErrors.NR_ERR_OK, info.releaseCode)
        assertEquals(1, releaser.calls.size)
        assertEquals(4L, releaser.calls[0].second)
    }

    @Test
    fun rebuildIndexCancelledIsPartialSuccessNotException() {
        val abi = FakeAbi().apply { rebuildIndexCode = NrErrors.NR_ERR_CANCELLED }
        val outcome = engine(abi).rebuildIndex(77L, 3L)

        assertTrue(outcome.cancelled)
        assertEquals(NrErrors.NR_ERR_CANCELLED, outcome.code)
        assertEquals(1L, outcome.result.partial)
        assertEquals(9L, outcome.result.contentIndexVersion)
    }

    @Test
    fun rebuildIndexHardFailureThrows() {
        val abi = FakeAbi().apply { rebuildIndexCode = NrErrors.NR_ERR_BOOK_NOT_FOUND }
        val error = assertThrows(NrException::class.java) { engine(abi).rebuildIndex(77L, 3L) }
        assertEquals(NrErrors.NR_ERR_BOOK_NOT_FOUND, error.rawCode)
    }

    @Test
    fun openTxtDecodesResultAndPropagatesCode() {
        val abi = FakeAbi()
        val opened = engine(abi).openTxt(
            NrOpenRequest(NrAbiConstants.NR_SOURCE_KIND_FILE, 4096L, "/tmp/book.txt".toByteArray()),
        )
        assertEquals(77L, opened.bookId)
        assertEquals(4096L, opened.totalBytes)
        assertEquals(5L, opened.contentIndexVersion)
        assertEquals(3L, opened.chapters)
        assertEquals(NrAbiConstants.NR_ENCODING_UTF8, opened.encoding)

        val failing = FakeAbi().apply { openTxtCode = NrErrors.NR_ERR_UTF8_BOUNDARY }
        val error = assertThrows(NrException::class.java) {
            engine(failing).openTxt(NrOpenRequest(NrAbiConstants.NR_SOURCE_KIND_FILE, 16L, ByteArray(4)))
        }
        assertEquals(NrErrors.NR_ERR_UTF8_BOUNDARY, error.rawCode)
    }

    @Test
    fun booleanOutParametersAreMapped() {
        val abi = FakeAbi()
        val engine = engine(abi)
        assertTrue(engine.savePosition(NrSavePositionRequest(0, 77L, 4L)))
        assertTrue(engine.deleteBookmark(1L))
        assertEquals(12L, engine.invalidateLayout(1L))
        assertEquals(99L, engine.createBookmark(NrBookmarkRequest(0, 77L, 4L)))
    }
}

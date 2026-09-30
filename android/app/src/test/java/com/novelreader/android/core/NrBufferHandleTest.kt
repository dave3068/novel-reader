package com.novelreader.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

/** 缓冲区归还语义：恰好一次、空凭据 no-op、失败码透传、槽位/长度一致性断言。 */
class NrBufferHandleTest {

    private fun handle(ptr: Long = 0x1000L, len: Long = 32L, capacity: Long = 64L): NrBufferHandle =
        NrBufferHandle.fromSlots(longArrayOf(ptr, len, capacity))

    @Test
    fun releasesExactlyOnce() {
        val releaser = FakeReleaser()
        val buffer = handle()

        assertEquals(NrErrors.NR_ERR_OK, buffer.release(releaser))
        assertTrue(buffer.released)
        assertEquals(1, releaser.calls.size)
        assertEquals(Triple(0x1000L, 32L, 64L), releaser.calls[0])

        // 重复归还：与 C 层同码，且不再触碰 native（不冒二次释放风险）。
        assertEquals(NrErrors.NR_ERR_INVALID_ARGUMENT, buffer.release(releaser))
        assertEquals(1, releaser.calls.size)
    }

    @Test
    fun emptyHandleIsSafeNoOp() {
        val releaser = FakeReleaser()
        val buffer = NrBufferHandle.fromSlots(longArrayOf(0L, 0L, 0L))

        assertTrue(buffer.isEmpty)
        assertEquals(NrErrors.NR_ERR_OK, buffer.release(releaser))
        assertTrue(buffer.released)
        assertTrue(releaser.calls.isEmpty())

        // 空凭据也遵循「只归还一次」：第二次是重复归还。
        assertEquals(NrErrors.NR_ERR_INVALID_ARGUMENT, buffer.release(releaser))
    }

    @Test
    fun nativeFailureKeepsHandleUnreleased() {
        val releaser = FakeReleaser(code = NrErrors.NR_ERR_IO)
        val buffer = handle()

        assertEquals(NrErrors.NR_ERR_IO, buffer.release(releaser))
        assertFalse(buffer.released)

        releaser.code = NrErrors.NR_ERR_OK
        assertEquals(NrErrors.NR_ERR_OK, buffer.release(releaser))
        assertTrue(buffer.released)
        assertEquals(2, releaser.calls.size)
    }

    @Test
    fun releaseOrThrowSurfacesFailureCode() {
        val releaser = FakeReleaser(code = NrErrors.NR_ERR_INVALID_ARGUMENT)
        val error = assertThrows(NrException::class.java) { handle().releaseOrThrow(releaser) }
        assertEquals(NrErrors.NR_ERR_INVALID_ARGUMENT, error.rawCode)
        assertEquals(NrErrorKind.CALLER, error.kind)
    }

    @Test
    fun slotsMustCarryThreeValues() {
        val error = assertThrows(NrException::class.java) { NrBufferHandle.fromSlots(longArrayOf(1L, 2L)) }
        assertEquals(NrErrors.NR_ERR_INVALID_ARGUMENT, error.rawCode)
    }

    @Test
    fun structSlotsMustMatchOutBuffer() {
        val buffer = handle()
        // 一致：不抛
        NrBufferHandle.requireSameSlots(buffer, 0x1000L, 32L, 64L)

        val error = assertThrows(NrException::class.java) {
            NrBufferHandle.requireSameSlots(buffer, 0x1000L, 32L, 128L)
        }
        assertEquals(NrErrors.NR_ERR_INTERNAL, error.rawCode)
    }

    @Test
    fun lengthMustMatchSelfReportedLength() {
        NrBufferHandle.requireLenMatches(handle(len = 32L), 32L)
        val error = assertThrows(NrException::class.java) { NrBufferHandle.requireLenMatches(handle(), 64L) }
        assertEquals(NrErrors.NR_ERR_INTERNAL, error.rawCode)
    }
}

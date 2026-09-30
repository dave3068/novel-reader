package com.novelreader.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

/** 错误码边界与「错误码 → 业务分类」映射（不解析错误文本）。 */
class NrErrorMappingTest {

    @Test
    fun frozenCodesAreZeroToElevenInOrder() {
        assertEquals(12, NrErrors.ALL.size)
        assertEquals((0..11).toList(), NrErrors.ALL)
        assertEquals(NrErrors.NR_ERR_CHAPTER_OUT_OF_RANGE, NrErrors.MAX_FROZEN_CODE)
        assertTrue(NrErrors.ALL.all { NrErrors.isFrozen(it) })
    }

    @Test
    fun everyFrozenCodeMapsToItsOwnBusinessKind() {
        val expected = mapOf(
            NrErrors.NR_ERR_OK to NrErrorKind.NONE,
            NrErrors.NR_ERR_INVALID_ARGUMENT to NrErrorKind.CALLER,
            NrErrors.NR_ERR_OUT_OF_RANGE to NrErrorKind.RANGE,
            NrErrors.NR_ERR_UTF8_BOUNDARY to NrErrorKind.ENCODING,
            NrErrors.NR_ERR_BOOK_NOT_FOUND to NrErrorKind.NOT_FOUND,
            NrErrors.NR_ERR_NO_OPEN_BOOK to NrErrorKind.SESSION,
            NrErrors.NR_ERR_IO to NrErrorKind.IO,
            NrErrors.NR_ERR_CANCELLED to NrErrorKind.CANCELLED,
            NrErrors.NR_ERR_LAYOUT_NOT_FOUND to NrErrorKind.LAYOUT,
            NrErrors.NR_ERR_ENGINE_DESTROYED to NrErrorKind.ENGINE_STATE,
            NrErrors.NR_ERR_INTERNAL to NrErrorKind.INTERNAL,
            NrErrors.NR_ERR_CHAPTER_OUT_OF_RANGE to NrErrorKind.CHAPTER,
        )
        // 12 项错误码必须各自映射到不同分类（不得两项合并后丢失可区分性）。
        assertEquals(12, expected.size)
        assertEquals(12, expected.values.toSet().size)
        for ((code, kind) in expected) {
            assertEquals("code=$code", kind, NrErrorKind.of(code))
        }
    }

    @Test
    fun unknownCodeIsNeverSuccess() {
        for (raw in listOf(12, 13, 77, -1, Int.MIN_VALUE, Int.MAX_VALUE)) {
            val kind = NrErrorKind.of(raw)
            assertEquals("raw=$raw", NrErrorKind.UNKNOWN, kind)
            assertNotEquals(NrErrorKind.NONE, kind)
            assertFalse(NrErrors.isFrozen(raw))
        }
    }

    @Test
    fun exceptionCarriesCodeAndKind() {
        val error = NrException(NrErrors.NR_ERR_IO)
        assertEquals(NrErrors.NR_ERR_IO, error.rawCode)
        assertEquals(NrErrorKind.IO, error.kind)
        assertTrue(error.message!!.contains("NR_ERR_6"))
        assertFalse(error.isCancelled)

        val cancelled = NrException(NrErrors.NR_ERR_CANCELLED)
        assertTrue(cancelled.isCancelled)
        assertEquals(NrErrorKind.CANCELLED, cancelled.kind)
    }

    @Test
    fun requireOkThrowsOnlyForFailure() {
        assertEquals(NrErrors.NR_ERR_OK, NrErrors.NR_ERR_OK.requireOk())
        val error = assertThrows(NrException::class.java) { NrErrors.NR_ERR_BOOK_NOT_FOUND.requireOk() }
        assertEquals(NrErrors.NR_ERR_BOOK_NOT_FOUND, error.rawCode)
        assertEquals(NrErrorKind.NOT_FOUND, error.kind)
    }
}

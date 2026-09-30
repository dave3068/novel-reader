package com.novelreader.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

/** 64 位 id 的字符串传输：绕过 JSON 数字（2^53）截断。 */
class NrWireIdsTest {

    @Test
    fun wireStringRoundTrips64BitIds() {
        val ids = listOf(
            0L,
            1L,
            NrWireIds.JSON_SAFE_INTEGER,
            NrWireIds.JSON_SAFE_INTEGER + 1,
            9_007_199_254_740_993L,
            1_700_000_000_123_456_789L,
            Long.MAX_VALUE,
            Long.MIN_VALUE,
        )
        for (id in ids) {
            assertEquals("id=$id", id, NrWireIds.fromWireString(NrWireIds.toWireString(id)))
        }
    }

    @Test
    fun stringTransportSurvivesTheDoublePrecisionBoundary() {
        // 若走 JSON 数字 / Double，这个 id 会被截断成 9007199254740992。
        val id = 9_007_199_254_740_993L
        val asDouble = id.toDouble().toLong()
        assertFalse("前提：该 id 超出 double 精确表示范围", asDouble == id)

        val wire = NrWireIds.toWireString(id)
        assertEquals("9007199254740993", wire)
        assertEquals(id, NrWireIds.fromWireString(wire))
        assertTrue(NrWireIds.exceedsJsonSafeInteger(id))
        assertFalse(NrWireIds.exceedsJsonSafeInteger(NrWireIds.JSON_SAFE_INTEGER))
    }

    @Test
    fun invalidWireTextIsRejectedAsCallerError() {
        for (text in listOf("", "   ", "12.5", "0x10", "abc", "1e3")) {
            val error = assertThrows(NrException::class.java) { NrWireIds.fromWireString(text) }
            assertEquals("text=$text", NrErrors.NR_ERR_INVALID_ARGUMENT, error.rawCode)
            assertEquals(NrErrorKind.CALLER, error.kind)
        }
        // 允许两端空白（传输层可能带空白），内部仍必须精确解析。
        assertEquals(9007199254740993L, NrWireIds.fromWireString(" 9007199254740993 "))
    }
}

package com.novelreader.probe

import com.novelreader.probe.core.NrErrorCodes
import com.novelreader.probe.core.NrException
import com.novelreader.probe.reader.Utf8OffsetMapper
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class BoundaryMathTest {
    @Test
    fun pageEndsAreLegalAndChainIsContinuous() {
        val text = "第一段。第二段，含 emoji 😀。第三段。"
        val starts = listOf(0, 3, 9, text.toByteArray(Charsets.UTF_8).size.toLong())
        for (offset in starts) assertTrue(Utf8OffsetMapper.isLegalBoundary(text, offset))
        val first = Utf8OffsetMapper.byteOffsetForCharIndex(text, 4)
        val second = Utf8OffsetMapper.byteOffsetForCharIndex(text, 10)
        assertEquals(first, Utf8OffsetMapper.byteOffsetForCharIndex(text, 4))
        assertTrue(second > first)
    }

    @Test
    fun errorCodesMapToNrException() {
        val error = NrException(NrErrorCodes.NR_ERR_CANCELLED, "cancelled")
        assertEquals(NrErrorCodes.NR_ERR_CANCELLED, error.code)
        assertTrue(error is RuntimeException)
    }
}

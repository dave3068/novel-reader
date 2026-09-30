package com.novelreader.probe

import com.novelreader.probe.reader.Utf8OffsetMapper
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class Utf8OffsetMapperTest {
    @Test
    fun mapsChineseAndEmojiWithoutSplittingCodePoints() {
        val text = "中文，A😀组合"
        val bytes = text.toByteArray(Charsets.UTF_8)
        assertEquals(bytes.size.toLong(), Utf8OffsetMapper.byteOffsetForCharIndex(text, text.length))
        val emojiIndex = text.indexOf("😀")
        val beforeEmoji = Utf8OffsetMapper.byteOffsetForCharIndex(text, emojiIndex)
        assertEquals(emojiIndex, Utf8OffsetMapper.charIndexForByteOffset(text, beforeEmoji))
        assertEquals(emojiIndex + 2, Utf8OffsetMapper.charIndexForByteOffset(text, beforeEmoji + 4))
        assertTrue(Utf8OffsetMapper.isLegalBoundary(text, beforeEmoji))
        assertTrue(Utf8OffsetMapper.isLegalBoundary(text, beforeEmoji + 4))
    }

    @Test
    fun floorsAnIncompleteUtf8ByteOffset() {
        val text = "你好"
        val firstEnd = 3L
        assertEquals(1, Utf8OffsetMapper.charIndexForByteOffset(text, firstEnd + 1))
        assertTrue(!Utf8OffsetMapper.isLegalBoundary(text, firstEnd + 1))
    }
}

package com.novelreader.android.core

import java.nio.ByteBuffer
import java.nio.ByteOrder
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

/** 结构体字节布局 / 边界映射（u32 不截断、64 位不退化）与章节批次布局解码。 */
class NrStructCodecTest {

    @Test
    fun structSizesMatchFrozenLayout() {
        // 与 android/jni/nr_jni.c 的 _Static_assert 和 README §4 表格一致。
        assertEquals(56, NrOpenResult.STRUCT_SIZE)
        assertEquals(48, NrWindow.STRUCT_SIZE)
        assertEquals(80, NrBoundaryResult.STRUCT_SIZE)
        assertEquals(80, NrPositionResult.STRUCT_SIZE)
        assertEquals(40, NrChapterBatch.STRUCT_SIZE)
        assertEquals(40, NrRebuildResult.STRUCT_SIZE)
        assertEquals(32, NrErrorInfo.STRUCT_SIZE)
        assertEquals(40, NrChapterEntry.STRUCT_SIZE)
    }

    @Test
    fun windowDecodesU32FieldsWithoutTruncation() {
        val buf = ByteBuffer.allocate(NrWindow.STRUCT_SIZE).order(ByteOrder.LITTLE_ENDIAN)
        buf.putLong(0, 1L shl 40)
        buf.putLong(8, Long.MAX_VALUE)
        buf.putLong(16, 1L)
        buf.putLong(24, Long.MAX_VALUE)
        buf.putInt(32, -1) // u32 0xFFFFFFFF：不得解成 -1
        buf.putInt(36, 2)
        buf.putInt(40, 4)
        buf.putInt(44, 1)

        val window = NrWindow.decode(buf.array())
        assertEquals(1L shl 40, window.offset)
        assertEquals(Long.MAX_VALUE, window.endByte)
        assertEquals(4_294_967_295L, window.len)
        assertEquals(2L, window.droppedLeading)
        assertEquals(4L, window.trimmedTail)
        assertEquals(1L, window.eof)
    }

    @Test
    fun openResultReservedFieldIsIgnoredAndU32StaysUnsigned() {
        val buf = ByteBuffer.allocate(NrOpenResult.STRUCT_SIZE).order(ByteOrder.LITTLE_ENDIAN)
        buf.putLong(0, 19L)
        buf.putLong(8, -1L)
        buf.putLong(16, 3L)
        buf.putInt(24, 1)
        buf.putInt(28, 5)
        buf.putInt(32, 1000)
        buf.putInt(36, 999)
        buf.putInt(40, 1)
        buf.putInt(44, -1) // u32 chapters
        buf.putInt(48, 1)
        buf.putInt(52, 12345) // reserved0 必须为 0，解码忽略

        val opened = NrOpenResult.decode(buf.array())
        assertEquals(19L, opened.bookId)
        assertEquals(-1L, opened.totalBytes)
        assertEquals(4_294_967_295L, opened.chapters)
        assertEquals(1000L, opened.encodingConfidence)
        assertEquals(1L, opened.indexFromCache)
    }

    @Test
    fun errorInfoDecodesSignedCodeAndMessageSlots() {
        val buf = ByteBuffer.allocate(NrErrorInfo.STRUCT_SIZE).order(ByteOrder.LITTLE_ENDIAN)
        buf.putInt(0, NrErrors.NR_ERR_CHAPTER_OUT_OF_RANGE)
        buf.putInt(4, 1)
        buf.putLong(8, 0x2000L)
        buf.putLong(16, 10L)
        buf.putLong(24, 16L)

        val info = NrErrorInfo.decode(buf.array())
        assertEquals(NrErrors.NR_ERR_CHAPTER_OUT_OF_RANGE, info.code)
        assertEquals(1L, info.hasMessage)
        assertEquals(0x2000L, info.messagePtr)
        assertEquals(10L, info.messageLen)
        assertEquals(16L, info.messageCapacity)
    }

    @Test
    fun wrongStructSizeIsInternalError() {
        val error = assertThrows(NrException::class.java) { NrOpenResult.decode(ByteArray(55)) }
        assertEquals(NrErrors.NR_ERR_INTERNAL, error.rawCode)
    }

    @Test
    fun boundaryResultDecodesEightU64Fields() {
        val buf = ByteBuffer.allocate(NrBoundaryResult.STRUCT_SIZE).order(ByteOrder.LITTLE_ENDIAN)
        buf.putInt(0, NrAbiConstants.NR_BOUNDARY_COMMITTED.toInt())
        buf.putInt(4, NrAbiConstants.NR_REJECT_NOT_ADVANCING.toInt())
        buf.putInt(8, NrErrors.NR_ERR_OK)
        buf.putInt(12, 0)
        buf.putLong(16, 1L)
        buf.putLong(24, 2L)
        buf.putLong(32, 3L)
        buf.putLong(40, 4L)
        buf.putLong(48, 5L)
        buf.putLong(56, 6L)
        buf.putLong(64, 7L)
        buf.putLong(72, 8L)

        val result = NrBoundaryResult.decode(buf.array())
        assertEquals(NrAbiConstants.NR_BOUNDARY_COMMITTED, result.status)
        assertEquals(NrAbiConstants.NR_REJECT_NOT_ADVANCING, result.rejection)
        assertEquals(1L, result.layoutId)
        assertEquals(8L, result.pageCount)
    }

    @Test
    fun chapterBatchDecodesEntriesAndTitles() {
        val bytes = chapterBatchBytes(listOf("第一章 起", "第二章 承"))
        val batch = ByteBuffer.allocate(NrChapterBatch.STRUCT_SIZE).order(ByteOrder.LITTLE_ENDIAN).apply {
            putInt(0, 2)
            putInt(4, 7)
            putInt(8, 12)
            putInt(12, 0)
            putLong(16, 0x3000L)
            putLong(24, bytes.size.toLong())
            putLong(32, bytes.size.toLong())
        }.array()

        val decoded = NrChapterBatch.decode(batch)
        assertEquals(2L, decoded.count)
        assertEquals(12L, decoded.totalChapters)
        assertEquals(0x3000L, decoded.bytesPtr)

        val entries = NrChapterCodec.entries(bytes, decoded.count)
        assertEquals(2, entries.size)
        assertEquals(0L, entries[0].sequence)
        assertEquals(1L, entries[0].ruleId)
        assertEquals(0L, entries[0].startByte)
        assertEquals(100L, entries[0].endByte)
        assertEquals(7L, entries[0].indexVersion)
        assertEquals(100L, entries[1].startByte)
        assertEquals("第一章 起", String(NrChapterCodec.title(bytes, entries[0]), Charsets.UTF_8))
        assertEquals("第二章 承", String(NrChapterCodec.title(bytes, entries[1]), Charsets.UTF_8))
    }

    @Test
    fun chapterCodecRejectsOutOfBoundsLayout() {
        val bytes = chapterBatchBytes(listOf("第一章"))
        val tooMany = assertThrows(NrException::class.java) { NrChapterCodec.entries(bytes, 99L) }
        assertEquals(NrErrors.NR_ERR_INTERNAL, tooMany.rawCode)

        val entry = NrChapterCodec.entries(bytes, 1L)[0]
        val outOfRange = NrChapterEntry(
            sequence = entry.sequence,
            ruleId = entry.ruleId,
            startByte = entry.startByte,
            endByte = entry.endByte,
            titleOffset = bytes.size.toLong() + 1,
            titleLen = 4L,
            indexVersion = entry.indexVersion,
        )
        val error = assertThrows(NrException::class.java) { NrChapterCodec.title(bytes, outOfRange) }
        assertEquals(NrErrors.NR_ERR_INTERNAL, error.rawCode)
    }

    @Test
    fun chapterCodecAcceptsEmptyBatch() {
        assertTrue(NrChapterCodec.entries(ByteArray(0), 0L).isEmpty())
    }

    private fun chapterBatchBytes(titles: List<String>): ByteArray {
        val encoded = titles.map { it.toByteArray(Charsets.UTF_8) }
        val header = titles.size * NrChapterEntry.STRUCT_SIZE
        val out = ByteArray(header + encoded.sumOf { it.size })
        val buf = ByteBuffer.wrap(out).order(ByteOrder.LITTLE_ENDIAN)
        var offset = header
        encoded.forEachIndexed { index, title ->
            val base = index * NrChapterEntry.STRUCT_SIZE
            buf.putInt(base, index)
            buf.putInt(base + 4, 1)
            buf.putLong(base + 8, index * 100L)
            buf.putLong(base + 16, (index + 1) * 100L)
            buf.putInt(base + 24, offset)
            buf.putInt(base + 28, title.size)
            buf.putInt(base + 32, 7)
            System.arraycopy(title, 0, out, offset, title.size)
            offset += title.size
        }
        return out
    }
}

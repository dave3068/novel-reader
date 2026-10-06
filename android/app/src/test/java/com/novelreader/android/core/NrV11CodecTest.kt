package com.novelreader.android.core

import java.nio.ByteBuffer
import java.nio.ByteOrder
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * V1.1 四组功能的出参解码（分类 / 阅读历史 / 配色 / 背景）。
 *
 * 这些测试只钉**布局与边界**（与 C 层 `_Static_assert`、`abi_layout` 同源），
 * 不判断业务语义 —— 语义（排序 / 派生 / 对比度 / 时间格式化）全在 Rust 核心。
 */
class NrV11CodecTest {

    private fun le(size: Int): ByteBuffer = ByteBuffer.allocate(size).order(ByteOrder.LITTLE_ENDIAN)

    /** 把若干字符串按顺序拼成 UTF-8 字节池，返回 (池, 各段偏移)。 */
    private fun pool(vararg parts: String): Pair<ByteArray, List<Int>> {
        var offset = 0
        val offsets = parts.map { part ->
            val at = offset
            offset += part.toByteArray(Charsets.UTF_8).size
            at
        }
        val bytes = ByteArray(offset)
        var cursor = 0
        parts.forEach { part ->
            val raw = part.toByteArray(Charsets.UTF_8)
            raw.copyInto(bytes, cursor)
            cursor += raw.size
        }
        return bytes to offsets
    }

    @Test
    fun structSizesMatchFrozenLayout() {
        // 与 android/jni/nr_jni.c 的 _Static_assert 和 abi_layout 的 EXPECTED_SIZES 一致。
        assertEquals(48, NrCategoryEntry.STRUCT_SIZE)
        assertEquals(32, NrCategoryBatch.STRUCT_SIZE)
        assertEquals(72, NrHistoryEntry.STRUCT_SIZE)
        assertEquals(32, NrHistoryBatch.STRUCT_SIZE)
        assertEquals(56, NrThemeBase.STRUCT_SIZE)
        assertEquals(24, NrThemeContrastResult.STRUCT_SIZE)
        assertEquals(48, NrBackgroundEntry.STRUCT_SIZE)
        assertEquals(32, NrBackgroundBatch.STRUCT_SIZE)
        assertEquals(16, NrBookCategoryEntry.STRUCT_SIZE)
        assertEquals(32, NrBookCategoryBatch.STRUCT_SIZE)
        assertEquals(32, NrIdList.STRUCT_SIZE)
    }

    @Test
    fun categoryBatchDecodesIdsCountsAndUtf8Names() {
        val (nameBytes, nameOffsets) = pool("科幻", "三体")
        val header = le(NrCategoryEntry.STRUCT_SIZE * 2)
        // 第一条：根下（parent = -1），直属 2 / 合计 5，深度 0。
        header.putLong(0, 10L)
        header.putLong(8, NrCategoryEntry.PARENT_NONE)
        header.putLong(16, 2L)
        header.putLong(24, 5L)
        header.putInt(32, 0)
        header.putInt(36, NrCategoryEntry.STRUCT_SIZE * 2 + nameOffsets[0])
        header.putInt(40, "科幻".toByteArray(Charsets.UTF_8).size)
        // 第二条：子分类（parent = 10），深度 1。
        val second = NrCategoryEntry.STRUCT_SIZE
        header.putLong(second, 11L)
        header.putLong(second + 8, 10L)
        header.putLong(second + 16, 1L)
        header.putLong(second + 24, 1L)
        header.putInt(second + 32, 1)
        header.putInt(second + 36, NrCategoryEntry.STRUCT_SIZE * 2 + nameOffsets[1])
        header.putInt(second + 40, "三体".toByteArray(Charsets.UTF_8).size)

        val bytes = header.array() + nameBytes
        val entries = NrCategoryCodec.entries(bytes, 2L)
        assertEquals(2, entries.size)
        assertEquals(10L, entries[0].id)
        assertEquals(NrCategoryEntry.PARENT_NONE, entries[0].parentId)
        assertEquals(5L, entries[0].totalBookCount)
        assertEquals(0L, entries[0].depth)
        assertEquals("科幻", NrCategoryCodec.name(bytes, entries[0]))
        assertEquals(10L, entries[1].parentId)
        assertEquals(1L, entries[1].depth)
        assertEquals("三体", NrCategoryCodec.name(bytes, entries[1]))
    }

    @Test
    fun historyBatchDecodesThreeSegmentsAndCarriesFormattedLabel() {
        // 池里每行三段：书名 · 文件名 · 时间标签（布局纪律：三段各自有 offset/len）。
        val (textBytes, offsets) = pool("三体", "a.txt", "14:32")
        val header = le(NrHistoryEntry.STRUCT_SIZE)
        header.putLong(0, 7L)
        header.putLong(8, 3L)
        header.putLong(16, 1_791_264_000L)
        header.putLong(24, 4L)
        header.putLong(32, 128L)
        header.putLong(40, 0L)
        header.putInt(48, NrHistoryEntry.STRUCT_SIZE + offsets[0])
        header.putInt(52, "三体".toByteArray(Charsets.UTF_8).size)
        header.putInt(56, NrHistoryEntry.STRUCT_SIZE + offsets[1])
        header.putInt(60, "a.txt".toByteArray(Charsets.UTF_8).size)
        header.putInt(64, NrHistoryEntry.STRUCT_SIZE + offsets[2])
        header.putInt(68, "14:32".toByteArray(Charsets.UTF_8).size)

        val bytes = header.array() + textBytes
        val entries = NrHistoryCodec.entries(bytes, 1L)
        assertEquals(1, entries.size)
        assertEquals("三体", NrHistoryCodec.title(bytes, entries[0]))
        assertEquals("a.txt", NrHistoryCodec.fileLabel(bytes, entries[0]))
        // 标签是**核心格式化的成品文本**，不是原始 Unix 秒 —— 界面不得自己格式化。
        assertEquals("14:32", NrHistoryCodec.timeLabel(bytes, entries[0]))
        assertFalse(
            "时间标签不得是时间戳数字串",
            NrHistoryCodec.timeLabel(bytes, entries[0]).toLongOrNull() != null,
        )
    }

    @Test
    fun historyBatchDecodesTotalIndependentlyOfCount() {
        val (textBytes, offsets) = pool("甲", "x.txt", "09:05")
        val header = le(NrHistoryEntry.STRUCT_SIZE)
        header.putInt(48, NrHistoryEntry.STRUCT_SIZE + offsets[0])
        header.putInt(52, "甲".toByteArray(Charsets.UTF_8).size)
        header.putInt(56, NrHistoryEntry.STRUCT_SIZE + offsets[1])
        header.putInt(60, "x.txt".toByteArray(Charsets.UTF_8).size)
        header.putInt(64, NrHistoryEntry.STRUCT_SIZE + offsets[2])
        header.putInt(68, "09:05".toByteArray(Charsets.UTF_8).size)

        val batchHeader = le(NrHistoryBatch.STRUCT_SIZE)
        batchHeader.putInt(0, 1) // count：本批返回 1 条
        batchHeader.putInt(4, 9) // total：全部 9 条（不受 limit 影响）
        val batch = NrHistoryBatch.decode(batchHeader.array())
        assertEquals(1L, batch.count)
        assertEquals(9L, batch.total)

        assertEquals(1, NrHistoryCodec.entries(header.array() + textBytes, batch.count).size)
    }

    @Test
    fun themeBaseDecodesFourSlotsAndTreatsZeroSlotAsUnset() {
        val buf = le(NrThemeBase.STRUCT_SIZE)
        buf.putInt(0, NrThemeBase.STRUCT_SIZE)
        buf.putInt(4, 1) // enabled
        "#222222".toByteArray(Charsets.UTF_8).copyInto(buf.array(), 8)
        // background / primary / accent 留全 0 ⇒ 未设置。

        val base = NrThemeBase.decode(buf.array())
        assertTrue(base.enabled)
        assertEquals("#222222", base.foreground)
        assertEquals("", base.background)
        assertEquals("", base.primary)
        assertEquals("", base.accent)
    }

    @Test
    fun themeBaseDecodesDisabledStateWithAllSlotsUnset() {
        val buf = le(NrThemeBase.STRUCT_SIZE)
        buf.putInt(0, NrThemeBase.STRUCT_SIZE)
        val base = NrThemeBase.decode(buf.array())
        assertFalse(base.enabled)
        assertEquals("", base.foreground)
    }

    @Test
    fun themeContrastDecodesIntegerRatioAndPassFlag() {
        val buf = le(NrThemeContrastResult.STRUCT_SIZE)
        buf.putInt(0, NrThemeContrastResult.STRUCT_SIZE)
        buf.putInt(4, 454) // 4.54 : 1
        buf.putInt(8, 1)
        buf.putInt(12, 0)

        val result = NrThemeContrastResult.decode(buf.array())
        assertEquals(454L, result.ratioX100)
        assertTrue(result.passes)
        assertFalse(result.worstInDark)
    }

    @Test
    fun backgroundBatchDecodesRelativeFileNameAndAlpha() {
        val (nameBytes, offsets) = pool("bg-1791264415-42.png")
        val header = le(NrBackgroundEntry.STRUCT_SIZE)
        header.putLong(0, 1024L)
        header.putLong(8, 600L)
        header.putLong(16, 0L)
        header.put(32, 153.toByte()) // scrim_alpha：核心已从千分比换算
        header.putInt(36, nameBytes.size)
        header.putInt(40, NrBackgroundEntry.STRUCT_SIZE + offsets[0])

        val bytes = header.array() + nameBytes
        val entry = NrBackgroundCodec.entry(bytes, 1L)
        assertEquals(600L, entry!!.scrimPermille)
        assertEquals(153, entry.scrimAlpha)
        val name = NrBackgroundCodec.fileName(bytes, entry)
        assertEquals("bg-1791264415-42.png", name)
        // **只回相对名**（R15）：不得含路径分隔符。
        assertFalse(name.contains('/'))
        assertFalse(name.contains('\\'))
    }

    @Test
    fun backgroundBatchAbsentYieldsNullAndEmptyName() {
        val batch = NrBackgroundBatch.decode(le(NrBackgroundBatch.STRUCT_SIZE).array())
        assertEquals(0L, batch.count)
        val entry = NrBackgroundCodec.entry(ByteArray(0), batch.count)
        assertNull(entry)
        assertEquals("", NrBackgroundCodec.fileName(ByteArray(0), entry))
    }

    @Test
    fun outOfRangeSlicesAreRejectedInsteadOfSilentlyTruncated() {
        // 布局错位属于内部协议违例：必须拒绝，不能静默截成乱码。
        val tooShort = assertThrows(NrException::class.java) {
            NrCategoryCodec.entries(ByteArray(10), 1L)
        }
        assertEquals(NrErrors.NR_ERR_INTERNAL, tooShort.rawCode)

        val negative = assertThrows(NrException::class.java) {
            NrCategoryCodec.entries(ByteArray(48), -1L)
        }
        assertEquals(NrErrors.NR_ERR_INTERNAL, negative.rawCode)

        val badBatchSize = assertThrows(NrException::class.java) {
            NrCategoryBatch.decode(ByteArray(31))
        }
        assertEquals(NrErrors.NR_ERR_INTERNAL, badBatchSize.rawCode)

        val badHistory = assertThrows(NrException::class.java) {
            NrHistoryCodec.entries(ByteArray(71), 1L)
        }
        assertEquals(NrErrors.NR_ERR_INTERNAL, badHistory.rawCode)

        // 文本越界：偏移指向批次之外。
        val outOfBounds = assertThrows(NrException::class.java) {
            NrBatchText.slice(ByteArray(8), 6L, 4L, "测试")
        }
        assertEquals(NrErrors.NR_ERR_INTERNAL, outOfBounds.rawCode)

        // 背景条目数只能是 0 或 1。
        val badCount = assertThrows(NrException::class.java) {
            NrBackgroundCodec.entry(ByteArray(48), 2L)
        }
        assertEquals(NrErrors.NR_ERR_INTERNAL, badCount.rawCode)
    }

    @Test
    fun u32FieldsDoNotTruncateAtSignBoundary() {
        // 分类 id 为 i64，但计数类字段是 u32：0xFFFFFFFF 不得解成 -1。
        val buf = le(NrCategoryEntry.STRUCT_SIZE)
        buf.putLong(0, Long.MAX_VALUE)
        buf.putInt(32, -1) // depth = 0xFFFFFFFF
        buf.putInt(40, -1) // name_len = 0xFFFFFFFF

        val entry = NrCategoryCodec.entries(buf.array(), 1L).single()
        assertEquals(Long.MAX_VALUE, entry.id)
        assertEquals(0xFFFFFFFFL, entry.depth)
        assertEquals(0xFFFFFFFFL, entry.nameLen)
    }

    @Test
    fun bookCategoryBatchDecodesMapAndKeepsUncategorizedSentinel() {
        // 两行：一本归入 10，一本未分类（-1）。行内无字符串。
        val buf = le(NrBookCategoryEntry.STRUCT_SIZE * 2)
        buf.putLong(0, 7L)
        buf.putLong(8, 10L)
        buf.putLong(16, 8L)
        buf.putLong(24, NrBookCategoryEntry.UNCATEGORIZED)

        val grouped = NrBookCategoryCodec.byBookId(buf.array(), 2L)
        assertEquals(2, grouped.size)
        assertEquals(10L, grouped[7L])
        // 未分类是 **-1 哨兵** —— 界面据此判断，不需要额外约定。
        assertEquals(NrBookCategoryEntry.UNCATEGORIZED, grouped[8L])
        assertEquals(-1L, NrBookCategoryEntry.UNCATEGORIZED)
    }

    @Test
    fun bookCategoryBatchRejectsShortPayloadAndBadCount() {
        val tooShort = assertThrows(NrException::class.java) {
            NrBookCategoryCodec.entries(ByteArray(15), 1L)
        }
        assertEquals(NrErrors.NR_ERR_INTERNAL, tooShort.rawCode)

        val negative = assertThrows(NrException::class.java) {
            NrBookCategoryCodec.entries(ByteArray(16), -1L)
        }
        assertEquals(NrErrors.NR_ERR_INTERNAL, negative.rawCode)

        val badSize = assertThrows(NrException::class.java) {
            NrBookCategoryBatch.decode(ByteArray(31))
        }
        assertEquals(NrErrors.NR_ERR_INTERNAL, badSize.rawCode)
    }

    @Test
    fun idListRejectsLengthThatDoesNotMatchCount() {
        // 字节数与 count 必须**恰好**自洽：多一字节少一字节都是载荷损坏。
        assertEquals(listOf(1L, 2L), NrIdListCodec.ids(
            java.nio.ByteBuffer.allocate(16).order(java.nio.ByteOrder.LITTLE_ENDIAN)
                .putLong(0, 1L).putLong(8, 2L).array(),
            2L,
        ))
        val tooShort = assertThrows(NrException::class.java) {
            NrIdListCodec.ids(ByteArray(15), 2L)
        }
        assertEquals(NrErrors.NR_ERR_INTERNAL, tooShort.rawCode)
        val tooLong = assertThrows(NrException::class.java) {
            NrIdListCodec.ids(ByteArray(17), 2L)
        }
        assertEquals(NrErrors.NR_ERR_INTERNAL, tooLong.rawCode)
    }
}

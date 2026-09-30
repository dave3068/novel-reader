package com.novelreader.android.reader

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 页边界数学与 UTF-8 偏移（android/checklist.md C3：页边界数学、UTF-8 偏移）。
 *
 * 全部为纯函数断言：不依赖 Android 排版栈，可在 JVM 上精确手算。
 */
class PaginationMathTest {

    private val spec = TextLayoutSpec(
        viewportWidthPx = 400,
        viewportHeightPx = 108,
        fontSizePx = 18f,
        lineSpacingMultiplier = 1.35f,
        horizontalMarginPx = 20f,
        verticalMarginPx = 0f,
    )

    // ---------------------------------------------------------------- UTF-8 偏移

    @Test
    fun utf8PrefixLengthMatchesHandCheckedValues() {
        assertEquals(0, Utf8Offsets.byteLengthOfPrefix("中文正文", 0))
        assertEquals(3, Utf8Offsets.byteLengthOfPrefix("中文正文", 1))
        assertEquals(6, Utf8Offsets.byteLengthOfPrefix("中文正文", 2))
        assertEquals(12, Utf8Offsets.byteLengthOfPrefix("中文正文", 4))
        // ASCII 1 字节；汉字 3 字节（a+b = 2，中 = 3）
        assertEquals(5, Utf8Offsets.byteLengthOfPrefix("ab中", 3))
        // 越界按末尾处理，不抛异常
        assertEquals(5, Utf8Offsets.byteLengthOfPrefix("ab中", 99))
    }

    @Test
    fun utf8PrefixCountsSurrogatePairAsFourBytes() {
        val emoji = "😀" // U+1F600：代理对 → 4 字节
        assertEquals(4, Utf8Offsets.byteLengthOfPrefix(emoji, emoji.length))
        // 中(3 字节) + 代理对(4 字节) = 7
        assertEquals(7, Utf8Offsets.byteLengthOfPrefix("中$emoji", 3))
        assertFalse(Utf8Offsets.isCodePointBoundary(emoji, 1))
        assertTrue(Utf8Offsets.isCodePointBoundary(emoji, 0))
        assertTrue(Utf8Offsets.isCodePointBoundary(emoji, 2))
        assertEquals(2, Utf8Offsets.nextCodePointIndex(emoji, 0))
    }

    // ---------------------------------------------------------------- 行数裁切

    @Test
    fun fittingLineCountReservesWholeLineSafetyMargin() {
        // 可用高度 108、行高 27：预算 = 108 − 27 = 81 ⇒ 恰好 3 行（3×27 ≤ 81 < 4×27）
        val measured = MeasuredText(lineCount = 40, lineHeightPx = 27f, lineEnds = (1..40).toList())
        assertEquals(3, PageLayout.fittingLineCount(measured, spec))
        assertTrue(
            "行数×行高不得超过可用高度（否则末行会被裁切）",
            3 * 27f <= PageLayout.usableContentHeightPx(spec),
        )
    }

    @Test
    fun fittingLineCountIsZeroWhenNotEvenOneLineFits() {
        val tiny = TextLayoutSpec(
            viewportWidthPx = 400,
            viewportHeightPx = 20,
            fontSizePx = 18f,
            lineSpacingMultiplier = 1.35f,
            horizontalMarginPx = 20f,
            verticalMarginPx = 0f,
        )
        val measured = MeasuredText(lineCount = 5, lineHeightPx = 27f, lineEnds = listOf(10, 20, 30, 40, 50))
        assertEquals(0, PageLayout.fittingLineCount(measured, tiny))
    }

    @Test
    fun contentAreaSubtractsMargins() {
        val withMargins = spec.copy(horizontalMarginPx = 40f, verticalMarginPx = 8f)
        assertEquals(400 - 80, withMargins.contentWidthPx)
        assertEquals(108f - 16f, withMargins.contentHeightPx, 0.001f)
    }

    // ---------------------------------------------------------------- 页内切分

    @Test
    fun sliceEndsOnLegalUtf8BoundaryOfCjkText() {
        val text = "中".repeat(20)
        val bytes = text.toByteArray(Charsets.UTF_8)
        val measured = MeasuredText(lineCount = 2, lineHeightPx = 27f, lineEnds = listOf(10, 20))
        val slice = PageLayout.slice(windowStartByte = 0L, text = text, measured = measured, lineCount = 2)
        assertEquals(0L, slice.startByte)
        assertEquals("两行共 20 个汉字 ⇒ 60 字节", 60L, slice.endByte)
        assertEquals(20, slice.text.length)
        assertTrue(isUtf8Boundary(bytes, slice.endByte))
        // 窗口内 20 个字符全部消费 ⇒ 已到达窗口末尾
        assertTrue(slice.reachedWindowEnd)
    }

    @Test
    fun sliceAdvancesAtLeastOneCodePointWhenNothingFits() {
        val text = "中文"
        val measured = MeasuredText(lineCount = 1, lineHeightPx = 27f, lineEnds = listOf(2))
        val slice = PageLayout.slice(windowStartByte = 6L, text = text, measured = measured, lineCount = 0)
        assertEquals(6L, slice.startByte)
        assertEquals("零行可用时至少消费一个完整码点，翻页必须前进", 9L, slice.endByte)
        assertEquals("中", slice.text)

        val emoji = "😀中文"
        val emojiSlice = PageLayout.slice(
            windowStartByte = 0L,
            text = emoji,
            measured = MeasuredText(1, 27f, listOf(1)),
            lineCount = 0,
        )
        assertEquals("代理对必须整体消费（4 字节）", 4L, emojiSlice.endByte)
    }

    @Test
    fun sliceOfEmptyWindowProducesZeroLengthPage() {
        val slice = PageLayout.slice(12L, "", MeasuredText(0, 27f, emptyList()), 0)
        assertEquals(12L, slice.startByte)
        assertEquals(12L, slice.endByte)
        assertTrue(slice.text.isEmpty())
        assertTrue(slice.reachedWindowEnd)
    }

    @Test
    fun lineSpacingMilliUsesDesktopConvention() {
        // 桌面端 line_spacing_milli 表示「额外行距」：(倍数 − 1) × 字号
        val wide = spec.copy(fontSizePx = 20f, lineSpacingMultiplier = 1.5f)
        assertEquals(10_000L, wide.extraLeadingMilli)
        assertEquals(20_000L, wide.fontSizeMilli)
        assertEquals(0L, spec.copy(lineSpacingMultiplier = 1.0f).extraLeadingMilli)
    }
}

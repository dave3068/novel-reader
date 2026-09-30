package com.novelreader.android.reader

import com.novelreader.android.core.NrAbiConstants
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.charset.Charset

/**
 * 内容编码：核心编码码 → 平台解码器，以及**编码感知的字节偏移**。
 *
 * 真机缺陷：打开部分文档显示乱码，并报 `NR_ERR_3 (ENCODING) 偏移不在合法字符边界`。
 * 根因：核心早已检测编码并经 `nr_open_result.encoding` 回报，而 Android 侧一律按 UTF-8
 * 解码正文窗口 / 章节标题、并按 UTF-8 计算页边界字节长度（`Utf8Offsets`）。
 */
@OptIn(ExperimentalCoroutinesApi::class)
class ContentEncodingTest {

    private val gb18030: Charset = Charset.forName("GB18030")

    @Test
    fun abiCodesMapToPlatformCharsetsAndNames() {
        assertEquals(ContentEncoding.UTF8, ContentEncoding.ofAbiCode(NrAbiConstants.NR_ENCODING_UTF8))
        assertEquals(ContentEncoding.UTF8_BOM, ContentEncoding.ofAbiCode(2L))
        assertEquals(ContentEncoding.UTF16_LE, ContentEncoding.ofAbiCode(3L))
        assertEquals(ContentEncoding.UTF16_BE, ContentEncoding.ofAbiCode(4L))
        assertEquals("GB18030", ContentEncoding.ofAbiCode(5L).displayName)
        assertEquals("GBK", ContentEncoding.ofAbiCode(6L).displayName)
        assertEquals("Big5", ContentEncoding.ofAbiCode(7L).displayName)
        assertEquals("未知码必须退回 UTF-8，绝不抛异常", ContentEncoding.UTF8, ContentEncoding.ofAbiCode(999L))
        assertEquals("未知(999)", ContentEncoding.describe(999L))
        assertTrue("GB18030 必须真的可用", Charset.isSupported("GB18030"))
    }

    @Test
    fun byteOffsetsComeFromSourceBytesNotReEncoding() {
        val text = "中文abc"
        val utf8Bytes = text.toByteArray(Charsets.UTF_8)
        val fromUtf8 = EncodedTextCodec.decode(utf8Bytes, Charsets.UTF_8)
        assertEquals("UTF-8：两个汉字 6 字节", 6L, fromUtf8.byteLengthOfPrefix(2))
        assertEquals("整窗必须被完整覆盖", utf8Bytes.size.toLong(), fromUtf8.prefixByteEnds.last())
        assertEquals(text, fromUtf8.text)

        val gbBytes = text.toByteArray(gb18030)
        val fromGb = EncodedTextCodec.decode(gbBytes, gb18030)
        assertEquals("GB18030：两个汉字 4 字节", 4L, fromGb.byteLengthOfPrefix(2))
        assertEquals(
            "字节终点必须来自源字节本身（页边界正确性的关键不变量）",
            gbBytes.size.toLong(),
            fromGb.prefixByteEnds.last(),
        )
        assertEquals(text, fromGb.text)

        val gbkBytes = text.toByteArray(Charset.forName("GBK"))
        val fromGbk = EncodedTextCodec.decode(gbkBytes, Charset.forName("GBK"))
        assertEquals(4L, fromGbk.byteLengthOfPrefix(2))
        assertEquals(0L, fromGbk.byteLengthOfPrefix(0))
        assertEquals("越界按末尾处理", fromGbk.prefixByteEnds.last(), fromGbk.byteLengthOfPrefix(999))
    }

    @Test
    fun chapterTitlesStayUtf8EvenWhenContentIsGb18030() = runTest {
        val harness = ReaderHarness(io = StandardTestDispatcher(testScheduler), scope = TestScope(testScheduler))
        harness.abi.encoding = NrAbiConstants.NR_ENCODING_GB18030
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)

        val chapters = harness.viewModel.ui.value.reader.chapters
        assertTrue("必须读出章节列表", chapters.isNotEmpty())
        assertTrue(
            "章节标题一律 UTF-8（头文件 §3），不得按内容编码解码 —— 曾因此目录乱码（真机实测）",
            chapters.first().title.contains("第1章"),
        )
    }

    @Test
    fun decodingWithReportedEncodingAvoidsMojibake() {
        val original = "第一章 测试标题"
        val bytes = original.toByteArray(gb18030)
        assertEquals("按核心编码解码必须还原原文", original, String(bytes, gb18030))
        assertFalse(
            "按 UTF-8 解码 GB18030 字节必然乱码（即真机现象）",
            String(bytes, Charsets.UTF_8) == original,
        )
    }

    @Test
    fun openResultEncodingIsSurfacedToUiState() = runTest {
        val harness = ReaderHarness(io = StandardTestDispatcher(testScheduler), scope = TestScope(testScheduler))
        harness.abi.encoding = NrAbiConstants.NR_ENCODING_GB18030
        harness.abi.encodingConfidence = 640
        harness.abi.needsEncodingConfirmation = 1
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)

        val reader = harness.viewModel.ui.value.reader
        assertEquals("界面必须显示当前使用的编码", "GB18030", reader.encodingName)
        assertEquals(640, reader.encodingConfidence)
        assertTrue("低可信度必须提示「待确认」，供后续手动覆盖", reader.needsEncodingConfirmation)
        assertTrue(
            "打开状态文案必须写明编码名（而不是裸码值）",
            harness.viewModel.ui.value.status.contains("GB18030"),
        )
    }

    @Test
    fun utf8BookKeepsUtf8NameWithoutWarning() = runTest {
        val harness = ReaderHarness(io = StandardTestDispatcher(testScheduler), scope = TestScope(testScheduler))
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)

        assertEquals("UTF-8", harness.viewModel.ui.value.reader.encodingName)
        assertFalse(harness.viewModel.ui.value.reader.needsEncodingConfirmation)
    }

    @Test
    fun leadingSplitCharacterIsDetectedAndSkipped() {
        // 核心的窗口对齐是 UTF-8 专属：legacy 编码下首字节可能被切断（解码出替换字符）。
        // 平台自己解码，因此能识别并跳过它 —— 跳过之后才是真正的字符边界。
        val invalidHead = byteArrayOf(0x81.toByte(), 0x20, 0xd2.toByte(), 0xbb.toByte())
        val raw = EncodedTextCodec.decode(invalidHead, gb18030)
        assertTrue(
            "非法首字节必须体现为开头的替换字符（真机「首行乱码」）",
            raw.leadingReplacementCount() >= 1,
        )

        val skipped = raw.afterSkipping(raw.leadingReplacementCount())
        assertEquals("跳过之后，字节映射必须重新基准化", 0L, skipped.prefixByteEnds.first())
        assertTrue("跳过之后应能读到「一」", skipped.text.contains("一"))
        assertEquals(
            "重新基准化后，映射长度仍与文本一致",
            skipped.text.length + 1,
            skipped.prefixByteEnds.size,
        )

        // 未切分的情形不得误跳：替换字符出现在**末尾**时 leadingReplacementCount 必须为 0。
        val tailBroken = EncodedTextCodec.decode(
            byteArrayOf(0xd2.toByte(), 0xbb.toByte(), 0xd2.toByte()),
            gb18030,
        )
        assertEquals("只有开头才是对齐问题", 0, tailBroken.leadingReplacementCount())
    }
}

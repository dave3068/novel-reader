package com.novelreader.android.reader

import com.novelreader.android.core.NrAbiConstants
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 排版签名与状态接线（android/checklist.md C3：设置状态测试；A-T3 真实状态接线）。
 *
 * 断言「设置 → 像素 → 13 项签名」整条链路，以及页边界提交与核心回执的一致性。
 */
@OptIn(ExperimentalCoroutinesApi::class)
class LayoutSignatureTest {

    @Test
    fun beginLayoutFieldsMatchTypographyAndViewport() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        val harness = ReaderHarness(io = dispatcher, scope = TestScope(testScheduler))
        harness.measureViewport(widthPx = 400, heightPx = 140, spToPx = 1f, dpToPx = 1f)
        harness.openAndLayout()
        harness.settle(this)

        val begin = harness.abi.boundarySubmissions
            .first { it.phase == NrAbiConstants.NR_SUBMIT_BEGIN_LAYOUT }
        val fields = begin.layoutFields
        assertNotNull("BEGIN_LAYOUT 必须携带 13 项排版签名", fields)
        assertEquals(NrAbiConstants.NR_LAYOUT_FIELDS_SLOTS, fields!!.size)

        // 内容区 = 视口 − 左右边距（默认 20dp × dpToPx=1）
        assertEquals(400L - 40L, fields[0])
        assertEquals(108L, fields[1])
        assertEquals(0L, fields[2]) // direction = 横向
        assertEquals(400L, fields[3]) // font_weight 默认 400
        assertEquals(18_000L, fields[4]) // 18sp × 1.0 → 18000 milli-px
        assertEquals(6_300L, fields[5]) // (1.35 − 1) × 18 = 6.3 px → 6300 milli-px
        assertEquals(0L, fields[6]) // 段距（固定 0：两侧渲染能力未对齐前不参与排版）
        assertEquals(0L, fields[7]) // 首行缩进（同上）
        assertEquals(0L, fields[8]) // 边距已计入内容区，签名内不重复
        assertEquals(0L, fields[9])
        assertEquals(NrAbiConstants.NR_LAYOUT_ALGORITHM_VERSION.toLong(), fields[10])
        assertEquals(0L, fields[11])
        assertEquals(0L, fields[12]) // 平台字体指纹：0 = 尚未取得
        assertEquals(
            harness.book.bytes.size.toLong(),
            begin.totalBytes,
        )
        assertEquals(5L, begin.contentIndexVersion)
        assertEquals("sans-serif", String(begin.fontFamily ?: ByteArray(0), Charsets.UTF_8))
    }

    @Test
    fun commitSubmissionEchoesCurrentGenerationAndLegalOffsets() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        val harness = ReaderHarness(io = dispatcher, scope = TestScope(testScheduler))
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)

        val begin = harness.abi.boundarySubmissions.first { it.phase == NrAbiConstants.NR_SUBMIT_BEGIN_LAYOUT }
        val commits = harness.abi.commitRecords
        assertTrue("至少提交一页", commits.isNotEmpty())
        val first = commits.first()
        assertEquals(NrAbiConstants.NR_SUBMIT_COMMIT, first.phase)
        assertEquals("组必须带 BEGIN_LAYOUT 返回的 layout_id", harness.viewModel.ui.value.reader.layoutId, first.layoutId)
        assertEquals(harness.viewModel.ui.value.reader.requestId, first.requestId)
        assertEquals(harness.book.bytes.size.toLong(), first.totalBytes)
        assertEquals(0L, first.startByte)
        assertTrue(first.endByte > first.startByte)
        assertTrue("页末必须落在合法 UTF-8 边界", isUtf8Boundary(harness.book.bytes, first.endByte))
        // BEGIN_LAYOUT 的签名与提交阶段是同一次排版世代
        assertNotNull(begin.layoutFields)
    }

    @Test
    fun openedBookPublishesRealShelfAndReaderState() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        val harness = ReaderHarness(io = dispatcher, scope = TestScope(testScheduler))
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)

        val state = harness.viewModel.ui.value
        assertEquals(ReaderRoute.READER, state.route)
        assertEquals(1, state.shelf.books.size)
        val book = state.shelf.books.first()
        assertEquals(901L, book.bookId)
        assertEquals(harness.book.bytes.size.toLong(), book.totalBytes)
        assertEquals(3, book.chapterCount)
        assertTrue(book.hasProgress)
        assertEquals(901L, state.shelf.selectedBookId)

        val reader = state.reader
        assertEquals(901L, reader.bookId)
        assertEquals(3, reader.chapters.size)
        assertEquals("第1章 测试标题", reader.chapters.first().title)
        assertEquals(0L, reader.chapters.first().startByte)
        assertNotNull("打开后必须已经排出第一页", reader.page)
        val page = reader.page!!
        assertEquals(0L, page.startByte)
        assertEquals(ReaderHarness.PAGE_ONE_END, page.endByte)
        assertEquals(3, page.lineCount)
        assertEquals(27f, page.lineHeightPx, 0.001f)
        assertEquals(18f, page.fontSizePx, 0.001f)
        // 页面文本必须等于同一字节区间的真实解码结果（不是拼接出来的展示文本）
        assertEquals(
            String(harness.book.bytes.copyOfRange(0, page.endByte.toInt()), Charsets.UTF_8),
            page.text,
        )
        assertNull(state.failure)
        assertTrue(state.status.isNotEmpty())
    }

    @Test
    fun pageTurnSubmitsLegalBoundaryAndPersistsPosition() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        val harness = ReaderHarness(io = dispatcher, scope = TestScope(testScheduler))
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)
        val savedBefore = harness.abi.savedPositions.size

        harness.viewModel.nextPage()
        harness.settle(this)

        val page = harness.viewModel.ui.value.reader.page!!
        assertEquals("第二页起点 = 第一页终点", ReaderHarness.PAGE_ONE_END, page.startByte)
        assertEquals(ReaderHarness.PAGE_TWO_END, page.endByte)
        assertEquals(ReaderHarness.BODY_PAGE_BYTES, page.endByte - page.startByte)
        assertEquals(2, page.pageOrdinal)
        assertTrue(harness.viewModel.ui.value.reader.canGoPreviousPage)
        assertEquals(
            String(harness.book.bytes.copyOfRange(page.startByte.toInt(), page.endByte.toInt()), Charsets.UTF_8),
            page.text,
        )
        assertTrue("翻页必须落进度", harness.abi.savedPositions.size > savedBefore)
        assertEquals(ReaderHarness.PAGE_ONE_END, harness.abi.savedPositions.last().second)

        harness.viewModel.previousPage()
        harness.settle(this)
        val back = harness.viewModel.ui.value.reader.page!!
        assertEquals(0L, back.startByte)
        assertEquals(1, back.pageOrdinal)
        assertTrue("回到第一页后不得再往前翻", harness.viewModel.ui.value.reader.canGoPreviousPage.not())
    }

    @Test
    fun chapterNavigationUsesCoreChapterRanges() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        val harness = ReaderHarness(io = dispatcher, scope = TestScope(testScheduler))
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)

        harness.viewModel.nextChapter()
        harness.settle(this)

        val page = harness.viewModel.ui.value.reader.page!!
        val second = harness.book.chapters[1]
        assertEquals(1, page.chapterIndex)
        assertEquals(second.startByte, page.startByte)
        assertEquals("第2章 测试标题", page.chapterTitle)
        assertEquals(1, page.pageOrdinal)

        harness.viewModel.previousChapter()
        harness.settle(this)
        assertEquals(0L, harness.viewModel.ui.value.reader.page!!.startByte)
    }

    @Test
    fun pageCannotBeLaidOutBeforeViewportIsKnown() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        val harness = ReaderHarness(io = dispatcher, scope = TestScope(testScheduler))
        // 不测量视口：零视口是非法签名，必须显式拒绝而不是排出无穷多空页
        harness.openAndLayout()
        harness.settle(this)

        val beforeViewport = harness.viewModel.ui.value
        assertNull("视口未知时不得建立排版世代", beforeViewport.reader.page)
        assertEquals(0L, beforeViewport.reader.layoutId)
        assertEquals(0, harness.abi.beginLayoutCalls)
        assertTrue(beforeViewport.status.contains("等待正文区域尺寸"))

        harness.measureViewport()
        harness.settle(this)
        assertNotNull("视口就绪后必须自动排版", harness.viewModel.ui.value.reader.page)
        assertTrue(harness.abi.beginLayoutCalls >= 1)
    }

    @Test
    fun fontAndMarginSettingsChangeTheLayoutSignature() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        val harness = ReaderHarness(io = dispatcher, scope = TestScope(testScheduler))
        harness.measureViewport(widthPx = 400, heightPx = 140, spToPx = 1.5f, dpToPx = 2f)
        harness.openAndLayout()
        harness.settle(this)

        val first = harness.abi.boundarySubmissions.first { it.phase == NrAbiConstants.NR_SUBMIT_BEGIN_LAYOUT }
        // 字号像素 = 18sp × spToPx(1.5) = 27px → 27000 milli-px；额外行距 = 0.35 × 27px
        assertEquals(27_000L, first.layoutFields!![4])
        assertEquals(9_450L, first.layoutFields[5])
        assertEquals(400L - 2 * 20L * 2L, first.layoutFields[0])

        harness.viewModel.onHorizontalMarginChange(40f)
        harness.settle(this)
        val after = harness.abi.boundarySubmissions
            .filter { it.phase == NrAbiConstants.NR_SUBMIT_BEGIN_LAYOUT }
            .last()
        assertEquals("左右边距变化必须改变内容区宽度（进而改变缓存键）", 400L - 2 * 40L * 2L, after.layoutFields!![0])
        assertTrue(
            "新签名必须有新的 layout_id",
            harness.viewModel.ui.value.reader.layoutId > 1L,
        )
    }
}

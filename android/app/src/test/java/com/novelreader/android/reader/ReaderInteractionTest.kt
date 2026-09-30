package com.novelreader.android.reader

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 阅读交互层：手势判定（纯逻辑）与章节目录跳转。
 *
 * 真机缺陷背景：PoC 有手势、正式工程从未移植；规格 `:620`（1/3 分带 + 中部切换界面）
 * 与 `:611`（临时菜单提供目录）均无卡片认领（见 `results/reader-interaction.md`）。
 */
@OptIn(ExperimentalCoroutinesApi::class)
class ReaderInteractionTest {

    // ---------------------------------------------------------------- 点击分带

    @Test
    fun tapZonesFollowThirds() {
        val width = 900
        assertEquals(ReaderTapZone.PREVIOUS_PAGE, ReaderInteraction.tapZoneFor(0f, width))
        assertEquals(ReaderTapZone.PREVIOUS_PAGE, ReaderInteraction.tapZoneFor(299f, width))
        assertEquals("第三分带起点属中部", ReaderTapZone.TOGGLE_CHROME, ReaderInteraction.tapZoneFor(300f, width))
        assertEquals(ReaderTapZone.TOGGLE_CHROME, ReaderInteraction.tapZoneFor(450f, width))
        assertEquals(ReaderTapZone.TOGGLE_CHROME, ReaderInteraction.tapZoneFor(599f, width))
        assertEquals("第三分带起点属下一页", ReaderTapZone.NEXT_PAGE, ReaderInteraction.tapZoneFor(600f, width))
        assertEquals(ReaderTapZone.NEXT_PAGE, ReaderInteraction.tapZoneFor(899f, width))
    }

    @Test
    fun tapZoneClampsOutOfRangeAndGuardsInvalidWidth() {
        assertEquals(ReaderTapZone.PREVIOUS_PAGE, ReaderInteraction.tapZoneFor(-50f, 900))
        assertEquals(ReaderTapZone.NEXT_PAGE, ReaderInteraction.tapZoneFor(5000f, 900))
        assertEquals(
            "宽度未知时只能切换界面，绝不误触翻页",
            ReaderTapZone.TOGGLE_CHROME,
            ReaderInteraction.tapZoneFor(10f, 0),
        )
    }

    // ---------------------------------------------------------------- 横滑

    @Test
    fun swipeFollowsDirectionAndThreshold() {
        val threshold = 120f
        assertEquals("位移不足不翻页", ReaderSwipeDirection.NONE, ReaderInteraction.swipePageFor(119f, threshold))
        assertEquals("位移不足不翻页", ReaderSwipeDirection.NONE, ReaderInteraction.swipePageFor(-119f, threshold))
        assertEquals("左划 = 下一页", ReaderSwipeDirection.NEXT_PAGE, ReaderInteraction.swipePageFor(-121f, threshold))
        assertEquals("右划 = 上一页", ReaderSwipeDirection.PREVIOUS_PAGE, ReaderInteraction.swipePageFor(121f, threshold))
    }

    @Test
    fun swipeGuardsInvalidThreshold() {
        assertEquals(
            "阈值非法时不得翻页",
            ReaderSwipeDirection.NONE,
            ReaderInteraction.swipePageFor(-500f, 0f),
        )
    }

    // ---------------------------------------------------------------- 目录跳转

    private fun harnessFor(scope: TestScope): ReaderHarness =
        ReaderHarness(io = StandardTestDispatcher(scope.testScheduler), scope = TestScope(scope.testScheduler))

    @Test
    fun jumpToChapterLandsOnChapterFirstPage() = runTest {
        val harness = harnessFor(this)
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)

        harness.viewModel.jumpToChapter(2)
        harness.settle(this)
        val last = requireNotNull(harness.viewModel.ui.value.reader.page) { "跳转后应有页面" }
        assertEquals("应跳到目标章", 2, last.chapterIndex)
        assertEquals("应落在该章首页（本夹具每章 207 字节）", 414L, last.startByte)
        assertTrue("跳转属正常导航，不得留下失败", harness.viewModel.ui.value.failure == null)

        harness.viewModel.jumpToChapter(0)
        harness.settle(this)
        val first = requireNotNull(harness.viewModel.ui.value.reader.page) { "跳回后应有页面" }
        assertEquals(0, first.chapterIndex)
        assertEquals(0L, first.startByte)
    }

    @Test
    fun jumpToChapterRejectsOutOfRangeWithoutChangingPage() = runTest {
        val harness = harnessFor(this)
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)
        val before = requireNotNull(harness.viewModel.ui.value.reader.page).startByte

        harness.viewModel.jumpToChapter(99)
        harness.settle(this)

        assertEquals("越界跳转不得改变页面", before, requireNotNull(harness.viewModel.ui.value.reader.page).startByte)
        assertTrue(
            "越界必须给出明确状态",
            harness.viewModel.ui.value.status.contains("不存在"),
        )
    }

    @Test
    fun jumpToChapterWithoutOpenBookReportsNoBook() = runTest {
        val harness = harnessFor(this)
        harness.settle(this)

        harness.viewModel.jumpToChapter(0)
        harness.settle(this)

        assertEquals(ReaderStatus.NO_BOOK, harness.viewModel.ui.value.status)
        assertFalse(harness.viewModel.ui.value.reader.isOpen)
    }
}

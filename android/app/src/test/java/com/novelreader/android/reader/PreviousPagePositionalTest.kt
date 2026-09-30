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
 * 「上一页」的**纯位置语义**与阅读进度读数。
 *
 * 用户要求（原文）：「当切章节后，点击『上一页』无需回到之前所在之处。
 * 应当与正常翻页逻辑一样，以当前所在位置向前推一页翻页。」
 * 即：从第 2 章首页按「上一页」必须到**第 1 章最后一页**，
 * 而不是回到「离开第 1 章时所在的那一页」。
 *
 * 夹具几何（见 `ReaderHarness.measureViewport`）：每章 207 字节、每页 3 行，
 * 章内页界为 `[0,83) [83,176) [176,207)`；全书 3 章共 621 字节。
 */
@OptIn(ExperimentalCoroutinesApi::class)
class PreviousPagePositionalTest {

    private fun harnessFor(scope: TestScope): ReaderHarness =
        ReaderHarness(io = StandardTestDispatcher(scope.testScheduler), scope = TestScope(scope.testScheduler))

    @Test
    fun chapterJumpFromMiddlePageGoesToPreviousChapterLastPage() = runTest {
        val harness = harnessFor(this)
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)

        // 只前进一页 → 停在**章中**（第 2 页，偏移 83），而非章末
        harness.viewModel.nextPage()
        harness.settle(this)
        val middle = requireNotNull(harness.viewModel.ui.value.reader.page) { "前进一页后应有页面" }
        assertEquals("该用例前提：停在章中页", CHAPTER_ONE_PAGE_TWO_START, middle.startByte)

        harness.viewModel.nextChapter()
        harness.settle(this)
        val atNextChapter = requireNotNull(harness.viewModel.ui.value.reader.page) { "切章后应有页面" }
        assertEquals(1, atNextChapter.chapterIndex)

        harness.viewModel.previousPage()
        harness.settle(this)
        val back = requireNotNull(harness.viewModel.ui.value.reader.page) { "后退后应有页面" }

        assertEquals("必须回到第 1 章（不是跳到别处）", 0, back.chapterIndex)
        assertEquals(
            "跨章后退必须落在「第 1 章最后一页」，而不是离开该章时所在的章中页",
            CHAPTER_ONE_LAST_PAGE_START,
            back.startByte,
        )
    }

    @Test
    fun repeatedPreviousPageWalksBackPageByPageAcrossChapters() = runTest {
        val harness = harnessFor(this)
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)
        harness.viewModel.nextChapter()
        harness.settle(this)

        val visited = mutableListOf<Long>()
        repeat(3) {
            harness.viewModel.previousPage()
            harness.settle(this)
            visited += requireNotNull(harness.viewModel.ui.value.reader.page).startByte
        }

        assertEquals(
            "连续后退应逐页回走：章末页 → 章中页 → 章首页",
            listOf(CHAPTER_ONE_LAST_PAGE_START, CHAPTER_ONE_PAGE_TWO_START, 0L),
            visited,
        )
        assertFalse(
            "回到全书第一页后必须置灰",
            harness.viewModel.ui.value.reader.canGoPreviousPage,
        )
    }

    @Test
    fun progressReadoutIsByteBasedAndSurvivesChapterJump() = runTest {
        val harness = harnessFor(this)
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)
        assertEquals("打开即 0%", 0, harness.viewModel.ui.value.reader.progressPermille)
        assertEquals("0.0%", harness.viewModel.ui.value.reader.progressText)

        harness.viewModel.nextPage()
        harness.settle(this)
        // 83 / 621 = 13.365% → 千分比 133
        assertEquals(133, harness.viewModel.ui.value.reader.progressPermille)
        assertEquals("13.3%", harness.viewModel.ui.value.reader.progressText)

        harness.viewModel.nextChapter()
        harness.settle(this)
        // 207 / 621 = 33.333% → 千分比 333（切章后仍然可算，不会退回 0）
        assertEquals(333, harness.viewModel.ui.value.reader.progressPermille)

        assertEquals(
            "状态栏与副标题共用同一读数来源",
            true,
            harness.viewModel.ui.value.status.contains("进度 33.3%"),
        )
    }

    private companion object {
        const val CHAPTER_ONE_PAGE_TWO_START = 83L
        const val CHAPTER_ONE_LAST_PAGE_START = 176L
    }
}

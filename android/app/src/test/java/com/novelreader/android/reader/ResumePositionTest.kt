package com.novelreader.android.reader

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * 「读到某章中部退出再入是否继续原位置」（移动端）。
 *
 * # 缺陷背景
 *
 * 桌面端在退出时保存当前位置（`crates/desktop-app/src/ui.rs` 的 `save_current(..., "退出")`）；
 * 移动端此前**只在翻页 / 切章 / 重排时**保存，退出（回到书架或退到后台）不落盘
 * ⇒ 章中退出再入会退回上一次保存点。
 *
 * 本用例锁定三件事：
 * 1. 离开阅读器（`closeBook`）前落盘当前页；
 * 2. 退到后台（`onHostPaused`）落盘当前页，且**不改写界面状态文案**；
 * 3. 再入后按该偏移继续（而不是回到章首 / 书首）。
 *
 * ⚠️ 核心侧已由真实探针确认无误：章中保存 → 销毁引擎 → 同库新引擎 → 读回仍是章中偏移
 * （`found=1 chapter=1 offset=2172 restored=1 fallback=0`）。
 */
@OptIn(ExperimentalCoroutinesApi::class)
class ResumePositionTest {

    private val identity = "/tmp/novel-reader-fixture.txt"

    private fun harnessFor(scope: TestScope): ReaderHarness =
        ReaderHarness(
            io = StandardTestDispatcher(scope.testScheduler),
            scope = TestScope(scope.testScheduler),
        )

    /** 让替身「读回最后一次保存」，与真实核心同构（默认脚本化旋钮保持不变）。 */
    private fun ReaderHarness.mirrorCorePosition() {
        abi.positionMirrorsSave = true
    }

    @Test
    fun leavingTheReaderMidChapterThenReenteringContinuesFromTheSameOffset() = runTest {
        val harness = harnessFor(this)
        harness.mirrorCorePosition()
        // 第 2 章章中：标题行 + 一个正文行的合法 UTF-8 边界。
        val mid = harness.book.chapters[1].startByte + 52L
        harness.abi.positionFound = 1L
        harness.abi.positionChapterIndex = 1L
        harness.abi.positionOffset = mid

        harness.measureViewport()
        harness.openAndLayout(identity)
        harness.settle(this)
        assertEquals(
            "前提：首次进入应停在章中偏移",
            mid,
            harness.viewModel.ui.value.reader.page!!.startByte,
        )

        // 退出到书架（此前不落盘 ⇒ 真机缺陷）。
        harness.viewModel.closeBook()
        harness.settle(this)
        assertEquals(
            "离开阅读器前必须落盘当前页",
            1L to mid,
            harness.abi.lastSavedPosition,
        )

        // 再入。
        harness.viewModel.openBook(identity)
        harness.settle(this)

        val page = harness.viewModel.ui.value.reader.page!!
        assertEquals("再入后必须继续章中位置（而非章首/书首）", mid, page.startByte)
        assertEquals(1, page.chapterIndex)
    }

    @Test
    fun backgroundingSavesCurrentPositionAndKeepsStatusText() = runTest {
        val harness = harnessFor(this)
        harness.measureViewport()
        harness.openAndLayout(identity)
        harness.settle(this)

        val page = harness.viewModel.ui.value.reader.page!!
        val statusBefore = harness.viewModel.ui.value.status

        harness.viewModel.onHostPaused()
        harness.settle(this)

        assertEquals(
            "退到后台必须落盘当前页",
            page.chapterIndex.toLong() to page.startByte,
            harness.abi.lastSavedPosition,
        )
        assertEquals(
            "退出保存不得改写界面状态文案（用户看到的仍是页面状态）",
            statusBefore,
            harness.viewModel.ui.value.status,
        )
    }

    @Test
    fun backgroundingWithoutOpenBookIsNoOp() = runTest {
        val harness = harnessFor(this)
        harness.settle(this)
        val statusBefore = harness.viewModel.ui.value.status

        harness.viewModel.onHostPaused()
        harness.settle(this)

        assertNull("未打开书籍时不得写库", harness.abi.lastSavedPosition)
        assertEquals("未打开书籍时状态不得变化", statusBefore, harness.viewModel.ui.value.status)
    }
}

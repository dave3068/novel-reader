package com.novelreader.android.reader

import com.novelreader.android.core.FakeReleaser
import com.novelreader.android.core.NrHistoryEntry
import com.novelreader.android.core.NrHistoryBatch
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import java.nio.ByteBuffer
import java.nio.ByteOrder
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 最近阅读面板（V1.1 规格 R8/R9、§3.6）。
 *
 * 钉住三条核心口径：
 * - **排序由核心给**（界面不重排）；
 * - **时间标签由核心格式化**（界面不二次加工）；
 * - 清除只删流水，且对不存在的 id **如实回报**。
 */
@OptIn(ExperimentalCoroutinesApi::class)
class HistoryPanelTest {

    private fun le(size: Int): ByteBuffer = ByteBuffer.allocate(size).order(ByteOrder.LITTLE_ENDIAN)

    /** 一条流水：id / bookId / 时间 / 章 / 偏移 / 方式 + 三段文本。 */
    private data class Row(
        val id: Long,
        val bookId: Long,
        val openedAt: Long,
        val chapterIndex: Long,
        val byteOffset: Long,
        val entryKind: Long,
        val title: String,
        val file: String,
        val label: String,
    )

    /** 造 `nr_history` 批次：所有表头在前，三段文本紧随其后（**不是**逐行交错）。 */
    private fun historyPayload(vararg rows: Row): ByteArray {
        val headerSize = NrHistoryEntry.STRUCT_SIZE * rows.size
        val texts = rows.flatMap { listOf(it.title, it.file, it.label) }
            .map { it.toByteArray(Charsets.UTF_8) }
        val total = headerSize + texts.sumOf { it.size }
        val buf = le(total)
        var textCursor = headerSize
        var textIndex = 0
        rows.forEachIndexed { index, row ->
            val base = index * NrHistoryEntry.STRUCT_SIZE
            buf.putLong(base, row.id)
            buf.putLong(base + 8, row.bookId)
            buf.putLong(base + 16, row.openedAt)
            buf.putLong(base + 24, row.chapterIndex)
            buf.putLong(base + 32, row.byteOffset)
            buf.putLong(base + 40, row.entryKind)
            listOf(48, 56, 64).forEachIndexed { slot, at ->
                buf.putInt(base + at, textCursor)
                buf.putInt(base + at + 4, texts[textIndex].size)
                textCursor += texts[textIndex].size
                textIndex += 1
            }
        }
        val out = buf.array()
        var at = headerSize
        texts.forEach { it.copyInto(out, at); at += it.size }
        return out
    }

    private fun viewModel(abi: ScriptedReaderAbi): ReaderViewModel {
        val releaser = FakeReleaser()
        return ReaderViewModel(
            engine = engineOn(abi, releaser),
            releaser = releaser,
            measurer = FakeTextMeasurer(),
            io = UnconfinedTestDispatcher(),
            workScope = kotlinx.coroutines.CoroutineScope(UnconfinedTestDispatcher()),
        )
    }

    private fun abiWith(vararg rows: Row) = ScriptedReaderAbi(FixtureBook.build()).apply {
        historyBytes = historyPayload(*rows)
        historyCount = rows.size
        historyTotal = rows.size
    }

    // ------------------------------------------------------------------ 读取

    @Test
    fun openingHistoryLoadsRowsWithCoreLabelsAndTotal() = runTest {
        val abi = abiWith(
            Row(1L, 7L, 1_791_264_000L, 3L, 128L, 0L, "三体", "a.txt", "14:32"),
            Row(2L, 8L, 1_791_200_000L, 0L, 0L, 2L, "活着", "b.txt", "10-05 09:05"),
        )
        val vm = viewModel(abi)

        vm.openHistory()

        val history = vm.ui.value.history
        assertEquals(ReaderRoute.HISTORY, vm.ui.value.route)
        assertEquals(2, history.rows.size)
        assertEquals(2, history.total)
        assertFalse(history.loading)

        val first = history.rows[0]
        assertEquals(1L, first.entryId)
        assertEquals(7L, first.bookId)
        assertEquals("三体", first.title)
        assertEquals("a.txt", first.fileLabel)
        // 标签是核心给的成品文本 —— 原样呈现。
        assertEquals("14:32", first.timeLabel)
        assertEquals("第 4 章 · 偏移 128 字节", first.positionLabel)
    }

    @Test
    fun rowsKeepCoreOrderWithoutResorting() = runTest {
        // 核心按 opened_at DESC 排好；界面若按 id 或标题重排就会破坏语义。
        val abi = abiWith(
            Row(9L, 7L, 1_791_264_000L, 0L, 0L, 0L, "甲", "a.txt", "14:32"),
            Row(3L, 8L, 1_791_200_000L, 0L, 0L, 0L, "乙", "b.txt", "13:00"),
        )
        val vm = viewModel(abi)

        vm.openHistory()

        assertEquals(listOf(9L, 3L), vm.ui.value.history.rows.map { it.entryId })
    }

    @Test
    fun emptyHistoryShowsEmptyStateWithZeroTotal() = runTest {
        val abi = ScriptedReaderAbi(FixtureBook.build()).apply {
            historyBytes = ByteArray(0)
            historyCount = 0
            historyTotal = 0
        }
        val vm = viewModel(abi)

        vm.openHistory()

        assertTrue(vm.ui.value.history.isEmpty)
        assertEquals(0, vm.ui.value.history.total)
    }

    @Test
    fun readFailureSurfacesInsteadOfShowingEmptyHistory() = runTest {
        val abi = ScriptedReaderAbi(FixtureBook.build()).apply {
            v11Code = com.novelreader.android.core.NrErrors.NR_ERR_INTERNAL
        }
        val vm = viewModel(abi)

        vm.openHistory()

        // 读失败必须回显错误，**不得**静默显示成「还没有阅读记录」。
        assertTrue(
            "读失败应有错误回显，实际：${vm.ui.value.status}",
            vm.ui.value.status.contains("读取阅读历史失败"),
        )
    }

    // ------------------------------------------------------------------ 清除

    @Test
    fun clearingMissingEntryReportsHonestly() = runTest {
        val abi = abiWith(Row(1L, 7L, 1L, 0L, 0L, 0L, "甲", "a.txt", "14:32")).apply {
            historyEntryRemoved = 0
        }
        val vm = viewModel(abi)

        vm.onClearHistoryEntry(999L)

        // 0 = 该 id 不存在：如实回显，不谎报「已清除」。
        assertTrue(
            "应如实回报「已不存在」，实际：${vm.ui.value.status}",
            vm.ui.value.status.contains("已不存在"),
        )
    }

    @Test
    fun clearingOneEntryReportsSuccess() = runTest {
        val abi = abiWith(Row(1L, 7L, 1L, 0L, 0L, 0L, "甲", "a.txt", "14:32")).apply {
            historyEntryRemoved = 1
        }
        val vm = viewModel(abi)

        vm.onClearHistoryEntry(1L)

        assertTrue(vm.ui.value.status.contains("已清除该条流水"))
    }

    @Test
    fun clearingAllDeletesPerBookAndReportsRealTotal() = runTest {
        // 两本书各 2 条：清空必须按书逐本调核心，并把**核心返回的条数之和**报出来。
        val abi = abiWith(
            Row(1L, 7L, 4L, 0L, 0L, 0L, "甲", "a.txt", "14:32"),
            Row(2L, 7L, 3L, 0L, 0L, 0L, "甲", "a.txt", "13:00"),
            Row(3L, 8L, 2L, 0L, 0L, 0L, "乙", "b.txt", "12:00"),
            Row(4L, 8L, 1L, 0L, 0L, 0L, "乙", "b.txt", "11:00"),
        ).apply { historyClearedCount = 3 }
        val vm = viewModel(abi)

        vm.openHistory()
        vm.onClearAllHistory()

        // 两本书 ⇒ 两次 per-book 清除（ABI 没有「清空全部」入口）。
        val perBookCalls = abi.events.count { it == "clearHistoryForBook:enter" }
        assertEquals(2, perBookCalls)
        // 报告的是**核心返回值之和**（3 + 3 = 6），不是界面行数（4）。
        // ⚠️ 两个数必须**可区分**，否则「按行数谎报」的实现也能通过（变异测试实测踩中：
        // 初版让核心每本回 2 条、界面 4 行，两者恰好相等，判据形同虚设）。
        assertEquals(4, vm.ui.value.history.rows.size)
        assertTrue("实际状态：${vm.ui.value.status}", vm.ui.value.status.contains("已清除 6 条流水"))
    }

    @Test
    fun clearAllWithNoRowsDoesNotCallCore() = runTest {
        val abi = ScriptedReaderAbi(FixtureBook.build()).apply {
            historyBytes = ByteArray(0)
            historyCount = 0
        }
        val vm = viewModel(abi)

        vm.openHistory()
        vm.onClearAllHistory()

        // 没有行就没有书要去清 —— 不得产生空转的原生调用。
        assertEquals(0, abi.events.count { it == "clearHistoryForBook:enter" })
    }
}

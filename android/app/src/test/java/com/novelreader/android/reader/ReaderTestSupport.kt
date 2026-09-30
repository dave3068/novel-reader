package com.novelreader.android.reader

import com.novelreader.android.core.FakeReleaser
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runCurrent

/**
 * 单测夹具：脚本化 ABI + 确定性测量器 + 真实 [com.novelreader.android.core.NrEngine] 薄封装。
 *
 * 宿主机无法加载 arm64 的 `libnr_jni.so`，但生产路径的装箱/拆箱、错误映射与缓冲区归还
 * 走的是与真机**同一份** Kotlin 代码，因此这里的断言是行为级证据，不是自报成功。
 */
internal class ReaderHarness(
    val book: FixtureBook = FixtureBook.build(),
    io: CoroutineDispatcher,
    scope: CoroutineScope,
    hostResolver: HostSourceResolver? = null,
    anchorStore: ReaderAnchorStore? = null,
    settingsStore: ReaderSettingsStore? = null,
    bookCatalog: HostBookCatalog? = null,
    hiddenBooks: HiddenBooksStore? = null,
    val measurer: FakeTextMeasurer = FakeTextMeasurer(),
    val abi: ScriptedReaderAbi = ScriptedReaderAbi(book),
) {
    val releaser = FakeReleaser()
    val engine = engineOn(abi, releaser)
    val viewModel = ReaderViewModel(
        engine = engine,
        releaser = releaser,
        measurer = measurer,
        io = io,
        hostResolver = hostResolver,
        anchorStore = anchorStore,
        settingsStore = settingsStore,
        bookCatalog = bookCatalog,
        hiddenBooks = hiddenBooks,
        workScope = scope,
    )

    /**
     * 正文区域 400×140 px（默认上下边距 16dp×1 ⇒ 内容区高 108）、行高 27 ⇒ 每页 **3 行**。
     *
     * 夹具章节结构（3 章 × 1 标题行 + 6 正文行，每正文行 = 10 个汉字 + 换行 = 31 字节）：
     * - 章节 0 = `[0, 207)`（标题行 21 字节 + 6×31 字节）；
     * - 每页 3 行 ⇒ 页 1 = `[0, 83)`（标题行 + 2 正文行）、页 2 = `[83, 176)`、页 3 = `[176, 207)`。
     */
    fun measureViewport(
        widthPx: Int = VIEWPORT_WIDTH_PX,
        heightPx: Int = VIEWPORT_HEIGHT_PX,
        spToPx: Float = 1f,
        dpToPx: Float = 1f,
    ) {
        viewModel.onViewportMeasured(widthPx, heightPx, spToPx, dpToPx)
    }

    /** 打开夹具书籍（顺序与真实 Compose 首帧一致：先测量视口，后打开）。 */
    fun openAndLayout(identity: String = "/tmp/novel-reader-fixture.txt") {
        viewModel.openBook(identity, "fixture.txt")
    }

    /**
     * 推进测试调度器直到状态落定。
     *
     * `advanceUntilIdle()` 不会跑 `backgroundScope` 里新排队的工作协程，因此这里显式
     * `runCurrent()`（处理当前虚拟时间的全部任务，包括其后续排队的同刻任务）再补 `advanceUntilIdle()`。
     */
    fun settle(scope: TestScope) {
        scope.runCurrent()
        scope.advanceUntilIdle()
        scope.runCurrent()
    }

    /** 记录已提交的页边界（按起点升序）。 */
    fun orderedCommits(): List<BoundaryRecord> = abi.commitRecords.sortedBy { it.startByte }

    /**
     * 断言**同一排版世代内**的页边界链首尾相接（无空隙、无重叠、同一起点不重复登记）。
     *
     * 跨世代比较无意义：核心在签名变化时会整体丢弃旧世代的边界（`set_layout`），
     * 因此不同 `layout_id` 的记录各成一条链。
     */
    fun assertContiguousChain() {
        abi.commitRecords.groupBy { it.layoutId }.forEach { (layoutId, records) ->
            val ordered = records.sortedBy { it.startByte }
            ordered.zipWithNext { previous, next ->
                check(previous.endByte == next.startByte) {
                    "世代 $layoutId 页边界链断裂：${previous.startByte}→${previous.endByte} 之后是 ${next.startByte}"
                }
            }
            val starts = ordered.map { it.startByte }
            check(starts.size == starts.distinct().size) {
                "世代 $layoutId 同一起点被登记了两条边界：$starts"
            }
        }
    }

    /** 某一世代已登记的页起点（按起点升序）。 */
    fun startsOf(layoutId: Long): List<Long> =
        abi.commitRecords.filter { it.layoutId == layoutId }.map { it.startByte }.sorted()

    companion object {
        const val VIEWPORT_WIDTH_PX = 400

        /** 视口高度 = 内容区 108 + 上下边距 2×16（默认 16dp、dpToPx=1）。 */
        const val VIEWPORT_HEIGHT_PX = 140

        /** 章节 0 的字节长度（21 + 6×31）。 */
        const val CHAPTER_ZERO_BYTES = 207L

        /** 第一页终点（标题行 21 字节 + 2 个正文行 62 字节）。 */
        const val PAGE_ONE_END = 83L

        /** 第二页终点（3 个正文行 = 93 字节）。 */
        const val PAGE_TWO_END = 176L

        /** 三个整行正文页的字节数（10 汉字 + 换行 = 31 字节 × 3）。 */
        const val BODY_PAGE_BYTES = 93L
    }
}

/** 忙等轮询：真实线程测试里等待状态收敛（真机/线程场景无法用虚拟时间）。 */
internal suspend fun awaitUntil(timeoutMillis: Long = 5_000L, condition: () -> Boolean): Boolean {
    val deadline = System.currentTimeMillis() + timeoutMillis
    while (System.currentTimeMillis() < deadline) {
        if (condition()) {
            return true
        }
        delay(5L)
    }
    return condition()
}

/** 字节偏移是否落在合法 UTF-8 字符边界（后续字节不是续接字节 0b10xxxxxx）。 */
internal fun isUtf8Boundary(bytes: ByteArray, offset: Long): Boolean {
    if (offset <= 0L) {
        return true
    }
    if (offset >= bytes.size.toLong()) {
        return true
    }
    return (bytes[offset.toInt()].toInt() and 0xC0) != 0x80
}

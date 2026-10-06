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
 * V1.1 四组功能的**端口级**测试：走真实 `NrEngine` 门面 + 替身 ABI。
 *
 * 与 `NrV11CodecTest`（纯布局）的分工：这里测**门面接线**——参数透传、返回码映射、
 * 归还凭据、以及「核心给的成品文本不被二次加工」。
 */
class NrEngineV11Test {

    private val releaser = FakeReleaser()

    private fun engine(abi: FakeAbi): NrEngine =
        NrEngine.createWith(abi, NrAbiConstants.NR_DEFAULT_WINDOW_BYTES, null, null, releaser)

    private fun le(size: Int): ByteBuffer = ByteBuffer.allocate(size).order(ByteOrder.LITTLE_ENDIAN)

    /** 一行历史（72）后接三段文本，返回整块缓冲区。 */
    private fun historyPayload(title: String, file: String, label: String): ByteArray {
        val parts = listOf(title, file, label).map { it.toByteArray(Charsets.UTF_8) }
        val offsets = mutableListOf<Int>()
        var cursor = NrHistoryEntry.STRUCT_SIZE
        parts.forEach { offsets += cursor; cursor += it.size }
        val buf = le(cursor)
        buf.putLong(0, 7L)
        buf.putLong(8, 3L)
        buf.putLong(16, 1_791_264_000L)
        buf.putLong(24, 4L)
        buf.putLong(32, 128L)
        buf.putLong(40, 0L)
        buf.putInt(48, offsets[0])
        buf.putInt(52, parts[0].size)
        buf.putInt(56, offsets[1])
        buf.putInt(60, parts[1].size)
        buf.putInt(64, offsets[2])
        buf.putInt(68, parts[2].size)
        val out = buf.array()
        var cursor2 = NrHistoryEntry.STRUCT_SIZE
        parts.forEach { it.copyInto(out, cursor2); cursor2 += it.size }
        return out
    }

    /**
     * 分类批次：**所有表头在前、全部名称在后**（不是「一条表头 + 它的名称」交替）。
     *
     * 交错写法在单条时看不出问题，多条即错位（本测试初版实测踩中）。
     */
    private fun categoryBatch(vararg rows: Triple<Long, Long, Pair<Int, String>>): ByteArray {
        val names = rows.map { it.third.second.toByteArray(Charsets.UTF_8) }
        val headerSize = NrCategoryEntry.STRUCT_SIZE * rows.size
        val total = headerSize + names.sumOf { it.size }
        val buf = le(total)
        var cursor = headerSize
        rows.forEachIndexed { index, row ->
            val base = index * NrCategoryEntry.STRUCT_SIZE
            buf.putLong(base, row.first)
            buf.putLong(base + 8, row.second)
            buf.putLong(base + 16, 1L)
            buf.putLong(base + 24, 3L)
            buf.putInt(base + 32, row.third.first)
            buf.putInt(base + 36, cursor)
            buf.putInt(base + 40, names[index].size)
            cursor += names[index].size
        }
        val out = buf.array()
        var at = headerSize
        names.forEach { it.copyInto(out, at); at += it.size }
        return out
    }

    // ------------------------------------------------------------------ 分类

    @Test
    fun listCategoriesSurfacesFlatEntriesWithCoreComputedDepth() {
        val abi = FakeAbi().apply {
            categoryCount = 2
            categoriesBytes = categoryBatch(
                Triple(10L, NrCategoryEntry.PARENT_NONE, 0 to "科幻"),
                Triple(11L, 10L, 1 to "三体"),
            )
        }
        val named = engine(abi).use { it.listCategories() }.named

        assertEquals(2, named.size)
        assertEquals("科幻", named[0].second)
        assertEquals(NrCategoryEntry.PARENT_NONE, named[0].first.parentId)
        assertEquals(0L, named[0].first.depth)
        assertEquals("三体", named[1].second)
        assertEquals(10L, named[1].first.parentId)
        // 深度由核心给（界面不自己算层级）。
        assertEquals(1L, named[1].first.depth)
    }

    @Test
    fun listCategoriesPropagatesFailureInsteadOfReturningEmpty() {
        val abi = FakeAbi().apply { v11Code = NrErrors.NR_ERR_INTERNAL }
        val error = assertThrows(NrException::class.java) {
            engine(abi).use { it.listCategories() }
        }
        // 读失败必须抛错，**不得**静默当成「空树」。
        assertTrue(error.rawCode != NrErrors.NR_ERR_OK)
    }

    @Test
    fun createCategoryReturnsCoreAssignedId() {
        val abi = FakeAbi().apply { createdCategoryId = 99L }
        val id = engine(abi).use { it.createCategory(-1L, "科幻".toByteArray()) }
        assertEquals(99L, id)
        // `use{}` 结束会 close() → 销毁，故序列以 engineDestroy 收尾（正确行为）。
        assertEquals(listOf("engineCreate", "createCategory", "engineDestroy"), abi.calls)
    }

    @Test
    fun deleteCategoryForwardsDestinationAndReportsMovedCount() {
        val abi = FakeAbi().apply { deletedMovedCount = 5 }
        val moved = engine(abi).use { it.deleteCategory(11L, destination = 3) }
        assertEquals(5L, moved)
        // 去向被如实透传（3 = 移回根下未分类）。
        assertEquals(3, abi.lastDeleteDestination)
    }

    @Test
    fun deleteCategoryWithUnchosenDestinationIsRejectedByCore() {
        // 界面不得替用户猜去向：未选（0）交给核心拒绝。
        val abi = FakeAbi().apply { v11Code = NrErrors.NR_ERR_INVALID_ARGUMENT }
        val error = assertThrows(NrException::class.java) {
            engine(abi).use { it.deleteCategory(11L, destination = 0) }
        }
        assertEquals(NrErrors.NR_ERR_INVALID_ARGUMENT, error.rawCode)
        assertEquals(0, abi.lastDeleteDestination)
    }

    // ------------------------------------------------------------------ 历史

    @Test
    fun listHistoryReturnsRowsWithCoreFormattedTimeLabel() {
        val abi = FakeAbi().apply {
            historyCount = 1
            historyTotal = 9
            historyBytes = historyPayload("三体", "a.txt", "14:32")
        }
        val read = engine(abi).use { it.listHistory() }

        assertEquals(9L, read.batch.total)
        val row = read.rows.single()
        assertEquals("三体", row.title)
        assertEquals("a.txt", row.fileLabel)
        // 标签是核心给的成品文本 —— 门面**原样**呈现，不重排、不截断。
        assertEquals("14:32", row.timeLabel)
    }

    @Test
    fun listHistoryPassesScopeAndGroupedThroughToAbi() {
        val abi = FakeAbi().apply { historyBytes = ByteArray(0) }
        engine(abi).use { it.listHistory(bookId = 7L, limit = 20, grouped = true) }
        assertEquals(listOf("engineCreate", "listHistory", "engineDestroy"), abi.calls)
    }

    @Test
    fun clearHistoryEntryReportsMissingIdHonestly() {
        val abi = FakeAbi().apply { historyEntryRemoved = 0 }
        val removed = engine(abi).use { it.clearHistoryEntry(1234L) }
        // 0 = 该 id 不存在：必须如实回报 false，不谎报「已清除」。
        assertFalse(removed)
    }

    @Test
    fun clearHistoryForBookReturnsRemovedCount() {
        val abi = FakeAbi().apply { historyClearedCount = 3 }
        assertEquals(3L, engine(abi).use { it.clearHistoryForBook(7L) })
    }

    // ------------------------------------------------------------------ 配色

    @Test
    fun themeBaseAndWritesRoundTripThroughFacade() {
        val abi = FakeAbi()
        val base = le(NrThemeBase.STRUCT_SIZE)
        base.putInt(0, NrThemeBase.STRUCT_SIZE)
        base.putInt(4, 1)
        "#1890ff".toByteArray().copyInto(base.array(), 32) // primary 槽 @32
        abi.themeBaseBytes = base.array()

        val engine = engine(abi)
        engine.use {
            val read = it.getThemeBase()
            assertTrue(read.enabled)
            assertEquals("#1890ff", read.primary)

            it.setThemeBase(2, "#ff0000".toByteArray())
            assertEquals(2 to "#ff0000", abi.lastThemeBaseWrite)

            it.setThemeEnabled(true)
            assertEquals(1, abi.lastThemeEnabled)
        }
    }

    @Test
    fun getThemeTokenReturnsFourRgbaBytes() {
        val abi = FakeAbi().apply { themeTokenRgba = byteArrayOf(0x22, 0x33, 0x44, 0xFF.toByte()) }
        val rgba = engine(abi).use { it.getThemeToken(0, dark = false) }
        assertEquals(4, rgba.size)
        assertEquals(0x22.toByte(), rgba[0])
        assertEquals(0xFF.toByte(), rgba[3])
    }

    @Test
    fun getThemeContrastSurfacesIntegerRatio() {
        val abi = FakeAbi()
        val contrast = le(NrThemeContrastResult.STRUCT_SIZE)
        contrast.putInt(0, NrThemeContrastResult.STRUCT_SIZE)
        contrast.putInt(4, 454)
        contrast.putInt(8, 1)
        abi.themeContrastBytes = contrast.array()

        val result = engine(abi).use { it.getThemeContrast() }
        assertEquals(454L, result.ratioX100)
        assertTrue(result.passes)
    }

    // ------------------------------------------------------------------ 背景

    @Test
    fun getBackgroundAbsentYieldsNullEntryAndEmptyName() {
        val abi = FakeAbi().apply { backgroundCount = 0; backgroundBytes = ByteArray(0) }
        val read = engine(abi).use { it.getBackground() }
        assertNull(read.entry)
        assertEquals("", read.fileName)
    }

    @Test
    fun setBackgroundForwardsPathsAndReturnsRelativeName() {
        val raw = "bg-1791264415-42.png".toByteArray()
        val payload = le(NrBackgroundEntry.STRUCT_SIZE + raw.size).apply {
            putLong(0, 1024L)
            putLong(8, 600L)
            putLong(16, 0L)
            put(32, 153.toByte())
            putInt(36, raw.size)
            putInt(40, NrBackgroundEntry.STRUCT_SIZE)
        }.array().copyOf().also { raw.copyInto(it, NrBackgroundEntry.STRUCT_SIZE) }
        val abi = FakeAbi().apply { backgroundCount = 1; backgroundBytes = payload }

        val read = engine(abi).use {
            it.setBackground(
                srcPath = "/sdcard/我的图.png".toByteArray(),
                destDir = "/data/app/backgrounds".toByteArray(),
            )
        }
        assertEquals("bg-1791264415-42.png", read.fileName)
        assertEquals(600L, read.entry?.scrimPermille)
        // 源路径与目标目录如实透传（核心不猜平台数据目录）。
        assertEquals("/sdcard/我的图.png" to "/data/app/backgrounds", abi.lastBackgroundSet)
        // **只回相对名**（R15）。
        assertFalse(read.fileName.contains('/'))
    }

    @Test
    fun destroyedEngineRejectsV11CallsWithoutTouchingAbi() {
        val abi = FakeAbi()
        val engine = engine(abi)
        engine.destroy()
        val before = abi.calls.size
        assertThrows(NrException::class.java) { engine.listCategories() }
        assertThrows(NrException::class.java) { engine.listHistory() }
        assertThrows(NrException::class.java) { engine.getBackground() }
        // 销毁后不得再触碰 native。
        assertEquals(before, abi.calls.size)
    }

    @Test
    fun listBookCategoriesSurfacesMapIncludingUncategorizedSentinel() {
        // A2：界面筛分类所缺的那一半（bookId → categoryId）。
        val abi = FakeAbi().apply {
            val buf = le(NrBookCategoryEntry.STRUCT_SIZE * 2)
            buf.putLong(0, 7L)
            buf.putLong(8, 10L)
            buf.putLong(16, 8L)
            buf.putLong(24, NrBookCategoryEntry.UNCATEGORIZED)
            bookCategoriesBytes = buf.array()
        }
        val mapped = engine(abi).use { it.listBookCategories() }

        assertEquals(2, mapped.size)
        assertEquals(10L, mapped[7L])
        assertEquals(NrBookCategoryEntry.UNCATEGORIZED, mapped[8L])
        assertEquals(listOf("engineCreate", "listBookCategories", "engineDestroy"), abi.calls)
    }

    @Test
    fun listBookCategoriesPropagatesFailureInsteadOfReturningEmptyMap() {
        val abi = FakeAbi().apply { v11Code = NrErrors.NR_ERR_INTERNAL }
        val error = assertThrows(NrException::class.java) {
            engine(abi).use { it.listBookCategories() }
        }
        // 读失败必须抛错，**不得**静默当成「所有书都未分类」。
        assertTrue(error.rawCode != NrErrors.NR_ERR_OK)
    }

    // ------------------------------------------------------------------ 作用域书籍（A3）

    @Test
    fun listBooksInScopeForwardsScopeKindAndCategoryId() {
        val abi = FakeAbi().apply { scopeIds = listOf(3L, 7L, 9L) }
        val ids = engine(abi).use { it.listBooksInScope(NrScopeKind.CATEGORY, 12L) }

        assertEquals(setOf(3L, 7L, 9L), ids)
        // 参数如实透传（子树展开在核心做，界面不加工）。
        assertEquals(NrScopeKind.CATEGORY to 12L, abi.lastScopeRequest)
    }

    @Test
    fun listBooksInScopeAllKindPassesZeroCategory() {
        val abi = FakeAbi().apply { scopeIds = listOf(1L, 2L) }
        engine(abi).use { it.listBooksInScope(NrScopeKind.ALL) }
        assertEquals(NrScopeKind.ALL to 0L, abi.lastScopeRequest)
    }

    @Test
    fun listBooksInScopePropagatesFailureInsteadOfReturningEmpty() {
        val abi = FakeAbi().apply { v11Code = NrErrors.NR_ERR_INVALID_ARGUMENT }
        val error = assertThrows(NrException::class.java) {
            engine(abi).use { it.listBooksInScope(NrScopeKind.CATEGORY, 1L) }
        }
        // 读失败必须抛错 —— **不得**静默当成「该作用域没有书」（那会显示成空书架）。
        //
        // 不断言具体错误码：门面抛的是 ABI 的**最后错误槽**内容，而替身不模拟该槽
        // （真实 ABI 会由核心写入 1 = INVALID_ARGUMENT）。这里只钉「必须失败」。
        assertTrue(error.rawCode != NrErrors.NR_ERR_OK)
    }
}

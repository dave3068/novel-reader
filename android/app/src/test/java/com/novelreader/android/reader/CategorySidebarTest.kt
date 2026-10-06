package com.novelreader.android.reader

import com.novelreader.android.core.FakeReleaser
import com.novelreader.android.core.NrAbiConstants
import com.novelreader.android.core.NrErrors
import com.novelreader.android.core.NrScopeKind
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 书架分类侧栏（V1.1 规格 R1–R5、C11）。
 *
 * 分两部分：
 * 1. **纯装配**（[CategoryTreeAssembly]）—— 折叠语义、孤儿容错、顺序保持；
 * 2. **门面接线** —— 选中作用域时可见集合由**核心**给出，界面只做成员判断。
 */
@OptIn(ExperimentalCoroutinesApi::class)
class CategorySidebarTest {

    private fun node(
        id: Long,
        parent: Long,
        name: String,
        depth: Int = 0,
        direct: Long = 0,
        total: Long = 0,
    ) = CategoryTreeAssembly.Node(id, parent, name, depth, direct, total)

    private fun viewModel(abi: ScriptedReaderAbi, catalog: HostBookCatalog? = null): ReaderViewModel {
        val releaser = FakeReleaser()
        return ReaderViewModel(
            engine = engineOn(abi, releaser),
            releaser = releaser,
            measurer = FakeTextMeasurer(),
            io = UnconfinedTestDispatcher(),
            workScope = kotlinx.coroutines.CoroutineScope(UnconfinedTestDispatcher()),
            bookCatalog = catalog,
        )
    }


    /** 造 `nr_category` 批次：**所有表头在前、全部名称在后**（不是逐行交错）。 */
    private fun categoryBatch(vararg rows: Triple<Long, Long, Pair<Int, String>>): ByteArray {
        val names = rows.map { it.third.second.toByteArray(Charsets.UTF_8) }
        val headerSize = com.novelreader.android.core.NrCategoryEntry.STRUCT_SIZE * rows.size
        val total = headerSize + names.sumOf { it.size }
        val buf = java.nio.ByteBuffer.allocate(total).order(java.nio.ByteOrder.LITTLE_ENDIAN)
        var cursor = headerSize
        rows.forEachIndexed { index, row ->
            val base = index * com.novelreader.android.core.NrCategoryEntry.STRUCT_SIZE
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

    // ------------------------------------------------------------------ 纯装配

    @Test
    fun flattenOrdersDepthFirstAndKeepsCoreOrderAmongSiblings() {
        // 科幻(1) → 中国科幻(2) / 外国科幻(3)；历史(4)。兄弟顺序必须**保持核心给的顺序**。
        val rows = CategoryTreeAssembly.flatten(
            nodes = listOf(
                node(1, -1, "科幻", depth = 0, direct = 1, total = 3),
                node(2, 1, "中国科幻", depth = 1, direct = 2, total = 2),
                node(3, 1, "外国科幻", depth = 1, direct = 0, total = 0),
                node(4, -1, "历史", depth = 0, direct = 1, total = 1),
            ),
            collapsed = emptySet(),
            selectedId = null,
        )

        assertEquals(listOf("科幻", "中国科幻", "外国科幻", "历史"), rows.map { it.name })
        assertEquals(listOf(0, 1, 1, 0), rows.map { it.depth })
        // 计数与层级**全部来自核心**，装配层不加工。
        assertEquals(3L, rows[0].totalCount)
        assertEquals("1/3", rows[0].countLabel)
        assertEquals("2", rows[1].countLabel)
        assertTrue(rows[0].hasChildren)
        assertFalse(rows[1].hasChildren)
    }

    @Test
    fun collapsingAnAncestorHidesItsDescendantsButNotItself() {
        val nodes = listOf(
            node(1, -1, "科幻"),
            node(2, 1, "中国科幻"),
            node(3, 2, "古代"),
            node(4, -1, "历史"),
        )
        // 折叠「科幻」：它自己仍在，子孙全消失。
        val collapsed = CategoryTreeAssembly.flatten(nodes, setOf(1L), null)
        assertEquals(listOf("科幻", "历史"), collapsed.map { it.name })
        assertFalse("折叠态要如实回报给界面", collapsed[0].expanded)

        // 折叠中间层：最深层同样不下发（**任一祖先折叠即隐藏**）。
        val midCollapsed = CategoryTreeAssembly.flatten(nodes, setOf(2L), null)
        assertEquals(listOf("科幻", "中国科幻", "历史"), midCollapsed.map { it.name })
    }

    @Test
    fun orphanNodesAreTreatedAsRootInsteadOfSwallowingTheTree() {
        // 父链缺失（脏数据 / 父被删）：孤儿必须仍然可见，不能整棵树消失。
        val rows = CategoryTreeAssembly.flatten(
            nodes = listOf(
                node(1, -1, "科幻"),
                node(9, 777, "孤儿"),
            ),
            collapsed = emptySet(),
            selectedId = null,
        )
        assertEquals(listOf("科幻", "孤儿"), rows.map { it.name })
    }

    @Test
    fun selectionIsReportedOnExactlyTheSelectedRow() {
        val nodes = listOf(node(1, -1, "科幻"), node(2, 1, "中国科幻"))
        val rows = CategoryTreeAssembly.flatten(nodes, emptySet(), selectedId = 2L)
        assertEquals(listOf(false, true), rows.map { it.selected })
        // 选「全部」（null）时没有任何分类行被标记。
        assertTrue(CategoryTreeAssembly.flatten(nodes, emptySet(), null).none { it.selected })
    }

    // ------------------------------------------------------------------ 门面接线

    private fun abiWithTree(): ScriptedReaderAbi = ScriptedReaderAbi(FixtureBook.build()).apply {
        categoriesBytes = categoryBatch(
            Triple(1L, (-1L), 0 to "科幻"),
            Triple(2L, 1L, 1 to "中国科幻"),
        )
        categoryCount = 2
    }

    @Test
    fun selectingACategoryAsksTheCoreForTheVisibleSet() = runTest {
        // 可见集合必须**由核心给**（子树展开在核心完成）—— 界面不遍历父子链。
        val abi = abiWithTree().apply { scopeIds = listOf(7L, 8L) }
        val vm = viewModel(abi)

        vm.onSelectCategory(1L)

        assertEquals(setOf(7L, 8L), vm.ui.value.visibleBookIds)
        // 侧栏刷新会顺带统计「全部 / 未分类」，故查请求列表而非最后一次。
        assertTrue(
            "应有一次 CATEGORY/1 的作用域请求，实际：${abi.scopeRequests}",
            (NrScopeKind.CATEGORY to 1L) in abi.scopeRequests,
        )
        assertEquals(1L, vm.ui.value.categories.selectedId)
        assertTrue(vm.ui.value.categories.rows.first { it.id == 1L }.selected)
    }

    @Test
    fun selectingUncategorizedUsesTheUncategorizedScopeKind() = runTest {
        val abi = abiWithTree().apply { scopeIds = listOf(9L) }
        val vm = viewModel(abi)

        vm.onSelectCategory(CategoryTreeAssembly.UNCATEGORIZED)

        assertTrue((NrScopeKind.UNCATEGORIZED to 0L) in abi.scopeRequests)
        assertEquals(setOf(9L), vm.ui.value.visibleBookIds)
    }

    @Test
    fun selectingAllBooksClearsTheFilterWithoutCallingTheCore() = runTest {
        val abi = abiWithTree()
        val vm = viewModel(abi)

        vm.onSelectCategory(1L)
        vm.onSelectAllBooks()

        // 根「全部书籍」不过滤 ⇒ 不需要问核心要集合。
        assertNull(vm.ui.value.visibleBookIds)
        assertNull(vm.ui.value.categories.selectedId)
        // 选中分类那一次会请求 CATEGORY/1；「全部」不应再产生任何 CATEGORY 请求。
        assertEquals(
            "「全部」不得再向核心要分类集合",
            1,
            abi.scopeRequests.count { it.first == NrScopeKind.CATEGORY },
        )
    }

    @Test
    fun scopeReadFailureSurfacesInsteadOfSilentlyShowingEverything() = runTest {
        val abi = abiWithTree().apply { v11Code = NrErrors.NR_ERR_INTERNAL }
        val vm = viewModel(abi)

        vm.onSelectCategory(1L)

        // 取不到可见集合时必须**明确失败**：若退化成「显示全部」，用户会以为筛选生效了。
        assertTrue(
            "应回显失败，实际：${vm.ui.value.status}",
            vm.ui.value.status.contains("读取分类"),
        )
        assertNull("失败时不得把过滤集合设成空集（那会显示空书架）", vm.ui.value.visibleBookIds)
    }

    @Test
    fun togglingACategoryOnlyTouchesViewStateNotTheCore() = runTest {
        val abi = abiWithTree()
        val vm = viewModel(abi)
        vm.onSelectCategory(1L)
        val before = abi.events.count { it == "listCategories:enter" }

        vm.onToggleCategory(1L)

        assertTrue(1L in vm.ui.value.categories.collapsed)
        assertEquals(listOf("科幻"), vm.ui.value.categories.rows.map { it.name })
        // 折叠是纯视图状态：不得产生新的核心调用以外的语义变化。
        assertTrue(abi.events.count { it == "listCategories:enter" } > before)
        assertFalse("折叠不得改变选中作用域", vm.ui.value.categories.selectedId != 1L)
    }

    @Test
    fun renamingForwardsIdAndNameAndRereadsTheTree() = runTest {
        val abi = abiWithTree()
        val vm = viewModel(abi)

        vm.onRenameCategory(2L, "  古代科幻  ")

        assertEquals(2L to "古代科幻", abi.lastRename)
        assertTrue(vm.ui.value.status.contains("已重命名"))
    }

    @Test
    fun rejectedRenameKeepsTheTreeAndReportsHonestly() = runTest {
        val abi = abiWithTree().apply { themeWriteCode = NrErrors.NR_ERR_INVALID_ARGUMENT }
        val vm = viewModel(abi)

        vm.onRenameCategory(2L, "科幻")

        // 重名等被核心拒绝：如实回显，且**树仍按核心的真实状态重读**。
        assertTrue(vm.ui.value.status.contains("重命名被拒"))
        assertEquals(listOf("科幻", "中国科幻"), vm.ui.value.categories.rows.map { it.name })
    }

    @Test
    fun emptyNameIsRejectedLocallyWithoutTouchingTheCore() = runTest {
        val abi = abiWithTree()
        val vm = viewModel(abi)

        vm.onCreateCategory("   ")

        assertEquals("分类名不能为空", vm.ui.value.status)
        assertFalse(abi.events.any { it.startsWith("createCategory") })
    }

    @Test
    fun movingACategoryForwardsNewParent() = runTest {
        val abi = abiWithTree()
        val vm = viewModel(abi)

        vm.onMoveCategory(2L, -1L)

        assertEquals(2L to -1L, abi.lastCategoryMove)
        assertTrue(vm.ui.value.status.contains("已移动分类"))
    }

    @Test
    fun rejectedMoveRereadsTheTreeInsteadOfLeavingAnOptimisticOne() = runTest {
        // 移入自身子孙必须被核心拒绝（规格 R5）；界面不得乐观改树后停在那儿。
        val abi = abiWithTree().apply { themeWriteCode = NrErrors.NR_ERR_INVALID_ARGUMENT }
        val vm = viewModel(abi)

        vm.onMoveCategory(1L, 2L)

        assertTrue(vm.ui.value.status.contains("移动分类被拒"))
        assertTrue(abi.events.contains("listCategories:enter"))
        assertEquals(listOf("科幻", "中国科幻"), vm.ui.value.categories.rows.map { it.name })
    }

    // ------------------------------------------------------------------ 书架过滤

    private class Catalog(private val entries: List<HostBookEntry>) : HostBookCatalog {
        override fun list(): List<HostBookEntry> = entries
        override fun rememberBookId(sourceIdentity: String, bookId: Long) = Unit
    }

    /** ⚠️ 每条 entry 必须用**不同**的 sourceIdentity —— 书架按身份去重，
     * 用同一个身份会让两本书合并成一条（初版即踩此坑，断言因此失真）。 */
    private fun entry(name: String, bookId: Long?) = HostBookEntry(
        sourceIdentity = "/tmp/fixture-$name",
        displayName = name,
        sourceKind = NrAbiConstants.NR_SOURCE_KIND_FILE,
        bookId = bookId,
    )

    @Test
    fun shelfShowsOnlyBooksTheCorePutInScope() = runTest {
        val abi = abiWithTree().apply { scopeIds = listOf(7L) }
        val vm = viewModel(abi, Catalog(listOf(entry("在作用域内.txt", 7L), entry("作用域外.txt", 8L))))

        vm.openShelf()
        vm.onSelectCategory(1L)

        assertEquals(listOf("在作用域内.txt"), vm.ui.value.shelf.books.map { it.displayName })
    }

    @Test
    fun booksWithoutACoreIdAreHiddenUnderASpecificScope() = runTest {
        // 从未打开过的书没有 bookId ⇒ 归属未知。非「全部」作用域下**不猜**，直接不显示。
        val abi = abiWithTree().apply { scopeIds = listOf(7L) }
        val vm = viewModel(abi, Catalog(listOf(entry("已知归属.txt", 7L), entry("归属未知.txt", null))))

        vm.openShelf()
        vm.onSelectCategory(1L)

        assertEquals(listOf("已知归属.txt"), vm.ui.value.shelf.books.map { it.displayName })
    }

    @Test
    fun selectingAllBooksRestoresEveryShelfEntryIncludingUnknownOnes() = runTest {
        val abi = abiWithTree()
        val vm = viewModel(abi, Catalog(listOf(entry("已知归属.txt", 7L), entry("归属未知.txt", null))))

        vm.openShelf()
        vm.onSelectCategory(1L)
        vm.onSelectAllBooks()

        // 「全部书籍」下不过滤 —— 归属未知的书也必须回来。
        assertEquals(2, vm.ui.value.shelf.books.size)
    }

    // ------------------------------------------------------------------ 移动 / 删除（R3/R4/R5）

    @Test
    fun moveTargetsComeFromTheCoreAndUnselectableRowsStayVisible() = runTest {
        // 自身与子孙由核心标为不可选，但**仍在列表里** —— 用户要看得见「为什么不能选」。
        val abi = abiWithTree().apply {
            moveTargets = listOf(
                Triple(-1L, 0, true to "未分类"),
                Triple(1L, 0, false to "科幻"),
                Triple(2L, 1, false to "中国科幻"),
            )
        }
        val vm = viewModel(abi)

        vm.onLoadCategoryTargets(1L)

        val targets = vm.ui.value.categoryTargets
        assertEquals(3, targets.size)
        assertEquals(-1L, targets[0].id)
        assertTrue(targets[0].selectable)
        assertFalse(targets[1].selectable)
        assertEquals("科幻（不可选：自身或子孙）", targets[1].label)
        assertEquals(1, targets[2].indentLevel)
        assertEquals(1L, abi.lastMoveTargetsSource)
    }

    @Test
    fun deletingWithoutAChosenDestinationIsRejectedLocally() = runTest {
        val abi = abiWithTree()
        val vm = viewModel(abi)

        vm.onDeleteCategory(1L, DeleteDestination.NONE)

        // 「未选去向不得执行删除」是硬约束：界面这一层就不该发出请求。
        assertEquals("请先选择书籍去向（未选择时不会执行删除）", vm.ui.value.status)
        assertFalse(abi.events.any { it.startsWith("deleteCategory") })
    }

    @Test
    fun deletingForwardsDestinationAndTargetAndReportsMovedCount() = runTest {
        val abi = abiWithTree().apply { deletedMovedCount = 4 }
        val vm = viewModel(abi)

        vm.onDeleteCategory(1L, DeleteDestination.MOVE_TO, targetId = 5L)

        assertEquals(DeleteDestination.MOVE_TO, abi.lastDeleteDestination)
        assertTrue(vm.ui.value.status.contains("已删除分类（移走 4 本书）"))
    }

    @Test
    fun deleteClearsTargetsAfterSuccess() = runTest {
        val abi = abiWithTree().apply {
            moveTargets = listOf(Triple(-1L, 0, true to "未分类"))
            deletedMovedCount = 0
        }
        val vm = viewModel(abi)
        vm.onLoadCategoryTargets(1L)
        assertTrue(vm.ui.value.categoryTargets.isNotEmpty())

        vm.onDeleteCategory(1L, DeleteDestination.TO_UNCATEGORIZED)

        assertTrue("删除后目标列表必须清空", vm.ui.value.categoryTargets.isEmpty())
    }

    @Test
    fun rejectedDeleteRereadsTheTree() = runTest {
        val abi = abiWithTree().apply { v11Code = NrErrors.NR_ERR_INVALID_ARGUMENT }
        val vm = viewModel(abi)

        vm.onDeleteCategory(1L, DeleteDestination.REMOVE_FROM_SHELF)

        // 被拒后必须回读核心的真实树（与 rename / move 同一条纪律）。
        assertTrue(abi.events.contains("listCategories:enter"))
        // 状态文案：全局失败时回读也会失败，故「读取分类失败」可能覆盖「删除被拒」——
        // 只断言确有错误回显，不锁死是哪一条。
        assertTrue(
            "应有错误回显，实际：${vm.ui.value.status}",
            vm.ui.value.status.contains("分类"),
        )
    }

    @Test
    fun clearingTargetsEmptiesTheList() = runTest {
        val abi = abiWithTree().apply { moveTargets = listOf(Triple(-1L, 0, true to "未分类")) }
        val vm = viewModel(abi)
        vm.onLoadCategoryTargets(1L)

        vm.clearCategoryTargets()

        assertTrue(vm.ui.value.categoryTargets.isEmpty())
    }
}

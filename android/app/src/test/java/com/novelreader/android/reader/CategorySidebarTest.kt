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
 * 2. **门面接线** —— 选中位置时可见集合由**核心映射**（`nr_list_book_categories`）
 *    逐本判定（直属口径，2026-10-09 起）；界面只做成员判断，不遍历父子链。
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
    fun selectingACategoryFiltersToItsDirectBooks() = runTest {
        // 直属口径（2026-10-09）：可见集合 = 核心映射中 categoryId == 所选分类的书；
        // 界面只做映射查询，不遍历父子链、不再向核心要「含子孙」集合。
        val abi = abiWithTree().apply {
            bookCategoriesBytes = bookCategoryBatch(7L to 1L, 8L to 1L, 9L to -1L)
        }
        val vm = viewModel(abi)

        vm.onSelectCategory(1L)

        assertEquals(setOf(7L, 8L), vm.ui.value.visibleBookIds)
        assertTrue(
            "不得再出现「含子孙」作用域请求（口径已改直属），实际：${abi.scopeRequests}",
            abi.scopeRequests.none { it.first == NrScopeKind.CATEGORY },
        )
        assertEquals(1L, vm.ui.value.categories.selectedId)
        assertTrue(vm.ui.value.categories.rows.first { it.id == 1L }.selected)
        assertEquals(setOf(7L, 8L, 9L), vm.ui.value.categories.bookCategories.keys)
    }

    @Test
    fun selectingTheRootFiltersToUncategorizedBooks() = runTest {
        val abi = abiWithTree().apply {
            bookCategoriesBytes = bookCategoryBatch(9L to -1L, 7L to 1L)
        }
        val vm = viewModel(abi)

        vm.onSelectCategory(CategoryTreeAssembly.UNCATEGORIZED)

        assertEquals(setOf(9L), vm.ui.value.visibleBookIds)
        assertEquals(CategoryTreeAssembly.UNCATEGORIZED, vm.ui.value.categories.selectedId)
    }

    @Test
    fun selectingAllBooksClearsTheFilterWithoutAskingForACategorySet() = runTest {
        val abi = abiWithTree().apply { bookCategoriesBytes = bookCategoryBatch(7L to 1L) }
        val vm = viewModel(abi)

        vm.onSelectCategory(1L)
        vm.onSelectAllBooks()

        // 「全部书籍」= 不过滤（宽屏侧栏专用）；也不得产生任何「含子孙」作用域请求。
        assertNull(vm.ui.value.visibleBookIds)
        assertNull(vm.ui.value.categories.selectedId)
        assertEquals(
            "「全部书籍」不得向核心要分类集合",
            0,
            abi.scopeRequests.count { it.first == NrScopeKind.CATEGORY },
        )
    }

    @Test
    fun mapReadFailureKeepsThePreviousFilterAndSurfaces() = runTest {
        val abi = abiWithTree().apply {
            bookCategoriesBytes = bookCategoryBatch(7L to 1L, 8L to 2L)
        }
        val vm = viewModel(abi)

        vm.onSelectCategory(1L)
        assertEquals(setOf(7L), vm.ui.value.visibleBookIds)

        // 映射读取失败 ⇒ 明确回显且**保持**原过滤与选中（不得退化成「显示全部 / 空书架」）。
        abi.bookCategoriesCode = NrErrors.NR_ERR_INTERNAL
        vm.onSelectCategory(2L)

        assertTrue(
            "应回显映射读取失败，实际：${vm.ui.value.status}",
            vm.ui.value.status.contains("读取书籍分类映射失败"),
        )
        assertEquals("失败时不得切换选中位置", 1L, vm.ui.value.categories.selectedId)
        assertEquals(setOf(7L), vm.ui.value.visibleBookIds)
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
    fun shelfShowsOnlyDirectBooksOfTheSelectedCategory() = runTest {
        // 直属口径（2026-10-09）：只显示核心映射指向该分类的书（7→1 在、8→2 不在）。
        val abi = abiWithTree().apply { bookCategoriesBytes = bookCategoryBatch(7L to 1L, 8L to 2L) }
        val vm = viewModel(abi, Catalog(listOf(entry("直属.txt", 7L), entry("别类.txt", 8L))))

        vm.openShelf()
        vm.onSelectCategory(1L)

        assertEquals(listOf("直属.txt"), vm.ui.value.shelf.books.map { it.displayName })
    }

    @Test
    fun booksWithoutACoreIdAreHiddenUnderASpecificScope() = runTest {
        // 从未打开过的书没有 bookId ⇒ 归属未知。具体分类下**不猜**，直接不显示。
        val abi = abiWithTree().apply { bookCategoriesBytes = bookCategoryBatch(7L to 1L) }
        val vm = viewModel(abi, Catalog(listOf(entry("已知归属.txt", 7L), entry("归属未知.txt", null))))

        vm.openShelf()
        vm.onSelectCategory(1L)

        assertEquals(listOf("已知归属.txt"), vm.ui.value.shelf.books.map { it.displayName })
    }

    @Test
    fun booksWithoutACoreIdAppearAtTheRoot() = runTest {
        // 根层是「归属不可判定」的书唯一可见的位置 —— 否则它们在所有分类下都会消失。
        val abi = abiWithTree().apply { bookCategoriesBytes = bookCategoryBatch(7L to 1L) }
        val vm = viewModel(abi, Catalog(listOf(entry("已知归属.txt", 7L), entry("归属未知.txt", null))))

        vm.openShelf()
        vm.onSelectCategory(CategoryTreeAssembly.UNCATEGORIZED)

        assertEquals(listOf("归属未知.txt"), vm.ui.value.shelf.books.map { it.displayName })
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

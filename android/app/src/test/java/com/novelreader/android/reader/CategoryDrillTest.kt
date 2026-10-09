package com.novelreader.android.reader

import com.novelreader.android.core.FakeReleaser
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 书架分类**下钻**（2026-10-09 文件管理器式；窄屏布局）。
 *
 * 分两部分：
 * 1. **纯装配**（[CategoryTreeAssembly] 的 `levelChildren` / `pathTo` / `parentOf` 与
 *    [ShelfLayout] 宽度判定）—— 边界由用例锁定；
 * 2. **门面接线** —— 逐级返回、悬空分类回落、移书链路（含未登记书的本地拒绝）。
 */
@OptIn(ExperimentalCoroutinesApi::class)
class CategoryDrillTest {

    private fun node(
        id: Long,
        parent: Long,
        name: String,
        depth: Int = 0,
        direct: Long = 0,
        total: Long = 0,
    ) = CategoryTreeAssembly.Node(id, parent, name, depth, direct, total)

    /** 科幻(1) → 中国科幻(2) / 外国科幻(3)；历史(4) —— 与侧栏测试同一形状。 */
    private val tree = listOf(
        node(1, -1, "科幻", 0, 1, 3),
        node(2, 1, "中国科幻", 1, 2, 2),
        node(3, 1, "外国科幻", 1, 0, 0),
        node(4, -1, "历史", 0, 1, 1),
    )

    // ------------------------------------------------------------------ 纯装配

    @Test
    fun levelChildrenAtRootAreTopLevelDirectoriesInCoreOrder() {
        // `null` 与 `-1`（根层/未分类哨兵）都取顶层目录，且保持核心给出的兄弟顺序。
        assertEquals(listOf("科幻", "历史"), CategoryTreeAssembly.levelChildren(tree, null).map { it.name })
        assertEquals(listOf("科幻", "历史"), CategoryTreeAssembly.levelChildren(tree, -1L).map { it.name })
        val first = CategoryTreeAssembly.levelChildren(tree, null).first()
        assertEquals("计数来自核心", "1/3", first.countLabel)
        assertTrue(first.hasChildren)
        assertFalse("无子目录的行不给假箭头", CategoryTreeAssembly.levelChildren(tree, null)[1].hasChildren)
    }

    @Test
    fun levelChildrenOfACategoryAreItsDirectChildren() {
        assertEquals(
            listOf("中国科幻", "外国科幻"),
            CategoryTreeAssembly.levelChildren(tree, 1L).map { it.name },
        )
        assertTrue("没有子目录不是错误", CategoryTreeAssembly.levelChildren(tree, 2L).isEmpty())
    }

    @Test
    fun orphanNodesStayReachableAtTheRootLevel() {
        // 父链缺失（脏数据 / 父被删）的孤儿必须仍在根层可见（与侧栏 flatten 同一规则）。
        val nodes = listOf(node(1, -1, "科幻"), node(9, 777, "孤儿"))
        assertEquals(listOf("科幻", "孤儿"), CategoryTreeAssembly.levelChildren(nodes, -1L).map { it.name })
    }

    @Test
    fun pathToWalksAncestorsFromRoot() {
        assertEquals(listOf("科幻", "中国科幻"), CategoryTreeAssembly.pathTo(tree, 2L).map { it.name })
        assertEquals(listOf("历史"), CategoryTreeAssembly.pathTo(tree, 4L).map { it.name })
        assertTrue("不存在的节点 ⇒ 空路径", CategoryTreeAssembly.pathTo(tree, 999L).isEmpty())
    }

    @Test
    fun pathToSurvivesCyclicDataWithoutHanging() {
        // 防御性：核心已防环，但脏数据不得让界面无限上溯。
        val cyclic = listOf(node(1, 2, "A"), node(2, 1, "B"))
        val path = CategoryTreeAssembly.pathTo(cyclic, 1L)
        assertEquals(2, path.size)
    }

    @Test
    fun parentOfReturnsNullForTopLevelAndMissing() {
        assertEquals(1L, CategoryTreeAssembly.parentOf(tree, 2L))
        assertNull("顶层没有真实父节点", CategoryTreeAssembly.parentOf(tree, 1L))
        assertNull(CategoryTreeAssembly.parentOf(tree, 999L))
    }

    @Test
    fun shelfLayoutIsCompactBelowSixHundredDp() {
        assertTrue(ShelfLayout.isCompact(360f))
        assertTrue(ShelfLayout.isCompact(599.9f))
        assertFalse("恰好 600dp 视为宽屏", ShelfLayout.isCompact(600f))
        assertFalse(ShelfLayout.isCompact(800f))
    }

    // ------------------------------------------------------------------ 门面接线

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

    private fun abiWithTree(): ScriptedReaderAbi = ScriptedReaderAbi(FixtureBook.build()).apply {
        categoriesBytes = categoryBatch(
            Triple(1L, (-1L), 0 to "科幻"),
            Triple(2L, 1L, 1 to "中国科幻"),
            Triple(4L, (-1L), 0 to "历史"),
        )
        categoryCount = 3
    }

    @Test
    fun navigateUpWalksToParentThenRoot() = runTest {
        val vm = viewModel(abiWithTree())
        vm.onSelectCategory(2L)

        vm.onNavigateUp()
        assertEquals(1L, vm.ui.value.categories.selectedId)

        vm.onNavigateUp()
        assertEquals(
            "顶部分类再向上 ⇒ 根层（-1）",
            CategoryTreeAssembly.UNCATEGORIZED,
            vm.ui.value.categories.selectedId,
        )

        // 根层不动（是否退出应用由宿主决定）。
        vm.onNavigateUp()
        assertEquals(CategoryTreeAssembly.UNCATEGORIZED, vm.ui.value.categories.selectedId)
    }

    @Test
    fun missingCategoryFallsBackToTheRootInsteadOfDangling() = runTest {
        val vm = viewModel(abiWithTree())

        vm.onSelectCategory(999L)

        assertEquals(CategoryTreeAssembly.UNCATEGORIZED, vm.ui.value.categories.selectedId)
    }

    @Test
    fun movingABookForwardsIdsAndReportsTheTarget() = runTest {
        val abi = abiWithTree().apply { bookCategoriesBytes = bookCategoryBatch(7L to -1L) }
        val vm = viewModel(abi)
        vm.onSelectCategory(1L)

        vm.onMoveBookToCategory(7L, 1L, "测试书")

        assertEquals(7L to 1L, abi.lastBookMove)
        assertTrue(
            "应回显「已移入」与目标名，实际：${vm.ui.value.status}",
            vm.ui.value.status.contains("已把《测试书》移入「科幻」"),
        )
    }

    @Test
    fun movingABookWithoutACoreIdIsRejectedLocally() = runTest {
        val abi = abiWithTree()
        val vm = viewModel(abi)

        vm.onMoveBookToCategory(0L, 1L)

        assertNull("未登记的书不得发出必然失败的请求", abi.lastBookMove)
        assertTrue(vm.ui.value.status.contains("尚未打开过"))
    }
}

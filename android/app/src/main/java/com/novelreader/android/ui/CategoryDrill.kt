package com.novelreader.android.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.novelreader.android.reader.CategoryRowView
import com.novelreader.android.reader.CategoryState
import com.novelreader.android.reader.CategoryTargetView
import com.novelreader.android.reader.CategoryTreeAssembly
import com.novelreader.android.reader.ReaderStatus
import com.novelreader.android.reader.ShelfBook
import com.novelreader.android.reader.ShelfState

/**
 * 窄屏书架：文件管理器式**下钻列表**（2026-10-09 主人确认）。
 *
 * # 结构（同一列表内）
 *
 * - 当前目录的**子目录在上、书籍在下**；点目录行进入下一层；
 * - 「← 返回××」行 + 系统返回键逐级回退（顶部分类 ⇒ 根层；根层 ⇒ 交给系统）；
 * - 根层 = 顶层目录 + 未分类书籍；每个书**只出现一次**（直属口径）。
 *
 * # 口径
 *
 * - 目录行与路径来自 [CategoryTreeAssembly]（核心节点装配；界面不遍历父链、不自行派生语义）；
 * - 书籍列表由 ViewModel 按直属口径过滤好，本层只渲染；
 * - 窄屏没有「全部书籍」位置：启动或从宽屏旋转过来时**归一化到根层**（`-1`）；
 * - 「新建分类」建在**当前目录**下（等价于文件管理器的「新建文件夹」）。
 */
@Composable
fun CategoryDrill(
    shelf: ShelfState,
    categories: CategoryState,
    onOpenBook: (ShelfBook) -> Unit,
    onBookMenu: (ShelfBook) -> Unit,
    onSelectCategory: (Long?) -> Unit,
    onNavigateUp: () -> Unit,
    onCreate: (String, Long) -> Unit,
    onRename: (Long, String) -> Unit,
    targets: List<CategoryTargetView>,
    onLoadTargets: (Long) -> Unit,
    onClearTargets: () -> Unit,
    onMoveCategory: (Long, Long) -> Unit,
    onDeleteCategory: (Long, Int, Long) -> Unit,
) {
    val ops = remember { CategoryOpsState() }
    CategoryOpsDialogs(
        ops = ops,
        state = categories,
        targets = targets,
        onLoadTargets = onLoadTargets,
        onClearTargets = onClearTargets,
        onCreate = onCreate,
        onRename = onRename,
        onMoveCategory = onMoveCategory,
        onDeleteCategory = onDeleteCategory,
    )

    val location = categories.selectedId
    // `drilledId != null` = 已进入某个分类；`null` = 根层（也涵盖尚未归一化的「全部书籍」）。
    val drilledId = location?.takeIf { it != CategoryTreeAssembly.UNCATEGORIZED }
    val atRoot = drilledId == null
    val parentId = drilledId?.let { CategoryTreeAssembly.parentOf(categories.nodes, it) }
    val parentLabel = when {
        drilledId == null -> null
        parentId == null -> "根目录"
        else -> categories.nodes.firstOrNull { it.id == parentId }?.name ?: "上一级"
    }
    val currentName = drilledId?.let { id -> categories.nodes.firstOrNull { it.id == id }?.name }
    val breadcrumb = drilledId?.let { id ->
        CategoryTreeAssembly.pathTo(categories.nodes, id).joinToString(" / ") { it.name }
    }

    // 归一化：窄屏没有「全部书籍」位置 —— 首次进入（或从宽屏旋转）时落到根层。
    LaunchedEffect(Unit) {
        if (categories.selectedId == null) onSelectCategory(CategoryTreeAssembly.UNCATEGORIZED)
    }
    // 系统返回键：逐级回退（在根层不拦截，交给系统）。
    BackHandler(enabled = drilledId != null) { onNavigateUp() }

    val dirs = CategoryTreeAssembly.levelChildren(categories.nodes, location)

    Column(modifier = Modifier.fillMaxSize().testTag(ReaderTestTags.DRILL)) {
        if (drilledId != null) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clickable(onClick = onNavigateUp)
                    .testTag(ReaderTestTags.DRILL_BACK)
                    .semantics { contentDescription = "返回「$parentLabel」" }
                    .padding(horizontal = 12.dp, vertical = 10.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = "← 返回$parentLabel",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.primary,
                )
                Spacer(modifier = Modifier.weight(1f))
                Text(
                    text = breadcrumb.orEmpty(),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            HorizontalDivider()
        }

        LazyColumn(modifier = Modifier.fillMaxWidth().weight(1f)) {
            items(items = dirs, key = { row -> "dir-${row.id}" }) { row ->
                DrillDirRow(
                    row = row,
                    onEnter = { onSelectCategory(row.id) },
                    onMenu = { ops.requestMenu(row.id) },
                )
                HorizontalDivider()
            }
            items(items = shelf.books, key = { book -> book.sourceIdentity }) { book ->
                ShelfRow(
                    book = book,
                    selected = book.bookId == shelf.selectedBookId,
                    onOpenBook = onOpenBook,
                    onMenuRequest = { onBookMenu(book) },
                )
                HorizontalDivider()
            }
            if (dirs.isEmpty() && shelf.books.isEmpty()) {
                item {
                    Text(
                        text = if (atRoot) ReaderStatus.EMPTY_SHELF else "（此目录下暂无书籍）",
                        style = MaterialTheme.typography.bodyMedium,
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 12.dp, vertical = 24.dp)
                            .testTag(
                                if (atRoot) {
                                    ReaderTestTags.SHELF_EMPTY_STATE
                                } else {
                                    ReaderTestTags.DRILL_EMPTY_HINT
                                },
                            ),
                    )
                }
            }
        }

        HorizontalDivider()
        OutlinedButton(
            onClick = {
                val parent = location?.takeIf { it > 0L } ?: CategoryTreeAssembly.UNCATEGORIZED
                ops.requestCreate(parent, currentName)
            },
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 12.dp, vertical = 6.dp)
                .testTag(ReaderTestTags.DRILL_CREATE)
                .semantics {
                    contentDescription = if (currentName != null) {
                        "在「$currentName」下新建分类"
                    } else {
                        "在根目录新建分类"
                    }
                },
        ) {
            Text(if (currentName != null) "新建分类（在「$currentName」下）" else "新建分类")
        }
    }
}

/** 下钻列表里的目录行：▸ + 名称 + 计数 + 「⋯」（进入 = 点击整行）。 */
@Composable
private fun DrillDirRow(
    row: CategoryRowView,
    onEnter: () -> Unit,
    onMenu: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onEnter)
            .testTag(ReaderTestTags.drillDir(row.id))
            .semantics { contentDescription = "目录 ${row.name}（${row.countLabel}）" }
            .padding(horizontal = 12.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(text = "▸", style = MaterialTheme.typography.labelMedium)
        Spacer(modifier = Modifier.width(6.dp))
        Text(
            text = row.name,
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.weight(1f),
        )
        Text(text = row.countLabel, style = MaterialTheme.typography.labelSmall)
        Text(
            text = "⋯",
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier
                .clickable(onClick = onMenu)
                .testTag(ReaderTestTags.drillDirMenu(row.id))
                .semantics { contentDescription = "${row.name} 的操作菜单" }
                .padding(start = 8.dp),
        )
    }
}

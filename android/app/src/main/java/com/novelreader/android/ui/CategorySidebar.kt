package com.novelreader.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
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
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.novelreader.android.reader.CategoryRowView
import com.novelreader.android.reader.CategoryState
import com.novelreader.android.reader.CategoryTargetView
import com.novelreader.android.reader.CategoryTreeAssembly

/**
 * 书架分类侧栏（V1.1；**宽屏布局**：横屏 / 平板。窄屏请用 [CategoryDrill]）。
 *
 * # 口径
 *
 * - 层级（`depth`）、直接/合计计数**全部来自核心**，界面只按 `depth` 缩进；
 * - 折叠是**纯视图状态**（哪些行下发），不影响核心的任何计算；
 * - 「全部书籍」「未分类」是**入口**而非真实分类记录（未分类不是一条分类记录）；
 * - 选中分类 ⇒ 列表只显示**直属**该分类的书（2026-10-09 起；集合由核心映射逐本判定）；
 * - 操作对话框（新建子分类 / 重命名 / 移动到… / 删除）由 [CategoryOpsDialogs] 承载 ——
 *   与窄屏下钻共用**同一套**，两布局不允许各自演化第二套口径；
 * - 「新建分类」建在**当前选中分类**下（无选中 = 根层）。
 */
@Composable
fun CategorySidebar(
    state: CategoryState,
    onSelectAll: () -> Unit,
    onSelectCategory: (Long) -> Unit,
    onToggle: (Long) -> Unit,
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
        state = state,
        targets = targets,
        onLoadTargets = onLoadTargets,
        onClearTargets = onClearTargets,
        onCreate = onCreate,
        onRename = onRename,
        onMoveCategory = onMoveCategory,
        onDeleteCategory = onDeleteCategory,
    )

    Column(
        modifier = Modifier
            .width(SIDEBAR_WIDTH)
            .fillMaxHeight()
            .testTag(ReaderTestTags.SIDEBAR)
            .semantics { contentDescription = "分类侧栏" },
    ) {
        SidebarFixedRow(
            label = "全部书籍",
            count = state.rootTotalCount,
            selected = state.selectedId == null,
            tag = ReaderTestTags.SIDEBAR_ALL,
            onClick = onSelectAll,
        )
        SidebarFixedRow(
            label = "未分类",
            count = state.uncategorizedCount,
            selected = state.selectedId == CategoryTreeAssembly.UNCATEGORIZED,
            tag = ReaderTestTags.SIDEBAR_UNCATEGORIZED,
            onClick = { onSelectCategory(CategoryTreeAssembly.UNCATEGORIZED) },
        )
        HorizontalDivider()
        if (state.rows.isEmpty()) {
            Text(
                text = "还没有分类",
                style = MaterialTheme.typography.labelSmall,
                modifier = Modifier.padding(horizontal = 12.dp, vertical = 10.dp),
            )
        } else {
            LazyColumn(modifier = Modifier.fillMaxWidth().weight(1f)) {
                items(items = state.rows, key = { row -> row.id }) { row ->
                    CategoryTreeRow(
                        row = row,
                        onSelect = { onSelectCategory(row.id) },
                        onToggle = { onToggle(row.id) },
                        onMenuRequest = { ops.requestMenu(row.id) },
                    )
                }
            }
        }
        HorizontalDivider()
        OutlinedButton(
            onClick = {
                val parent = state.selectedId?.takeIf { it > 0L } ?: CategoryTreeAssembly.UNCATEGORIZED
                ops.requestCreate(parent, state.nodes.firstOrNull { it.id == parent }?.name)
            },
            modifier = Modifier
                .fillMaxWidth()
                .padding(8.dp)
                .testTag(ReaderTestTags.SIDEBAR_CREATE)
                .semantics { contentDescription = "新建分类（在当前分类下）" },
        ) {
            Text("新建分类")
        }
    }
}

private val SIDEBAR_WIDTH = 190.dp

@Composable
private fun SidebarFixedRow(
    label: String,
    count: Long,
    selected: Boolean,
    tag: String,
    onClick: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(if (selected) MaterialTheme.colorScheme.secondaryContainer else Color.Transparent)
            .clickable(onClick = onClick)
            .testTag(tag)
            .semantics { contentDescription = "$label（$count 本）" }
            .padding(horizontal = 12.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.weight(1f),
        )
        Text(text = "$count", style = MaterialTheme.typography.labelSmall)
    }
}

@Composable
private fun CategoryTreeRow(
    row: CategoryRowView,
    onSelect: () -> Unit,
    onToggle: () -> Unit,
    onMenuRequest: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(if (row.selected) MaterialTheme.colorScheme.secondaryContainer else Color.Transparent)
            .clickable(onClick = onSelect)
            .testTag(ReaderTestTags.sidebarCategory(row.id))
            .semantics { contentDescription = "分类 ${row.name}（${row.countLabel}）" }
            .padding(start = (8 + row.indentLevel * 12).dp, end = 8.dp, top = 8.dp, bottom = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // 展开 / 折叠：只有真有子分类的行才给箭头（没箭头可点比假箭头诚实）。
        if (row.hasChildren) {
            Text(
                text = if (row.expanded) "▾" else "▸",
                style = MaterialTheme.typography.labelMedium,
                modifier = Modifier
                    .clickable(onClick = onToggle)
                    .testTag(ReaderTestTags.sidebarToggle(row.id))
                    .semantics { contentDescription = if (row.expanded) "折叠" else "展开" }
                    .padding(horizontal = 2.dp),
            )
        } else {
            Spacer(modifier = Modifier.width(14.dp))
        }
        Text(
            text = row.name,
            style = MaterialTheme.typography.bodySmall,
            modifier = Modifier.weight(1f),
        )
        Text(text = row.countLabel, style = MaterialTheme.typography.labelSmall)
        Text(
            text = "⋯",
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier
                .clickable(onClick = onMenuRequest)
                .testTag(ReaderTestTags.sidebarMenu(row.id))
                .semantics { contentDescription = "${row.name} 的操作菜单" }
                .padding(start = 6.dp),
        )
    }
}

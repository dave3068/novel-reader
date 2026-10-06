package com.novelreader.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
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
import com.novelreader.android.reader.DeleteDestination
import com.novelreader.android.reader.CategoryTreeAssembly

/**
 * 书架分类侧栏（V1.1 规格 R1–R5）。
 *
 * # 口径
 *
 * - 层级（`depth`）、直接/合计计数**全部来自核心**，界面只按 `depth` 缩进；
 * - 折叠是**纯视图状态**（哪些行下发），不影响核心的任何计算；
 * - 「全部书籍」「未分类」是**入口**而非真实分类记录（未分类不是一条分类）；
 * - 选中分类 ⇒ 书架列表由核心给出的可见集合过滤（子树展开在核心完成）。
 */
@Composable
fun CategorySidebar(
    state: CategoryState,
    onSelectAll: () -> Unit,
    onSelectCategory: (Long) -> Unit,
    onToggle: (Long) -> Unit,
    onCreate: (String) -> Unit,
    onRename: (Long, String) -> Unit,
    targets: List<CategoryTargetView>,
    onLoadTargets: (Long) -> Unit,
    onClearTargets: () -> Unit,
    onMoveCategory: (Long, Long) -> Unit,
    onDeleteCategory: (Long, Int, Long) -> Unit,
) {
    // 对话框状态：`null` = 关闭；`Create` = 新建；`Rename(id)` = 重命名。
    var naming by remember { mutableStateOf<NameTarget?>(null) }
    // `menuFor` = 打开「⋯」菜单的分类；`moveSource` / `deleteSource` = 正在选目标的分类。
    var menuFor by remember { mutableStateOf<Long?>(null) }
    var moveSource by remember { mutableStateOf<Long?>(null) }
    var deleteSource by remember { mutableStateOf<Long?>(null) }
    var deleteDestination by remember { mutableStateOf(DeleteDestination.NONE) }
    var pickedTarget by remember { mutableStateOf<Long?>(null) }

    if (naming != null) {
        val target = naming!!
        var text by remember(target) { mutableStateOf(target.initial) }
        AlertDialog(
            onDismissRequest = { naming = null },
            title = { Text(if (target.id == null) "新建分类" else "重命名分类") },
            text = {
                OutlinedTextField(
                    value = text,
                    onValueChange = { text = it },
                    label = { Text("分类名") },
                    singleLine = true,
                    modifier = Modifier
                        .fillMaxWidth()
                        .testTag(ReaderTestTags.SIDEBAR_NAME_FIELD)
                        .semantics { contentDescription = "分类名" },
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        val name = text.trim()
                        naming = null
                        if (name.isEmpty()) return@TextButton
                        if (target.id == null) onCreate(name) else onRename(target.id, name)
                    },
                    modifier = Modifier
                        .testTag(ReaderTestTags.SIDEBAR_NAME_CONFIRM)
                        .semantics { contentDescription = "确认分类名" },
                ) {
                    Text("确定")
                }
            },
            dismissButton = {
                TextButton(
                    onClick = { naming = null },
                    modifier = Modifier.semantics { contentDescription = "取消" },
                ) {
                    Text("取消")
                }
            },
        )
    }

    menuFor?.let { id ->
        val row = state.rows.firstOrNull { it.id == id }
        AlertDialog(
            onDismissRequest = { menuFor = null },
            title = { Text(row?.name ?: "分类") },
            text = { Text("选择要执行的操作。") },
            confirmButton = {
                TextButton(
                    onClick = {
                        menuFor = null
                        naming = NameTarget(id, row?.name ?: "")
                    },
                    modifier = Modifier
                        .testTag(ReaderTestTags.SIDEBAR_MENU_RENAME)
                        .semantics { contentDescription = "重命名" },
                ) { Text("重命名") }
            },
            dismissButton = {
                Row {
                    TextButton(
                        onClick = {
                            menuFor = null
                            moveSource = id
                            pickedTarget = null
                            onLoadTargets(id)
                        },
                        modifier = Modifier
                            .testTag(ReaderTestTags.SIDEBAR_MENU_MOVE)
                            .semantics { contentDescription = "移动到其他分类" },
                    ) { Text("移动到…") }
                    TextButton(
                        onClick = {
                            menuFor = null
                            deleteSource = id
                            deleteDestination = DeleteDestination.NONE
                            pickedTarget = null
                            onLoadTargets(id)
                        },
                        modifier = Modifier
                            .testTag(ReaderTestTags.SIDEBAR_MENU_DELETE)
                            .semantics { contentDescription = "删除分类" },
                    ) { Text("删除") }
                }
            },
        )
    }

    // 目标选择（移动）与删除去向（删除）：同一个目标列表，语义不同。
    val targetDialogFor = moveSource ?: deleteSource
    targetDialogFor?.let { source ->
        val isDelete = deleteSource != null
        AlertDialog(
            onDismissRequest = {
                moveSource = null
                deleteSource = null
                onClearTargets()
            },
            modifier = Modifier.testTag(ReaderTestTags.SIDEBAR_TARGET_DIALOG),
            title = { Text(if (isDelete) "删除分类：选择书籍去向" else "移动到…") },
            text = {
                Column {
                    if (isDelete) {
                        Text(
                            text = "未选择去向时不会执行删除；原文件始终不受影响。",
                            style = MaterialTheme.typography.labelSmall,
                        )
                        listOf(
                            DeleteDestination.MOVE_TO to "移入另一分类",
                            DeleteDestination.REMOVE_FROM_SHELF to "一并移出书架",
                            DeleteDestination.TO_UNCATEGORIZED to "移回根下未分类",
                        ).forEach { (kind, label) ->
                            Text(
                                text = (if (deleteDestination == kind) "● " else "○ ") + label,
                                style = MaterialTheme.typography.bodySmall,
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .clickable {
                                        deleteDestination = kind
                                        if (kind != DeleteDestination.MOVE_TO) pickedTarget = null
                                    }
                                    .testTag(ReaderTestTags.sidebarDeleteDestination(kind))
                                    .semantics { contentDescription = label }
                                    .padding(vertical = 6.dp),
                            )
                        }
                    }
                    val needTarget = !isDelete || deleteDestination == DeleteDestination.MOVE_TO
                    if (needTarget) {
                        if (targets.isEmpty()) {
                            Text("正在读取可选分类…", style = MaterialTheme.typography.labelSmall)
                        }
                        targets.forEach { target ->
                            Text(
                                text = (if (pickedTarget == target.id) "● " else "○ ") +
                                    "　".repeat(target.indentLevel) + target.label,
                                style = MaterialTheme.typography.bodySmall,
                                // 不可选的目标**置灰但仍显示** —— 让用户看得见「为什么不能选」。
                                color = if (target.selectable) {
                                    MaterialTheme.colorScheme.onSurface
                                } else {
                                    MaterialTheme.colorScheme.onSurfaceVariant
                                },
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .clickable(enabled = target.selectable) {
                                        pickedTarget = target.id
                                    }
                                    .testTag(ReaderTestTags.sidebarTarget(target.id))
                                    .semantics { contentDescription = target.label }
                                    .padding(vertical = 6.dp),
                            )
                        }
                    }
                }
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        val source2 = source
                        val dest = deleteDestination
                        val target = pickedTarget
                        moveSource = null
                        deleteSource = null
                        onClearTargets()
                        if (isDelete) {
                            onDeleteCategory(source2, dest, target ?: -1L)
                        } else if (target != null) {
                            onMoveCategory(source2, target)
                        }
                    },
                    modifier = Modifier
                        .testTag(ReaderTestTags.SIDEBAR_CONFIRM)
                        .semantics { contentDescription = "确认" },
                ) {
                    Text("确定")
                }
            },
            dismissButton = {
                TextButton(
                    onClick = {
                        moveSource = null
                        deleteSource = null
                        onClearTargets()
                    },
                    modifier = Modifier.semantics { contentDescription = "取消" },
                ) { Text("取消") }
            },
        )
    }

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
                        onMenuRequest = { menuFor = row.id },
                    )
                }
            }
        }
        HorizontalDivider()
        OutlinedButton(
            onClick = { naming = NameTarget(null, "") },
            modifier = Modifier
                .fillMaxWidth()
                .padding(8.dp)
                .testTag(ReaderTestTags.SIDEBAR_CREATE)
                .semantics { contentDescription = "在根层新建分类" },
        ) {
            Text("新建分类")
        }
    }
}

/** 命名对话框的目标：`id == null` 表示新建。 */
private data class NameTarget(val id: Long?, val initial: String)

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

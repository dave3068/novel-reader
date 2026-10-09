package com.novelreader.android.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.novelreader.android.reader.CategoryState
import com.novelreader.android.reader.CategoryTargetView
import com.novelreader.android.reader.CategoryTreeAssembly
import com.novelreader.android.reader.DeleteDestination

/**
 * 分类操作对话框的**共享宿主**（宽屏侧栏与窄屏下钻共用）。
 *
 * 2026-10-09 抽取：同一套「命名 / 行菜单 / 移动到… / 删除（含去向）」此前只存在于侧栏；
 * 窄屏下钻的目录行需要同样的操作 —— 若复制两份，两个布局会各自演化出口径不一致的第二套。
 *
 * 用法：调用方 `remember { CategoryOpsState() }`，行内「⋯」调用
 * [CategoryOpsState.requestMenu]（或 [CategoryOpsState.requestCreate]/[CategoryOpsState.requestRename]），
 * 再渲染本宿主即可。
 */
class CategoryOpsState {

    /** 命名对话框请求（`editId == null` = 新建）。 */
    var naming by mutableStateOf<NameRequest?>(null)

    /** 「⋯」菜单打开的分类 id。 */
    var menuFor by mutableStateOf<Long?>(null)

    /** 目标选择（移动）与删除去向（删除）的来源分类。 */
    var moveSource by mutableStateOf<Long?>(null)
    var deleteSource by mutableStateOf<Long?>(null)
    var deleteDestination by mutableStateOf(DeleteDestination.NONE)
    var pickedTarget by mutableStateOf<Long?>(null)

    /** 打开命名对话框（新建；[parentId] `-1` = 根层）。 */
    fun requestCreate(parentId: Long, parentLabel: String?) {
        naming = NameRequest(editId = null, parentId = parentId, initial = "", parentLabel = parentLabel)
    }

    /** 打开命名对话框（重命名）。 */
    fun requestRename(id: Long, initial: String) {
        naming = NameRequest(editId = id, parentId = CategoryTreeAssembly.UNCATEGORIZED, initial = initial, parentLabel = null)
    }

    /** 打开某一行的「⋯」菜单。 */
    fun requestMenu(id: Long) {
        menuFor = id
    }
}

/** 命名请求：[editId] `null` = 新建（[parentId] 为其父）。 */
data class NameRequest(
    val editId: Long?,
    val parentId: Long,
    val initial: String,
    val parentLabel: String?,
)

@Composable
fun CategoryOpsDialogs(
    ops: CategoryOpsState,
    state: CategoryState,
    targets: List<CategoryTargetView>,
    onLoadTargets: (Long) -> Unit,
    onClearTargets: () -> Unit,
    onCreate: (String, Long) -> Unit,
    onRename: (Long, String) -> Unit,
    onMoveCategory: (Long, Long) -> Unit,
    onDeleteCategory: (Long, Int, Long) -> Unit,
) {
    // ---- 命名对话框（新建 / 重命名）----
    ops.naming?.let { request ->
        var text by remember(request) { mutableStateOf(request.initial) }
        AlertDialog(
            onDismissRequest = { ops.naming = null },
            title = { Text(if (request.editId == null) "新建分类" else "重命名分类") },
            text = {
                Column {
                    if (request.editId == null && request.parentId != CategoryTreeAssembly.UNCATEGORIZED) {
                        Text(
                            text = "将在「${request.parentLabel ?: "…"}」下新建。",
                            style = MaterialTheme.typography.labelSmall,
                        )
                    }
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
                }
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        val name = text.trim()
                        ops.naming = null
                        if (name.isEmpty()) return@TextButton
                        if (request.editId == null) onCreate(name, request.parentId) else onRename(request.editId, name)
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
                    onClick = { ops.naming = null },
                    modifier = Modifier.semantics { contentDescription = "取消" },
                ) {
                    Text("取消")
                }
            },
        )
    }

    // ---- 行「⋯」菜单（新建子分类 / 重命名 / 移动到… / 删除）----
    ops.menuFor?.let { id ->
        val row = state.rows.firstOrNull { it.id == id }
        AlertDialog(
            onDismissRequest = { ops.menuFor = null },
            title = { Text(row?.name ?: "分类") },
            text = {
                Column {
                    DialogActionRow(
                        label = "新建子分类",
                        tag = ReaderTestTags.SIDEBAR_MENU_CREATE,
                        onClick = {
                            ops.menuFor = null
                            ops.requestCreate(id, row?.name)
                        },
                    )
                    DialogActionRow(
                        label = "重命名",
                        tag = ReaderTestTags.SIDEBAR_MENU_RENAME,
                        onClick = {
                            ops.menuFor = null
                            ops.requestRename(id, row?.name ?: "")
                        },
                    )
                    DialogActionRow(
                        label = "移动到…",
                        tag = ReaderTestTags.SIDEBAR_MENU_MOVE,
                        onClick = {
                            ops.menuFor = null
                            ops.moveSource = id
                            ops.deleteSource = null
                            ops.pickedTarget = null
                            onLoadTargets(id)
                        },
                    )
                    DialogActionRow(
                        label = "删除",
                        tag = ReaderTestTags.SIDEBAR_MENU_DELETE,
                        onClick = {
                            ops.menuFor = null
                            ops.deleteSource = id
                            ops.moveSource = null
                            ops.deleteDestination = DeleteDestination.NONE
                            ops.pickedTarget = null
                            onLoadTargets(id)
                        },
                    )
                }
            },
            confirmButton = {
                TextButton(
                    onClick = { ops.menuFor = null },
                    modifier = Modifier.semantics { contentDescription = "取消" },
                ) {
                    Text("取消")
                }
            },
        )
    }

    // ---- 目标选择（移动）与删除去向（删除）：同一个目标列表，语义不同。----
    val targetDialogFor = ops.moveSource ?: ops.deleteSource
    targetDialogFor?.let { source ->
        val isDelete = ops.deleteSource != null
        AlertDialog(
            onDismissRequest = {
                ops.moveSource = null
                ops.deleteSource = null
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
                                text = (if (ops.deleteDestination == kind) "● " else "○ ") + label,
                                style = MaterialTheme.typography.bodySmall,
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .clickable {
                                        ops.deleteDestination = kind
                                        if (kind != DeleteDestination.MOVE_TO) ops.pickedTarget = null
                                    }
                                    .testTag(ReaderTestTags.sidebarDeleteDestination(kind))
                                    .semantics { contentDescription = label }
                                    .padding(vertical = 6.dp),
                            )
                        }
                    }
                    val needTarget = !isDelete || ops.deleteDestination == DeleteDestination.MOVE_TO
                    if (needTarget) {
                        if (targets.isEmpty()) {
                            Text("正在读取可选分类…", style = MaterialTheme.typography.labelSmall)
                        }
                        targets.forEach { target ->
                            Text(
                                text = (if (ops.pickedTarget == target.id) "● " else "○ ") +
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
                                        ops.pickedTarget = target.id
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
                        val dest = ops.deleteDestination
                        val target = ops.pickedTarget
                        ops.moveSource = null
                        ops.deleteSource = null
                        onClearTargets()
                        if (isDelete) {
                            onDeleteCategory(source, dest, target ?: -1L)
                        } else if (target != null) {
                            onMoveCategory(source, target)
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
                        ops.moveSource = null
                        ops.deleteSource = null
                        onClearTargets()
                    },
                    modifier = Modifier.semantics { contentDescription = "取消" },
                ) { Text("取消") }
            },
        )
    }
}

/**
 * 对话框里的一行动作（文字行，避免多个按钮在窄屏溢出）。
 *
 * [enabled] 为 `false` 时置灰但仍显示 —— 与「不可选目标」同一条口径：让用户看得见原因。
 */
@Composable
fun DialogActionRow(
    label: String,
    tag: String,
    onClick: () -> Unit,
    enabled: Boolean = true,
) {
    Text(
        text = label,
        style = MaterialTheme.typography.bodySmall,
        color = if (enabled) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier
            .fillMaxWidth()
            .clickable(enabled = enabled, onClick = onClick)
            .testTag(tag)
            .semantics { contentDescription = label }
            .padding(vertical = 8.dp),
    )
}

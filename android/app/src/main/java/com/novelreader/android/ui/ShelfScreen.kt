package com.novelreader.android.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.novelreader.android.reader.CategoryState
import com.novelreader.android.reader.CategoryTargetView
import com.novelreader.android.reader.CategoryTreeAssembly
import com.novelreader.android.reader.ReaderState
import com.novelreader.android.reader.ReaderStatus
import com.novelreader.android.reader.ShelfBook
import com.novelreader.android.reader.ShelfLayout
import com.novelreader.android.reader.ShelfState

/**
 * 书架页：显示**真实**的会话书架状态（空书架就是空态）。
 *
 * 每个控件都接真实状态：
 * - 条目点击 → [onOpenBook]（打开来源并恢复进度）；
 * - 「导入 TXT」→ [onImportRequest]（宿主注入：AN4 接 SAF `OpenDocument`，本层不做空实现）；
 * - 「继续阅读」/「设置」→ 路由切换（仅在有书/无书时给出真实差异）；
 * - 书籍行「⋯」→ 移动到…（2026-10-09 起）/ 移除（不删原文件）。
 *
 * # 自适应布局（2026-10-09 主人确认）
 *
 * - **窄屏（< 600dp，竖屏手机）**：文件管理器式下钻列表（[CategoryDrill]）——目录在上、
 *   书籍在下，逐层进入，系统返回键逐级回退；
 * - **宽屏（横屏 / 平板）**：保留左右布局（[CategorySidebar] + 书籍列表）。
 *   两布局共用同一套分类操作对话框与同一份过滤口径（直属）。
 *
 * 本层不解析来源标识（AN3 是文件路径，AN4 起是持久 URI），只做展示。
 */
@Composable
fun ShelfScreen(
    shelf: ShelfState,
    categories: CategoryState,
    reader: ReaderState,
    abiVersion: Int?,
    status: String,
    onOpenBook: (ShelfBook) -> Unit,
    onRemoveBook: (ShelfBook) -> Unit,
    onImportRequest: () -> Unit,
    onOpenReader: () -> Unit,
    onOpenSettings: () -> Unit,
    onOpenHistory: () -> Unit,
    onSelectCategory: (Long?) -> Unit,
    onToggleCategory: (Long) -> Unit,
    onNavigateUp: () -> Unit,
    onCreateCategory: (String, Long) -> Unit,
    onRenameCategory: (Long, String) -> Unit,
    categoryTargets: List<CategoryTargetView>,
    onLoadCategoryTargets: (Long) -> Unit,
    onClearCategoryTargets: () -> Unit,
    onMoveCategory: (Long, Long) -> Unit,
    onDeleteCategory: (Long, Int, Long) -> Unit,
    onMoveBookToCategory: (bookId: Long, categoryId: Long, label: String?) -> Unit,
) {
    Column(modifier = Modifier.fillMaxSize().nrScreenInsets().testTag(ReaderTestTags.SHELF_SCREEN)) {
        NrTopBar(
            title = "书架",
            subtitle = if (abiVersion == null) "ABI 版本未知" else "ABI 版本 $abiVersion · 会话内 ${shelf.books.size} 本",
        ) {
            TextButton(
                onClick = onOpenReader,
                enabled = reader.isOpen,
                modifier = Modifier
                    .testTag(ReaderTestTags.SHELF_OPEN_READER_BUTTON)
                    .semantics { contentDescription = "继续阅读当前书籍" },
            ) {
                Text("继续阅读")
            }
            TextButton(
                onClick = onOpenHistory,
                modifier = Modifier
                    .testTag(ReaderTestTags.SHELF_OPEN_HISTORY_BUTTON)
                    .semantics { contentDescription = "查看最近阅读" },
            ) {
                Text("最近阅读")
            }
            TextButton(
                onClick = onOpenSettings,
                modifier = Modifier
                    .testTag(ReaderTestTags.SHELF_OPEN_SETTINGS_BUTTON)
                    .semantics { contentDescription = "打开排版设置" },
            ) {
                Text("设置")
            }
        }
        HorizontalDivider()

        // 书籍操作对话框（两布局共用）：
        // - menuForBook：「⋯」菜单（移动到… / 移除）；
        // - moveForBook + moveTarget：移动目标选择（目标 = 根下 + 全部分类，核心写入）；
        // - pendingRemoval：移除二次确认（文案写明**不删原文件**）。
        var pendingRemoval by remember { mutableStateOf<ShelfBook?>(null) }
        var menuForBook by remember { mutableStateOf<ShelfBook?>(null) }
        var moveForBook by remember { mutableStateOf<ShelfBook?>(null) }
        var moveTarget by remember { mutableStateOf<Long?>(null) }

        pendingRemoval?.let { target ->
            AlertDialog(
                onDismissRequest = { pendingRemoval = null },
                modifier = Modifier.testTag(ReaderTestTags.SHELF_REMOVE_DIALOG),
                title = { Text("从书架移除《${target.displayName}》？") },
                text = {
                    Text(
                        "仅从书架移除并清除本书的本地索引缓存，**不会删除原文件**；" +
                            "阅读位置会保留，重新导入同一文件即可恢复。",
                    )
                },
                confirmButton = {
                    TextButton(
                        onClick = {
                            pendingRemoval = null
                            onRemoveBook(target)
                        },
                        modifier = Modifier
                            .testTag(ReaderTestTags.SHELF_REMOVE_CONFIRM)
                            .semantics { contentDescription = "确认移除且不删除原文件" },
                    ) {
                        Text("移除（不删原文件）")
                    }
                },
                dismissButton = {
                    TextButton(
                        onClick = { pendingRemoval = null },
                        modifier = Modifier
                            .testTag(ReaderTestTags.SHELF_REMOVE_CANCEL)
                            .semantics { contentDescription = "取消移除" },
                    ) {
                        Text("取消")
                    }
                },
            )
        }

        menuForBook?.let { book ->
            AlertDialog(
                onDismissRequest = { menuForBook = null },
                title = { Text(book.displayName) },
                text = {
                    Column {
                        DialogActionRow(
                            label = "移动到…",
                            tag = ReaderTestTags.BOOK_MENU_MOVE,
                            enabled = book.bookId != 0L,
                            onClick = {
                                menuForBook = null
                                moveTarget = null
                                moveForBook = book
                            },
                        )
                        if (book.bookId == 0L) {
                            // 看得见「为什么不能选」：尚未打开过的书没有核心登记。
                            Text(
                                text = "（尚未打开过：首次打开后即可移入分类）",
                                style = MaterialTheme.typography.labelSmall,
                            )
                        }
                        DialogActionRow(
                            label = "移除（不删原文件）",
                            tag = ReaderTestTags.shelfRemoveButton(book.sourceIdentity),
                            onClick = {
                                menuForBook = null
                                pendingRemoval = book
                            },
                        )
                    }
                },
                confirmButton = {
                    TextButton(
                        onClick = { menuForBook = null },
                        modifier = Modifier.semantics { contentDescription = "取消" },
                    ) {
                        Text("取消")
                    }
                },
            )
        }

        moveForBook?.let { book ->
            val currentCategory =
                categories.bookCategories[book.bookId] ?: CategoryTreeAssembly.UNCATEGORIZED
            val categoryRows = CategoryTreeAssembly.flatten(categories.nodes, emptySet(), null)
            AlertDialog(
                onDismissRequest = { moveForBook = null },
                modifier = Modifier.testTag(ReaderTestTags.BOOK_MOVE_DIALOG),
                title = { Text("移动《${book.displayName}》") },
                text = {
                    Column {
                        Text(
                            text = "选择目标位置（原文件始终不受影响）。",
                            style = MaterialTheme.typography.labelSmall,
                        )
                        MoveTargetRow(
                            id = CategoryTreeAssembly.UNCATEGORIZED,
                            label = "根下（未分类）",
                            selected = moveTarget == CategoryTreeAssembly.UNCATEGORIZED,
                            current = currentCategory == CategoryTreeAssembly.UNCATEGORIZED,
                            onPick = { moveTarget = CategoryTreeAssembly.UNCATEGORIZED },
                        )
                        categoryRows.forEach { row ->
                            MoveTargetRow(
                                id = row.id,
                                label = "　".repeat(row.indentLevel) + row.name,
                                selected = moveTarget == row.id,
                                current = currentCategory == row.id,
                                onPick = { moveTarget = row.id },
                            )
                        }
                    }
                },
                confirmButton = {
                    TextButton(
                        onClick = {
                            val target = moveTarget
                            moveForBook = null
                            if (target != null) {
                                onMoveBookToCategory(book.bookId, target, book.displayName)
                            }
                        },
                        enabled = moveTarget != null,
                        modifier = Modifier
                            .testTag(ReaderTestTags.BOOK_MOVE_CONFIRM)
                            .semantics { contentDescription = "确认移动" },
                    ) {
                        Text("确定")
                    }
                },
                dismissButton = {
                    TextButton(
                        onClick = { moveForBook = null },
                        modifier = Modifier.semantics { contentDescription = "取消" },
                    ) {
                        Text("取消")
                    }
                },
            )
        }

        BoxWithConstraints(modifier = Modifier.fillMaxWidth().weight(1f)) {
            if (ShelfLayout.isCompact(maxWidth.value)) {
                // 窄屏：文件管理器式下钻（单列；目录在上、书籍在下）——上下导航都在列表内。
                if (shelf.loading) {
                    ShelfLoadingState(modifier = Modifier.fillMaxSize())
                } else {
                    CategoryDrill(
                        shelf = shelf,
                        categories = categories,
                        onOpenBook = onOpenBook,
                        onBookMenu = { menuForBook = it },
                        onSelectCategory = onSelectCategory,
                        onNavigateUp = onNavigateUp,
                        onCreate = onCreateCategory,
                        onRename = onRenameCategory,
                        targets = categoryTargets,
                        onLoadTargets = onLoadCategoryTargets,
                        onClearTargets = onClearCategoryTargets,
                        onMoveCategory = onMoveCategory,
                        onDeleteCategory = onDeleteCategory,
                    )
                }
            } else {
                // 宽屏：保留左右布局（分类侧栏 + 书籍列表）。
                Row(modifier = Modifier.fillMaxWidth()) {
                    CategorySidebar(
                        state = categories,
                        onSelectAll = { onSelectCategory(null) },
                        onSelectCategory = { id -> onSelectCategory(id) },
                        onToggle = onToggleCategory,
                        onCreate = onCreateCategory,
                        onRename = onRenameCategory,
                        targets = categoryTargets,
                        onLoadTargets = onLoadCategoryTargets,
                        onClearTargets = onClearCategoryTargets,
                        onMoveCategory = onMoveCategory,
                        onDeleteCategory = onDeleteCategory,
                    )
                    Column(modifier = Modifier.weight(1f)) {
                        when {
                            shelf.loading -> {
                                ShelfLoadingState(modifier = Modifier.fillMaxWidth().weight(1f))
                            }
                            shelf.isEmpty -> {
                                Column(
                                    modifier = Modifier.fillMaxWidth().weight(1f).padding(24.dp),
                                    verticalArrangement = Arrangement.Center,
                                    horizontalAlignment = Alignment.CenterHorizontally,
                                ) {
                                    Text(
                                        text = ReaderStatus.EMPTY_SHELF,
                                        style = MaterialTheme.typography.bodyLarge,
                                        modifier = Modifier.testTag(ReaderTestTags.SHELF_EMPTY_STATE),
                                    )
                                    Spacer(modifier = Modifier.height(16.dp))
                                    Button(
                                        onClick = onImportRequest,
                                        modifier = Modifier
                                            .testTag(ReaderTestTags.SHELF_IMPORT_BUTTON)
                                            .semantics { contentDescription = "导入 TXT 文件" },
                                    ) {
                                        Text("导入 TXT")
                                    }
                                }
                            }
                            else -> {
                                LazyColumn(modifier = Modifier.fillMaxWidth().weight(1f)) {
                                    items(items = shelf.books, key = { book -> book.sourceIdentity }) { book ->
                                        ShelfRow(
                                            book = book,
                                            selected = book.bookId == shelf.selectedBookId,
                                            onOpenBook = onOpenBook,
                                            onMenuRequest = { menuForBook = book },
                                        )
                                        HorizontalDivider()
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        HorizontalDivider()
        Column(modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 6.dp)) {
            Text(text = status, style = MaterialTheme.typography.labelMedium)
        }
        OutlinedButton(
            onClick = onImportRequest,
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 12.dp, vertical = 6.dp)
                .testTag(ReaderTestTags.SHELF_IMPORT_BUTTON_BOTTOM)
                .semantics { contentDescription = "导入 TXT 文件（底部入口）" },
        ) {
            Text("导入 TXT")
        }
    }
}

/** 书架加载态（两布局共用）。 */
@Composable
private fun ShelfLoadingState(modifier: Modifier) {
    Column(
        modifier = modifier,
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        CircularProgressIndicator()
        Spacer(modifier = Modifier.height(12.dp))
        Text(ReaderStatus.OPENING, style = MaterialTheme.typography.bodyMedium)
    }
}

/**
 * 书籍行（两布局共用）。
 *
 * 整行点击 = 打开；行尾「⋯」= 操作菜单（移动到… / 移除 —— 二者都有二次确认或明确回显）。
 * 「打开」文字保留为可发现性提示。
 */
@Composable
fun ShelfRow(
    book: ShelfBook,
    selected: Boolean,
    onOpenBook: (ShelfBook) -> Unit,
    onMenuRequest: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable { onOpenBook(book) }
            .testTag(ReaderTestTags.shelfBook(book.bookId))
            .semantics { contentDescription = "打开《${book.displayName}》" }
            .padding(horizontal = 12.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = book.displayName,
                style = MaterialTheme.typography.titleSmall,
                color = if (selected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface,
            )
            val progress = if (book.hasProgress) {
                "进度：第 ${(book.lastChapterIndex ?: 0) + 1} 章 · 偏移 ${book.lastByteOffset}"
            } else {
                "进度：尚无"
            }
            Text(
                text = "${book.totalBytes} 字节 · ${book.chapterCount} 章 · $progress",
                style = MaterialTheme.typography.labelSmall,
            )
        }
        Text(text = "打开", style = MaterialTheme.typography.labelMedium)
        TextButton(
            onClick = onMenuRequest,
            modifier = Modifier
                .testTag(ReaderTestTags.shelfBookMenu(book.sourceIdentity))
                .semantics { contentDescription = "《${book.displayName}》的操作菜单" },
        ) {
            Text("⋯")
        }
    }
}

/** 移动目标行（单选；`●` 已选、`（当前）` 标注当前归属）。 */
@Composable
private fun MoveTargetRow(
    id: Long,
    label: String,
    selected: Boolean,
    current: Boolean,
    onPick: () -> Unit,
) {
    Text(
        text = (if (selected) "● " else "○ ") + label + if (current) "（当前）" else "",
        style = MaterialTheme.typography.bodySmall,
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onPick)
            .testTag(ReaderTestTags.bookMoveTarget(id))
            .semantics { contentDescription = label }
            .padding(vertical = 6.dp),
    )
}

package com.novelreader.android.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
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
import com.novelreader.android.reader.ReaderState
import com.novelreader.android.reader.CategoryState
import com.novelreader.android.reader.CategoryTargetView
import com.novelreader.android.reader.ReaderStatus
import com.novelreader.android.reader.ShelfBook
import com.novelreader.android.reader.ShelfState

/**
 * 书架页：显示**真实**的会话书架状态（空书架就是空态）。
 *
 * 每个控件都接真实状态：
 * - 条目点击 → [onOpenBook]（打开来源并恢复进度）；
 * - 「导入 TXT」→ [onImportRequest]（宿主注入：AN4 接 SAF `OpenDocument`，本层不做空实现）；
 * - 「继续阅读」/「设置」→ 路由切换（仅在有书/无书时给出真实差异）。
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
    onCreateCategory: (String) -> Unit,
    onRenameCategory: (Long, String) -> Unit,
    categoryTargets: List<CategoryTargetView>,
    onLoadCategoryTargets: (Long) -> Unit,
    onClearCategoryTargets: () -> Unit,
    onMoveCategory: (Long, Long) -> Unit,
    onDeleteCategory: (Long, Int, Long) -> Unit,
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
        // 「移除」必须二次确认，且文案写明**不会删除原文件**（本项目硬约束）。
        var pendingRemoval by remember { mutableStateOf<ShelfBook?>(null) }
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
        Row(modifier = Modifier.fillMaxWidth().weight(1f)) {
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
                Column(
                    modifier = Modifier.fillMaxWidth().weight(1f),
                    verticalArrangement = Arrangement.Center,
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    CircularProgressIndicator()
                    Spacer(modifier = Modifier.height(12.dp))
                    Text(ReaderStatus.OPENING, style = MaterialTheme.typography.bodyMedium)
                }
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
                            onRequestRemove = { pendingRemoval = it },
                        )
                        HorizontalDivider()
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

@Composable
private fun ShelfRow(
    book: ShelfBook,
    selected: Boolean,
    onOpenBook: (ShelfBook) -> Unit,
    onRequestRemove: (ShelfBook) -> Unit,
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
            onClick = { onRequestRemove(book) },
            modifier = Modifier
                .testTag(ReaderTestTags.shelfRemoveButton(book.sourceIdentity))
                .semantics { contentDescription = "移除《${book.displayName}》（不删原文件）" },
        ) {
            Text("移除")
        }
    }
}

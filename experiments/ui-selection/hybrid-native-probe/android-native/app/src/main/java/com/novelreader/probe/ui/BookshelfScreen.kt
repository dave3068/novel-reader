package com.novelreader.probe.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import com.novelreader.probe.reader.BookItem

/**
 * 书架页。书籍列表来自共享库的书籍批次（`getBookBatch`），点击即打开；
 * 选中书以高亮区分，给出可见反馈。
 */
@Composable
fun BookshelfScreen(
    books: List<BookItem>,
    selectedId: Long,
    onBookSelected: (BookItem) -> Unit,
    onOpenFile: () -> Unit,
) {
    Column(Modifier.fillMaxSize().padding(12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                "书架",
                style = MaterialTheme.typography.titleLarge,
                modifier = Modifier.weight(1f),
            )
            Button(onClick = onOpenFile) { Text("打开文件") }
        }
        Text(
            "共 ${books.size} 本（来自共享库书籍批次）",
            style = MaterialTheme.typography.bodySmall,
            modifier = Modifier.padding(vertical = 6.dp),
        )
        LazyColumn(Modifier.weight(1f).testTag("shelf_list"), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            items(books, key = { it.id }) { book ->
                Surface(
                    color = if (book.id == selectedId) {
                        MaterialTheme.colorScheme.primaryContainer
                    } else {
                        MaterialTheme.colorScheme.surfaceVariant
                    },
                    shape = MaterialTheme.shapes.medium,
                    modifier = Modifier
                        .fillMaxWidth()
                        .clickable { onBookSelected(book) },
                ) {
                    Column(Modifier.padding(12.dp)) {
                        Text(book.title, style = MaterialTheme.typography.titleMedium)
                        Text(
                            "${book.author} · ${book.meta}",
                            style = MaterialTheme.typography.bodySmall,
                        )
                    }
                }
            }
        }
    }
}

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
import com.novelreader.android.reader.HistoryRow
import com.novelreader.android.reader.ReaderUiState

/**
 * 最近阅读（V1.1，规格 R8/R9）。
 *
 * # 口径
 *
 * - **时间标签由核心格式化**（今天 `HH:MM` / 本年 `MM-DD HH:MM` / 跨年 `yyyy-MM-dd HH:MM`），
 *   界面**原样显示**，不做任何格式化或截断；
 * - **排序由核心完成**（时间倒序），界面不重排；
 * - 点条目 ⇒ [onOpenBook]（按 `book_id` 反查来源；反查失败由 ViewModel **如实回报**，
 *   界面不猜测、不打开别的书）；
 * - 「清除」只删**流水**，不动书籍 / 进度 / 书签；「清空全部」需二次确认。
 */
@Composable
fun HistoryScreen(
    state: ReaderUiState,
    onBack: () -> Unit,
    onOpenBook: (Long) -> Unit,
    onClearEntry: (Long) -> Unit,
    onClearAll: () -> Unit,
) {
    Column(modifier = Modifier.fillMaxSize().nrScreenInsets().testTag(ReaderTestTags.HISTORY_SCREEN)) {
        NrTopBar(
            title = "最近阅读",
            subtitle = "共 ${state.history.total} 条",
        ) {
            TextButton(
                onClick = onBack,
                modifier = Modifier
                    .testTag(ReaderTestTags.HISTORY_BACK_BUTTON)
                    .semantics { contentDescription = "返回书架" },
            ) {
                Text("返回")
            }
        }
        HorizontalDivider()
        var pendingClearAll by remember { mutableStateOf(false) }
        if (pendingClearAll) {
            AlertDialog(
                onDismissRequest = { pendingClearAll = false },
                title = { Text("清空全部阅读记录？") },
                text = {
                    Text("只清除阅读流水，**不会删除书籍、阅读进度与书签**，也不会改动原文件。")
                },
                confirmButton = {
                    TextButton(
                        onClick = {
                            pendingClearAll = false
                            onClearAll()
                        },
                        modifier = Modifier.semantics { contentDescription = "确认清空全部阅读记录" },
                    ) {
                        Text("清空（不删书与进度）")
                    }
                },
                dismissButton = {
                    TextButton(
                        onClick = { pendingClearAll = false },
                        modifier = Modifier.semantics { contentDescription = "取消清空" },
                    ) {
                        Text("取消")
                    }
                },
            )
        }
        when {
            state.history.loading -> {
                Column(
                    modifier = Modifier.fillMaxWidth().weight(1f),
                    verticalArrangement = Arrangement.Center,
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    CircularProgressIndicator()
                }
            }
            state.history.isEmpty -> {
                Column(
                    modifier = Modifier.fillMaxWidth().weight(1f).padding(24.dp),
                    verticalArrangement = Arrangement.Center,
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Text(
                        text = state.history.emptyTitle,
                        style = MaterialTheme.typography.bodyLarge,
                        modifier = Modifier.testTag(ReaderTestTags.HISTORY_EMPTY_STATE),
                    )
                    Spacer(modifier = Modifier.height(8.dp))
                    Text(text = state.history.emptyDesc, style = MaterialTheme.typography.labelMedium)
                }
            }
            else -> {
                LazyColumn(modifier = Modifier.fillMaxWidth().weight(1f)) {
                    items(items = state.history.rows, key = { row -> row.entryId }) { row ->
                        HistoryRowItem(
                            row = row,
                            onOpenBook = onOpenBook,
                            onClearEntry = onClearEntry,
                        )
                        HorizontalDivider()
                    }
                }
            }
        }
        HorizontalDivider()
        Text(
            text = "共 ${state.history.total} 条",
            style = MaterialTheme.typography.labelMedium,
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 12.dp, vertical = 6.dp)
                .testTag(ReaderTestTags.HISTORY_TOTAL),
        )
        OutlinedButton(
            onClick = { pendingClearAll = true },
            enabled = state.history.rows.isNotEmpty(),
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 12.dp, vertical = 6.dp)
                .testTag(ReaderTestTags.HISTORY_CLEAR_ALL_BUTTON)
                .semantics { contentDescription = "清空全部阅读记录（不删书与进度）" },
        ) {
            Text("清空全部")
        }
    }
}

@Composable
private fun HistoryRowItem(
    row: HistoryRow,
    onOpenBook: (Long) -> Unit,
    onClearEntry: (Long) -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable { onOpenBook(row.bookId) }
            .testTag(ReaderTestTags.historyRow(row.entryId))
            .semantics { contentDescription = "回到《${row.title}》" }
            .padding(horizontal = 12.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(text = row.title, style = MaterialTheme.typography.titleSmall)
            // 时间标签是核心给的成品文本 —— 原样显示（规格 §3.6）。
            Text(text = row.timeLabel, style = MaterialTheme.typography.labelMedium)
            Text(
                text = "${row.fileLabel} · ${row.positionLabel}",
                style = MaterialTheme.typography.labelSmall,
            )
        }
        TextButton(
            onClick = { onClearEntry(row.entryId) },
            modifier = Modifier
                .testTag(ReaderTestTags.historyClearEntry(row.entryId))
                .semantics { contentDescription = "清除这条阅读记录" },
        ) {
            Text("清除")
        }
    }
}

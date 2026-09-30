package com.novelreader.android.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.novelreader.android.reader.ChapterEntry

/**
 * 章节列表浮层（规格 `docs/novel-reader-v1-development-plan.md:611`
 * 「临时菜单提供目录、页码跳转、书签和设置」）。
 *
 * 覆盖式全屏列表：列出全部章节、**高亮当前章**、点击即跳转到该章首页。
 * 本组件不保存任何阅读状态：跳转由 ViewModel 的 `jumpToChapter` 执行。
 *
 * ⚠️ 刻意与 `ReaderScreen.kt` **分文件**：阅读页正文路径必须无滚动容器
 * （NFR-A4，由 `PaginationContractTest.readerBodyNeverUsesAScrollContainer` 按文件静态审计）。
 * 目录是浮层而非正文，两者混在一个文件里会让该审计失去判据。
 */
@Composable
internal fun ChapterListOverlay(
    chapters: List<ChapterEntry>,
    currentChapterIndex: Int?,
    onJump: (Int) -> Unit,
    onClose: () -> Unit,
) {
    Surface(
        modifier = Modifier.fillMaxSize().testTag(ReaderTestTags.READER_CHAPTER_LIST),
        color = MaterialTheme.colorScheme.surface,
    ) {
        Column(modifier = Modifier.fillMaxSize().nrScreenInsets().padding(12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    text = "目录（共 ${chapters.size} 章）",
                    style = MaterialTheme.typography.titleMedium,
                    modifier = Modifier.weight(1f),
                )
                TextButton(
                    onClick = onClose,
                    modifier = Modifier
                        .testTag(ReaderTestTags.READER_CHAPTER_LIST_CLOSE)
                        .semantics { contentDescription = "关闭目录" },
                ) {
                    Text("关闭")
                }
            }
            HorizontalDivider()
            LazyColumn(modifier = Modifier.fillMaxSize()) {
                itemsIndexed(chapters) { index, chapter ->
                    val current = index == currentChapterIndex
                    Text(
                        text = "${index + 1}. ${chapter.title}",
                        style = MaterialTheme.typography.bodyMedium,
                        color = if (current) {
                            MaterialTheme.colorScheme.primary
                        } else {
                            MaterialTheme.colorScheme.onSurface
                        },
                        modifier = Modifier
                            .fillMaxWidth()
                            .clickable { onJump(index) }
                            .padding(horizontal = 8.dp, vertical = 12.dp)
                            .testTag(ReaderTestTags.readerChapterItem(index)),
                    )
                }
            }
        }
    }
}

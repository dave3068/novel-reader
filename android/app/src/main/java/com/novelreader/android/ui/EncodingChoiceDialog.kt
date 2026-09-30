package com.novelreader.android.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.novelreader.android.reader.ContentEncoding

/**
 * 文本编码选择弹框（规格 CR2「用户手动覆盖」）。
 *
 * - 「自动识别」= 清除覆盖（编码码 0），由核心重新检测；
 * - 其余为 7 种受支持编码（UTF-8 / UTF-8 BOM / UTF-16LE / UTF-16BE / GB18030 / GBK / Big5）。
 *
 * 选择后由 `ReaderViewModel.setEncoding` 交给核心：**只改变解释方式，永不写回原文件**，
 * 核心随即按新编码重扫章节索引。
 *
 * ⚠️ 与 `ReaderScreen.kt` **分文件**：阅读页正文路径必须无滚动容器
 * （`PaginationContractTest.readerBodyNeverUsesAScrollContainer` 按文件静态审计），
 * 而本弹框需要 `LazyColumn`。
 */
@Composable
internal fun EncodingChoiceDialog(
    currentName: String,
    onPick: (Int) -> Unit,
    onClose: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onClose,
        modifier = Modifier.testTag(ReaderTestTags.READER_ENCODING_DIALOG),
        title = { Text("选择文本编码（当前：${currentName.ifEmpty { "未知" }}）") },
        text = {
            Column(modifier = Modifier.fillMaxWidth()) {
                Text(
                    text = "选择后立即按新编码重新识别正文与章节；不会修改原文件。",
                    style = MaterialTheme.typography.labelSmall,
                )
                LazyColumn(modifier = Modifier.fillMaxWidth().heightIn(max = 320.dp)) {
                    item(key = "auto") {
                        Text(
                            text = "自动识别（核心检测）",
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.primary,
                            modifier = Modifier
                                .fillMaxWidth()
                                .clickable { onPick(0) }
                                .padding(horizontal = 8.dp, vertical = 12.dp)
                                .testTag(ReaderTestTags.READER_ENCODING_AUTO),
                        )
                    }
                    items(ContentEncoding.entries.toList(), key = { it.abiCode }) { encoding ->
                        Text(
                            text = "${encoding.displayName}（编码码 ${encoding.abiCode}）",
                            style = MaterialTheme.typography.bodyMedium,
                            modifier = Modifier
                                .fillMaxWidth()
                                .clickable { onPick(encoding.abiCode.toInt()) }
                                .padding(horizontal = 8.dp, vertical = 12.dp)
                                .testTag(ReaderTestTags.readerEncodingOption(encoding.abiCode.toInt())),
                        )
                    }
                }
            }
        },
        confirmButton = {
            TextButton(
                onClick = onClose,
                modifier = Modifier
                    .testTag(ReaderTestTags.READER_ENCODING_DIALOG + "-close")
                    .semantics { contentDescription = "关闭编码选择" },
            ) {
                Text("取消")
            }
        },
    )
}

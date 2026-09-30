package com.novelreader.probe.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/**
 * 阅读页。
 *
 * 排版对齐（修复真机「一页装不下、正文可滚动」）：
 *  - 正文约束由本页提供给 ViewModel，**扣掉内边距后的纯文本区域**尺寸；
 *  - 字号与行高都用 ViewModel 实测出的**像素值**（`sp = px / density`），
 *    与 `StaticLayout` 的测量口径完全一致，因此一页恰好铺满、无需滚动。
 *
 * 交互：
 *  - 系统返回键回到书架（H-R7）；
 *  - 正文左 1/3 点击 = 上一页，右 1/3 = 下一页；
 *  - 底部固定「上一页 / 下一页」按钮与状态行（可见反馈）。
 */
@Composable
fun ReaderScreen(
    pageText: String,
    pageOrdinal: Int,
    cacheAhead: Int,
    chapterTitle: String,
    status: String,
    fontSizePx: Float,
    lineHeightPx: Float,
    onPrevious: () -> Unit,
    onNext: () -> Unit,
    onBack: () -> Unit = {},
    onViewport: (Int, Int, Float) -> Unit = { _, _, _ -> },
) {
    BackHandler { onBack() }

    val density = LocalDensity.current

    Column(Modifier.fillMaxSize().padding(horizontal = 12.dp)) {
        Row(
            Modifier.fillMaxWidth().padding(top = 4.dp, bottom = 2.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Text(chapterTitle, style = MaterialTheme.typography.labelLarge)
            Text("第 $pageOrdinal 页 · 预读 $cacheAhead", style = MaterialTheme.typography.labelMedium)
        }

        // 内边距放在 Box 上：BoxWithConstraints 内部拿到的就是纯文本区域尺寸
        BoxWithConstraints(
            Modifier
                .testTag("reader_area")
                .weight(1f)
                .fillMaxWidth()
                .clip(RoundedCornerShape(6.dp))
                .background(MaterialTheme.colorScheme.surfaceVariant)
                .padding(horizontal = 10.dp, vertical = 8.dp)
                .pointerInput(Unit) {
                    detectTapGestures { offset ->
                        when {
                            offset.x < size.width / 3f -> onPrevious()
                            offset.x > size.width * 2f / 3f -> onNext()
                            else -> Unit
                        }
                    }
                }
        ) {
            val widthPx = with(density) { maxWidth.roundToPx() }
            val heightPx = with(density) { maxHeight.roundToPx() }
            LaunchedEffect(widthPx, heightPx, density.density) {
                onViewport(widthPx, heightPx, density.density)
            }

            // 注意：这里**不使用 verticalScroll**。一页应当恰好铺满；
            // 若出现滚动即说明测量口径与渲染不一致（本文件与 ViewModel 已对齐）。
            // 显式指定字号 / 行高 / 字距，避免继承 bodyLarge 的 letterSpacing（0.5sp）：
            // 字符间距会让 Compose 每行少放几个字，从而比测量结果多出一行而被裁切
            // （字号越小，0.5sp 占字宽比例越大，故小字号时才会暴露）。
            val textStyle = MaterialTheme.typography.bodyLarge.copy(
                fontSize = (fontSizePx / density.density).sp,
                lineHeight = (lineHeightPx / density.density).sp,
                letterSpacing = 0.sp,
                fontWeight = FontWeight.Normal,
            )
            Text(
                text = pageText,
                modifier = Modifier.fillMaxSize(),
                style = textStyle,
            )
        }

        Row(
            Modifier.fillMaxWidth().padding(top = 6.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Button(onClick = onPrevious, modifier = Modifier.weight(1f).testTag("btn_prev")) {
                Text("上一页")
            }
            Button(onClick = onNext, modifier = Modifier.weight(1f).testTag("btn_next")) {
                Text("下一页")
            }
        }

        Text(
            status,
            style = MaterialTheme.typography.labelSmall,
            modifier = Modifier
                .fillMaxWidth()
                .padding(top = 4.dp, bottom = 4.dp)
                .testTag("reader_status"),
        )
    }
}

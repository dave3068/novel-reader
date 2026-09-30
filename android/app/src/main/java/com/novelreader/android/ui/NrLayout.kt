package com.novelreader.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

/**
 * 页面顶栏（标题 + 副标题 + 右侧动作）。
 *
 * 顶栏自身**不**处理 insets：内边距由每个页面的根布局统一处理（[nrScreenInsets]），
 * 避免两处叠加。targetSdk 36 强制 edge-to-edge 时，`Scaffold` 的 topBar 槽位在 y=0，
 * 不会自动让出状态栏 —— 真机实测漏加内边距时标题会落在状态栏下面（y≈22），
 * 因此所有页面根布局都必须带 [statusBarsPadding]（NFR-A3）。
 */
@Composable
fun NrTopBar(
    title: String,
    subtitle: String = "",
    actions: @Composable RowScope.() -> Unit,
) {
    Surface(color = MaterialTheme.colorScheme.surfaceVariant, tonalElevation = 2.dp) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .height(56.dp)
                .padding(horizontal = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.Start,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = title,
                    style = MaterialTheme.typography.titleMedium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (subtitle.isNotEmpty()) {
                    Text(
                        text = subtitle,
                        style = MaterialTheme.typography.labelSmall,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            actions()
        }
    }
}

/**
 * 页面根布局的 insets：状态栏（顶部不遮挡）+ 导航栏（底部按钮可点）。
 *
 * 只在这里加一次，页面内部的顶栏与底栏不再各自加。
 */
fun Modifier.nrScreenInsets(): Modifier = statusBarsPadding().navigationBarsPadding()

package com.novelreader.probe.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp

/**
 * 配置页。所有控件都接真实状态与回调（此前滑杆/开关回调为空实现，导致「点了没反应」）。
 */
@Composable
fun SettingsScreen(
    fontScale: Float,
    dark: Boolean,
    status: String,
    selectedFile: String?,
    onFontScale: (Float) -> Unit,
    onDark: (Boolean) -> Unit,
    onOpenFile: () -> Unit,
) {
    Column(
        Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text("阅读设置", style = MaterialTheme.typography.titleLarge)

        Text("字体大小：${fontScale.toInt()} px")
        Slider(
            value = fontScale,
            onValueChange = onFontScale,
            valueRange = 12f..34f,
            steps = 21,
        )
        Text(
            "调整字体会调用共享库的 invalidateLayout 失效页面边界缓存并重排。",
            style = MaterialTheme.typography.bodySmall,
        )

        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("夜间模式", Modifier.weight(1f), style = MaterialTheme.typography.titleMedium)
            Switch(checked = dark, onCheckedChange = onDark, modifier = Modifier.testTag("switch_dark"))
        }

        Button(onClick = onOpenFile, modifier = Modifier.fillMaxWidth().testTag("btn_open_file")) {
            Text("打开本地文件（SAF）")
        }
        Text("已选文件：${selectedFile ?: "（未选择）"}", style = MaterialTheme.typography.bodySmall)

        Text("状态：$status", style = MaterialTheme.typography.bodySmall)
    }
}

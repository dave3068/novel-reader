package com.novelreader.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.novelreader.android.reader.ReaderUiState
import com.novelreader.android.reader.ReaderStatus
import com.novelreader.android.reader.TypographySettings
import com.novelreader.android.reader.milliPx

/**
 * 设置页：排版参数与阅读命令，全部接真实状态。
 *
 * 受控滑杆（AR4）：`value` 来自状态，`onValueChange` **同步**写回 ViewModel 状态，
 * 因此拖动滑块不会弹回旧值，重排只保留最终值。
 *
 * 滚动说明：**本页允许滚动**（设置项在横屏/小屏会超出一屏，否则底部按钮会被挤出屏幕）。
 * 正文区（[ReaderScreen]）**不含任何滚动容器** —— 「不得用滚动容器掩盖分页错误」针对正文。
 */
@Composable
fun SettingsScreen(
    state: ReaderUiState,
    onBack: () -> Unit,
    onFontSizeChange: (Float) -> Unit,
    onLineSpacingChange: (Float) -> Unit,
    onHorizontalMarginChange: (Float) -> Unit,
    onVerticalMarginChange: (Float) -> Unit,
    onDarkThemeChange: (Boolean) -> Unit,
    onRelayout: () -> Unit,
    onSavePosition: () -> Unit,
    onRebuildIndex: () -> Unit,
    onCancelRequest: () -> Unit,
) {
    val settings = state.settings
    val reader = state.reader
    Column(modifier = Modifier.fillMaxSize().nrScreenInsets().testTag(ReaderTestTags.SETTINGS_SCREEN)) {
        NrTopBar(
            title = "设置",
            subtitle = "排版参数 · 阅读命令",
        ) {
            TextButton(
                onClick = onBack,
                modifier = Modifier
                    .testTag(ReaderTestTags.SETTINGS_BACK)
                    .semantics { contentDescription = "返回上一页" },
            ) {
                Text("返回")
            }
        }
        HorizontalDivider()
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .weight(1f)
                // 设置页允许滚动（正文区才是分页禁区）
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            SettingSlider(
                label = "字号",
                valueText = "${settings.fontSizeSp.toInt()} sp（${(settings.fontSizeSp * state.spToPx).toInt()} px）",
                value = settings.fontSizeSp,
                valueRange = TypographySettings.MIN_FONT_SIZE_SP..TypographySettings.MAX_FONT_SIZE_SP,
                steps = 21,
                testTag = ReaderTestTags.SETTINGS_FONT_SIZE_SLIDER,
                contentDescription = "字号",
                onValueChange = onFontSizeChange,
            )
            SettingSlider(
                label = "行距倍数",
                valueText = String.format("%.2fx", settings.lineSpacingMultiplier),
                value = settings.lineSpacingMultiplier,
                valueRange = TypographySettings.MIN_LINE_SPACING_MULTIPLIER..TypographySettings.MAX_LINE_SPACING_MULTIPLIER,
                steps = 19,
                testTag = ReaderTestTags.SETTINGS_LINE_SPACING_SLIDER,
                contentDescription = "行距倍数",
                onValueChange = onLineSpacingChange,
            )
            SettingSlider(
                label = "左右边距",
                valueText = "${settings.horizontalMarginDp.toInt()} dp",
                value = settings.horizontalMarginDp,
                valueRange = TypographySettings.MIN_HORIZONTAL_MARGIN_DP..TypographySettings.MAX_HORIZONTAL_MARGIN_DP,
                steps = 15,
                testTag = ReaderTestTags.SETTINGS_HORIZONTAL_MARGIN_SLIDER,
                contentDescription = "左右边距",
                onValueChange = onHorizontalMarginChange,
            )
            SettingSlider(
                label = "上下边距",
                valueText = "${settings.verticalMarginDp.toInt()} dp",
                value = settings.verticalMarginDp,
                valueRange = TypographySettings.MIN_VERTICAL_MARGIN_DP..TypographySettings.MAX_VERTICAL_MARGIN_DP,
                steps = 15,
                testTag = ReaderTestTags.SETTINGS_VERTICAL_MARGIN_SLIDER,
                contentDescription = "上下边距",
                onValueChange = onVerticalMarginChange,
            )
            Row(
                modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(modifier = Modifier.weight(1f)) {
                    Text("夜间模式", style = MaterialTheme.typography.bodyMedium)
                    Text(
                        text = "只影响配色，不参与排版签名（不触发重排）",
                        style = MaterialTheme.typography.labelSmall,
                    )
                }
                Switch(
                    checked = settings.darkTheme,
                    onCheckedChange = onDarkThemeChange,
                    modifier = Modifier
                        .testTag(ReaderTestTags.SETTINGS_DARK_THEME_SWITCH)
                        .semantics { contentDescription = "夜间模式" },
                )
            }
            HorizontalDivider()
            Text("阅读命令", style = MaterialTheme.typography.titleSmall)
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Button(
                    onClick = onRelayout,
                    enabled = reader.isOpen,
                    modifier = Modifier
                        .weight(1f)
                        .testTag(ReaderTestTags.SETTINGS_RELAYOUT_BUTTON)
                        .semantics { contentDescription = "重新排版" },
                ) {
                    Text("重新排版")
                }
                OutlinedButton(
                    onClick = onSavePosition,
                    enabled = reader.isOpen,
                    modifier = Modifier
                        .weight(1f)
                        .testTag(ReaderTestTags.SETTINGS_SAVE_POSITION_BUTTON)
                        .semantics { contentDescription = "保存阅读位置" },
                ) {
                    Text("保存位置")
                }
            }
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Button(
                    onClick = onRebuildIndex,
                    enabled = reader.isOpen && !reader.indexRebuilding,
                    modifier = Modifier
                        .weight(1f)
                        .testTag(ReaderTestTags.SETTINGS_REBUILD_INDEX_BUTTON)
                        .semantics { contentDescription = "重建章节索引" },
                ) {
                    Text(if (reader.indexRebuilding) "重建中…" else "重建索引")
                }
                OutlinedButton(
                    onClick = onCancelRequest,
                    // 真实状态：只有存在进行中的请求时才可取消（不取串行锁，长任务期间仍可达）。
                    enabled = reader.indexRebuilding,
                    modifier = Modifier
                        .weight(1f)
                        .testTag(ReaderTestTags.SETTINGS_CANCEL_REQUEST_BUTTON)
                        .semantics { contentDescription = "取消索引重建" },
                ) {
                    Text("取消重建")
                }
            }
            HorizontalDivider()
            Text("排版诊断（真实状态）", style = MaterialTheme.typography.titleSmall)
            Text(
                text = diagnostics(state),
                style = MaterialTheme.typography.labelSmall,
                modifier = Modifier.testTag(ReaderTestTags.SETTINGS_DIAGNOSTICS),
            )
            val failure = state.failure
            if (failure != null) {
                Text(
                    text = "最近错误：码 ${failure.rawCode} · ${failure.kind.name} · ${failure.detail.orEmpty()}",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.error,
                )
            }
        }
    }
}

@Composable
private fun SettingSlider(
    label: String,
    valueText: String,
    value: Float,
    valueRange: ClosedFloatingPointRange<Float>,
    steps: Int,
    testTag: String,
    contentDescription: String,
    onValueChange: (Float) -> Unit,
) {
    Column(modifier = Modifier.fillMaxWidth()) {
        Text(text = "$label：$valueText", style = MaterialTheme.typography.bodyMedium)
        Slider(
            value = value,
            onValueChange = onValueChange,
            valueRange = valueRange,
            steps = steps,
            modifier = Modifier
                .fillMaxWidth()
                .testTag(testTag)
                .semantics { this.contentDescription = contentDescription },
        )
    }
}

/** 诊断文本：把测量/签名/核心回执的关键数值原样呈现（便于真机核对口径）。 */
private fun diagnostics(state: ReaderUiState): String {
    val reader = state.reader
    val page = reader.page
    val viewport = if (state.hasViewport) {
        "${state.viewportWidthPx}×${state.viewportHeightPx}px（sp×${state.spToPx} dp×${state.dpToPx}）"
    } else {
        "尚未测量"
    }
    return buildString {
        append("视口：$viewport\n")
        append(
            "内容区：宽 ${(state.viewportWidthPx - 2f * state.settings.horizontalMarginDp * state.dpToPx).toInt()}px · " +
                "高 ${(state.viewportHeightPx - 2f * state.settings.verticalMarginDp * state.dpToPx).toInt()}px\n",
        )
        append(
            "签名：字号 ${milliPx(state.settings.fontSizeSp * state.spToPx)} milli-px · " +
                "额外行距 ${milliPx((state.settings.lineSpacingMultiplier - 1f) * state.settings.fontSizeSp * state.spToPx)} milli-px · " +
                "算法版本 1\n",
        )
        append("世代：layout_id=${reader.layoutId} · request_id=${reader.requestId} · 本地 generation 由 ViewModel 持有\n")
        append("书籍：${reader.bookId ?: "未打开"} · 总字节 ${reader.totalBytes} · 索引版本 ${reader.contentIndexVersion} · 章节 ${reader.chapters.size}\n")
        append(
            "当前页：${page?.let { "偏移 ${it.startByte}→${it.endByte} · ${it.lineCount} 行 · 行高 ${it.lineHeightPx}px · 已登记 ${it.registeredPageCount} 页" } ?: "无"}\n",
        )
        append("预取缓存：${reader.forwardCacheCount} 页 · ABI 版本：${state.abiVersion ?: "未知"}\n")
        append("状态：${state.status.ifEmpty { ReaderStatus.READY }}")
    }
}

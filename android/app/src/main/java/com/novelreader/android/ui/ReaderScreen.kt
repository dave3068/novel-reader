package com.novelreader.android.ui

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.novelreader.android.reader.ReaderInteraction
import com.novelreader.android.reader.ReaderRoute
import com.novelreader.android.reader.ReaderSwipeDirection
import com.novelreader.android.reader.ReaderTapZone
import com.novelreader.android.reader.ReaderUiState
import com.novelreader.android.reader.ReaderStatus
import kotlin.math.abs

/**
 * 阅读页（AR3/AR4）。
 *
 * 关键约定：
 * - **正文区不使用 `verticalScroll`**：一页就是一次测量的结果，整页结束位置由合法 UTF-8 字节偏移
 *   决定（NFR-A4）。滚动容器会掩盖分页错误（半字、裁切、装不下）。
 * - 字号以 sp 渲染，与测量侧的 `fontSizeSp × spToPx` 得到同一像素值；
 *   `lineHeight` 用**测量回传的实测行高**（像素→sp），`letterSpacing = 0.sp` 显式覆盖 Material
 *   `bodyLarge` 自带的 0.5sp（否则每行少放几个字、比测量多出一行而被裁切）。
 * - 正文区域尺寸通过 [onViewportMeasured] 回传，且**必须挂在始终存在的正文容器上**：
 *   `onSizeChanged` 放在容器（`padding` 之外），拿到的是**含边距**的正文区域，边距由测量侧统一扣除。
 *   ⚠️ 若挂到「已渲染出页面」的正文 `Text` 上会形成**死锁** —— 没有视口就排不出页、`page` 恒为 null、
 *   回传节点不存在、视口永远不来（真机实测表现为长期停在「正在排版…（等待正文区域尺寸）」）。
 * - **手势**（规格 `:620`）：左 1/3 点击 = 上一页，右 1/3 = 下一页，中部点击 = 隐藏/显示界面；
 *   另支持左右滑动翻页（左划下一页、右划上一页）。手势与点击**互斥**（拖动后不再按点击处理），
 *   且两种界面状态下都可用。判定逻辑在 [ReaderInteraction]（纯函数，有 JVM 单测）。
 * - **沉浸模式**：隐藏顶栏与底部信息/按钮，只留正文；中部点击切回。切换会改变正文区尺寸，
 *   因而会触发一次重排（页面位置按锚点保持，属预期行为）。
 */
@Composable
fun ReaderScreen(
    state: ReaderUiState,
    onBack: () -> Unit,
    onPreviousPage: () -> Unit,
    onNextPage: () -> Unit,
    onPreviousChapter: () -> Unit,
    onNextChapter: () -> Unit,
    onSavePosition: () -> Unit,
    onOpenSettings: () -> Unit,
    onJumpToChapter: (Int) -> Unit,
    onSetEncoding: (Int) -> Unit,
    onViewportMeasured: (widthPx: Int, heightPx: Int, spToPx: Float, dpToPx: Float) -> Unit,
) {
    val density = LocalDensity.current
    val reader = state.reader
    val page = reader.page

    // 界面显隐（沉浸模式）：字形/旋转后保持，故用 rememberSaveable。
    var chromeVisible by rememberSaveable { mutableStateOf(true) }
    // 目录浮层：瞬态，无需保存。
    var chapterListVisible by remember { mutableStateOf(false) }
    // 编码选择弹框：瞬态。
    var encodingDialogVisible by remember { mutableStateOf(false) }

    val swipeThresholdPx = with(density) { ReaderInteraction.SWIPE_THRESHOLD_DP.dp.toPx() }

    Box(modifier = Modifier.fillMaxSize().testTag(ReaderTestTags.READER_SCREEN)) {
        Column(modifier = Modifier.fillMaxSize().nrScreenInsets()) {
            if (chromeVisible) {
                NrTopBar(
                    title = reader.displayName.ifEmpty { "阅读" },
                    // 不显示页码（切章/重开/改字号即失真），改用与排版无关的阅读进度；
                    // 并显示**当前使用的文本编码**（核心判定值；低可信度时标注待确认）。
                    subtitle = page?.let {
                        val encoding = state.reader.encodingName.ifEmpty { "编码未知" }
                        val pending = if (state.reader.needsEncodingConfirmation) "（待确认）" else ""
                        "${it.chapterTitle} · 进度 ${state.reader.progressText} · $encoding$pending · ${it.lineCount} 行"
                    } ?: ReaderStatus.NO_BOOK,
                ) {
                    TextButton(
                        onClick = { chapterListVisible = true },
                        modifier = Modifier
                            .testTag(ReaderTestTags.READER_CHAPTER_LIST_BUTTON)
                            .semantics { contentDescription = "章节目录" },
                    ) {
                        Text("目录")
                    }
                    TextButton(
                        onClick = onSavePosition,
                        modifier = Modifier
                            .testTag(ReaderTestTags.READER_SAVE_POSITION)
                            .semantics { contentDescription = "保存阅读位置" },
                    ) {
                        Text("保存")
                    }
                    TextButton(
                        onClick = onOpenSettings,
                        modifier = Modifier
                            .testTag(ReaderTestTags.READER_OPEN_SETTINGS)
                            .semantics { contentDescription = "打开排版设置" },
                    ) {
                        Text("设置")
                    }
                    TextButton(
                        onClick = onBack,
                        modifier = Modifier
                            .testTag(ReaderTestTags.READER_BACK)
                            .semantics { contentDescription = "返回书架" },
                    ) {
                        Text("书架")
                    }
                }
                HorizontalDivider()
            }

            // 正文区：一屏一页，无滚动容器。
            // 视口回传挂在**容器**上（与 page 是否已排出无关），避免「等视口→排页→需视口」的死锁。
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .weight(1f)
                    .onSizeChanged { size ->
                        onViewportMeasured(
                            size.width,
                            size.height,
                            density.fontScale * density.density,
                            density.density,
                        )
                    }
                    // 手势：分带点击 + 横滑。两种界面状态下都生效（沉浸时靠它翻页与切回）。
                    .pointerInput(swipeThresholdPx) {
                        awaitEachGesture {
                            val down = awaitFirstDown()
                            val startX = down.position.x
                            var lastX = startX
                            var dragged = false
                            var released = false
                            while (true) {
                                val event = awaitPointerEvent()
                                val change = event.changes.firstOrNull { it.id == down.id } ?: break
                                if (!change.pressed) {
                                    released = true
                                    break
                                }
                                val position = change.position
                                if (position.x != lastX) {
                                    lastX = position.x
                                    if (abs(lastX - startX) > viewConfiguration.touchSlop) dragged = true
                                }
                            }
                            if (!released) return@awaitEachGesture
                            if (dragged) {
                                // 拖动 → 按位移翻页（拖动与点击互斥，避免同一次手势翻两页）
                                when (ReaderInteraction.swipePageFor(lastX - startX, swipeThresholdPx)) {
                                    ReaderSwipeDirection.NEXT_PAGE -> onNextPage()
                                    ReaderSwipeDirection.PREVIOUS_PAGE -> onPreviousPage()
                                    ReaderSwipeDirection.NONE -> Unit
                                }
                            } else {
                                when (ReaderInteraction.tapZoneFor(startX, size.width)) {
                                    ReaderTapZone.NEXT_PAGE -> onNextPage()
                                    ReaderTapZone.PREVIOUS_PAGE -> onPreviousPage()
                                    ReaderTapZone.TOGGLE_CHROME -> chromeVisible = !chromeVisible
                                }
                            }
                        }
                    },
            ) {
                if (page == null) {
                    Text(
                        text = state.status,
                        style = MaterialTheme.typography.bodyMedium,
                        modifier = Modifier.align(Alignment.Center).padding(24.dp),
                    )
                } else {
                    Text(
                        text = page.text,
                        modifier = Modifier
                            .fillMaxSize()
                            // 不在此处回传视口：这里拿到的是扣除 padding 后的更小尺寸，
                            // 会与测量侧的边距扣除叠加（二次扣除）并触发多余重排。
                            .padding(
                                horizontal = state.settings.horizontalMarginDp.dp,
                                vertical = state.settings.verticalMarginDp.dp,
                            )
                            .testTag(ReaderTestTags.READER_BODY),
                        style = TextStyle(
                            fontSize = state.settings.fontSizeSp.sp,
                            lineHeight = with(density) { page.lineHeightPx.toSp() },
                            letterSpacing = 0.sp,
                            fontWeight = FontWeight.Normal,
                            fontFamily = FontFamily.SansSerif,
                            color = MaterialTheme.colorScheme.onSurface,
                            textAlign = TextAlign.Start,
                        ),
                        overflow = TextOverflow.Clip,
                    )
                }
            }

            if (chromeVisible) {
                HorizontalDivider()
                Column(modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 4.dp)) {
                    Text(
                        text = state.status,
                        style = MaterialTheme.typography.labelSmall,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.testTag(ReaderTestTags.READER_STATUS),
                    )
                    // 编码入口与显示同处：用户看到的编码就在这里可直接改（规格 CR2 手动覆盖）。
                    TextButton(
                        onClick = { encodingDialogVisible = true },
                        modifier = Modifier
                            .testTag(ReaderTestTags.READER_ENCODING_BUTTON)
                            .semantics { contentDescription = "选择文本编码（当前 ${state.reader.encodingName}）" },
                    ) {
                        Text(
                            "编码：${state.reader.encodingName.ifEmpty { "未知" }}" +
                                if (state.reader.needsEncodingConfirmation) "（待确认，可手动指定）" else "",
                            style = MaterialTheme.typography.labelSmall,
                        )
                    }
                    val failure = state.failure
                    if (failure != null) {
                        Text(
                            text = "错误码 ${failure.rawCode} · ${failure.kind.name} · ${failure.detail.orEmpty()}",
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.error,
                        )
                    }
                }
                Row(
                    modifier = Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 4.dp),
                    horizontalArrangement = Arrangement.SpaceEvenly,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    OutlinedButton(
                        onClick = onPreviousChapter,
                        enabled = reader.isOpen,
                        modifier = Modifier
                            .testTag(ReaderTestTags.READER_PREVIOUS_CHAPTER)
                            .semantics { contentDescription = "上一章" },
                    ) {
                        Text("上一章")
                    }
                    OutlinedButton(
                        onClick = onPreviousPage,
                        // 真实状态：不在全书第一页即可后退（与访问历史无关）。
                        enabled = reader.canGoPreviousPage,
                        modifier = Modifier
                            .testTag(ReaderTestTags.READER_PREVIOUS_PAGE)
                            .semantics { contentDescription = "上一页" },
                    ) {
                        Text("上一页")
                    }
                    OutlinedButton(
                        onClick = onNextPage,
                        enabled = reader.isOpen,
                        modifier = Modifier
                            .testTag(ReaderTestTags.READER_NEXT_PAGE)
                            .semantics { contentDescription = "下一页" },
                    ) {
                        Text("下一页")
                    }
                    OutlinedButton(
                        onClick = onNextChapter,
                        enabled = reader.isOpen,
                        modifier = Modifier
                            .testTag(ReaderTestTags.READER_NEXT_CHAPTER)
                            .semantics { contentDescription = "下一章" },
                    ) {
                        Text("下一章")
                    }
                }
            }
        }

        if (encodingDialogVisible) {
            EncodingChoiceDialog(
                currentName = state.reader.encodingName,
                onPick = { code ->
                    encodingDialogVisible = false
                    onSetEncoding(code)
                },
                onClose = { encodingDialogVisible = false },
            )
        }

        if (chapterListVisible) {
            // 目录浮层在独立文件（ChapterListOverlay.kt）：正文路径必须保持无滚动容器。
            ChapterListOverlay(
                chapters = reader.chapters,
                currentChapterIndex = page?.chapterIndex,
                onJump = { index ->
                    chapterListVisible = false
                    onJumpToChapter(index)
                },
                onClose = { chapterListVisible = false },
            )
        }
    }
}

/** 路由判定（供 [NovelReaderApp] 的返回行为复用；纯函数，便于单测）。 */
fun backRouteFor(state: ReaderUiState): ReaderRoute =
    if (state.reader.isOpen) ReaderRoute.READER else ReaderRoute.SHELF

/** 从最近阅读返回的目标（与设置页共用同一「返回」语义）。 */
fun backRouteFromHistory(state: ReaderUiState): ReaderRoute =
    if (state.reader.isOpen) ReaderRoute.READER else ReaderRoute.SHELF

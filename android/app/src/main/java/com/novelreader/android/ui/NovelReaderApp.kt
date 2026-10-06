package com.novelreader.android.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.novelreader.android.reader.ReaderRoute
import com.novelreader.android.reader.ReaderViewModel

/**
 * 应用外壳：三个页面（书架 / 阅读 / 设置）与 ViewModel 的全部接线。
 *
 * # 接线说明
 *
 * 本文件是**唯一**把界面控件连到 ViewModel 的地方，因此每个控件的回调都是真实状态变更，
 * 不存在空回调占位（`scripts/verify-android-scope.py` 的 `[3b]` 检查项）。
 *
 * # 挂载点
 *
 * 由 [com.novelreader.android.MainActivity] 以
 * `setContent { NovelReaderApp(onImportRequest = { …OpenDocument… }, buildMarker = …, viewModel = …) }`
 * 挂载；宿主负责注册系统文件选择器（`SafHost`）、传入落盘 `dbPath` 与平台来源转发器。
 *
 * # 构建标记
 *
 * [buildMarker] 只在**书架页**以右下角小字显示（真机核对「装的是哪一次构建」，子规格 A-T7）。
 * 采用覆盖式角标而非插入布局流：不改变任何页面的既有测量与分页前提。
 *
 * `onPickBackgroundImage`：拉起系统选图器（V1.1 背景图），由宿主注入。
 * 为 `null` 时点「选择图片…」会**如实回报**「宿主未提供选图入口」，不给无反应的按钮。
 */
@Composable
fun NovelReaderApp(
    onImportRequest: () -> Unit,
    onPickBackgroundImage: (() -> Unit)? = null,
    buildMarker: String = "",
    viewModel: ReaderViewModel = viewModel(factory = ReaderViewModel.productionFactory()),
) {
    val state by viewModel.ui.collectAsStateWithLifecycle()
    MaterialTheme(
        colorScheme = if (state.settings.darkTheme) darkColorScheme() else lightColorScheme(),
    ) {
        Box(modifier = Modifier.fillMaxSize()) {
            Surface(modifier = Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
                when (state.route) {
                    ReaderRoute.SHELF -> ShelfScreen(
                        shelf = state.shelf,
                        categories = state.categories,
                        reader = state.reader,
                        abiVersion = state.abiVersion,
                        status = state.status,
                        onOpenBook = viewModel::openBook,
                            onRemoveBook = viewModel::removeBook,
                        onImportRequest = onImportRequest,
                        onOpenReader = viewModel::openReader,
                        onOpenSettings = viewModel::openSettings,
                        onOpenHistory = viewModel::openHistory,
                        onSelectCategory = viewModel::onSelectCategory,
                        onToggleCategory = viewModel::onToggleCategory,
                        onCreateCategory = { name -> viewModel.onCreateCategory(name) },
                        onRenameCategory = viewModel::onRenameCategory,
                        categoryTargets = state.categoryTargets,
                        onLoadCategoryTargets = viewModel::onLoadCategoryTargets,
                        onClearCategoryTargets = viewModel::clearCategoryTargets,
                        onMoveCategory = viewModel::onMoveCategory,
                        onDeleteCategory = viewModel::onDeleteCategory,
                    )

                    ReaderRoute.HISTORY -> HistoryScreen(
                        state = state,
                        onBack = {
                            when (backRouteFromHistory(state)) {
                                ReaderRoute.READER -> viewModel.openReader()
                                ReaderRoute.SHELF -> viewModel.openShelf()
                                ReaderRoute.SETTINGS -> viewModel.openSettings()
                                ReaderRoute.HISTORY -> viewModel.openShelf()
                            }
                        },
                        onOpenBook = viewModel::openHistoryBook,
                        onClearEntry = viewModel::onClearHistoryEntry,
                        onClearAll = viewModel::onClearAllHistory,
                    )

                    ReaderRoute.READER -> ReaderScreen(
                        state = state,
                        onBack = viewModel::openShelf,
                        onPreviousPage = viewModel::previousPage,
                        onNextPage = viewModel::nextPage,
                        onPreviousChapter = viewModel::previousChapter,
                        onNextChapter = viewModel::nextChapter,
                        onSavePosition = viewModel::savePositionNow,
                        onOpenSettings = viewModel::openSettings,
                        onJumpToChapter = viewModel::jumpToChapter,
                        onSetEncoding = viewModel::setEncoding,
                        onViewportMeasured = viewModel::onViewportMeasured,
                    )

                    ReaderRoute.SETTINGS -> SettingsScreen(
                        state = state,
                        onBack = {
                            when (backRouteFor(state)) {
                                ReaderRoute.READER -> viewModel.openReader()
                                ReaderRoute.SHELF -> viewModel.openShelf()
                                ReaderRoute.SETTINGS -> viewModel.openSettings()
                                ReaderRoute.HISTORY -> viewModel.openShelf()
                            }
                        },
                        onFontSizeChange = viewModel::onFontSizeChange,
                        onLineSpacingChange = viewModel::onLineSpacingChange,
                        onHorizontalMarginChange = viewModel::onHorizontalMarginChange,
                        onVerticalMarginChange = viewModel::onVerticalMarginChange,
                        onDarkThemeChange = viewModel::onDarkThemeChange,
                        onThemeEnabledChange = viewModel::onThemeEnabledChange,
                        onThemeBaseChange = viewModel::onThemeBaseChange,
                        onBackgroundScrimChange = viewModel::onBackgroundScrimChange,
                        onBackgroundPick = { fit, scrim ->
                            viewModel.onBackgroundPickRequest(fit, scrim, onPickBackgroundImage)
                        },
                        onBackgroundClear = viewModel::onBackgroundClear,
                        onRelayout = viewModel::requestRelayout,
                        onSavePosition = viewModel::savePositionNow,
                        onRebuildIndex = viewModel::rebuildIndex,
                        onCancelRequest = viewModel::cancelIndexRebuild,
                    )
                }
            }

            if (buildMarker.isNotEmpty() && state.route == ReaderRoute.SHELF) {
                Text(
                    text = "构建标记 $buildMarker",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier
                        .align(Alignment.BottomEnd)
                        .padding(12.dp),
                )
            }
        }
    }
}

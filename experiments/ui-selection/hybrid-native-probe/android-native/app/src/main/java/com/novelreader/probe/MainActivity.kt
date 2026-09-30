package com.novelreader.probe

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.novelreader.probe.data.SafBookSource
import com.novelreader.probe.reader.ReaderViewModel
import com.novelreader.probe.ui.BookshelfScreen
import com.novelreader.probe.ui.ReaderScreen
import com.novelreader.probe.ui.SettingsScreen
import com.novelreader.probe.ui.theme.NovelReaderTheme

/**
 * 宿主 Activity。
 *
 * 修复要点（真机反馈）：
 *  1. `enableEdgeToEdge()` + `Scaffold` + 顶栏 `statusBarsPadding()` → 顶部不再被状态栏遮挡；
 *  2. 顶栏压缩为**单行**（标题 + 状态），减少上方空白；
 *  3. 三个页面全部接入真实状态与回调；正文视口尺寸与密度上报给 ViewModel，
 *     使「一页恰好铺满、无需滚动」；
 *  4. SAF 选中文件后交给共享库 `open_external` 建立单章坐标空间，正文即可正常分页显示。
 */
class MainActivity : ComponentActivity() {

    private lateinit var readerViewModel: ReaderViewModel
    private lateinit var safSource: SafBookSource
    private lateinit var lifecycleObserver: LifecycleEventObserver

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()

        readerViewModel = ViewModelProvider(this)[ReaderViewModel::class.java]
        safSource = SafBookSource(this) { uri ->
            if (uri != null) {
                val displayName = safSource.displayNameOf(uri)
                val sizeBytes = safSource.sizeOf(uri)
                readerViewModel.onFilePicked(displayName, sizeBytes) { offset, length ->
                    safSource.readChunk(uri, offset, length)
                }
            }
        }

        lifecycleObserver = LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_STOP -> readerViewModel.persistCurrentPosition()
                Lifecycle.Event.ON_START -> readerViewModel.loadBook()
                else -> Unit
            }
        }
        lifecycle.addObserver(lifecycleObserver)

        setContent {
            val state by readerViewModel.ui.collectAsStateWithLifecycle()
            NovelReaderTheme(darkTheme = state.dark) {
                var tab by rememberSaveable { mutableIntStateOf(1) }
                Scaffold(
                    topBar = { StatusHeader(state.status) },
                    bottomBar = {
                        NavigationBar {
                            NavigationBarItem(
                                modifier = Modifier.testTag("tab_shelf"),
                                selected = tab == 0,
                                onClick = { tab = 0 },
                                icon = { Text("目录") },
                                label = { Text("书架") },
                            )
                            NavigationBarItem(
                                modifier = Modifier.testTag("tab_reader"),
                                selected = tab == 1,
                                onClick = { tab = 1 },
                                icon = { Text("正文") },
                                label = { Text("阅读") },
                            )
                            NavigationBarItem(
                                modifier = Modifier.testTag("tab_settings"),
                                selected = tab == 2,
                                onClick = { tab = 2 },
                                icon = { Text("配置") },
                                label = { Text("设置") },
                            )
                        }
                    },
                ) { innerPadding ->
                    Box(Modifier.fillMaxSize().padding(innerPadding)) {
                        when (tab) {
                            0 -> BookshelfScreen(
                                books = state.books,
                                selectedId = state.selectedBookId,
                                onBookSelected = { book ->
                                    readerViewModel.loadBook(book.id)
                                    tab = 1
                                },
                                onOpenFile = { safSource.requestBook() },
                            )

                            2 -> SettingsScreen(
                                fontScale = state.fontScale,
                                dark = state.dark,
                                status = state.status,
                                selectedFile = state.selectedFile,
                                onFontScale = readerViewModel::setFontScale,
                                onDark = readerViewModel::setDark,
                                onOpenFile = { safSource.requestBook() },
                            )

                            else -> ReaderScreen(
                                pageText = state.pageText,
                                pageOrdinal = state.pageOrdinal,
                                cacheAhead = state.cacheAhead,
                                chapterTitle = state.chapterTitle,
                                status = state.status,
                                fontSizePx = state.fontSizePx,
                                lineHeightPx = state.lineHeightPx,
                                onPrevious = readerViewModel::previousPage,
                                onNext = readerViewModel::nextPage,
                                onBack = { tab = 0 },
                                onViewport = readerViewModel::setViewport,
                            )
                        }
                    }
                }
            }
        }
    }

    override fun onDestroy() {
        lifecycle.removeObserver(lifecycleObserver)
        super.onDestroy()
    }
}

/** 压缩为单行的顶栏：标题 + 状态（超出省略），让出状态栏高度，减少上方空白。 */
@Composable
private fun StatusHeader(status: String) {
    Column(
        Modifier
            .fillMaxWidth()
            .statusBarsPadding()
            .padding(start = 12.dp, end = 12.dp, top = 2.dp, bottom = 4.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                "novel-reader 探针 ${BuildConfig.BUILD_MARKER}",
                style = MaterialTheme.typography.labelLarge,
                modifier = Modifier.padding(end = 8.dp),
            )
            Text(
                status,
                style = MaterialTheme.typography.labelSmall,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

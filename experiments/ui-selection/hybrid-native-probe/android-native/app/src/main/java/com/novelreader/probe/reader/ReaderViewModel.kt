package com.novelreader.probe.reader

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.novelreader.probe.core.NrEngine
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import org.json.JSONObject

/** 书架条目（由共享库书籍批次的传输 JSON 解码而来）。 */
data class BookItem(
    val id: Long,
    val title: String,
    val author: String,
    val meta: String,
)

/**
 * 纯 UI 编排：线程切换、状态投影、生命周期。
 *
 * 所有阅读语义（有限窗口读取、页边界登记、位置/锚点、缓存失效、取消、错误模型）
 * 都在共享库（Rust）内完成；本类只做 JNI 编解码与状态发布。
 *
 * 关键修复：
 *  - 排版尺寸统一为像素（`sp × density`），并与 Compose 渲染共用实测行高 → 一页恰好铺满；
 *  - 受控滑杆的 `fontScale` **同步**写回状态，避免拖动时滑块弹回；
 *  - 重排请求做**合并**（拖拽中只保留最终值），避免几十次并发重排互相覆盖；
 *  - 所有状态变更经 `Mutex` 串行化，避免并发重填预读缓存；
 *  - SAF 选中的文件通过共享库 `open_external` 建立单章坐标空间后即可正常分页显示。
 */
class ReaderViewModel : ViewModel() {

    data class UiState(
        val pageText: String = "正在加载…",
        val pageOrdinal: Int = 1,
        val cacheAhead: Int = 0,
        val chapterTitle: String = "第 1 章",
        val status: String = "就绪",
        /** 用户可见字号（sp）。 */
        val fontScale: Float = DEFAULT_FONT_SCALE,
        /** 实际用于排版与渲染的字号（像素）。 */
        val fontSizePx: Float = DEFAULT_FONT_SCALE * 3f,
        /** 实测行高（像素），供 Compose 对齐。 */
        val lineHeightPx: Float = DEFAULT_FONT_SCALE * 3f * LINE_SPACING_MULTIPLIER,
        val dark: Boolean = false,
        val selectedFile: String? = null,
        val books: List<BookItem> = emptyList(),
        val selectedBookId: Long = DEFAULT_BOOK_ID,
    )

    private val engine = NrEngine()

    private var viewportWidthPx = 0
    private var viewportHeightPx = 0
    private var density = 3f
    private var fontScale = DEFAULT_FONT_SCALE
    private var externalWindow: (suspend (Int, Long, Int) -> ByteArray)? = null
    private var layoutId = DEFAULT_LAYOUT_ID
    private var layouter = buildLayouter()

    private var chapter = 0
    private var position = 0L
    private var ordinal = 1
    private val backStack = ArrayDeque<Long>()
    private val ahead = ArrayDeque<AndroidPageLayouter.Page>()
    private var current: AndroidPageLayouter.Page? = null

    /** 串行化状态变更（并发重填会让预读缓存膨胀，真机实测出现过「预读 9 页」）。 */
    private val gate = Mutex()
    private var reflowJob: Job? = null
    private var reflowDirty = false

    private val _ui = MutableStateFlow(UiState())
    val ui: StateFlow<UiState> = _ui.asStateFlow()

    private fun buildLayouter() = AndroidPageLayouter(
        engine = engine,
        widthPx = viewportWidthPx.coerceAtLeast(320),
        heightPx = viewportHeightPx.coerceAtLeast(480),
        fontSizePx = fontScale * density,
        lineSpacingMultiplier = LINE_SPACING_MULTIPLIER,
        layoutId = layoutId,
        byteWindow = externalWindow,
    )

    private fun computeLayoutId(): Long {
        var hash = 1125899906842597L
        hash = hash * 31 + (fontScale * 10).toLong()
        hash = hash * 31 + viewportWidthPx
        hash = hash * 31 + viewportHeightPx
        hash = hash * 31 + if (externalWindow != null) 1L else 0L
        return hash and Long.MAX_VALUE
    }

    init {
        loadBooks()
    }

    // ------------------------------------------------------------------
    // 书籍批次（薄解码）
    // ------------------------------------------------------------------

    private fun loadBooks() = viewModelScope.launch {
        runCatching {
            val json = withContext(Dispatchers.IO) {
                String(engine.getBookBatch(0L, 20), Charsets.UTF_8)
            }
            parseBooks(json)
        }.onSuccess { books ->
            _ui.value = _ui.value.copy(books = books)
            loadBook(_ui.value.selectedBookId)
        }.onFailure { error ->
            setStatus("读取书籍批次失败：${error.message}")
        }
    }

    private fun parseBooks(json: String): List<BookItem> {
        val items = JSONObject(json).optJSONArray("items") ?: return emptyList()
        return buildList {
            for (i in 0 until items.length()) {
                val o = items.optJSONObject(i) ?: continue
                add(
                    BookItem(
                        id = o.optLong("id"),
                        title = o.optString("title"),
                        author = o.optString("author"),
                        meta = "${o.optInt("chapterCount")} 章 / ${o.optLong("totalBytes") / 1024} KB",
                    )
                )
            }
        }
    }

    // ------------------------------------------------------------------
    // 视口与书籍
    // ------------------------------------------------------------------

    /** 由 Compose 汇报**纯文本区域**尺寸（像素）与屏幕密度。 */
    fun setViewport(widthPx: Int, heightPx: Int, densityValue: Float) {
        if (widthPx <= 0 || heightPx <= 0 || densityValue <= 0f) return
        if (widthPx == viewportWidthPx && heightPx == viewportHeightPx &&
            kotlin.math.abs(densityValue - density) < 0.01f
        ) {
            return
        }
        viewportWidthPx = widthPx
        viewportHeightPx = heightPx
        density = densityValue
        requestReflow("排版就绪")
    }

    fun loadBook(bookId: Long = DEFAULT_BOOK_ID) = viewModelScope.launch {
        gate.withLock {
            runCatching {
                withContext(Dispatchers.IO) { engine.openBook(bookId) }
                chapter = 0
                ordinal = 1
                backStack.clear()
                current = null
                externalWindow = null
                _ui.value = _ui.value.copy(selectedBookId = bookId, selectedFile = null)
                relaunchLayout("已打开书籍 #$bookId")
            }.onFailure { setStatus("打开书籍失败：${it.message}") }
        }
    }

    /**
     * 已选中的外部文件（SAF）。共享库只登记单章坐标空间，字节由 [reader] 按窗口提供，
     * 因此 SAF 文件同样能正常分页、翻页并保存位置。
     */
    fun onFilePicked(displayName: String, sizeBytes: Long, reader: (Long, Int) -> ByteArray) {
        _ui.value = _ui.value.copy(selectedFile = displayName)
        if (sizeBytes <= 0L) {
            setStatus("无法读取文件大小：$displayName")
            return
        }
        viewModelScope.launch {
            gate.withLock {
                runCatching {
                    withContext(Dispatchers.IO) { engine.openExternalBook(DEFAULT_BOOK_ID, sizeBytes) }
                    chapter = 0
                    ordinal = 1
                    backStack.clear()
                    current = null
                    externalWindow = { _, offset, length -> reader(offset, length) }
                    relaunchLayout("已打开文件 $displayName（$sizeBytes 字节）")
                }.onFailure { setStatus("打开文件失败：${it.message}") }
            }
        }
    }

    // ------------------------------------------------------------------
    // 排版
    // ------------------------------------------------------------------

    /**
     * 合并式重排请求：拖拽字号时只保留最终值，且全部经同一把锁串行执行。
     */
    private fun requestReflow(action: String) {
        reflowDirty = true
        if (reflowJob?.isActive == true) return
        reflowJob = viewModelScope.launch {
            gate.withLock {
                while (reflowDirty) {
                    reflowDirty = false
                    runCatching { relaunchLayout(action) }
                        .onFailure { setStatus("重排失败：${it.message}") }
                }
            }
        }
    }

    /** 以当前配置重新建立排版（必要时失效共享库中的旧边界缓存）。 */
    private suspend fun relaunchLayout(action: String) {
        val previous = layoutId
        layoutId = computeLayoutId()
        if (layoutId != previous) {
            runCatching { withContext(Dispatchers.IO) { engine.invalidateLayout(previous) } }
        }
        layouter = buildLayouter()
        val restart = current?.startByte ?: position
        val page = withContext(Dispatchers.IO) { layouter.measurePage(chapter, restart) }
        current = page
        position = restart
        backStack.clear()
        refillAhead()
        publish(page, action)
    }

    /** 三页预读缓存（规格要求的三页缓存）。 */
    private suspend fun refillAhead() = withContext(Dispatchers.IO) {
        ahead.clear()
        var start = current?.endByte ?: return@withContext
        repeat(PAGE_CACHE) {
            val page = layouter.measurePage(chapter, start)
            if (page.text.isEmpty()) return@repeat
            ahead.addLast(page)
            start = page.endByte
        }
    }

    // ------------------------------------------------------------------
    // 翻页
    // ------------------------------------------------------------------

    fun nextPage() = viewModelScope.launch {
        gate.withLock {
            runCatching {
                val next = ahead.removeFirstOrNull()
                    ?: withContext(Dispatchers.IO) {
                        layouter.measurePage(chapter, current?.endByte ?: 0L)
                    }.takeIf { it.text.isNotEmpty() }
                if (next == null) {
                    setStatus("已到本章末尾（第 $ordinal 页）")
                    return@launch
                }
                backStack.addLast(current?.startByte ?: 0L)
                current = next
                position = next.startByte
                ordinal += 1
                refillAhead()
                persistPosition()
                publish(next, "下一页")
            }.onFailure { setStatus("翻页失败：${it.message}") }
        }
    }

    fun previousPage() = viewModelScope.launch {
        gate.withLock {
            runCatching {
                val previousStart = backStack.removeLastOrNull()
                if (previousStart == null) {
                    setStatus("已到本章开头（第 $ordinal 页）")
                    return@launch
                }
                val page = withContext(Dispatchers.IO) { layouter.measurePage(chapter, previousStart) }
                if (page.text.isEmpty()) {
                    setStatus("上一页为空")
                    return@launch
                }
                current = page
                position = page.startByte
                ordinal = (ordinal - 1).coerceAtLeast(1)
                refillAhead()
                persistPosition()
                publish(page, "上一页")
            }.onFailure { setStatus("翻页失败：${it.message}") }
        }
    }

    // ------------------------------------------------------------------
    // 设置
    // ------------------------------------------------------------------

    /**
     * 字号（sp）。**同步**写回状态，保证受控滑杆不会弹回、文案立即响应；
     * 重排走合并式请求。
     */
    fun setFontScale(value: Float) {
        val next = value.coerceIn(12f, 34f)
        if (kotlin.math.abs(next - fontScale) < 0.01f) return
        fontScale = next
        _ui.value = _ui.value.copy(fontScale = next, fontSizePx = next * density)
        requestReflow("字号 ${next.toInt()} sp")
    }

    fun setDark(value: Boolean) {
        _ui.value = _ui.value.copy(dark = value, status = if (value) "夜间模式" else "日间模式")
    }

    /** 显式保存当前位置（薄委托，非共享入口实现）。 */
    fun persistCurrentPosition() = viewModelScope.launch {
        runCatching { persistPosition() }.onFailure { setStatus("位置保存失败：${it.message}") }
    }

    /** 排版配置变化后失效边界缓存并重排（薄委托）。 */
    fun reflowAfterSettingsChange() = viewModelScope.launch {
        gate.withLock {
            runCatching {
                withContext(Dispatchers.IO) { engine.invalidateLayout(layoutId) }
                refillAhead()
                setStatus("已失效布局缓存并重排")
            }.onFailure { setStatus("重排失败：${it.message}") }
        }
    }

    /** 取消进行中的请求（薄委托）。 */
    fun abortRequest(requestId: Long) = viewModelScope.launch {
        runCatching { engine.cancelRequest(requestId) }
            .onSuccess { setStatus("已取消请求 #$requestId") }
            .onFailure { setStatus("取消失败：${it.message}") }
    }

    // ------------------------------------------------------------------
    // 内部
    // ------------------------------------------------------------------

    private suspend fun persistPosition() {
        withContext(Dispatchers.IO) {
            engine.savePosition(_ui.value.selectedBookId, chapter, position)
        }
    }

    private fun setStatus(message: String) {
        _ui.value = _ui.value.copy(status = message)
    }

    private fun publish(page: AndroidPageLayouter.Page, action: String) {
        _ui.value = _ui.value.copy(
            pageText = page.text.ifEmpty { "（本章末尾）" },
            pageOrdinal = ordinal,
            cacheAhead = ahead.size,
            chapterTitle = if (externalWindow != null) "外部文件" else "第 ${chapter + 1} 章",
            fontSizePx = fontScale * density,
            lineHeightPx = page.lineHeightPx,
            status = "$action · 第 $ordinal 页 · 章内偏移 ${page.startByte} · ${page.lineCount} 行 · " +
                "预读 ${ahead.size} 页 · 字号 ${fontScale.toInt()} sp（${(fontScale * density).toInt()} px）",
        )
    }

    override fun onCleared() {
        engine.close()
        super.onCleared()
    }

    companion object {
        const val DEFAULT_BOOK_ID = 1L
        const val DEFAULT_LAYOUT_ID = 1L
        const val DEFAULT_FONT_SCALE = 18f
        const val LINE_SPACING_MULTIPLIER = 1.35f

        /** 规格要求的三页缓存。 */
        const val PAGE_CACHE = 3
    }
}

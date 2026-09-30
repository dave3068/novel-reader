package com.novelreader.android.reader

import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import com.novelreader.android.core.NrAbiConstants
import com.novelreader.android.core.NrChapterCodec
import com.novelreader.android.core.NrEngine
import com.novelreader.android.core.NrErrorKind
import com.novelreader.android.core.NrErrors
import com.novelreader.android.core.NrException
import com.novelreader.android.core.NrOpenRequest
import com.novelreader.android.core.NrPageBoundarySubmission
import com.novelreader.android.core.NrReleaser
import com.novelreader.android.core.NrSourceProvider
import com.novelreader.android.core.NrSavePositionRequest
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.util.concurrent.atomic.AtomicLong

/**
 * 阅读编排（子规格 A-T3：三页面 + 真实状态接线；AR3/AR4）。
 *
 * # 职责边界
 *
 * - 只做**编排与状态投影**：调用薄封装（[NrEngine]），把回执发布成 [ReaderUiState]；
 * - 章节识别、位置校验、页边界合法性、缓存失效、锚点恢复**全部**由 Rust `reading-core` 裁定
 *   （Kotlin 不做这些业务）；
 * - 平台侧必须做的两件事是：**文本测量**（[TextMeasurer]，公开 Android 排版 API）
 *   与**UI 状态编排**（本类）。
 *
 * # 并发与过期结果（AR4）
 *
 * - 打开/切章/翻页/重排/保存位置/索引重建全部经**同一把 [gate]** 串行执行：不允许两个挂起入口
 *   并发修改阅读状态或并发重填前向缓存；
 * - 字号等设置**同步**写回状态（受控滑杆不得弹回），重排请求**合并**：拖动期间只保留最终值，
 *   由单一重排作业 `while (dirty)` 收敛；
 * - 每次排版推进本地 [generation]，测量结果在发布前必须重新比对世代；核心判定的
 *   `STALE_LAYOUT_ID / STALE_REQUEST_ID / STALE_CONTENT_INDEX_VERSION` 一律丢弃并按当前世代重试，
 *   绝不用过期结果覆盖当前页面；
 * - 宿主生命周期（旋转 / 后台）用 [cancelInFlightWork] **取消在途任务**：取消只作用于协程，
 *   绝不销毁引擎句柄（句柄只在 [onCleared] 释放），因此返回前台后可继续翻页、保存与重排；
 * - 取消不是业务失败：`CancellationException` 在任何 native 边界都原样向上传播（[throwIfCancellation]），
 *   不得被映射成 [ReaderFailure]，也不得吞掉结构化取消；
 * - [cancelIndexRebuild] **不取** [gate]：长任务运行期间取消必须仍然可达（头文件 §7）。
 *
 * # 可测试性
 *
 * 宿主机无法加载 arm64 的 `libnr_jni.so`，因此单测通过 `NrEngine.createWith` 注入脚本化 ABI、
 * 通过 [TextMeasurer] 注入确定性测量器，并通过 [workScope]/[io] 使用测试调度器。
 */
class ReaderViewModel(
    private val engine: NrEngine,
    private val releaser: NrReleaser = NrReleaser.JNI,
    private val measurer: TextMeasurer = AndroidTextMeasurer(),
    private val io: CoroutineDispatcher = Dispatchers.IO,
    /**
     * 宿主解析 SAF 来源（书架再次打开平台窗口来源时使用）。
     *
     * `null` = 宿主未提供（单测/桌面式纯文件场景）：此时平台窗口来源无法重开，走文件通道原逻辑。
     */
    private val hostResolver: HostSourceResolver? = null,
    /**
     * 页锚点持久化端口（规格：上一页优先使用边界栈或**持久化稀疏锚点**）。
     *
     * `null` = 宿主未提供：退化为「仅会话内边界栈」，「上一页」改走位置兜底路径。
     */
    private val anchorStore: ReaderAnchorStore? = null,
    /** 排版设置持久化端口（宿主 KV）。`null` = 不持久化，设置仅存活于本次会话。 */
    private val settingsStore: ReaderSettingsStore? = null,
    /** 书架目录端口（宿主登记的来源）。`null` = 书架仅显示本次会话打开过的书籍。 */
    private val bookCatalog: HostBookCatalog? = null,
    private val hiddenBooks: HiddenBooksStore? = null,
    workScope: CoroutineScope? = null,
) : ViewModel() {

    /** 状态变更的单一串行入口（生产 = `viewModelScope`；单测注入受控 scope）。 */
    private val work: CoroutineScope = workScope ?: viewModelScope

    /** 阅读状态串行锁：所有会改动阅读状态的挂起入口都必须经它。 */
    private val gate = Mutex()

    /**
     * 在途任务登记表（宿主生命周期可取消；见 [cancelInFlightWork]）。
     *
     * 只在 [launchSerial] 里增删，读写都用 [inFlight] 自身做同步 —— 取消必须能**不取** [gate] 地
     * 立刻生效（[gate] 可能正被长任务持有）。
     */
    private val inFlight = LinkedHashSet<Job>()

    /** 状态写入的原子化锁：`update` 是「读-改-写」，宿主线程与 gate 内的协程都会写。 */
    private val uiLock = Any()

    private val _ui = MutableStateFlow(ReaderUiState())

    /** 唯一状态源（Compose `collectAsStateWithLifecycle` 消费）。 */
    val ui: StateFlow<ReaderUiState> = _ui.asStateFlow()

    // ---- 会话状态（都在 gate 内读写） ----
    private val shelf = LinkedHashMap<String, ShelfBook>()

    /**
     * 「已移除」来源标识（用户从书架移除的书）。
     *
     * 语义（用户裁定）：仅移除书架、**绝不删除原文件**；重新导入同一文件即恢复显示。
     * 目录重建书架时据此过滤，避免移除过的书在下次启动又出现。
     */
    private val hiddenBooksSet = LinkedHashSet<String>()
    private val chapters = ArrayList<ChapterEntry>()
    /** 当前打开书籍的来源标识（边界栈与稀疏锚点的持久化键）。 */
    private var currentBookKey: String? = null

    /**
     * 当前书籍的**内容编码**（来自核心 `nr_open_result.encoding`）。
     *
     * 解码正文窗口、章节标题与计算页边界字节长度**必须**统一用它：
     * 核心的坐标空间是原始文件字节，用错编码既会显示乱码，也会让页边界被判为
     * `NR_ERR_UTF8_BOUNDARY`（真机实测缺陷）。
     */
    private var contentEncoding: ContentEncoding = ContentEncoding.DEFAULT

    /** 本会话访问过的页锚点（有界）——稀疏锚点的候选集，用于持久化。 */
    private val sparseAnchors = ArrayDeque<PageAnchor>()
    private val forwardCache = ArrayDeque<PageState>()

    /**
     * 当前排版世代**已测量并已提交**的页（按起始字节索引）。
     *
     * 作用：预取与翻页共用同一份测量结果，避免同一页被重复测量/重复提交
     * （真机原型曾因重复预取把缓存撑到 9 页）。新世代建立时整体作废。
     */
    private val measuredPages = LinkedHashMap<Long, PageState>()
    private var restoredAnchor = PageAnchor(0, 0L)

    /**
     * 上一次**成功发布**页面所用的排版参数（`null` = 尚未排版过）。
     *
     * 用途：宿主回到前台时判断「界面上的页面是否还代表当前设置/视口」——只比像素参数字段，
     * 不做任何排版业务判断（断行与页边界仍由核心 + 平台测量决定）。
     */
    private var laidOutSpec: TextLayoutSpec? = null
    private var ordinal = 0
    private var layoutId = 0L
    private var requestId = 0L

    /**
     * 本地排版世代：设置/视口每次变化都推进，用于丢弃过期测量结果。
     *
     * 用 [AtomicLong] 而不是 `var Long`：宿主的生命周期取消（[cancelInFlightWork]）会在**不取**
     * [gate] 的情况下推进它，而 gate 内的排版也在推进它 —— 非原子自增会丢增量，
     * 让「过期结果在发布前被丢弃」这条判据失效。
     */
    private val generation = AtomicLong(0L)
    private var reflowJob: Job? = null
    private var reflowDirty = false
    private var reflowLabel = ReaderStatus.REFLOW
    private var forceInvalidate = false

    /** 索引重建请求号；[cancelIndexRebuild] 在 gate 外读取，因此与 [generation] 一样用原子量。 */
    private val rebuildRequestId = AtomicLong(0L)

    init {
        reportAbiVersion()
        // 启动期防御：宿主会话存储/目录异常只应导致「设置未恢复 / 书架为空」，
        // 绝不能让应用启动即崩溃（真机实测过一次启动闪退，教训即此）。
// 「已移除」书籍：启动即载入，供目录重建书架时过滤。
runCatching { hiddenBooks?.load()?.let { hiddenBooksSet.addAll(it) } }
        runCatching { loadPersistedSettings() }
        runCatching { refreshShelfFromCatalog() }
    }

    // ------------------------------------------------------------------ 路由与视口

    /** 显示书架页。 */
    fun openShelf() {
        // 每次进入书架都从宿主目录补齐一次（重启后书架因此不再为空）。
        refreshShelfFromCatalog()
        update { it.copy(route = ReaderRoute.SHELF) }
    }

    /** 显示设置页（三个页面互不改变阅读状态）。 */
    fun openSettings() {
        update { it.copy(route = ReaderRoute.SETTINGS) }
    }

    /** 回到阅读页（未打开书籍时保持真实状态并给出提示）。 */
    fun openReader() {
        val state = _ui.value
        if (!state.reader.isOpen) {
            setStatus(ReaderStatus.NO_BOOK)
            return
        }
        update { it.copy(route = ReaderRoute.READER) }
    }

    /**
     * Compose 汇报**正文区域**尺寸与换算因子（AR3 像素口径）。
     *
     * `onSizeChanged` 给的是正文区域（含边距设置）的像素尺寸；`spToPx` 含系统字体缩放，
     * `dpToPx` 不含 —— 两者分开传入，避免用同一个因子换算字号与边距造成口径漂移。
     */
    fun onViewportMeasured(widthPx: Int, heightPx: Int, spToPx: Float, dpToPx: Float) {
        if (widthPx <= 0 || heightPx <= 0 || spToPx <= 0f || dpToPx <= 0f) {
            return
        }
        val current = _ui.value
        val unchanged = current.viewportWidthPx == widthPx &&
            current.viewportHeightPx == heightPx &&
            Math.abs(current.spToPx - spToPx) < 0.001f &&
            Math.abs(current.dpToPx - dpToPx) < 0.001f
        if (unchanged) {
            return
        }
        update {
            it.copy(
                viewportWidthPx = widthPx,
                viewportHeightPx = heightPx,
                spToPx = spToPx,
                dpToPx = dpToPx,
            )
        }
        requestReflow("正文区域已更新")
    }

    // ------------------------------------------------------------------ 打开 / 关闭

    /**
     * 打开一个 TXT 来源（AN3：本地文件路径 / 私有缓存副本路径；AN4 起为 SAF 持久 URI）。
     *
     * 打开成功后才登记书架条目 —— 书架显示的是**真实回执**（书籍 id / 总字节 / 章节数），
     * 不是占位数据。
     */
    fun openBook(sourceIdentity: String, displayName: String? = null) {
        if (sourceIdentity.isEmpty()) {
            update {
                it.copy(
                    failure = ReaderFailure(NrErrors.NR_ERR_INVALID_ARGUMENT, NrErrorKind.CALLER, "来源标识为空"),
                    status = "来源标识为空",
                    shelf = it.shelf.copy(loading = false),
                )
            }
            return
        }
        update { it.copy(status = ReaderStatus.OPENING, shelf = it.shelf.copy(loading = true)) }
        launchSerial { openBookLocked(sourceIdentity, displayName) }
    }

    /**
     * 从书架打开。
     *
     * SAF 平台窗口来源（[ShelfBook.sourceKind] 为 `NR_SOURCE_KIND_PLATFORM`）**必须**经宿主重新解析：
     * 窗口会话的描述符不跨会话存活，且 `content://` URI 不能当文件路径打开
     * （真机实测会得到 `NR_ERR_6 (IO) 打开书籍（数据源读取失败）`）。
     * 其余来源（本地文件 / 私有缓存副本）仍走文件通道，行为不变。
     */
    fun openBook(book: ShelfBook) {
        val resolver = hostResolver
        if (book.sourceKind != NrAbiConstants.NR_SOURCE_KIND_PLATFORM || resolver == null) {
            openBook(book.sourceIdentity, book.displayName)
            return
        }
        update { it.copy(status = ReaderStatus.OPENING, shelf = it.shelf.copy(loading = true)) }
        launchSerial {
            val target = try {
                withContext(io) { resolver.resolve(book.sourceIdentity) }
            } catch (error: Throwable) {
                throwIfCancellation(error)
                failLocked(error, "重新打开来源")
                update { it.copy(shelf = it.shelf.copy(loading = false)) }
                return@launchSerial
            }
            openBookLocked(
                sourceIdentity = target.identity,
                displayName = target.displayName ?: book.displayName,
                kind = target.kind,
                totalBytes = target.totalBytes,
            )
        }
    }

    /**
     * 以**平台窗口**（SAF 可定位来源）打开书籍。
     *
     * [sourceIdentity] 为 SAF URI；[totalBytes] 必须来自已绑定的 `SafWindowSession.totalBytes`，
     * 即由平台实际长度给出 —— 本方法不猜测、不裁剪，坐标空间与可读范围一律由核心裁定。
     */
    fun openPlatformBook(sourceIdentity: String, totalBytes: Long, displayName: String? = null) {
        if (sourceIdentity.isEmpty() || totalBytes <= 0L) {
            update {
                it.copy(
                    failure = ReaderFailure(
                        NrErrors.NR_ERR_INVALID_ARGUMENT,
                        NrErrorKind.CALLER,
                        "平台窗口来源需要有效的来源标识与长度（identity=$sourceIdentity, totalBytes=$totalBytes）",
                    ),
                    status = "平台窗口来源参数无效",
                    shelf = it.shelf.copy(loading = false),
                )
            }
            return
        }
        update { it.copy(status = ReaderStatus.OPENING, shelf = it.shelf.copy(loading = true)) }
        launchSerial {
            openBookLocked(
                sourceIdentity = sourceIdentity,
                displayName = displayName,
                kind = NrAbiConstants.NR_SOURCE_KIND_PLATFORM,
                totalBytes = totalBytes,
            )
        }
    }

    /**
     * 宿主上报一次导入失败（SAF 被拒 / 需要重新关联 / 读不到字节）。
     *
     * 只更新可见状态与失败信息，不触碰阅读状态 —— 界面因此不会停在「加载中」。
     */
    fun reportHostFailure(detail: String) {
        update {
            it.copy(
                failure = ReaderFailure(NrErrors.NR_ERR_IO, NrErrorKind.CALLER, detail),
                status = "导入失败：$detail",
                shelf = it.shelf.copy(loading = false),
            )
        }
    }

    /** 关闭当前书籍（核心 `nr_close_book`），回到书架页。 */
    fun closeBook() {
        launchSerial {
            val bookId = _ui.value.reader.bookId
            if (bookId != null) {
                try {
                    withContext(io) { engine.closeBook(bookId) }
                } catch (error: Throwable) {
                    throwIfCancellation(error)
                    failLocked(error, "关闭书籍")
                }
            }
            generation.incrementAndGet()
            layoutId = 0L
            requestId = 0L
            ordinal = 0
            restoredAnchor = PageAnchor(0, 0L)
            laidOutSpec = null
            chapters.clear()
            forwardCache.clear()
            measuredPages.clear()
            update {
                it.copy(
                    route = ReaderRoute.SHELF,
                    reader = ReaderState(),
                    status = "已关闭书籍",
                    failure = null,
                )
            }
        }
    }

    // ------------------------------------------------------------------ 翻页 / 切章

    /** 下一页（前向缓存命中则不重新测量）。 */
    fun nextPage() {
        launchSerial { stepForwardLocked("下一页") }
    }

    /** 上一页（会话返回栈；返回栈为空即第一页）。 */
    fun previousPage() {
        launchSerial { stepBackwardLocked("上一页") }
    }

    /** 下一章（章节区间来自核心章节批次）。 */
    fun nextChapter() {
        launchSerial { stepChapterLocked(1) }
    }

    /** 上一章。 */
    fun previousChapter() {
        launchSerial { stepChapterLocked(-1) }
    }

    // ------------------------------------------------------------------ 排版设置

    /**
     * 字号（sp）：**先同步写回状态**（受控滑杆立即反映新值），再请求合并式重排。
     */
    fun onFontSizeChange(valueSp: Float) {
        val next = valueSp.coerceIn(TypographySettings.MIN_FONT_SIZE_SP, TypographySettings.MAX_FONT_SIZE_SP)
        if (Math.abs(next - _ui.value.settings.fontSizeSp) < 0.01f) {
            return
        }
        update { it.copy(settings = it.settings.copy(fontSizeSp = next), status = ReaderStatus.REFLOW) }
        requestReflow("字号 ${next.toInt()} sp")
    }

    /** 行距倍数：同步写回 + 合并式重排。 */
    fun onLineSpacingChange(multiplier: Float) {
        val next = multiplier.coerceIn(
            TypographySettings.MIN_LINE_SPACING_MULTIPLIER,
            TypographySettings.MAX_LINE_SPACING_MULTIPLIER,
        )
        if (Math.abs(next - _ui.value.settings.lineSpacingMultiplier) < 0.001f) {
            return
        }
        update { it.copy(settings = it.settings.copy(lineSpacingMultiplier = next), status = ReaderStatus.REFLOW) }
        requestReflow("行距 ${next}x")
    }

    /** 左右边距（dp）：同步写回 + 合并式重排。 */
    fun onHorizontalMarginChange(valueDp: Float) {
        val next = valueDp.coerceIn(
            TypographySettings.MIN_HORIZONTAL_MARGIN_DP,
            TypographySettings.MAX_HORIZONTAL_MARGIN_DP,
        )
        if (Math.abs(next - _ui.value.settings.horizontalMarginDp) < 0.01f) {
            return
        }
        update { it.copy(settings = it.settings.copy(horizontalMarginDp = next), status = ReaderStatus.REFLOW) }
        requestReflow("左右边距 ${next.toInt()} dp")
    }

    /** 上下边距（dp）：同步写回 + 合并式重排。 */
    fun onVerticalMarginChange(valueDp: Float) {
        val next = valueDp.coerceIn(
            TypographySettings.MIN_VERTICAL_MARGIN_DP,
            TypographySettings.MAX_VERTICAL_MARGIN_DP,
        )
        if (Math.abs(next - _ui.value.settings.verticalMarginDp) < 0.01f) {
            return
        }
        update { it.copy(settings = it.settings.copy(verticalMarginDp = next), status = ReaderStatus.REFLOW) }
        requestReflow("上下边距 ${next.toInt()} dp")
    }

    /**
     * 明亮/暗黑主题：只影响界面配色，**不参与排版签名**，因此不触发重排
     * （避免无变化的通知清空分页缓存）。
     */
    fun onDarkThemeChange(enabled: Boolean) {
        if (enabled == _ui.value.settings.darkTheme) {
            return
        }
        update {
            it.copy(
                settings = it.settings.copy(darkTheme = enabled),
                status = if (enabled) "已切换到夜间模式" else "已切换到日间模式",
            )
        }
        persistSettings()
    }

    /** 显式重新排版：先失效当前排版世代的边界，再按当前设置重建。 */
    fun requestRelayout() {
        forceInvalidate = true
        requestReflow("重新排版")
    }

    // ------------------------------------------------------------------ 保存位置 / 索引

    /** 手动保存当前阅读位置（薄委托 `nr_save_position`）。 */
    fun savePositionNow() {
        launchSerial { saveCurrentPositionLocked("手动保存") }
    }

    /** 重建章节索引（可取消的长任务入口）。 */
    fun rebuildIndex() {
        launchSerial {
            val state = _ui.value
            val bookId = state.reader.bookId
            if (bookId == null) {
                setStatus(ReaderStatus.NO_BOOK)
                return@launchSerial
            }
            rebuildRequestId.incrementAndGet()
            val request = rebuildRequestId.get()
            update {
                it.copy(reader = it.reader.copy(indexRebuilding = true), status = ReaderStatus.INDEX_REBUILDING)
            }
            val outcome = try {
                withContext(io) { engine.rebuildIndex(bookId, request) }
            } catch (error: Throwable) {
                throwIfCancellation(error)
                failLocked(error, "重建章节索引")
                null
            }
            update { it.copy(reader = it.reader.copy(indexRebuilding = false)) }
            if (outcome == null) {
                return@launchSerial
            }
            if (outcome.cancelled) {
                setStatus(ReaderStatus.INDEX_CANCELLED)
                return@launchSerial
            }
            // 索引版本变化后必须重新登记排版世代：旧世代提交的边界会被核心判为过期。
            update {
                it.copy(
                    reader = it.reader.copy(
                        contentIndexVersion = outcome.result.contentIndexVersion,
                        chapters = it.reader.chapters,
                    ),
                    status = "章节索引已重建（${outcome.result.chapters} 章 · 扫描 ${outcome.result.scannedBytes} 字节）",
                )
            }
            loadChaptersLocked(bookId)
            layoutId = 0L
            requestId = 0L
            generation.incrementAndGet()
            beginLayoutLocked()
            val page = _ui.value.reader.page
            layoutAtLocked(
                chapterIndex = page?.chapterIndex ?: restoredAnchor.chapterIndex,
                startByte = page?.startByte ?: restoredAnchor.startByte,
                ordinalForPage = page?.pageOrdinal ?: 1,
                action = "索引重建后重排",
            )
        }
    }

    /**
     * 取消进行中的索引重建。
     *
     * **不取 [gate]**：长任务运行期间必须仍然可达（头文件 §7 跨线程取消）；
     * 未知/已完成的 requestId 由核心裁定（幂等语义在核心侧）。
     *
     * 本入口**不登记**到 [inFlight]：它本身就是取消通道，宿主取消在途任务时不能把「取消请求」
     * 本身也取消掉（否则长任务会失去唯一的取消投递路径）。
     */
    fun cancelIndexRebuild() {
        val request = rebuildRequestId.get()
        if (request == 0L) {
            setStatus("没有可取消的请求")
            return
        }
        work.launch {
            try {
                withContext(io) { engine.cancelRequest(request) }
                setStatus("已请求取消索引重建 #$request")
            } catch (error: Throwable) {
                throwIfCancellation(error)
                failLocked(error, "取消请求")
            }
        }
    }

    // ------------------------------------------------------------------ 宿主生命周期（AR4）

    /**
     * 宿主动作：**旋转 / 退到后台时取消在途任务，但保留应用级引擎句柄**。
     *
     * 调用点（宿主）：`Activity.onStop`，以及配置变更（旋转）后重新测量正文区域之前 ——
     * ViewModel 在两种情况下都存活，`NrEngine` 句柄属于 ViewModel，**只**在 [onCleared] 释放。
     *
     * 语义：
     * - **即时可达**：本方法**不取** [gate]（[gate] 可能正被长任务持有），只取消协程并复位瞬时标志；
     * - **不销毁句柄**：绝不调用 `nr_engine_destroy`，返回前台 / 旋转后可继续翻页、保存位置与重排
     *   （句柄被销毁即 `NR_ERR_ENGINE_DESTROYED`，本方法不得成为这种路径）；
     * - **过期结果仍被丢弃**：取消同时推进本地 [generation] 并清掉重排脏标记，任何已经拿到测量结果
     *   的在途路径都会在发布前被判为过期，不会覆盖当前页面；
     * - **取消不是失败**：被取消的任务不得写 [ReaderFailure]（见 [throwIfCancellation]），
     *   会话状态（当前页 / 章节 / 排版世代）保留，只复位「进行中」的瞬时标志。
     *
     * @param reason 宿主原因（写进状态回显，便于真机定位）。
     * @return 实际取消的活跃任务数；`0` 表示当时没有在途任务（状态与文案均不变）。
     */
    fun cancelInFlightWork(reason: String = "宿主暂停"): Int {
        generation.incrementAndGet()
        reflowDirty = false
        val active = synchronized(inFlight) { inFlight.filter { it.isActive } }
        if (active.isEmpty()) {
            return 0
        }
        active.forEach { it.cancel() }
        update {
            it.copy(
                shelf = it.shelf.copy(loading = false),
                reader = it.reader.copy(indexRebuilding = false),
                status = ReaderStatus.hostCancelled(reason, active.size),
            )
        }
        return active.size
    }

    /**
     * 未完成（含「已被取消但尚未收尾」）的在途任务数。
     *
     * 口径刻意用**登记表大小**而不是 `count { it.isActive }`：取消只是把 Job 置为 cancelling，
     * 协程还要跑完 `catch` / `finally` 才真正结束，这段时间里它仍可能写状态。
     * 宿主与测试需要的是**真正的静止**（可以安全地重新测量 / 断言），不是「已发出取消」。
     */
    fun inFlightCount(): Int = synchronized(inFlight) { inFlight.size }

    /**
     * 宿主动作：**回到前台 / 旋转完成后重新对齐排版**。
     *
     * 与 [cancelInFlightWork] 配对：取消会丢掉在途重排，若期间改了字号或视口，界面必须重新收敛。
     *
     * - 未打开书籍 → 空操作；
     * - 已发布页面的排版参数（[laidOutSpec]）与当前设置/视口**一致** → 空操作（不触碰 native，
     *   避免无变化的通知白清一次分页缓存）；
     * - 不一致（后台期间改过设置、旋转换了视口、或重排被取消）→ 请求一次**合并式**重排。
     */
    fun onHostResumed() {
        val state = _ui.value
        if (!state.reader.isOpen || state.reader.page == null) {
            return
        }
        val spec = currentSpec(state) ?: return
        if (laidOutSpec == spec) {
            return
        }
        requestReflow("回到前台")
    }

    /**
     * 以 [gate] 串行执行一个状态变更任务，并登记到 [inFlight] 供 [cancelInFlightWork] 取消。
     *
     * **所有**会改动阅读状态的挂起入口都必须经这里：`gate` 保证同一时刻只有一条状态机路径，
     * 登记表保证宿主生命周期能一次性收拢它们（不会出现「旋转后旧任务继续重填缓存」）。
     */
    private fun launchSerial(block: suspend () -> Unit): Job {
        val job = work.launch {
            gate.withLock { block() }
        }
        synchronized(inFlight) { inFlight.add(job) }
        job.invokeOnCompletion { synchronized(inFlight) { inFlight.remove(job) } }
        return job
    }

    /**
     * 取消不是业务失败：在任何 native 调用边界上原样向上传播。
     *
     * 若被 `catch (Throwable)` 吞掉并映射成 [ReaderFailure]，界面会在「用户只是退到后台」时显示
     * 内部错误，同时破坏结构化取消（父作用域无法得知子任务已取消）。
     */
    private fun throwIfCancellation(error: Throwable) {
        if (error is CancellationException) {
            throw error
        }
    }

    // ------------------------------------------------------------------ 内部：打开流程

    /**
     * 打开书籍。
     *
     * [kind] / [totalBytes] 由宿主决定（本方法只组装参数，**不**判定坐标空间与边界）：
     * - 本地文件源：`NR_SOURCE_KIND_FILE` + 0，核心以文件实际长度为准；
     * - SAF 可定位来源：`NR_SOURCE_KIND_PLATFORM` + 已绑定窗口会话的长度。
     */
    private suspend fun openBookLocked(
        sourceIdentity: String,
        displayName: String?,
        kind: Int = NrAbiConstants.NR_SOURCE_KIND_FILE,
        totalBytes: Long = 0L,
    ) {
        val request = NrOpenRequest(
            kind = kind,
            // 本地文件源传 0：核心以文件实际长度为准（头文件 nr_open_request.total_bytes 注释）。
            totalBytes = totalBytes,
            identity = sourceIdentity.toByteArray(Charsets.UTF_8),
            displayName = displayName?.toByteArray(Charsets.UTF_8),
        )
        val opened = try {
            withContext(io) { engine.openTxt(request) }
        } catch (error: Throwable) {
            throwIfCancellation(error)
            failLocked(error, "打开书籍")
            update { it.copy(shelf = it.shelf.copy(loading = false)) }
            return
        }
        // 核心判定的编码必须在建立章节/分页之前生效（窗口解码与字节计数都依赖它）。
        val openedEncoding = ContentEncoding.ofAbiCode(opened.encoding)
        contentEncoding = openedEncoding
        generation.incrementAndGet()
        layoutId = 0L
        requestId = 0L
        ordinal = 0
        chapters.clear()
        forwardCache.clear()
        restoredAnchor = PageAnchor(0, 0L)
        laidOutSpec = null
        currentBookKey = sourceIdentity
        sparseAnchors.clear()
        loadSparseAnchors(sparseAnchors)
        val label = displayName ?: sourceIdentity
        shelf[sourceIdentity] = ShelfBook(
            sourceIdentity = sourceIdentity,
            displayName = label,
            bookId = opened.bookId,
            totalBytes = opened.totalBytes,
            chapterCount = opened.chapters.toInt(),
            sourceKind = kind,
        )
        bookCatalog?.rememberBookId(sourceIdentity, opened.bookId)
        update {
            it.copy(
                route = ReaderRoute.READER,
                shelf = it.shelf.copy(
                    books = shelf.values.toList(),
                    selectedBookId = opened.bookId,
                    loading = false,
                ),
                reader = ReaderState(
                    bookId = opened.bookId,
                    displayName = label,
                    totalBytes = opened.totalBytes,
                    contentIndexVersion = opened.contentIndexVersion,
                    // 编码来自核心回执：界面据此显示「当前使用的编码」。
                    encodingName = openedEncoding.displayName,
                    encodingConfidence = opened.encodingConfidence.toInt(),
                    needsEncodingConfirmation = opened.needsEncodingConfirmation != 0L,
                ),
                status = "已打开 $label（${opened.totalBytes} 字节 · ${opened.chapters} 章 · " +
                    "编码 ${openedEncoding.displayName}（可信度 ${opened.encodingConfidence}‰）· 索引版本 ${opened.contentIndexVersion}）",
                failure = null,
            )
        }
        loadChaptersLocked(opened.bookId)
        restorePositionLocked(opened.bookId)
    }

    /** 读取章节批次（分批，单批 ≤ [NrAbiConstants.NR_MAX_CHAPTER_BATCH]，绝不一次取整库）。 */
    private suspend fun loadChaptersLocked(bookId: Long) {
        val collected = ArrayList<ChapterEntry>()
        var sequence = 0
        var batches = 0
        while (batches < MAX_CHAPTER_BATCHES) {
            batches += 1
            val batch = try {
                withContext(io) {
                    engine.getChapters(bookId, sequence, NrAbiConstants.NR_MAX_CHAPTER_BATCH)
                }
            } catch (error: Throwable) {
                throwIfCancellation(error)
                failLocked(error, "读取章节列表")
                return
            }
            try {
                batch.entries.forEach { entry ->
                    collected.add(
                        ChapterEntry(
                            sequence = entry.sequence.toInt(),
                            // ⚠️ 章节标题**一律 UTF-8**（头文件 §3「字符串：一律 UTF-8 指针 + 长度」）。
                            // 曾误按「内容编码」解码该字节串 → 目录标题乱码（真机实测）。
                            title = String(NrChapterCodec.title(batch.bytes, entry), Charsets.UTF_8),
                            startByte = entry.startByte,
                            endByte = entry.endByte,
                        ),
                    )
                }
            } catch (error: Throwable) {
                failLocked(error, "解码章节批次")
                return
            } finally {
                batch.buffer.release(releaser)
            }
            val total = batch.batch.totalChapters
            if (collected.isEmpty() || collected.size.toLong() >= total) {
                break
            }
            if (batch.batch.count < NrAbiConstants.NR_MAX_CHAPTER_BATCH) {
                break
            }
            sequence = collected.last().sequence + 1
        }
        chapters.clear()
        chapters.addAll(collected)
        update { it.copy(reader = it.reader.copy(chapters = collected.toList())) }
    }

    /** 恢复阅读位置（核心 `nr_get_position`）；失败不静默：如实回报并从章节开头排版。 */
    private suspend fun restorePositionLocked(bookId: Long) {
        val restored = try {
            withContext(io) { engine.getPosition(bookId) }
        } catch (error: Throwable) {
            throwIfCancellation(error)
            failLocked(error, "读取阅读进度")
            null
        }
        val anchor = if (restored == null) {
            PageAnchor(0, 0L)
        } else {
            val chapterIndex = restored.position.chapterIndex.toInt().coerceAtLeast(0)
            val offset = restored.position.originalByteOffset
            restored.buffer.release(releaser)
            if (restored.position.found != 0L && offset > 0L) {
                update { it.copy(reader = it.reader.copy(lastSavedByteOffset = offset)) }
                PageAnchor(chapterIndex, offset)
            } else {
                PageAnchor(0, 0L)
            }
        }
        restoredAnchor = anchor
        seedSparseAnchorsFromStore(anchor)
        if (!beginLayoutLocked()) {
            return
        }
        layoutAtLocked(anchor.chapterIndex, anchor.startByte, ordinalForPage = 1, action = "已打开")
    }

    // ------------------------------------------------------------------ 内部：排版与翻页

    /** 当前设置与视口换算出的像素排版参数；视口未就绪返回 `null`（零视口是非法签名）。 */
    private fun currentSpec(state: ReaderUiState = _ui.value): TextLayoutSpec? {
        if (!state.hasViewport) {
            return null
        }
        if (!TextLayoutContract.isValidScale(state.spToPx) || !TextLayoutContract.isValidScale(state.dpToPx)) {
            // 换算因子非法（零/负/非有限）时不得建立排版世代：否则字号会退化成 0 像素。
            return null
        }
        val settings = state.settings
        val spec = TextLayoutSpec(
            viewportWidthPx = state.viewportWidthPx,
            viewportHeightPx = state.viewportHeightPx,
            // 字号与边距的像素换算只经 [TextLayoutContract]（与渲染端同一口径，见 AR3）。
            fontSizePx = TextLayoutContract.fontSizePx(settings.fontSizeSp, state.spToPx),
            lineSpacingMultiplier = settings.lineSpacingMultiplier,
            horizontalMarginPx = TextLayoutContract.marginPx(settings.horizontalMarginDp, state.dpToPx),
            verticalMarginPx = TextLayoutContract.marginPx(settings.verticalMarginDp, state.dpToPx),
            fontFamily = settings.fontFamily,
        )
        return spec.takeIf { it.isValid }
    }

    /**
     * 排版签名 13 项（顺序与 `nr_layout_signature` / `android/jni/README.md` §4 逐项一致）。
     *
     * 视口字段填写**正文内容区**（已减去边距），因此边距字段为 0：与桌面端
     * 「留白由正文区域表达、签名内不重复计入边距」同一口径。
     * 段距 / 首行缩进固定 0（两侧渲染能力未对齐前不参与排版，见 [TypographySettings]）。
     * `platform_font_fingerprint` 传 0 = 尚未取得度量指纹（同一设备同一参数稳定）。
     */
    private fun layoutFields(spec: TextLayoutSpec): LongArray = longArrayOf(
        spec.contentWidthPx.toLong(),
        Math.round(spec.contentHeightPx).toLong(),
        DIRECTION_HORIZONTAL,
        spec.fontWeight.toLong(),
        spec.fontSizeMilli,
        spec.extraLeadingMilli,
        0L,
        0L,
        0L,
        0L,
        NrAbiConstants.NR_LAYOUT_ALGORITHM_VERSION.toLong(),
        0L,
        0L,
    )

    /** 登记排版签名（`BEGIN_LAYOUT`），成功后持有本次世代的 `layout_id` / `request_id`。 */
    private suspend fun beginLayoutLocked(): Boolean {
        val state = _ui.value
        if (!state.reader.isOpen) {
            return false
        }
        val spec = currentSpec(state)
        if (spec == null) {
            setStatus("${ReaderStatus.LAYOUT}（等待正文区域尺寸）")
            return false
        }
        val submission = NrPageBoundarySubmission(
            phase = NrAbiConstants.NR_SUBMIT_BEGIN_LAYOUT,
            contentIndexVersion = state.reader.contentIndexVersion,
            layoutId = 0L,
            requestId = 0L,
            totalBytes = state.reader.totalBytes,
            startByte = 0L,
            endByte = 0L,
            layoutFields = layoutFields(spec),
            fontFamily = state.settings.fontFamily.toByteArray(Charsets.UTF_8),
        )
        val result = try {
            withContext(io) { engine.submitPageBoundary(submission) }
        } catch (error: Throwable) {
            throwIfCancellation(error)
            failLocked(error, "登记排版签名")
            return false
        }
        if (result.status != NrAbiConstants.NR_BOUNDARY_LAYOUT_READY) {
            update {
                it.copy(
                    failure = ReaderFailure(result.errorCode, NrErrorKind.of(result.errorCode), "登记排版签名"),
                    status = "登记排版签名未就绪（status=${result.status}）",
                )
            }
            return false
        }
        if (result.layoutId != layoutId) {
            // 换代（或首次登记）：上一世代的测量结果整体作废
            measuredPages.clear()
        }
        layoutId = result.layoutId
        requestId = result.requestId
        update { it.copy(reader = it.reader.copy(layoutId = layoutId, requestId = requestId)) }
        return true
    }

    /**
     * 在 [chapterIndex]/[startByte] 处测量一页并提交给核心（`COMMIT`）。
     *
     * 只返回结果，不做发布 —— 前向缓存预取复用同一条路径，保证「预取」与「翻页」走的是
     * **同一套口径**（同一签名、同一测量器、同一提交入口）。
     */
    private suspend fun measurePageLocked(
        chapterIndex: Int,
        startByte: Long,
        spec: TextLayoutSpec,
        ordinalForPage: Int,
    ): MeasuredPage {
        val state = _ui.value
        // 窗口长度按**章节区间**裁剪：核心只校验偏移属于该章，不会把窗口裁到章节末尾，
        // 若不裁就会把下一章的开头排进本页（章节间"越界显示"）。
        val chapterEnd = chapters.getOrNull(chapterIndex)?.endByte
        val windowLength = chapterEnd
            ?.let { (it - startByte).coerceAtMost(NrAbiConstants.NR_DEFAULT_WINDOW_BYTES) }
            ?: NrAbiConstants.NR_DEFAULT_WINDOW_BYTES
        if (windowLength <= 0L) {
            return MeasuredPage(MeasureOutcome.AT_END)
        }
        val read = try {
            withContext(io) {
                engine.readWindow(chapterIndex, startByte, windowLength)
            }
        } catch (error: Throwable) {
            throwIfCancellation(error)
            failLocked(error, "读取正文窗口")
            return MeasuredPage(MeasureOutcome.FAILED)
        }
        // 按核心判定的编码解码，并**记录每个字符的源字节终点**（页边界的唯一依据）。
        val decodedRaw = EncodedTextCodec.decode(read.bytes, contentEncoding.charset)
        // 核心的窗口对齐是 UTF-8 专属：GB18030/GBK/Big5 文档的窗口首字节可能落在字符中间，
        // 解码即得替换字符（真机「首行乱码」）。跳过它之后才是**真正的字符边界**——
        // 否则提交的页起点落在字符内部，会被核心拒绝 ⇒ 后退扫描中止 ⇒「无法上一页」。
        val skipped = decodedRaw.leadingReplacementCount()
        val decoded = decodedRaw.afterSkipping(skipped)
        val text = decoded.text
        // 窗口字节起点 = 请求偏移 + 因字符边界裁掉的头部字节数（头文件 nr_window）
        //              + 平台为避免落在字符内部而额外跳过的字节数。
        val dataStart = read.window.offset + read.window.droppedLeading +
            decodedRaw.byteLengthOfPrefix(skipped)
        read.buffer.release(releaser)
        if (text.isEmpty()) {
            return MeasuredPage(MeasureOutcome.AT_END)
        }
        val measured = measurer.measure(text, spec)
        val slice = PageLayout.slice(
            windowStartByte = dataStart,
            text = text,
            measured = measured,
            lineCount = PageLayout.fittingLineCount(measured, spec),
            prefixByteEnds = decoded.prefixByteEnds,
        )
        val boundary = try {
            withContext(io) {
                engine.submitPageBoundary(
                    NrPageBoundarySubmission(
                        phase = NrAbiConstants.NR_SUBMIT_COMMIT,
                        contentIndexVersion = state.reader.contentIndexVersion,
                        layoutId = layoutId,
                        requestId = requestId,
                        totalBytes = state.reader.totalBytes,
                        startByte = slice.startByte,
                        endByte = slice.endByte,
                    ),
                )
            }
        } catch (error: Throwable) {
            throwIfCancellation(error)
            failLocked(error, "提交页边界")
            return MeasuredPage(MeasureOutcome.FAILED)
        }
        if (isStale(boundary.status)) {
            // 过期结果：交给调用方按核心给出的当前世代重试，绝不写入页面状态。
            return MeasuredPage(MeasureOutcome.STALE)
        }
        if (boundary.status == NrAbiConstants.NR_BOUNDARY_REJECTED) {
            update {
                it.copy(
                    failure = ReaderFailure(
                        boundary.errorCode,
                        NrErrorKind.of(boundary.errorCode),
                        "页边界被拒",
                    ),
                    status = "页边界被拒（rejection=${boundary.rejection}）",
                )
            }
            return MeasuredPage(MeasureOutcome.FAILED)
        }
        return MeasuredPage(
            outcome = MeasureOutcome.READY,
            page = PageState(
                chapterIndex = chapterIndex,
                chapterTitle = chapterTitle(chapterIndex),
                text = slice.text,
                startByte = slice.startByte,
                endByte = slice.endByte,
                lineCount = slice.lineCount,
                lineHeightPx = measured.lineHeightPx,
                fontSizePx = spec.fontSizePx,
                pageOrdinal = ordinalForPage,
                registeredPageCount = boundary.pageCount,
                reachedContentEnd = slice.endByte >= state.reader.totalBytes,
            ),
        )
    }

    /**
     * 测量并**发布**一页（首次打开、恢复位置、翻页、切章、重排都走这里）。
     *
     * 过期结果按核心给出的当前世代重新登记并重试（[MAX_STALE_RETRY] 次），
     * 期间**不改变**当前页面。
     */
    private suspend fun layoutAtLocked(
        chapterIndex: Int,
        startByte: Long,
        ordinalForPage: Int,
        action: String,
        attempt: Int = 0,
    ): Boolean {
        val state = _ui.value
        if (!state.reader.isOpen) {
            return false
        }
        val spec = currentSpec(state)
        if (spec == null) {
            setStatus("${ReaderStatus.LAYOUT}（等待正文区域尺寸）")
            return false
        }
        if (layoutId == 0L && !beginLayoutLocked()) {
            return false
        }
        val myGeneration = generation.get()
        val measured = measurePageLocked(chapterIndex, startByte, spec, ordinalForPage)
        if (generation.get() != myGeneration) {
            // 本地世代已推进（设置/视口在测量期间变化）：丢弃本次结果。
            return false
        }
        when (measured.outcome) {
            MeasureOutcome.READY -> {
                val page = measured.page ?: return false
                // 先落进度、再发布页面：persist 的状态行必须让位给本次动作
                // （见 persistPositionLocked 的调用顺序约定）。
                persistPositionLocked(page)
                // 记录**本次实际使用**的排版参数（不是发布时刻的当前值）：设置若在测量期间变化，
                // 这里必须留下旧参数，宿主回到前台时才会重新对齐。
                laidOutSpec = spec
                publishPageLocked(page, action)
                prefetchLocked()
                return true
            }
            MeasureOutcome.AT_END -> {
                setStatus(ReaderStatus.CONTENT_END)
                return false
            }
            MeasureOutcome.STALE -> {
                if (attempt >= MAX_STALE_RETRY) {
                    update {
                        it.copy(
                            failure = ReaderFailure(NrErrors.NR_ERR_LAYOUT_NOT_FOUND, NrErrorKind.LAYOUT, "页边界世代过期"),
                            status = "页边界世代过期（已重试 $attempt 次）",
                        )
                    }
                    return false
                }
                // 重新登记排版世代（拿回核心当前的 layout_id/request_id）后重试。
                layoutId = 0L
                if (!beginLayoutLocked()) {
                    return false
                }
                return layoutAtLocked(chapterIndex, startByte, ordinalForPage, action, attempt + 1)
            }
            MeasureOutcome.FAILED -> return false
        }
    }

    private suspend fun stepForwardLocked(action: String) {
        val state = _ui.value
        if (!state.reader.isOpen) {
            setStatus(ReaderStatus.NO_BOOK)
            return
        }
        val page = state.reader.page
        if (page == null) {
            layoutAtLocked(restoredAnchor.chapterIndex, restoredAnchor.startByte, ordinalForPage = 1, action = action)
            return
        }
        if (page.endByte >= state.reader.totalBytes && page.reachedContentEnd) {
            setStatus(ReaderStatus.CONTENT_END)
            return
        }
        rememberSparseAnchor(PageAnchor(page.chapterIndex, page.startByte))
        val cached = forwardCache.removeFirstOrNull()
        if (cached != null) {
            val next = cached.copy(pageOrdinal = ordinal + 1)
            // 先落进度（见 persistPositionLocked 顺序约定），再发布页面。
            persistPositionLocked(next)
            // 命中预取缓存也是一次「发布」：稀疏锚点必须同步记录，否则重开后后退落点会缺页。
            rememberSparseAnchor(PageAnchor(next.chapterIndex, next.startByte))
            update {
                it.copy(
                    reader = it.reader.copy(
                        page = next,
                        canGoPreviousPage = true,
                        forwardCacheCount = forwardCache.size,
                    ),
                    status = pageStatus(next, action),
                    failure = null,
                )
            }
            ordinal += 1
            prefetchLocked()
            return
        }
        val anchor = nextAnchor(page)
        if (anchor == null) {
            setStatus(ReaderStatus.CONTENT_END)
            return
        }
        if (!layoutAtLocked(anchor.chapterIndex, anchor.startByte, ordinal + 1, action)) {
            // 失败不动页面状态（「上一页」改为纯位置判据，无需回滚任何历史栈）。
            update { it.copy(reader = it.reader.copy(canGoPreviousPage = previousPageAvailable(page))) }
        }
    }

    /**
     * 载入宿主持久化的排版设置（真机缺陷：退出应用后字号/夜间模式被重置）。
     *
     * 只覆盖排版与外观；阅读进度由核心负责，不在此处重复存储。
     */
    private fun loadPersistedSettings() {
        val snapshot = settingsStore?.load() ?: return
        update {
            it.copy(
                settings = it.settings.copy(
                    fontSizeSp = snapshot.fontSizeSp,
                    lineSpacingMultiplier = snapshot.lineSpacingMultiplier,
                    horizontalMarginDp = snapshot.horizontalMarginDp,
                    verticalMarginDp = snapshot.verticalMarginDp,
                    darkTheme = snapshot.darkTheme,
                ),
            )
        }
    }

    /** 持久化当前排版设置（由设置变更的唯一出口调用；失败不影响阅读主链路）。 */
    private fun persistSettings() {
        val store = settingsStore ?: return
        val settings = _ui.value.settings
        store.save(
            ReaderSettingsSnapshot(
                fontSizeSp = settings.fontSizeSp,
                lineSpacingMultiplier = settings.lineSpacingMultiplier,
                horizontalMarginDp = settings.horizontalMarginDp,
                verticalMarginDp = settings.verticalMarginDp,
                darkTheme = settings.darkTheme,
            ),
        )
    }

    /**
     * 从宿主目录重建书架（真机缺陷：退出应用后书架被清空）。
     *
     * 已存在的条目**保留**其运行期字段（章节数、进度），避免用占位值覆盖打开时拿到的真实回执。
     * 目录不可用时静默跳过 —— 书架为空也不得影响阅读主链路。
     */
    private fun refreshShelfFromCatalog() {
        val catalog = bookCatalog ?: return
        val entries = try {
            catalog.list()
        } catch (error: Throwable) {
            return
        }
        var added = false
        for (entry in entries) {
            if (shelf.containsKey(entry.sourceIdentity)) continue
            // 已移除的书不得由目录重建再次出现（否则「移除」形同虚设）。
            if (entry.sourceIdentity in hiddenBooksSet) continue
            shelf[entry.sourceIdentity] = ShelfBook(
                sourceIdentity = entry.sourceIdentity,
                displayName = entry.displayName,
                bookId = 0L,
                totalBytes = 0L,
                chapterCount = 0,
                sourceKind = entry.sourceKind,
            )
            added = true
        }
        if (added) {
            update { it.copy(shelf = it.shelf.copy(books = shelf.values.toList())) }
        }
    }

    /**
     * 手动指定内容编码（规格 CR2「用户手动覆盖」；由核心裁定并落库）。
     *
     * `encodingCode` 取 `NR_ENCODING_*`（1..=7）；**`0` = 清除覆盖**、回到自动检测。
     * 核心会按新编码立即重扫章节索引并递增内容索引版本 ⇒ 旧章节缓存与分页边界全部失效；
     * 这里随后重新载入章节，并把阅读位置**按字节偏移**保持
     * （编码变了，同一字节的含义会变，只能按字节续读；偏移越界时退回到该章开头）。
     * 核心只改变解释方式，**永不写回原文件**。
     */
    fun setEncoding(encodingCode: Int) {
        val state = _ui.value
        if (!state.reader.isOpen) {
            setStatus(ReaderStatus.NO_BOOK)
            return
        }
        // 未打开（或尚未拿到核心 book_id）时不做任何事：不猜测、不误改。
        val bookId = state.reader.bookId ?: run {
            setStatus(ReaderStatus.NO_BOOK)
            return
        }
        val previous = state.reader.page
        launchSerial {
            val outcome = try {
                withContext(io) { engine.setEncoding(bookId, encodingCode) }
            } catch (error: Throwable) {
                throwIfCancellation(error)
                failLocked(error, "设置编码")
                return@launchSerial
            }
            contentEncoding = ContentEncoding.ofAbiCode(outcome.encodingCode.toLong())
            update {
                it.copy(
                    reader = it.reader.copy(
                        encodingName = contentEncoding.displayName,
                        encodingConfidence = 1000,
                        needsEncodingConfirmation = false,
                    ),
                    status = "编码已设为 ${contentEncoding.displayName}" +
                        "（已按新编码重扫章节；原文件未改动）",
                )
            }
            chapters.clear()
            loadChaptersLocked(bookId)
            // 章节区间随编码变化，旧的排版世代与预取缓存必须作废。
            layoutId = 0L
            forwardCache.clear()
            val targetChapter = previous?.chapterIndex ?: 0
            val chapter = chapters.getOrNull(targetChapter)
            val requested = previous?.startByte ?: 0L
            // 章节区间随编码变化：旧偏移若越界（或整章消失）就退回到该章开头。
            val start = when {
                chapter == null -> 0L
                requested < chapter.startByte || requested >= chapter.endByte -> chapter.startByte
                else -> requested
            }
            layoutAtLocked(
                chapterIndex = targetChapter,
                startByte = start,
                ordinalForPage = 1,
                action = "编码 ${contentEncoding.displayName}",
            )
        }
    }

    /**
     * 从书架移除一本书（用户要求：避免为刷新缓存而卸载应用）。
     *
     * 语义（用户裁定）：**仅移除书架 + 清空本书本地索引（保留阅读位置），绝不删除原文件**。
     * 副作用只限应用自身状态：书架映射 + 持久化的「已移除来源」集合；
     * 不触碰数据源、不改动核心书库（核心索引按扫描器版本自动重扫）。
     */
    fun removeBook(book: ShelfBook) {
        hiddenBooksSet.add(book.sourceIdentity)
        persistHiddenBooksLocked()
        shelf.remove(book.sourceIdentity)
        val selected = _ui.value.shelf.selectedBookId?.takeIf { it != book.bookId }
        update {
            it.copy(
                shelf = it.shelf.copy(
                    books = shelf.values.toList(),
                    selectedBookId = selected,
                ),
                status = "已移除《${book.displayName}》（原文件未删除；重新导入即恢复）",
            )
        }
    }

    /**
     * 把来源从「已移除」集合取出（用户**重新导入**同一文件时调用）。
     *
     * 与 [removeBook] 配对：移除 → 重新导入 → 恢复显示。
     */
    fun restoreToShelf(sourceIdentity: String) {
        if (!hiddenBooksSet.remove(sourceIdentity)) return
        persistHiddenBooksLocked()
        refreshShelfFromCatalog()
    }

    /** 持久化「已移除」集合（失败不影响主链路：端口不抛异常）。 */
    private fun persistHiddenBooksLocked() {
        try {
            hiddenBooks?.save(hiddenBooksSet.toSet())
        } catch (_: Throwable) {
            // 忽略：移除在当前会话内仍然生效。
        }
    }

    /**
     * 记录稀疏锚点并持久化（有界、去重）。
     *
     * 两个用途：① 重开书籍恢复位置后「上一页」有确定的扫描起点；
     * ② 跨章后退时以「最近读过的页」为起点，使扫描步数最少。
     * 持久化失败不影响阅读主链路（端口不抛异常）。
     */
    private fun rememberSparseAnchor(anchor: PageAnchor) {
        val store = anchorStore ?: return
        val key = currentBookKey ?: return
        val spec = currentSpec() ?: return
        if (sparseAnchors.lastOrNull() != anchor) {
            sparseAnchors.addLast(anchor)
            while (sparseAnchors.size > MAX_SPARSE_ANCHORS) sparseAnchors.removeFirst()
        }
        store.save(key, anchorSignature(spec), sparseAnchors.toList())
    }

    /**
     * 打开书籍时载入宿主持久化的稀疏锚点（签名不匹配则为空）。
     *
     * **不得**在打开时就覆盖持久化记录：否则「恢复位置 → 未翻页即关闭」会把此前记录的锚点抹掉。
     */
    private fun loadSparseAnchors(into: ArrayDeque<PageAnchor>) {
        val store = anchorStore ?: return
        val key = currentBookKey ?: return
        val spec = currentSpec() ?: return
        val stored = store.load(key, anchorSignature(spec))
            .sortedWith(compareBy<PageAnchor>({ it.chapterIndex }, { it.startByte }))
        if (stored.isEmpty()) return
        into.clear()
        for (anchor in stored) {
            into.addLast(anchor)
            while (into.size > MAX_SPARSE_ANCHORS) into.removeFirst()
        }
    }

    /**
     * 稀疏锚点的排版签名：只有影响分页的参数一致时，旧页锚点才是可信的扫描起点。
     *
     * 取签名 13 项中的关键派生量（内容区宽高 + 字号 + 行距），不落完整 `layoutFields`。
     */
    private fun anchorSignature(spec: TextLayoutSpec): String =
        "${spec.contentWidthPx}x${Math.round(spec.contentHeightPx)}@${spec.fontSizeMilli}/${spec.extraLeadingMilli}"

    /** 恢复位置时把宿主持久化的锚点并入候选集（只取严格早于恢复点的锚点）。 */
    private fun seedSparseAnchorsFromStore(restored: PageAnchor) {
        val store = anchorStore ?: return
        val key = currentBookKey ?: return
        val spec = currentSpec() ?: return
        val stored = store.load(key, anchorSignature(spec))
            .filter {
                it.chapterIndex < restored.chapterIndex ||
                    (it.chapterIndex == restored.chapterIndex && it.startByte < restored.startByte)
            }
            .sortedWith(compareBy<PageAnchor>({ it.chapterIndex }, { it.startByte }))
        if (stored.isEmpty()) return
        for (anchor in stored) {
            if (anchor !in sparseAnchors) sparseAnchors.addLast(anchor)
        }
        while (sparseAnchors.size > MAX_SPARSE_ANCHORS) sparseAnchors.removeFirst()
    }

    /**
     * 「上一页」是否可用：**纯位置判据**（与访问历史、与是否切过章无关）。
     *
     * 只要当前页不是全书第一页，就必然存在「紧邻前一页」，按钮即应可用。
     */
    private fun previousPageAvailable(page: PageState?): Boolean {
        if (page == null) return false
        return page.startByte > 0L || page.chapterIndex > 0
    }

    /**
     * 求**紧邻当前页之前**的那一页（纯位置语义：与正常翻页同一套逻辑，不回到「离开处」）。
     *
     * 从「最近的已知较早锚点」起向前逐页测量，直到越过当前页起点，返回途中最后一页：
     * - 逐页后退：起点即上一页，通常只多测 1 页；
     * - **跨章后退**：会一路量到**上一章最后一页**（用户明确要求）；
     * - 扫描有上限（[MAX_BACKWARD_SCAN]）；达到上限或中途失败返回 `null`，绝不虚构边界。
     */
    private suspend fun previousPageState(page: PageState, spec: TextLayoutSpec): PageState? {
        if (page.startByte <= 0L && page.chapterIndex == 0) return null
        val targetOrdinal = (page.pageOrdinal - 1).coerceAtLeast(1)
        var cursor = scanStartAnchor(page) ?: return null
        var last: PageState? = null
        var steps = 0
        while (steps < MAX_BACKWARD_SCAN) {
            steps += 1
            val reachedCurrent = cursor.chapterIndex > page.chapterIndex ||
                (cursor.chapterIndex == page.chapterIndex && cursor.startByte >= page.startByte)
            if (reachedCurrent) return last
            val measured = measurePageLocked(cursor.chapterIndex, cursor.startByte, spec, targetOrdinal)
            val measuredPage = measured.page
            if (measured.outcome != MeasureOutcome.READY || measuredPage == null) return null
            last = measuredPage
            cursor = nextAnchor(measuredPage) ?: return last
        }
        return last
    }

    /**
     * 后退扫描的起点：严格早于当前页的**最近**已知锚点。
     *
     * 候选取最大者：已读过的稀疏锚点、本章开头、上一章开头。
     * 以「已读过的页」优先，可让跨章后退的扫描步数最少。
     */
    private fun scanStartAnchor(page: PageState): PageAnchor? {
        val visited = sparseAnchors
            .filter {
                it.chapterIndex < page.chapterIndex ||
                    (it.chapterIndex == page.chapterIndex && it.startByte < page.startByte)
            }
            .maxWithOrNull(compareBy<PageAnchor>({ it.chapterIndex }, { it.startByte }))
        val chapterStart = chapters.getOrNull(page.chapterIndex)
            ?.takeIf { it.startByte < page.startByte }
            ?.let { PageAnchor(page.chapterIndex, it.startByte) }
        val previousChapterStart = if (page.chapterIndex > 0) {
            chapters.getOrNull(page.chapterIndex - 1)?.let { PageAnchor(page.chapterIndex - 1, it.startByte) }
        } else {
            null
        }
        return listOfNotNull(visited, chapterStart, previousChapterStart)
            .maxWithOrNull(compareBy<PageAnchor>({ it.chapterIndex }, { it.startByte }))
    }


    private suspend fun stepBackwardLocked(action: String) {
        val state = _ui.value
        val page = state.reader.page
        if (!state.reader.isOpen || page == null) {
            setStatus(ReaderStatus.NO_BOOK)
            return
        }
        val spec = currentSpec(state)
        if (spec == null) {
            setStatus("${ReaderStatus.LAYOUT}（等待正文区域尺寸）")
            return
        }
        // 纯位置语义：从当前位置向前推一页（跨章即上一章最后一页），与历史无关。
        val restoredPage = previousPageState(page, spec)
        if (restoredPage == null) {
            if (page.startByte <= 0L && page.chapterIndex == 0) {
                setStatus("已是第一页")
                update { it.copy(reader = it.reader.copy(canGoPreviousPage = false)) }
            } else {
                setStatus("上一页不可用（未取到紧邻的前一页）")
                update { it.copy(reader = it.reader.copy(canGoPreviousPage = previousPageAvailable(page))) }
            }
            return
        }
        ordinal = restoredPage.pageOrdinal
        // 先落进度（见 persistPositionLocked 顺序约定），再发布页面。
        persistPositionLocked(restoredPage)
        laidOutSpec = spec
        forwardCache.clear()
        update {
            it.copy(
                reader = it.reader.copy(
                    page = restoredPage,
                    canGoPreviousPage = previousPageAvailable(restoredPage),
                    forwardCacheCount = 0,
                ),
                status = pageStatus(restoredPage, action),
                failure = null,
            )
        }
        prefetchLocked()
    }

    private suspend fun stepChapterLocked(delta: Int) {
        val state = _ui.value
        val page = state.reader.page
        if (!state.reader.isOpen || page == null) {
            setStatus(ReaderStatus.NO_BOOK)
            return
        }
        val target = page.chapterIndex + delta
        val chapter = state.reader.chapters.getOrNull(target)
        if (chapter == null) {
            setStatus(if (delta > 0) "已是最后一章" else "已是第一章")
            return
        }
        layoutChapterLocked(target, page, if (delta > 0) "下一章" else "上一章")
    }

    /** 跳转到指定章节的**首页**（章节目录用；越界或未打开书籍时不改变阅读状态）。 */
    fun jumpToChapter(chapterIndex: Int) {
        val state = _ui.value
        if (!state.reader.isOpen) {
            setStatus(ReaderStatus.NO_BOOK)
            return
        }
        if (chapterIndex < 0 || chapterIndex >= state.reader.chapters.size) {
            setStatus("章节 ${chapterIndex + 1} 不存在（共 ${state.reader.chapters.size} 章）")
            return
        }
        launchSerial { layoutChapterLocked(chapterIndex, _ui.value.reader.page, "跳转章节") }
    }

    /**
     * 切到目标章节的**首页**（切章与目录跳转共用）。
     *
     * 只记录当前页锚点作为后退扫描起点；「上一页」为纯位置语义，与从何处跳来无关。
     */
    private suspend fun layoutChapterLocked(targetIndex: Int, currentPage: PageState?, action: String) {
        val chapter = _ui.value.reader.chapters.getOrNull(targetIndex) ?: return
        if (currentPage != null) {
            rememberSparseAnchor(PageAnchor(currentPage.chapterIndex, currentPage.startByte))
        }
        forwardCache.clear()
        layoutAtLocked(
            chapterIndex = targetIndex,
            startByte = chapter.startByte,
            ordinalForPage = 1,
            action = action,
        )
    }

    /** 下一页的锚点：越过章节末尾时切到下一章开头（章节区间来自核心）。 */
    private fun nextAnchor(page: PageState): PageAnchor? {
        val state = _ui.value
        if (page.endByte >= state.reader.totalBytes) {
            return null
        }
        val chapter = state.reader.chapters.getOrNull(page.chapterIndex)
        if (chapter != null && page.endByte >= chapter.endByte) {
            val next = state.reader.chapters.getOrNull(page.chapterIndex + 1) ?: return null
            return PageAnchor(next.sequence, next.startByte)
        }
        return PageAnchor(page.chapterIndex, page.endByte)
    }

    /**
     * 前向缓存预取（[MAX_FORWARD_CACHE] 页）。
     *
     * **只在 [gate] 内**顺序执行：并发预取会让同一个阅读状态被多个挂起入口重填
     * （真机原型实测出现过「预读 9 页」），本卡用锁保证最多只有一条预取路径。
     */
    private suspend fun prefetchLocked() {
        val state = _ui.value
        val spec = currentSpec(state)
        val current = state.reader.page
        if (spec == null || current == null) {
            return
        }
        forwardCache.clear()
        var anchor = nextAnchor(current)
        while (anchor != null && forwardCache.size < MAX_FORWARD_CACHE) {
            val page = pageAtLocked(anchor, spec) ?: break
            forwardCache.addLast(page)
            anchor = nextAnchor(page)
        }
        update { it.copy(reader = it.reader.copy(forwardCacheCount = forwardCache.size)) }
    }

    /**
     * 取某锚点处的页：优先复用本世代已测量的结果，否则测量并提交给核心。
     *
     * 复用是**必需**的：同一页在「预取」与「翻页」两条路径上各测一次会让同一 (起点,终点)
     * 被提交两次（核心会判为幂等重复），也白读一次窗口。
     */
    private suspend fun pageAtLocked(anchor: PageAnchor, spec: TextLayoutSpec): PageState? {
        measuredPages[anchor.startByte]?.let { return it }
        val measured = measurePageLocked(anchor.chapterIndex, anchor.startByte, spec, PAGE_ORDINAL_UNSET)
        val page = measured.page ?: return null
        measuredPages[anchor.startByte] = page
        return page
    }

    private fun publishPageLocked(page: PageState, action: String) {
        ordinal = page.pageOrdinal
        measuredPages[page.startByte] = page
        rememberSparseAnchor(PageAnchor(page.chapterIndex, page.startByte))
        forwardCache.clear()
        update {
            it.copy(
                reader = it.reader.copy(
                    page = page,
                    canGoPreviousPage = previousPageAvailable(page),
                    forwardCacheCount = 0,
                ),
                status = pageStatus(page, action),
                failure = null,
            )
        }
    }

    // ------------------------------------------------------------------ 内部：重排

    /**
     * 请求重排（**合并**）：拖动字号会触发几十次，这里只保留最终值；
     * 重排作业只有一条，且 `while (dirty)` 收敛到最新设置。
     */
    private fun requestReflow(label: String) {
        // 排版设置变更的唯一收口：此处持久化，重启后不再被重置（真机缺陷回归）。
        persistSettings()
        reflowLabel = label
        reflowDirty = true
        if (reflowJob?.isActive == true) {
            return
        }
        reflowJob = launchSerial {
            while (reflowDirty) {
                reflowDirty = false
                relayoutLocked(reflowLabel)
                // 收敛判据：本次重排之后，已发布页面的排版参数已等于当前设置/视口 → 丢弃期间
                // 累积的重复请求（例如「视口更新」与「设置变更」在同一帧到达），不白排第二次。
                if (laidOutSpec != null && laidOutSpec == currentSpec(_ui.value)) {
                    reflowDirty = false
                }
            }
        }
    }

    private suspend fun relayoutLocked(label: String) {
        val state = _ui.value
        if (!state.reader.isOpen) {
            return
        }
        val page = state.reader.page
        val anchor = if (page != null) {
            PageAnchor(page.chapterIndex, page.startByte)
        } else {
            restoredAnchor
        }
        val ordinalForPage = page?.pageOrdinal ?: 1
        generation.incrementAndGet()
        // 重排不需要重置任何历史：后退按位置重算，锚点集只作为扫描起点。
        // （旧实现在此清空历史栈导致「上一页」置灰，见 results/nav-persist-fix.md）
        forwardCache.clear()
        if (forceInvalidate) {
            forceInvalidate = false
            val staleLayoutId = layoutId
            if (staleLayoutId != 0L) {
                try {
                    val removed = withContext(io) { engine.invalidateLayout(staleLayoutId) }
                    setStatus("已失效排版世代 #$staleLayoutId（清理 $removed 条边界）")
                } catch (error: Throwable) {
                    throwIfCancellation(error)
                    failLocked(error, "失效排版边界")
                }
            }
        }
        // 签名可能已变化：必须重新 BEGIN_LAYOUT（核心据此切换世代并丢弃旧边界）。
        layoutId = 0L
        if (!beginLayoutLocked()) {
            return
        }
        layoutAtLocked(anchor.chapterIndex, anchor.startByte, ordinalForPage, label)
    }

    // ------------------------------------------------------------------ 内部：位置 / 工具

    private suspend fun saveCurrentPositionLocked(action: String) {
        val state = _ui.value
        val page = state.reader.page
        val bookId = state.reader.bookId
        if (page == null || bookId == null) {
            setStatus(ReaderStatus.NO_BOOK)
            return
        }
        persistPositionLocked(page)
        setStatus("$action · 章 ${page.chapterIndex} · 偏移 ${page.startByte}")
    }

    /**
     * 落库阅读进度（`nr_save_position`）并同步书架进度与 [ReaderState.lastSavedByteOffset]。
     *
     * **调用顺序约定**：本方法会写「已保存阅读位置」这一状态行，因此凡是需要回显本次动作
     * （翻页 / 切章 / 重排 / 索引重建）的调用点，都必须在**发布页面之前**调用它 —— 发布时写入的
     * 动作状态含页号、行数、字号、行高与偏移，是界面唯一的排版事实反馈，不能被「已保存」覆盖。
     * 只有显式保存入口（[savePositionNow]）在之后自行给出保存回显。
     */
    private suspend fun persistPositionLocked(page: PageState) {
        val state = _ui.value
        val bookId = state.reader.bookId ?: return
        val changed = try {
            withContext(io) {
                engine.savePosition(
                    NrSavePositionRequest(
                        chapterIndex = page.chapterIndex,
                        bookId = bookId,
                        originalByteOffset = page.startByte,
                    ),
                )
            }
        } catch (error: Throwable) {
            throwIfCancellation(error)
            failLocked(error, "保存阅读位置")
            return
        }
        val book = shelf.values.firstOrNull { it.bookId == bookId }
        if (book != null) {
            shelf[book.sourceIdentity] = book.copy(
                lastChapterIndex = page.chapterIndex,
                lastByteOffset = page.startByte,
            )
        }
        update {
            it.copy(
                reader = it.reader.copy(lastSavedByteOffset = page.startByte),
                shelf = it.shelf.copy(books = shelf.values.toList()),
                status = if (changed) {
                    "已保存阅读位置（章 ${page.chapterIndex} · 偏移 ${page.startByte}）"
                } else {
                    it.status
                },
            )
        }
    }

    private fun chapterTitle(chapterIndex: Int): String =
        chapters.getOrNull(chapterIndex)?.title?.takeIf { it.isNotEmpty() } ?: "第 ${chapterIndex + 1} 章"

    /**
     * 状态栏文案。
     *
     * **不显示页码**：页码只属某个排版签名，切章/重开/改字号后必然失真
     * （真机实测恒显示「第 1 页」）。改用与排版无关的**阅读进度百分比**（按字节位置）。
     */
    private fun pageStatus(page: PageState, action: String): String {
        val total = _ui.value.reader.totalBytes
        val permille = if (total <= 0L) 0 else ((page.startByte * 1000L) / total).toInt().coerceIn(0, 1000)
        // 编码常驻信息区（用户要求）：换页/切章后仍能一眼看到当前使用的编码。
        val encoding = _ui.value.reader.encodingName.ifEmpty { "编码未知" }
        val pending = if (_ui.value.reader.needsEncodingConfirmation) "待确认" else ""
        return "$action · ${page.chapterTitle} · 进度 ${formatPermille(permille)} · " +
            "编码 $encoding$pending · 字 ${page.fontSizePx.toInt()}px · " +
            "行高 ${page.lineHeightPx.toInt()}px · ${page.lineCount} 行 · 偏移 ${page.startByte}→${page.endByte} · " +
            "已登记 ${page.registeredPageCount} 页"
    }

    private fun isStale(status: Long): Boolean = status == NrAbiConstants.NR_BOUNDARY_STALE_LAYOUT_ID ||
        status == NrAbiConstants.NR_BOUNDARY_STALE_REQUEST_ID ||
        status == NrAbiConstants.NR_BOUNDARY_STALE_CONTENT_INDEX_VERSION

    private fun update(transform: (ReaderUiState) -> ReaderUiState) {
        // 读-改-写必须原子：宿主线程（生命周期 / 设置 / 视口）与 gate 内的协程都会写同一份状态，
        // 非原子更新的结果是「丢字段」（例如刚发布的页面被另一路径的旧快照覆盖）。
        synchronized(uiLock) {
            _ui.value = transform(_ui.value)
        }
    }

    private fun setStatus(status: String) {
        update { it.copy(status = status) }
    }

    private fun reportAbiVersion() {
        try {
            val version = NrEngine.abiVersion()
            update { it.copy(abiVersion = version) }
        } catch (error: Throwable) {
            // 取不到 ABI 版本时保持 null（不假装成功）；失败状态照样可被界面显示。
            update {
                it.copy(
                    status = "无法读取 ABI 版本（${error.javaClass.simpleName}）",
                )
            }
        }
    }

    /** 跨边界失败统一落成 [ReaderFailure]：分类来自错误码，提示文案不参与业务分支。 */
    private fun failLocked(error: Throwable, what: String) {
        val failure = when (error) {
            is NrException -> ReaderFailure(error.rawCode, error.kind, what)
            else -> ReaderFailure(NrErrors.NR_ERR_INTERNAL, NrErrorKind.INTERNAL, what)
        }
        update {
            it.copy(
                failure = failure,
                status = "${ReaderStatus.failure(failure.rawCode, failure.kind)} · $what（${messageFor(failure.kind)}）",
            )
        }
    }

    override fun onCleared() {
        // 句柄只销毁一次；销毁后所有调用立即失败，不再触碰 native。
        engine.destroy()
        super.onCleared()
    }

    /** 会话内的页锚点（章节 + 起始字节）。 */

    /** 一次测量的结果分类（READY 之外的三种都必须能被调用方区分处理）。 */
    private enum class MeasureOutcome { READY, STALE, FAILED, AT_END }

    private data class MeasuredPage(val outcome: MeasureOutcome, val page: PageState? = null)

    companion object {
        /** 前向预取页数（与桌面端一致的「三页可达」策略：当前页 + 预取 2 页）。 */
        const val MAX_FORWARD_CACHE = 2

        /** 后退扫描的页数上限（防止异常数据造成无界循环；达到上限如实返回已确认页）。 */
        const val MAX_BACKWARD_SCAN = 256

        /** 每本书持久化的稀疏锚点数上限。 */
        const val MAX_SPARSE_ANCHORS = 16

        /** 页边界被判过期时的最大重试次数（超过即如实回报失败）。 */
        const val MAX_STALE_RETRY = 2

        /** 章节批次循环上限（防止异常数据造成无界循环）。 */
        private const val MAX_CHAPTER_BATCHES = 64

        /** 预取页的会话页码占位：发布时统一改写为真实页码。 */
        private const val PAGE_ORDINAL_UNSET = 0

        /** 排版方向：横向（`nr_layout_signature.direction`）。 */
        private const val DIRECTION_HORIZONTAL = 0L

        /**
         * 生产入口：加载 `libnr_jni.so` 创建引擎。
         *
         * [dbPath] 由宿主提供（AN4/AN6 传应用私有目录路径）；`null` 表示核心用内存数据库。
         */
        fun productionFactory(
            dbPath: ByteArray? = null,
            provider: NrSourceProvider? = null,
            hostResolver: HostSourceResolver? = null,
            anchorStore: ReaderAnchorStore? = null,
            settingsStore: ReaderSettingsStore? = null,
            bookCatalog: HostBookCatalog? = null,
            hiddenBooks: HiddenBooksStore? = null,
        ): ViewModelProvider.Factory = viewModelFactory {
            initializer {
                ReaderViewModel(
                    engine = NrEngine.create(dbPath = dbPath, provider = provider),
                    hostResolver = hostResolver,
                    anchorStore = anchorStore,
                    settingsStore = settingsStore,
                    bookCatalog = bookCatalog,
                    hiddenBooks = hiddenBooks,
                )
            }
        }
    }
}

package com.novelreader.android.source

import java.io.File
import android.content.Context
import android.os.Handler
import android.os.Looper
import com.novelreader.android.core.NrAbiConstants
import com.novelreader.android.reader.HostBookCatalog
import com.novelreader.android.reader.HostBookEntry
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors

/**
 * SAF 宿主接线：把「用户在系统选择器里选中的 URI」变成一次可打开的数据源。
 *
 * # 职责边界
 *
 * 本类只做**编排**：`intake` → 按 `accessMode` 选择通道 → 回报「用哪个 kind、什么标识、多长」。
 * 内容坐标、UTF-8 边界、章节与位置语义**全部**由 Rust 核心裁定（本类不做任何阅读业务判断）。
 *
 * # 两条通道
 *
 * - [SafAccessMode.WINDOW]：可定位来源 → [SafBookSource.openWindowSession] 得到
 *   `NrSourceProvider` 会话，绑定到 [SafWindowProviderHolder.shared]，以
 *   `NR_SOURCE_KIND_PLATFORM` + URI + `session.totalBytes` 打开；
 * - [SafAccessMode.CACHE_COPY]：不可定位 / 长度未知 → [SafBookSource.startFallbackCopy]
 *   复制到应用私有缓存，再以 `NR_SOURCE_KIND_FILE` + 缓存文件路径打开。
 *
 * 两条通道都不整本载入内存（窗口读取由核心按 `maxLength` 裁定；复制走流式 sink）。
 *
 * # 线程
 *
 * [open] 在后台单线程执行（ContentResolver 是阻塞 IO，且回退复制可能很长），
 * 回调一律切回**主线程**，宿主可直接更新 UI 状态。
 *
 * # 生命周期
 *
 * 实例按 Activity 创建即可（内部只持有 application context）。
 * [close] 只关闭本实例的执行器，**不**解绑会话 —— 已打开书籍的窗口会话必须跨 Activity 重建存活，
 * 其归还有两条路径：打开下一本书时自动归还上一个，或进程结束由系统回收。
 */
class SafHost(context: Context) : AutoCloseable, HostBookCatalog {

    companion object {
        /** 背景图临时目录名（与书籍缓存 `saf-cache` 分开）。 */
        private const val BACKGROUND_SRC_DIR = "background-src"
    }

    /** 一次可打开的数据源描述（宿主据此调用 ViewModel 的打开入口）。 */
    data class OpenTarget(
        /** [NrAbiConstants.NR_SOURCE_KIND_PLATFORM] 或 [NrAbiConstants.NR_SOURCE_KIND_FILE]。 */
        val kind: Int,
        /** 平台窗口 = SAF URI；私有缓存 = 缓存文件路径。 */
        val identity: String,
        /** 平台窗口的坐标空间长度；文件源传 0（以文件实际长度为准）。 */
        val totalBytes: Long,
        val displayName: String?,
    )

    private val source: SafBookSource
    /** 平台桥接句柄（V1.1 背景图物化需要它开流；与 [source] 共用同一实例）。 */
    private val platform: AndroidSafPlatform
    private val mainHandler = Handler(Looper.getMainLooper())

    private val executor: ExecutorService = Executors.newSingleThreadExecutor { runnable ->
        Thread(runnable, "saf-host").apply { isDaemon = true }
    }

    init {
        val application = context.applicationContext
        val bridge = AndroidSafPlatform(application)
        platform = bridge
        source = SafBookSource.create(
            platform = bridge,
            keyValueStore = AndroidSafKeyValue(application),
        )
    }

    /**
     * 把用户选中的图片**物化**成本地文件，返回可交给核心的路径（V1.1 背景图）。
     *
     * 复用既有 SAF 回退复制的机制（定长块流式复制 + 原子转正 + 半成品不可见），
     * 但目标是**独立的图片缓存目录** —— 不与书籍缓存混放，避免书籍缓存的清理策略误删背景图。
     *
     * 核心随后会把这个文件**复制**到应用数据目录并只存相对文件名（规格 R14），
     * 故本文件只是一次性的中转件：调用方用完可删，删了也不影响已设置的背景。
     *
     * @throws IllegalStateException 失败时抛出（`message` 可直接展示给用户）。
     */
    fun materializeBackgroundSource(uri: String): String {
        val store = SafCacheStore(File(platform.privateCacheRoot(), BACKGROUND_SRC_DIR))
        if (!store.ensureRoot()) {
            throw IllegalStateException("无法创建背景图临时目录")
        }
        // 平台返回 null = 无法为该 URI 开流（提供者拒绝 / 权限失效）—— 如实失败。
        val stream = platform.openSequentialStream(uri)
            ?: throw IllegalStateException("无法打开所选图片（提供者拒绝或权限失效）")
        val outcome = SafCacheCopyTask(
            stream = stream,
            sink = store.sinkFor(uri),
            expectedBytes = SafPolicy.UNKNOWN_SIZE,
        ).run()
        return when (outcome) {
            is SafCopyOutcome.Completed -> outcome.cachePath
            is SafCopyOutcome.Cancelled -> throw IllegalStateException("背景图复制已取消")
            is SafCopyOutcome.Failed -> throw IllegalStateException(outcome.detail)
        }
    }

    /** 已登记的来源记录（诊断用途；不参与阅读业务）。 */
    fun records(): List<SafSourceRecord> = source.records()

    /**
     * 书架目录：把宿主登记的来源映射为**可直接打开**的条目。
     *
     * - 平台窗口来源 → SAF URI（重开时由 [resolveExisting] 重建会话）；
     * - 已有私有缓存副本 → 缓存文件路径（可直接以文件通道打开）；
     * - 需要重新关联 / 无可读副本 → **不列出**，避免「点开必失败」。
     */
    override fun list(): List<HostBookEntry> = source.records().mapNotNull { record ->
        when {
            record.accessMode == SafAccessMode.WINDOW ->
                HostBookEntry(
                    record.uri,
                    record.displayName,
                    NrAbiConstants.NR_SOURCE_KIND_PLATFORM,
                    record.bookId,
                )

            record.hasCacheCopy && !record.cachePath.isNullOrBlank() ->
                HostBookEntry(
                    record.cachePath,
                    record.displayName,
                    NrAbiConstants.NR_SOURCE_KIND_FILE,
                    record.bookId,
                )

            else -> null
        }
    }

    /** 打开成功后回填核心 `book_id`（仅对已登记的 URI 生效；缓存路径键会被忽略）。 */
    override fun rememberBookId(sourceIdentity: String, bookId: Long) {
        if (source.recordOf(sourceIdentity) == null) return
        source.attachBookId(sourceIdentity, bookId)
    }

    /**
     * 异步物化背景图（在自己的线程上复制，回调回主线程）—— 与 [open] 同一套线程模型。
     *
     * 复制大图可能耗时，故**不得**在 UI 线程直接调 [materializeBackgroundSource]。
     */
    fun materializeBackgroundSourceAsync(
        uri: String,
        onReady: (String) -> Unit,
        onFailure: (String) -> Unit,
    ) {
        executor.execute {
            val result = runCatching { materializeBackgroundSource(uri) }
            mainHandler.post {
                result.fold(
                    onSuccess = onReady,
                    onFailure = { error -> onFailure(error.message ?: "背景图读取失败") },
                )
            }
        }
    }

    /**
     * 为**已登记**的来源重新解析打开目标（书架再次打开时使用）。
     *
     * - 平台窗口来源：重新建立窗口会话并绑定转发器（描述符不跨会话存活），返回新的坐标空间长度；
     * - 私有缓存来源：直接复用既有副本路径，不重新复制；
     * - 已失效来源：抛异常，由调用方转成可见失败（绝不静默降级）。
     *
     * 需在后台线程调用（ContentResolver 为阻塞 IO）。
     */
    fun resolveExisting(uri: String): OpenTarget {
        val record = source.recordOf(uri)
            ?: throw IllegalStateException("来源记录不存在（可能已被清理）：$uri")
        return when (record.accessMode) {
            SafAccessMode.WINDOW -> openWindow(record)
            SafAccessMode.CACHE_COPY -> openCacheCopy(record)
            SafAccessMode.NEEDS_REASSOCIATION ->
                throw IllegalStateException("来源已失效，需要重新关联：${record.displayName}")
        }
    }

    /**
     * 处理一次 SAF 选择结果。
     *
     * [onOpened] 与 [onFailure] 恰有一个被调用，且都在**主线程**上；
     * 失败一律带可展示的原因（拒绝 / 需要重新关联 / 读不到字节），不静默。
     */
    fun open(
        uri: String,
        reportedMimeType: String?,
        onOpened: (OpenTarget) -> Unit,
        onFailure: (String) -> Unit,
    ) {
        executor.execute {
            val target = runCatching { resolve(uri, reportedMimeType) }
            mainHandler.post {
                target.fold(
                    onSuccess = onOpened,
                    onFailure = { error -> onFailure(error.message ?: error.javaClass.simpleName) },
                )
            }
        }
    }

    /** 在后台线程完成 intake 与通道选择；失败以异常携带可展示原因。 */
    private fun resolve(uri: String, reportedMimeType: String?): OpenTarget =
        when (val intake = source.intake(uri, reportedMimeType)) {
            is SafIntakeResult.Rejected ->
                throw IllegalStateException("导入被拒绝：${intake.detail}")

            is SafIntakeResult.NeedsReassociation ->
                throw IllegalStateException("来源已失效，需要重新关联：${intake.detail}")

            is SafIntakeResult.Ready -> when (intake.mode) {
                SafAccessMode.WINDOW -> openWindow(intake.record)
                SafAccessMode.CACHE_COPY -> openCacheCopy(intake.record)
                SafAccessMode.NEEDS_REASSOCIATION ->
                    throw IllegalStateException("来源已失效，需要重新关联：${intake.record.displayName}")
            }
        }

    private fun openWindow(record: SafSourceRecord): OpenTarget {
        val session = source.openWindowSession(record.uri)
            ?: throw IllegalStateException("无法按平台窗口打开：权限或来源已失效（${record.displayName}）")
        SafWindowProviderHolder.shared.bind(session)
        return OpenTarget(
            kind = NrAbiConstants.NR_SOURCE_KIND_PLATFORM,
            identity = record.uri,
            totalBytes = session.totalBytes,
            displayName = record.displayName,
        )
    }

    private fun openCacheCopy(record: SafSourceRecord): OpenTarget {
        if (!record.hasCacheCopy) {
            val copied = source.startFallbackCopy(record)
            if (copied !is SafCopyOutcome.Completed) {
                throw IllegalStateException("回退复制未完成，无法取得私有缓存副本（${record.displayName}）")
            }
        }
        val updated = source.recordOf(record.uri)
        val cachePath = updated?.cachePath
        if (cachePath.isNullOrBlank()) {
            throw IllegalStateException("私有缓存副本路径缺失（${record.displayName}）")
        }
        return OpenTarget(
            kind = NrAbiConstants.NR_SOURCE_KIND_FILE,
            identity = cachePath,
            totalBytes = 0L,
            displayName = updated.displayName,
        )
    }

    override fun close() {
        executor.shutdownNow()
    }
}

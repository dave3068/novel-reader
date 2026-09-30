package com.novelreader.android.source

import java.io.File

/**
 * SAF 数据源协调器：导入 → 模式判定 → 窗口读取 / 回退复制 → 权限失效与重新关联。
 *
 * 卡片交付物的入口全部在这里（AR1 / AR2 / AR6）：
 * - [intake]：`OpenDocument` 返回的 URI 进厂，申请持久读权限、读元数据、判定访问模式并落记录；
 * - [openWindowSession]：可定位来源按**窗口**读取（`NR_SOURCE_KIND_PLATFORM`）；
 * - [startFallbackCopy]：不可定位 / 长度未知时**可取消地**复制到私有缓存，随后以文件方式分页
 *   （`NR_SOURCE_KIND_FILE` + `record.cachePath`）；
 * - [onAccessFailure] / [reassociate]：权限失效进入「需要重新关联」，**保留书籍记录**；
 * - [cancelWindowRequest]：把取消转达给正在飞的窗口读取（取消是部分成功）。
 *
 * 宿主接线（`MainActivity` / `ReaderViewModel` 属其它卡片范围，此处只给挂载点）：
 * ```kotlin
 * // MainActivity：注册 OpenDocument 启动器（SAF 选择器会给出持久化所需的 flags）
 * val picker = registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
 *     // uri?.let { viewModel.onDocumentPicked(it.toString()) }
 * }
 * picker.launch(arrayOf("text/plain"))
 *
 * // ReaderViewModel：构造一次（进程级），不持有 Activity
 * val source = SafBookSource(
 *     platform = AndroidSafPlatform(applicationContext),
 *     store = SafSourceStore(AndroidSafKeyValue(applicationContext)),
 *     cache = SafCacheStore(File(AndroidSafPlatform(applicationContext).privateCacheRoot())),
 * )
 * // 平台窗口：engine.create(provider = session) 后 openTxt(kind = NR_SOURCE_KIND_PLATFORM, identity = uri, total_bytes = session.totalBytes)
 * // 私有缓存：复制完成后以 NR_SOURCE_KIND_FILE + session/record.cachePath 打开
 * ```
 *
 * 本类**不持有** Activity / Context（平台侧只保存 application context），旋转与后台切换不会
 * 留下失效引用。
 */
class SafBookSource(
    private val platform: SafPlatform,
    private val store: SafSourceStore,
    private val cache: SafCacheStore,
    private val cancels: SafCancelRegistry = SafCancelRegistry(),
) {

    /** 回填核心返回的 `book_id`（宿主在 `nr_open_txt` 之后调用）。 */
    fun attachBookId(uri: String, bookId: Long): SafSourceRecord? = store.attachBookId(uri, bookId)

    /** 已登记的来源记录（供书架展示与诊断）。 */
    fun records(): List<SafSourceRecord> = store.list()

    fun recordOf(uri: String): SafSourceRecord? = store.load(uri)

    /**
     * 导入一个 SAF 选择结果。
     *
     * 三种出口都带明确状态：可用的 [SafIntakeResult.Ready]、明确拒绝的
     * [SafIntakeResult.Rejected]、以及需要重新关联的 [SafIntakeResult.NeedsReassociation]
     * （后者**不删除**任何记录）。
     */
    fun intake(uri: String, reportedMimeType: String? = null): SafIntakeResult {
        if (uri.isBlank()) {
            return SafIntakeResult.Rejected(SafRejectionReason.METADATA_UNAVAILABLE, "空 URI，无法导入")
        }
        val permissionGranted = platform.takePersistableReadPermission(uri)
        val meta = platform.queryMetadata(uri)
            ?: return needsReassociation(uri, permissionGranted, "元数据不可读：权限或来源已失效")
        SafPolicy.validate(meta, reportedMimeType)?.let { violation ->
            return SafIntakeResult.Rejected(violation.reason, violation.detail)
        }
        val handle = platform.openReadOnlyDescriptor(uri)
        if (handle < 0L) {
            return needsReassociation(uri, permissionGranted, "无法打开只读描述符：权限或来源已失效")
        }
        val totalBytes: Long
        val seekable: Boolean
        try {
            val descriptorSize = platform.descriptorSize(handle)
            totalBytes = if (descriptorSize >= 0L) descriptorSize else meta.sizeBytes
            seekable = platform.isSeekable(handle)
        } finally {
            platform.closeDescriptor(handle)
        }
        if (totalBytes == 0L) {
            return SafIntakeResult.Rejected(SafRejectionReason.EMPTY_FILE, "文件长度为 0 字节")
        }
        val mode = SafPolicy.decideAccessMode(seekable, totalBytes)
        val previous = store.load(uri)
        val keepsCache = mode == SafAccessMode.CACHE_COPY
        val record = SafSourceRecord(
            uri = uri,
            displayName = SafPolicy.displayNameOf(meta.displayName, uri),
            sizeBytes = totalBytes,
            mimeType = SafPolicy.effectiveMime(meta, reportedMimeType),
            accessMode = mode,
            permissionGranted = permissionGranted,
            bookId = previous?.bookId,
            cachePath = if (keepsCache) previous?.cachePath else null,
            cacheBytes = if (keepsCache) previous?.cacheBytes ?: SafPolicy.UNKNOWN_SIZE else SafPolicy.UNKNOWN_SIZE,
            lastFailure = SafPolicy.describePermission(permissionGranted),
        )
        store.save(record)
        return SafIntakeResult.Ready(record, mode)
    }

    /**
     * 重新关联：用户在权限失效后再次选择同一本书的文件。
     *
     * [previousUri] 为原记录（可为 null：此时等价于一次普通导入）。成功后记录被迁到
     * [newUri] 且保留 `bookId`，原键删除 —— 阅读进度与书签因此不丢。
     */
    fun reassociate(
        previousUri: String?,
        newUri: String,
        reportedMimeType: String? = null,
    ): SafIntakeResult {
        val previous = previousUri?.let { store.load(it) }
        val result = intake(newUri, reportedMimeType)
        if (previous == null || previousUri == newUri) return result
        if (result !is SafIntakeResult.Ready) return result
        val merged = store.reassociate(previousUri, newUri) ?: return result
        return SafIntakeResult.Ready(merged, merged.accessMode)
    }

    /**
     * 以窗口模式打开会话（仅对 [SafAccessMode.WINDOW] 的记录有意义）。
     *
     * 返回 null 表示不可用，且**已经**把原因写进记录（[SafAccessMode.NEEDS_REASSOCIATION]）——
     * 宿主应据此提示「需要重新关联」，而不是静默什么都不做。
     */
    fun openWindowSession(uri: String): SafWindowSession? {
        val record = store.load(uri) ?: return null
        if (record.accessMode != SafAccessMode.WINDOW) return null
        val handle = platform.openReadOnlyDescriptor(uri)
        if (handle < 0L) {
            onAccessFailure(uri, "打开只读描述符失败：权限或来源已失效")
            return null
        }
        val descriptorSize = platform.descriptorSize(handle)
        val totalBytes = if (descriptorSize >= 0L) descriptorSize else record.sizeBytes
        if (totalBytes <= 0L) {
            platform.closeDescriptor(handle)
            onAccessFailure(uri, "来源长度无效（$totalBytes），无法按窗口读取")
            return null
        }
        val session = SafWindowSession(
            record = record,
            reader = SafWindowReader(platform, handle, totalBytes, cancels),
            platform = platform,
            handle = handle,
        )
        store.save(record.copy(sizeBytes = totalBytes))
        return session
    }

    /**
     * 回退复制到私有缓存（可取消、有进度）。
     *
     * 成功后记录被更新为「已有缓存副本」，宿主应以 `NR_SOURCE_KIND_FILE` + `record.cachePath`
     * 打开书籍；取消 / 失败时半成品已被丢弃，记录保持原状（不写入假路径）。
     */
    fun startFallbackCopy(
        record: SafSourceRecord,
        token: SafCancelToken = SafCancelToken(),
        onProgress: (SafCopyProgress) -> Unit = {},
    ): SafCopyOutcome {
        cache.ensureRoot()
        val stream = platform.openSequentialStream(record.uri)
        if (stream == null) {
            onAccessFailure(record.uri, "无法打开来源顺序流：权限或来源已失效")
            return SafCopyOutcome.Failed("无法打开来源顺序流：权限或来源已失效", 0L)
        }
        val outcome = SafCacheCopyTask(
            stream = stream,
            sink = cache.sinkFor(record.uri),
            token = token,
            expectedBytes = record.sizeBytes,
            onProgress = onProgress,
        ).run()
        if (outcome is SafCopyOutcome.Completed) {
            store.updateCache(record.uri, outcome.cachePath, outcome.bytes)
            store.updateAccessMode(record.uri, SafAccessMode.CACHE_COPY)
        }
        return outcome
    }

    /**
     * 权限 / 来源失效：记录转入「需要重新关联」，**不删除**书籍记录。
     *
     * 返回更新后的记录；URI 从未登记过时返回 null。
     */
    fun onAccessFailure(uri: String, detail: String): SafSourceRecord? =
        store.markNeedsReassociation(uri, detail, permissionGranted = false)

    /** 权限恢复（用户在系统设置里重新授权后）：刷新状态与说明。 */
    fun onPermissionRestored(uri: String): SafSourceRecord? {
        val flags = platform.persistedReadPermissionFlags(uri)
        return store.refreshPermission(uri, flags != 0)
    }

    /** 把取消转达给正在飞的窗口读取（头文件 §7：取消标志与引擎锁分离，可跨线程生效）。 */
    fun cancelWindowRequest(requestId: Long) {
        cancels.cancel(requestId)
    }

    /** 窗口读取收尾：清除取消登记，避免长跑应用累积。 */
    fun completeWindowRequest(requestId: Long) {
        cancels.complete(requestId)
    }

    /** 删除私有缓存副本（包括半成品）并清空记录里的缓存字段。 */
    fun purgeCache(uri: String): Boolean {
        val removed = cache.remove(uri)
        store.updateCache(uri, null, SafPolicy.UNKNOWN_SIZE)
        return removed
    }

    private fun needsReassociation(
        uri: String,
        permissionGranted: Boolean,
        detail: String,
    ): SafIntakeResult {
        val existing = store.load(uri)
        val base = existing ?: SafSourceRecord(
            uri = uri,
            displayName = SafPolicy.displayNameOf(null, uri),
            sizeBytes = SafPolicy.UNKNOWN_SIZE,
            mimeType = "",
            accessMode = SafAccessMode.NEEDS_REASSOCIATION,
            permissionGranted = permissionGranted,
        )
        val record = base.copy(
            accessMode = SafAccessMode.NEEDS_REASSOCIATION,
            permissionGranted = permissionGranted,
            lastFailure = detail,
        )
        store.save(record)
        return SafIntakeResult.NeedsReassociation(record, detail)
    }

    companion object {

        /** 便捷构造：平台决定缓存根目录（应用私有缓存，不占用户可见存储）。 */
        fun create(platform: SafPlatform, keyValueStore: SafKeyValueStore): SafBookSource =
            SafBookSource(
                platform = platform,
                store = SafSourceStore(keyValueStore),
                cache = SafCacheStore(File(platform.privateCacheRoot())),
            )
    }
}

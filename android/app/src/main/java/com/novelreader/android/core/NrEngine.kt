package com.novelreader.android.core

/** 打开请求的薄参数（对应头文件 `nr_open_request`，结构体在 C 侧组装）。 */
class NrOpenRequest(
    val kind: Int,
    val totalBytes: Long,
    val identity: ByteArray,
    val displayName: ByteArray? = null,
)

/**
 * 页边界提交参数（对应头文件 `nr_page_boundary_submission`）。
 *
 * 本类只是参数容器：**不**计算页边界、**不**测量文本、**不**决定断行 —— 页宽页高来自
 * 平台测量（由 UI 卡片实现），合法性与世代判定由 Rust 核心裁定。
 */
class NrPageBoundarySubmission(
    val phase: Int,
    val contentIndexVersion: Long,
    val layoutId: Long,
    val requestId: Long,
    val totalBytes: Long,
    val startByte: Long,
    val endByte: Long,
    val layoutFields: LongArray? = null,
    val fontFamily: ByteArray? = null,
)

/** 保存阅读位置参数（对应头文件 `nr_position`）。 */
class NrSavePositionRequest(
    val chapterIndex: Int,
    val bookId: Long,
    val originalByteOffset: Long,
    val contentUnitId: Long = NrAbiConstants.NR_CONTENT_UNIT_SINGLE,
    val fingerprint: ByteArray? = null,
)

/** 新增书签参数（对应头文件 `nr_bookmark_input`）；[title] 为空时由引擎取章节标题。 */
class NrBookmarkRequest(
    val chapterIndex: Int,
    val bookId: Long,
    val byteOffset: Long,
    val contentUnitId: Long = NrAbiConstants.NR_CONTENT_UNIT_SINGLE,
    val title: ByteArray? = null,
)

/** 读取窗口的结果：元数据 + 正文字节 + 归还凭据。 */
class NrReadResult(
    val window: NrWindow,
    val bytes: ByteArray,
    val buffer: NrBufferHandle,
)

/** 读取位置的结果：位置结构体 + 附近文本指纹字节 + 归还凭据。 */
class NrPositionRead(
    val position: NrPositionResult,
    val fingerprint: ByteArray,
    val buffer: NrBufferHandle,
)

/** 章节批次结果：元数据 + 批次字节 + 归还凭据；[entries] 是布局解码，不含章节识别。 */
class NrChaptersRead(
    val batch: NrChapterBatch,
    val bytes: ByteArray,
    val buffer: NrBufferHandle,
) {
    val entries: List<NrChapterEntry>
        get() = NrChapterCodec.entries(bytes, batch.count)
}

/** 最近错误：错误码结构体 + 已复制出的消息字节（消息缓冲区已立即归还，[releaseCode] 回报归还结果）。 */
class NrLastError(
    val info: NrErrorInfo,
    val message: ByteArray,
    val releaseCode: Int,
)

/** 索引重建结果；[code] 可能是 `NR_ERR_CANCELLED`（部分成功，`partial = 1` 且不落库）。 */
/** [`NrEngine.setEncoding`] 的结果（手动指定内容编码）。 */
class NrSetEncodingOutcome(
    val result: NrSetEncodingResult,
) {
    /** 生效编码（`NR_ENCODING_*`）。 */
    val encodingCode: Int get() = result.encoding
    /** 重建后的内容索引版本（旧分页缓存据此失效）。 */
    val contentIndexVersion: Long get() = result.contentIndexVersion
    /** 按新编码重扫后的章节数量。 */
    val chapters: Int get() = result.chapters
    /** 是否就地重扫了索引。 */
    val indexRebuilt: Boolean get() = result.indexRebuilt
}

class NrRebuildOutcome(
    val result: NrRebuildResult,
    val code: Int,
) {
    val cancelled: Boolean
        get() = code == NrErrors.NR_ERR_CANCELLED
}

/**
 * 引擎薄封装：18 项 C ABI（[NrCore]）之上的**生命周期、错误映射与缓冲释放**。
 *
 * 职责边界（子规格 AR2 / design.md）：
 * - 只做参数装箱/拆箱、错误码 → [NrErrorKind] 分类、缓冲区归还；
 * - **不做**章节识别、位置校验、分页计算、缓存失效或书签规则 —— 这些语义全部由
 *   Rust `reading-core` 裁定（Kotlin 只编排与适配）；
 * - 句柄是不透明 64 位整数：`0` 永远无效，销毁后任何调用不再触碰 native，
 *   直接抛 [NrException]（`NR_ERR_ENGINE_DESTROYED`），避免 use-after-free；
 * - **不加 Kotlin 侧锁**：串行化由 Rust 引擎互斥锁负责，这样 [cancelRequest] 才能像
 *   头文件 §7 要求的那样在长任务（[rebuildIndex]）运行期间由另一线程生效；
 * - 64 位 id / 偏移全程 `Long`；跨 JSON 边界请使用 [NrWireIds]（字符串，避免 2^53 截断）。
 *
 * 结构化并发由调用方（ViewModel）编排；本类不持有任何 Activity/Compose 状态。
 */
class NrEngine internal constructor(
    private val abi: NrAbi,
    val handle: Long,
    private val releaser: NrReleaser,
) : AutoCloseable {

    /** 是否已销毁（销毁后所有调用立即失败，不再触碰 native）。 */
    var isDestroyed: Boolean = false
        private set

    // ------------------------------------------------------------------ 生命周期

    /**
     * 销毁引擎（`nr_engine_destroy`）。
     *
     * 首次返回 native 的返回码；**重复调用不触碰 native**，直接返回
     * `NR_ERR_ENGINE_DESTROYED`（与头文件 §1 的「重复销毁」语义一致，并保证句柄只被释放一次）。
     *
     * 实现先置销毁标志再调用 native：宁可让并发调用立即失败，也不冒 use-after-free 风险。
     */
    fun destroy(): Int {
        if (isDestroyed) {
            return NrErrors.NR_ERR_ENGINE_DESTROYED
        }
        isDestroyed = true
        return abi.engineDestroy(handle)
    }

    /** `AutoCloseable` 适配：幂等且不抛异常（重复关闭返回 [NrErrors.NR_ERR_ENGINE_DESTROYED]）。 */
    override fun close() {
        destroy()
    }

    // ------------------------------------------------------------------ 18 项入口

    /** 4. 打开 TXT（导入 upsert、编码识别、章节索引；正文不入库）。 */
    fun openTxt(request: NrOpenRequest): NrOpenResult {
        requireAlive()
        val out = ByteArray(NrOpenResult.STRUCT_SIZE)
        abi.openTxt(
            handle,
            request.kind,
            request.totalBytes,
            request.identity,
            request.displayName,
            out,
        ).requireOk()
        return NrOpenResult.decode(out)
    }

    /** 5. 关闭当前书籍。 */
    fun closeBook(bookId: Long) {
        requireAlive()
        abi.closeBook(handle, bookId).requireOk()
    }

    /**
     * 6. 读取有限窗口。
     *
     * 长度上限与合法性由核心裁定（`length == 0` 或超过窗口上限返回 `NR_ERR_INVALID_ARGUMENT`）。
     * 返回的 [NrReadResult.buffer] 必须由调用方归还（共 1 次）。
     */
    fun readWindow(chapterIndex: Int, offset: Long, length: Long): NrReadResult {
        requireAlive()
        val outWindow = ByteArray(NrWindow.STRUCT_SIZE)
        val outBuffer = LongArray(NrAbiConstants.NR_BUFFER_SLOTS)
        val bytes = abi.readWindow(handle, chapterIndex, offset, length, outWindow, outBuffer)
            ?: throw NrException(lastErrorRawCode())
        val window = NrWindow.decode(outWindow)
        val buffer = NrBufferHandle.fromSlots(outBuffer)
        // window.len 与归还凭据的 len 必须一致：不一致说明跨边界布局已错位。
        NrBufferHandle.requireLenMatches(buffer, window.len)
        return NrReadResult(window, bytes, buffer)
    }

    /** 7. 提交页边界（BEGIN_LAYOUT 由 [NrPageBoundarySubmission.phase] 表达）。 */
    fun submitPageBoundary(submission: NrPageBoundarySubmission): NrBoundaryResult {
        requireAlive()
        val out = ByteArray(NrBoundaryResult.STRUCT_SIZE)
        abi.submitPageBoundary(
            handle,
            submission.phase,
            submission.contentIndexVersion,
            submission.layoutId,
            submission.requestId,
            submission.totalBytes,
            submission.startByte,
            submission.endByte,
            submission.layoutFields,
            submission.fontFamily,
            out,
        ).requireOk()
        return NrBoundaryResult.decode(out)
    }

    /** 8. 查询页边界（未命中返回 `NR_ERR_LAYOUT_NOT_FOUND`）。 */
    fun getPageBoundary(layoutId: Long, chapterIndex: Int, startByte: Long): NrBoundaryResult {
        requireAlive()
        val out = ByteArray(NrBoundaryResult.STRUCT_SIZE)
        abi.getPageBoundary(handle, layoutId, chapterIndex, startByte, out).requireOk()
        return NrBoundaryResult.decode(out)
    }

    /** 9. 保存阅读位置（幂等 upsert）；返回 `out_changed != 0`。 */
    fun savePosition(request: NrSavePositionRequest): Boolean {
        requireAlive()
        val changed = IntArray(1)
        abi.savePosition(
            handle,
            request.chapterIndex,
            request.bookId,
            request.contentUnitId,
            request.originalByteOffset,
            request.fingerprint,
            changed,
        ).requireOk()
        return changed[0] != 0
    }

    /** 10. 读取阅读位置；返回的 [NrPositionRead.buffer] 必须由调用方归还（共 1 次）。 */
    fun getPosition(bookId: Long): NrPositionRead {
        requireAlive()
        val out = ByteArray(NrPositionResult.STRUCT_SIZE)
        val outBuffer = LongArray(NrAbiConstants.NR_BUFFER_SLOTS)
        val fingerprint = abi.getPosition(handle, bookId, out, outBuffer)
            ?: throw NrException(lastErrorRawCode())
        val position = NrPositionResult.decode(out)
        val buffer = NrBufferHandle.fromSlots(outBuffer)
        NrBufferHandle.requireSameSlots(
            buffer,
            position.nearbyFingerprintPtr,
            position.nearbyFingerprintLen,
            position.nearbyFingerprintCapacity,
        )
        NrBufferHandle.requireLenMatches(buffer, fingerprint.size.toLong())
        return NrPositionRead(position, fingerprint, buffer)
    }

    /** 11. 章节批次（`1 ≤ maxCount ≤ NR_MAX_CHAPTER_BATCH`）；批次字节必须归还。 */
    fun getChapters(bookId: Long, startSequence: Int, maxCount: Int): NrChaptersRead {
        requireAlive()
        val out = ByteArray(NrChapterBatch.STRUCT_SIZE)
        val outBuffer = LongArray(NrAbiConstants.NR_BUFFER_SLOTS)
        val bytes = abi.getChapters(handle, bookId, startSequence, maxCount, out, outBuffer)
            ?: throw NrException(lastErrorRawCode())
        val batch = NrChapterBatch.decode(out)
        val buffer = NrBufferHandle.fromSlots(outBuffer)
        NrBufferHandle.requireSameSlots(buffer, batch.bytesPtr, batch.bytesLen, batch.bytesCapacity)
        NrBufferHandle.requireLenMatches(buffer, bytes.size.toLong())
        return NrChaptersRead(batch, bytes, buffer)
    }

    /** 12. 新增书签，返回书签 id。 */
    fun createBookmark(request: NrBookmarkRequest): Long {
        requireAlive()
        val outId = LongArray(1)
        abi.createBookmark(
            handle,
            request.chapterIndex,
            request.bookId,
            request.contentUnitId,
            request.byteOffset,
            request.title,
            outId,
        ).requireOk()
        return outId[0]
    }

    /** 13. 删除书签；返回是否确有删除。 */
    fun deleteBookmark(bookmarkId: Long): Boolean {
        requireAlive()
        val deleted = IntArray(1)
        abi.deleteBookmark(handle, bookmarkId, deleted).requireOk()
        return deleted[0] != 0
    }

    /** 14. 失效当前排版世代；返回被清理的分页记录数。 */
    fun invalidateLayout(layoutId: Long): Long {
        requireAlive()
        val removed = LongArray(1)
        abi.invalidateLayout(handle, layoutId, removed).requireOk()
        return removed[0]
    }

    /**
     * 15. 重建章节索引（可取消）。
     *
     * 被取消时**不抛异常**：返回 [NrRebuildOutcome.cancelled] 为 true 的结果
     * （`partial = 1`，核心保证不落库），与 JNI 层「取消是部分成功」的约定一致。
     */
    /**
     * 19. 手动指定内容编码：`encodingCode` 取 `NR_ENCODING_*`（1..=7），`0` 表示清除覆盖、回到自动检测。
     *
     * 只改变「如何解释文件字节」，**永不写回原文件**；返回后章节索引已按新编码重扫。
     */
    fun setEncoding(bookId: Long, encodingCode: Int): NrSetEncodingOutcome {
        requireAlive()
        val out = ByteArray(NrSetEncodingResult.STRUCT_SIZE)
        val code = abi.setEncoding(handle, bookId, encodingCode, out)
        if (code != NrErrors.NR_ERR_OK) {
            throw NrException(code)
        }
        return NrSetEncodingOutcome(NrSetEncodingResult.decode(out))
    }

    fun rebuildIndex(bookId: Long, requestId: Long): NrRebuildOutcome {
        requireAlive()
        val out = ByteArray(NrRebuildResult.STRUCT_SIZE)
        val code = abi.rebuildIndex(handle, bookId, requestId, out)
        if (code != NrErrors.NR_ERR_OK && code != NrErrors.NR_ERR_CANCELLED) {
            throw NrException(code)
        }
        return NrRebuildOutcome(NrRebuildResult.decode(out), code)
    }

    /**
     * 16. 取消请求（协作式）。`requestId` 必须非 0（头文件 §7）。
     *
     * 本类不加锁，因此该方法可以在 [rebuildIndex] 运行期间由另一线程调用。
     * 未知 / 已完成的 requestId 由核心裁定（幂等语义在核心侧）。
     */
    fun cancelRequest(requestId: Long) {
        requireAlive()
        if (requestId == 0L) {
            throw NrException(
                NrErrors.NR_ERR_INVALID_ARGUMENT,
                message = "request_id 必须非 0（头文件 §7）",
            )
        }
        abi.cancelRequest(handle, requestId).requireOk()
    }

    /**
     * 17. 最近一次错误。
     *
     * 消息缓冲区在这里立即归还（内容已复制成 Kotlin 数组），避免调用方遗漏；归还结果经
     * [NrLastError.releaseCode] 回报（不静默丢弃）。消息**只用于展示**，判断分支一律使用
     * [NrErrorInfo.code] / [NrErrorKind.of]。
     */
    fun lastError(): NrLastError {
        requireAlive()
        val out = ByteArray(NrErrorInfo.STRUCT_SIZE)
        val outBuffer = LongArray(NrAbiConstants.NR_BUFFER_SLOTS)
        val message = abi.getLastError(handle, out, outBuffer)
            ?: throw NrException(
                NrErrors.NR_ERR_INTERNAL,
                message = "nr_get_last_error 未返回消息缓冲区（跨边界协议违例）",
            )
        val info = NrErrorInfo.decode(out)
        val buffer = NrBufferHandle.fromSlots(outBuffer)
        NrBufferHandle.requireSameSlots(buffer, info.messagePtr, info.messageLen, info.messageCapacity)
        val releaseCode = buffer.release(releaser)
        return NrLastError(info, message, releaseCode)
    }

    // ------------------------------------------------------------------ 内部

    /** 销毁后一律拒绝，不触碰 native。 */
    private fun requireAlive() {
        if (isDestroyed) {
            throw NrException(NrErrors.NR_ERR_ENGINE_DESTROYED)
        }
    }

    /**
     * `null` 返回时的错误码：按 README §2 从 `nr_get_last_error` 取。
     *
     * `null` 返回但最近错误码为 0 属于跨边界协议违例 —— 此时**不得**当作成功，
     * 归入 `NR_ERR_INTERNAL`。
     */
    private fun lastErrorRawCode(): Int {
        val code = try {
            lastError().info.code
        } catch (error: NrException) {
            error.rawCode
        }
        return if (code == NrErrors.NR_ERR_OK) NrErrors.NR_ERR_INTERNAL else code
    }

    companion object {

        /** 查询 ABI 版本（无句柄要求；应与 [NrAbiConstants.NR_ABI_VERSION] 对照）。 */
        fun abiVersion(): Int = NrJniAbi.abiVersion()

        /**
         * 创建引擎；句柄 `0` 视为失败（按 `NR_ERR_INTERNAL` 抛出，具体码见 C 层日志）。
         *
         * [maxWindowBytes] 传 0 表示使用核心硬上限（头文件 `nr_engine_config`）。
         */
        fun create(
            maxWindowBytes: Long = NrAbiConstants.NR_DEFAULT_WINDOW_BYTES,
            dbPath: ByteArray? = null,
            provider: NrSourceProvider? = null,
        ): NrEngine = createWith(NrJniAbi, maxWindowBytes, dbPath, provider, NrReleaser.JNI)

        /** 单测入口：用假实现替换 JNI 绑定与归还器（生产路径见 [create]）。 */
        internal fun createWith(
            abi: NrAbi,
            maxWindowBytes: Long,
            dbPath: ByteArray?,
            provider: NrSourceProvider?,
            releaser: NrReleaser,
        ): NrEngine {
            val handle = abi.engineCreate(maxWindowBytes, dbPath, provider)
            if (handle == 0L) {
                throw NrException(
                    NrErrors.NR_ERR_INTERNAL,
                    message = "nr_engine_create 返回句柄 0（创建失败，具体码见 C 层日志）",
                )
            }
            return NrEngine(abi, handle, releaser)
        }
    }
}

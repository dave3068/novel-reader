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

/** 分类列表读取结果：批次元数据 + 扁平条目 + 归还凭据。 */
class NrCategoriesRead(
    val batch: NrCategoryBatch,
    val bytes: ByteArray,
    val buffer: NrBufferHandle,
) {
    /** 扁平条目（**不在这里拼树** —— 树形由界面按 `parentId` 组装）。 */
    val entries: List<NrCategoryEntry>
        get() = NrCategoryCodec.entries(bytes, batch.count)

    /** 条目 + 已解码名称（最常见用法，避免调用方逐个再解一遍）。 */
    val named: List<Pair<NrCategoryEntry, String>>
        get() = entries.map { it to NrCategoryCodec.name(bytes, it) }
}

/** 阅读历史读取结果：批次元数据 + 条目 + 归还凭据。 */
class NrHistoryRead(
    val batch: NrHistoryBatch,
    val bytes: ByteArray,
    val buffer: NrBufferHandle,
) {
    val entries: List<NrHistoryEntry>
        get() = NrHistoryCodec.entries(bytes, batch.count)

    /** 条目 + 已解码三段文本（书名 · 文件名 · **核心格式化的时间标签**）。 */
    val rows: List<NrHistoryRow>
        get() = entries.map { entry ->
            NrHistoryRow(
                entry = entry,
                title = NrHistoryCodec.title(bytes, entry),
                fileLabel = NrHistoryCodec.fileLabel(bytes, entry),
                timeLabel = NrHistoryCodec.timeLabel(bytes, entry),
            )
        }
}

/** 一条历史流水的展示用组合。 */
class NrHistoryRow(
    val entry: NrHistoryEntry,
    val title: String,
    val fileLabel: String,
    /** **已由核心格式化**，界面不得再格式化或截断（规格 §3.6）。 */
    val timeLabel: String,
)

/** 背景读取结果：批次元数据 + 单条条目 + 归还凭据。 */
class NrBackgroundRead(
    val batch: NrBackgroundBatch,
    val bytes: ByteArray,
    val buffer: NrBufferHandle,
) {
    /** 未设置时为 `null`（批次 `count = 0`）。 */
    val entry: NrBackgroundEntry?
        get() = NrBackgroundCodec.entry(bytes, batch.count)

    /** **相对文件名**（未设置为空串）；绝对路径由调用方用 app-private 目录拼（R15）。 */
    val fileName: String
        get() = NrBackgroundCodec.fileName(bytes, entry)
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

    // ------------------------------------------------------------------ V1.1 分类

    /**
     * 列出全部分类（`nr_list_categories`）。
     *
     * 回的是**扁平列表**（含 `parentId` / `depth`）—— 树形由界面按 `parentId` 组装，
     * 层级语义（深度、计数）已由核心算好。
     */
    fun listCategories(): NrCategoriesRead {
        requireAlive()
        val out = ByteArray(NrCategoryBatch.STRUCT_SIZE)
        val outBuffer = LongArray(NrAbiConstants.NR_BUFFER_SLOTS)
        val bytes = abi.listCategories(handle, out, outBuffer) ?: throw NrException(lastErrorRawCode())
        val batch = NrCategoryBatch.decode(out)
        val buffer = NrBufferHandle.fromSlots(outBuffer)
        NrBufferHandle.requireSameSlots(buffer, batch.bytesPtr, batch.bytesLen, batch.bytesCapacity)
        NrBufferHandle.requireLenMatches(buffer, bytes.size.toLong())
        return NrCategoriesRead(batch, bytes, buffer)
    }

    /**
     * 新建分类（`nr_create_category`）。同父同名由**核心拒绝**（不在这里判重）。
     *
     * [name] 是 UTF-8 字节；`parentId = -1` 表示建在根下。
     */
    fun createCategory(parentId: Long, name: ByteArray): Long {
        requireAlive()
        val outId = LongArray(1)
        val code = abi.createCategory(handle, parentId, name, outId)
        if (code != NrErrors.NR_ERR_OK) {
            throw NrException(code)
        }
        return outId[0]
    }

    /** 把书移入分类（`nr_move_book_to_category`）；`categoryId = -1` 表示移回未分类。 */
    fun moveBookToCategory(bookId: Long, categoryId: Long) {
        requireAlive()
        val code = abi.moveBookToCategory(handle, bookId, categoryId)
        if (code != NrErrors.NR_ERR_OK) {
            throw NrException(code)
        }
    }

    /**
     * 删除分类（`nr_delete_category`），并处理其书籍去向。
     *
     * [destination]：`1` = 移入 [targetId]；`2` = 移出书架；`3` = 移回根下未分类。
     * **未选（`0`）由核心拒绝且零副作用** —— 界面必须显式给出选择。
     *
     * @return 实际被移动的书籍数。
     */
    fun deleteCategory(categoryId: Long, destination: Int, targetId: Long = -1L): Long {
        requireAlive()
        val outMoved = IntArray(1)
        val code = abi.deleteCategory(handle, categoryId, destination, targetId, outMoved)
        if (code != NrErrors.NR_ERR_OK) {
            throw NrException(code)
        }
        return outMoved[0].toLong() and 0xFFFFFFFFL
    }

    // ------------------------------------------------------------------ V1.1 历史

    /**
     * 列出阅读历史（`nr_list_history`）。
     *
     * [bookId] > 0 限定单书；[limit] > 0 限量；[grouped] = true 走按书聚合。
     * **排序在核心完成**（时间倒序），界面不得重排。
     */
    fun listHistory(bookId: Long = 0L, limit: Int = 0, grouped: Boolean = false): NrHistoryRead {
        requireAlive()
        val out = ByteArray(NrHistoryBatch.STRUCT_SIZE)
        val outBuffer = LongArray(NrAbiConstants.NR_BUFFER_SLOTS)
        val bytes = abi.listHistory(handle, bookId, limit, if (grouped) 1 else 0, out, outBuffer)
            ?: throw NrException(lastErrorRawCode())
        val batch = NrHistoryBatch.decode(out)
        val buffer = NrBufferHandle.fromSlots(outBuffer)
        NrBufferHandle.requireSameSlots(buffer, batch.bytesPtr, batch.bytesLen, batch.bytesCapacity)
        NrBufferHandle.requireLenMatches(buffer, bytes.size.toLong())
        return NrHistoryRead(batch, bytes, buffer)
    }

    /**
     * 清除一条阅读流水（`nr_clear_history_entry`）。
     *
     * @return `true` = 已清除；`false` = 该 id 不存在（**如实回报**，不谎报成功）。
     */
    fun clearHistoryEntry(entryId: Long): Boolean {
        requireAlive()
        val outRemoved = IntArray(1)
        val code = abi.clearHistoryEntry(handle, entryId, outRemoved)
        if (code != NrErrors.NR_ERR_OK) {
            throw NrException(code)
        }
        return outRemoved[0] != 0
    }

    /** 清除某本书的全部阅读流水（`nr_clear_history_for_book`）。**只删流水**。 */
    fun clearHistoryForBook(bookId: Long): Long {
        requireAlive()
        val outRemoved = IntArray(1)
        val code = abi.clearHistoryForBook(handle, bookId, outRemoved)
        if (code != NrErrors.NR_ERR_OK) {
            throw NrException(code)
        }
        return outRemoved[0].toLong() and 0xFFFFFFFFL
    }

    // ------------------------------------------------------------------ V1.1 配色

    /**
     * 读回四个基色与启用位（`nr_get_theme_base`）。
     *
     * **只回基色** —— 40 个语义令牌不落库（规格 R11）；令牌值走 [getThemeToken]。
     */
    fun getThemeBase(): NrThemeBase {
        requireAlive()
        val out = ByteArray(NrThemeBase.STRUCT_SIZE)
        val code = abi.getThemeBase(handle, out)
        if (code != NrErrors.NR_ERR_OK) {
            throw NrException(code)
        }
        return NrThemeBase.decode(out)
    }

    /**
     * 写入一个基色（`nr_set_theme_base`）。
     *
     * [which]：`0`=正文 `1`=背景 `2`=主色 `3`=强调；[value] 空 = **清除该项**（回落内置值）。
     * 非法色值由核心拒绝并**保留原值**；写基色**不会**自动启用（那是用户的显式选择）。
     */
    fun setThemeBase(which: Int, value: ByteArray) {
        requireAlive()
        val code = abi.setThemeBase(handle, which, value)
        if (code != NrErrors.NR_ERR_OK) {
            throw NrException(code)
        }
    }

    /** 切换「启用自定义配色」（`nr_set_theme_enabled`）；关闭时**保留基色**。 */
    fun setThemeEnabled(enabled: Boolean) {
        requireAlive()
        val code = abi.setThemeEnabled(handle, if (enabled) 1 else 0)
        if (code != NrErrors.NR_ERR_OK) {
            throw NrException(code)
        }
    }

    /**
     * 取一个派生令牌的 RGBA（`nr_get_theme_token`）。
     *
     * [tokenIndex] 是令牌表下标（越界由核心拒绝）；[dark] 取暗色一套。
     * **派生全在核心** —— 同一基色在桌面与 Android 必然显示一致。
     *
     * @return 4 字节 RGBA（R、G、B、A 各 0..255）。
     */
    fun getThemeToken(tokenIndex: Int, dark: Boolean): ByteArray {
        requireAlive()
        // 核心写**4 字节** RGBA（不是打包整数），故用 ByteArray 而非 IntArray。
        val out = ByteArray(4)
        val code = abi.getThemeToken(handle, tokenIndex, if (dark) 1 else 0, out)
        if (code != NrErrors.NR_ERR_OK) {
            throw NrException(code)
        }
        return out
    }

    /**
     * 取对比度读数（`nr_get_theme_contrast`）。
     *
     * 比值是**整数 × 100**（`454` ⇒ `4.54 : 1`）；`passes = false` 时核心**已保留用户基色**。
     */
    fun getThemeContrast(): NrThemeContrastResult {
        requireAlive()
        val out = ByteArray(NrThemeContrastResult.STRUCT_SIZE)
        val code = abi.getThemeContrast(handle, out)
        if (code != NrErrors.NR_ERR_OK) {
            throw NrException(code)
        }
        return NrThemeContrastResult.decode(out)
    }

    // ------------------------------------------------------------------ V1.1 背景

    /**
     * 读回全局背景（`nr_get_background`）。**只回相对文件名**（规格 R15）。
     *
     * 未设置时 [NrBackgroundRead.fileName] 为空串、[NrBackgroundRead.entry] 为 `null`。
     */
    fun getBackground(): NrBackgroundRead {
        requireAlive()
        val out = ByteArray(NrBackgroundBatch.STRUCT_SIZE)
        val outBuffer = LongArray(NrAbiConstants.NR_BUFFER_SLOTS)
        val bytes = abi.getBackground(handle, out, outBuffer) ?: throw NrException(lastErrorRawCode())
        return decodeBackground(out, bytes, outBuffer)
    }

    /**
     * 设置全局背景（`nr_set_background`）。
     *
     * 核心**复制** [srcPath] 到 [destDir]（原图只读，绝不移动/删除），只把**相对名**写库。
     * [destDir] 由调用方给（Android 传 app-private 目录）—— 核心不猜平台数据目录。
     * [fitMode] / [scrimPermille] 传负数 = 用默认值。
     */
    fun setBackground(
        srcPath: ByteArray,
        destDir: ByteArray,
        fitMode: Int = -1,
        scrimPermille: Int = -1,
    ): NrBackgroundRead {
        requireAlive()
        val out = ByteArray(NrBackgroundBatch.STRUCT_SIZE)
        val outBuffer = LongArray(NrAbiConstants.NR_BUFFER_SLOTS)
        val bytes = abi.setBackground(handle, srcPath, destDir, fitMode, scrimPermille, out, outBuffer)
            ?: throw NrException(lastErrorRawCode())
        return decodeBackground(out, bytes, outBuffer)
    }

    /**
     * 只调遮罩强度（`nr_set_background_scrim`）—— **不换图**。
     *
     * `scrimPermille` 必须在 `0..=1000`，否则由核心拒绝。
     */
    fun setBackgroundScrim(scrimPermille: Int): NrBackgroundRead {
        requireAlive()
        val out = ByteArray(NrBackgroundBatch.STRUCT_SIZE)
        val outBuffer = LongArray(NrAbiConstants.NR_BUFFER_SLOTS)
        val bytes = abi.setBackgroundScrim(handle, scrimPermille, out, outBuffer)
            ?: throw NrException(lastErrorRawCode())
        return decodeBackground(out, bytes, outBuffer)
    }

    /**
     * 清除背景（`nr_clear_background`）。
     *
     * **只删核心生成的文件**（`bg-*` 前缀）；**不动用户原图**。
     */
    fun clearBackground(destDir: ByteArray): NrBackgroundRead {
        requireAlive()
        val out = ByteArray(NrBackgroundBatch.STRUCT_SIZE)
        val outBuffer = LongArray(NrAbiConstants.NR_BUFFER_SLOTS)
        val bytes = abi.clearBackground(handle, destDir, out, outBuffer)
            ?: throw NrException(lastErrorRawCode())
        return decodeBackground(out, bytes, outBuffer)
    }

    /** 背景批次共用的解码 + 归还凭据核对（三个入口同一口径）。 */
    private fun decodeBackground(out: ByteArray, bytes: ByteArray, outBuffer: LongArray): NrBackgroundRead {
        val batch = NrBackgroundBatch.decode(out)
        val buffer = NrBufferHandle.fromSlots(outBuffer)
        NrBufferHandle.requireSameSlots(buffer, batch.bytesPtr, batch.bytesLen, batch.bytesCapacity)
        NrBufferHandle.requireLenMatches(buffer, bytes.size.toLong())
        return NrBackgroundRead(batch, bytes, buffer)
    }

    /**
     * 书籍 → 分类映射（`nr_list_book_categories`）。
     *
     * 只含在架书籍；`-1` 表示未分类。界面把书架条目按分类过滤时用它：
     * 宿主目录给 `sourceIdentity → bookId`，本方法给 `bookId → categoryId`。
     */
    fun listBookCategories(): Map<Long, Long> {
        requireAlive()
        val out = ByteArray(NrBookCategoryBatch.STRUCT_SIZE)
        val outBuffer = LongArray(NrAbiConstants.NR_BUFFER_SLOTS)
        val bytes = abi.listBookCategories(handle, out, outBuffer)
            ?: throw NrException(lastErrorRawCode())
        val batch = NrBookCategoryBatch.decode(out)
        val buffer = NrBufferHandle.fromSlots(outBuffer)
        NrBufferHandle.requireSameSlots(buffer, batch.bytesPtr, batch.bytesLen, batch.bytesCapacity)
        NrBufferHandle.requireLenMatches(buffer, bytes.size.toLong())
        val mapped = NrBookCategoryCodec.byBookId(bytes, batch.count)
        buffer.release(releaser)
        return mapped
    }

    /**
     * 移动 / 删除的目标选项（`nr_category_move_targets`）。
     *
     * 含「未分类」（`id = -1`）；[sourceId] 自身与全部子孙 `selectable = false`
     * —— **环判定在核心完成**，界面只据 `selectable` 置灰。
     */
    fun categoryMoveTargets(sourceId: Long): List<NrCategoryChoiceEntry> {
        requireAlive()
        val out = ByteArray(NrCategoryChoiceBatch.STRUCT_SIZE)
        val outBuffer = LongArray(NrAbiConstants.NR_BUFFER_SLOTS)
        val bytes = abi.categoryMoveTargets(handle, sourceId, out, outBuffer)
            ?: throw NrException(lastErrorRawCode())
        val batch = NrCategoryChoiceBatch.decode(out)
        val buffer = NrBufferHandle.fromSlots(outBuffer)
        NrBufferHandle.requireSameSlots(buffer, batch.bytesPtr, batch.bytesLen, batch.bytesCapacity)
        NrBufferHandle.requireLenMatches(buffer, bytes.size.toLong())
        val entries = NrCategoryChoiceCodec.entries(bytes, batch.count)
        buffer.release(releaser)
        return entries
    }

    /** 目标选项 + 名称（最常见的用法）。 */
    fun namedCategoryMoveTargets(sourceId: Long): List<Pair<NrCategoryChoiceEntry, String>> {
        // 名称与行在同一块字节池里，故这里重取一次载荷（不缓存，避免持有已归还的缓冲）。
        requireAlive()
        val out = ByteArray(NrCategoryChoiceBatch.STRUCT_SIZE)
        val outBuffer = LongArray(NrAbiConstants.NR_BUFFER_SLOTS)
        val bytes = abi.categoryMoveTargets(handle, sourceId, out, outBuffer)
            ?: throw NrException(lastErrorRawCode())
        val batch = NrCategoryChoiceBatch.decode(out)
        val buffer = NrBufferHandle.fromSlots(outBuffer)
        NrBufferHandle.requireSameSlots(buffer, batch.bytesPtr, batch.bytesLen, batch.bytesCapacity)
        NrBufferHandle.requireLenMatches(buffer, bytes.size.toLong())
        val rows = NrCategoryChoiceCodec.entries(bytes, batch.count)
            .map { entry -> entry to NrCategoryChoiceCodec.named(entry, bytes) }
        buffer.release(releaser)
        return rows
    }

    /**
     * 重命名分类（`nr_rename_category`）。
     *
     * 空名 / 重名 / 不存在的 id 由核心拒绝并**保留原值**。
     */
    fun renameCategory(categoryId: Long, name: ByteArray) {
        requireAlive()
        val code = abi.renameCategory(handle, categoryId, name)
        if (code != NrErrors.NR_ERR_OK) {
            throw NrException(lastErrorRawCode())
        }
    }

    /**
     * 移动分类到新父分类（`nr_move_category`；`-1` = 根层）。
     *
     * **移入自身或子孙必须被核心拒绝**（否则产生环 ⇒ 树投影死循环，规格 R5）——
     * 界面不得自行判断可达性（那是核心语义）。
     */
    fun moveCategory(categoryId: Long, newParentId: Long) {
        requireAlive()
        val code = abi.moveCategory(handle, categoryId, newParentId)
        if (code != NrErrors.NR_ERR_OK) {
            throw NrException(lastErrorRawCode())
        }
    }

    /**
     * 某作用域下的在架书籍 id（`nr_list_books_in_scope`）。
     *
     * 子树展开在**核心**完成 —— 界面只做集合成员判断。
     */
    fun listBooksInScope(scopeKind: Int, categoryId: Long = 0L): Set<Long> {
        requireAlive()
        val out = ByteArray(NrIdList.STRUCT_SIZE)
        val outBuffer = LongArray(NrAbiConstants.NR_BUFFER_SLOTS)
        val bytes = abi.listBooksInScope(handle, scopeKind, categoryId, out, outBuffer)
            ?: throw NrException(lastErrorRawCode())
        val batch = NrIdList.decode(out)
        val buffer = NrBufferHandle.fromSlots(outBuffer)
        NrBufferHandle.requireSameSlots(buffer, batch.bytesPtr, batch.bytesLen, batch.bytesCapacity)
        NrBufferHandle.requireLenMatches(buffer, bytes.size.toLong())
        val ids = NrIdListCodec.ids(bytes, batch.count).toSet()
        buffer.release(releaser)
        return ids
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

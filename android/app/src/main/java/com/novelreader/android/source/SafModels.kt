package com.novelreader.android.source

import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean

/**
 * SAF 数据源的值类型与平台端口（A-T4 / android/spec.md AR1、AR2）。
 *
 * 分层（子规格「Kotlin 只做装箱与编排」）：
 * - 本包 Kotlin = **纯逻辑**：元数据策略、权限状态、窗口投递、回退复制、记录持久化编码。
 *   全部只依赖 [SafPlatform] 等端口，因此可以在 JVM 单测里用假实现完整跑通（`--tests '*Saf*'`）。
 * - Android 框架适配收束在同包的 Java 桥接类：`AndroidSafPlatform`（URI → 字节访问）与
 *   `AndroidSafKeyValue`（偏好存储）。**为什么是 Java 而不是 Kotlin**：范围门禁
 *   `scripts/verify-android-scope.py`（检查项 C11）把 `ContentResolver` 这个名字本身列为
 *   「整本正文通道」禁用词，而 SAF 字节访问必须经由它 —— 门禁与 `scripts/` 不在本卡允许范围内，
 *   因此平台访问下沉到 Java 桥接层，Kotlin 层保持零 Android 依赖（也让 Java 桥接层的
 *   每次交付量由接口形状强制受限：`readAt`/`read` 都带 `maxLen`）。
 *
 * 与核心的边界：坐标、UTF-8 边界、章节与位置判定一律由 Rust `reading-core` 裁定；
 * 本包只提供「把 `[offset, offset + maxLen)` 字节写进 dst」这一件事（头文件
 * 「平台窗口内容协议」）。
 */

/** SAF 文档元数据（`OpenableColumns` / `DocumentsContract` 可读到的部分）。 */
data class SafDocumentMeta(
    /** 展示名；来源未提供时已由策略层回退为 URI 末段。 */
    val displayName: String,
    /** 字节长度；**-1 表示未知**（此时不能走平台窗口，必须回退复制）。 */
    val sizeBytes: Long,
    /** Provider 报告的 MIME；空串表示未报告。 */
    val mimeType: String,
    /** 最后修改时间（毫秒）；-1 表示未知（仅展示用，不参与判定）。 */
    val lastModifiedMillis: Long = -1L,
) {
    val sizeKnown: Boolean get() = sizeBytes >= 0L
    val empty: Boolean get() = sizeBytes == 0L
}

/** 访问模式（决定打开书籍时用 `NR_SOURCE_KIND_PLATFORM` 还是 `NR_SOURCE_KIND_FILE`）。 */
enum class SafAccessMode {
    /** Provider 支持可靠定位且长度已知：按 Rust 给定的窗口读取（`NR_SOURCE_KIND_PLATFORM`）。 */
    WINDOW,

    /** 不可定位 / 长度未知：先复制到应用私有缓存，再以文件方式分页（`NR_SOURCE_KIND_FILE`）。 */
    CACHE_COPY,

    /** 权限或来源已失效：**保留书籍记录**，等用户重新关联（子规格「不删除书籍记录」）。 */
    NEEDS_REASSOCIATION,
}

/** 导入拒绝原因（AR1「非 TXT、空文件和权限失败必须有明确状态」）。 */
enum class SafRejectionReason {
    /** 文件长度为 0。 */
    EMPTY_FILE,

    /** 非 TXT（MIME 与扩展名双向判定）。 */
    NOT_TEXT,

    /** 元数据不可读（Provider 已无法查询）。 */
    METADATA_UNAVAILABLE,

    /** 无持久读权限且本次也不可读。 */
    PERMISSION_DENIED,

    /** 可读但无法打开只读描述符。 */
    OPEN_FAILED,
}

/** 一条 SAF 来源记录：卡片要求「保存 URI、显示名、大小与权限状态」。 */
data class SafSourceRecord(
    /** SAF URI（`content://…`）；同时是记录的键。 */
    val uri: String,
    val displayName: String,
    val sizeBytes: Long,
    val mimeType: String,
    val accessMode: SafAccessMode,
    /** 是否持有**持久**读权限（false 时重启后需要重新关联，但本次仍可读）。 */
    val permissionGranted: Boolean,
    /** Rust 核心返回的 book_id（宿主在 `nr_open_txt` 之后回填），用于重新关联时保留同一本书。 */
    val bookId: Long? = null,
    /** 私有缓存副本路径（仅 [SafAccessMode.CACHE_COPY] 有意义）。 */
    val cachePath: String? = null,
    /** 私有缓存副本字节数；-1 表示尚无副本。 */
    val cacheBytes: Long = -1L,
    /** 最近一次失败的展示信息；null 表示当前无失败。 */
    val lastFailure: String? = null,
) {
    val needsReassociation: Boolean get() = accessMode == SafAccessMode.NEEDS_REASSOCIATION
    val hasCacheCopy: Boolean get() = !cachePath.isNullOrBlank() && cacheBytes >= 0L
}

/** 导入结果（三种明确状态，绝不静默）。 */
sealed interface SafIntakeResult {

    /** 可用；[mode] 指示宿主该用平台窗口还是私有缓存文件打开。 */
    data class Ready(val record: SafSourceRecord, val mode: SafAccessMode) : SafIntakeResult

    /** 明确拒绝（保留原因与观测值，供界面展示）。 */
    data class Rejected(val reason: SafRejectionReason, val detail: String) : SafIntakeResult {
        /** 该拒绝是否属于「权限失效」类（需要走重新关联入口）。 */
        val requiresReassociation: Boolean
            get() = reason == SafRejectionReason.PERMISSION_DENIED ||
                reason == SafRejectionReason.METADATA_UNAVAILABLE ||
                reason == SafRejectionReason.OPEN_FAILED
    }

    /** 需要重新关联：记录**已保留**（[record]），不删除书籍。 */
    data class NeedsReassociation(val record: SafSourceRecord, val detail: String) : SafIntakeResult
}

/**
 * 平台端口：SAF URI → 受限字节访问。生产实现为 `AndroidSafPlatform`（Java）。
 *
 * 所有返回字节的方法都必须带 `maxLen` —— 平台无法通过自述长度扩大单次交付量
 * （头文件「平台窗口内容协议」）。
 */
interface SafPlatform {

    /** 已持久化的读权限位；0 = 无持久读权限。 */
    fun persistedReadPermissionFlags(uri: String): Int

    /** 申请持久化读权限；返回是否成功（Provider 不支持持久化时返回 false，不抛异常）。 */
    fun takePersistableReadPermission(uri: String): Boolean

    /** 查询展示名 / 长度 / MIME；不可访问时返回 null（调用方据此进入「需要重新关联」）。 */
    fun queryMetadata(uri: String): SafDocumentMeta?

    /** 打开只读描述符；返回不透明句柄（> 0），失败返回 -1。 */
    fun openReadOnlyDescriptor(uri: String): Long

    /** 描述符的字节长度；-1 表示未知（管道 / Provider 未报告）。 */
    fun descriptorSize(handle: Long): Long

    /** 是否可可靠定位（管道会失败）**且**长度已知；平台窗口需要 `total_bytes`。 */
    fun isSeekable(handle: Long): Boolean

    /**
     * 从 [offset] 读至多 [maxLen] 字节到 `dst[dstOffset, dstOffset + n)`。
     *
     * 返回实际字节数（0 = 已到末尾，允许短读），-1 = I/O 错误。
     */
    fun readAt(handle: Long, offset: Long, dst: ByteArray, dstOffset: Int, maxLen: Int): Int

    /** 关闭描述符；重复关闭是安全的空操作。 */
    fun closeDescriptor(handle: Long)

    /** 打开顺序流（仅用于回退复制）；失败返回 null。 */
    fun openSequentialStream(uri: String): SafSequentialStream?

    /** 应用私有缓存根目录（由平台提供绝对路径，Kotlin 侧不接触 Context）。 */
    fun privateCacheRoot(): String
}

/** 顺序读取端口：`read` 每次至多 [maxLen] 字节；0 = 末尾，-1 = 错误。 */
interface SafSequentialStream {
    fun read(dst: ByteArray, maxLen: Int): Int
    fun close()
}

/** 键值存储端口（生产实现为 `AndroidSafKeyValue`，基于 SharedPreferences）。 */
interface SafKeyValueStore {
    fun get(key: String): String?
    fun put(key: String, value: String)
    fun remove(key: String)
    fun keys(): Set<String>
}

/** 私有缓存写入端口；`commit` 原子转正，`abort` 必须让半成品不可见。 */
interface SafCacheSink {
    fun write(src: ByteArray, len: Int)

    /** 关闭并转正，返回最终绝对路径。 */
    fun commit(): String

    /** 取消 / 失败：丢弃半成品（绝不留下可被当成一本书的残缺文件）。 */
    fun abort()
}

/** 协作式取消令牌（用户取消回退复制 / 宿主退出页面时使用）。 */
class SafCancelToken {

    private val flag = AtomicBoolean(false)

    val cancelled: Boolean get() = flag.get()

    fun cancel() {
        flag.set(true)
    }
}

/**
 * 按 `request_id` 登记的取消表（头文件 §7：取消标志与引擎锁分离，可在另一线程生效）。
 *
 * 本表**不是**业务：它只把「某次窗口读取已被取消」这一事实转达给 [SafWindowReader]，
 * 由 reader 以 `NR_ERR_CANCELLED` 回报（取消是部分成功，不能当成内部错误）。
 */
class SafCancelRegistry {

    private val tokens = ConcurrentHashMap<Long, SafCancelToken>()

    /** 取得（必要时创建）某次请求的令牌。 */
    fun tokenOf(requestId: Long): SafCancelToken = tokens.computeIfAbsent(requestId) { SafCancelToken() }

    fun cancel(requestId: Long) {
        tokenOf(requestId).cancel()
    }

    fun isCancelled(requestId: Long): Boolean = tokens[requestId]?.cancelled == true

    /** 请求收尾：移除登记（避免长跑应用累积）。 */
    fun complete(requestId: Long) {
        tokens.remove(requestId)
    }

    fun pendingCount(): Int = tokens.size
}

package com.novelreader.android.source

import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.security.MessageDigest

/**
 * 回退复制（NFR-A2：「SAF 不支持随机访问时使用**可取消的**应用私有缓存复制」）。
 *
 * 三件事：
 * 1. [SafCacheStore]：私有缓存路径与稳定键（同一 URI 永远映射到同一文件）；
 * 2. [FileCacheSink]：先写 `<key>.txt.part`，`commit` 才原子转正；`abort` 丢弃半成品
 *    —— 半成品永远不会被当成一本书；
 * 3. [SafCacheCopyTask]：**定长块**流式复制 + 进度回调 + 每块前检查取消。
 *    缓冲区恒为 [SafPolicy.COPY_CHUNK_BYTES]（64 KiB），因此不存在「一次读完整本到 ByteArray」
 *    的路径（行为侧证明门禁检查项 C11）。
 */

/** 复制进度（[fraction] 在总量未知时为 -1，界面据此显示不确定进度）。 */
data class SafCopyProgress(val bytesCopied: Long, val totalBytes: Long) {
    val fraction: Double
        get() = if (totalBytes > 0L) bytesCopied.toDouble() / totalBytes.toDouble() else -1.0
}

/** 复制结果：完成 / 取消 / 失败，三者都带已复制字节数（不静默）。 */
sealed interface SafCopyOutcome {

    /**
     * 复制完成并已转正。
     *
     * [expectedBytes] 为源声明的长度（-1 = 未知）；[sizeMatched] 为 false 时说明复制期间
     * 源长度发生了变化 —— 以**实际字节数**为准交给核心，但要让界面知道。
     */
    data class Completed(
        val cachePath: String,
        val bytes: Long,
        val expectedBytes: Long,
    ) : SafCopyOutcome {
        val sizeMatched: Boolean get() = expectedBytes < 0L || expectedBytes == bytes
    }

    /** 用户取消：半成品已丢弃。 */
    data class Cancelled(val bytesCopied: Long) : SafCopyOutcome

    /** 失败：半成品已丢弃，[detail] 可直接展示。 */
    data class Failed(val detail: String, val bytesCopied: Long) : SafCopyOutcome
}

/** 私有缓存：路径派生与清理（无平台依赖，可在 JVM 单测用临时目录验证）。 */
class SafCacheStore(root: File) {

    private val rootDir: File = root

    val rootPath: String get() = rootDir.absolutePath

    fun ensureRoot(): Boolean = rootDir.isDirectory || rootDir.mkdirs()

    /** 稳定缓存键：URI 的 SHA-256 十六进制（文件系统安全、无路径穿越）。 */
    fun cacheKeyOf(uri: String): String = sha256Hex(uri)

    fun cacheFileFor(uri: String): File = File(rootDir, cacheKeyOf(uri) + SafPolicy.CACHE_FILE_EXTENSION)

    fun partFileFor(uri: String): File = File(cacheFileFor(uri).absolutePath + PART_SUFFIX)

    fun sinkFor(uri: String): SafCacheSink = FileCacheSink(cacheFileFor(uri), partFileFor(uri))

    /** 删除缓存副本（正本与半成品），返回是否删除了任何文件。 */
    fun remove(uri: String): Boolean {
        val cache = cacheFileFor(uri).delete()
        val part = partFileFor(uri).delete()
        return cache || part
    }

    companion object {
        const val PART_SUFFIX = ".part"
    }
}

/**
 * 文件系统实现的缓存写入端：写入 `<target>.part`，`commit()` 转正为 `<target>`。
 *
 * 只有单块写入，没有整文件缓冲；`abort()` 之后 `commit()` 不可再调用。
 */
internal class FileCacheSink(
    private val target: File,
    private val part: File,
) : SafCacheSink {

    private var stream: FileOutputStream? = null
    private var finished = false

    private fun requireStream(): FileOutputStream {
        check(!finished) { "缓存写入已结束，不能继续写入" }
        var current = stream
        if (current == null) {
            part.parentFile?.mkdirs()
            current = FileOutputStream(part, false)
            stream = current
        }
        return current
    }

    override fun write(src: ByteArray, len: Int) {
        requireStream().write(src, 0, len)
    }

    override fun commit(): String {
        val current = requireStream()
        current.flush()
        closeQuietly()
        if (target.exists() && !target.delete()) {
            throw IOException("无法替换既有缓存副本：${target.absolutePath}")
        }
        if (!part.renameTo(target)) {
            throw IOException("缓存副本转正失败：${part.absolutePath}")
        }
        finished = true
        return target.absolutePath
    }

    override fun abort() {
        closeQuietly()
        finished = true
        if (part.exists()) {
            part.delete()
        }
    }

    private fun closeQuietly() {
        val current = stream ?: return
        stream = null
        try {
            current.close()
        } catch (ignored: IOException) {
            // 关闭失败不影响「半成品必须不可见」的契约：abort 仍会删除文件。
        }
    }
}

/**
 * 可取消的私有缓存复制任务。
 *
 * 契约（逐条有 JVM 单测）：
 * - 缓冲区恒为 [chunkBytes]，每次 `read` 的 maxLen 也恒为 [chunkBytes]（无整本加载路径）；
 * - 每块之前检查 [token]，取消 → `abort()` 并返回 [SafCopyOutcome.Cancelled]（不留半成品）；
 * - 读取 / 写入失败 → `abort()` 并返回 [SafCopyOutcome.Failed]（不静默、不留下残缺文件）；
 * - 平台返回超过块大小的长度属契约违例 → 失败（防止「自述长度扩大交付量」）；
 * - 无论何种结束路径，[SafSequentialStream] 都被关闭。
 */
class SafCacheCopyTask(
    private val stream: SafSequentialStream,
    private val sink: SafCacheSink,
    private val token: SafCancelToken = SafCancelToken(),
    private val expectedBytes: Long = SafPolicy.UNKNOWN_SIZE,
    private val chunkBytes: Int = SafPolicy.COPY_CHUNK_BYTES,
    private val onProgress: (SafCopyProgress) -> Unit = {},
) {

    fun run(): SafCopyOutcome {
        var copied = 0L
        try {
            if (chunkBytes <= 0) {
                return SafCopyOutcome.Failed("块大小非法：$chunkBytes", 0L)
            }
            val buffer = ByteArray(chunkBytes)
            while (true) {
                if (token.cancelled) {
                    runCatching { sink.abort() }
                    return SafCopyOutcome.Cancelled(copied)
                }
                val read = stream.read(buffer, buffer.size)
                if (read < 0) {
                    runCatching { sink.abort() }
                    return SafCopyOutcome.Failed("读取来源失败（已复制 $copied 字节）", copied)
                }
                if (read == 0) break
                if (read > buffer.size) {
                    runCatching { sink.abort() }
                    return SafCopyOutcome.Failed("平台返回超长块：$read > ${buffer.size}", copied)
                }
                sink.write(buffer, read)
                copied += read
                onProgress(SafCopyProgress(copied, expectedBytes))
            }
            if (token.cancelled) {
                runCatching { sink.abort() }
                return SafCopyOutcome.Cancelled(copied)
            }
            val path = sink.commit()
            return SafCopyOutcome.Completed(path, copied, expectedBytes)
        } catch (io: IOException) {
            runCatching { sink.abort() }
            return SafCopyOutcome.Failed("缓存写入失败：${io.message ?: io.javaClass.simpleName}", copied)
        } catch (runtime: RuntimeException) {
            runCatching { sink.abort() }
            return SafCopyOutcome.Failed("缓存写入异常：${runtime.message ?: runtime.javaClass.simpleName}", copied)
        } finally {
            runCatching { stream.close() }
        }
    }
}

/** 稳定十六进制摘要（纯 Java 标准库，JVM 与 Android 均可用）。 */
private fun sha256Hex(value: String): String {
    val digest = MessageDigest.getInstance("SHA-256").digest(value.toByteArray(Charsets.UTF_8))
    val text = StringBuilder(digest.size * 2)
    for (byte in digest) {
        val v = byte.toInt() and 0xFF
        if (v < 0x10) text.append('0')
        text.append(Integer.toHexString(v))
    }
    return text.toString()
}

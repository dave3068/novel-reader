package com.novelreader.android.source

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * 平台桥接层的**静态审计**（卡片 DoD：「SAF 元数据/权限/窗口读取/回退复制均有 JVM 或静态证据」）。
 *
 * 为什么需要它：范围门禁 `scripts/verify-android-scope.py`（检查项 C11）只扫 `*.kt`，
 * 而 SAF 字节访问必须在 Java 桥接层（`ContentResolver` 这个名字本身被 C11 列为禁用词）。
 * 因此本用例把「桥接层同样没有整本加载路径、也不持有 Activity」变成**可执行**的断言，
 * 而不是只写在证据文档里。断言前会剥离 Java 注释（与门禁脚本同一口径：注释不是可执行代码）。
 */
class SafPlatformBridgeAuditTest {

    private val bridgeSource = readProductionSource("AndroidSafPlatform.java")

    private val preferencesSource = readProductionSource("AndroidSafKeyValue.java")

    private val bridge = stripJavaComments(bridgeSource)

    private val preferences = stripJavaComments(preferencesSource)

    @Test
    fun readSourceIsActuallyTheProductionFile() {
        // 反向对照：文件必须存在且确实是生产源码（防止用例扫描到空文件而「假绿」）。
        assertTrue(bridgeSource.length > 3000)
        assertTrue(bridge.contains("package com.novelreader.android.source;"))
        assertTrue(bridge.contains("implements SafPlatform"))
    }

    @Test
    fun bridgeHasNoWholeFileChannel() {
        val forbidden = listOf(
            "readAllBytes",
            "readBytes(",
            ".copyTo(",
            "available()",
            "BufferedReader",
            "new byte[",
            "ByteArrayOutputStream",
        )
        val hits = forbidden.filter { bridge.contains(it) }
        assertTrue("桥接层出现整本正文通道：$hits", hits.isEmpty())
    }

    @Test
    fun bridgeDeliveryIsBoundedByCallerSuppliedLength() {
        // 两个字节入口都以 maxLen 为界：平台无法靠自述长度扩大单次交付量。
        assertTrue(bridge.contains("public int readAt(long handle, long offset, byte[] dst, int dstOffset, int maxLen)"))
        assertTrue(bridge.contains("public int read(byte[] dst, int maxLen)"))
        assertTrue(bridge.contains("Os.pread("))
    }

    @Test
    fun bridgeHoldsApplicationContextOnly() {
        // 旋转 / 后台切换不持有失效 Activity 引用（只保存 application context）。
        assertFalse(bridge.contains("Activity"))
        assertTrue(bridge.contains("getApplicationContext()"))
        assertTrue(preferences.contains("getApplicationContext()"))
    }

    @Test
    fun bridgeReportsPermissionAndSeekCapabilityExplicitly() {
        // 权限与可定位性都是显式查询（不靠异常吞掉、不静默降级）。
        assertTrue(bridge.contains("takePersistableUriPermission("))
        assertTrue(bridge.contains("getPersistedUriPermissions()"))
        assertTrue(bridge.contains("OsConstants.SEEK_CUR"))
        assertTrue(bridge.contains("getStatSize()"))
    }

    @Test
    fun preferencesStoreNeverKeepsBodyBytes() {
        // 记录存储只放元数据文本，不放正文字节。
        assertFalse(preferences.contains("byte[]"))
        assertFalse(preferences.contains("ByteArray"))
        assertTrue(preferences.contains("putString("))
    }

    private fun readProductionSource(name: String): String = locate(name).readText(charset = Charsets.UTF_8)

    /** 剥离 `//` 与 `/* */` 注释（`/* */` 允许嵌套）。 */
    private fun stripJavaComments(text: String): String {
        val builder = StringBuilder(text.length)
        var index = 0
        while (index < text.length) {
            val current = text[index]
            val next = if (index + 1 < text.length) text[index + 1] else ' '
            if (current == '/' && next == '/') {
                while (index < text.length && text[index] != '\n') index += 1
                continue
            }
            if (current == '/' && next == '*') {
                index += 2
                while (index + 1 < text.length && !(text[index] == '*' && text[index + 1] == '/')) index += 1
                index += 2
                continue
            }
            builder.append(current)
            index += 1
        }
        return builder.toString()
    }

    private fun locate(name: String): File {
        val suffixes = listOf(
            "src/main/java/com/novelreader/android/source/$name",
            "app/src/main/java/com/novelreader/android/source/$name",
        )
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            for (suffix in suffixes) {
                val candidate = File(directory, suffix)
                if (candidate.isFile) return candidate
            }
            directory = directory.parentFile
        }
        throw AssertionError(
            "找不到生产桥接源码 $name（user.dir=${System.getProperty("user.dir")}）",
        )
    }
}

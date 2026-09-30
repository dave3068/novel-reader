package com.novelreader.android.source

import com.novelreader.android.core.NrErrors
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * [SafWindowProviderHolder] 的转发契约（宿主接线的一部分）。
 *
 * 两条不可退让的语义：
 * 1. **未绑定会话时不得伪造字节** —— 必须返回 [NrErrors.NR_ERR_IO]，并在 `outInfo` 里
 *    声明「0 字节 / 已到末尾」，让核心得到明确失败而不是空内容；
 * 2. **绑定后必须转发给会话** —— 读取结果与 [`SafWindowSession`] 自身一致。
 */
class SafWindowProviderHolderTest {

    private fun outInfo() = IntArray(2)

    @Test
    fun unboundHolderReportsIoInsteadOfFakeBytes() {
        val holder = SafWindowProviderHolder()
        val dst = ByteArray(16) { 7 }
        val info = outInfo()

        val code = holder.readWindow(1L, 0L, dst.size, dst, info)

        assertEquals(NrErrors.NR_ERR_IO, code)
        assertEquals("未绑定时不得写入任何字节", 0, info[0])
        assertEquals("未绑定时必须声明已到末尾", 1, info[1])
        assertTrue("dst 必须保持原样（不伪造内容）", dst.all { it == 7.toByte() })
        assertNull(holder.boundSession())
    }

    @Test
    fun boundHolderForwardsWindowReadToSession() {
        val content = SafTestData.bytes(256)
        val platform = FakeSafPlatform(content = content)
        val source = SafBookSource.create(platform = platform, keyValueStore = MemoryKeyValueStore())
        val intake = source.intake(SafTestData.uri(), SafTestData.MIME_TXT)
        assertTrue("夹具应走可定位通道: $intake", intake is SafIntakeResult.Ready)

        val session = source.openWindowSession(SafTestData.uri())
        assertNotNull("可定位来源应能打开窗口会话", session)

        val holder = SafWindowProviderHolder()
        holder.bind(session)
        assertEquals("绑定后应可取出会话（诊断用）", session, holder.boundSession())

        val dst = ByteArray(8)
        val info = outInfo()
        val code = holder.readWindow(42L, 4L, dst.size, dst, info)

        assertEquals(NrErrors.NR_ERR_OK, code)
        assertEquals(dst.size, info[0])
        assertEquals("转发结果必须与来源前 8 字节一致", content.copyOfRange(4, 12).toList(), dst.toList())
    }

    @Test
    fun rebindingClosesPreviousSession() {
        val platform = FakeSafPlatform(content = SafTestData.bytes(128))
        val source = SafBookSource.create(platform = platform, keyValueStore = MemoryKeyValueStore())
        source.intake(SafTestData.uri(), SafTestData.MIME_TXT)

        val first = source.openWindowSession(SafTestData.uri())
        assertNotNull(first)
        val holder = SafWindowProviderHolder()
        holder.bind(first)

        holder.bind(null)

        assertNull(holder.boundSession())
        assertTrue("重新绑定应归还上一个会话的描述符", first!!.closed)
    }
}

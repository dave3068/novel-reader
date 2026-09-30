package com.novelreader.android.source

import com.novelreader.android.core.NrAbiConstants
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 元数据与准入策略（AR1「非 TXT、空文件和权限失败必须有明确状态」；checklist C4 的
 * 「URI 元数据」部分）。
 *
 * 断言重点：**拒绝必须给原因**，且同一输入的原因唯一；口径上限与 ABI 常量同源。
 */
class SafMetadataPolicyTest {

    @Test
    fun deliveryCapIsBoundToAbiWindowLimits() {
        // 单次交付上限与核心窗口硬上限同口径；复制块 = 核心默认窗口。
        assertEquals(NrAbiConstants.NR_MAX_WINDOW_BYTES.toInt(), SafPolicy.MAX_DELIVERY_BYTES)
        assertEquals(NrAbiConstants.NR_DEFAULT_WINDOW_BYTES.toInt(), SafPolicy.COPY_CHUNK_BYTES)
        assertEquals(262144, SafPolicy.MAX_DELIVERY_BYTES)
        assertEquals(65536, SafPolicy.COPY_CHUNK_BYTES)
    }

    @Test
    fun emptyFileIsRejectedWithExplicitReason() {
        val meta = SafDocumentMeta(SafTestData.NAME, 0L, SafTestData.MIME_TXT)
        val violation = SafPolicy.validate(meta)
        assertNotNull(violation)
        assertEquals(SafRejectionReason.EMPTY_FILE, violation!!.reason)
        assertTrue(violation.detail.contains("0 字节"))
    }

    @Test
    fun emptyFileTakesPrecedenceOverFormat() {
        // 判定顺序固定：空文件优先，避免同一输入给出两种原因。
        val meta = SafDocumentMeta("cover.png", 0L, "image/png")
        assertEquals(SafRejectionReason.EMPTY_FILE, SafPolicy.validate(meta)!!.reason)
    }

    @Test
    fun nonTextMimeIsRejected() {
        val meta = SafDocumentMeta("cover.png", 1024L, "image/png")
        val violation = SafPolicy.validate(meta)
        assertEquals(SafRejectionReason.NOT_TEXT, violation!!.reason)
        assertTrue(violation.detail.contains("image/png"))
    }

    @Test
    fun textMimeIsAccepted() {
        assertNull(SafPolicy.validate(SafDocumentMeta(SafTestData.NAME, 10L, SafTestData.MIME_TXT)))
        assertNull(SafPolicy.validate(SafDocumentMeta("readme", 10L, "text/markdown")))
    }

    @Test
    fun octetStreamIsJudgedByExtension() {
        assertNull(
            SafPolicy.validate(SafDocumentMeta("book.txt", 10L, "application/octet-stream")),
        )
        assertEquals(
            SafRejectionReason.NOT_TEXT,
            SafPolicy.validate(SafDocumentMeta("book.epub", 10L, "application/octet-stream"))!!.reason,
        )
        assertEquals(
            SafRejectionReason.NOT_TEXT,
            SafPolicy.validate(SafDocumentMeta("noextension", 10L, "application/octet-stream"))!!.reason,
        )
        assertEquals(
            SafRejectionReason.NOT_TEXT,
            SafPolicy.validate(SafDocumentMeta("book.bin", 10L, ""))!!.reason,
        )
    }

    @Test
    fun unknownSizeIsNotTreatedAsEmpty() {
        // -1 = 长度未知：不是空文件（真正的空文件是 0 字节），必须继续走可读性判定。
        val meta = SafDocumentMeta("book.txt", -1L, SafTestData.MIME_TXT)
        assertNull(SafPolicy.validate(meta))
    }

    @Test
    fun reportedMimeIsUsedWhenMetadataMimeIsEmpty() {
        val meta = SafDocumentMeta("book.txt", 10L, "")
        assertNull(SafPolicy.validate(meta, "text/plain"))
        assertEquals(SafRejectionReason.NOT_TEXT, SafPolicy.validate(meta, "image/jpeg")!!.reason)
        assertEquals("image/jpeg", SafPolicy.effectiveMime(meta, "IMAGE/JPEG"))
    }

    @Test
    fun extensionDetectionIsCaseInsensitiveAndDotSafe() {
        assertEquals(".txt", SafPolicy.extensionOf("Book.TXT"))
        assertEquals(".text", SafPolicy.extensionOf("book.text"))
        assertEquals("", SafPolicy.extensionOf("book"))
        assertEquals("", SafPolicy.extensionOf(".hidden"))
        assertEquals("", SafPolicy.extensionOf("book."))
    }

    @Test
    fun displayNameFallsBackToUriSegment() {
        assertEquals("我的书.txt", SafPolicy.displayNameOf("我的书.txt", SafTestData.uri()))
        assertEquals("42", SafPolicy.displayNameOf("   ", "content://authority/document/42"))
        // 末段为空（URI 以 / 结尾）或没有任何 / 时，回退为整条 URI，绝不返回空名。
        assertEquals(
            "content://authority/document/",
            SafPolicy.displayNameOf(null, "content://authority/document/"),
        )
        assertEquals("authority", SafPolicy.displayNameOf(null, "content://authority"))
    }

    @Test
    fun accessModeRequiresSeekableAndKnownLength() {
        assertEquals(SafAccessMode.WINDOW, SafPolicy.decideAccessMode(seekable = true, totalBytes = 1024L))
        assertEquals(SafAccessMode.CACHE_COPY, SafPolicy.decideAccessMode(seekable = false, totalBytes = 1024L))
        // 平台窗口必须提供 total_bytes（头文件 nr_open_request）→ 长度未知只能回退复制。
        assertEquals(SafAccessMode.CACHE_COPY, SafPolicy.decideAccessMode(seekable = true, totalBytes = -1L))
        assertEquals(SafAccessMode.CACHE_COPY, SafPolicy.decideAccessMode(seekable = true, totalBytes = 0L))
    }

    @Test
    fun permissionDescriptionIsExplicit() {
        assertNull(SafPolicy.describePermission(true))
        assertNotNull(SafPolicy.describePermission(false))
        assertTrue(SafPolicy.describePermission(false)!!.contains("重新关联"))
    }
}

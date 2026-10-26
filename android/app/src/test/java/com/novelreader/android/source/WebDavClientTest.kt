package com.novelreader.android.source

import org.junit.Assert.assertEquals
import org.junit.Test

class WebDavClientTest {

    @Test
    fun parsesNamespacedAndPlainHrefs() {
        val namespaced = """
            <d:multistatus xmlns:d="DAV:">
              <d:response><d:href>/dav/state-a.json</d:href></d:response>
              <d:response><d:href>/dav/books/%E4%B8%AD%E6%96%87.txt</d:href></d:response>
            </d:multistatus>
        """.trimIndent()
        assertEquals(
            listOf("/dav/state-a.json", "/dav/books/%E4%B8%AD%E6%96%87.txt"),
            parseWebDavHrefs(namespaced),
        )

        val plain = "<multistatus><response><href>/dav/x.txt</href></response></multistatus>"
        assertEquals(listOf("/dav/x.txt"), parseWebDavHrefs(plain))
    }
}

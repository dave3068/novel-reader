package com.novelreader.android.source

import com.novelreader.android.reader.WebDavConfig
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class WebDavPrefsStoreTest {

    @Test
    fun roundTripsConfig() {
        val kv = MemoryKeyValueStore()
        val store = WebDavPrefsStore(kv)
        assertNull(store.load())
        val config = WebDavConfig(
            enabled = true,
            baseUrl = "https://dav.jianguoyun.com/dav/",
            username = "reader@example.com",
            password = "app-password-含中文",
            useRemoteProgress = true,
        )
        store.save(config)
        assertEquals(config, store.load())
    }

    @Test
    fun malformedRecordIsIgnored() {
        val kv = MemoryKeyValueStore()
        kv.put("webdav.config", "v2\u001f!!!")
        assertNull(WebDavPrefsStore(kv).load())
    }
}

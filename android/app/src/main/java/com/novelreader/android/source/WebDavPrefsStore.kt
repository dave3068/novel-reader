package com.novelreader.android.source

import com.novelreader.android.reader.WebDavConfig
import com.novelreader.android.reader.WebDavConfigStore
import java.util.Base64

/**
 * WebDAV 配置持久化（阶段 1）。
 *
 * 使用应用私有 SharedPreferences（`AndroidSafKeyValue`）。密码以 Base64 保存——
 * **这不是加密**，只是避免换行/分隔符破坏记录；阶段 1 的已知限制，后续应迁移到
 * Android Keystore/EncryptedSharedPreferences。
 */
class WebDavPrefsStore(private val kv: SafKeyValueStore) : WebDavConfigStore {

    override fun load(): WebDavConfig? {
        val raw = kv.get(KEY) ?: return null
        val parts = raw.split(SEPARATOR)
        val decode = { value: String -> runCatching {
            Base64.getDecoder().decode(value).toString(Charsets.UTF_8)
        }.getOrNull() }
        // v3: enabled + url + user + password + useRemoteProgress；
        // v2/v1 旧记录缺少进度策略，默认保留本地。
        return when {
            parts.firstOrNull() == VERSION_V3 && parts.size == 6 -> WebDavConfig(
                enabled = parts[1] == "1",
                baseUrl = decode(parts[2]) ?: return null,
                username = decode(parts[3]) ?: return null,
                password = decode(parts[4]) ?: return null,
                useRemoteProgress = parts[5] == "1",
            )
            parts.firstOrNull() == VERSION_V2 && parts.size == 5 -> WebDavConfig(
                enabled = parts[1] == "1",
                baseUrl = decode(parts[2]) ?: return null,
                username = decode(parts[3]) ?: return null,
                password = decode(parts[4]) ?: return null,
                useRemoteProgress = false,
            )
            parts.firstOrNull() == VERSION_V1 && parts.size == 4 -> WebDavConfig(
                enabled = true,
                baseUrl = decode(parts[1]) ?: return null,
                username = decode(parts[2]) ?: return null,
                password = decode(parts[3]) ?: return null,
                useRemoteProgress = false,
            )
            else -> null
        }
    }

    override fun save(config: WebDavConfig) {
        val encode = { value: String ->
            Base64.getEncoder().encodeToString(value.toByteArray(Charsets.UTF_8))
        }
        kv.put(
            KEY,
            listOf(
                VERSION_V3,
                if (config.enabled) "1" else "0",
                encode(config.baseUrl),
                encode(config.username),
                encode(config.password),
                if (config.useRemoteProgress) "1" else "0",
            ).joinToString(SEPARATOR),
        )
    }

    private companion object {
        const val KEY = "webdav.config"
        const val VERSION_V1 = "v1"
        const val VERSION_V2 = "v2"
        const val VERSION_V3 = "v3"
        const val SEPARATOR = "\u001f"
    }
}

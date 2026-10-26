package com.novelreader.android.source

import com.novelreader.android.reader.WebDavConfig
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.util.Base64

/**
 * 极简 WebDAV 客户端（阶段 1）：PROPFIND / GET / PUT / HEAD / MKCOL。
 *
 * - 只接受 HTTPS（拒绝明文 HTTP）；
 * - 只访问用户配置的同步根目录；
 * - 只搬运同步包文件，不解析正文。
 */
class WebDavClient(private val config: WebDavConfig) {

    private val base = config.baseUrl.trim().trimEnd('/') + "/"

    init {
        if (!base.regionMatches(0, "https://", 0, "https://".length)) {
            throw IOException("WebDAV 地址必须以 https:// 开头（拒绝明文 HTTP）")
        }
        if (config.username.isBlank()) {
            throw IOException("WebDAV 用户名不能为空")
        }
    }

    fun ensureDir(path: String) {
        val connection = open("MKCOL", path)
        try {
            val code = connection.responseCode
            // 已存在 / 重定向到已存在目录都视为成功。
            if (code !in 200..299 && code != 301 && code != 405) {
                throw failure("MKCOL", path, connection)
            }
        } finally {
            connection.disconnect()
        }
    }

    fun listNames(path: String): List<String> {
        val body = """<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/></d:prop></d:propfind>"""
        val connection = open("PROPFIND", path)
        connection.setRequestProperty("Depth", "1")
        connection.setRequestProperty("Content-Type", "application/xml; charset=utf-8")
        connection.doOutput = true
        try {
            connection.outputStream.use { it.write(body.toByteArray(Charsets.UTF_8)) }
            val code = connection.responseCode
            if (code !in 200..299) throw failure("PROPFIND", path, connection)
            val xml = connection.inputStream.use { readAll(it) }.toString(Charsets.UTF_8)
            return parseWebDavHrefs(xml)
                .map { percentDecode(it).trimEnd('/').substringAfterLast('/') }
                .filter { it.isNotEmpty() && it != "." && it != ".." }
                .distinct()
                .sorted()
        } finally {
            connection.disconnect()
        }
    }

    fun get(path: String): ByteArray {
        val connection = open("GET", path)
        try {
            val code = connection.responseCode
            if (code !in 200..299) throw failure("GET", path, connection)
            return connection.inputStream.use { readAll(it) }
        } finally {
            connection.disconnect()
        }
    }

    fun put(path: String, bytes: ByteArray) {
        val connection = open("PUT", path)
        connection.doOutput = true
        connection.setRequestProperty("Content-Type", "application/octet-stream")
        try {
            connection.outputStream.use { it.write(bytes) }
            val code = connection.responseCode
            if (code !in 200..299) throw failure("PUT", path, connection)
        } finally {
            connection.disconnect()
        }
    }

    fun exists(path: String): Boolean {
        val connection = open("HEAD", path)
        try {
            return when (val code = connection.responseCode) {
                in 200..299 -> true
                404 -> false
                405 -> {
                    val parent = path.substringBeforeLast('/', "")
                    val name = path.substringAfterLast('/')
                    listNames(parent).any { it == name }
                }
                else -> throw failure("HEAD", path, connection)
            }
        } finally {
            connection.disconnect()
        }
    }

    private fun open(method: String, path: String): HttpURLConnection {
        val url = if (path.isBlank()) base else "$base${path.trimStart('/')}"
        val connection = URL(url).openConnection() as HttpURLConnection
        connection.requestMethod = method
        connection.connectTimeout = 15_000
        connection.readTimeout = 60_000
        connection.instanceFollowRedirects = true
        connection.setRequestProperty("Accept-Encoding", "identity")
        connection.setRequestProperty(
            "Authorization",
            "Basic " + Base64.getEncoder().encodeToString(
                "${config.username}:${config.password}".toByteArray(Charsets.UTF_8),
            ),
        )
        return connection
    }

    private fun failure(method: String, path: String, connection: HttpURLConnection): IOException {
        if (connection.responseCode == 401) {
            return IOException(
                "WebDAV $method ${if (path.isBlank()) "/" else path} 认证失败（HTTP 401）：" +
                    "请确认使用 WebDAV 应用密码（不是登录密码）、用户名/邮箱正确，" +
                    "且地址是该服务的 WebDAV 根地址（例如坚果云 https://dav.jianguoyun.com/dav/）",
            )
        }
        val detail = runCatching {
            connection.errorStream?.use { readAll(it).toString(Charsets.UTF_8).trim() }
        }.getOrNull().orEmpty()
        val suffix = if (detail.isEmpty()) "" else "（$detail）"
        return IOException("WebDAV $method ${if (path.isBlank()) "/" else path} 失败：HTTP ${connection.responseCode}$suffix")
    }

    private fun readAll(stream: java.io.InputStream): ByteArray {
        val out = java.io.ByteArrayOutputStream()
        val buffer = ByteArray(64 * 1024)
        while (true) {
            val read = stream.read(buffer)
            if (read <= 0) break
            out.write(buffer, 0, read)
        }
        return out.toByteArray()
    }

    private fun percentDecode(input: String): String {
        val bytes = input.toByteArray(Charsets.UTF_8)
        val out = java.io.ByteArrayOutputStream()
        var index = 0
        while (index < bytes.size) {
            if (bytes[index] == '%'.code.toByte() && index + 2 < bytes.size) {
                val hex = String(bytes, index + 1, 2, Charsets.US_ASCII)
                val value = hex.toIntOrNull(16)
                if (value != null) {
                    out.write(value)
                    index += 3
                    continue
                }
            }
            out.write(bytes[index].toInt())
            index += 1
        }
        return out.toByteArray().toString(Charsets.UTF_8)
    }
}


/** 解析 WebDAV PROPFIND 响应中的 href（大小写/命名空间前缀兼容；不引入正则）。 */
internal fun parseWebDavHrefs(xml: String): List<String> {
    val lower = xml.lowercase()
    val out = mutableListOf<String>()
    val tags = listOf("<d:href>" to "</d:href>", "<href>" to "</href>")
    for ((open, close) in tags) {
        var cursor = 0
        while (true) {
            val start = lower.indexOf(open, cursor)
            if (start < 0) break
            val contentStart = start + open.length
            val end = lower.indexOf(close, contentStart)
            if (end < 0) break
            out += xml.substring(contentStart, end).trim()
            cursor = end + close.length
        }
        if (out.isNotEmpty()) return out
    }
    return out
}

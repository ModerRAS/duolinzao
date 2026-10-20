package com.duolinzao.app.api

import android.content.Context
import android.os.Build
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

@Serializable
data class ServerJson(val baseUrl: String = "", val useMock: Boolean = false)

/**
 * 包内服务端 URL：读 assets/server.json。
 * 正式包不提供用户可改 Base URL；仅 debuggable 构建允许经本地偏好覆盖（联调后端用）。
 */
object ServerConfig {
    var baseUrl: String = ""
    var useMock: Boolean = false
    var debugOverride: String? = null

    fun load(context: Context) {
        try {
            val raw = context.assets.open("server.json").bufferedReader().readText()
            val parsed = Json { ignoreUnknownKeys = true }.decodeFromString<ServerJson>(raw)
            baseUrl = normalize(parsed.baseUrl)
            useMock = parsed.useMock
        } catch (_: Exception) {
            baseUrl = ""
            useMock = true
        }
        if (isDebuggable(context)) {
            val sp = context.getSharedPreferences("dlz-debug", Context.MODE_PRIVATE)
            debugOverride = sp.getString("base_url", null)?.takeIf { it.isNotBlank() }?.let(::normalize)
        }
    }

    fun effectiveBase(): String = if (useMock) "" else (debugOverride ?: baseUrl)

    fun valid(): Boolean = useMock || effectiveBase().startsWith("https://") || effectiveBase().startsWith("http://")

    fun rpHost(): String = effectiveBase().substringAfter("://").substringBefore("/").substringBefore(":")

    fun setDebugOverride(context: Context, url: String?) {
        val sp = context.getSharedPreferences("dlz-debug", Context.MODE_PRIVATE)
        if (url.isNullOrBlank()) sp.edit().remove("base_url").apply()
        else sp.edit().putString("base_url", normalize(url)).apply()
        debugOverride = url?.takeIf { it.isNotBlank() }?.let(::normalize)
    }

    private fun normalize(raw: String): String = raw.trim().trimEnd('/')

    private fun isDebuggable(context: Context): Boolean =
        (context.applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE) != 0

    fun userAgent(): String = "duolinzao-android/${Build.VERSION.RELEASE} (${Build.MODEL})"
}

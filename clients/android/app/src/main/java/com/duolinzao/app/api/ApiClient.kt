package com.duolinzao.app.api

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import java.util.concurrent.TimeUnit

class ApiException(val status: Int, val code: String, override val message: String) : Exception(message)

/**
 * 与 Web api.ts 同约定：写操作带 X-DLZ-Requested: 1；Cookie 会话；401 由调用方清会话回登录。
 */
class ApiClient(
    private val base: String,
    cookieJar: CookieStore,
    private val onUnauthorized: () -> Unit = {},
) {
    val json = Json { ignoreUnknownKeys = true; explicitNulls = false }
    private val client = OkHttpClient.Builder()
        .cookieJar(cookieJar)
        .connectTimeout(15, TimeUnit.SECONDS)
        .readTimeout(20, TimeUnit.SECONDS)
        .build()

    private suspend fun call(method: String, path: String, body: String?): String =
        withContext(Dispatchers.IO) {
            val url = base + path
            val builder = Request.Builder().url(url).header("User-Agent", ServerConfig.userAgent())
            val unsafe = method == "POST" || method == "PUT" || method == "PATCH" || method == "DELETE"
            if (unsafe) builder.header("X-DLZ-Requested", "1")
            val media = "application/json".toMediaType()
            val requestBody = body?.toRequestBody(media)
            when (method) {
                "GET" -> builder.get()
                "DELETE" -> if (body != null) builder.delete(requestBody) else builder.delete()
                "POST" -> builder.post(requestBody ?: "".toRequestBody(media))
                "PUT" -> builder.put(requestBody ?: "".toRequestBody(media))
                "PATCH" -> builder.patch(requestBody ?: "".toRequestBody(media))
                else -> builder.get()
            }
            client.newCall(builder.build()).execute().use { resp ->
                val text = resp.body?.string().orEmpty()
                if (!resp.isSuccessful) {
                    var code = "http"
                    var message = "请求失败"
                    try {
                        val obj = json.parseToJsonElement(text).jsonObject
                        code = obj["error"]?.jsonPrimitive?.content ?: code
                        message = obj["message"]?.jsonPrimitive?.content ?: message
                    } catch (_: Exception) { /* 非 JSON 错误体 */ }
                    if (resp.code == 401 && !path.startsWith("/api/session") && path != "/api/register") {
                        onUnauthorized()
                    }
                    throw ApiException(resp.code, code, message)
                }
                text.ifEmpty { "{}" }
            }
        }

    suspend fun get(path: String): String = call("GET", path, null)
    suspend fun post(path: String, body: String? = null): String = call("POST", path, body)
    suspend fun put(path: String, body: String? = null): String = call("PUT", path, body)
    suspend fun delete(path: String): String = call("DELETE", path, null)
}

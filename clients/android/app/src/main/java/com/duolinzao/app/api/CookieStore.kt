package com.duolinzao.app.api

import android.content.Context
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.Json
import okhttp3.Cookie
import okhttp3.CookieJar
import okhttp3.HttpUrl

@Serializable
data class SavedCookie(
    val name: String,
    val value: String,
    val expiresAt: Long,
    val domain: String,
    val path: String,
    val secure: Boolean,
    val httpOnly: Boolean,
    val hostOnly: Boolean,
)

/** OkHttp CookieJar：持久化到应用私有存储。密码不落盘，只存会话 cookie。 */
class CookieStore(context: Context) : CookieJar {
    private val sp = context.getSharedPreferences("dlz-cookies", Context.MODE_PRIVATE)
    private val json = Json { ignoreUnknownKeys = true }
    private val lock = Any()
    private var cache: MutableList<Cookie> = load()

    private fun load(): MutableList<Cookie> {
        return try {
            val raw = sp.getString("cookies", "[]") ?: "[]"
            json.decodeFromString<List<SavedCookie>>(raw).mapNotNull { saved ->
                Cookie.Builder()
                    .name(saved.name).value(saved.value)
                    .expiresAt(saved.expiresAt)
                    .path(saved.path)
                    .apply { if (saved.hostOnly) hostOnlyDomain(saved.domain) else domain(saved.domain) }
                    .apply { if (saved.secure) secure() }
                    .apply { if (saved.httpOnly) httpOnly() }
                    .build()
            }.toMutableList()
        } catch (_: Exception) {
            mutableListOf()
        }
    }

    private fun persist() {
        val saved = cache.map {
            SavedCookie(it.name, it.value, it.expiresAt, it.domain, it.path, it.secure, it.httpOnly, it.hostOnly)
        }
        sp.edit().putString("cookies", json.encodeToString(ListSerializer(SavedCookie.serializer()), saved)).apply()
    }

    override fun saveFromResponse(url: HttpUrl, cookies: List<Cookie>) {
        synchronized(lock) {
            for (cookie in cookies) {
                cache.removeAll { it.name == cookie.name }
                if (cookie.expiresAt > System.currentTimeMillis()) cache.add(cookie)
            }
            persist()
        }
    }

    override fun loadForRequest(url: HttpUrl): List<Cookie> {
        synchronized(lock) {
            cache = cache.filter { it.expiresAt > System.currentTimeMillis() }.toMutableList()
            return cache.filter { it.matches(url) }
        }
    }

    fun clear() {
        synchronized(lock) {
            cache.clear()
            persist()
        }
    }
}

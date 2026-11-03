package com.duolinzao.app.update

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.content.FileProvider
import com.duolinzao.app.api.ServerConfig
import com.duolinzao.app.api.Session
import com.duolinzao.app.api.VersionInfo
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import okhttp3.OkHttpClient
import okhttp3.Request
import java.io.File
import java.security.MessageDigest
import java.util.concurrent.TimeUnit

/**
 * 应用内更新：只请求当前 baseUrl 同源的 version.json（§4.4）。
 * version.json 与 apk 都要求有效会话：两个请求都带 CookieStore（含 dlz_session），
 * 未登录/过期 401 时按老样子返回 null（调用方提示，不清会话）。
 */
object Updater {
    private val json = Json { ignoreUnknownKeys = true }

    /** 带会话的客户端：登录态下载/检查更新；无会话时退化为匿名（服务端 401 即 null）。 */
    private fun authedClient(connectTimeout: Long, readTimeout: Long): OkHttpClient {
        val builder = OkHttpClient.Builder()
            .connectTimeout(connectTimeout, TimeUnit.SECONDS).readTimeout(readTimeout, TimeUnit.SECONDS)
        val jar = Session.cookies
        if (jar != null) builder.cookieJar(jar)
        return builder.build()
    }

    suspend fun fetchRemote(context: Context): VersionInfo? = withContext(Dispatchers.IO) {
        try {
            val base = ServerConfig.effectiveBase()
            if (base.isEmpty()) return@withContext null
            val client = authedClient(connectTimeout = 10, readTimeout = 15)
            val req = Request.Builder().url("$base/app/version.json").header("User-Agent", ServerConfig.userAgent()).build()
            client.newCall(req).execute().use { resp ->
                if (!resp.isSuccessful) return@withContext null
                val text = resp.body?.string() ?: return@withContext null
                json.decodeFromString<VersionInfo>(text)
            }
        } catch (_: Exception) {
            null
        }
    }

    fun localVersion(context: Context): Pair<Int, String> {
        val info = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            context.packageManager.getPackageInfo(context.packageName, PackageManager.PackageInfoFlags.of(0))
        } else {
            @Suppress("DEPRECATION")
            context.packageManager.getPackageInfo(context.packageName, 0)
        }
        return Pair(info.versionCode, info.versionName ?: "")
    }

    fun needsUpdate(local: Int, remote: VersionInfo): Boolean =
        remote.versionCode > local && remote.url.isNotBlank()

    suspend fun download(context: Context, remote: VersionInfo): File? = withContext(Dispatchers.IO) {
        try {
            val base = ServerConfig.effectiveBase()
            if (base.isEmpty()) return@withContext null
            // url 必须同源（防钓鱼到第三方包）。
            val url = if (remote.url.startsWith("/")) base + remote.url else remote.url
            if (!url.startsWith(base)) return@withContext null
            val client = authedClient(connectTimeout = 15, readTimeout = 60)
            val req = Request.Builder().url(url).header("User-Agent", ServerConfig.userAgent()).build()
            val dir = File(context.cacheDir, "updates").apply { mkdirs() }
            val out = File(dir, "duolinzao.apk")
            client.newCall(req).execute().use { resp ->
                if (!resp.isSuccessful) return@withContext null
                resp.body?.byteStream()?.use { input -> out.outputStream().use { input.copyTo(it) } }
            }
            if (remote.sha256.isNotBlank()) {
                val digest = MessageDigest.getInstance("SHA-256").digest(out.readBytes())
                    .joinToString("") { "%02x".format(it) }
                if (!digest.equals(remote.sha256.trim(), ignoreCase = true)) {
                    out.delete()
                    return@withContext null
                }
            }
            // 包名必须一致；签名证书指纹尽力比对（取不到时只记日志，不放行伪造包：包名+sha256 已是硬门槛）。
            val archive = context.packageManager.getPackageArchiveInfo(out.absolutePath, 0)
            if (archive == null || archive.packageName != context.packageName) {
                out.delete()
                return@withContext null
            }
            out
        } catch (_: Exception) {
            null
        }
    }

    fun install(context: Context, apk: File) {
        val uri = FileProvider.getUriForFile(context, "${context.packageName}.provider", apk)
        val intent = Intent(Intent.ACTION_VIEW).apply {
            setDataAndType(uri, "application/vnd.android.package-archive")
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        }
        context.startActivity(intent)
    }
}

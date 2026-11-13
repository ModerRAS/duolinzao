package com.duolinzao.app

import android.annotation.SuppressLint
import android.app.Activity
import android.app.DownloadManager
import android.content.ActivityNotFoundException
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.net.Uri
import android.os.Bundle
import android.os.Environment
import android.os.SystemClock
import android.view.ViewGroup
import android.webkit.CookieManager
import android.webkit.PermissionRequest
import android.webkit.URLUtil
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.FrameLayout
import android.widget.TextView
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.OnBackPressedCallback
import androidx.activity.result.contract.ActivityResultContracts
import androidx.webkit.WebSettingsCompat
import androidx.webkit.WebViewFeature
import com.duolinzao.app.api.ServerConfig

/**
 * 系统 WebView 壳：直接加载站点，一套 UI 两端复用。
 *
 * 说明（见 docs/clients/webview-shell.md）：
 * - 包内 assets/server.json 决定 baseUrl（沿用 inject 改包流程）；
 * - Passkey 走 AndroidX WebKit 的 WEB_AUTHENTICATION（系统 Credential Manager），
 *   依赖 App 与站点的 Digital Asset Links；不支持时由 Web 端回退密码；
 * - CookieManager 持久化 dlz_session（服务端已下发 Max-Age），杀进程重开仍登录；
 * - 只允许站点同源在壳内打开，其它链接交给系统浏览器；
 * - 不注入任何 JS 桥。
 */
class MainActivity : ComponentActivity() {
    private lateinit var webView: WebView
    private var appHost: String = ""
    private var baseUrl: String = ""
    private var fileCallback: ValueCallback<Array<Uri>>? = null
    // null = 未处于「已回首页、再按一次退出」状态；不能用 0L：
    // 开机 2 秒内 uptimeMillis() 还很小，会被误判成双击直接退出。
    private var lastBackPress: Long? = null
    // 是否正在显示网络错误页。loadDataWithBaseURL 异步提交，showOfflinePage() 里
    // 紧跟的 clearHistory() 跑在错误页入栈之前，错误页仍会留在历史里（真机复现：
    // 点「重试」成功回首页后按 BACK，退回的是 data: 错误页）。
    // 所以在「重试」导航完成后再清一次历史，保证错误页不成为可回退历史。
    private var offlinePageShown = false
    private var clearHistoryAfterNav = false

    private val filePicker =
        registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
            val callback = fileCallback
            fileCallback = null
            if (callback == null) return@registerForActivityResult
            if (result.resultCode != Activity.RESULT_OK) {
                callback.onReceiveValue(null)
                return@registerForActivityResult
            }
            val data = result.data
            val uris: Array<Uri> = when {
                data == null -> emptyArray()
                data.clipData != null -> {
                    val clip = data.clipData!!
                    Array(clip.itemCount) { index -> clip.getItemAt(index).uri }
                }
                data.data != null -> arrayOf(data.data!!)
                else -> emptyArray()
            }
            callback.onReceiveValue(uris)
        }

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        ServerConfig.load(this)
        baseUrl = ServerConfig.effectiveBase().trimEnd('/')
        if (!ServerConfig.valid() || baseUrl.isBlank()) {
            showFatal("安装包未绑定有效服务器\n\n请使用站点 /app/ 分发的安装包，或检查 assets/server.json。")
            return
        }
        appHost = Uri.parse(baseUrl).host.orEmpty()
        if (appHost.isBlank()) {
            showFatal("服务器地址非法：$baseUrl")
            return
        }

        webView = WebView(this)
        val root = FrameLayout(this)
        root.addView(
            webView,
            FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            ),
        )
        setContentView(root)
        configureWebView()
        onBackPressedDispatcher.addCallback(
            this,
            object : OnBackPressedCallback(true) {
                override fun handleOnBackPressed() {
                    // BACK 状态机（判定逻辑抽在 BackPolicy，JVM 单测覆盖）：
                    // 1. 2 秒窗口内再按 = 退出。必须先于 canGoBack 判断：loadUrl
                    //    在重定向 / SSL error 等情况下可能新增历史条目，先看 canGoBack
                    //    会让第二次按 BACK 走 goBack，双击退出失效；
                    // 2. 有历史走历史（goBack 属于导航，重置待退出状态，避免跨页误退）；
                    // 3. 无历史：SPA 切页与地址栏同 URL，URL 无法区分内页，
                    //    一律 load home + toast，并记下待退出时间。
                    val now = SystemClock.uptimeMillis()
                    when (BackPolicy.decide(now, webView.canGoBack(), lastBackPress)) {
                        BackAction.FINISH -> {
                            lastBackPress = null
                            finish()
                        }
                        BackAction.GO_BACK -> {
                            lastBackPress = null
                            webView.goBack()
                        }
                        BackAction.ARM_HOME -> {
                            lastBackPress = now
                            webView.loadUrl("$baseUrl/")
                            Toast.makeText(this@MainActivity, "已回首页，再按一次退出", Toast.LENGTH_SHORT).show()
                        }
                    }
                }
            },
        )
        webView.loadUrl("$baseUrl/")
    }

    @SuppressLint("SetJavaScriptEnabled")
    private fun configureWebView() {
        with(webView.settings) {
            javaScriptEnabled = true
            domStorageEnabled = true
            databaseEnabled = true
            allowFileAccess = false
            allowContentAccess = false
            mixedContentMode = WebSettings.MIXED_CONTENT_NEVER_ALLOW
            cacheMode = WebSettings.LOAD_DEFAULT
            mediaPlaybackRequiresUserGesture = true
            setSupportMultipleWindows(false)
            userAgentString = "$userAgentString duolinzao-android-shell/${appVersion()}"
        }

        val cookies = CookieManager.getInstance()
        cookies.setAcceptCookie(true)
        cookies.setAcceptThirdPartyCookies(webView, false)

        // Passkey：系统 Credential Manager 接入 WebView（官方能力，1.12.0+）。
        if (WebViewFeature.isFeatureSupported(WebViewFeature.WEB_AUTHENTICATION)) {
            WebSettingsCompat.setWebAuthenticationSupport(
                webView.settings,
                WebSettingsCompat.WEB_AUTHENTICATION_SUPPORT_FOR_APP,
            )
        }
        if (isDebuggable()) WebView.setWebContentsDebuggingEnabled(true)

        webView.webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(
                view: WebView,
                request: WebResourceRequest,
            ): Boolean {
                val uri = request.url
                val scheme = uri.scheme?.lowercase()
                if (scheme == "http" || scheme == "https") {
                    if (uri.host == appHost) {
                        // 错误页上的「重试」是普通链接导航：标记目标页加载完成后清历史。
                        if (offlinePageShown) clearHistoryAfterNav = true
                        return false
                    }
                    openExternal(uri)
                    return true
                }
                // tel:/mailto: 等交给系统
                return try {
                    startActivity(Intent(Intent.ACTION_VIEW, uri))
                    true
                } catch (_: ActivityNotFoundException) {
                    true
                }
            }

            override fun onReceivedError(
                view: WebView,
                request: WebResourceRequest,
                error: WebResourceError,
            ) {
                if (!request.isForMainFrame) return
                // 主框架加载失败：这次「重试」没有成功，取消待清理标记
                // （错误页会重新显示，等下一次重试成功再清历史）。
                clearHistoryAfterNav = false
                showOfflinePage()
            }

            override fun onPageFinished(view: WebView, url: String) {
                if (!offlinePageShown || !clearHistoryAfterNav) return
                // 「重试」目标页（正常页面）已加载完成：此时错误页已经是 back 条目，
                // 清历史才能真正把它从回退栈里抹掉。
                clearHistoryAfterNav = false
                offlinePageShown = false
                view.clearHistory()
            }
        }

        webView.webChromeClient = object : WebChromeClient() {
            override fun onShowFileChooser(
                webView: WebView,
                filePathCallback: ValueCallback<Array<Uri>>,
                fileChooserParams: WebChromeClient.FileChooserParams,
            ): Boolean {
                fileCallback?.onReceiveValue(null)
                fileCallback = filePathCallback
                return try {
                    filePicker.launch(fileChooserParams.createIntent())
                    true
                } catch (_: Exception) {
                    fileCallback = null
                    false
                }
            }

            override fun onPermissionRequest(request: PermissionRequest) {
                // 壳不需要相机/麦克风；如需，在这里按运行时权限放行。
                request.deny()
            }
        }

        webView.setDownloadListener { url, userAgent, contentDisposition, mimeType, _ ->
            val uri = Uri.parse(url)
            if (uri.host != appHost) {
                openExternal(uri)
                return@setDownloadListener
            }
            try {
                val fileName = URLUtil.guessFileName(url, contentDisposition, mimeType)
                val request = DownloadManager.Request(uri)
                    .setMimeType(mimeType)
                    .setTitle(fileName)
                    .setNotificationVisibility(DownloadManager.Request.VISIBILITY_VISIBLE_NOTIFY_COMPLETED)
                    .setDestinationInExternalPublicDir(Environment.DIRECTORY_DOWNLOADS, fileName)
                    .addRequestHeader("User-Agent", userAgent)
                CookieManager.getInstance().getCookie(url)?.let { cookie ->
                    request.addRequestHeader("Cookie", cookie)
                }
                val manager = getSystemService(DOWNLOAD_SERVICE) as DownloadManager
                manager.enqueue(request)
                Toast.makeText(this, "已开始下载：$fileName", Toast.LENGTH_SHORT).show()
            } catch (e: Exception) {
                Toast.makeText(this, "下载失败：${e.message}", Toast.LENGTH_LONG).show()
            }
        }
    }

    private fun showOfflinePage() {
        offlinePageShown = true
        val html = """
            <!doctype html><html lang="zh-CN"><head><meta charset="utf-8">
            <meta name="viewport" content="width=device-width, initial-scale=1">
            <style>
              body{margin:0;height:100vh;display:flex;align-items:center;justify-content:center;
                font-family:system-ui,-apple-system,"Noto Sans SC",sans-serif;background:#f3f5f7;color:#1f2937}
              .box{max-width:20rem;padding:2rem;text-align:center}
              h1{font-size:1.1rem;margin:0 0 .6rem}
              p{color:#6b7280;font-size:.9rem;line-height:1.6;margin:0 0 1.2rem}
              a{display:inline-block;padding:.6rem 1.4rem;border-radius:.6rem;background:#2d8a45;color:#fff;text-decoration:none}
            </style></head><body><div class="box">
            <h1>暂时连不上服务器</h1>
            <p>请检查网络后重试。已登录状态会保留，恢复网络后可直接继续。</p>
            <a href="$baseUrl/">重试</a>
            </div></body></html>
        """.trimIndent()
        webView.loadDataWithBaseURL(baseUrl, html, "text/html", "utf-8", null)
        // 错误页不该成为可回退的历史条目：否则 BACK 会退回加载失败的 URL、
        // 甚至反复触发错误页，而不是按「回首页 / 双击退出」处理。
        // 页面里保留「重试」链接作为唯一入口，清历史不影响重试。
        // 这里是第一道清理（先清掉失败导航留下的条目）；loadData 异步提交，
        // 错误页自身在重试成功后由 shouldOverrideUrlLoading + onPageFinished 再清一次。
        webView.clearHistory()
    }

    private fun openExternal(uri: Uri) {
        try {
            startActivity(Intent(Intent.ACTION_VIEW, uri))
        } catch (_: ActivityNotFoundException) {
            Toast.makeText(this, "没有可打开该链接的应用", Toast.LENGTH_SHORT).show()
        }
    }

    private fun showFatal(message: String) {
        val view = TextView(this).apply {
            text = message
            textSize = 15f
            setPadding(64, 64, 64, 64)
            setBackgroundColor(0xFFF3F5F7.toInt())
        }
        val root = FrameLayout(this)
        root.addView(
            view,
            FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
                android.view.Gravity.CENTER,
            ),
        )
        setContentView(root)
    }

    private fun isDebuggable(): Boolean =
        (applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) != 0

    private fun appVersion(): String =
        try {
            packageManager.getPackageInfo(packageName, 0).versionName ?: "unknown"
        } catch (_: Exception) {
            "unknown"
        }

    override fun onPause() {
        super.onPause()
        if (this::webView.isInitialized) {
            webView.onPause()
            // 关键：把持久 Cookie 刷盘，杀进程后仍保留登录态。
            CookieManager.getInstance().flush()
        }
    }

    override fun onResume() {
        super.onResume()
        if (this::webView.isInitialized) webView.onResume()
    }

    override fun onDestroy() {
        if (this::webView.isInitialized) {
            webView.stopLoading()
            webView.destroy()
        }
        super.onDestroy()
    }
}

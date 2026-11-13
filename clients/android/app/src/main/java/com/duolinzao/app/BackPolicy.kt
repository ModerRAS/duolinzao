package com.duolinzao.app

/** 壳 BACK 键决策结果（纯逻辑，见 [BackPolicy]）。 */
enum class BackAction {
    /** 2 秒内第二次按 BACK：退出 App。 */
    FINISH,

    /** WebView 有可回退历史：走 goBack。 */
    GO_BACK,

    /** 无历史：回首页并提示「再按一次退出」，记录待退出时间。 */
    ARM_HOME,
}

/**
 * 壳 BACK 键纯判定逻辑：不依赖 Android API，可用 JVM 单测覆盖（见 app/src/test）。
 *
 * 判定顺序很重要：**先判双击退出，再判 canGoBack**。loadUrl 在重定向 / SSL error
 * 等情况下可能新增历史条目，若先看 canGoBack，第二次按 BACK 会走 goBack，双击退出失效。
 *
 * 待退出时间用可空 Long 表示：初始为 null（未处于待退出状态），避免进程启动后
 * uptimeMillis() < 2000ms 时把「上次按 BACK 时间 = 0」误判成双击而直接退出。
 */
object BackPolicy {
    /** 双击退出窗口（毫秒）。 */
    const val DOUBLE_PRESS_WINDOW_MS = 2000L

    /**
     * @param now 当前时间（SystemClock.uptimeMillis()）。
     * @param canGoBack WebView 是否有可回退历史。
     * @param armedAt 上一次「已回首页，再按一次退出」的时间；null 表示未处于待退出状态。
     */
    fun decide(now: Long, canGoBack: Boolean, armedAt: Long?): BackAction {
        if (armedAt != null && now >= armedAt && now - armedAt < DOUBLE_PRESS_WINDOW_MS) {
            return BackAction.FINISH
        }
        return if (canGoBack) BackAction.GO_BACK else BackAction.ARM_HOME
    }
}

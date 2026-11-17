package com.duolinzao.app

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * BACK 状态机纯逻辑单测：不依赖 Android 运行时，`./gradlew :app:testDebugUnitTest` 可跑。
 *
 * 覆盖审查 P2 要求：双击判定先于 canGoBack、导航后重置待退出状态、
 * 启动不足 2 秒时不因 uptime 从 0 起算而误 finish。
 */
class BackPolicyTest {
    @Test
    fun `无历史且未待退出时提示回首页`() {
        assertEquals(
            BackAction.ARM_HOME,
            BackPolicy.decide(now = 10_000L, canGoBack = false, armedAt = null),
        )
    }

    @Test
    fun `有历史且未待退出时走 goBack`() {
        assertEquals(
            BackAction.GO_BACK,
            BackPolicy.decide(now = 10_000L, canGoBack = true, armedAt = null),
        )
    }

    @Test
    fun `窗口内再按退出即使此时 canGoBack 为真`() {
        // 回归点：loadUrl 在重定向 / SSL error 等情况下可能新增历史条目。
        // 双击判定必须先于 canGoBack，否则第二次按 BACK 会变成 goBack。
        assertEquals(
            BackAction.FINISH,
            BackPolicy.decide(now = 11_999L, canGoBack = true, armedAt = 10_000L),
        )
    }

    @Test
    fun `窗口内最后一毫秒仍退出`() {
        assertEquals(
            BackAction.FINISH,
            BackPolicy.decide(now = 11_999L, canGoBack = false, armedAt = 10_000L),
        )
    }

    @Test
    fun `窗口到期后不退出有历史走 goBack`() {
        assertEquals(
            BackAction.GO_BACK,
            BackPolicy.decide(
                now = 10_000L + BackPolicy.DOUBLE_PRESS_WINDOW_MS,
                canGoBack = true,
                armedAt = 10_000L,
            ),
        )
    }

    @Test
    fun `窗口到期后不退出无历史重新提示回首页`() {
        assertEquals(
            BackAction.ARM_HOME,
            BackPolicy.decide(
                now = 10_000L + BackPolicy.DOUBLE_PRESS_WINDOW_MS,
                canGoBack = false,
                armedAt = 10_000L,
            ),
        )
    }

    @Test
    fun `启动不足 2 秒且未按过 BACK 不会误退出`() {
        // 旧实现 lastBackPress 初值 0L，uptimeMillis() < 2000 时 now - 0 < 2000 → 首次按就 finish。
        assertEquals(
            BackAction.ARM_HOME,
            BackPolicy.decide(now = 500L, canGoBack = false, armedAt = null),
        )
    }
}

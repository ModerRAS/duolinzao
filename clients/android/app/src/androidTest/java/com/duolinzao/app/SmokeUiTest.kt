package com.duolinzao.app

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Compose UI 冒烟：mock 模式（assets/server.json useMock=true），不连真后端。
 * 走密码兜底，不依赖 Passkey（模拟器上无凭据提供方）。
 */
@RunWith(AndroidJUnit4::class)
class SmokeUiTest {
    @get:Rule
    val composeRule = createAndroidComposeRule<MainActivity>()

    @Test
    fun canteenLoginEntersCanteenShell() {
        // 食堂 mock 账号：用户名 canteen，任意非空密码
        composeRule.onNodeWithText("员工登录").assertIsDisplayed()
        composeRule.onNodeWithTag("login_password_toggle").performScrollTo().performClick()
        composeRule.onNodeWithTag("login_username").performScrollTo().performTextInput("canteen")
        composeRule.onNodeWithTag("login_password").performScrollTo().performTextInput("password123")
        composeRule.onNodeWithTag("login_submit").performScrollTo().performClick()

        // 食堂竖屏壳四 Tab 出现，人数卡可见
        composeRule.waitUntil(timeoutMillis = 15_000) {
            composeRule.onAllNodesWithTag("tab_headcount").fetchSemanticsNodes().isNotEmpty()
        }
        composeRule.onNodeWithTag("tab_headcount").assertIsDisplayed()
        composeRule.onNodeWithTag("tab_optimize").assertIsDisplayed()
        composeRule.onNodeWithTag("tab_menu").assertIsDisplayed()
        composeRule.onNodeWithTag("tab_more").assertIsDisplayed()
        composeRule.onNodeWithTag("headcount_card_lunch").assertIsDisplayed()

        // C2 菜谱：列表真数据，无占位文案
        composeRule.onNodeWithTag("tab_menu").performClick()
        composeRule.waitUntil(timeoutMillis = 15_000) {
            composeRule.onAllNodesWithTag("menu_new").fetchSemanticsNodes().isNotEmpty()
        }
        composeRule.onNodeWithTag("menu_new").assertIsDisplayed()
        composeRule.onNodeWithTag("menu_row_d1").assertIsDisplayed()

        // C3 选菜：周视图真数据，无占位文案
        composeRule.onNodeWithTag("tab_optimize").performClick()
        composeRule.waitUntil(timeoutMillis = 15_000) {
            composeRule.onAllNodesWithTag("opt_prev_week").fetchSemanticsNodes().isNotEmpty()
        }
        composeRule.onNodeWithTag("opt_prev_week").assertIsDisplayed()
        composeRule.onNodeWithTag("opt_next_week").assertIsDisplayed()
    }

    @Test
    fun loginPasswordEntersShell() {
        // 1. 启动后能看到登录页
        composeRule.onNodeWithText("员工登录").assertIsDisplayed()

        // 2. 展开密码区（默认收起），填任意非空账号/密码（mock 接受）
        composeRule.onNodeWithTag("login_password_toggle").performScrollTo().performClick()
        composeRule.onNodeWithTag("login_username").performScrollTo().performTextInput("demo")
        composeRule.onNodeWithTag("login_password").performScrollTo().performTextInput("password123")
        composeRule.onNodeWithTag("login_submit").performScrollTo().performClick()

        // 3. 登录后壳底部三 Tab 出现
        composeRule.waitUntil(timeoutMillis = 15_000) {
            composeRule.onAllNodesWithTag("tab_meals").fetchSemanticsNodes().isNotEmpty()
        }
        composeRule.onNodeWithTag("tab_meals").assertIsDisplayed()
        composeRule.onNodeWithTag("tab_score").assertIsDisplayed()
        composeRule.onNodeWithTag("tab_mine").assertIsDisplayed()
    }
}

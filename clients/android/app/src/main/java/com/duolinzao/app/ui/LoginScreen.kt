package com.duolinzao.app.ui

import android.app.Activity
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import com.duolinzao.app.api.ApiException
import com.duolinzao.app.api.Session
import com.duolinzao.app.auth.PasskeyHelper
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.serialization.json.Json

/**
 * 登录：默认 Passkey（平台 API）；无可用凭据/取消/失败自动回退密码。
 * 邮箱登录仅 OTP（request 一律 accepted）；认证器 TOTP 独立；新设备可走页内确认等待。
 */
@Composable
fun LoginScreen(onDone: () -> Unit, onRegister: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var username by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var totpUser by remember { mutableStateOf("") }
    var totpCode by remember { mutableStateOf("") }
    var otpUser by remember { mutableStateOf("") }
    var otpCode by remember { mutableStateOf("") }
    var otpSent by remember { mutableStateOf(false) }
    var showPwd by remember { mutableStateOf(false) }
    var showTotp by remember { mutableStateOf(false) }
    var showOtp by remember { mutableStateOf(false) }
    var showWait by remember { mutableStateOf(false) }
    var waitInfo by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    var methods by remember { mutableStateOf(listOf("passkey", "totp", "password")) }

    LaunchedEffect(Unit) {
        try {
            val m = Session.repo.methods()
            methods = m.methods
        } catch (_: Exception) { /* 保持默认顺序 */ }
    }

    suspend fun enter() {
        Session.setMe(Session.repo.me())
        onDone()
    }

    fun fail(e: Exception, fallbackPwd: Boolean = false) {
        error = if (e is ApiException) e.message else "请求失败"
        if (fallbackPwd) showPwd = true
    }

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text("多邻灶", style = MaterialTheme.typography.headlineMedium)
        Text("员工登录", style = MaterialTheme.typography.titleMedium)
        Spacer(Modifier.height(8.dp))
        DlTextField(username, { username = it }, "账号")

        DlPrimaryButton("使用 Passkey 登录") {
            scope.launch {
                busy = true; error = null
                try {
                    val activity = context as Activity
                    val started = Session.repo.passkeyStart(username.ifBlank { throw ApiException(400, "need_username", "Passkey 需要先填账号") })
                    val requestJson = started.publicKey?.let {
                        Json.encodeToString(kotlinx.serialization.json.JsonElement.serializer(), it)
                    }.orEmpty()
                    val assertion = PasskeyHelper.getAssertion(activity, requestJson)
                    if (assertion == null) {
                        showPwd = true
                        error = "Passkey 不可用，已切换到密码登录"
                    } else {
                        Session.repo.passkeyFinish(username, started.challenge_id, assertion)
                        enter()
                    }
                } catch (e: Exception) { fail(e as Exception, fallbackPwd = true) }
                busy = false
            }
        }
        if (methods.contains("totp")) {
            DlOutlineButton(if (showTotp) "收起 TOTP" else "使用 TOTP") { showTotp = !showTotp }
        }
        if (showTotp) {
            DlTextField(totpUser.ifBlank { username }, { totpUser = it }, "账号")
            DlTextField(totpCode, { totpCode = it }, "6 位验证码")
            DlPrimaryButton("验证并登录") {
                scope.launch {
                    try {
                        Session.repo.loginTotp(totpUser.ifBlank { username }, totpCode)
                        enter()
                    } catch (e: Exception) { fail(e as Exception) }
                }
            }
        }
        if (methods.contains("email_otp")) {
            DlOutlineButton(if (showOtp) "收起邮箱验证码" else "邮箱验证码") { showOtp = !showOtp }
        }
        if (showOtp) {
            DlTextField(otpUser.ifBlank { username }, { otpUser = it }, "账号")
            if (!otpSent) {
                DlOutlineButton("发送验证码") {
                    scope.launch {
                        try {
                            Session.repo.requestEmailOtp(otpUser.ifBlank { username })
                            otpSent = true
                            error = "如果账号已绑定邮箱，验证码已发出"
                        } catch (e: Exception) { fail(e as Exception) }
                    }
                }
            } else {
                DlTextField(otpCode, { otpCode = it }, "6 位验证码")
                DlPrimaryButton("验证并登录") {
                    scope.launch {
                        try {
                            Session.repo.verifyEmailOtp(otpUser.ifBlank { username }, otpCode)
                            enter()
                        } catch (e: Exception) { fail(e as Exception) }
                    }
                }
            }
        }

        TextButton(onClick = { showPwd = !showPwd }) { Text(if (showPwd) "收起密码登录 ▴" else "密码登录（兜底） ▾") }
        if (showPwd) {
            DlTextField(password, { password = it }, "密码", password = true)
            DlPrimaryButton("登录") {
                scope.launch {
                    try {
                        Session.repo.loginPassword(username, password)
                        enter()
                    } catch (e: Exception) { fail(e as Exception) }
                }
            }
        }

        TextButton(onClick = { showWait = !showWait }) { Text("新设备等待页内确认") }
        if (showWait) {
            DlOutlineButton("请已登录设备确认") {
                scope.launch {
                    try {
                        val created = Session.repo.createApproval(username)
                        waitInfo = "等待确认…"
                        while (true) {
                            delay(2000)
                            val poll = Session.repo.pollApproval(created.id, created.poll_token)
                            if (poll.status == "allowed") {
                                waitInfo = "已通过"
                                enter()
                                break
                            }
                            if (poll.status != "pending") {
                                waitInfo = "状态：${poll.status}"
                                break
                            }
                        }
                    } catch (e: Exception) { fail(e as Exception) }
                }
            }
            if (waitInfo.isNotEmpty()) Text(waitInfo)
        }

        ErrorLine(error)
        TextButton(onClick = onRegister) { Text("有邀请码，去注册") }
        if (busy) Text("处理中…")
    }
}

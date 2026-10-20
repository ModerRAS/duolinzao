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
import kotlinx.coroutines.launch
import kotlinx.serialization.json.Json

/**
 * 邀请注册：成功即登录（服务端 Set-Cookie），随后绑定 Passkey（可跳过/不支持自动跳过）。
 * 不走新设备确认。
 */
@Composable
fun RegisterScreen(onDone: () -> Unit, onBack: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var username by remember { mutableStateOf("") }
    var invite by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var bindState by remember { mutableStateOf<String?>(null) }

    suspend fun bindPasskeyOrSkip() {
        try {
            val activity = context as Activity
            val started = Session.repo.passkeyRegisterStart()
            val requestJson = started.publicKey?.let {
                Json.encodeToString(kotlinx.serialization.json.JsonElement.serializer(), it)
            }.orEmpty()
            val credential = PasskeyHelper.createPasskey(activity, requestJson)
            if (credential == null) {
                bindState = "此设备不支持 Passkey，已跳过（可在「我的 → 设备与 Passkey」补绑）"
            } else {
                Session.repo.passkeyRegisterFinish(started.challenge_id, credential, "本机")
                bindState = "Passkey 绑定成功"
            }
        } catch (_: Exception) {
            bindState = "Passkey 绑定跳过（可在「我的 → 设备与 Passkey」补绑）"
        }
    }

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text("邀请码注册", style = MaterialTheme.typography.headlineMedium)
        DlTextField(username, { username = it }, "账号")
        DlTextField(invite, { invite = it }, "邀请码")
        DlTextField(password, { password = it }, "密码（至少 8 位）", password = true)
        DlPrimaryButton("注册并登录") {
            scope.launch {
                error = null
                try {
                    Session.repo.register(username, invite, password)
                    try {
                        Session.setMe(Session.repo.me())
                    } catch (_: Exception) {
                        // 旧服务端注册不带会话：回退一次密码登录。
                        Session.repo.loginPassword(username, password)
                        Session.setMe(Session.repo.me())
                    }
                    bindPasskeyOrSkip()
                    onDone()
                } catch (e: Exception) {
                    error = if (e is ApiException) e.message else "注册失败"
                }
            }
        }
        if (bindState != null) Text(bindState!!)
        ErrorLine(error)
        Spacer(Modifier.height(8.dp))
        TextButton(onClick = onBack) { Text("返回登录") }
    }
}

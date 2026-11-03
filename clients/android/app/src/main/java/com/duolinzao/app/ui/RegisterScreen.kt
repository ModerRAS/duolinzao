package com.duolinzao.app.ui

import android.app.Activity
import androidx.activity.compose.BackHandler
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
 * 邀请注册：账号 + 邀请码。能创建通行密钥时必须绑定成功才进入；
 * 设备不支持、用户取消或创建失败时，改为必填至少 8 位密码。
 * 不走新设备确认。TOTP 不在这一步设置。
 */
@Composable
fun RegisterScreen(onDone: () -> Unit, onBack: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var username by remember { mutableStateOf("") }
    var invite by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    val existing = Session.me.value
    var step by remember {
        mutableStateOf(
            if (existing != null && !existing.has_password && !existing.has_passkey) "passkey" else "form",
        )
    }

    BackHandler(enabled = step != "form") { }

    suspend fun tryBind(): Boolean {
        val activity = context as? Activity ?: return false
        val started = Session.repo.passkeyRegisterStart()
        val requestJson = started.publicKey?.let {
            Json.encodeToString(kotlinx.serialization.json.JsonElement.serializer(), it)
        }.orEmpty()
        if (requestJson.isBlank()) return false
        val credential = PasskeyHelper.createPasskey(activity, requestJson) ?: return false
        Session.repo.passkeyRegisterFinish(started.challenge_id, credential, "本机")
        Session.setMe(Session.repo.me())
        return true
    }

    fun bindThenEnter() {
        scope.launch {
            busy = true
            error = null
            try {
                if (tryBind()) onDone() else step = "password"
            } catch (e: Exception) {
                error = if (e is ApiException) e.message else "通行密钥没有完成，请设置密码"
                step = "password"
            } finally {
                busy = false
            }
        }
    }

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        when (step) {
            "passkey" -> {
                Text("绑定通行密钥", style = MaterialTheme.typography.headlineMedium)
                Text("要先绑定通行密钥才能进入。这台设备可以创建通行密钥时，不能改用密码跳过。")
                DlPrimaryButton("绑定通行密钥", enabled = !busy) { bindThenEnter() }
                if (busy) Text("正在唤起通行密钥…")
            }
            "password" -> {
                Text("设置密码", style = MaterialTheme.typography.headlineMedium)
                Text("通行密钥现在用不了（设备不支持、已取消或创建失败）。请设置至少 8 位密码后再进入。")
                DlTextField(password, { password = it }, "密码（至少 8 位）", password = true)
                DlPrimaryButton("设置密码并进入", enabled = !busy) {
                    scope.launch {
                        busy = true
                        error = null
                        try {
                            if (password.length < 8) {
                                error = "密码至少 8 个字符"
                            } else {
                                Session.repo.changePassword("", password)
                                Session.setMe(Session.repo.me())
                                onDone()
                            }
                        } catch (e: Exception) {
                            error = if (e is ApiException) e.message else "密码设置失败"
                        } finally {
                            busy = false
                        }
                    }
                }
                TextButton(onClick = { step = "passkey" }, enabled = !busy) { Text("重试通行密钥") }
            }
            else -> {
                Text("邀请码注册", style = MaterialTheme.typography.headlineMedium)
                DlTextField(username, { username = it }, "账号")
                DlTextField(invite, { invite = it }, "邀请码")
                DlPrimaryButton("注册并登录", enabled = !busy) {
                    scope.launch {
                        busy = true
                        error = null
                        try {
                            Session.repo.register(username, invite, null)
                            try {
                                Session.setMe(Session.repo.me())
                            } catch (_: Exception) {
                                error = "注册未完成，请重试"
                                return@launch
                            }
                            step = "passkey"
                            val bound = try {
                                tryBind()
                            } catch (_: Exception) {
                                false
                            }
                            if (bound) onDone() else step = "password"
                        } catch (e: Exception) {
                            error = if (e is ApiException) e.message else "注册失败"
                        } finally {
                            busy = false
                        }
                    }
                }
                Spacer(Modifier.height(8.dp))
                TextButton(onClick = onBack) { Text("返回登录") }
            }
        }
        ErrorLine(error)
    }
}

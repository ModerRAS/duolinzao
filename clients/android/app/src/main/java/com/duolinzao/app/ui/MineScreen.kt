package com.duolinzao.app.ui

import android.app.Activity
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Card
import androidx.compose.material3.Checkbox
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import com.duolinzao.app.api.ApiException
import com.duolinzao.app.api.ApprovalItem
import com.duolinzao.app.api.Canteen
import com.duolinzao.app.api.DevicesResp
import com.duolinzao.app.api.Notice
import com.duolinzao.app.api.ServerConfig
import com.duolinzao.app.api.Session
import com.duolinzao.app.auth.PasskeyHelper
import com.duolinzao.app.update.Updater
import kotlinx.coroutines.launch
import kotlinx.serialization.json.Json

/** 我的 Tab：食堂绑定、账户设置、设备与 Passkey、新设备确认、通知、检查更新、退出。 */
@Composable
fun MineScreen(onLogout: () -> Unit, onOpenShift: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val me = Session.me.collectAsState().value
    var error by remember { mutableStateOf<String?>(null) }
    var devices by remember { mutableStateOf<DevicesResp?>(null) }
    var approvals by remember { mutableStateOf<List<ApprovalItem>>(emptyList()) }
    var notices by remember { mutableStateOf<List<Notice>>(emptyList()) }
    var canteens by remember { mutableStateOf<List<Canteen>>(emptyList()) }
    var bound by remember { mutableStateOf<Set<String>>(emptySet()) }
    var displayName by remember { mutableStateOf(me?.display_name.orEmpty()) }
    var email by remember { mutableStateOf(me?.email.orEmpty()) }
    var curPwd by remember { mutableStateOf("") }
    var newPwd by remember { mutableStateOf("") }
    var totpCode by remember { mutableStateOf("") }
    var totpSecret by remember { mutableStateOf<String?>(null) }
    var updateMsg by remember { mutableStateOf<String?>(null) }
    var debugUrl by remember { mutableStateOf(ServerConfig.debugOverride.orEmpty()) }
    val isDebug = (context.applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE) != 0

    suspend fun loadAll() {
        try {
            Session.setMe(Session.repo.me())
            devices = Session.repo.devices()
            approvals = Session.repo.pendingApprovals()
            notices = Session.repo.notifications()
            canteens = Session.repo.allCanteens()
            bound = Session.repo.myCanteens().map { it.id }.toSet()
            error = null
        } catch (e: Exception) {
            error = e.message ?: "加载失败"
        }
    }
    fun refreshAll() {
        scope.launch { loadAll() }
    }
    LaunchedEffect(Unit) { refreshAll() }

    suspend fun run(call: suspend () -> Unit, ok: String) {
        try { call(); loadAll(); if (ok.isNotEmpty()) error = ok } catch (e: Exception) { error = if (e is ApiException) e.message else "操作失败" }
    }

    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp)) {
        Text("我的", style = MaterialTheme.typography.headlineSmall)
        Text(me?.personName().orEmpty() + "（${me?.username.orEmpty()}）")
        if (me?.username != null && me?.display_name.isNullOrBlank().not()) {
            // username 只读展示
        }

        SectionTitle("我的食堂（可改绑定）")
        canteens.forEach { c ->
            Row(Modifier.fillMaxWidth().toggleable(value = bound.contains(c.id), onValueChange = {
                bound = if (bound.contains(c.id)) bound - c.id else bound + c.id
            }).padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                Checkbox(checked = bound.contains(c.id), onCheckedChange = null)
                Text(c.name)
            }
        }
        DlOutlineButton("保存食堂绑定") { scope.launch { run({ Session.repo.bindCanteens(bound.toList()) }, "已保存") } }

        SectionTitle("账户设置")
        DlTextField(displayName, { displayName = it }, "显示名（账号只读）")
        DlOutlineButton("保存显示名") { scope.launch { run({ Session.repo.updateDisplayName(displayName) }, "已保存") } }
        if (me?.email_enabled == true) {
            DlTextField(email, { email = it }, "邮箱")
            DlOutlineButton("绑定邮箱") { scope.launch { run({ Session.repo.bindEmail(email) }, "已绑定") } }
        }
        DlTextField(curPwd, { curPwd = it }, "当前密码", password = true)
        DlTextField(newPwd, { newPwd = it }, "新密码（至少 8 位）", password = true)
        DlOutlineButton("自助改密（保留本会话，吊销其它会话）") {
            scope.launch { run({ Session.repo.changePassword(curPwd, newPwd); curPwd = ""; newPwd = "" }, "已改密") }
        }
        if (me?.totp == true) {
            Text("已绑定认证器 TOTP")
        } else {
            DlOutlineButton("设置 TOTP（获取密钥）") {
                scope.launch {
                    try { totpSecret = Session.repo.totpSetup().secret; error = "请把密钥填入认证器 App" }
                    catch (e: Exception) { error = e.message }
                }
            }
            if (totpSecret != null) {
                Text("密钥：$totpSecret", style = MaterialTheme.typography.bodySmall)
                DlTextField(totpCode, { totpCode = it }, "认证器 6 位码")
                DlOutlineButton("确认绑定 TOTP") { scope.launch { run({ Session.repo.totpConfirm(totpCode) }, "TOTP 已绑定") } }
            }
        }

        SectionTitle("设备与 Passkey")
        Text("本页可绑定/解绑 Passkey、移除设备", style = MaterialTheme.typography.bodySmall)
        DlOutlineButton("绑定本机 Passkey") {
            scope.launch {
                try {
                    val activity = context as Activity
                    val started = Session.repo.passkeyRegisterStart()
                    val requestJson = started.publicKey?.let {
                        Json.encodeToString(kotlinx.serialization.json.JsonElement.serializer(), it)
                    }.orEmpty()
                    val credential = PasskeyHelper.createPasskey(activity, requestJson)
                    if (credential == null) error = "此设备不支持 Passkey"
                    else run({ Session.repo.passkeyRegisterFinish(started.challenge_id, credential, android.os.Build.MODEL) }, "Passkey 已绑定")
                } catch (e: Exception) { error = if (e is ApiException) e.message else "绑定失败" }
            }
        }
        devices?.passkeys?.forEach { p ->
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
                Text("Passkey：${p.friendly_name ?: p.id.take(8)}")
                TextButton(onClick = { scope.launch { run({ Session.repo.deletePasskey(p.id) }, "已解绑") } }) { Text("解绑") }
            }
        }
        devices?.devices?.forEach { d ->
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
                Text("${d.friendly_name}（${if (d.session_active) "会话中" else "无活跃会话"}）", modifier = Modifier.weight(1f))
                TextButton(onClick = { scope.launch { run({ Session.repo.deleteDevice(d.id) }, "已移除") } }) { Text("移除") }
            }
        }

        SectionTitle("新设备确认（${approvals.size} 条待处理）")
        approvals.forEach { a ->
            Card(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
                Column(Modifier.padding(12.dp)) {
                    Text("${a.friendly_name} · ${a.ip}")
                    Text(a.user_agent, style = MaterialTheme.typography.bodySmall)
                    Row {
                        TextButton(onClick = { scope.launch { run({ Session.repo.allowApproval(a.id) }, "已允许") } }) { Text("允许") }
                        TextButton(onClick = { scope.launch { run({ Session.repo.denyApproval(a.id) }, "已拒绝") } }) { Text("拒绝") }
                    }
                }
            }
        }

        SectionTitle("站内通知")
        notices.take(20).forEach { n ->
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text((if (n.read_at == null) "● " else "") + n.title, style = MaterialTheme.typography.bodyMedium)
                    Text(n.body, style = MaterialTheme.typography.bodySmall)
                }
                if (n.read_at == null) TextButton(onClick = { scope.launch { run({ Session.repo.readNotification(n.id) }, "") } }) { Text("已读") }
            }
        }

        SectionTitle("应用内更新")
        DlOutlineButton("检查更新") {
            scope.launch {
                try {
                    val remote = Session.repo.checkUpdate() ?: Updater.fetchRemote(context)
                    if (remote == null) { updateMsg = "当前为演示/离线模式，无更新源"; return@launch }
                    val (code, _) = Updater.localVersion(context)
                    if (!Updater.needsUpdate(code, remote)) { updateMsg = "已是最新（${remote.versionName}）"; return@launch }
                    updateMsg = "发现新版 ${remote.versionName}，下载中…"
                    val apk = Updater.download(context, remote)
                    if (apk == null) { updateMsg = "下载或校验失败，已丢弃"; return@launch }
                    updateMsg = "校验通过，正在调起安装…"
                    Updater.install(context, apk)
                } catch (e: Exception) { updateMsg = e.message ?: "检查失败" }
            }
        }
        if (updateMsg != null) Text(updateMsg!!)

        if (isDebug) {
            SectionTitle("开发设置（仅 debug 包）")
            DlTextField(debugUrl, { debugUrl = it }, "Base URL 覆盖（空=用包内 server.json）")
            DlOutlineButton("保存覆盖并重进") {
                ServerConfig.setDebugOverride(context, debugUrl.ifBlank { null })
                onLogout()
            }
        }

        SectionTitle("退出")
        DlOutlineButton("退出登录") {
            scope.launch {
                try { Session.repo.logout() } catch (_: Exception) { }
                Session.logoutLocal()
                onLogout()
            }
        }
        TextButton(onClick = onOpenShift) { Text("去选择班制") }
        ErrorLine(error)
        Text("Powered by 多邻灶 · Version: ${Updater.localVersion(context).second}", style = MaterialTheme.typography.bodySmall)
    }
}

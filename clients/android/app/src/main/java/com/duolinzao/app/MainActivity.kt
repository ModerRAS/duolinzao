package com.duolinzao.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import com.duolinzao.app.api.ApiClient
import com.duolinzao.app.api.CookieStore
import com.duolinzao.app.api.MockRepository
import com.duolinzao.app.api.NetworkRepository
import com.duolinzao.app.api.ServerConfig
import com.duolinzao.app.api.Session
import com.duolinzao.app.ui.CanteenShellScreen
import com.duolinzao.app.ui.DlTheme
import com.duolinzao.app.ui.LoginScreen
import com.duolinzao.app.ui.RegisterScreen
import com.duolinzao.app.ui.ShellScreen

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        ServerConfig.load(this)
        val cookies = CookieStore(this)
        Session.cookies = cookies
        Session.repo = if (ServerConfig.useMock) {
            MockRepository()
        } else {
            val api = ApiClient(ServerConfig.effectiveBase(), cookies) {
                Session.logoutLocal()
            }
            NetworkRepository(api)
        }
        // 新建 Activity（非旋转重建）：清掉同一进程上一个 Activity 留下的内存会话。
        // 否则 Session 单例跨仪器化用例复用，第二个冒烟用例会跳过登录直进 shell。
        if (savedInstanceState == null) Session.setMe(null)
        setContent {
            DlTheme {
                Root(
                    onLogout = {
                        recreate()
                    },
                )
            }
        }
    }
}

@Composable
private fun Root(onLogout: () -> Unit) {
    val nav = rememberNavController()
    val me by Session.me.collectAsState()
    var checked by remember { mutableStateOf(false) }
    var bootError by remember { mutableStateOf<String?>(null) }
    // 双角色切换只改本地导航根，不重登。
    var forceEmployee by remember { mutableStateOf(false) }
    val inWorkbench = me?.hasWorkbench() == true && !forceEmployee

    LaunchedEffect(Unit) {
        if (ServerConfig.useMock) {
            checked = true
            return@LaunchedEffect
        }
        if (!ServerConfig.valid()) {
            bootError = "安装包未绑定有效服务器"
            checked = true
            return@LaunchedEffect
        }
        try {
            Session.setMe(Session.repo.me())
        } catch (_: Exception) {
            // 未登录：留在登录页
        }
        checked = true
    }

    if (!checked) {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Text("多邻灶") }
        return
    }
    if (bootError != null) {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Text(bootError!!) }
        return
    }
    val current = me
    val needsFactor = current != null && !current.has_password && !current.has_passkey
    NavHost(
        nav,
        startDestination = when {
            needsFactor -> "register"
            current != null -> "shell"
            else -> "login"
        },
    ) {
        composable("login") {
            LoginScreen(
                onDone = { nav.navigate("shell") { popUpTo("login") { inclusive = true } } },
                onRegister = { nav.navigate("register") },
            )
        }
        composable("register") {
            RegisterScreen(
                // 从登录页推进来，或冷启动停在注册页，进入后都清掉后面的返回栈。
                onDone = { nav.navigate("shell") { popUpTo(nav.graph.id) { inclusive = true } } },
                onBack = { nav.popBackStack() },
            )
        }
        composable("shell") {
            if (inWorkbench) {
                CanteenShellScreen(
                    onLogout = onLogout,
                    onSwitchEmployee = { forceEmployee = true },
                )
            } else {
                ShellScreen(
                    onLogout = onLogout,
                    onSwitchWorkbench = if (me?.hasWorkbench() == true) ({
                        forceEmployee = false
                        nav.navigate("shell") { popUpTo("shell") { inclusive = true } }
                    }) else null,
                )
            }
        }
    }
}

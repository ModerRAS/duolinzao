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
    NavHost(nav, startDestination = if (me != null) "shell" else "login") {
        composable("login") {
            LoginScreen(
                onDone = { nav.navigate("shell") { popUpTo("login") { inclusive = true } } },
                onRegister = { nav.navigate("register") },
            )
        }
        composable("register") {
            RegisterScreen(
                onDone = { nav.navigate("shell") { popUpTo("login") { inclusive = true } } },
                onBack = { nav.popBackStack() },
            )
        }
        composable("shell") { ShellScreen(onLogout = onLogout) }
    }
}

package com.duolinzao.app.ui

import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController

/** 员工壳：底部三 Tab（报餐 | 评分 | 我的）；班制页为子页。 */
@Composable
fun ShellScreen(onLogout: () -> Unit) {
    val nav = rememberNavController()
    var tab by remember { mutableStateOf(0) }
    Scaffold(
        bottomBar = {
            NavigationBar {
                NavigationBarItem(selected = tab == 0, onClick = { tab = 0; nav.navigate("meals") }, icon = {}, label = { Text("报餐") })
                NavigationBarItem(selected = tab == 1, onClick = { tab = 1; nav.navigate("score") }, icon = {}, label = { Text("评分") })
                NavigationBarItem(selected = tab == 2, onClick = { tab = 2; nav.navigate("mine") }, icon = {}, label = { Text("我的") })
            }
        },
    ) { padding ->
        NavHost(nav, startDestination = "meals", modifier = Modifier.padding(padding)) {
            composable("meals") {
                MealsScreen(onOpenShift = { nav.navigate("shift") }, onOpenCanteens = { tab = 2; nav.navigate("mine") })
            }
            composable("score") { ScoreScreen() }
            composable("mine") { MineScreen(onLogout = onLogout, onOpenShift = { nav.navigate("shift") }) }
            composable("shift") { ShiftScreen(onBack = { nav.popBackStack() }) }
        }
    }
}

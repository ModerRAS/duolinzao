package com.duolinzao.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Card
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import com.duolinzao.app.api.HomeBoard
import com.duolinzao.app.api.MealList
import com.duolinzao.app.api.Session
import com.duolinzao.app.api.addDays
import com.duolinzao.app.api.shanghaiToday
import kotlinx.coroutines.launch

private val SLOTS = listOf("breakfast", "lunch", "dinner")

/** 报餐 Tab：首页 + 本周报餐。绿色=报，橙色=不报，点按切换写回。 */
@Composable
fun MealsScreen(onOpenShift: () -> Unit, onOpenCanteens: () -> Unit) {
    val scope = rememberCoroutineScope()
    var home by remember { mutableStateOf<HomeBoard?>(null) }
    var week by remember { mutableStateOf<MealList?>(null) }
    var canteenName by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }

    fun refresh() {
        scope.launch {
            try {
                home = Session.repo.home()
                val today = shanghaiToday()
                week = Session.repo.meals(today, addDays(today, 6))
                canteenName = Session.repo.myCanteens().firstOrNull()?.name
                error = null
            } catch (e: Exception) {
                error = e.message ?: "加载失败"
            }
        }
    }
    LaunchedEffect(Unit) { refresh() }

    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp)) {
        Text("本周报餐", style = MaterialTheme.typography.headlineSmall)
        if (canteenName != null) Text("已绑食堂：$canteenName（跨食堂按所选生效）", style = MaterialTheme.typography.bodySmall)
        Text("次日三餐将按北京时间提前一天锁定", style = MaterialTheme.typography.bodySmall, color = Color.Gray)

        val next = home?.next
        if (next != null) {
            Spacer(Modifier.height(8.dp))
            Card(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(12.dp)) {
                    Text("下一餐：${next.date} ${slotName(next.slot)}", style = MaterialTheme.typography.titleSmall)
                    Text(next.canteen_name ?: "未选食堂", style = MaterialTheme.typography.bodyMedium)
                    next.menu.forEach { (kind, dishes) ->
                        if (dishes.isNotEmpty()) Text("$kind：${dishes.joinToString("、")}", style = MaterialTheme.typography.bodySmall)
                    }
                }
            }
        }

        Spacer(Modifier.height(8.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            TextButton(onClick = onOpenShift) { Text("选择班制 / 模板") }
            TextButton(onClick = onOpenCanteens) { Text("绑定食堂") }
            TextButton(onClick = { refresh() }) { Text("刷新") }
        }

        (home?.days ?: emptyList()).forEach { day ->
            val date = day.date
            Card(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
                Column(Modifier.padding(12.dp)) {
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                        Text(date + weekdayOf(date), style = MaterialTheme.typography.titleSmall)
                        if (day.locked) Text("已锁定", color = Color.Gray)
                    }
                    SLOTS.forEach { slot ->
                        val item = week?.items?.find { it.date == date && it.slot == slot }
                        val ordering = item?.ordering == true
                        val locked = item?.locked == true || day.locked
                        val color = when {
                            locked -> mealColor("locked")
                            ordering -> mealColor("green")
                            else -> mealColor("orange")
                        }
                        Row(
                            Modifier.fillMaxWidth().padding(vertical = 4.dp)
                                .background(color.copy(alpha = 0.15f), RoundedCornerShape(8.dp))
                                .clickable(enabled = !locked) {
                                    scope.launch {
                                        try {
                                            val canteenId = Session.repo.myCanteens().firstOrNull()?.id
                                            Session.repo.putMeal(date, slot, !ordering, canteenId)
                                            refresh()
                                        } catch (e: Exception) {
                                            error = e.message ?: "修改失败"
                                        }
                                    }
                                }.padding(8.dp),
                            horizontalArrangement = Arrangement.SpaceBetween,
                        ) {
                            Text(slotName(slot))
                            Text(if (locked) "锁定" else if (ordering) "报餐" else "不报", color = color)
                        }
                    }
                }
            }
        }
        ErrorLine(error)
    }
}

private fun weekdayOf(iso: String): String {
    return try {
        val date = java.time.LocalDate.parse(iso)
        listOf("周一", "周二", "周三", "周四", "周五", "周六", "周日")[date.dayOfWeek.value - 1]
    } catch (_: Exception) { "" }
}

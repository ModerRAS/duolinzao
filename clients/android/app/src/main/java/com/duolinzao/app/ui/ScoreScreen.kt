package com.duolinzao.app.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.duolinzao.app.api.ApiException
import com.duolinzao.app.api.ScoreTargets
import com.duolinzao.app.api.Session
import com.duolinzao.app.api.shanghaiToday
import kotlinx.coroutines.launch

/** 评分 Tab：餐后一周内可评，菜/饭/汤分开，−2～+2，可选匿名评语≤300字。不展示排行。 */
@Composable
fun ScoreScreen() {
    val scope = rememberCoroutineScope()
    var date by remember { mutableStateOf(shanghaiToday()) }
    var slot by remember { mutableStateOf("lunch") }
    var targets by remember { mutableStateOf<ScoreTargets?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var values by remember { mutableStateOf<Map<String, Int>>(emptyMap()) }
    var comments by remember { mutableStateOf<Map<String, String>>(emptyMap()) }
    var anonymous by remember { mutableStateOf(true) }

    suspend fun load() {
        try {
            targets = Session.repo.scoreTargets(date, slot)
            error = null
        } catch (e: Exception) {
            error = e.message ?: "加载失败"
        }
    }
    fun refresh() {
        scope.launch { load() }
    }
    LaunchedEffect(date, slot) { refresh() }

    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp)) {
        Text("评分", style = MaterialTheme.typography.headlineSmall)
        Text("仅对已报过的餐、餐后一周内可评", style = MaterialTheme.typography.bodySmall)
        Row {
            listOf("breakfast" to "早", "lunch" to "中", "dinner" to "晚").forEach { (s, label) ->
                Row(Modifier.selectable(selected = slot == s, onClick = { slot = s }).padding(4.dp),
                    verticalAlignment = Alignment.CenterVertically) {
                    RadioButton(selected = slot == s, onClick = { slot = s })
                    Text(label)
                }
            }
            TextButton(onClick = { refresh() }) { Text("刷新") }
        }
        DlTextField(date, { date = it }, "日期 YYYY-MM-DD")
        if (targets?.window_open == false) Text("该餐次暂不可评（未报餐或超出餐后一周）")

        targets?.groups?.forEach { (kind, dishes) ->
            if (dishes.isNotEmpty()) {
                SectionTitle(kind)
                dishes.forEach { dish ->
                    Card(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
                        Column(Modifier.padding(12.dp)) {
                            Row { Text(dish.name, style = MaterialTheme.typography.titleSmall) }
                            if (dish.taste_tags.isNotEmpty()) {
                                Text(dish.taste_tags.joinToString(" · "), style = MaterialTheme.typography.bodySmall)
                            }
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                (-2..2).forEach { v ->
                                    Row(Modifier.selectable(
                                        selected = values[dish.dish_id] == v,
                                        onClick = { values = values + (dish.dish_id to v) },
                                    ).padding(4.dp), verticalAlignment = Alignment.CenterVertically) {
                                        RadioButton(selected = values[dish.dish_id] == v, onClick = { values = values + (dish.dish_id to v) })
                                        Text("$v")
                                    }
                                }
                            }
                            DlTextField(
                                comments[dish.dish_id].orEmpty(),
                                { comments = comments + (dish.dish_id to it) },
                                "匿名评语（可选，≤300字）",
                            )
                            TextButton(onClick = {
                                scope.launch {
                                    try {
                                        val value = values[dish.dish_id]
                                            ?: throw ApiException(400, "need_value", "请先打分（−2～+2），不可只写评语")
                                        Session.repo.submitScore(date, slot, dish.dish_id, value,
                                            comments[dish.dish_id]?.takeIf { it.isNotBlank() }, anonymous)
                                        load()
                                        error = "已提交：${dish.name}"
                                    } catch (e: Exception) {
                                        error = if (e is ApiException) e.message else "提交失败"
                                    }
                                }
                            }) { Text(if (dish.scored) "已评（可覆盖）" else "提交评分") }
                        }
                    }
                }
            }
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            RadioButton(selected = anonymous, onClick = { anonymous = !anonymous })
            Text("匿名评语", Modifier.padding(start = 4.dp))
        }
        ErrorLine(error)
    }
}

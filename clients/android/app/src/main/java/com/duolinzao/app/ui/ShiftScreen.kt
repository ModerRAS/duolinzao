package com.duolinzao.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.duolinzao.app.api.ApiException
import com.duolinzao.app.api.Session
import com.duolinzao.app.api.TemplateResp
import com.duolinzao.app.api.addDays
import com.duolinzao.app.api.shanghaiToday
import kotlinx.coroutines.launch

/** 班制与模板：上二休四 / 自定义；描述生成模板；预览点按切换；套用写回个人 DSL。 */
@Composable
fun ShiftScreen(onBack: () -> Unit) {
    val scope = rememberCoroutineScope()
    var kind by remember { mutableStateOf<String?>(null) }
    var desc by remember { mutableStateOf("") }
    var preview by remember { mutableStateOf<TemplateResp?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var okMsg by remember { mutableStateOf<String?>(null) }
    var anchor by remember { mutableStateOf(shanghaiToday()) }
    var canteenId by remember { mutableStateOf<String?>(null) }
    var picked by remember { mutableStateOf<MutableMap<String, Boolean>>(mutableMapOf()) }

    suspend fun load() {
        try {
            kind = Session.repo.shift().kind
            canteenId = Session.repo.myCanteens().firstOrNull()?.id
            error = null
        } catch (e: Exception) {
            error = e.message ?: "加载失败"
        }
    }
    fun refresh() {
        scope.launch {
            load()
            okMsg = null
        }
    }
    LaunchedEffect(Unit) { refresh() }

    suspend fun setShift(selected: String) {
        try {
            val cid = canteenId ?: throw ApiException(400, "canteen_required", "请先到「我的」绑定食堂")
            // 上二休四必须带锚点日（服务端 anchor_required）；自定义不需要。
            Session.repo.putShift(selected, if (selected == "work2_rest4") anchor else null, cid)
            load()
            okMsg = "已切换班制"
        } catch (e: Exception) {
            error = if (e is ApiException) e.message else "切换失败"
        }
    }

    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp)) {
        Text("选择班制", style = MaterialTheme.typography.headlineSmall)
        Text("仅「上二休四」/「自定义」", style = MaterialTheme.typography.bodySmall)
        DlTextField(anchor, { anchor = it }, "锚点日 YYYY-MM-DD（上二休四用）")
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.padding(vertical = 8.dp)) {
            Card(
                Modifier.weight(1f).clickable { scope.launch { setShift("work2_rest4") } },
            ) {
                Column(Modifier.padding(12.dp)) {
                    Text("上二休四")
                    if (kind == "work2_rest4") Text("当前", color = MaterialTheme.colorScheme.primary)
                }
            }
            Card(
                Modifier.weight(1f).clickable { scope.launch { setShift("custom") } },
            ) {
                Column(Modifier.padding(12.dp)) {
                    Text("自定义排班")
                    if (kind == "custom") Text("当前", color = MaterialTheme.colorScheme.primary)
                }
            }
        }
        SuccessLine(okMsg)

        SectionTitle("用描述生成模板")
        DlTextField(desc, { desc = it }, "例如：周一周二上班吃午餐")
        DlPrimaryButton("生成并预览") {
            scope.launch {
                try {
                    val tpl = Session.repo.generateTemplate(desc)
                    preview = tpl
                    picked = tpl.preview.flatMap { day ->
                        day.slots.map { (slot, s) -> "${day.date}/$slot" to s.ordering }
                    }.toMap().toMutableMap()
                    error = null
                } catch (e: Exception) {
                    error = if (e is ApiException) e.message else "生成失败"
                }
            }
        }
        preview?.preview?.forEach { day ->
            Card(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
                Column(Modifier.padding(12.dp)) {
                    Text(day.date, style = MaterialTheme.typography.titleSmall)
                    day.slots.forEach { (slot, s) ->
                        val key = "${day.date}/$slot"
                        val ordering = picked[key] ?: s.ordering
                        Row(
                            Modifier.fillMaxWidth().padding(vertical = 4.dp)
                                .background(mealColor(if (ordering) "green" else "orange").copy(alpha = 0.15f), RoundedCornerShape(8.dp))
                                .clickable { picked[key] = !ordering }
                                .padding(8.dp),
                            horizontalArrangement = Arrangement.SpaceBetween,
                        ) {
                            Text(slotName(slot))
                            Text(if (ordering) "报餐" else "不报")
                        }
                    }
                }
            }
        }
        if (preview != null) {
            DlPrimaryButton("套用本周（写回个人 DSL）") {
                scope.launch {
                    try {
                        val id = preview!!.id ?: throw ApiException(400, "no_template", "暂无模板可套用")
                        picked.forEach { (key, ordering) ->
                            val date = key.substringBefore("/")
                            val slot = key.substringAfter("/")
                            Session.repo.putTemplateSlot(id, date, slot, ordering)
                        }
                        val today = shanghaiToday()
                        Session.repo.applyTemplate(id, today, addDays(today, 6))
                        error = "已套用"
                    } catch (e: Exception) {
                        error = if (e is ApiException) e.message else "套用失败"
                    }
                }
            }
            Text("预览绿色=报、橙色=不报，点按即改（套用后以预览为准）", style = MaterialTheme.typography.bodySmall)
        }
        ErrorLine(error)
    }
}

package com.duolinzao.app.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.Checkbox
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.duolinzao.app.api.ApiException
import com.duolinzao.app.api.Canteen
import com.duolinzao.app.api.Candidate
import com.duolinzao.app.api.Dish
import com.duolinzao.app.api.Session
import com.duolinzao.app.api.addDays
import com.duolinzao.app.api.shanghaiToday
import kotlinx.coroutines.launch

private val KINDS = listOf("菜", "饭", "汤")
private val SLOT_IDS = listOf("breakfast", "lunch", "dinner")
private val SLOT_NAMES = mapOf("breakfast" to "早餐", "lunch" to "午餐", "dinner" to "晚餐")
private val SLOT_SHORT = mapOf("breakfast" to "早", "lunch" to "中", "dinner" to "晚")

/** 偏好色阶：更高更绿，更低更红；只看 tone，不看口味标签（与 Web toneColor 同规则）。 */
private fun toneColor(tone: String): Color = when (tone) {
    "green" -> Color(0xFF2D8A45)
    "lime" -> Color(0xFF7CB342)
    "amber" -> Color(0xFFE6A23C)
    "red" -> Color(0xFFE53935)
    else -> Color(0xFFD1D5DB)
}

/**
 * 食堂竖屏壳：底栏四 Tab（人数 | 选菜 | 菜谱 | 更多）。
 * 顶栏食堂范围为壳级状态（S1），人数/选菜/菜谱共用同一 canteenId，切换后重拉。
 * S1_NONE：管理范围为空时各 Tab 进空态「没有可管理的食堂」。
 */
@Composable
fun CanteenShellScreen(onLogout: () -> Unit, onSwitchEmployee: () -> Unit) {
    var tab by remember { mutableStateOf(0) }
    // 壳级食堂范围（S1）：Single/Multi 默认首个；空为 S1_NONE。
    val me = Session.me.collectAsState().value
    val scope = rememberCoroutineScope()
    var canteens by remember { mutableStateOf<List<Canteen>>(emptyList()) }
    var picked by remember { mutableStateOf("") }
    var scopeLoaded by remember { mutableStateOf(false) }
    LaunchedEffect(me) {
        scope.launch {
            try {
                val all = Session.repo.allCanteens()
                val ids = me?.roles?.flatMap { it.canteen_ids }?.toSet()
                canteens = if (me?.roles?.any { it.role == "super_admin" } == true || ids.isNullOrEmpty()) all
                    else all.filter { ids.contains(it.id) }
            } catch (_: Exception) {
                canteens = emptyList()
            }
            if (picked.isEmpty()) picked = canteens.firstOrNull()?.id.orEmpty()
            scopeLoaded = true
        }
    }
    Scaffold(
        bottomBar = {
            NavigationBar {
                NavigationBarItem(selected = tab == 0, onClick = { tab = 0 }, icon = {}, label = { Text("人数") }, modifier = Modifier.testTag("tab_headcount"))
                NavigationBarItem(selected = tab == 1, onClick = { tab = 1 }, icon = {}, label = { Text("选菜") }, modifier = Modifier.testTag("tab_optimize"))
                NavigationBarItem(selected = tab == 2, onClick = { tab = 2 }, icon = {}, label = { Text("菜谱") }, modifier = Modifier.testTag("tab_menu"))
                NavigationBarItem(selected = tab == 3, onClick = { tab = 3 }, icon = {}, label = { Text("更多") }, modifier = Modifier.testTag("tab_more"))
            }
        },
    ) { padding ->
        Column(Modifier.padding(padding).fillMaxSize()) {
            CanteenTopBar(canteens = canteens, picked = picked, loaded = scopeLoaded, onPick = { picked = it })
            val empty = scopeLoaded && (picked.isEmpty() || canteens.isEmpty())
            when (tab) {
                0 -> HeadcountTab(canteenId = picked, empty = empty)
                1 -> OptimizeTab(canteenId = picked, empty = empty)
                2 -> MenuTab(canteenId = picked, empty = empty)
                3 -> MoreTab(onLogout = onLogout, onSwitchEmployee = onSwitchEmployee)
            }
        }
    }
}

/** 顶栏：管理范围内食堂选择器；仅 1 个时只读展示；空范围时空态提示。 */
@Composable
private fun CanteenTopBar(canteens: List<Canteen>, picked: String, loaded: Boolean, onPick: (String) -> Unit) {
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text("多邻灶 · 食堂工作台", style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
        if (!loaded) {
            Text("…", color = MaterialTheme.colorScheme.onSurfaceVariant)
        } else if (canteens.isEmpty()) {
            Text("没有可管理的食堂", color = MaterialTheme.colorScheme.error)
        } else if (canteens.size == 1) {
            Text(canteens.first().name, color = MaterialTheme.colorScheme.primary)
        } else {
            canteens.forEach {
                TextButton(onClick = { onPick(it.id) }, modifier = Modifier.testTag("canteen_pick_${it.id}")) {
                    Text(if (it.id == picked) "● ${it.name}" else it.name)
                }
            }
        }
    }
}

@Composable
private fun ScopeEmpty() {
    Column(Modifier.fillMaxSize().padding(24.dp), verticalArrangement = Arrangement.Center, horizontalAlignment = Alignment.CenterHorizontally) {
        Text("没有可管理的食堂", style = MaterialTheme.typography.titleMedium, modifier = Modifier.testTag("scope_empty"))
        Text("请联系超管分配管理范围", color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(top = 8.dp))
    }
}

@Composable
private fun HeadcountTab(canteenId: String, empty: Boolean) {
    if (empty || canteenId.isEmpty()) {
        ScopeEmpty()
        return
    }
    val scope = rememberCoroutineScope()
    var date by remember(canteenId) { mutableStateOf(shanghaiToday()) }
    var counts by remember { mutableStateOf(mapOf("breakfast" to 0, "lunch" to 0, "dinner" to 0)) }
    var locked by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var loading by remember { mutableStateOf(false) }
    fun load() {
        scope.launch {
            loading = true
            try {
                val hc = Session.repo.headcount(canteenId, date)
                counts = mapOf(
                    "breakfast" to (hc.slots["breakfast"] ?: 0),
                    "lunch" to (hc.slots["lunch"] ?: 0),
                    "dinner" to (hc.slots["dinner"] ?: 0),
                )
                locked = hc.locked
                error = null
            } catch (e: Exception) {
                error = e.message ?: "加载失败"
            } finally {
                loading = false
            }
        }
    }
    LaunchedEffect(date, canteenId) { load() }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.SpaceBetween, modifier = Modifier.fillMaxWidth()) {
            TextButton(onClick = { date = addDays(date, -1) }) { Text("‹") }
            Text(date, modifier = Modifier.testTag("headcount_date"))
            TextButton(onClick = { date = addDays(date, 1) }) { Text("›") }
        }
        if (error != null) {
            Text(error!!, color = MaterialTheme.colorScheme.error)
            Button(onClick = { load() }, modifier = Modifier.testTag("headcount_retry")) { Text("重试") }
        }
        if (loading) Text("加载中…", color = MaterialTheme.colorScheme.onSurfaceVariant)
        SLOT_IDS.forEach { slot ->
            Card(Modifier.fillMaxWidth().testTag("headcount_card_$slot")) {
                Row(Modifier.fillMaxWidth().padding(18.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.SpaceBetween) {
                    Column {
                        Text(SLOT_NAMES[slot].orEmpty(), color = MaterialTheme.colorScheme.onSurfaceVariant)
                        Text("有效报餐", style = MaterialTheme.typography.titleMedium)
                    }
                    Column(horizontalAlignment = Alignment.End) {
                        Text("${counts[slot] ?: 0}", fontSize = 40.sp, color = MaterialTheme.colorScheme.primary)
                        Text("人", color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
        }
        Text(if (locked) "该日三餐已锁定（一次锁定）" else "该日三餐尚未锁定", color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

/** C2 菜谱（S4）：分组列表 / 新建编辑 / 行内启停（on→off 确认）。图片上传暂不支持（下一 PR）。 */
@Composable
private fun MenuTab(canteenId: String, empty: Boolean) {
    if (empty || canteenId.isEmpty()) {
        ScopeEmpty()
        return
    }
    val scope = rememberCoroutineScope()
    var groups by remember { mutableStateOf<Map<String, List<Dish>>>(emptyMap()) }
    var kind by remember(canteenId) { mutableStateOf("菜") }
    var loading by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var editing by remember(canteenId) { mutableStateOf<Dish?>(null) }
    var creating by remember { mutableStateOf(false) }
    var confirmOff by remember { mutableStateOf<Dish?>(null) }
    fun load() {
        scope.launch {
            loading = true
            try {
                groups = Session.repo.dishes(canteenId)
                error = null
            } catch (e: Exception) {
                error = e.message ?: "加载失败"
            } finally {
                loading = false
            }
        }
    }
    LaunchedEffect(canteenId) {
        creating = false
        editing = null
        load()
    }
    if (creating || editing != null) {
        DishForm(
            initial = editing,
            canteenId = canteenId,
            onBack = { creating = false; editing = null },
            onSaved = { creating = false; editing = null; load() },
        )
        return
    }
    confirmOff?.let { dish ->
        AlertDialog(
            onDismissRequest = { confirmOff = null },
            title = { Text("停用确认") },
            text = { Text("须显式确认；强提醒不会自动停用。确定停用该菜？") },
            confirmButton = {
                TextButton(
                    onClick = {
                        confirmOff = null
                        scope.launch {
                            try {
                                Session.repo.updateDish(canteenId, dish.id, dish.name, dish.kind, dish.taste_tags, false)
                                load()
                            } catch (e: Exception) {
                                error = e.message ?: "停用失败"
                            }
                        }
                    },
                    modifier = Modifier.testTag("menu_confirm_off"),
                ) { Text("确认停用") }
            },
            dismissButton = { TextButton(onClick = { confirmOff = null }) { Text("取消") } },
        )
    }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
            Text("菜谱", style = MaterialTheme.typography.titleLarge)
            Button(onClick = { editing = null; creating = true }, modifier = Modifier.testTag("menu_new")) { Text("新建菜品") }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            KINDS.forEach {
                TextButton(onClick = { kind = it }, modifier = Modifier.testTag("menu_kind_$it")) {
                    Text(if (it == kind) "● $it" else it)
                }
            }
        }
        if (error != null) {
            Text(error!!, color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("menu_error"))
            Button(onClick = { load() }, modifier = Modifier.testTag("menu_retry")) { Text("重试") }
        }
        if (loading) Text("加载中…", color = MaterialTheme.colorScheme.onSurfaceVariant)
        (groups[kind].orEmpty()).forEach { dish ->
            Card(Modifier.fillMaxWidth().testTag("menu_row_${dish.id}")) {
                Column(Modifier.fillMaxWidth().padding(12.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
                        Text(dish.name, style = MaterialTheme.typography.titleMedium)
                        Text(
                            if (dish.enabled) "已启用" else "已停用",
                            color = if (dish.enabled) Color(0xFF2D8A45) else Color(0xFF6B7280),
                        )
                    }
                    if (dish.taste_tags.isNotEmpty()) {
                        Text(dish.taste_tags.joinToString(" · "), color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        TextButton(onClick = { editing = dish; creating = false }, modifier = Modifier.testTag("menu_edit_${dish.id}")) { Text("编辑") }
                        TextButton(
                            onClick = {
                                if (dish.enabled) {
                                    confirmOff = dish
                                } else {
                                    scope.launch {
                                        try {
                                            Session.repo.updateDish(canteenId, dish.id, dish.name, dish.kind, dish.taste_tags, true)
                                            load()
                                        } catch (e: Exception) {
                                            error = e.message ?: "启用失败"
                                        }
                                    }
                                }
                            },
                            modifier = Modifier.testTag("menu_toggle_${dish.id}"),
                        ) { Text(if (dish.enabled) "停用" else "启用") }
                    }
                }
            }
        }
        if (!loading && error == null && groups[kind].isNullOrEmpty()) {
            Text("这一组还没有菜品", color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Text("图片上传暂不支持，下一 PR 接入", color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

/** 菜品表单：新建（默认启用 on）/ 编辑；编辑页 on→off 经确认直接保存，off→on 只改本地须点保存。 */
@Composable
private fun DishForm(initial: Dish?, canteenId: String, onBack: () -> Unit, onSaved: () -> Unit) {
    val scope = rememberCoroutineScope()
    var name by remember(initial?.id, canteenId) { mutableStateOf(initial?.name.orEmpty()) }
    var kind by remember(initial?.id, canteenId) { mutableStateOf(initial?.kind?.takeIf { it in KINDS } ?: "菜") }
    var tagsText by remember(initial?.id, canteenId) { mutableStateOf(initial?.taste_tags?.joinToString(" ") .orEmpty()) }
    var enabled by remember(initial?.id, canteenId) { mutableStateOf(initial?.enabled ?: true) }
    var saving by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var confirmOff by remember { mutableStateOf(false) }
    fun doSave(nextEnabled: Boolean) {
        scope.launch {
            saving = true
            error = null
            try {
                val tags = tagsText.split(" ", "，", ",").map { it.trim() }.filter { it.isNotEmpty() }
                if (initial == null) {
                    Session.repo.createDish(canteenId, name, kind, tags, nextEnabled)
                } else {
                    Session.repo.updateDish(canteenId, initial.id, name, kind, tags, nextEnabled)
                }
                onSaved()
            } catch (e: Exception) {
                error = e.message ?: "保存失败"
            } finally {
                saving = false
            }
        }
    }
    if (confirmOff) {
        AlertDialog(
            onDismissRequest = { confirmOff = false },
            title = { Text("停用确认") },
            text = { Text("须显式确认；强提醒不会自动停用。确定停用该菜？") },
            confirmButton = {
                TextButton(onClick = { confirmOff = false; doSave(false) }, modifier = Modifier.testTag("menu_confirm_off")) { Text("确认停用") }
            },
            dismissButton = { TextButton(onClick = { confirmOff = false }) { Text("取消") } },
        )
    }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(if (initial == null) "新建菜品" else "编辑菜品", style = MaterialTheme.typography.titleLarge)
        if (error != null) Text(error!!, color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("menu_form_error"))
        TextField(value = name, onValueChange = { name = it }, label = { Text("名称") }, modifier = Modifier.fillMaxWidth().testTag("menu_form_name"))
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            Text("类别")
            KINDS.forEach {
                TextButton(onClick = { kind = it }, modifier = Modifier.testTag("menu_form_kind_$it")) {
                    Text(if (it == kind) "● $it" else it)
                }
            }
        }
        TextField(value = tagsText, onValueChange = { tagsText = it }, label = { Text("口味标签（空格分隔）") }, modifier = Modifier.fillMaxWidth().testTag("menu_form_tags"))
        if (initial == null) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("启用")
                Switch(checked = enabled, onCheckedChange = { enabled = it }, modifier = Modifier.testTag("menu_form_enabled"))
                Text("默认启用", color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        } else {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("当前${if (enabled) "已启用" else "已停用"}")
                if (enabled) {
                    TextButton(onClick = { confirmOff = true }, modifier = Modifier.testTag("menu_form_disable")) { Text("停用") }
                } else {
                    TextButton(onClick = { enabled = true }, modifier = Modifier.testTag("menu_form_enable")) { Text("启用（点保存生效）") }
                }
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = { doSave(enabled) }, enabled = !saving, modifier = Modifier.testTag("menu_form_save")) {
                Text(if (saving) "保存中…" else "保存")
            }
            TextButton(onClick = onBack, modifier = Modifier.testTag("menu_form_cancel")) { Text("取消") }
        }
    }
}

/** C3 选菜（S3）：周视图点格进当餐，勾选即时保存；下一餐只切 date/slot。 */
@Composable
private fun OptimizeTab(canteenId: String, empty: Boolean) {
    if (empty || canteenId.isEmpty()) {
        ScopeEmpty()
        return
    }
    val scope = rememberCoroutineScope()
    var mode by remember(canteenId) { mutableStateOf("week") }
    var weekDate by remember(canteenId) { mutableStateOf(shanghaiToday()) }
    var weekStart by remember { mutableStateOf("") }
    var days by remember { mutableStateOf<List<com.duolinzao.app.api.WeekDay>>(emptyList()) }
    var pickDate by remember(canteenId) { mutableStateOf(shanghaiToday()) }
    var pickSlot by remember(canteenId) { mutableStateOf("lunch") }
    var groups by remember { mutableStateOf<Map<String, List<Candidate>>>(emptyMap()) }
    var headcount by remember { mutableStateOf(0) }
    var next by remember { mutableStateOf<Pair<String, String>?>(null) }
    var loading by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var noPerm by remember { mutableStateOf(false) }
    var saving by remember { mutableStateOf(false) }
    fun loadWeek() {
        scope.launch {
            loading = true
            error = null
            noPerm = false
            try {
                val week = Session.repo.selectionWeek(canteenId, weekDate)
                weekStart = week.week_start
                days = week.days
            } catch (e: ApiException) {
                if (e.status == 403) noPerm = true
                error = e.message
            } catch (e: Exception) {
                error = e.message ?: "加载失败"
            } finally {
                loading = false
            }
        }
    }
    fun loadPick() {
        scope.launch {
            loading = true
            error = null
            noPerm = false
            try {
                val body = Session.repo.selectionSlot(canteenId, pickDate, pickSlot)
                groups = body.groups
                headcount = body.expected_headcount
                next = body.next.date to body.next.slot
            } catch (e: ApiException) {
                if (e.status == 403) noPerm = true
                error = e.message
            } catch (e: Exception) {
                error = e.message ?: "加载失败"
            } finally {
                loading = false
            }
        }
    }
    LaunchedEffect(canteenId, weekDate, mode) { if (mode == "week") loadWeek() }
    LaunchedEffect(canteenId, pickDate, pickSlot, mode) { if (mode == "pick") loadPick() }
    if (mode == "pick") {
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                TextButton(onClick = { mode = "week" }, modifier = Modifier.testTag("opt_back")) { Text("‹ 返回周视图") }
                Text("$pickDate · ${SLOT_NAMES[pickSlot].orEmpty()}", style = MaterialTheme.typography.titleMedium, modifier = Modifier.testTag("opt_pick_title"))
            }
            Text("预计就餐 $headcount 人 · 勾选即保存 · 色阶按偏好高低", color = MaterialTheme.colorScheme.onSurfaceVariant)
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                SLOT_IDS.forEach {
                    TextButton(onClick = { pickSlot = it }, modifier = Modifier.testTag("opt_slot_$it")) {
                        Text(if (it == pickSlot) "● ${SLOT_SHORT[it]}" else SLOT_SHORT[it].orEmpty())
                    }
                }
                TextButton(
                    onClick = {
                        val n = next ?: return@TextButton
                        pickDate = n.first
                        pickSlot = n.second
                    },
                    modifier = Modifier.testTag("opt_next"),
                ) { Text("下一餐 ›") }
            }
            if (noPerm) Text("没有选菜权限", color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("opt_noperm"))
            if (error != null) {
                Text(error!!, color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("opt_error"))
                Button(onClick = { loadPick() }, modifier = Modifier.testTag("opt_retry")) { Text("重试") }
            }
            if (loading) Text("加载中…", color = MaterialTheme.colorScheme.onSurfaceVariant)
            if (saving) Text("保存中…", color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.testTag("opt_saving"))
            KINDS.forEach { k ->
                Text(k, style = MaterialTheme.typography.titleMedium)
                (groups[k].orEmpty()).forEach { item ->
                    Card(
                        Modifier.fillMaxWidth().testTag("opt_dish_${item.dish_id}"),
                        border = androidx.compose.foundation.BorderStroke(2.dp, toneColor(item.tone)),
                    ) {
                        Row(Modifier.fillMaxWidth().padding(12.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            Checkbox(
                                checked = item.selected,
                                onCheckedChange = {
                                    scope.launch {
                                        saving = true
                                        error = null
                                        try {
                                            val ids = KINDS.flatMap { kk -> groups[kk].orEmpty() }
                                                .filter { c -> if (c.dish_id == item.dish_id) !c.selected else c.selected }
                                                .map { c -> c.dish_id }
                                            val body = Session.repo.putSelection(canteenId, pickDate, pickSlot, ids)
                                            groups = body.groups
                                            headcount = body.expected_headcount
                                            next = body.next.date to body.next.slot
                                        } catch (e: Exception) {
                                            error = e.message ?: "保存失败"
                                        } finally {
                                            saving = false
                                        }
                                    }
                                },
                            )
                            Column(Modifier.weight(1f)) {
                                Text(item.name, style = MaterialTheme.typography.titleMedium)
                                if (item.taste_tags.isNotEmpty()) {
                                    Text(item.taste_tags.joinToString(" · "), color = MaterialTheme.colorScheme.onSurfaceVariant)
                                }
                            }
                        }
                    }
                }
                if (groups[k].isNullOrEmpty()) Text("这一组没有启用的餐品", color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        return
    }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
            Text("选菜优化", style = MaterialTheme.typography.titleLarge)
            Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                TextButton(onClick = { weekDate = addDays(if (weekStart.isEmpty()) weekDate else weekStart, -7) }, modifier = Modifier.testTag("opt_prev_week")) { Text("‹ 上周") }
                TextButton(onClick = { weekDate = addDays(if (weekStart.isEmpty()) weekDate else weekStart, 7) }, modifier = Modifier.testTag("opt_next_week")) { Text("下周 ›") }
            }
        }
        Text("${if (weekStart.isEmpty()) "本周" else weekStart} 起 · 周一到周日 · 点某一餐进入选定", color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.testTag("opt_week_anchor"))
        if (noPerm) Text("没有选菜权限", color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("opt_noperm"))
        if (error != null) {
            Text(error!!, color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("opt_error"))
            Button(onClick = { loadWeek() }, modifier = Modifier.testTag("opt_retry")) { Text("重试") }
        }
        if (loading) Text("加载中…", color = MaterialTheme.colorScheme.onSurfaceVariant)
        days.forEach { day ->
            Text(day.date, style = MaterialTheme.typography.titleMedium)
            SLOT_IDS.forEach { slot ->
                val cell = day.slots[slot]
                Card(Modifier.fillMaxWidth().testTag("opt_cell_${day.date}_$slot")) {
                    TextButton(
                        onClick = {
                            pickDate = day.date
                            pickSlot = slot
                            mode = "pick"
                        },
                    ) {
                        Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                            Text("${SLOT_NAMES[slot]} · ${cell?.expected_headcount ?: 0} 人", color = MaterialTheme.colorScheme.onSurfaceVariant)
                            KINDS.forEach { k ->
                                val names = cell?.groups?.get(k).orEmpty()
                                Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                                    Text(k, color = Color(0xFF9CA3AF))
                                    if (names.isEmpty()) Text("未选", color = Color(0xFF9CA3AF))
                                    else names.forEach { Text(it.name, color = toneColor(it.tone)) }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun MoreTab(onLogout: () -> Unit, onSwitchEmployee: () -> Unit) {
    val me = Session.me.collectAsState().value
    val scope = rememberCoroutineScope()
    val canEmployee = me?.permissions?.any { it == "meal.self_read" || it == "shift.self_write" } == true
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text("账户：${me?.personName().orEmpty()}", style = MaterialTheme.typography.titleMedium)
        Text("以下入口下一 PR 接入（C4/C5）：", color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.testTag("more_next"))
        listOf("强提醒", "评分汇总", "评语", "截止后改单", "邀请码").forEach {
            Text("$it（下一 PR）", color = Color(0xFF9CA3AF))
        }
        if (canEmployee) {
            TextButton(onClick = onSwitchEmployee, modifier = Modifier.testTag("canteen_to_employee")) { Text("切换到员工端") }
        }
        TextButton(onClick = {
            scope.launch {
                try { Session.repo.logout() } catch (_: Exception) { }
                Session.logoutLocal()
                onLogout()
            }
        }, modifier = Modifier.testTag("canteen_logout")) { Text("退出登录") }
    }
}

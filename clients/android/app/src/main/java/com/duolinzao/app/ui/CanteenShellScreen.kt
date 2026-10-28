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
import com.duolinzao.app.api.AdminMealRow
import com.duolinzao.app.api.AlertRow
import com.duolinzao.app.api.Canteen
import com.duolinzao.app.api.Candidate
import com.duolinzao.app.api.CommentRow
import com.duolinzao.app.api.Dish
import com.duolinzao.app.api.Invite
import com.duolinzao.app.api.ScoreSummary
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
    // 强提醒「去停用」深链：dishId，菜谱消费后清零。
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
    var menuDeepLink by remember { mutableStateOf<String?>(null) }
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
                2 -> MenuTab(canteenId = picked, empty = empty, openDishId = menuDeepLink, onOpenConsumed = { menuDeepLink = null })
                3 -> MoreTab(canteenId = picked, empty = empty, onLogout = onLogout, onSwitchEmployee = onSwitchEmployee, onGoToDish = { menuDeepLink = it; tab = 2 })
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

/** C2 菜谱（S4）：分组列表 / 新建编辑 / 行内启停（on→off 确认）。图片上传暂不支持（下一 PR）。
 * openDishId：强提醒「去停用」深链，命中后进编辑并聚焦启用开关。 */
@Composable
private fun MenuTab(canteenId: String, empty: Boolean, openDishId: String?, onOpenConsumed: () -> Unit) {
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
    var fromAlert by remember(canteenId) { mutableStateOf(false) }
    var deepLinked by remember(canteenId) { mutableStateOf(false) }
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
        fromAlert = false
        load()
    }
    // S5_ALERT 深链：菜品列表就绪后按 dishId 进编辑。
    LaunchedEffect(openDishId, groups) {
        val id = openDishId
        if (id == null || deepLinked) return@LaunchedEffect
        val found = groups.values.flatten().find { it.id == id }
        if (found != null) {
            editing = found
            creating = false
            fromAlert = true
            deepLinked = true
            onOpenConsumed()
        }
    }
    if (creating || editing != null) {
        DishForm(
            initial = editing,
            canteenId = canteenId,
            fromAlert = fromAlert,
            onBack = { creating = false; editing = null; fromAlert = false },
            onSaved = { creating = false; editing = null; fromAlert = false; load() },
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
private fun DishForm(initial: Dish?, canteenId: String, fromAlert: Boolean, onBack: () -> Unit, onSaved: () -> Unit) {
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
        if (fromAlert) Text("从强提醒进入：请检查启用开关", color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("menu_form_from_alert"))
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

/** S5_MORE：各子项按权限码显隐，空组不出现。 */
@Composable
private fun MoreTab(canteenId: String, empty: Boolean, onLogout: () -> Unit, onSwitchEmployee: () -> Unit, onGoToDish: (String) -> Unit) {
    var route by remember { mutableStateOf("root") }
    when (route) {
        "alert" -> AlertTab(canteenId = canteenId, empty = empty, onBack = { route = "root" }, onGoToDish = { route = "root"; onGoToDish(it) })
        "rating" -> RatingTab(canteenId = canteenId, empty = empty, onBack = { route = "root" })
        "comment" -> CommentTab(canteenId = canteenId, empty = empty, onBack = { route = "root" })
        "override" -> OverrideTab(canteenId = canteenId, empty = empty, onBack = { route = "root" })
        "invite" -> InviteTab(onBack = { route = "root" })
        else -> MoreRoot(canteenId = canteenId, onOpen = { route = it }, onLogout = onLogout, onSwitchEmployee = onSwitchEmployee)
    }
}

@Composable
private fun MoreRoot(canteenId: String, onOpen: (String) -> Unit, onLogout: () -> Unit, onSwitchEmployee: () -> Unit) {
    val me = Session.me.collectAsState().value
    val scope = rememberCoroutineScope()
    val perms = me?.permissions.orEmpty()
    val roles = me?.roles.orEmpty()
    val isSuper = roles.any { it.role == "super_admin" }
    val canAlert = isSuper || roles.any { it.role == "canteen" && it.canteen_ids.contains(canteenId) }
    val canEmployee = perms.any { it == "meal.self_read" || it == "shift.self_write" }
    val entries = buildList {
        if (canAlert) add(Triple("alert", "强提醒", "more_entry_alert"))
        if (perms.contains("rating.admin_read")) add(Triple("rating", "评分汇总", "more_entry_rating"))
        if (perms.contains("comment.admin_read")) add(Triple("comment", "评语", "more_entry_comment"))
        if (perms.contains("meal.admin_override")) add(Triple("override", "截止后改单", "more_entry_override"))
        if (perms.contains("user.invite")) add(Triple("invite", "邀请码", "more_entry_invite"))
    }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text("账户：${me?.personName().orEmpty()}", style = MaterialTheme.typography.titleMedium)
        entries.forEach { (route, label, tag) ->
            TextButton(onClick = { onOpen(route) }, modifier = Modifier.testTag(tag)) { Text(label) }
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

@Composable
private fun SubBack(onBack: () -> Unit) {
    TextButton(onClick = onBack, modifier = Modifier.testTag("more_back")) { Text("‹ 返回更多") }
}

/** C4 强提醒（S5_ALERT）：均分偏低列表；「去停用」深链进菜谱编辑，列表不直接下架。 */
@Composable
private fun AlertTab(canteenId: String, empty: Boolean, onBack: () -> Unit, onGoToDish: (String) -> Unit) {
    val scope = rememberCoroutineScope()
    var groups by remember { mutableStateOf(emptyMap<String, List<AlertRow>>()) }
    var threshold by remember { mutableStateOf(0.0) }
    var kind by remember(canteenId) { mutableStateOf("菜") }
    var loading by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    fun load() {
        scope.launch {
            loading = true
            try {
                val body = Session.repo.alerts(canteenId)
                groups = body.groups
                threshold = body.threshold
                error = null
            } catch (e: Exception) {
                error = e.message ?: "加载失败"
            } finally {
                loading = false
            }
        }
    }
    LaunchedEffect(canteenId) { load() }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SubBack(onBack)
            Text("强提醒", style = MaterialTheme.typography.titleLarge)
        }
        if (empty || canteenId.isEmpty()) {
            ScopeEmpty()
            return@Column
        }
        Text("强提醒：以下菜品可见均分低于当前阈值。系统不自动下架，请人工核查。", color = Color(0xFFC62828))
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            Text("当前阈值 ≥ ${"%.2f".format(threshold)}", color = MaterialTheme.colorScheme.onSurfaceVariant)
            TextButton(onClick = { load() }, modifier = Modifier.testTag("alert_refresh")) { Text("刷新") }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            KINDS.forEach {
                TextButton(onClick = { kind = it }, modifier = Modifier.testTag("alert_kind_$it")) {
                    Text(if (it == kind) "● $it" else it)
                }
            }
        }
        if (error != null) {
            Text(error!!, color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("alert_error"))
            Button(onClick = { load() }, modifier = Modifier.testTag("alert_retry")) { Text("重试") }
        }
        if (loading) Text("加载中…", color = MaterialTheme.colorScheme.onSurfaceVariant)
        (groups[kind].orEmpty()).forEach { row ->
            Card(Modifier.fillMaxWidth().testTag("alert_row_${row.dish_id}")) {
                Column(Modifier.fillMaxWidth().padding(12.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
                        Text(row.name, style = MaterialTheme.typography.titleMedium)
                        Text(if (row.enabled) "已启用" else "已停用", color = if (row.enabled) Color(0xFF2D8A45) else Color(0xFF6B7280))
                    }
                    Text("样本量 ${row.sample_count} · 可见均分 ${"%.2f".format(row.visible_mean)} ↓", color = Color(0xFFE53935))
                    Text(row.message.ifEmpty { "阈值以下菜品仍可继续出品，请人工评估后再处理。" }, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    TextButton(onClick = { onGoToDish(row.dish_id) }, modifier = Modifier.testTag("alert_goto_${row.dish_id}")) { Text("去停用") }
                }
            }
        }
        if (!loading && error == null && groups[kind].isNullOrEmpty()) {
            Text("暂无低于阈值的菜品", color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

/** C4 评分汇总（S5_RATING）：只读。 */
@Composable
private fun RatingTab(canteenId: String, empty: Boolean, onBack: () -> Unit) {
    val scope = rememberCoroutineScope()
    var groups by remember { mutableStateOf(emptyMap<String, List<ScoreSummary>>()) }
    var kind by remember(canteenId) { mutableStateOf("菜") }
    var loading by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    fun load() {
        scope.launch {
            loading = true
            try {
                groups = Session.repo.scoreSummaries(canteenId)
                error = null
            } catch (e: Exception) {
                error = e.message ?: "加载失败"
            } finally {
                loading = false
            }
        }
    }
    LaunchedEffect(canteenId) { load() }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SubBack(onBack)
            Text("评分汇总", style = MaterialTheme.typography.titleLarge)
        }
        if (empty || canteenId.isEmpty()) {
            ScopeEmpty()
            return@Column
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            KINDS.forEach {
                TextButton(onClick = { kind = it }, modifier = Modifier.testTag("rating_kind_$it")) {
                    Text(if (it == kind) "● $it" else it)
                }
            }
        }
        if (error != null) {
            Text(error!!, color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("rating_error"))
            Button(onClick = { load() }, modifier = Modifier.testTag("rating_retry")) { Text("重试") }
        }
        if (loading) Text("加载中…", color = MaterialTheme.colorScheme.onSurfaceVariant)
        (groups[kind].orEmpty()).forEach { row ->
            Card(Modifier.fillMaxWidth().testTag("rating_row_${row.dish_id}")) {
                Column(Modifier.fillMaxWidth().padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(row.name, style = MaterialTheme.typography.titleMedium)
                    Text("可见均分 ${row.visible_mean?.let { "%.2f".format(it) } ?: "—"} · 样本量 ${row.sample_count}", color = MaterialTheme.colorScheme.onSurfaceVariant)
                    Text("−2/−1/0/1/2：${row.counts["-2"] ?: 0} / ${row.counts["-1"] ?: 0} / ${row.counts["0"] ?: 0} / ${row.counts["1"] ?: 0} / ${row.counts["2"] ?: 0}", color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        }
        if (!loading && error == null && groups[kind].isNullOrEmpty()) {
            Text("这一组还没有评分", color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

/** C4 评语（S5_COMMENT）：只读；匿名不展示账号；无删除。 */
@Composable
private fun CommentTab(canteenId: String, empty: Boolean, onBack: () -> Unit) {
    val scope = rememberCoroutineScope()
    var groups by remember { mutableStateOf(emptyMap<String, List<CommentRow>>()) }
    var kind by remember(canteenId) { mutableStateOf("菜") }
    var loading by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    fun load() {
        scope.launch {
            loading = true
            try {
                groups = Session.repo.comments(canteenId)
                error = null
            } catch (e: Exception) {
                error = e.message ?: "加载失败"
            } finally {
                loading = false
            }
        }
    }
    LaunchedEffect(canteenId) { load() }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SubBack(onBack)
            Text("评语", style = MaterialTheme.typography.titleLarge)
        }
        if (empty || canteenId.isEmpty()) {
            ScopeEmpty()
            return@Column
        }
        Text("匿名评语不展示账号，评语不能删除。", color = MaterialTheme.colorScheme.onSurfaceVariant)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            KINDS.forEach {
                TextButton(onClick = { kind = it }, modifier = Modifier.testTag("comment_kind_$it")) {
                    Text(if (it == kind) "● $it" else it)
                }
            }
        }
        if (error != null) {
            Text(error!!, color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("comment_error"))
            Button(onClick = { load() }, modifier = Modifier.testTag("comment_retry")) { Text("重试") }
        }
        if (loading) Text("加载中…", color = MaterialTheme.colorScheme.onSurfaceVariant)
        (groups[kind].orEmpty()).forEach { row ->
            Card(Modifier.fillMaxWidth().testTag("comment_row_${row.comment_id}")) {
                Column(Modifier.fillMaxWidth().padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text("${row.dish_name} · ${row.kind} · ${row.meal_date} ${SLOT_NAMES[row.slot].orEmpty()}", style = MaterialTheme.typography.titleMedium)
                    Text(row.body)
                    Text(if (row.anonymous) "匿名" else (row.display_name?.trim()?.ifEmpty { null } ?: row.username.orEmpty()), color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        }
        if (!loading && error == null && groups[kind].isNullOrEmpty()) {
            Text("这一类还没有评语", color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

/** C5 截止后改单（S5_OVERRIDE）：载入 → 改 → 保存；失败保留参数可再存。 */
@Composable
private fun OverrideTab(canteenId: String, empty: Boolean, onBack: () -> Unit) {
    val scope = rememberCoroutineScope()
    var date by remember(canteenId) { mutableStateOf(shanghaiToday()) }
    var rows by remember { mutableStateOf<List<AdminMealRow>>(emptyList()) }
    var locked by remember { mutableStateOf(false) }
    var loading by remember { mutableStateOf(false) }
    var savingId by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    fun load() {
        scope.launch {
            loading = true
            try {
                val body = Session.repo.adminMeals(date, canteenId)
                rows = body.items
                locked = body.locked
                error = null
            } catch (e: Exception) {
                error = e.message ?: "加载失败"
            } finally {
                loading = false
            }
        }
    }
    LaunchedEffect(canteenId, date) { load() }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SubBack(onBack)
            Text("截止后改单", style = MaterialTheme.typography.titleLarge)
        }
        if (empty || canteenId.isEmpty()) {
            ScopeEmpty()
            return@Column
        }
        Text("截止后仅管理员可改。${if (locked) "该日已锁定。" else "未锁定时员工仍可自己改。"}", color = Color(0xFF246E37))
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.SpaceBetween, modifier = Modifier.fillMaxWidth()) {
            TextButton(onClick = { date = addDays(date, -1) }, modifier = Modifier.testTag("override_prev_day")) { Text("‹") }
            Text(date, modifier = Modifier.testTag("override_date"))
            TextButton(onClick = { date = addDays(date, 1) }, modifier = Modifier.testTag("override_next_day")) { Text("›") }
        }
        if (error != null) {
            Text(error!!, color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("override_error"))
            Button(onClick = { load() }, modifier = Modifier.testTag("override_retry")) { Text("重试") }
        }
        if (loading) Text("加载中…", color = MaterialTheme.colorScheme.onSurfaceVariant)
        rows.forEach { row ->
            Card(Modifier.fillMaxWidth().testTag("override_row_${row.account_id}_${row.slot}")) {
                Row(Modifier.fillMaxWidth().padding(12.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.SpaceBetween) {
                    Column(Modifier.weight(1f)) {
                        Text(row.display_name?.trim()?.ifEmpty { null } ?: row.username, style = MaterialTheme.typography.titleMedium)
                        Text("${SLOT_NAMES[row.slot].orEmpty()} · ${if (row.ordering) "报" else "不报"} · ${if (row.locked_at != null) "已锁定" else "未锁定"}", color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    TextButton(
                        onClick = {
                            scope.launch {
                                savingId = "${row.account_id}/${row.slot}"
                                error = null
                                try {
                                    Session.repo.adminPutMeal(row.account_id, row.date, row.slot, !row.ordering, canteenId)
                                    val body = Session.repo.adminMeals(date, canteenId)
                                    rows = body.items
                                    locked = body.locked
                                } catch (e: Exception) {
                                    error = e.message ?: "保存失败"
                                } finally {
                                    savingId = null
                                }
                            }
                        },
                        enabled = savingId == null,
                        modifier = Modifier.testTag("override_toggle_${row.account_id}_${row.slot}"),
                    ) { Text(if (savingId == "${row.account_id}/${row.slot}") "保存中…" else if (row.ordering) "改为不报" else "改为报") }
                }
            }
        }
        if (!loading && error == null && rows.isEmpty()) {
            Text("该日暂无报餐记录", color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

/** C5 邀请码（S6_*）：创建后明文只展示一次；作废需确认；改次数 ≥ 已用。 */
@Composable
private fun InviteTab(onBack: () -> Unit) {
    val scope = rememberCoroutineScope()
    var rows by remember { mutableStateOf<List<Invite>>(emptyList()) }
    var canteens by remember { mutableStateOf<List<Canteen>>(emptyList()) }
    var role by remember { mutableStateOf("employee") }
    var scopeIds by remember { mutableStateOf(setOf<String>()) }
    var maxUsesText by remember { mutableStateOf("1") }
    var creating by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var shown by remember { mutableStateOf<String?>(null) }
    var dialog by remember { mutableStateOf(false) }
    var editTarget by remember { mutableStateOf<Invite?>(null) }
    var editText by remember { mutableStateOf("") }
    var revokeTarget by remember { mutableStateOf<Invite?>(null) }
    val names = canteens.associate { it.id to it.name }
    fun load() {
        scope.launch {
            try {
                rows = Session.repo.invites()
                canteens = Session.repo.allCanteens()
            } catch (e: Exception) {
                error = e.message ?: "加载失败"
            }
        }
    }
    LaunchedEffect(Unit) { load() }
    fun statusOf(inv: Invite): Pair<String, Color> = when {
        inv.revoked_at != null -> "已作废" to Color(0xFF6B7280)
        inv.used_count >= inv.max_uses -> "已用完 ${inv.used_count}/${inv.max_uses}" to Color(0xFF6B7280)
        inv.used_count > 0 -> "使用中 ${inv.used_count}/${inv.max_uses}" to Color(0xFFB45309)
        else -> "未使用 0/${inv.max_uses}" to Color(0xFF2D8A45)
    }
    if (dialog && shown != null) {
        AlertDialog(
            onDismissRequest = { dialog = false; shown = null },
            title = { Text("邀请码展示") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(shown!!, modifier = Modifier.testTag("invite_code"))
                    Text("完整码仅显示一次，离开后列表不再显示全文", color = MaterialTheme.colorScheme.error)
                }
            },
            confirmButton = { TextButton(onClick = { dialog = false; shown = null }, modifier = Modifier.testTag("invite_close")) { Text("关闭") } },
        )
    }
    editTarget?.let { inv ->
        AlertDialog(
            onDismissRequest = { editTarget = null },
            title = { Text("修改可用次数") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("已用 ${inv.used_count} 次，可改成不小于 ${inv.used_count} 的次数（改到等于已用即立即用尽）")
                    TextField(value = editText, onValueChange = { editText = it }, label = { Text("可用次数") }, modifier = Modifier.testTag("invite_edit_field"))
                }
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        scope.launch {
                            val n = editText.trim().toIntOrNull() ?: 0
                            try {
                                Session.repo.updateInviteMaxUses(inv.id, n)
                                editTarget = null
                                load()
                            } catch (e: Exception) {
                                error = e.message ?: "修改失败"
                            }
                        }
                    },
                    modifier = Modifier.testTag("invite_edit_save"),
                ) { Text("保存") }
            },
            dismissButton = { TextButton(onClick = { editTarget = null }) { Text("取消") } },
        )
    }
    revokeTarget?.let { inv ->
        AlertDialog(
            onDismissRequest = { revokeTarget = null },
            title = { Text("作废邀请码") },
            text = { Text("作废后该邀请码不能再用于注册，是否继续？") },
            confirmButton = {
                TextButton(
                    onClick = {
                        revokeTarget = null
                        scope.launch {
                            try {
                                Session.repo.revokeInvite(inv.id)
                                load()
                            } catch (e: Exception) {
                                error = e.message ?: "作废失败"
                            }
                        }
                    },
                    modifier = Modifier.testTag("invite_confirm_revoke"),
                ) { Text("确认作废") }
            },
            dismissButton = { TextButton(onClick = { revokeTarget = null }) { Text("取消") } },
        )
    }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SubBack(onBack)
            Text("邀请码", style = MaterialTheme.typography.titleLarge)
        }
        Column(verticalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.fillMaxWidth()) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                TextButton(onClick = { role = "employee" }, modifier = Modifier.testTag("invite_role_employee")) { Text(if (role == "employee") "● 员工" else "员工") }
                TextButton(onClick = { role = "canteen" }, modifier = Modifier.testTag("invite_role_canteen")) { Text(if (role == "canteen") "● 食堂" else "食堂") }
            }
            TextField(value = maxUsesText, onValueChange = { maxUsesText = it }, label = { Text("可用次数") }, modifier = Modifier.fillMaxWidth().testTag("invite_max_uses"))
            Button(onClick = {
                scope.launch {
                    creating = true
                    error = null
                    try {
                        val n = maxUsesText.trim().toIntOrNull() ?: 0
                        val created = Session.repo.createInvite(n, role, if (role == "canteen") scopeIds.toList() else emptyList())
                        shown = created.code
                        dialog = true
                        load()
                    } catch (e: Exception) {
                        error = e.message ?: "发放失败"
                    } finally {
                        creating = false
                    }
                }
            }, enabled = !creating, modifier = Modifier.testTag("invite_create")) { Text("发放邀请码") }
        }
        if (role == "canteen") {
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text("食堂范围（至少选一个）", color = MaterialTheme.colorScheme.onSurfaceVariant)
                canteens.forEach { c ->
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Checkbox(checked = scopeIds.contains(c.id), onCheckedChange = { on ->
                            scopeIds = if (on) scopeIds + c.id else scopeIds - c.id
                        }, modifier = Modifier.testTag("invite_scope_${c.id}"))
                        Text(c.name)
                    }
                }
            }
        }
        if (error != null) Text(error!!, color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("invite_error"))
        rows.forEach { inv ->
            val (status, color) = statusOf(inv)
            Card(Modifier.fillMaxWidth().testTag("invite_row_${inv.id}")) {
                Column(Modifier.fillMaxWidth().padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text("完整码只在发放时出现一次", color = MaterialTheme.colorScheme.onSurfaceVariant)
                    Text("${if (inv.role == "canteen") "食堂" else "员工"} · 范围 ${if (inv.role != "canteen" || inv.canteen_ids.isEmpty()) "—" else inv.canteen_ids.map { names[it] ?: it }.joinToString("、")} · ${inv.used_count}/${inv.max_uses}", color = MaterialTheme.colorScheme.onSurfaceVariant)
                    Text(status, color = color)
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        TextButton(
                            onClick = { editTarget = inv; editText = inv.max_uses.toString() },
                            enabled = inv.revoked_at == null,
                            modifier = Modifier.testTag("invite_edit_${inv.id}"),
                        ) { Text("改次数") }
                        TextButton(
                            onClick = { revokeTarget = inv },
                            enabled = inv.revoked_at == null,
                            modifier = Modifier.testTag("invite_revoke_${inv.id}"),
                        ) { Text("作废") }
                    }
                }
            }
        }
    }
}

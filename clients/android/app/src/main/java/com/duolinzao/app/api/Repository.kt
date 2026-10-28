package com.duolinzao.app.api

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.add
import kotlinx.serialization.json.put
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter

/** 全局会话：登录后进员工壳；401 回登录并清 Cookie。 */
object Session {
    lateinit var repo: DlRepository
    var cookies: CookieStore? = null
    private val _me = MutableStateFlow<Me?>(null)
    val me: StateFlow<Me?> = _me

    fun setMe(me: Me?) { _me.value = me }
    fun logoutLocal() {
        _me.value = null
        cookies?.clear()
    }
}

interface DlRepository {
    suspend fun methods(): MethodsResp
    suspend fun loginPassword(username: String, password: String)
    suspend fun loginTotp(username: String, code: String)
    suspend fun requestEmailOtp(username: String)
    suspend fun verifyEmailOtp(username: String, code: String)
    suspend fun passkeyStart(username: String): PasskeyStartResp
    suspend fun passkeyFinish(username: String, challengeId: String, credentialJson: String)
    /** 注册成功即登录（服务端 Set-Cookie；旧服务端无 Cookie 时调用方回退密码登录）。 */
    suspend fun register(username: String, inviteCode: String, password: String)
    suspend fun me(): Me
    suspend fun logout()
    suspend fun updateDisplayName(name: String): String?
    suspend fun changePassword(current: String, next: String)
    suspend fun bindEmail(email: String)
    suspend fun totpSetup(): TotpSetup
    suspend fun totpConfirm(code: String)
    suspend fun devices(): DevicesResp
    suspend fun deleteDevice(id: String)
    suspend fun passkeyRegisterStart(): PasskeyStartResp
    suspend fun passkeyRegisterFinish(challengeId: String, credentialJson: String, friendlyName: String?)
    suspend fun deletePasskey(id: String)
    suspend fun pendingApprovals(): List<ApprovalItem>
    suspend fun allowApproval(id: String)
    suspend fun denyApproval(id: String)
    suspend fun createApproval(username: String): ApprovalCreated
    suspend fun pollApproval(id: String, pollToken: String): ApprovalPoll
    suspend fun home(): HomeBoard
    suspend fun meals(start: String, end: String): MealList
    suspend fun putMeal(date: String, slot: String, ordering: Boolean, canteenId: String?)
    suspend fun myCanteens(): List<Canteen>
    suspend fun allCanteens(): List<Canteen>
    suspend fun bindCanteens(ids: List<String>)
    suspend fun shift(): ShiftResp
    suspend fun putShift(kind: String, anchorDate: String?, canteenId: String?)
    suspend fun template(): TemplateResp
    suspend fun putTemplateSlot(id: String, date: String, slot: String, ordering: Boolean)
    suspend fun generateTemplate(description: String): TemplateResp
    suspend fun applyTemplate(id: String, start: String, end: String)
    suspend fun scoreTargets(date: String, slot: String): ScoreTargets
    suspend fun submitScore(date: String, slot: String, dishId: String, value: Int, comment: String?, anonymous: Boolean)
    suspend fun notifications(): List<Notice>
    suspend fun readNotification(id: String)
    suspend fun checkUpdate(): VersionInfo?
    /** 食堂人数：所选日期早/午/晚有效报餐人数（Web OverviewPage 同接口）。 */
    suspend fun headcount(canteenId: String, date: String): Headcount
    /** 菜谱列表：按菜/饭/汤分组（Web MenuPage 同接口）。 */
    suspend fun dishes(canteenId: String): Map<String, List<Dish>>
    suspend fun createDish(canteenId: String, name: String, kind: String, tags: List<String>, enabled: Boolean): Dish
    suspend fun updateDish(canteenId: String, dishId: String, name: String, kind: String, tags: List<String>, enabled: Boolean): Dish
    /** 选菜周视图 / 当餐 / 写回（Web OptimizePage 同接口）。 */
    suspend fun selectionWeek(canteenId: String, date: String): SelectionWeekResp
    suspend fun selectionSlot(canteenId: String, date: String, slot: String): SelectionSlotResp
    suspend fun putSelection(canteenId: String, date: String, slot: String, dishIds: List<String>): SelectionSlotResp
    /** C4 强提醒（与 AlertPage 同接口）。 */
    suspend fun alerts(canteenId: String): AlertsResp
    /** C4 评分汇总 / 评语（与 RatingPage / CommentsPage 同接口）。 */
    suspend fun scoreSummaries(canteenId: String): Map<String, List<ScoreSummary>>
    suspend fun comments(canteenId: String): Map<String, List<CommentRow>>
    /** C5 截止后改单（与 OverridePage 同接口）。 */
    suspend fun adminMeals(date: String, canteenId: String): AdminMealsResp
    suspend fun adminPutMeal(accountId: String, date: String, slot: String, ordering: Boolean, canteenId: String)
    /** C5 邀请码（与 InvitePage 同接口）。 */
    suspend fun invites(): List<Invite>
    suspend fun createInvite(maxUses: Int, role: String, canteenIds: List<String>): InviteCreated
    suspend fun updateInviteMaxUses(id: String, maxUses: Int)
    suspend fun revokeInvite(id: String)
}

private val SHANGHAI: ZoneId = ZoneId.of("Asia/Shanghai")
private val DF: DateTimeFormatter = DateTimeFormatter.ISO_LOCAL_DATE

fun shanghaiToday(): String = LocalDate.now(SHANGHAI).format(DF)
fun addDays(iso: String, days: Long): String = LocalDate.parse(iso).plusDays(days).format(DF)
/** 周一起始（与 Web weekStart 同规则）。 */
fun weekStart(iso: String): String {
    val d = LocalDate.parse(iso)
    return d.minusDays((d.dayOfWeek.value - 1).toLong()).format(DF)
}
val SLOTS = listOf("breakfast", "lunch", "dinner")
/** 下一餐：只切 date/slot（与 Web jump 同规则）。 */
fun nextSlot(date: String, slot: String): Pair<String, String> {
    val i = SLOTS.indexOf(slot)
    return if (i < 0 || i + 1 >= SLOTS.size) Pair(addDays(date, 1), SLOTS.first())
    else Pair(date, SLOTS[i + 1])
}

class NetworkRepository(private val api: ApiClient) : DlRepository {
    private val json = Json { ignoreUnknownKeys = true; explicitNulls = false }

    private inline fun <reified T> parse(raw: String): T = json.decodeFromString(raw)

    override suspend fun methods(): MethodsResp = parse(api.get("/api/session/methods"))
    override suspend fun loginPassword(username: String, password: String) {
        api.post("/api/session/password", buildJsonObject {
            put("username", username); put("password", password)
        }.toString())
    }
    override suspend fun loginTotp(username: String, code: String) {
        api.post("/api/session/totp", buildJsonObject {
            put("username", username); put("code", code)
        }.toString())
    }
    override suspend fun requestEmailOtp(username: String) {
        api.post("/api/session/email-otp/request", buildJsonObject {
            put("username", username)
        }.toString())
    }
    override suspend fun verifyEmailOtp(username: String, code: String) {
        api.post("/api/session/email-otp/verify", buildJsonObject {
            put("username", username); put("code", code)
        }.toString())
    }
    override suspend fun passkeyStart(username: String): PasskeyStartResp =
        parse(api.post("/api/session/passkey/start", buildJsonObject { put("username", username) }.toString()))
    override suspend fun passkeyFinish(username: String, challengeId: String, credentialJson: String) {
        val credential = json.parseToJsonElement(credentialJson)
        api.post("/api/session/passkey/finish", buildJsonObject {
            put("username", username); put("challenge_id", challengeId); put("credential", credential)
        }.toString())
    }
    override suspend fun register(username: String, inviteCode: String, password: String) {
        api.post("/api/register", buildJsonObject {
            put("username", username); put("invite_code", inviteCode); put("password", password)
        }.toString())
    }
    override suspend fun me(): Me = parse(api.get("/api/me"))
    override suspend fun logout() {
        try { api.post("/api/session/logout") } catch (_: Exception) { /* 掉线也清本地 */ }
    }
    override suspend fun updateDisplayName(name: String): String? {
        val raw = api.put("/api/me/display-name", buildJsonObject { put("display_name", name) }.toString())
        return try { json.parseToJsonElement(raw).jsonObject["display_name"]?.toString()?.trim('"') } catch (_: Exception) { null }
    }
    override suspend fun changePassword(current: String, next: String) {
        api.post("/api/me/password", buildJsonObject {
            put("current_password", current); put("new_password", next)
        }.toString())
    }
    override suspend fun bindEmail(email: String) {
        api.put("/api/me/email", buildJsonObject { put("email", email) }.toString())
    }
    override suspend fun totpSetup(): TotpSetup = parse(api.post("/api/me/totp/setup"))
    override suspend fun totpConfirm(code: String) {
        api.post("/api/me/totp/confirm", buildJsonObject { put("code", code) }.toString())
    }
    override suspend fun devices(): DevicesResp = parse(api.get("/api/devices"))
    override suspend fun deleteDevice(id: String) {
        api.delete("/api/devices/$id")
    }
    override suspend fun passkeyRegisterStart(): PasskeyStartResp = parse(api.post("/api/passkeys/register/start"))
    override suspend fun passkeyRegisterFinish(challengeId: String, credentialJson: String, friendlyName: String?) {
        val credential = json.parseToJsonElement(credentialJson)
        api.post("/api/passkeys/register/finish", buildJsonObject {
            put("challenge_id", challengeId); put("credential", credential)
            if (friendlyName != null) put("friendly_name", friendlyName)
        }.toString())
    }
    override suspend fun deletePasskey(id: String) {
        api.delete("/api/passkeys/$id")
    }
    override suspend fun pendingApprovals(): List<ApprovalItem> =
        parse<PendingResp>(api.get("/api/login-approvals/pending")).items
    override suspend fun allowApproval(id: String) {
        api.post("/api/login-approvals/$id/allow")
    }
    override suspend fun denyApproval(id: String) {
        api.post("/api/login-approvals/$id/deny")
    }
    override suspend fun createApproval(username: String): ApprovalCreated =
        parse(api.post("/api/login-approvals", buildJsonObject { put("username", username) }.toString()))
    override suspend fun pollApproval(id: String, pollToken: String): ApprovalPoll =
        parse(api.get("/api/login-approvals/$id?poll_token=$pollToken"))
    override suspend fun home(): HomeBoard = parse(api.get("/api/me/home"))
    override suspend fun meals(start: String, end: String): MealList =
        parse(api.get("/api/me/meals?start=$start&end=$end"))
    override suspend fun putMeal(date: String, slot: String, ordering: Boolean, canteenId: String?) {
        api.put("/api/meals/$date/$slot", buildJsonObject {
            put("ordering", ordering)
            if (canteenId != null) put("canteen_id", canteenId)
        }.toString())
    }
    override suspend fun myCanteens(): List<Canteen> =
        parse<CanteenList>(api.get("/api/me/canteens")).items
    override suspend fun allCanteens(): List<Canteen> =
        parse<CanteenList>(api.get("/api/canteens")).items
    override suspend fun bindCanteens(ids: List<String>) {
        api.put("/api/me/canteens", buildJsonObject {
            put("canteen_ids", buildJsonArray { ids.forEach { add(it) } })
        }.toString())
    }
    override suspend fun shift(): ShiftResp = parse(api.get("/api/me/shift"))
    override suspend fun putShift(kind: String, anchorDate: String?, canteenId: String?) {
        api.put("/api/me/shift", buildJsonObject {
            put("kind", kind)
            if (anchorDate != null) put("anchor_date", anchorDate)
            if (canteenId != null) put("canteen_id", canteenId)
        }.toString())
    }
    override suspend fun template(): TemplateResp = parse(api.get("/api/me/meal-template"))
    override suspend fun putTemplateSlot(id: String, date: String, slot: String, ordering: Boolean) {
        api.put("/api/meal-templates/$id/slots", buildJsonObject {
            put("date", date); put("slot", slot); put("ordering", ordering)
        }.toString())
    }
    override suspend fun generateTemplate(description: String): TemplateResp =
        parse(api.post("/api/meal-templates/generate", buildJsonObject { put("description", description) }.toString()))
    override suspend fun applyTemplate(id: String, start: String, end: String) {
        api.post("/api/meal-templates/$id/apply", buildJsonObject {
            put("start_date", start); put("end_date", end)
        }.toString())
    }
    override suspend fun scoreTargets(date: String, slot: String): ScoreTargets =
        parse(api.get("/api/me/score-targets?date=$date&slot=$slot"))
    override suspend fun submitScore(date: String, slot: String, dishId: String, value: Int, comment: String?, anonymous: Boolean) {
        api.post("/api/scores", buildJsonObject {
            put("meal_date", date); put("slot", slot); put("dish_id", dishId)
            put("value", value)
            if (comment != null) put("comment", comment)
            put("anonymous", anonymous)
        }.toString())
    }
    override suspend fun notifications(): List<Notice> =
        parse<NoticeList>(api.get("/api/notifications")).items
    override suspend fun readNotification(id: String) {
        api.post("/api/notifications/$id/read")
    }
    override suspend fun checkUpdate(): VersionInfo? {
        return null
    }
    override suspend fun headcount(canteenId: String, date: String): Headcount =
        parse(api.get("/api/canteens/$canteenId/headcount?date=$date"))
    override suspend fun dishes(canteenId: String): Map<String, List<Dish>> =
        parse<DishGroupsResp>(api.get("/api/canteens/$canteenId/dishes")).groups
    override suspend fun createDish(canteenId: String, name: String, kind: String, tags: List<String>, enabled: Boolean): Dish =
        parse(api.post("/api/canteens/$canteenId/dishes", buildJsonObject {
            put("name", name); put("kind", kind)
            put("taste_tags", buildJsonArray { tags.forEach { add(it) } })
            put("enabled", enabled)
        }.toString()))
    override suspend fun updateDish(canteenId: String, dishId: String, name: String, kind: String, tags: List<String>, enabled: Boolean): Dish =
        parse(api.put("/api/canteens/$canteenId/dishes/$dishId", buildJsonObject {
            put("name", name); put("kind", kind)
            put("taste_tags", buildJsonArray { tags.forEach { add(it) } })
            put("enabled", enabled); put("canteen_id", canteenId)
        }.toString()))
    override suspend fun selectionWeek(canteenId: String, date: String): SelectionWeekResp =
        parse(api.get("/api/canteens/$canteenId/selection/week?date=$date"))
    override suspend fun selectionSlot(canteenId: String, date: String, slot: String): SelectionSlotResp =
        parse(api.get("/api/canteens/$canteenId/selection?date=$date&slot=$slot"))
    override suspend fun putSelection(canteenId: String, date: String, slot: String, dishIds: List<String>): SelectionSlotResp =
        parse(api.put("/api/canteens/$canteenId/selection/$date/$slot", buildJsonObject {
            put("dish_ids", buildJsonArray { dishIds.forEach { add(it) } })
        }.toString()))
    override suspend fun alerts(canteenId: String): AlertsResp =
        parse(api.get("/api/canteens/$canteenId/alerts"))
    override suspend fun scoreSummaries(canteenId: String): Map<String, List<ScoreSummary>> =
        parse<ScoreSummariesResp>(api.get("/api/admin/score-summaries?canteen_id=$canteenId")).groups
    override suspend fun comments(canteenId: String): Map<String, List<CommentRow>> =
        parse<CommentsResp>(api.get("/api/admin/comments?canteen_id=$canteenId")).groups
    override suspend fun adminMeals(date: String, canteenId: String): AdminMealsResp =
        parse(api.get("/api/admin/meals?date=$date&canteen_id=$canteenId"))
    override suspend fun adminPutMeal(accountId: String, date: String, slot: String, ordering: Boolean, canteenId: String) {
        api.put("/api/admin/meals/$accountId/$date/$slot", buildJsonObject {
            put("ordering", ordering); put("canteen_id", canteenId)
        }.toString())
    }
    override suspend fun invites(): List<Invite> =
        parse<InvitesResp>(api.get("/api/admin/invites")).items
    override suspend fun createInvite(maxUses: Int, role: String, canteenIds: List<String>): InviteCreated =
        parse(api.post("/api/admin/invites", buildJsonObject {
            put("max_uses", maxUses); put("role", role)
            put("canteen_ids", buildJsonArray { canteenIds.forEach { add(it) } })
        }.toString()))
    override suspend fun updateInviteMaxUses(id: String, maxUses: Int) {
        api.put("/api/admin/invites/$id", buildJsonObject {
            put("max_uses", maxUses)
        }.toString())
    }
    override suspend fun revokeInvite(id: String) {
        api.delete("/api/admin/invites/$id")
    }

}

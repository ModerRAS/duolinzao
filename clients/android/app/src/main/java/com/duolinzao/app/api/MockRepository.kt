package com.duolinzao.app.api

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/**
 * 离线演示：server.json useMock=true 时启用，覆盖报餐主路径。
 * 对接真后端时 useMock=false，走 NetworkRepository。
 */
class MockRepository : DlRepository {
    private val json = Json { ignoreUnknownKeys = true }
    private val canteens = listOf(Canteen("c1", "一食堂"), Canteen("c2", "二食堂"))
    private var bound: MutableSet<String> = mutableSetOf("c1")
    private var authed = false
    private var shiftKind: String? = null
    private val meals: MutableMap<String, Boolean> = mutableMapOf()
    private var displayName: String? = "演示用户"
    private var email: String? = null
    private var totpOn = false
    private val notices = mutableListOf(
        Notice("n1", "shift", "欢迎来到多邻灶", "这是离线演示数据：选班制后即可看到报餐。", null, "2026-10-20T00:00:00Z"),
    )
    private val passkeys = mutableListOf(PasskeyInfo("p1", "演示手机", "2026-10-20T00:00:00Z"))

    private fun needAuth() {
        if (!authed) throw ApiException(401, "unauthorized", "请先登录")
    }

    override suspend fun methods() = MethodsResp(listOf("passkey", "totp", "email_otp", "password"), true)
    override suspend fun loginPassword(username: String, password: String) {
        if (username.isBlank() || password.isBlank()) throw ApiException(400, "login_failed", "账号或密码不正确")
        authed = true
    }
    override suspend fun loginTotp(username: String, code: String) {
        if (code.trim().length != 6) throw ApiException(400, "totp_invalid", "验证码不正确")
        authed = true
    }
    override suspend fun requestEmailOtp(username: String) { /* 一律 accepted，不发信 */ }
    override suspend fun verifyEmailOtp(username: String, code: String) {
        if (code.trim() != "123456") throw ApiException(400, "email_otp_invalid", "验证码不正确或已过期")
        authed = true
    }
    override suspend fun passkeyStart(username: String) =
        PasskeyStartResp("mock-challenge", buildJsonObject { put("mock", true) })
    override suspend fun passkeyFinish(username: String, challengeId: String, credentialJson: String) {
        authed = true
    }
    override suspend fun register(username: String, inviteCode: String, password: String) {
        if (username.isBlank() || inviteCode.isBlank() || password.length < 8) {
            throw ApiException(400, "register_invalid", "账号、邀请码或密码不符合要求")
        }
        authed = true
    }
    override suspend fun me(): Me {
        needAuth()
        return Me("mock-id", "demo", displayName, email, email != null, totpOn,
            listOf(RoleAssign("employee")), listOf("meal.self_read", "meal.self_write"), true)
    }
    override suspend fun logout() { authed = false }
    override suspend fun updateDisplayName(name: String): String? {
        needAuth()
        displayName = name.trim().ifEmpty { null }
        return displayName
    }
    override suspend fun changePassword(current: String, next: String) {
        needAuth()
        if (next.length < 8) throw ApiException(400, "password_weak", "密码至少 8 个字符")
    }
    override suspend fun bindEmail(email: String) {
        needAuth()
        if (!email.contains("@")) throw ApiException(400, "email_invalid", "邮箱格式不正确")
        this.email = email
    }
    override suspend fun totpSetup(): TotpSetup {
        needAuth()
        return TotpSetup("JBSWY3DPEHPK3PXP", "otpauth://totp/mock?secret=JBSWY3DPEHPK3PXP")
    }
    override suspend fun totpConfirm(code: String) {
        needAuth()
        totpOn = true
    }
    override suspend fun devices(): DevicesResp {
        needAuth()
        return DevicesResp(
            listOf(DeviceInfo("d1", "本机（演示）", ServerConfig.userAgent(), "10.0.2.2", "2026-10-20T00:00:00Z", true)),
            passkeys.toList(),
        )
    }
    override suspend fun deleteDevice(id: String) { needAuth() }
    override suspend fun passkeyRegisterStart(): PasskeyStartResp {
        needAuth()
        return PasskeyStartResp("mock-bind", buildJsonObject { put("mock", true) })
    }
    override suspend fun passkeyRegisterFinish(challengeId: String, credentialJson: String, friendlyName: String?) {
        needAuth()
        passkeys.add(PasskeyInfo("p${passkeys.size + 1}", friendlyName ?: "演示设备", "2026-10-20T00:00:00Z"))
    }
    override suspend fun deletePasskey(id: String) {
        needAuth()
        passkeys.removeAll { it.id == id }
    }
    override suspend fun pendingApprovals(): List<ApprovalItem> {
        needAuth()
        return emptyList()
    }
    override suspend fun allowApproval(id: String) { needAuth() }
    override suspend fun denyApproval(id: String) { needAuth() }
    override suspend fun createApproval(username: String) = ApprovalCreated("mock-appr", "mock-poll", "pending")
    override suspend fun pollApproval(id: String, pollToken: String) = ApprovalPoll("pending")
    override suspend fun home(): HomeBoard {
        needAuth()
        val today = shanghaiToday()
        val days = (0..6).map { offset ->
            val date = addDays(today, offset.toLong())
            HomeDay(date, locked = offset == 0, mapOf(
                "breakfast" to SlotState(if (meals["$date/breakfast"] == true) "ordering" else "unset"),
                "lunch" to SlotState(if (meals["$date/lunch"] == true) "ordering" else "unset"),
                "dinner" to SlotState(if (meals["$date/dinner"] == true) "ordering" else "unset"),
            ))
        }
        return HomeBoard(
            HomeNext(today, "lunch", "c1", "一食堂", mapOf("菜" to listOf("番茄炒蛋"), "饭" to listOf("米饭"), "汤" to listOf("紫菜汤"))),
            days,
        )
    }
    override suspend fun meals(start: String, end: String): MealList {
        needAuth()
        var cursor = start
        val items = mutableListOf<MealItem>()
        while (cursor <= end) {
            for (slot in listOf("breakfast", "lunch", "dinner")) {
                val key = "$cursor/$slot"
                items.add(MealItem(cursor, slot, "c1", meals[key] == true, "manual", locked = false, color = if (meals[key] == true) "green" else "orange"))
            }
            cursor = addDays(cursor, 1)
        }
        return MealList(items, emptyList())
    }
    override suspend fun putMeal(date: String, slot: String, ordering: Boolean, canteenId: String?) {
        needAuth()
        meals["$date/$slot"] = ordering
    }
    override suspend fun myCanteens(): List<Canteen> {
        needAuth()
        return canteens.filter { bound.contains(it.id) }
    }
    override suspend fun allCanteens(): List<Canteen> {
        needAuth()
        return canteens
    }
    override suspend fun bindCanteens(ids: List<String>) {
        needAuth()
        bound = ids.toMutableSet()
    }
    override suspend fun shift(): ShiftResp {
        needAuth()
        return ShiftResp(shiftKind, null, null, null)
    }
    override suspend fun putShift(kind: String, anchorDate: String?, canteenId: String?) {
        needAuth()
        shiftKind = kind
    }
    override suspend fun template(): TemplateResp {
        needAuth()
        return TemplateResp(null, null, emptyList())
    }
    override suspend fun putTemplateSlot(id: String, date: String, slot: String, ordering: Boolean) { /* 演示：直接套用 */ }
    override suspend fun generateTemplate(description: String): TemplateResp {
        needAuth()
        val today = shanghaiToday()
        val preview = (0..6).map { offset ->
            val date = addDays(today, offset.toLong())
            PreviewDay(date, false, mapOf(
                "breakfast" to PreviewSlot(offset % 3 != 2, "green", "c1", false),
                "lunch" to PreviewSlot(true, "green", "c1", false),
                "dinner" to PreviewSlot(offset % 2 == 0, if (offset % 2 == 0) "green" else "orange", "c1", false),
            ))
        }
        return TemplateResp("mock-tpl", JsonObject(emptyMap()), preview)
    }
    override suspend fun applyTemplate(id: String, start: String, end: String) {
        needAuth()
        shiftKind = "custom"
    }
    override suspend fun scoreTargets(date: String, slot: String): ScoreTargets {
        needAuth()
        val dishes = mapOf(
            "菜" to listOf(ScoreDish("d1", "番茄炒蛋", "菜", listOf("酸", "开胃"), null, false)),
            "饭" to listOf(ScoreDish("d2", "米饭", "饭", emptyList(), null, false)),
            "汤" to listOf(ScoreDish("d3", "紫菜汤", "汤", emptyList(), null, false)),
        )
        return ScoreTargets(date, slot, true, dishes)
    }
    override suspend fun submitScore(date: String, slot: String, dishId: String, value: Int, comment: String?, anonymous: Boolean) {
        needAuth()
        if (value !in -2..2) throw ApiException(400, "score_invalid", "分值只能是 -2～2")
    }
    override suspend fun notifications(): List<Notice> {
        needAuth()
        return notices.toList()
    }
    override suspend fun readNotification(id: String) {
        needAuth()
        notices.replaceAll { if (it.id == id) it.copy(read_at = "2026-10-20T00:00:00Z") else it }
    }
    override suspend fun checkUpdate(): VersionInfo? = null
}

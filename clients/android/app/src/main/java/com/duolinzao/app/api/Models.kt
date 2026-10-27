package com.duolinzao.app.api

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement

@Serializable
data class MethodsResp(
    val methods: List<String> = emptyList(),
    val email_enabled: Boolean = false,
)

@Serializable
data class RoleAssign(
    val role: String = "",
    val canteen_ids: List<String> = emptyList(),
    val source_label: String? = null,
)

@Serializable
data class Me(
    val id: String = "",
    val username: String = "",
    val display_name: String? = null,
    val email: String? = null,
    val email_verified: Boolean = false,
    val totp: Boolean = false,
    val roles: List<RoleAssign> = emptyList(),
    val permissions: List<String> = emptyList(),
    val email_enabled: Boolean = false,
) {
    fun personName(): String {
        val custom = display_name?.trim().orEmpty()
        return custom.ifEmpty { username.trim() }
    }

    /** 食堂竖屏工作台能力：任一角色命中，或权限含运营/总管码（与 Web 侧栏 show 条件同思路）。 */
    fun hasWorkbench(): Boolean {
        if (roles.any { it.role == "canteen" || it.role == "admin" || it.role == "super_admin" }) return true
        return permissions.any { it in WORKBENCH_CODES }
    }
}

val WORKBENCH_CODES = setOf(
    "menu.read", "menu.write", "optimizer.read", "meal.admin_override",
    "rating.admin_read", "comment.admin_read", "user.invite", "export.run",
    "audit.read", "system.super", "rating.strong_alert_config", "user.recover",
)

@Serializable
data class IdResp(val id: String = "")

@Serializable
data class AccountResp(val account_id: String = "", val username: String = "")

@Serializable
data class PasskeyStartResp(
    val challenge_id: String = "",
    val publicKey: JsonElement? = null,
)

@Serializable
data class ApprovalCreated(
    val id: String = "",
    val poll_token: String = "",
    val status: String = "",
)

@Serializable
data class ApprovalPoll(
    val status: String = "",
)

@Serializable
data class ApprovalItem(
    val id: String = "",
    val user_agent: String = "",
    val friendly_name: String = "",
    val ip: String = "",
    val created_at: String = "",
    val status: String = "",
)

@Serializable
data class PendingResp(val items: List<ApprovalItem> = emptyList())

@Serializable
data class DeviceInfo(
    val id: String = "",
    val friendly_name: String = "",
    val user_agent: String = "",
    val ip: String = "",
    val created_at: String = "",
    val session_active: Boolean = false,
)

@Serializable
data class PasskeyInfo(
    val id: String = "",
    val friendly_name: String? = null,
    val created_at: String = "",
)

@Serializable
data class DevicesResp(
    val devices: List<DeviceInfo> = emptyList(),
    val passkeys: List<PasskeyInfo> = emptyList(),
)

@Serializable
data class Canteen(val id: String = "", val name: String = "")

@Serializable
data class CanteenList(val items: List<Canteen> = emptyList())

@Serializable
data class CanteenIds(val canteen_ids: List<String> = emptyList())

@Serializable
data class SlotState(val state: String = "unset")

@Serializable
data class HomeDay(
    val date: String = "",
    val locked: Boolean = false,
    val slots: Map<String, SlotState> = emptyMap(),
)

@Serializable
data class HomeNext(
    val date: String = "",
    val slot: String = "",
    val canteen_id: String? = null,
    val canteen_name: String? = null,
    val menu: Map<String, List<String>> = emptyMap(),
)

@Serializable
data class HomeBoard(val next: HomeNext? = null, val days: List<HomeDay> = emptyList())

@Serializable
data class MealItem(
    val date: String = "",
    val slot: String = "",
    val canteen_id: String? = null,
    val ordering: Boolean = false,
    val source: String = "",
    val locked: Boolean = false,
    val color: String = "",
)

@Serializable
data class MealList(val items: List<MealItem> = emptyList(), val locked_dates: List<String> = emptyList())

@Serializable
data class Headcount(val locked: Boolean = false, val slots: Map<String, Int> = emptyMap())

@Serializable
data class ShiftResp(
    val kind: String? = null,
    val anchor_date: String? = null,
    val template_id: String? = null,
    val dsl: JsonElement? = null,
)

@Serializable
data class TemplateResp(
    val id: String? = null,
    val dsl: JsonElement? = null,
    val preview: List<PreviewDay> = emptyList(),
)

@Serializable
data class PreviewDay(
    val date: String = "",
    val locked: Boolean = false,
    val slots: Map<String, PreviewSlot> = emptyMap(),
)

@Serializable
data class PreviewSlot(
    val ordering: Boolean = false,
    val color: String = "",
    val canteen_id: String? = null,
    val locked: Boolean = false,
)

@Serializable
data class ScoreDish(
    val dish_id: String = "",
    val name: String = "",
    val kind: String = "",
    val taste_tags: List<String> = emptyList(),
    val image_url: String? = null,
    val scored: Boolean = false,
)

@Serializable
data class ScoreTargets(
    val date: String = "",
    val slot: String = "",
    val window_open: Boolean = false,
    val groups: Map<String, List<ScoreDish>> = emptyMap(),
)

@Serializable
data class Notice(
    val id: String = "",
    val kind: String = "",
    val title: String = "",
    val body: String = "",
    val read_at: String? = null,
    val created_at: String = "",
)

@Serializable
data class NoticeList(val items: List<Notice> = emptyList())

@Serializable
data class TotpSetup(val secret: String = "", val otpauth_url: String = "")

@Serializable
data class Dish(
    val id: String = "",
    val name: String = "",
    val kind: String = "",
    val taste_tags: List<String> = emptyList(),
    val image_url: String? = null,
    val enabled: Boolean = true,
    val canteen_id: String = "",
)

@Serializable
data class DishGroupsResp(val groups: Map<String, List<Dish>> = emptyMap())

@Serializable
data class Candidate(
    val dish_id: String = "",
    val name: String = "",
    val kind: String = "",
    val taste_tags: List<String> = emptyList(),
    val image_url: String? = null,
    val selected: Boolean = false,
    val tone: String = "",
)

@Serializable
data class NextSlot(val date: String = "", val slot: String = "")

@Serializable
data class SelectionSlotResp(
    val date: String = "",
    val slot: String = "",
    val expected_headcount: Int = 0,
    val groups: Map<String, List<Candidate>> = emptyMap(),
    val next: NextSlot = NextSlot(),
)

@Serializable
data class WeekDish(val dish_id: String = "", val name: String = "", val tone: String = "")

@Serializable
data class WeekCell(
    val expected_headcount: Int = 0,
    val groups: Map<String, List<WeekDish>> = emptyMap(),
)

@Serializable
data class WeekDay(val date: String = "", val slots: Map<String, WeekCell> = emptyMap())

@Serializable
data class SelectionWeekResp(val week_start: String = "", val days: List<WeekDay> = emptyList())

@Serializable
data class VersionInfo(
    val versionCode: Int = 0,
    val versionName: String = "",
    val url: String = "",
    val sha256: String = "",
    // 服务端标注：release（正式 keystore）/ ephemeral（CI 一次性 keystore）/ unsigned。
    // 仅展示/排查用，不参与更新判断（json 已配 ignoreUnknownKeys，缺字段也兼容）。
    val signing: String = "",
)

<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { ElMessage, ElMessageBox } from "element-plus";
import { api, explain, logout, personName } from "../api";
import { auth, can, managedCanteens, refreshSession } from "../session";
import { loadVersion, versionFooter } from "../version";
import AccountPage from "./AccountPage.vue";
import AlertPage from "./AlertPage.vue";
import AppSigningPage from "./AppSigningPage.vue";
import AuditPage from "./AuditPage.vue";
import CommentsPage from "./CommentsPage.vue";
import DevicePage from "./DevicePage.vue";
import DishEditPage from "./DishEditPage.vue";
import EmailPage from "./EmailPage.vue";
import ExportPage from "./ExportPage.vue";
import InvitePage from "./InvitePage.vue";
import LlmPage from "./LlmPage.vue";
import MenuPage from "./MenuPage.vue";
import OptimizePage from "./OptimizePage.vue";
import OverridePage from "./OverridePage.vue";
import OverviewPage from "./OverviewPage.vue";
import RatingPage from "./RatingPage.vue";
import RecoverPage from "./RecoverPage.vue";
import ThresholdPage from "./ThresholdPage.vue";

type Tab = "headcount" | "optimize" | "menu" | "more";
type Sub =
  | "tabs"
  | "dishEdit"
  | "alert"
  | "rating"
  | "comment"
  | "change"
  | "device"
  | "invite"
  | "export"
  | "audit"
  | "email"
  | "appSigning"
  | "threshold"
  | "recover"
  | "llm"
  | "account";

const emit = defineEmits<{ "to-employee": [] }>();

const tab = ref<Tab>("headcount");
const sub = ref<Sub>("tabs");
const dishId = ref<string | null>(null);
const newCanteen = ref("");

const canteens = computed(() => managedCanteens());
const showCanteen = computed(
  () => sub.value === "tabs" || sub.value === "dishEdit",
);
const canEmployee = computed(() => can("meal.self_read") || can("shift.self_write"));

interface MoreItem {
  key: Sub | "employee" | "logout";
  label: string;
  show: boolean;
}

const groups = computed((): { name: string; items: MoreItem[] }[] => {
  const feedback: MoreItem[] = [
    { key: "alert", label: "强提醒", show: can("menu.write") || can("optimizer.read") || can("system.super") },
    { key: "rating", label: "评分汇总", show: can("rating.admin_read") },
    { key: "comment", label: "评语", show: can("comment.admin_read") },
  ];
  const manage: MoreItem[] = [
    { key: "change", label: "截止后改单", show: can("meal.admin_override") },
    { key: "invite", label: "邀请码", show: can("user.invite") },
  ];
  const admin: MoreItem[] = [
    { key: "export", label: "导出", show: can("export.run") },
    { key: "audit", label: "审计", show: can("audit.read") },
    { key: "email", label: "邮箱", show: can("system.super") },
    { key: "threshold", label: "阈值", show: can("rating.strong_alert_config") },
    { key: "recover", label: "找回", show: can("user.recover") },
    { key: "llm", label: "LLM", show: false },
    { key: "appSigning", label: "应用签名", show: can("system.super") },
  ];
  const account: MoreItem[] = [
    { key: "account", label: "账户设置", show: true },
    { key: "device", label: "设备与通行密钥", show: can("device.approve") },
    { key: "employee", label: "切换到员工端", show: canEmployee.value },
    { key: "logout", label: "退出登录", show: true },
  ];
  return [
    { name: "反馈", items: feedback },
    { name: "管理", items: manage },
    { name: "总管", items: admin },
    { name: "账户", items: account },
  ]
    .map((group) => ({ ...group, items: group.items.filter((item) => item.show) }))
    .filter((group) => group.items.length > 0);
});

const titles: Record<Sub, string> = {
  tabs: "",
  dishEdit: "菜品",
  alert: "强提醒",
  rating: "评分汇总",
  comment: "评语",
  change: "截止后改单",
  device: "设备与通行密钥",
  invite: "邀请码",
  export: "导出",
  audit: "审计",
  email: "邮箱",
  appSigning: "应用签名",
  threshold: "阈值",
  recover: "找回",
  llm: "LLM",
  account: "账户设置",
};

const title = computed(() => {
  if (sub.value !== "tabs") return titles[sub.value];
  return tab.value === "headcount"
    ? "多邻灶"
    : tab.value === "optimize"
      ? "本周选菜"
      : tab.value === "menu"
        ? "菜谱"
        : "更多";
});

const subtitle = computed(() => {
  if (sub.value !== "tabs") return "点左上返回";
  return tab.value === "headcount"
    ? "食堂工作台"
    : tab.value === "optimize"
      ? "勾选立即保存"
      : tab.value === "menu"
        ? "点行编辑 · 右上新建"
        : "按权限显隐";
});

function openTab(next: Tab) {
  tab.value = next;
  sub.value = "tabs";
}

function openSub(next: Sub) {
  if (next === "dishEdit") dishId.value = null;
  sub.value = next;
}

function back() {
  if (sub.value === "dishEdit") {
    dishId.value = null;
    sub.value = "tabs";
    if (tab.value !== "menu") tab.value = "menu";
    return;
  }
  sub.value = "tabs";
}

function editDish(id: string | null) {
  dishId.value = id;
  sub.value = "dishEdit";
}

async function createCanteen() {
  const name = newCanteen.value.trim();
  if (!name) return;
  try {
    const created = await api<{ id: string }>("/api/admin/canteens", {
      method: "POST",
      body: JSON.stringify({ name }),
    });
    newCanteen.value = "";
    await refreshSession();
    auth.canteenId = created.id;
    ElMessage.success("食堂已建立");
  } catch (error) {
    ElMessage.error(explain(error));
  }
}

async function tap(item: MoreItem) {
  if (item.key === "employee") {
    emit("to-employee");
    return;
  }
  if (item.key === "logout") {
    try {
      await ElMessageBox.confirm("退出当前账号？", "退出登录", {
        confirmButtonText: "退出登录",
        cancelButtonText: "取消",
        confirmButtonType: "danger",
      });
    } catch {
      return;
    }
    try {
      await logout();
      auth.me = null;
    } catch (error) {
      ElMessage.error(explain(error));
    }
    return;
  }
  openSub(item.key);
}

onMounted(() => {
  loadVersion();
  if (!auth.canteenId && canteens.value.length > 0) {
    auth.canteenId = canteens.value[0].id;
  }
});
</script>

<template>
  <div class="portrait">
    <header class="top">
      <button v-if="sub !== 'tabs'" type="button" class="back" @click="back">‹</button>
      <div class="brand">
        <span class="logo">灶</span>
        <div>
          <h1>{{ title }}</h1>
          <p>{{ subtitle }}</p>
        </div>
      </div>
      <el-select
        v-if="showCanteen && canteens.length > 0"
        v-model="auth.canteenId"
        placeholder="选择食堂"
        class="picker"
      >
        <el-option v-for="item in canteens" :key="item.id" :label="item.name" :value="item.id" />
      </el-select>
    </header>

    <div class="body">
      <form v-if="can('system.super') && sub === 'tabs' && (canteens.length === 0 || tab === 'more')" class="create-canteen" @submit.prevent="createCanteen">
        <el-input v-model="newCanteen" placeholder="新食堂名称" @keyup.enter="createCanteen" />
        <el-button type="primary" native-type="submit">新建食堂</el-button>
      </form>
      <template v-if="sub === 'tabs'">
        <OverviewPage v-if="tab === 'headcount'" />
        <OptimizePage v-else-if="tab === 'optimize'" />
        <MenuPage v-else-if="tab === 'menu'" @edit="editDish" />
        <div v-else class="more">
          <section v-for="group in groups" :key="group.name" class="group">
            <div class="group-title">{{ group.name }}</div>
            <div class="group-card">
              <button
                v-for="item in group.items"
                :key="item.key"
                type="button"
                class="row"
                :class="{ danger: item.key === 'logout' }"
                @click="tap(item)"
              >
                <span>{{ item.label }}</span><span class="chev">›</span>
              </button>
            </div>
          </section>
          <p class="who">{{ personName(auth.me) }}</p>
        </div>
      </template>
      <DishEditPage v-else-if="sub === 'dishEdit'" :dish-id="dishId" @back="back" />
      <AlertPage v-else-if="sub === 'alert'" />
      <RatingPage v-else-if="sub === 'rating'" />
      <CommentsPage v-else-if="sub === 'comment'" />
      <OverridePage v-else-if="sub === 'change'" />
      <DevicePage v-else-if="sub === 'device'" />
      <InvitePage v-else-if="sub === 'invite'" />
      <ExportPage v-else-if="sub === 'export'" />
      <AuditPage v-else-if="sub === 'audit'" />
      <EmailPage v-else-if="sub === 'email'" />
      <ThresholdPage v-else-if="sub === 'threshold'" />
      <RecoverPage v-else-if="sub === 'recover'" />
      <LlmPage v-else-if="sub === 'llm'" />
      <AppSigningPage v-else-if="sub === 'appSigning'" />
      <AccountPage v-else-if="sub === 'account'" embedded @open-devices="openSub('device')" />
    </div>

    <nav v-if="sub === 'tabs'" class="nav">
      <button type="button" :class="{ active: tab === 'headcount' }" @click="openTab('headcount')">
        <span class="ico">👥</span>人数
      </button>
      <button type="button" :class="{ active: tab === 'optimize' }" @click="openTab('optimize')">
        <span class="ico">📅</span>选菜
      </button>
      <button type="button" :class="{ active: tab === 'menu' }" @click="openTab('menu')">
        <span class="ico">📋</span>菜谱
      </button>
      <button type="button" :class="{ active: tab === 'more' }" @click="openTab('more')">
        <span class="ico">⋯</span>更多
      </button>
    </nav>
    <div v-if="sub === 'tabs'" class="ver">{{ versionFooter() }}</div>
  </div>
</template>

<style scoped>
.portrait {
  height: 100vh; max-width: 480px; margin: 0 auto; background: #f3f5f7; color: #1a1f24;
  display: flex; flex-direction: column; overflow: hidden;
}
.top {
  display: flex; align-items: center; gap: 10px; padding: 14px 16px 10px;
  background: #f3f5f7; flex-shrink: 0;
}
.back {
  border: none; background: transparent; font-size: 26px; line-height: 1; cursor: pointer;
  color: #2d8a45; padding: 0 4px;
}
.brand { display: flex; align-items: center; gap: 10px; flex: 1; min-width: 0; }
.logo {
  width: 36px; height: 36px; border-radius: 10px; background: #2d8a45; color: #fff;
  display: grid; place-items: center; font-weight: 700; font-size: 15px; flex-shrink: 0;
}
.brand h1 { font-size: 18px; margin: 0; line-height: 1.1; }
.brand p { font-size: 12px; color: #6b7280; margin: 2px 0 0; }
.picker { width: 128px; flex-shrink: 0; }
.body { flex: 1; overflow: auto; padding: 0 16px 12px; }
.create-canteen { display: flex; gap: 8px; margin-bottom: 12px; }
.create-canteen .el-input { flex: 1; }
.body :deep(.page-title) { display: none; }
.more { display: flex; flex-direction: column; gap: 4px; padding-bottom: 8px; }
.group { margin-top: 12px; }
.group-title { font-size: 13px; color: #6b7280; margin: 0 4px 6px; }
.group-card { background: #fff; border: 1px solid #e5e8ec; border-radius: 18px; overflow: hidden; }
.row {
  width: 100%; display: flex; justify-content: space-between; align-items: center;
  border: none; border-bottom: 1px solid #eef0f3; background: #fff;
  padding: 13px 16px; font: inherit; font-size: 15px; cursor: pointer; color: #1a1f24;
}
.row:last-child { border-bottom: none; }
.row.danger { color: #c0392b; }
.chev { color: #9aa3ad; }
.who { text-align: center; color: #9aa3ad; font-size: 12px; margin: 14px 0 4px; }
.nav {
  height: 64px; background: #fff; border-top: 1px solid #e5e8ec; flex-shrink: 0;
  display: grid; grid-template-columns: repeat(4, 1fr); padding: 6px 6px 14px;
}
.nav button {
  border: none; background: transparent; color: #6b7280; cursor: pointer;
  display: flex; flex-direction: column; align-items: center; gap: 4px; font-size: 11px; font-family: inherit;
}
.nav button.active { color: #2d8a45; font-weight: 600; }
.nav .ico {
  width: 26px; height: 26px; border-radius: 8px; display: grid; place-items: center; font-size: 14px;
}
.nav button.active .ico { background: #e8f5ec; }
.ver { text-align: center; font-size: 11px; color: #9aa3ad; padding: 6px 0 10px; background: #fff; }
</style>

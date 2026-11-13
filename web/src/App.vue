<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { Bell } from "@element-plus/icons-vue";
import { ElMessage } from "element-plus";
import { api, explain, personName } from "./api";
import { useNarrow } from "./narrow";
import AccountPage from "./pages/AccountPage.vue";
import AppSigningPage from "./pages/AppSigningPage.vue";
import AlertPage from "./pages/AlertPage.vue";
import AuditPage from "./pages/AuditPage.vue";
import CommentsPage from "./pages/CommentsPage.vue";
import DevicePage from "./pages/DevicePage.vue";
import DishEditPage from "./pages/DishEditPage.vue";
import EmailPage from "./pages/EmailPage.vue";
import CanteenPortraitShell from "./pages/CanteenPortraitShell.vue";
import EmployeePage from "./pages/EmployeePage.vue";
import ExportPage from "./pages/ExportPage.vue";
import InvitePage from "./pages/InvitePage.vue";
import LlmPage from "./pages/LlmPage.vue";
import LoginPage from "./pages/LoginPage.vue";
import MenuPage from "./pages/MenuPage.vue";
import OptimizePage from "./pages/OptimizePage.vue";
import OverridePage from "./pages/OverridePage.vue";
import OverviewPage from "./pages/OverviewPage.vue";
import RatingPage from "./pages/RatingPage.vue";
import RecoverPage from "./pages/RecoverPage.vue";
import ThresholdPage from "./pages/ThresholdPage.vue";
import { auth, can, managedCanteens, refreshSession } from "./session";
import { loadVersion, versionFooter } from "./version";

// 页面清单是唯一事实源：type Page 由 ALL_PAGES 推导，避免手工维护两份。
const ALL_PAGES = [
  "overview",
  "menu",
  "dishEdit",
  "optimize",
  "alert",
  "rating",
  "comment",
  "change",
  "device",
  "invite",
  "export",
  "audit",
  "email",
  "appSigning",
  "threshold",
  "recover",
  "llm",
  "account",
  "employee",
] as const;
type Page = (typeof ALL_PAGES)[number];
// popstate 的 state 来自历史条目，先按 string 查表再收窄为 Page。
const PAGE_SET: ReadonlySet<string> = new Set(ALL_PAGES);

interface Note {
  id: string;
  title: string;
  body: string;
  read_at: string | null;
}

const page = ref<Page>("overview");
const dishId = ref<string | null>(null);
const notesOpen = ref(false);
const notes = ref<Note[]>([]);
const newCanteen = ref("");

const landscapeCodes = [
  "menu.read",
  "menu.write",
  "optimizer.read",
  "meal.admin_override",
  "rating.admin_read",
  "comment.admin_read",
  "user.invite",
  "export.run",
  "audit.read",
  "system.super",
  "rating.strong_alert_config",
  "user.recover",
];

const hasLandscape = computed(() => landscapeCodes.some((code) => can(code)));

// 工作台能力：角色任一命中，或权限含运营/总管码（与横屏侧栏 show 条件同思路）。
// 纯员工（仅 meal.self_* / shift）无工作台，走员工竖屏。
const WORKBENCH_ROLES = ["canteen", "admin", "super_admin"];
const hasWorkbench = computed(
  () => auth.me?.roles.some((item) => WORKBENCH_ROLES.includes(item.role)) || hasLandscape.value,
);
// 注册已发会话，但还没有通行密钥也没有密码时，不能进业务界面。
const needsFactor = computed(
  () => Boolean(auth.me && auth.me.has_password === false && auth.me.has_passkey === false),
);

// 窄屏只看 viewport（matchMedia），不用 UA；断点见 narrow.ts 注释。
const isNarrow = useNarrow();
const portraitEmployee = ref(false);
const canteens = computed(() => managedCanteens());
const needsCanteen = computed(() =>
  ["overview", "menu", "dishEdit", "optimize", "alert", "rating", "comment", "change"].includes(page.value),
);
const unread = computed(() => notes.value.filter((item) => !item.read_at).length);

const nav = computed(() =>
  [
    { group: "食堂运营", key: "overview" as Page, label: "人数总览", icon: "User", show: can("menu.read") || can("optimizer.read") || can("meal.admin_override") || can("system.super") },
    { group: "食堂运营", key: "menu" as Page, label: "菜谱", icon: "Food", show: can("menu.read") },
    { group: "食堂运营", key: "dishEdit" as Page, label: "编辑菜", icon: "EditPen", show: can("menu.write") },
    { group: "食堂运营", key: "optimize" as Page, label: "选菜优化", icon: "MagicStick", show: can("optimizer.read") },
    { group: "食堂运营", key: "alert" as Page, label: "强提醒", icon: "Bell", show: can("menu.write") || can("optimizer.read") || can("system.super") },
    { group: "食堂运营", key: "rating" as Page, label: "评分", icon: "Histogram", show: can("rating.admin_read") },
    { group: "食堂运营", key: "comment" as Page, label: "评语", icon: "ChatDotRound", show: can("comment.admin_read") },
    { group: "食堂运营", key: "change" as Page, label: "改单", icon: "Edit", show: can("meal.admin_override") },
    { group: "食堂运营", key: "device" as Page, label: "设备", icon: "Monitor", show: hasLandscape.value && can("device.approve") },
    { group: "食堂运营", key: "invite" as Page, label: "邀请码", icon: "Ticket", show: can("user.invite") },
    { group: "总管 / 超管", key: "export" as Page, label: "导出", icon: "Download", show: can("export.run") },
    { group: "总管 / 超管", key: "audit" as Page, label: "审计", icon: "Document", show: can("audit.read") },
    { group: "总管 / 超管", key: "email" as Page, label: "邮箱", icon: "Message", show: can("system.super") },
    { group: "总管 / 超管", key: "threshold" as Page, label: "阈值", icon: "Operation", show: can("rating.strong_alert_config") },
    { group: "总管 / 超管", key: "recover" as Page, label: "找回", icon: "Key", show: can("user.recover") },
    { group: "总管 / 超管", key: "llm" as Page, label: "LLM", icon: "Cpu", show: can("system.super") },
    { group: "总管 / 超管", key: "appSigning" as Page, label: "应用签名", icon: "Key", show: can("system.super") },
    { group: "其它", key: "account" as Page, label: "账户", icon: "Setting", show: true },
    { group: "其它", key: "employee" as Page, label: "员工竖屏", icon: "Iphone", show: hasLandscape.value && (can("meal.self_read") || can("shift.self_write")) },
  ].filter((item) => item.show),
);

const groups = computed(() => {
  const order = ["食堂运营", "总管 / 超管", "其它"];
  return order
    .map((name) => ({ name, items: nav.value.filter((item) => item.group === name) }))
    .filter((group) => group.items.length > 0);
});

function firstPage(): Page {
  return nav.value.find((item) => item.key !== "employee" && item.key !== "account")?.key ?? "employee";
}

// 窄屏手机壳 BACK 逐页回退：页面切换时写历史栈，popstate 按栈恢复。
// 只有窄屏（手机浏览器 / Android WebView 壳）维护历史；桌面端不写历史、
// 不新增历史条目，浏览器前进/后退仍按整页导航处理，行为与改动前一致。
type HistoryState = { page: Page; dishId: string | null; emp: boolean };

function currentState(): HistoryState {
  return { page: page.value, dishId: dishId.value, emp: portraitEmployee.value };
}

function writeHistory(replace: boolean) {
  if (typeof window === "undefined" || !isNarrow.value) return;
  const next = currentState();
  const cur = window.history.state as Partial<HistoryState> | null;
  if (
    !replace && cur && cur.page === next.page &&
    (cur.dishId ?? null) === (next.dishId ?? null) && (cur.emp ?? false) === next.emp
  ) {
    return;
  }
  try {
    if (replace) window.history.replaceState(next, "");
    else window.history.pushState(next, "");
  } catch {
    // 忽略（如受限上下文 pushState 抛错，不挡页面切换）
  }
}

function parseHistoryState(state: unknown): HistoryState | null {
  if (!state || typeof state !== "object") return null;
  const prev = state as Partial<HistoryState>;
  if (typeof prev.page !== "string" || !PAGE_SET.has(prev.page)) return null;
  return {
    page: prev.page as Page,
    dishId: typeof prev.dishId === "string" ? prev.dishId : null,
    emp: prev.emp === true,
  };
}

// 默认工作台（与 onMounted / enter 的初始页同一来源），用于历史条目不可用时的兜底。
function resetToWorkbench() {
  portraitEmployee.value = false;
  dishId.value = null;
  page.value = firstPage();
}

function applyHistory(state: HistoryState) {
  // 回退中：直接恢复，不再写历史
  portraitEmployee.value = state.emp;
  dishId.value = state.dishId;
  page.value = state.page;
}

function onPopState(event: PopStateEvent) {
  // 桌面端不维护页面历史，popstate 不参与页面恢复。
  if (!isNarrow.value) return;
  const next = parseHistoryState(event.state);
  if (next) {
    applyHistory(next);
    return;
  }
  // state 缺失/非法（历史条目损坏，或在壳外留下的条目）：不能静默 return，
  // 否则 BACK 被吃掉、页面停在原处。恢复默认工作台，并把当前条目规范化为合法 state。
  resetToWorkbench();
  writeHistory(true);
}

function toEmployee() {
  portraitEmployee.value = true;
  page.value = "employee";
  writeHistory(false);
}

function backToPortrait() {
  // 员工竖屏是 toEmployee() 推入的历史条目（emp=true）。直接 history.back() 由
  // popstate 恢复食堂工作台；不能再 push 一条，否则 BACK 会先回到员工端。
  const cur = window.history.state as Partial<HistoryState> | null;
  if (isNarrow.value && cur?.emp === true) {
    window.history.back();
    return;
  }
  // 降级：state 丢失等情况下没有可回退的员工条目，按原路由直接切回工作台。
  resetToWorkbench();
  writeHistory(false);
}

function open(key: Page) {
  if (key === "dishEdit") dishId.value = null;
  page.value = key;
  writeHistory(false);
}

function editDish(id: string | null) {
  dishId.value = id;
  page.value = "dishEdit";
  writeHistory(false);
}

async function enter() {
  portraitEmployee.value = false;
  page.value = firstPage();
  writeHistory(true);
  await loadNotes();
}

async function loadNotes() {
  try {
    const body = await api<{ items: Note[] }>("/api/notifications");
    notes.value = body.items;
  } catch {
    notes.value = [];
  }
}

async function readNote(item: Note) {
  if (item.read_at) return;
  await api(`/api/notifications/${item.id}/read`, { method: "POST" });
  item.read_at = new Date().toISOString();
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

onMounted(async () => {
  await loadVersion();
  await refreshSession();
  if (auth.me && hasWorkbench.value) {
    page.value = firstPage();
    writeHistory(true);
    await loadNotes();
  }
  window.addEventListener("popstate", onPopState);
});

onBeforeUnmount(() => {
  window.removeEventListener("popstate", onPopState);
});
</script>

<template>
  <div v-if="!auth.ready" class="boot">正在连接…</div>
  <LoginPage v-else-if="!auth.me || needsFactor" :enrolling="needsFactor" @enter="enter" />
  <div v-else-if="!hasWorkbench" class="solo">
    <EmployeePage />
  </div>
  <!-- 窄屏食堂竖屏壳：宽屏 .shell 原样不动 -->
  <div v-else-if="isNarrow && portraitEmployee" class="solo portrait-emp">
    <button type="button" class="back-bar" @click="backToPortrait">‹ 返回食堂工作台</button>
    <div class="emp-fill"><EmployeePage /></div>
  </div>
  <CanteenPortraitShell v-else-if="isNarrow" @to-employee="toEmployee" />
  <div v-else class="shell">
    <aside class="side">
      <div class="brand">
        <svg viewBox="0 0 24 24" width="28" height="28" aria-hidden="true">
          <path d="M12 3c-1.8 0-3.3 1-4.1 2.4A3.5 3.5 0 0 0 4 8.8C4 10.8 5.6 12.4 7.6 12.4h8.8c2 0 3.6-1.6 3.6-3.6a3.5 3.5 0 0 0-3.9-3.4A4.6 4.6 0 0 0 12 3Z" fill="#2d8a45"/>
          <rect x="7" y="12.2" width="10" height="7.3" rx="1.2" fill="#2d8a45"/>
          <rect x="6.2" y="18.8" width="11.6" height="1.8" rx="0.6" fill="#246e37"/>
        </svg>
        <div>
          <strong>多邻灶</strong>
          <div class="sub">食堂运营</div>
        </div>
      </div>
      <nav v-for="group in groups" :key="group.name">
        <div class="group">{{ group.name }}</div>
        <button
          v-for="item in group.items"
          :key="item.key"
          type="button"
          class="nav-btn"
          :class="{ active: page === item.key }"
          @click="open(item.key)"
        >
          <el-icon><component :is="item.icon" /></el-icon>
          {{ item.label }}
        </button>
      </nav>
      <div class="side-foot">{{ versionFooter() }}</div>
    </aside>
    <main class="main">
      <div class="toolbar">
        <div v-if="needsCanteen" class="canteen-row">
          <span>食堂</span>
          <el-select v-model="auth.canteenId" placeholder="选择食堂" style="width: 220px">
            <el-option v-for="item in canteens" :key="item.id" :label="item.name" :value="item.id" />
          </el-select>
          <template v-if="can('system.super')">
            <el-input v-model="newCanteen" placeholder="新食堂名称" style="width: 160px" @keyup.enter="createCanteen" />
            <el-button @click="createCanteen">新建食堂</el-button>
          </template>
        </div>
        <div class="spacer" />
        <el-badge :value="unread" :hidden="unread === 0">
          <el-button circle @click="notesOpen = true"><el-icon><Bell /></el-icon></el-button>
        </el-badge>
        <button type="button" class="who" :title="auth.me?.username" @click="open('account')">
          {{ personName(auth.me) }}
        </button>
      </div>
      <p v-if="needsCanteen && canteens.length === 0" class="muted">当前没有可管理的食堂。</p>
      <OverviewPage v-if="page === 'overview'" />
      <MenuPage v-else-if="page === 'menu'" @edit="editDish" />
      <DishEditPage v-else-if="page === 'dishEdit'" :dish-id="dishId" @back="open('menu')" />
      <OptimizePage v-else-if="page === 'optimize'" />
      <AlertPage v-else-if="page === 'alert'" />
      <RatingPage v-else-if="page === 'rating'" />
      <CommentsPage v-else-if="page === 'comment'" />
      <OverridePage v-else-if="page === 'change'" />
      <DevicePage v-else-if="page === 'device'" />
      <InvitePage v-else-if="page === 'invite'" />
      <ExportPage v-else-if="page === 'export'" />
      <AuditPage v-else-if="page === 'audit'" />
      <EmailPage v-else-if="page === 'email'" />
      <ThresholdPage v-else-if="page === 'threshold'" />
      <RecoverPage v-else-if="page === 'recover'" />
      <LlmPage v-else-if="page === 'llm'" />
      <AppSigningPage v-else-if="page === 'appSigning'" />
      <AccountPage v-else-if="page === 'account'" @open-devices="open('device')" />
      <div v-else class="phone-wrap">
        <div class="phone"><EmployeePage /></div>
      </div>
    </main>
    <el-drawer v-model="notesOpen" title="站内提示" size="360px">
      <el-empty v-if="notes.length === 0" description="没有站内提示" />
      <button v-for="item in notes" :key="item.id" type="button" class="note" @click="readNote(item)">
        <strong>{{ item.title }}</strong>
        <span>{{ item.body }}</span>
        <em>{{ item.read_at ? "已读" : "未读" }}</em>
      </button>
    </el-drawer>
  </div>
</template>

<style scoped>
.boot, .solo { min-height: 100vh; }
.boot { display: grid; place-items: center; color: #6b7280; }
.solo { height: 100vh; max-width: 480px; margin: 0 auto; background: #fff; }
.portrait-emp { display: flex; flex-direction: column; background: #f7f8fa; }
.back-bar {
  border: none; background: #fff; color: #2d8a45; font: inherit; font-size: 14px; font-weight: 600;
  text-align: left; padding: 12px 16px; cursor: pointer; border-bottom: 1px solid #e8eaed; flex-shrink: 0;
}
.emp-fill { flex: 1; min-height: 0; display: flex; flex-direction: column; }
.emp-fill .emp { flex: 1; min-height: 0; }
.shell { display: flex; min-height: 100vh; }
.side {
  width: var(--dlz-sidebar-w); background: #fff; border-right: 1px solid #e8eaed;
  position: sticky; top: 0; height: 100vh; overflow: auto; flex-shrink: 0;
  display: flex; flex-direction: column;
}
.brand { display: flex; gap: 10px; align-items: center; padding: 18px 16px 8px; }
.brand strong { font-size: 18px; }
.sub, .group, .side-foot { color: #9ca3af; font-size: 12px; }
.group { padding: 14px 16px 6px; }
.nav-btn {
  width: calc(100% - 12px); margin: 2px 0 2px 6px; text-align: left; border: none; background: transparent;
  padding: 10px 12px; border-radius: 10px; display: flex; gap: 8px; align-items: center;
  cursor: pointer; color: #374151; font: inherit;
}
.nav-btn.active { background: #e8f5ec; color: #2d8a45; font-weight: 650; }
.side-foot { margin-top: auto; padding: 16px; }
.main { flex: 1; min-width: 0; padding: 18px 22px 40px; }
.toolbar, .canteen-row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.toolbar { margin-bottom: 14px; }
.spacer { flex: 1; }
.who {
  border: none; background: transparent; cursor: pointer; font: inherit;
  color: #6b7280; font-size: 13px; padding: 4px 2px;
}
.phone-wrap { display: flex; justify-content: center; }
.phone {
  width: 390px; height: min(844px, calc(100vh - 120px)); background: #fff; border-radius: 28px;
  border: 10px solid #1f2937; overflow: hidden; box-shadow: 0 16px 40px rgba(15, 23, 42, 0.16);
}
.note {
  width: 100%; text-align: left; border: none; background: #f8faf9; border-radius: 10px;
  padding: 10px 12px; margin-bottom: 8px; display: flex; flex-direction: column; gap: 4px; cursor: pointer; font: inherit;
}
.note em { font-style: normal; color: #2d8a45; font-size: 12px; }
</style>

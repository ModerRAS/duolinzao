<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from "vue";
import {
  NavBar as VanNavBar,
  Button as VanButton,
  Cell as VanCell,
  CellGroup as VanCellGroup,
  Checkbox as VanCheckbox,
  CheckboxGroup as VanCheckboxGroup,
  Empty as VanEmpty,
  Field as VanField,
  NoticeBar as VanNoticeBar,
  Tab as VanTab,
  Tabbar as VanTabbar,
  TabbarItem as VanTabbarItem,
  Tabs as VanTabs,
  Tag as VanTag,
  showConfirmDialog,
  showFailToast,
  showSuccessToast,
} from "vant";
import {
  addDays,
  api,
  explain,
  KINDS,
  shanghaiToday,
  SLOT_NAME,
  SLOT_SHORT,
  SLOTS,
  personName,
  weekdayLabel,
  weekStart,
  type Canteen,
  type Groups,
  type Kind,
  type Slot,
} from "../api";
import Thumb from "../components/Thumb.vue";
import AccountPage from "./AccountPage.vue";
import { auth } from "../session";
import { appVersion, loadVersion } from "../version";

type Tab = "meal" | "score" | "mine";
type Screen = "home" | "shift" | "week" | "generate" | "preview" | "mine" | "bind" | "devices" | "notices" | "account";

interface Shift {
  kind: string | null;
  anchor_date: string | null;
}
interface MealItem {
  date: string;
  slot: Slot;
  canteen_id: string | null;
  ordering: boolean;
  locked: boolean;
  color: string;
}
interface PreviewSlot {
  ordering: boolean;
  color: string;
  canteen_id: string | null;
  locked?: boolean;
}
interface HomeMenu {
  菜: string[];
  饭: string[];
  汤: string[];
}
interface HomeNext {
  date: string;
  slot: Slot;
  canteen_id: string | null;
  canteen_name: string | null;
  menu: HomeMenu;
}
interface HomeDay {
  date: string;
  locked: boolean;
  slots: Record<Slot, { state: "locked" | "ordering" | "skip" | "unset" }>;
}
interface PreviewDay {
  date: string;
  slots: Record<Slot, PreviewSlot>;
}
interface Target {
  dish_id: string;
  name: string;
  kind: Kind;
  taste_tags: string[];
  image_url: string | null;
  scored: boolean;
}
interface Device {
  id: string;
  friendly_name: string;
  user_agent: string;
  ip: string;
  session_active: boolean;
}
interface Passkey {
  id: string;
  friendly_name: string;
}
interface Approval {
  id: string;
  friendly_name: string;
  user_agent: string;
  ip: string;
}
interface Note {
  id: string;
  title: string;
  body: string;
  read_at: string | null;
  created_at: string;
}

const tab = ref<Tab>("meal");
const screen = ref<Screen>("home");
const shift = ref<Shift | null>(null);
const bound = ref<Canteen[]>([]);
const allCanteens = ref<Canteen[]>([]);
const picked = ref<string[]>([]);
const mealCanteen = ref("");
const anchor = ref(shanghaiToday());
const weekAnchor = ref(weekStart(shanghaiToday()));
const meals = ref<MealItem[]>([]);
const lockedDates = ref<string[]>([]);
const homeNext = ref<HomeNext | null>(null);
const homeDays = ref<HomeDay[]>([]);
const description = ref("");
const templateId = ref<string | null>(null);
const preview = ref<PreviewDay[]>([]);
const scoreDate = ref(shanghaiToday());
const scoreSlot = ref<Slot>("lunch");
const scoreKind = ref<Kind>("菜");
const windowOpen = ref(false);
const targets = ref<Groups<Target>>({ 菜: [], 饭: [], 汤: [] });
const drafts = reactive<Record<string, { value: number | null; comment: string; anonymous: boolean }>>({});
const devices = ref<Device[]>([]);
const passkeys = ref<Passkey[]>([]);
const pending = ref<Approval[]>([]);
const notes = ref<Note[]>([]);

const weekDays = computed(() => Array.from({ length: 7 }, (_, index) => addDays(weekAnchor.value, index)));
const shiftLabel = computed(() => {
  if (shift.value?.kind === "work2_rest4") return "上二休四";
  if (shift.value?.kind === "custom") return "自定义排班";
  return "";
});
const title = computed(() => {
  if (tab.value === "score") return "评分";
  if (screen.value === "shift") return "选择班制";
  if (screen.value === "week") return weekTitle(weekAnchor.value);
  if (screen.value === "generate") return "用描述生成模板";
  if (screen.value === "preview") return "模板预览";
  if (screen.value === "bind") return "绑定食堂";
  if (screen.value === "devices") return "设备与 Passkey";
  if (screen.value === "notices") return "站内提示";
  if (screen.value === "account") return "账户设置";
  if (tab.value === "mine") return "我的";
  if (screen.value === "home") return "首页";
  return "报餐";
});
const canBack = computed(() => screen.value !== "home" && screen.value !== "mine");
const unread = computed(() => notes.value.filter((item) => !item.read_at).length);

function toastError(error: unknown) {
  showFailToast(explain(error));
}

function weekTitle(anchor: string) {
  const current = weekStart(shanghaiToday());
  if (anchor === current) return "本周报餐";
  return `${anchor.slice(5)}–${addDays(anchor, 6).slice(5)} 报餐`;
}

function greeting() {
  const hour = Number(
    new Intl.DateTimeFormat("en-GB", {
      timeZone: "Asia/Shanghai",
      hour: "numeric",
      hourCycle: "h23",
    }).format(new Date()),
  );
  if (hour < 11) return "早上好";
  if (hour < 14) return "中午好";
  if (hour < 18) return "下午好";
  return "晚上好";
}

function relativeDay(iso: string) {
  const today = shanghaiToday();
  if (iso === today) return "今天";
  if (iso === addDays(today, 1)) return "明天";
  return weekdayLabel(iso);
}

function monthDay(iso: string) {
  const [, month, day] = iso.split("-");
  return `${Number(month)}月${Number(day)}日`;
}

function shortMonthDay(iso: string) {
  const [, month, day] = iso.split("-");
  return `${month}/${day}`;
}

function menuNames(names: string[] | undefined) {
  return names && names.length ? names.join("、") : "未排菜";
}

function cell(date: string, slot: Slot) {
  return meals.value.find((item) => item.date === date && item.slot === slot);
}

function canteenName(id: string | null) {
  return bound.value.find((item) => item.id === id)?.name ?? "";
}

function draftOf(id: string) {
  if (!drafts[id]) drafts[id] = { value: null, comment: "", anonymous: false };
  return drafts[id];
}

function back() {
  if (screen.value === "generate" || screen.value === "preview") screen.value = "shift";
  else if (tab.value === "mine") screen.value = "mine";
  else screen.value = "home";
}

function openTab(next: Tab) {
  tab.value = next;
  if (next === "meal") screen.value = "home";
  if (next === "mine") screen.value = "mine";
}

async function loadBound() {
  const body = await api<{ items: Canteen[] }>("/api/me/canteens");
  bound.value = body.items;
  if (!bound.value.some((item) => item.id === mealCanteen.value)) {
    mealCanteen.value = bound.value[0]?.id ?? "";
  }
}

async function loadShift() {
  shift.value = await api<Shift>("/api/me/shift");
}

async function loadHome() {
  const [board] = await Promise.all([
    api<{ next: HomeNext | null; days: HomeDay[] }>("/api/me/home"),
    loadShift(),
    loadNotes(),
  ]);
  homeNext.value = board.next;
  homeDays.value = board.days;
}

async function loadWeek() {
  await loadBound();
  const start = weekAnchor.value;
  const end = addDays(start, 6);
  const body = await api<{ items: MealItem[]; locked_dates?: string[] }>(
    `/api/me/meals?start=${start}&end=${end}`,
  );
  meals.value = body.items;
  lockedDates.value = body.locked_dates ?? [];
}

async function loadPreview() {
  await loadBound();
  const body = await api<{ id: string | null; preview: PreviewDay[] }>("/api/me/meal-template");
  templateId.value = body.id;
  preview.value = body.preview ?? [];
}

async function loadBind() {
  const [all, mine] = await Promise.all([
    api<{ items: Canteen[] }>("/api/canteens"),
    api<{ items: Canteen[] }>("/api/me/canteens"),
  ]);
  allCanteens.value = all.items;
  picked.value = mine.items.map((item) => item.id);
  bound.value = mine.items;
}

async function loadDevices() {
  const body = await api<{ devices: Device[]; passkeys: Passkey[] }>("/api/devices");
  devices.value = body.devices;
  passkeys.value = body.passkeys;
  const approvals = await api<{ items: Approval[] }>("/api/login-approvals/pending");
  pending.value = approvals.items;
}

async function loadNotes() {
  const body = await api<{ items: Note[] }>("/api/notifications");
  notes.value = body.items;
}

async function loadScores() {
  const body = await api<{ window_open: boolean; groups: Groups<Target> }>(
    `/api/me/score-targets?date=${scoreDate.value}&slot=${scoreSlot.value}`,
  );
  windowOpen.value = body.window_open;
  targets.value = body.groups;
}

async function refresh() {
  try {
    if (tab.value === "score") {
      await loadScores();
      return;
    }
    if (screen.value === "home") await loadHome();
    else if (screen.value === "shift") {
      await Promise.all([loadShift(), loadBound()]);
    } else if (screen.value === "week") await loadWeek();
    else if (screen.value === "generate") await loadBound();
    else if (screen.value === "preview") await loadPreview();
    else if (screen.value === "bind") await loadBind();
    else if (screen.value === "devices") await loadDevices();
    else if (screen.value === "notices" || screen.value === "mine") await loadNotes();
  } catch (error) {
    toastError(error);
  }
}

function boundCanteen() {
  if (!bound.value.length || !mealCanteen.value) {
    showFailToast("请先绑定食堂");
    return "";
  }
  return mealCanteen.value;
}

async function saveWork2() {
  const canteenId = boundCanteen();
  if (!canteenId) return;
  if (!anchor.value) {
    showFailToast("上二休四需要锚点日");
    return;
  }
  try {
    await api("/api/me/shift", {
      method: "PUT",
      body: JSON.stringify({
        kind: "work2_rest4",
        anchor_date: anchor.value,
        canteen_id: canteenId,
      }),
    });
    showSuccessToast("已设为上二休四");
    screen.value = "home";
  } catch (error) {
    toastError(error);
  }
}

async function saveCustom() {
  const canteenId = boundCanteen();
  if (!canteenId) return;
  try {
    await api("/api/me/shift", {
      method: "PUT",
      body: JSON.stringify({ kind: "custom", canteen_id: canteenId }),
    });
    showSuccessToast("已设为自定义排班");
    screen.value = "home";
  } catch (error) {
    toastError(error);
  }
}

function openCurrentWeek() {
  weekAnchor.value = weekStart(shanghaiToday());
  screen.value = "week";
}

async function generate() {
  try {
    await api("/api/meal-templates/generate", {
      method: "POST",
      body: JSON.stringify({
        description: description.value,
        anchor_date: anchor.value || null,
        canteen_id: mealCanteen.value || null,
      }),
    });
    showSuccessToast("模板已写入");
    screen.value = "preview";
  } catch (error) {
    toastError(error);
  }
}

async function toggleMeal(date: string, slot: Slot) {
  const current = cell(date, slot);
  if (dayLocked(date) || current?.locked) return;
  const ordering = !(current?.ordering ?? false);
  if (ordering && !mealCanteen.value) {
    showFailToast("请先绑定并选择食堂");
    return;
  }
  try {
    await api(`/api/meals/${date}/${slot}`, {
      method: "PUT",
      body: JSON.stringify({ ordering, canteen_id: ordering ? mealCanteen.value : current?.canteen_id ?? null }),
    });
    await loadWeek();
  } catch (error) {
    toastError(error);
  }
}

async function togglePreview(day: PreviewDay, slot: Slot) {
  if (!templateId.value) return;
  const current = day.slots[slot];
  if (current.locked) return;
  const ordering = !current.ordering;
  if (ordering && !mealCanteen.value) {
    showFailToast("报餐需要选择食堂");
    return;
  }
  try {
    const body = await api<{ ordering?: boolean; color: string; locked: boolean; order_updated: boolean }>(
      `/api/meal-templates/${templateId.value}/slots`,
      {
        method: "PUT",
        body: JSON.stringify({
          date: day.date,
          slot,
          ordering,
          canteen_id: ordering ? mealCanteen.value : current.canteen_id,
        }),
      },
    );
    if (body.locked) {
      current.locked = true;
      showFailToast("该餐已锁定");
      return;
    }
    current.ordering = ordering;
    current.color = body.color;
  } catch (error) {
    toastError(error);
  }
}

async function submitScore(dish: Target) {
  const draft = draftOf(dish.dish_id);
  if (draft.value === null) {
    showFailToast("请选择 −2 到 2");
    return;
  }
  try {
    await api("/api/scores", {
      method: "POST",
      body: JSON.stringify({
        meal_date: scoreDate.value,
        slot: scoreSlot.value,
        dish_id: dish.dish_id,
        value: draft.value,
        comment: draft.comment.trim() || null,
        anonymous: draft.anonymous,
      }),
    });
    showSuccessToast("已提交");
    await loadScores();
  } catch (error) {
    toastError(error);
  }
}

async function saveBind() {
  try {
    await api("/api/me/canteens", {
      method: "PUT",
      body: JSON.stringify({ canteen_ids: picked.value }),
    });
    showSuccessToast("已绑定");
    screen.value = "mine";
  } catch (error) {
    toastError(error);
  }
}

async function kick(id: string) {
  try {
    await showConfirmDialog({ title: "踢下线", message: "这台设备上的会话会结束。" });
  } catch {
    return;
  }
  try {
    await api(`/api/devices/${id}`, { method: "DELETE" });
    await loadDevices();
  } catch (error) {
    toastError(error);
  }
}

async function unbind(id: string) {
  try {
    await api(`/api/passkeys/${id}`, { method: "DELETE" });
    await loadDevices();
  } catch (error) {
    toastError(error);
  }
}

async function decide(id: string, allow: boolean) {
  try {
    await api(`/api/login-approvals/${id}/${allow ? "allow" : "deny"}`, { method: "POST" });
    showSuccessToast(allow ? "已允许" : "已拒绝");
    await loadDevices();
  } catch (error) {
    toastError(error);
  }
}

async function readNote(item: Note) {
  if (item.read_at) return;
  try {
    await api(`/api/notifications/${item.id}/read`, { method: "POST" });
    item.read_at = new Date().toISOString();
  } catch (error) {
    toastError(error);
  }
}

function dayLocked(date: string) {
  return lockedDates.value.includes(date) || SLOTS.some((slot) => cell(date, slot)?.locked);
}

function mealClass(date: string, slot: Slot) {
  if (dayLocked(date)) return "locked";
  const item = cell(date, slot);
  if (item?.ordering) return "yes";
  if (item) return "no";
  return "unset";
}

function previewClass(slot: PreviewSlot) {
  if (slot.locked) return "locked";
  if (slot.color === "green" || slot.ordering) return "yes";
  return "no";
}

function homeClass(state: string) {
  if (state === "ordering") return "yes";
  if (state === "skip") return "no";
  if (state === "locked") return "locked";
  return "unset";
}

function homeLabel(state: string) {
  if (state === "ordering") return "报";
  if (state === "skip") return "不报";
  if (state === "locked") return "已锁";
  return "未定";
}

watch([screen, tab, weekAnchor, scoreDate, scoreSlot], refresh);
onMounted(() => {
  loadVersion();
  refresh();
});
</script>

<template>
  <div class="emp">
    <van-nav-bar :title="title" :left-arrow="canBack" @click-left="back" />
    <div class="body">
      <template v-if="tab === 'meal' && screen === 'home'">
        <div class="hello">
          <strong>多邻灶</strong>
          <p>{{ greeting() }}，{{ personName(auth.me) }}</p>
        </div>
        <p v-if="!shiftLabel" class="hint">尚未选择班制，默认不报任何一餐。</p>
        <section v-if="homeNext" class="next-card">
          <span class="eyebrow">下一顿已报餐</span>
          <h2>{{ relativeDay(homeNext.date) }} · {{ SLOT_NAME[homeNext.slot] }}</h2>
          <p>{{ monthDay(homeNext.date) }} {{ weekdayLabel(homeNext.date) }} · {{ homeNext.canteen_name || "未选食堂" }}</p>
          <div class="menu-row">
            <div v-for="kind in KINDS" :key="kind">
              <span>{{ kind }}</span>
              <strong :class="{ missing: !homeNext.menu[kind]?.length }">{{ menuNames(homeNext.menu[kind]) }}</strong>
            </div>
          </div>
        </section>
        <section v-else class="next-card quiet">
          <span class="eyebrow">下一顿已报餐</span>
          <h2>还没有即将要吃的报餐</h2>
          <p>选好班制，或在报餐里打开某一餐后，这里会显示菜单。</p>
        </section>
        <section class="panel week-board">
          <div class="board-head">
            <h3>近一周报餐</h3>
            <span>今天起 7 天</span>
          </div>
          <div class="legend">
            <span><i class="dot yes" />报</span>
            <span><i class="dot no" />不报</span>
            <span><i class="dot unset" />未定</span>
            <span><i class="dot locked" />已锁</span>
          </div>
          <div class="board-cols"><span>日期</span><span>早</span><span>中</span><span>晚</span></div>
          <div v-for="day in homeDays" :key="day.date" class="board-row">
            <div class="when">
              <strong>{{ relativeDay(day.date) }}</strong>
              <small>{{ shortMonthDay(day.date) }} {{ weekdayLabel(day.date).slice(1) }}</small>
            </div>
            <span
              v-for="slot in SLOTS"
              :key="slot"
              class="status"
              :class="homeClass(day.slots[slot].state)"
            >{{ homeLabel(day.slots[slot].state) }}</span>
          </div>
        </section>
        <div class="pills">
          <button type="button" class="pill" @click="screen = 'shift'">选择班制</button>
          <button type="button" class="pill" @click="openCurrentWeek">手动改报餐</button>
        </div>
      </template>

      <template v-else-if="tab === 'meal' && screen === 'shift'">
        <p class="hint">只有上二休四和自定义排班。用描述生成模板是子流程，不是第三种班制。</p>
        <div v-if="bound.length" class="pills">
          <button
            v-for="item in bound"
            :key="item.id"
            type="button"
            class="pill"
            :class="{ on: item.id === mealCanteen }"
            @click="mealCanteen = item.id"
          >{{ item.name }}</button>
        </div>
        <p v-else class="hint">还没绑定食堂。请先到「我的」绑定，再保存班制。</p>
        <section class="panel">
          <h3>上二休四</h3>
          <label class="field-label">锚点日</label>
          <input v-model="anchor" type="date" class="date" />
          <van-button type="primary" block round @click="saveWork2">保存</van-button>
        </section>
        <section class="panel">
          <h3>自定义排班</h3>
          <p class="hint">不自动生成休息日以外的报餐，之后在首页或本周报餐里逐餐填写。</p>
          <van-button type="primary" block round @click="saveCustom">保存</van-button>
        </section>
        <van-button block plain type="primary" @click="screen = 'generate'">用描述生成模板</van-button>
      </template>

      <template v-else-if="tab === 'meal' && screen === 'generate'">
        <van-notice-bar wrapable text="按总管理已保存的全站渠道顺序生成，并写入个人模板。员工不填写 key。" />
        <van-field v-model="description" type="textarea" rows="5" maxlength="2000" show-word-limit placeholder="描述你的班制" />
        <label class="field-label">锚点日（可选）</label>
        <input v-model="anchor" type="date" class="date" />
        <van-button type="primary" block round class="gap" @click="generate">生成并预览</van-button>
      </template>

      <template v-else-if="tab === 'meal' && screen === 'preview'">
        <p class="hint">绿色是报餐，橙色是不报。已锁定是灰色，不能改。点未锁定的餐次会写回个人模板。</p>
        <div v-if="bound.length" class="pills">
          <button
            v-for="item in bound"
            :key="item.id"
            type="button"
            class="pill"
            :class="{ on: item.id === mealCanteen }"
            @click="mealCanteen = item.id"
          >{{ item.name }}</button>
        </div>
        <van-empty v-if="!templateId" description="还没有个人模板" />
        <article v-for="day in preview" :key="day.date" class="panel">
          <h3>{{ weekdayLabel(day.date) }} {{ day.date.slice(5) }}</h3>
          <div class="pills">
            <button
              v-for="slot in SLOTS"
              :key="slot"
              type="button"
              class="pill"
              :class="previewClass(day.slots[slot])"
              :disabled="!!day.slots[slot].locked"
              @click="togglePreview(day, slot)"
            >{{ day.slots[slot].locked ? `${SLOT_SHORT[slot]} 已锁定` : SLOT_SHORT[slot] }}</button>
          </div>
        </article>
      </template>

      <template v-else-if="tab === 'meal' && screen === 'week'">
        <van-notice-bar wrapable text="跨食堂按当前选中的食堂生效。锁定时间一到，次日三餐一起锁定。" />
        <div class="pills">
          <button type="button" class="pill" @click="weekAnchor = addDays(weekAnchor, -7)">上一周</button>
          <button type="button" class="pill" @click="weekAnchor = addDays(weekAnchor, 7)">下一周</button>
        </div>
        <div v-if="bound.length" class="pills">
          <button
            v-for="item in bound"
            :key="item.id"
            type="button"
            class="pill"
            :class="{ on: item.id === mealCanteen }"
            @click="mealCanteen = item.id"
          >{{ item.name }}</button>
        </div>
        <p v-else class="hint">还没有绑定食堂，不能报餐。</p>
        <article v-for="day in weekDays" :key="day" class="panel">
          <h3>{{ weekdayLabel(day) }} {{ day.slice(5) }}</h3>
          <div class="pills">
            <button
              v-for="slot in SLOTS"
              :key="slot"
              type="button"
              class="pill"
              :class="mealClass(day, slot)"
              :disabled="dayLocked(day)"
              @click="toggleMeal(day, slot)"
            >
              {{ dayLocked(day) ? `${SLOT_SHORT[slot]} 已锁定` : SLOT_SHORT[slot] }}
            </button>
          </div>
          <p v-if="cell(day, 'breakfast')?.canteen_id || cell(day, 'lunch')?.canteen_id || cell(day, 'dinner')?.canteen_id" class="hint">
            {{ SLOTS.filter((slot) => cell(day, slot)?.ordering).map((slot) => `${SLOT_SHORT[slot]} ${canteenName(cell(day, slot)?.canteen_id ?? null)}`).join(" · ") }}
          </p>
        </article>
      </template>

      <template v-else-if="tab === 'score'">
        <div class="filters">
          <input v-model="scoreDate" type="date" class="date" />
          <div class="pills">
            <button
              v-for="slot in SLOTS"
              :key="slot"
              type="button"
              class="pill"
              :class="{ on: scoreSlot === slot }"
              @click="scoreSlot = slot"
            >{{ SLOT_NAME[slot] }}</button>
          </div>
        </div>
        <van-notice-bar text="餐后一周内可评。每道餐品单独打分，评语挂在这一道上。" />
        <van-empty v-if="!windowOpen" description="这一餐的评分窗口还没打开" />
        <van-tabs v-else v-model:active="scoreKind">
          <van-tab v-for="kind in KINDS" :key="kind" :title="kind" :name="kind">
            <van-empty v-if="targets[kind].length === 0" description="这一类没有可评的餐品" />
            <article v-for="dish in targets[kind]" :key="dish.dish_id" class="panel dish">
              <Thumb :src="dish.image_url" />
              <div class="dish-main">
                <strong>{{ dish.name }}</strong>
                <div>
                  <van-tag v-for="tag in dish.taste_tags" :key="tag" plain type="success" class="tag">{{ tag }}</van-tag>
                </div>
                <p v-if="dish.scored" class="hint">已评分，不能再改</p>
                <template v-else>
                  <div class="pills">
                    <button
                      v-for="value in [-2, -1, 0, 1, 2]"
                      :key="value"
                      type="button"
                      class="pill"
                      :class="{ on: draftOf(dish.dish_id).value === value }"
                      @click="draftOf(dish.dish_id).value = value"
                    >{{ value }}</button>
                  </div>
                  <van-field
                    v-model="draftOf(dish.dish_id).comment"
                    type="textarea"
                    rows="2"
                    maxlength="300"
                    show-word-limit
                    placeholder="可选评语，随这条评分一起提交"
                  />
                  <van-checkbox v-model="draftOf(dish.dish_id).anonymous">匿名，管理端不展示账号</van-checkbox>
                  <van-button size="small" type="primary" round @click="submitScore(dish)">提交</van-button>
                </template>
              </div>
            </article>
          </van-tab>
        </van-tabs>
      </template>

      <template v-else-if="screen === 'mine'">
        <van-cell-group inset>
          <van-cell title="显示名" :value="personName(auth.me)" />
          <van-cell title="账号" :value="auth.me?.username" />
          <van-cell title="账户设置" is-link @click="screen = 'account'" />
          <van-cell title="绑定食堂" is-link @click="screen = 'bind'" />
          <van-cell title="设备与 Passkey" is-link @click="screen = 'devices'" />
          <van-cell title="站内提示" is-link :value="unread ? `${unread} 条未读` : ''" @click="screen = 'notices'" />
        </van-cell-group>
        <p class="hint">绑定食堂只决定报餐可用范围，不是食堂管理范围。</p>
      </template>

      <template v-else-if="screen === 'bind'">
        <p class="hint">可以绑定多个食堂。这是报餐时的使用范围。</p>
        <van-checkbox-group v-model="picked">
          <van-cell-group inset>
            <van-cell v-for="item in allCanteens" :key="item.id" :title="item.name" clickable @click="picked = picked.includes(item.id) ? picked.filter((id) => id !== item.id) : [...picked, item.id]">
              <template #right-icon>
                <van-checkbox :name="item.id" @click.stop />
              </template>
            </van-cell>
          </van-cell-group>
        </van-checkbox-group>
        <van-button type="primary" block round class="gap" @click="saveBind">保存</van-button>
      </template>

      <template v-else-if="screen === 'devices'">
        <h3 class="block-title">待确认</h3>
        <van-empty v-if="pending.length === 0" description="没有待确认的新设备" />
        <article v-for="item in pending" :key="item.id" class="panel">
          <strong>{{ item.friendly_name }}</strong>
          <p class="hint">{{ item.ip }}</p>
          <p class="hint">{{ item.user_agent }}</p>
          <div class="pills">
            <van-button size="small" type="primary" @click="decide(item.id, true)">允许</van-button>
            <van-button size="small" @click="decide(item.id, false)">拒绝</van-button>
          </div>
        </article>
        <h3 class="block-title">设备</h3>
        <article v-for="item in devices" :key="item.id" class="panel">
          <strong>{{ item.friendly_name }}</strong>
          <p class="hint">{{ item.session_active ? "在线" : "已结束" }} · {{ item.ip }}</p>
          <p class="hint">{{ item.user_agent }}</p>
          <van-button size="small" type="danger" @click="kick(item.id)">踢下线</van-button>
        </article>
        <h3 class="block-title">Passkey</h3>
        <van-empty v-if="passkeys.length === 0" description="还没有绑定 Passkey" />
        <article v-for="item in passkeys" :key="item.id" class="panel row">
          <span>{{ item.friendly_name || "Passkey" }}</span>
          <van-button size="small" type="danger" plain @click="unbind(item.id)">解绑</van-button>
        </article>
      </template>

      <template v-else-if="screen === 'account'">
        <AccountPage embedded @open-devices="screen = 'devices'" />
      </template>

      <template v-else-if="screen === 'notices'">
        <van-empty v-if="notes.length === 0" description="没有站内提示" />
        <van-cell
          v-for="item in notes"
          :key="item.id"
          :title="item.title"
          :label="item.body"
          :value="item.read_at ? '已读' : '未读'"
          @click="readNote(item)"
        />
      </template>
    </div>
    <van-tabbar :model-value="tab" :fixed="false" active-color="#2d8a45" inactive-color="#6b7280">
      <van-tabbar-item name="meal" @click="openTab('meal')">报餐</van-tabbar-item>
      <van-tabbar-item name="score" @click="openTab('score')">评分</van-tabbar-item>
      <van-tabbar-item name="mine" @click="openTab('mine')">我的</van-tabbar-item>
    </van-tabbar>
    <div v-if="appVersion" class="ver">Powered by 多邻灶 · Version: {{ appVersion }}</div>
  </div>
</template>

<style scoped>
.emp { height: 100%; min-height: 100%; display: flex; flex-direction: column; background: #f7f8fa; color: #1f2937; }
.body { flex: 1; overflow: auto; padding: 12px 12px 20px; }
.hello { padding: 4px 4px 8px; }
.hello strong { font-size: 18px; }
.hello p, .hint { color: #6b7280; font-size: 13px; margin: 4px 0; }
.next-card {
  background: #2d8a45; color: #fff; border-radius: 16px; padding: 16px; margin: 8px 0 12px;
}
.next-card.quiet { background: #e8f5ec; color: #1f2937; }
.next-card h2 { margin: 6px 0 4px; font-size: 22px; }
.next-card p { margin: 0; font-size: 13px; opacity: 0.92; }
.eyebrow { display: inline-block; font-size: 12px; padding: 2px 8px; border-radius: 999px; background: rgba(255,255,255,.18); }
.quiet .eyebrow { background: #d7efe0; color: #246e37; }
.menu-row { display: grid; grid-template-columns: repeat(3, 1fr); gap: 8px; margin-top: 14px; }
.menu-row div { background: rgba(255,255,255,.16); border-radius: 12px; padding: 8px; min-width: 0; }
.quiet .menu-row div { background: #fff; }
.menu-row span { display: block; font-size: 12px; opacity: 0.85; }
.menu-row strong { display: block; margin-top: 4px; font-size: 14px; font-weight: 650; word-break: break-all; }
.menu-row strong.missing { font-weight: 500; opacity: 0.8; }
.week-board { padding: 12px; }
.board-head, .board-cols, .board-row, .legend { display: grid; grid-template-columns: 1.3fr repeat(3, 1fr); gap: 8px; align-items: center; }
.board-head { grid-template-columns: 1fr auto; margin-bottom: 8px; }
.board-head h3 { margin: 0; }
.board-head span, .legend { color: #6b7280; font-size: 12px; }
.legend { display: flex; gap: 10px; margin-bottom: 8px; }
.dot { width: 8px; height: 8px; border-radius: 50%; display: inline-block; margin-right: 4px; }
.dot.yes, .status.yes { background: #e5f6ea; color: #1f7a3a; }
.dot.yes { background: #2d8a45; }
.dot.no { background: #e6a23c; }
.dot.unset { background: #fff; box-shadow: inset 0 0 0 1px #d1d5db; }
.dot.locked { background: #d1d5db; }
.board-cols { color: #9ca3af; font-size: 12px; text-align: center; }
.board-cols span:first-child, .when { text-align: left; }
.board-row { margin-top: 10px; }
.when strong, .when small { display: block; }
.when small { color: #9ca3af; font-size: 12px; }
.status {
  text-align: center; border-radius: 999px; padding: 6px 0; font-size: 13px; background: #fff;
  border: 1px solid #e5e7eb; color: #6b7280;
}
.status.yes { background: #e7f6ec; color: #1f7a3a; border-color: #b7e4c7; }
.status.no { background: #fff4e5; color: #b86e00; border-color: #f3d19e; }
.status.locked { background: #f3f4f6; color: #9ca3af; border-color: #e5e7eb; }
.gap { margin-top: 10px; }
.panel { background: #fff; border-radius: 12px; padding: 12px; margin: 10px 0; }
.panel h3, .block-title { margin: 8px 0; font-size: 15px; }
.field-label { display: block; margin: 8px 0 4px; font-size: 13px; color: #6b7280; }
.date { width: 100%; border: 1px solid #e5e7eb; border-radius: 8px; padding: 8px; margin-bottom: 10px; font: inherit; }
.pills { display: flex; flex-wrap: wrap; gap: 8px; margin: 8px 0; }
.pill {
  border: 1px solid #e5e7eb; background: #fff; border-radius: 999px; padding: 6px 12px; font: inherit; cursor: pointer;
}
.pill.on, .pill.yes { background: #2d8a45; color: #fff; border-color: #2d8a45; }
.pill.no { background: #e6a23c; color: #fff; border-color: #e6a23c; }
.pill.unset { background: #fff; color: #374151; }
.pill.locked, .pill:disabled { background: #e5e7eb; color: #9ca3af; border-color: #e5e7eb; cursor: not-allowed; }
.dish { display: flex; gap: 10px; align-items: flex-start; }
.dish-main { flex: 1; min-width: 0; }
.tag { margin-right: 4px; }
.row { display: flex; justify-content: space-between; align-items: center; }
.filters { margin-bottom: 8px; }
.ver { text-align: center; font-size: 11px; color: #9ca3af; padding: 6px 0 10px; }
</style>

<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { Check, QuestionFilled } from "@element-plus/icons-vue";
import { ElMessage } from "element-plus";
import {
  addDays,
  api,
  explain,
  KINDS,
  shanghaiToday,
  SLOT_NAME,
  SLOT_SHORT,
  SLOTS,
  toneColor,
  weekdayLabel,
  type Candidate,
  type Groups,
  type Kind,
  type Slot,
} from "../api";
import Thumb from "../components/Thumb.vue";
import { auth } from "../session";

interface WeekCell {
  expected_headcount: number;
  groups: Groups<Candidate>;
}
interface WeekDay {
  date: string;
  slots: Record<Slot, WeekCell>;
}
interface MealRef {
  canteenId: string;
  date: string;
  slot: Slot;
}

const mode = ref<"week" | "pick">("week");
const anchor = ref("");
const date = ref(shanghaiToday());
const slot = ref<Slot>("lunch");
const groups = ref<Groups<Candidate>>({ 菜: [], 饭: [], 汤: [] });
const headcount = ref(0);
const next = ref<{ date: string; slot: Slot } | null>(null);
const weekDays = ref<WeekDay[]>([]);
// 每餐最近已知 groups（key = canteen/date/slot）：切走后排队 save 用它做基数，保证连续点击组合正确。
const groupsByMeal = new Map<string, Groups<Candidate>>();
// groups.value 当前对应的餐次：切视图后、新数据回来前，避免把上一餐的数据当成本餐基数。
const groupsRef = ref<MealRef | null>(null);

const days = computed(() =>
  Array.from({ length: 7 }, (_, index) => {
    const day = addDays(anchor.value || date.value, index);
    return { date: day, label: weekdayLabel(day), md: day.slice(5).replace("-", "/") };
  }),
);

const selected = computed(() => KINDS.flatMap((kind) => groups.value[kind].filter((item) => item.selected)));

function emptyCell(): WeekCell {
  return { expected_headcount: 0, groups: { 菜: [], 饭: [], 汤: [] } };
}

function mealKey(ref: MealRef) {
  return `${ref.canteenId}/${ref.date}/${ref.slot}`;
}

function currentMealRef(): MealRef | null {
  if (!auth.canteenId) return null;
  return { canteenId: auth.canteenId, date: date.value, slot: slot.value };
}

function sameMeal(ref: MealRef, other: MealRef | null) {
  return !!other && ref.canteenId === other.canteenId && ref.date === other.date && ref.slot === other.slot;
}

function inPickView(ref: MealRef) {
  return mode.value === "pick" && sameMeal(ref, currentMealRef());
}

// 晚到响应/并发写序号：只在最新一次请求、且期间该餐没有新 save 落库时才回填界面。
let weekLoadSeq = 0;
let pickLoadSeq = 0;
const saveEpochs = new Map<string, number>();

function saveEpoch(key: string) {
  return saveEpochs.get(key) ?? 0;
}

function markSaved(key: string) {
  saveEpochs.set(key, saveEpoch(key) + 1);
}

async function loadWeek() {
  const canteenId = auth.canteenId;
  if (!canteenId) return;
  const requestedDate = date.value;
  const seq = ++weekLoadSeq;
  const body = await api<{ week_start: string; days: WeekDay[] }>(
    `/api/canteens/${canteenId}/selection/week?date=${requestedDate}`,
  );
  // 晚到响应丢弃：期间切了食堂/周次或已进入当餐视图，不能覆盖当前界面。
  if (
    seq !== weekLoadSeq ||
    auth.canteenId !== canteenId ||
    date.value !== requestedDate ||
    mode.value !== "week"
  ) {
    return;
  }
  anchor.value = body.week_start;
  weekDays.value = body.days.map((day) => ({
    date: day.date,
    slots: {
      breakfast: day.slots?.breakfast ?? emptyCell(),
      lunch: day.slots?.lunch ?? emptyCell(),
      dinner: day.slots?.dinner ?? emptyCell(),
    },
  }));
  // 周视图顺带刷新每餐缓存：之后从周视图点进某餐、或切走后排队 save，都有该餐基数可用。
  for (const day of weekDays.value) {
    for (const item of SLOTS) {
      groupsByMeal.set(
        mealKey({ canteenId, date: day.date, slot: item }),
        day.slots[item].groups,
      );
    }
  }
}

async function loadPick() {
  const ref = currentMealRef();
  if (!ref) return;
  const key = mealKey(ref);
  // 请求发出时的写序号：晚于它的 save 一旦落库，本次读到的就是旧快照，回填会把刚勾的选择滚掉。
  const epoch = saveEpoch(key);
  const seq = ++pickLoadSeq;
  const week = await api<{ week_start: string }>(
    `/api/canteens/${ref.canteenId}/selection/week?date=${ref.date}`,
  );
  // 晚到响应丢弃：视图已变（切餐/日期/食堂或回周视图）时不能覆盖当前界面。
  if (seq !== pickLoadSeq || !inPickView(ref)) return;
  anchor.value = week.week_start;
  const body = await api<{
    expected_headcount: number;
    groups: Groups<Candidate>;
    next: { date: string; slot: Slot };
  }>(`/api/canteens/${ref.canteenId}/selection?date=${ref.date}&slot=${ref.slot}`);
  if (seq !== pickLoadSeq || !inPickView(ref) || saveEpoch(key) !== epoch) return;
  groups.value = body.groups;
  groupsRef.value = ref;
  headcount.value = body.expected_headcount;
  next.value = body.next;
  groupsByMeal.set(key, body.groups);
}

async function load() {
  try {
    if (mode.value === "week") await loadWeek();
    else await loadPick();
  } catch (error) {
    ElMessage.error(explain(error));
  }
}

async function save(ref: MealRef, nextGroups: Groups<Candidate>) {
  const key = mealKey(ref);
  const ids = KINDS.flatMap((kind) => nextGroups[kind].filter((item) => item.selected).map((item) => item.dish_id));
  try {
    const body = await api<{
      expected_headcount: number;
      groups: Groups<Candidate>;
      next: { date: string; slot: Slot };
    }>(`/api/canteens/${ref.canteenId}/selection/${ref.date}/${ref.slot}`, {
      method: "PUT",
      body: JSON.stringify({ dish_ids: ids }),
    });
    // 响应按快照餐次归位：只有它仍是当前视图才更新界面，绝不把 A 餐的响应贴到 B 餐。
    groupsByMeal.set(key, body.groups);
    markSaved(key);
    if (inPickView(ref)) {
      groups.value = body.groups;
      groupsRef.value = ref;
      headcount.value = body.expected_headcount;
      next.value = body.next;
    }
  } catch (error) {
    if (inPickView(ref)) {
      ElMessage.error(explain(error));
      // 失败回读服务端态：写可能已生效（如响应超时），以服务端为准重建本地态。
      try {
        await loadPick();
      } catch {
        /* 已提示过，保持本地旧态 */
      }
    } else {
      // 已切走：不把该餐的提示/回读贴到别的餐上；丢弃本地缓存，后续排队 save 以服务端为准。
      groupsByMeal.delete(key);
    }
  }
}

// 同餐 save 串行化：队列键与请求 URL 都用点击时的餐次快照，避免排队期间切餐写错目标。
const saveChains = new Map<string, Promise<void>>();

function toggle(kind: Kind, dishId: string) {
  const ref = currentMealRef();
  if (!ref) return;
  const key = mealKey(ref);
  const prev = saveChains.get(key) ?? Promise.resolve();
  const cur: Promise<void> = prev.catch(() => {}).then(async () => {
    // 执行时取该餐最新已知态：当前视图且数据确属该餐 → groups.value（含前一次 save 结果）；
    // 否则 → 该餐缓存，保证连续点击组合正确；都取不到就跳过，绝不拿别的餐当基数。
    const base = inPickView(ref) && sameMeal(ref, groupsRef.value) ? groups.value : groupsByMeal.get(key);
    if (!base) return;
    const copy: Groups<Candidate> = {
      菜: base.菜.map((item) => ({ ...item })),
      饭: base.饭.map((item) => ({ ...item })),
      汤: base.汤.map((item) => ({ ...item })),
    };
    const dish = copy[kind].find((item) => item.dish_id === dishId);
    // 找不到就跳过：服务端重排/下架后不按 index 猜，避免勾错菜。
    if (!dish) return;
    dish.selected = !dish.selected;
    await save(ref, copy);
  });
  // 摘掉已完成的队尾，避免 Map 无限增长；后到的 toggle 已挂在 cur 之后，不受影响。
  void cur.then(
    () => {
      if (saveChains.get(key) === cur) saveChains.delete(key);
    },
    () => {
      if (saveChains.get(key) === cur) saveChains.delete(key);
    },
  );
  saveChains.set(key, cur);
}

function openSlot(day: string, item: Slot) {
  date.value = day;
  slot.value = item;
  mode.value = "pick";
}

function shiftWeek(delta: number) {
  date.value = addDays(anchor.value || date.value, delta);
  mode.value = "week";
}

function jump() {
  if (!next.value) return;
  date.value = next.value.date;
  slot.value = next.value.slot;
}

watch(() => [auth.canteenId, date.value, slot.value, mode.value], load, { immediate: true });
</script>

<template>
  <div class="opt">
    <template v-if="mode === 'week'">
      <div class="top-bar">
        <div>
          <span class="label">选菜优化</span>
          <span class="pills">周一到周日 · 点某一餐进入选定</span>
        </div>
        <div class="days">
          <el-button @click="shiftWeek(-7)">上一周</el-button>
          <span class="muted">{{ anchor || "本周" }} 起</span>
          <el-button @click="shiftWeek(7)">下一周</el-button>
        </div>
      </div>
      <div class="week-grid">
        <section v-for="day in weekDays" :key="day.date" class="day-col">
          <header>{{ weekdayLabel(day.date) }} {{ day.date.slice(5) }}</header>
          <button v-for="item in SLOTS" :key="item" type="button" class="slot-cell" @click="openSlot(day.date, item)">
            <div class="slot-name">{{ SLOT_NAME[item] }} · {{ day.slots[item].expected_headcount }} 人</div>
            <div v-for="kind in KINDS" :key="kind" class="kind-line">
              <span class="k">{{ kind }}</span>
              <span v-if="day.slots[item].groups[kind].length" class="names">
                <span
                  v-for="dish in day.slots[item].groups[kind]"
                  :key="dish.dish_id"
                  :style="{ color: toneColor(dish.tone) }"
                >{{ dish.name }}</span>
              </span>
              <span v-else class="muted">未选</span>
            </div>
          </button>
        </section>
      </div>
    </template>

    <template v-else>
      <div class="top-bar">
        <div>
          <el-button link type="primary" @click="mode = 'week'">返回周视图</el-button>
          <span class="label">选菜优化</span>
          <span class="pills">预计就餐 {{ headcount }} 人 · 色阶按偏好高低</span>
        </div>
        <div class="days">
          <button
            v-for="day in days"
            :key="day.date"
            type="button"
            class="day"
            :class="{ active: day.date === date }"
            @click="date = day.date"
          >
            <span>{{ day.label }}</span>
            <strong>{{ day.md }}</strong>
            <el-icon v-if="day.date === date" :size="12"><Check /></el-icon>
          </button>
        </div>
        <div class="autosave">
          <span class="ok">● 勾选即保存</span>
          <el-icon :size="16" color="#9ca3af"><QuestionFilled /></el-icon>
        </div>
      </div>
      <div class="toolbar">
        <el-radio-group v-model="slot">
          <el-radio-button v-for="item in SLOTS" :key="item" :value="item">{{ SLOT_SHORT[item] }}</el-radio-button>
        </el-radio-group>
        <el-button type="primary" @click="jump">下一餐</el-button>
        <div class="legend">
          色阶（更高更绿 · 更低更红）
          <span class="lg high">高</span>
          <span class="lg mid">中</span>
          <span class="lg low">低</span>
        </div>
      </div>
      <div class="body">
        <div class="cols">
          <section v-for="kind in KINDS" :key="kind" class="col">
            <h3>{{ kind }}</h3>
            <div
              v-for="item in groups[kind]"
              :key="item.dish_id"
              class="dish-card"
              :class="{ selected: item.selected }"
              :style="{ borderColor: toneColor(item.tone) }"
              @click="toggle(kind, item.dish_id)"
            >
              <Thumb :src="item.image_url" round />
              <div class="meta">
                <div class="name">{{ item.name }}</div>
                <div class="tags">
                  <el-tag v-for="tag in item.taste_tags" :key="tag" size="small" effect="plain" round>{{ tag }}</el-tag>
                </div>
              </div>
              <el-checkbox :model-value="item.selected" @click.stop @change="toggle(kind, item.dish_id)" />
            </div>
            <p v-if="groups[kind].length === 0" class="muted">这一组没有启用的餐品</p>
          </section>
        </div>
        <aside class="right">
          <h3>本餐已选</h3>
          <div v-for="item in selected" :key="item.dish_id" class="sel-card" :style="{ borderColor: toneColor(item.tone) }">
            <Thumb :src="item.image_url" round />
            <div class="name">{{ item.name }}</div>
          </div>
          <div class="summary">已选 {{ selected.length }} 道</div>
        </aside>
      </div>
    </template>
  </div>
</template>

<style scoped>
.opt { display: flex; flex-direction: column; gap: 16px; }
.top-bar, .toolbar, .days, .legend, .autosave { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.label { font-weight: 700; font-size: 18px; margin-right: 8px; }
.pills { color: #6b7280; font-size: 13px; }
.days { flex: 1; justify-content: center; }
.day {
  border: 1px solid #e5e7eb; background: #fff; border-radius: 10px; padding: 8px 10px; min-width: 58px;
  display: flex; flex-direction: column; align-items: center; cursor: pointer; font-size: 12px; color: #6b7280;
}
.day strong { color: #111827; }
.day.active { background: #e8f5ec; border-color: #2d8a45; color: #2d8a45; }
.ok { color: #2d8a45; font-size: 13px; }
.legend { margin-left: auto; font-size: 12px; color: #6b7280; }
.lg { padding: 2px 8px; border-radius: 999px; color: #fff; font-weight: 600; }
.lg.high { background: #2d8a45; }
.lg.mid { background: #e6a23c; }
.lg.low { background: #e53935; }
.body { display: flex; gap: 16px; align-items: flex-start; }
.cols { flex: 1; display: grid; grid-template-columns: repeat(3, 1fr); gap: 12px; }
.col, .right { background: #fff; border: 1px solid #e8eaed; border-radius: 12px; padding: 12px; }
.right { width: 240px; }
.col h3, .right h3 { margin: 0 0 10px; }
.dish-card, .sel-card { display: flex; align-items: center; gap: 10px; border: 2px solid; border-radius: 10px; padding: 10px; margin-bottom: 8px; cursor: pointer; }
.meta { flex: 1; min-width: 0; }
.name { font-weight: 600; }
.tags { display: flex; gap: 4px; flex-wrap: wrap; margin-top: 4px; }
.summary { margin-top: 12px; padding-top: 12px; border-top: 1px solid #f0f1f3; font-size: 13px; }
.week-grid { display: grid; grid-template-columns: repeat(7, minmax(140px, 1fr)); gap: 10px; overflow-x: auto; }
.day-col { background: #fff; border: 1px solid #e8eaed; border-radius: 12px; padding: 10px; }
.day-col header { font-weight: 700; margin-bottom: 8px; }
.slot-cell {
  display: block; width: 100%; text-align: left; background: #f8faf9; border: 1px solid #e5e7eb;
  border-radius: 10px; padding: 8px; margin-bottom: 8px; cursor: pointer;
}
.slot-name { font-size: 12px; color: #6b7280; margin-bottom: 4px; }
.kind-line { display: flex; gap: 6px; font-size: 12px; margin-top: 2px; }
.k { color: #9ca3af; width: 16px; flex-shrink: 0; }
.names { display: flex; flex-direction: column; gap: 2px; font-weight: 600; }
@media (max-width: 1100px) { .cols { grid-template-columns: 1fr; } .body { flex-direction: column; } .right { width: 100%; } }
</style>

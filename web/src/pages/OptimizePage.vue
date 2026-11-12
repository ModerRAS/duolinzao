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

const mode = ref<"week" | "pick">("week");
const anchor = ref("");
const date = ref(shanghaiToday());
const slot = ref<Slot>("lunch");
const groups = ref<Groups<Candidate>>({ 菜: [], 饭: [], 汤: [] });
const headcount = ref(0);
const next = ref<{ date: string; slot: Slot } | null>(null);
const weekDays = ref<WeekDay[]>([]);

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

async function loadWeek() {
  if (!auth.canteenId) return;
  const body = await api<{ week_start: string; days: WeekDay[] }>(
    `/api/canteens/${auth.canteenId}/selection/week?date=${date.value}`,
  );
  anchor.value = body.week_start;
  weekDays.value = body.days.map((day) => ({
    date: day.date,
    slots: {
      breakfast: day.slots?.breakfast ?? emptyCell(),
      lunch: day.slots?.lunch ?? emptyCell(),
      dinner: day.slots?.dinner ?? emptyCell(),
    },
  }));
}

async function loadPick() {
  if (!auth.canteenId) return;
  const week = await api<{ week_start: string }>(
    `/api/canteens/${auth.canteenId}/selection/week?date=${date.value}`,
  );
  anchor.value = week.week_start;
  const body = await api<{
    expected_headcount: number;
    groups: Groups<Candidate>;
    next: { date: string; slot: Slot };
  }>(`/api/canteens/${auth.canteenId}/selection?date=${date.value}&slot=${slot.value}`);
  groups.value = body.groups;
  headcount.value = body.expected_headcount;
  next.value = body.next;
}

async function load() {
  try {
    if (mode.value === "week") await loadWeek();
    else await loadPick();
  } catch (error) {
    ElMessage.error(explain(error));
  }
}

async function save(nextGroups: Groups<Candidate>) {
  const ids = KINDS.flatMap((kind) => nextGroups[kind].filter((item) => item.selected).map((item) => item.dish_id));
  try {
    const body = await api<{
      expected_headcount: number;
      groups: Groups<Candidate>;
      next: { date: string; slot: Slot };
    }>(`/api/canteens/${auth.canteenId}/selection/${date.value}/${slot.value}`, {
      method: "PUT",
      body: JSON.stringify({ dish_ids: ids }),
    });
    groups.value = body.groups;
    headcount.value = body.expected_headcount;
    next.value = body.next;
  } catch (error) {
    ElMessage.error(explain(error));
    // 失败回读服务端态：写可能已生效（如响应超时），以服务端为准重建本地态。
    try {
      await loadPick();
    } catch {
      /* 已提示过，保持本地旧态 */
    }
  }
}

// 同餐 save 串行化：在途锁按餐键排队，toggle 永远基于最新本地态计算，
// 避免快速连点形成基于过期快照的并发全量 PUT。
const saveChains = new Map<string, Promise<void>>();

function mealKey() {
  return `${auth.canteenId}/${date.value}/${slot.value}`;
}

function toggle(kind: Kind, index: number) {
  const key = mealKey();
  const prev = saveChains.get(key) ?? Promise.resolve();
  const cur: Promise<void> = prev.catch(() => {}).then(async () => {
    const copy: Groups<Candidate> = {
      菜: groups.value.菜.map((item) => ({ ...item })),
      饭: groups.value.饭.map((item) => ({ ...item })),
      汤: groups.value.汤.map((item) => ({ ...item })),
    };
    copy[kind][index].selected = !copy[kind][index].selected;
    await save(copy);
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
              v-for="(item, index) in groups[kind]"
              :key="item.dish_id"
              class="dish-card"
              :class="{ selected: item.selected }"
              :style="{ borderColor: toneColor(item.tone) }"
              @click="toggle(kind, index)"
            >
              <Thumb :src="item.image_url" round />
              <div class="meta">
                <div class="name">{{ item.name }}</div>
                <div class="tags">
                  <el-tag v-for="tag in item.taste_tags" :key="tag" size="small" effect="plain" round>{{ tag }}</el-tag>
                </div>
              </div>
              <el-checkbox :model-value="item.selected" @click.stop @change="toggle(kind, index)" />
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

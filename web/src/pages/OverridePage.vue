<script setup lang="ts">
import { ref, watch } from "vue";
import { Lock } from "@element-plus/icons-vue";
import { ElMessage } from "element-plus";
import { api, personName, shanghaiToday, SLOT_NAME, type Slot } from "../api";
import { auth } from "../session";

interface Row {
  account_id: string;
  username: string;
  display_name: string | null;
  date: string;
  slot: Slot;
  canteen_id: string | null;
  ordering: boolean;
  locked_at: string | null;
}

const date = ref(shanghaiToday());
const rows = ref<Row[]>([]);

async function load() {
  if (!auth.canteenId) return;
  const body = await api<{ items: Row[] }>(`/api/admin/meals?date=${date.value}&canteen_id=${auth.canteenId}`);
  rows.value = body.items;
}

async function setOrdering(row: Row, ordering: boolean) {
  await api(`/api/admin/meals/${row.account_id}/${row.date}/${row.slot}`, {
    method: "PUT",
    body: JSON.stringify({ ordering, canteen_id: auth.canteenId }),
  });
  ElMessage.success("已改单");
  await load();
}

watch(() => [auth.canteenId, date.value], load, { immediate: true });
</script>

<template>
  <div>
    <div class="lock-banner">
      <el-icon :size="18"><Lock /></el-icon>
      <div>
        <strong>截止后仅管理员可改</strong>
        <span>修改作用于所选日期的报餐。未锁定时员工仍可自己改。</span>
      </div>
    </div>
    <div class="head">
      <h1 class="page-title">改单</h1>
      <el-date-picker v-model="date" type="date" value-format="YYYY-MM-DD" />
    </div>
    <div class="card">
      <el-table :data="rows">
        <el-table-column label="账号">
          <template #default="{ row }">
            <div>{{ personName(row) }}</div>
            <div v-if="row.display_name && row.display_name !== row.username" class="muted">{{ row.username }}</div>
          </template>
        </el-table-column>
        <el-table-column label="餐次" width="100">
          <template #default="{ row }">{{ SLOT_NAME[row.slot as Slot] }}</template>
        </el-table-column>
        <el-table-column label="是否报餐" width="120">
          <template #default="{ row }">{{ row.ordering ? "报" : "不报" }}</template>
        </el-table-column>
        <el-table-column label="锁定" width="100">
          <template #default="{ row }">{{ row.locked_at ? "已锁定" : "未锁定" }}</template>
        </el-table-column>
        <el-table-column label="操作" width="180">
          <template #default="{ row }">
            <el-button size="small" @click="setOrdering(row, !row.ordering)">{{ row.ordering ? "改为不报" : "改为报" }}</el-button>
          </template>
        </el-table-column>
      </el-table>
    </div>
  </div>
</template>

<style scoped>
.lock-banner { display: flex; gap: 10px; align-items: center; background: #e8f5ec; color: #246e37; border-radius: 12px; padding: 12px 14px; margin-bottom: 14px; }
.lock-banner span { display: block; font-size: 12px; }
.head { display: flex; justify-content: space-between; align-items: center; }
.card { overflow: hidden; }
</style>

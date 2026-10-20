<script setup lang="ts">
import { ref, watch } from "vue";
import { api, KINDS, personName, SLOT_NAME, type Kind, type Slot } from "../api";
import { auth } from "../session";

interface CommentRow {
  comment_id: string;
  body: string;
  anonymous: boolean;
  meal_date: string;
  slot: Slot;
  dish_name: string;
  kind: Kind;
  username: string | null;
  display_name: string | null;
}

const tab = ref<Kind>("菜");
const groups = ref<Record<Kind, CommentRow[]>>({ 菜: [], 饭: [], 汤: [] });

async function load() {
  if (!auth.canteenId) return;
  const body = await api<{ groups: Record<Kind, CommentRow[]> }>(
    `/api/admin/comments?canteen_id=${auth.canteenId}`,
  );
  groups.value = body.groups;
}

watch(() => auth.canteenId, load, { immediate: true });
</script>

<template>
  <div>
    <h1 class="page-title">评语</h1>
    <p class="muted">匿名评语不展示账号，评语不能删除。</p>
    <el-tabs v-model="tab">
      <el-tab-pane v-for="kind in KINDS" :key="kind" :label="kind" :name="kind" />
    </el-tabs>
    <div class="grid">
      <article v-for="item in groups[tab]" :key="item.comment_id" class="card item">
        <div class="top">
          <strong>{{ item.dish_name }}</strong>
          <el-tag size="small" effect="plain">{{ item.kind }}</el-tag>
          <span class="muted">{{ item.meal_date }} {{ SLOT_NAME[item.slot] }}</span>
        </div>
        <p>{{ item.body }}</p>
        <div class="muted">{{ item.anonymous ? "匿名" : personName(item) }}</div>
      </article>
      <p v-if="groups[tab].length === 0" class="muted">这一类还没有评语</p>
    </div>
  </div>
</template>

<style scoped>
.grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); gap: 12px; }
.item { padding: 14px 16px; }
.top { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
p { margin: 8px 0; }
</style>

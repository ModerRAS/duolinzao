<script setup lang="ts">
import { onMounted, ref } from "vue";
import { ElMessage, ElMessageBox } from "element-plus";
import { api, personName } from "../api";

interface Invite {
  id: string;
  created_at: string;
  used_at: string | null;
  used_by: string | null;
  used_by_display_name: string | null;
  max_uses: number;
  used_count: number;
  revoked_at: string | null;
}

const rows = ref<Invite[]>([]);
const shown = ref<string | null>(null);
const dialog = ref(false);
const maxUses = ref(1);

function usedLabel(row: Invite): string {
  if (!row.used_by) return "";
  const shown = personName({ display_name: row.used_by_display_name, username: row.used_by });
  if (shown !== row.used_by) return ` · ${shown}（${row.used_by}）`;
  return ` · ${row.used_by}`;
}

function statusOf(row: Invite): { text: string; color: string } {
  if (row.revoked_at) return { text: "已作废", color: "#6b7280" };
  if (row.used_count >= row.max_uses) return { text: `已用完 ${row.used_count}/${row.max_uses}`, color: "#6b7280" };
  if (row.used_count > 0) return { text: `使用中 ${row.used_count}/${row.max_uses}`, color: "#b45309" };
  return { text: `未使用 0/${row.max_uses}`, color: "#2d8a45" };
}

async function load() {
  const body = await api<{ items: Invite[] }>("/api/admin/invites");
  rows.value = body.items;
}

async function create() {
  const n = Math.floor(maxUses.value);
  if (!Number.isFinite(n) || n < 1) {
    ElMessage.error("可使用次数至少为 1");
    return;
  }
  const body = await api<{ code: string }>("/api/admin/invites", {
    method: "POST",
    body: JSON.stringify({ max_uses: n }),
  });
  shown.value = body.code;
  dialog.value = true;
  await load();
}

async function editMaxUses(row: Invite) {
  try {
    const { value } = await ElMessageBox.prompt(
      `已用 ${row.used_count} 次，可改成不小于 ${row.used_count} 的次数（改到等于已用次数即立即用尽）`,
      "修改可用次数",
      { inputValue: String(row.max_uses), inputType: "number" },
    );
    const n = Math.floor(Number(value));
    if (!Number.isFinite(n) || n < 1) {
      ElMessage.error("可使用次数至少为 1");
      return;
    }
    await api(`/api/admin/invites/${row.id}`, {
      method: "PATCH",
      body: JSON.stringify({ max_uses: n }),
    });
    ElMessage.success("已修改");
    await load();
  } catch (err) {
    if ((err as { code?: string })?.code) ElMessage.error((err as Error).message);
  }
}

async function revoke(row: Invite) {
  try {
    await ElMessageBox.confirm("作废后该邀请码不能再用于注册，是否继续？", "作废邀请码", { type: "warning" });
    await api(`/api/admin/invites/${row.id}`, { method: "DELETE" });
    ElMessage.success("已作废");
    await load();
  } catch (err) {
    if ((err as { code?: string })?.code) ElMessage.error((err as Error).message);
  }
}

async function copy() {
  if (!shown.value) return;
  await navigator.clipboard.writeText(shown.value);
  ElMessage.success("已复制");
}

onMounted(load);
</script>

<template>
  <div>
    <div class="head">
      <h1 class="page-title">邀请码</h1>
      <div class="issue">
        <el-input-number v-model="maxUses" :min="1" :precision="0" aria-label="可使用次数" />
        <el-button type="primary" @click="create">发放邀请码</el-button>
      </div>
    </div>
    <div class="card">
      <el-table :data="rows">
        <el-table-column label="邀请码">
          <template #default>完整码只在发放时出现一次</template>
        </el-table-column>
        <el-table-column prop="created_at" label="发放时间" />
        <el-table-column label="已用/上限" width="120">
          <template #default="{ row }">{{ (row as Invite).used_count }}/{{ (row as Invite).max_uses }}</template>
        </el-table-column>
        <el-table-column label="状态" width="180">
          <template #default="{ row }">
            <span :style="{ color: statusOf(row as Invite).color }">{{ statusOf(row as Invite).text }}{{ usedLabel(row as Invite) }}</span>
          </template>
        </el-table-column>
        <el-table-column label="操作" width="190">
          <template #default="{ row }">
            <el-button link type="primary" :disabled="!!(row as Invite).revoked_at" @click="editMaxUses(row as Invite)">改次数</el-button>
            <el-button link type="danger" :disabled="!!(row as Invite).revoked_at" @click="revoke(row as Invite)">作废</el-button>
          </template>
        </el-table-column>
      </el-table>
    </div>
    <el-dialog v-model="dialog" title="邀请码展示" width="440px" align-center @closed="shown = null">
      <div class="code">{{ shown }}</div>
      <el-alert type="warning" :closable="false" show-icon title="完整码仅显示一次，离开后列表不再显示全文" />
      <template #footer>
        <el-button type="primary" plain @click="copy">复制</el-button>
        <el-button @click="dialog = false">关闭</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.head { display: flex; justify-content: space-between; align-items: center; }
.issue { display: flex; gap: 8px; align-items: center; }
.card { overflow: hidden; }
.code { font-family: ui-monospace, monospace; font-size: 20px; font-weight: 700; text-align: center; padding: 8px 0 16px; word-break: break-all; }
</style>

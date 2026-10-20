<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { ElMessage, ElMessageBox } from "element-plus";
import { api, explain, personName } from "../api";

interface Account {
  id: string;
  username: string;
  display_name: string | null;
  email: string | null;
  has_password: boolean;
  passkey_count: number;
  totp: boolean;
}

const rows = ref<Account[]>([]);
const query = ref("");
const dialog = ref(false);
const target = ref("");
const password = ref("");

const filtered = computed(() => {
  const queryText = query.value.trim();
  return rows.value.filter(
    (item) => item.username.includes(queryText) || (item.display_name ?? "").includes(queryText),
  );
});

async function load() {
  const body = await api<{ items: Account[] }>("/api/admin/accounts");
  rows.value = body.items;
}

function openReset(username: string) {
  target.value = username;
  password.value = "";
  dialog.value = true;
}

async function reset() {
  try {
    await api("/api/admin/recovery", {
      method: "POST",
      body: JSON.stringify({ username: target.value, action: "reset_password", password: password.value }),
    });
    dialog.value = false;
    ElMessage.success("密码已重置");
    await load();
  } catch (error) {
    ElMessage.error(explain(error));
  }
}

async function clear(username: string) {
  try {
    await ElMessageBox.confirm(`清除 ${username} 的 Passkey、TOTP，并结束其会话。`, "清除登录材料", { type: "warning" });
  } catch {
    return;
  }
  try {
    await api("/api/admin/recovery", {
      method: "POST",
      body: JSON.stringify({ username, action: "clear_credentials" }),
    });
    ElMessage.success("已清除");
    await load();
  } catch (error) {
    ElMessage.error(explain(error));
  }
}

onMounted(load);
</script>

<template>
  <div>
    <h1 class="page-title">账号找回</h1>
    <el-alert type="info" show-icon :closable="false" class="mb" title="不展示密码哈希和密钥。这里没有员工自助申请。" />
    <el-input v-model="query" placeholder="按账号查找" style="max-width: 280px; margin-bottom: 12px" />
    <div class="card">
      <el-table :data="filtered">
        <el-table-column label="显示名" min-width="120">
          <template #default="{ row }">{{ personName(row) }}</template>
        </el-table-column>
        <el-table-column prop="username" label="账号" />
        <el-table-column label="邮箱" min-width="160">
          <template #default="{ row }">{{ row.email || "—" }}</template>
        </el-table-column>
        <el-table-column label="密码" width="90">
          <template #default="{ row }">{{ row.has_password ? "已设置" : "无" }}</template>
        </el-table-column>
        <el-table-column prop="passkey_count" label="Passkey" width="100" />
        <el-table-column label="TOTP" width="90">
          <template #default="{ row }">{{ row.totp ? "已确认" : "无" }}</template>
        </el-table-column>
        <el-table-column label="操作" width="220">
          <template #default="{ row }">
            <el-button link type="primary" @click="openReset(row.username)">重置密码</el-button>
            <el-button link type="danger" @click="clear(row.username)">清除登录材料</el-button>
          </template>
        </el-table-column>
      </el-table>
    </div>
    <el-dialog v-model="dialog" title="重置密码" width="420px">
      <p>账号 {{ target }}</p>
      <el-input v-model="password" type="password" show-password placeholder="新密码（至少 8 位）" />
      <template #footer>
        <el-button @click="dialog = false">取消</el-button>
        <el-button type="primary" @click="reset">确认</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.mb { margin-bottom: 16px; }
.card { overflow: hidden; }
</style>

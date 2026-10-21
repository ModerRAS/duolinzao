<script setup lang="ts">
import { onMounted, ref } from "vue";
import { ElMessage, ElMessageBox } from "element-plus";
import { api, explain } from "../api";

interface SigningStatus {
  configured: boolean;
  key_alias?: string | null;
  keystore_sha256?: string | null;
  updated_at?: string | null;
}

interface SignResult {
  versionCode: number;
  versionName: string;
  url: string;
  sha256: string;
  signing: string;
}

const status = ref<SigningStatus>({ configured: false });
const ready = ref(false);
const file = ref<File | null>(null);
const fileName = ref("");
const ksPass = ref("");
const alias = ref("");
const keyPass = ref("");
const uploading = ref(false);
const signing = ref(false);
const lastSign = ref<SignResult | null>(null);

async function load() {
  try {
    status.value = await api<SigningStatus>("/api/admin/app-signing");
  } catch (error) {
    ElMessage.error(explain(error));
  } finally {
    ready.value = true;
  }
}

function onFile(uploadFile: { raw?: File; name: string }) {
  // element-plus 的 on-change 回调只取 raw 文件，不自动上传。
  file.value = uploadFile.raw ?? null;
  fileName.value = uploadFile.raw?.name ?? uploadFile.name;
}

async function upload() {
  if (!file.value) {
    ElMessage.warning("先选择 keystore 文件（.jks）。");
    return;
  }
  if (!ksPass.value || !alias.value.trim() || !keyPass.value) {
    ElMessage.warning("keystore 密码、别名、key 密码都要填。");
    return;
  }
  uploading.value = true;
  try {
    const form = new FormData();
    form.append("keystore", file.value);
    form.append("keystore_password", ksPass.value);
    form.append("key_alias", alias.value.trim());
    form.append("key_password", keyPass.value);
    const body = await api<SigningStatus>("/api/admin/app-signing", { method: "PUT", body: form });
    status.value = body;
    ksPass.value = "";
    keyPass.value = "";
    file.value = null;
    fileName.value = "";
    ElMessage.success("签名材料已保存到数据库。");
  } catch (error) {
    ElMessage.error(explain(error));
  } finally {
    uploading.value = false;
  }
}

async function sign() {
  try {
    await ElMessageBox.confirm(
      "用数据库里的 keystore 给当前版本签名并发布？每次部署新镜像后都要点一次。",
      "签名发布",
      { type: "warning", confirmButtonText: "签名并发布", cancelButtonText: "取消" },
    );
  } catch {
    return;
  }
  signing.value = true;
  try {
    lastSign.value = await api<SignResult>("/api/admin/app-signing/sign", { method: "POST" });
    ElMessage.success(`已发布正式包（versionCode=${lastSign.value.versionCode}）。`);
  } catch (error) {
    ElMessage.error(explain(error));
  } finally {
    signing.value = false;
  }
}

onMounted(load);
</script>

<template>
  <div>
    <h1 class="page-title">应用签名</h1>
    <div class="card box">
      <div class="row">
        <div>
          <strong>签名状态</strong>
          <p class="muted">{{ ready && status.configured ? `已配置（别名 ${status.key_alias}）` : "未配置：/app/* 现在是 CI 未签包，App 装上也提示更新失败。" }}</p>
        </div>
        <el-tag v-if="ready" :type="status.configured ? 'success' : 'warning'">{{ status.configured ? "已配置" : "未配置" }}</el-tag>
      </div>
      <div v-if="status.configured" class="meta">
        <span>别名：{{ status.key_alias }}</span>
        <span class="mono">keystore 指纹：{{ status.keystore_sha256 }}</span>
      </div>
      <el-alert type="warning" show-icon :closable="false" title="签名材料只活在数据库。数据库备份即包含签名密钥，备份必须离线保护；换 keystore 后已装设备只能卸载重装。" />
      <el-alert type="info" show-icon :closable="false" title="每次部署新镜像后要点一次「签名发布」，否则 /app/* 还是未签包。" />
    </div>
    <div class="card box">
      <strong>上传 / 替换 keystore（超管一次性操作）</strong>
      <el-upload drag :auto-upload="false" :show-file-list="false" :on-change="onFile" accept=".jks,.keystore">
        <div class="up-hint">{{ fileName || "拖入 keystore 文件（.jks）" }}<br /><span>只进数据库，不进 git、不出内网</span></div>
      </el-upload>
      <el-form label-position="top" style="max-width: 520px">
        <el-form-item label="keystore 密码">
          <el-input v-model="ksPass" type="password" show-password autocomplete="new-password" />
        </el-form-item>
        <el-form-item label="key 别名">
          <el-input v-model="alias" placeholder="keytool 建库时填的 alias" />
        </el-form-item>
        <el-form-item label="key 密码">
          <el-input v-model="keyPass" type="password" show-password autocomplete="new-password" />
        </el-form-item>
        <el-button type="primary" :loading="uploading" @click="upload">保存到数据库</el-button>
      </el-form>
    </div>
    <div v-if="status.configured" class="card box">
      <div class="row">
        <div>
          <strong>签名发布</strong>
          <p class="muted">用库里 keystore 给当前镜像里的包签名，重算 sha256 并写出 version.json。</p>
        </div>
        <el-button type="success" :loading="signing" @click="sign">签名并发布</el-button>
      </div>
      <div v-if="lastSign" class="meta">
        <span>versionCode：{{ lastSign.versionCode }}（{{ lastSign.versionName }}）</span>
        <span class="mono">sha256：{{ lastSign.sha256 }}</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.box { padding: 18px; display: flex; flex-direction: column; gap: 16px; margin-bottom: 16px; }
.row { display: flex; justify-content: space-between; align-items: center; gap: 16px; }
.meta { display: flex; flex-direction: column; gap: 4px; font-size: 13px; color: #555; }
.mono { word-break: break-all; font-family: monospace; }
.up-hint { padding: 12px; }
</style>

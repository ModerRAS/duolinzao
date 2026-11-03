<script setup lang="ts">
import { onMounted, ref } from "vue";
import { ElMessage, ElMessageBox } from "element-plus";
import { api, explain } from "../api";

interface SigningStatus {
  configured: boolean;
  key_alias?: string | null;
  keystore_sha256?: string | null;
  updated_at?: string | null;
  release?: {
    version_code: number;
    version_name: string;
    signing: string;
    sha256: string;
  } | null;
}

interface SignResult {
  versionCode: number;
  versionName: string;
  url: string;
  sha256: string;
  signing: string;
  keystore_generated?: boolean;
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
const generating = ref(false);
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
  // 已配置时覆盖=换 key：已装设备应用内更新失败，只能卸载重装，先二次确认。
  if (status.value.configured) {
    try {
      await ElMessageBox.confirm(
        "已配置 keystore。覆盖上传后，已安装旧包的设备应用内更新会失败，只能卸载重装。继续覆盖？",
        "覆盖 keystore",
        { type: "warning", confirmButtonText: "覆盖", cancelButtonText: "取消" },
      );
    } catch {
      return;
    }
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
  // 空库时点「签名发布」即自动生成一把入库再签；已配置则直接用库里的签。
  const tip = status.value.configured
    ? "用数据库里的 keystore 给当前版本签名并发布？每次部署新镜像后都要点一次。"
    : "数据库里还没有 keystore。将自动生成一把 release 级 keystore 存进数据库，再用它签名并发布（之后仍可在同页覆盖成自己的 key）。继续？";
  try {
    await ElMessageBox.confirm(tip, "签名发布", {
      type: "warning",
      confirmButtonText: "签名并发布",
      cancelButtonText: "取消",
    });
  } catch {
    return;
  }
  signing.value = true;
  try {
    lastSign.value = await api<SignResult>("/api/admin/app-signing/sign", { method: "POST" });
    await load();
    ElMessage.success(`已发布正式包（versionCode=${lastSign.value.versionCode}）。`);
  } catch (error) {
    ElMessage.error(explain(error));
  } finally {
    signing.value = false;
  }
}

/// 受控分发：已发布包的下载入口只放本页（超管）。同源会话下载，
/// 匿名打 `/app/*` 直接 401；不要在员工壳/食堂壳/登录页/页脚再加入口。
const FALLBACK_APK = "/app/duolinzao.apk";

async function publishedApkUrl(): Promise<string> {
  try {
    const meta = await api<{ url?: string }>("/app/version.json");
    if (meta.url && meta.url.startsWith("/app/")) return meta.url;
  } catch {
    /* 未发布或会话失效时走默认路径，由服务端返回对应状态 */
  }
  return FALLBACK_APK;
}

function downloadApk() {
  void publishedApkUrl().then((url) => {
    window.location.href = url;
  });
}

async function copyLink() {
  try {
    const url = new URL(await publishedApkUrl(), window.location.origin).toString();
    await navigator.clipboard.writeText(url);
    ElMessage.success("下载链接已复制（打开需先登录）");
  } catch {
    ElMessage.error("复制失败");
  }
}

async function generate() {
  generating.value = true;
  try {
    status.value = await api<SigningStatus>("/api/admin/app-signing/generate", { method: "POST" });
    ElMessage.success("已自动生成 keystore 并存进数据库，可点「签名发布」。");
  } catch (error) {
    ElMessage.error(explain(error));
  } finally {
    generating.value = false;
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
          <p class="muted">{{ ready && status.configured ? `已配置（别名 ${status.key_alias}）` : "未配置：可点「签名发布」自动生成一把存进数据库，或自行上传 .jks。" }}</p>
        </div>
        <el-tag v-if="ready" :type="status.configured ? 'success' : 'warning'">{{ status.configured ? "已配置" : "未配置" }}</el-tag>
      </div>
      <div v-if="status.configured" class="meta">
        <span>别名：{{ status.key_alias }}</span>
        <span class="mono">keystore 指纹：{{ status.keystore_sha256 }}</span>
      </div>
      <p v-if="ready && status.release" class="muted">已发布：versionCode {{ status.release.version_code }}（{{ status.release.version_name }}，signing={{ status.release.signing }}）。</p>
      <p v-else-if="ready" class="muted">暂无已发布的包：新镜像就绪后会自动签名发布，也可手动点「签名并发布」。</p>
      <el-alert type="warning" show-icon :closable="false" title="签名材料只活在数据库。数据库备份即包含签名密钥，备份必须离线保护；换 keystore 后已装设备只能卸载重装。" />
      <el-alert type="info" show-icon :closable="false" title="每次部署新镜像后会自动签名发布（启动自检；失败时看服务端日志，也可在这里手动点）。" />
    </div>
    <div class="card box">
      <strong>上传 / 覆盖 keystore（随时可换成自己的）</strong>
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
    <div class="card box">
      <div class="row">
        <div>
          <strong>签名发布</strong>
          <p class="muted">空库时先自动生成一把 keystore 入库再签；已有则直接用库里的签，重算 sha256 并写出 version.json。</p>
        </div>
        <div class="actions">
          <el-button v-if="ready && !status.configured" :loading="generating" @click="generate">生成 keystore</el-button>
          <el-button type="success" :loading="signing" @click="sign">签名并发布</el-button>
        </div>
      </div>
      <div v-if="lastSign" class="meta">
        <span>versionCode：{{ lastSign.versionCode }}（{{ lastSign.versionName }}）</span>
        <span class="mono">sha256：{{ lastSign.sha256 }}</span>
        <span v-if="lastSign.keystore_generated">本次已自动生成 keystore 并存进数据库（别名 {{ status.key_alias }}）。</span>
      </div>
    </div>
    <div v-if="ready && status.release" class="card box">
      <div class="row">
        <div>
          <strong>下载安装包（受控分发）</strong>
          <p class="muted">已发布 versionCode {{ status.release.version_code }}（{{ status.release.version_name }}）。仅登录用户可下载；不要把链接贴到公开渠道，员工壳 / 登录页 / 页脚不设入口。</p>
        </div>
        <div class="actions">
          <el-button type="primary" @click="downloadApk">下载 APK</el-button>
          <el-button @click="copyLink">复制下载链接</el-button>
        </div>
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
.actions { display: flex; gap: 8px; align-items: center; }
</style>

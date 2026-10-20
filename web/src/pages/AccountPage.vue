<script setup lang="ts">
import { onMounted, ref, watch } from "vue";
import { ElMessage, ElMessageBox } from "element-plus";
import { api, explain, logout, personName } from "../api";
import { auth, refreshSession } from "../session";
import { prepareCreation, registrationToJson, unwrapPublicKey } from "../webauthn";

defineProps<{ embedded?: boolean }>();
const emit = defineEmits<{ "open-devices": [] }>();

interface Passkey {
  id: string;
  friendly_name: string | null;
}

const displayName = ref("");
const emailInput = ref("");
const passkeyName = ref("");
const passkeys = ref<Passkey[]>([]);
const secret = ref("");
const otpauth = ref("");
const totpCode = ref("");
const currentPassword = ref("");
const newPassword = ref("");
const confirmPassword = ref("");
const busy = ref(false);

watch(
  () => auth.me,
  (me) => {
    if (!me) return;
    displayName.value = me.display_name ?? "";
    emailInput.value = me.email ?? "";
  },
  { immediate: true },
);

async function loadPasskeys() {
  const body = await api<{ passkeys: Passkey[] }>("/api/devices");
  passkeys.value = body.passkeys;
}

async function saveName() {
  busy.value = true;
  try {
    await api("/api/me/display-name", {
      method: "PUT",
      body: JSON.stringify({ display_name: displayName.value }),
    });
    await refreshSession();
    ElMessage.success("显示名已保存");
  } catch (error) {
    ElMessage.error(explain(error));
  } finally {
    busy.value = false;
  }
}

async function bindPasskey() {
  if (!window.PublicKeyCredential) {
    ElMessage.error("这个浏览器不能绑定 Passkey");
    return;
  }
  busy.value = true;
  try {
    const started = await api<{ challenge_id: string; publicKey: Record<string, unknown> }>(
      "/api/passkeys/register/start",
      { method: "POST" },
    );
    const options = unwrapPublicKey(started);
    const credential = (await navigator.credentials.create({
      publicKey: prepareCreation(options),
    })) as PublicKeyCredential | null;
    if (!credential) return;
    const friendly = passkeyName.value.trim();
    await api("/api/passkeys/register/finish", {
      method: "POST",
      body: JSON.stringify({
        challenge_id: started.challenge_id,
        credential: registrationToJson(credential),
        friendly_name: friendly || null,
      }),
    });
    passkeyName.value = "";
    ElMessage.success("Passkey 已绑定");
    await loadPasskeys();
  } catch (error) {
    ElMessage.error(explain(error));
  } finally {
    busy.value = false;
  }
}

async function unbind(id: string) {
  try {
    await ElMessageBox.confirm("解绑后这把 Passkey 不能再登录。", "解绑 Passkey", { type: "warning" });
  } catch {
    return;
  }
  try {
    await api(`/api/passkeys/${id}`, { method: "DELETE" });
    ElMessage.success("已解绑");
    await loadPasskeys();
  } catch (error) {
    ElMessage.error(explain(error));
  }
}

async function startTotp() {
  if (auth.me?.totp) {
    try {
      await ElMessageBox.confirm(
        "重新设置会先停用现在的验证器，填入新验证码并确认后才恢复。",
        "重新设置验证器",
        { type: "warning" },
      );
    } catch {
      return;
    }
  }
  busy.value = true;
  try {
    const body = await api<{ secret: string; otpauth_url: string }>("/api/me/totp/setup", {
      method: "POST",
    });
    secret.value = body.secret;
    otpauth.value = body.otpauth_url;
    totpCode.value = "";
    if (auth.me) auth.me.totp = false;
    ElMessage.success("请把密钥加入验证器，再填写验证码");
  } catch (error) {
    ElMessage.error(explain(error));
  } finally {
    busy.value = false;
  }
}

async function confirmTotp() {
  busy.value = true;
  try {
    await api("/api/me/totp/confirm", {
      method: "POST",
      body: JSON.stringify({ code: totpCode.value.trim() }),
    });
    secret.value = "";
    otpauth.value = "";
    totpCode.value = "";
    await refreshSession();
    ElMessage.success("验证器已启用");
  } catch (error) {
    ElMessage.error(explain(error));
  } finally {
    busy.value = false;
  }
}

async function saveEmail() {
  busy.value = true;
  try {
    await api("/api/me/email", {
      method: "PUT",
      body: JSON.stringify({ email: emailInput.value.trim() }),
    });
    await refreshSession();
    ElMessage.success("邮箱已保存");
  } catch (error) {
    ElMessage.error(explain(error));
  } finally {
    busy.value = false;
  }
}

async function savePassword() {
  if (newPassword.value !== confirmPassword.value) {
    ElMessage.warning("两次输入的新密码不一致");
    return;
  }
  busy.value = true;
  try {
    const body = await api<{ revoked_other_sessions: number }>("/api/me/password", {
      method: "POST",
      body: JSON.stringify({
        current_password: currentPassword.value,
        new_password: newPassword.value,
      }),
    });
    currentPassword.value = "";
    newPassword.value = "";
    confirmPassword.value = "";
    const others = body.revoked_other_sessions;
    ElMessage.success(
      others > 0 ? `密码已更新，另外 ${others} 个登录已退出` : "密码已更新，当前登录仍然有效",
    );
  } catch (error) {
    ElMessage.error(explain(error));
  } finally {
    busy.value = false;
  }
}

async function signOut() {
  try {
    await ElMessageBox.confirm("退出后需要重新登录。", "退出登录", { type: "warning" });
  } catch {
    return;
  }
  try {
    await logout();
    auth.me = null;
  } catch (error) {
    ElMessage.error(explain(error));
  }
}

onMounted(async () => {
  try {
    await loadPasskeys();
  } catch (error) {
    ElMessage.error(explain(error));
  }
});
</script>

<template>
  <div class="account">
    <h1 class="page-title">账户设置</h1>
    <p class="muted lead">账号用于登录，不能在这里修改。界面上的人名优先用显示名。</p>

    <section class="card box">
      <h2>资料</h2>
      <label class="field">
        <span>显示名</span>
        <el-input v-model="displayName" maxlength="64" placeholder="显示名，留空则界面使用账号" />
      </label>
      <label class="field">
        <span>账号</span>
        <el-input :model-value="auth.me?.username ?? ''" disabled placeholder="账号" />
      </label>
      <p class="muted">当前展示：{{ personName(auth.me) }}</p>
      <el-button type="primary" native-type="button" :disabled="busy" @click="saveName">保存显示名</el-button>
    </section>

    <section class="card box">
      <h2>Passkey</h2>
      <p class="muted">登录之后可以补绑。解绑只影响这一把钥匙。</p>
      <el-input v-model="passkeyName" placeholder="Passkey 名称，例如办公室电脑" />
      <el-button type="primary" native-type="button" :disabled="busy" @click="bindPasskey">绑定 Passkey</el-button>
      <div v-for="item in passkeys" :key="item.id" class="row">
        <span>{{ item.friendly_name || "Passkey" }}</span>
        <el-button type="danger" plain size="small" native-type="button" @click="unbind(item.id)">解绑</el-button>
      </div>
      <p v-if="passkeys.length === 0" class="muted">还没有绑定 Passkey</p>
      <el-button native-type="button" @click="emit('open-devices')">设备与 Passkey</el-button>
    </section>

    <section class="card box">
      <h2>验证器</h2>
      <p class="muted">{{ auth.me?.totp ? "已启用" : "尚未启用" }}。验证码来自认证器。</p>
      <el-button native-type="button" :disabled="busy" @click="startTotp">
        {{ auth.me?.totp ? "重新设置验证器" : "设置验证器" }}
      </el-button>
      <template v-if="secret">
        <p class="muted">密钥</p>
        <code class="secret">{{ secret }}</code>
        <p class="muted break">{{ otpauth }}</p>
        <el-input v-model="totpCode" maxlength="8" placeholder="6 位验证码" />
        <el-button type="primary" native-type="button" :disabled="busy" @click="confirmTotp">确认验证器</el-button>
      </template>
    </section>

    <section class="card box">
      <h2>邮箱</h2>
      <el-alert
        v-if="!auth.me?.email_enabled"
        type="info"
        show-icon
        :closable="false"
        title="邮箱能力已关闭，不能在这里绑定或修改邮箱。"
      />
      <template v-else>
        <p class="muted">
          {{ auth.me?.email ? (auth.me.email_verified ? "已验证" : "未验证") : "尚未绑定" }}
        </p>
        <el-input v-model="emailInput" placeholder="邮箱" />
        <el-button type="primary" native-type="button" :disabled="busy" @click="saveEmail">保存邮箱</el-button>
      </template>
    </section>

    <section class="card box">
      <h2>密码</h2>
      <p class="muted">改成功后，这台设备保持登录，其它登录会退出。</p>
      <el-input v-model="currentPassword" type="password" show-password placeholder="当前密码" autocomplete="current-password" />
      <el-input v-model="newPassword" type="password" show-password placeholder="新密码，至少 8 位" autocomplete="new-password" />
      <el-input v-model="confirmPassword" type="password" show-password placeholder="再输入一次新密码" autocomplete="new-password" />
      <el-button type="primary" native-type="button" :disabled="busy" @click="savePassword">更新密码</el-button>
    </section>

    <section class="card box">
      <h2>会话</h2>
      <el-button type="danger" native-type="button" @click="signOut">退出登录</el-button>
    </section>
  </div>
</template>

<style scoped>
.account { max-width: 640px; display: flex; flex-direction: column; gap: 12px; }
.lead { margin-top: 0; }
.box { padding: 16px; display: flex; flex-direction: column; gap: 10px; }
h2 { margin: 0; font-size: 16px; }
.field { display: flex; flex-direction: column; gap: 4px; font-size: 13px; color: #6b7280; }
.row { display: flex; justify-content: space-between; align-items: center; gap: 8px; }
.secret {
  display: block; padding: 8px 10px; background: #f3f5f7; border-radius: 8px;
  word-break: break-all; font-size: 14px;
}
.break { word-break: break-all; }
</style>

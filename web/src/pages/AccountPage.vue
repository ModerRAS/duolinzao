<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { ElMessage, ElMessageBox } from "element-plus";
import { api, explain, logout, personName } from "../api";
import { auth, refreshSession } from "../session";
import { prepareCreation, registrationToJson, unwrapPublicKey } from "../webauthn";

defineProps<{ embedded?: boolean }>();
const emit = defineEmits<{ "open-devices": [] }>();

type Section = "profile" | "security" | "device";
type Panel = "name" | "passkey" | "totp" | "email" | "password";

interface Passkey {
  id: string;
  friendly_name: string | null;
  created_at: string;
}

const section = ref<Section>("profile");
const panel = ref<Panel | null>(null);
const displayName = ref("");
const emailInput = ref("");
const passkeyName = ref("");
const addingPasskey = ref(false);
const passkeys = ref<Passkey[]>([]);
const secret = ref("");
const otpauth = ref("");
const totpCode = ref("");
const currentPassword = ref("");
const newPassword = ref("");
const confirmPassword = ref("");
const passwordError = ref("");
const busy = ref(false);

const shownName = computed(() => personName(auth.me));
const hasDisplayName = computed(() => {
  const name = auth.me?.display_name?.trim();
  return Boolean(name) && name !== auth.me?.username;
});
const initial = computed(() => [...shownName.value][0] || "灶");
const displayLabel = computed(() => auth.me?.display_name?.trim() || "未设置");
const passkeyStatus = computed(() => (passkeys.value.length ? `已绑定 ${passkeys.value.length} 个` : "未设置"));
const totpStatus = computed(() => {
  if (secret.value && !auth.me?.totp) return "待确认";
  return auth.me?.totp ? "已开启" : "未开启";
});
const emailStatus = computed(() => {
  if (!auth.me?.email_enabled) return "已关闭";
  if (!auth.me.email) return "未绑定";
  return auth.me.email;
});
const strength = computed(() => {
  const value = newPassword.value;
  const length = [...value].length;
  if (!length) return 0;
  if (length < 8) return 1;
  const kinds = [/[a-z]/.test(value), /[A-Z]/.test(value), /\d/.test(value), /[^\da-zA-Z]/.test(value)].filter(Boolean).length;
  return kinds >= 3 ? 3 : 2;
});
const passwordHint = computed(() => {
  const tail = "其它设备会被退出，本机保持登录。";
  if (strength.value === 0) return `至少 8 个字符。${tail}`;
  if (strength.value === 1) return `还差 ${8 - [...newPassword.value].length} 个字符。${tail}`;
  if (strength.value === 2) return `长度已经够了。混入大小写、数字或符号会更好认。${tail}`;
  return `这样就可以。${tail}`;
});
const confirmHint = computed(() => {
  if (!confirmPassword.value || !newPassword.value) return "";
  return confirmPassword.value === newPassword.value ? "" : "两次输入的新密码不一致";
});
const panelTitle = computed(() => {
  if (panel.value === "name") return "显示名";
  if (panel.value === "passkey") return "Passkey";
  if (panel.value === "totp") return "验证器";
  if (panel.value === "email") return "邮箱";
  return "密码";
});
const sectionTitle = computed(() => {
  if (section.value === "profile") return "个人信息";
  if (section.value === "security") return "登录与安全";
  return "设备";
});

watch(
  () => auth.me,
  (me) => {
    if (!me) return;
    displayName.value = me.display_name ?? "";
    emailInput.value = me.email ?? "";
  },
  { immediate: true },
);

watch(panel, () => {
  window.scrollTo({ top: 0 });
});

function pick(next: Section) {
  section.value = next;
  panel.value = null;
  addingPasskey.value = false;
}

function openPanel(next: Panel) {
  panel.value = next;
  passwordError.value = "";
  if (next === "name") section.value = "profile";
  else if (next === "passkey" || next === "totp" || next === "email" || next === "password") section.value = "security";
  if (next === "passkey") addingPasskey.value = false;
}

function closePanel() {
  panel.value = null;
  addingPasskey.value = false;
  passwordError.value = "";
}

function formatWhen(iso: string) {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  return new Intl.DateTimeFormat("zh-CN", {
    timeZone: "Asia/Shanghai",
    year: "numeric",
    month: "short",
    day: "numeric",
  }).format(date);
}

function defaultPasskeyName() {
  const ua = navigator.userAgent;
  let browser = "浏览器";
  if (ua.includes("Edg/")) browser = "Edge";
  else if (ua.includes("Chrome/")) browser = "Chrome";
  else if (ua.includes("Firefox/")) browser = "Firefox";
  else if (ua.includes("Safari/") && !ua.includes("Chrome/")) browser = "Safari";
  let os = "这台设备";
  if (ua.includes("Windows")) os = "Windows";
  else if (ua.includes("Mac OS")) os = "macOS";
  else if (ua.includes("Android")) os = "Android";
  else if (ua.includes("iPhone") || ua.includes("iPad")) os = "iOS";
  else if (ua.includes("Linux")) os = "Linux";
  return `${browser} · ${os}`;
}

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
    closePanel();
    ElMessage.success("显示名已保存");
  } catch (error) {
    ElMessage.error(explain(error));
  } finally {
    busy.value = false;
  }
}

function beginAddPasskey() {
  passkeyName.value = defaultPasskeyName();
  addingPasskey.value = true;
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
    addingPasskey.value = false;
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
    await ElMessageBox.confirm("解绑后这把 Passkey 不能再登录。", "解绑 Passkey", {
      type: "warning",
      confirmButtonText: "解绑",
      cancelButtonText: "取消",
      confirmButtonType: "danger",
    });
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
    closePanel();
    ElMessage.success("邮箱已保存");
  } catch (error) {
    ElMessage.error(explain(error));
  } finally {
    busy.value = false;
  }
}

async function savePassword() {
  passwordError.value = "";
  if (confirmPassword.value !== newPassword.value) {
    passwordError.value = "两次输入的新密码不一致";
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
    ElMessage.success(others > 0 ? `密码已更新，另外 ${others} 个登录已退出` : "密码已更新，当前登录仍然有效");
  } catch (error) {
    passwordError.value = explain(error);
  } finally {
    busy.value = false;
  }
}

async function signOut() {
  try {
    await ElMessageBox.confirm("退出后需要重新登录。", "退出登录", {
      type: "warning",
      confirmButtonText: "退出登录",
      cancelButtonText: "取消",
      confirmButtonType: "danger",
    });
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
  <div class="account" :class="{ embedded }">
    <h1 v-if="!embedded" class="page-title">账户设置</h1>
    <div class="layout">
      <aside v-if="!embedded" class="rail">
        <div class="identity">
          <div class="avatar" aria-hidden="true">{{ initial }}</div>
          <div>
            <strong>{{ shownName }}</strong>
            <span v-if="hasDisplayName">{{ auth.me?.username }}</span>
          </div>
        </div>
        <nav class="rail-nav" aria-label="账户设置">
          <button type="button" :class="{ on: section === 'profile' }" @click="pick('profile')">个人信息</button>
          <button type="button" :class="{ on: section === 'security' }" @click="pick('security')">登录与安全</button>
          <button type="button" :class="{ on: section === 'device' }" @click="pick('device')">设备</button>
        </nav>
        <button type="button" class="rail-logout" @click="signOut">退出登录</button>
      </aside>

      <div class="pane">
        <template v-if="panel">
          <button type="button" class="back" @click="closePanel">
            <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M10 3.5 5.5 8 10 12.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>
            返回
          </button>
          <h2>{{ panelTitle }}</h2>

          <div v-if="panel === 'name'" class="editor">
            <p class="muted">给人看的名字。留空后，界面会改用账号。</p>
            <label class="field">
              <span>显示名</span>
              <el-input v-model="displayName" maxlength="64" placeholder="显示名，留空则界面使用账号" />
            </label>
            <div class="form-actions">
              <el-button type="primary" native-type="button" :disabled="busy" @click="saveName">保存</el-button>
            </div>
          </div>

          <div v-else-if="panel === 'passkey'" class="editor">
            <p class="muted">用来登录的钥匙。解绑只影响这一把。</p>
            <p v-if="passkeys.length === 0 && !addingPasskey" class="empty">还没有绑定 Passkey。添加后可以用它登录。</p>
            <div v-for="item in passkeys" :key="item.id" class="passkey-item">
              <div>
                <strong>{{ item.friendly_name || "Passkey" }}</strong>
                <span class="muted">{{ formatWhen(item.created_at) }} 添加</span>
              </div>
              <button type="button" class="text-danger" @click="unbind(item.id)">解绑</button>
            </div>
            <div v-if="addingPasskey" class="composer">
              <label class="field">
                <span>名称</span>
                <el-input v-model="passkeyName" maxlength="64" placeholder="Passkey 名称，例如办公室电脑" />
              </label>
              <p class="muted">已按浏览器和系统填了一个名字，可以改。</p>
              <div class="form-actions">
                <el-button type="primary" native-type="button" :disabled="busy" @click="bindPasskey">绑定 Passkey</el-button>
                <button type="button" class="text" :disabled="busy" @click="addingPasskey = false">取消</button>
              </div>
            </div>
            <div v-else class="form-actions">
              <el-button type="primary" native-type="button" :disabled="busy" @click="beginAddPasskey">添加 Passkey</el-button>
            </div>
          </div>

          <div v-else-if="panel === 'totp'" class="editor">
            <p class="status-line">
              <span class="pill" :class="auth.me?.totp ? 'good' : 'idle'">{{ totpStatus }}</span>
              <span class="muted">验证码来自认证器。</span>
            </p>
            <div class="form-actions" v-if="!secret">
              <el-button type="primary" native-type="button" :disabled="busy" @click="startTotp">
                {{ auth.me?.totp ? "重新设置验证器" : "设置验证器" }}
              </el-button>
            </div>
            <template v-else>
              <p class="kicker">密钥</p>
              <code class="secret">{{ secret }}</code>
              <p class="muted break">{{ otpauth }}</p>
              <label class="field">
                <span>验证码</span>
                <el-input v-model="totpCode" maxlength="8" placeholder="6 位验证码" />
              </label>
              <div class="form-actions">
                <el-button type="primary" native-type="button" :disabled="busy" @click="confirmTotp">确认验证器</el-button>
              </div>
            </template>
          </div>

          <div v-else-if="panel === 'email'" class="editor">
            <div v-if="!auth.me?.email_enabled" class="note">邮箱能力已关闭，不能在这里绑定或修改邮箱。</div>
            <template v-else>
              <p class="status-line">
                <span class="pill" :class="auth.me?.email_verified ? 'good' : 'idle'">
                  {{ auth.me?.email ? (auth.me.email_verified ? "已验证" : "未验证") : "未绑定" }}
                </span>
              </p>
              <label class="field">
                <span>邮箱</span>
                <el-input v-model="emailInput" placeholder="邮箱" />
              </label>
              <div class="form-actions">
                <el-button type="primary" native-type="button" :disabled="busy" @click="saveEmail">保存邮箱</el-button>
              </div>
            </template>
          </div>

          <div v-else class="editor">
            <p class="muted">其它设备会被退出，本机保持登录。</p>
            <label class="field">
              <span>当前密码</span>
              <el-input v-model="currentPassword" type="password" show-password placeholder="当前密码" autocomplete="current-password" />
            </label>
            <label class="field">
              <span>新密码</span>
              <el-input v-model="newPassword" type="password" show-password placeholder="新密码，至少 8 位" autocomplete="new-password" />
              <span class="meter" :data-level="strength" aria-hidden="true"><span></span></span>
              <span class="muted">{{ passwordHint }}</span>
            </label>
            <label class="field">
              <span>确认新密码</span>
              <el-input v-model="confirmPassword" type="password" show-password placeholder="再输入一次新密码" autocomplete="new-password" />
              <span v-if="confirmHint" class="form-error">{{ confirmHint }}</span>
            </label>
            <p v-if="passwordError" class="form-error" role="alert">{{ passwordError }}</p>
            <div class="form-actions">
              <el-button type="primary" native-type="button" :disabled="busy" @click="savePassword">更新密码</el-button>
            </div>
          </div>
        </template>

        <template v-else>
          <h2 v-if="!embedded" class="pane-title">{{ sectionTitle }}</h2>

          <div v-if="embedded" class="identity">
            <div class="avatar" aria-hidden="true">{{ initial }}</div>
            <div>
              <strong>{{ shownName }}</strong>
              <span v-if="hasDisplayName">{{ auth.me?.username }}</span>
            </div>
          </div>

          <section v-if="embedded || section === 'profile'" class="group">
            <h2 v-if="embedded">个人信息</h2>
            <div class="inset">
              <button type="button" class="set-row" aria-label="显示名" @click="openPanel('name')">
                <span>显示名</span>
                <span class="set-value">{{ displayLabel }}</span>
                <svg class="chev" viewBox="0 0 16 16" aria-hidden="true"><path d="M6 3.5 10.5 8 6 12.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>
              </button>
              <div class="set-row is-static">
                <span>账号</span>
                <span class="set-value">{{ auth.me?.username }}</span>
              </div>
            </div>
          </section>

          <section v-if="embedded || section === 'security'" class="group">
            <h2 v-if="embedded">登录与安全</h2>
            <div class="inset">
              <button type="button" class="set-row" aria-label="Passkey" @click="openPanel('passkey')">
                <span>Passkey</span>
                <span class="set-value">{{ passkeyStatus }}</span>
                <svg class="chev" viewBox="0 0 16 16" aria-hidden="true"><path d="M6 3.5 10.5 8 6 12.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>
              </button>
              <button type="button" class="set-row" aria-label="验证器" @click="openPanel('totp')">
                <span>验证器</span>
                <span class="set-value" :class="{ good: auth.me?.totp }">{{ totpStatus }}</span>
                <svg class="chev" viewBox="0 0 16 16" aria-hidden="true"><path d="M6 3.5 10.5 8 6 12.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>
              </button>
              <button type="button" class="set-row" aria-label="邮箱" @click="openPanel('email')">
                <span>邮箱</span>
                <span class="set-value">{{ emailStatus }}</span>
                <svg class="chev" viewBox="0 0 16 16" aria-hidden="true"><path d="M6 3.5 10.5 8 6 12.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>
              </button>
              <button type="button" class="set-row" aria-label="密码" @click="openPanel('password')">
                <span>密码</span>
                <span class="set-value">更改</span>
                <svg class="chev" viewBox="0 0 16 16" aria-hidden="true"><path d="M6 3.5 10.5 8 6 12.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>
              </button>
            </div>
          </section>

          <section v-if="embedded || section === 'device'" class="group">
            <h2 v-if="embedded">设备</h2>
            <div class="inset">
              <button type="button" class="set-row" aria-label="设备与 Passkey" @click="emit('open-devices')">
                <span>设备与 Passkey</span>
                <span class="set-value">登录设备</span>
                <svg class="chev" viewBox="0 0 16 16" aria-hidden="true"><path d="M6 3.5 10.5 8 6 12.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>
              </button>
            </div>
            <p class="muted group-note">新设备确认和踢下线在设备页，只处理本账号。</p>
          </section>

          <section v-if="embedded" class="group">
            <div class="inset">
              <button type="button" class="set-row danger" @click="signOut">退出登录</button>
            </div>
          </section>
        </template>
      </div>
    </div>
  </div>
</template>

<style scoped>
.account.embedded { max-width: none; }
.account:not(.embedded) { width: min(1120px, 100%); margin: 0 auto; }
.layout { display: block; }
.account:not(.embedded) .layout {
  display: grid;
  grid-template-columns: 260px minmax(0, 1fr);
  gap: 24px;
  align-items: start;
}
.rail {
  position: sticky;
  top: 12px;
  background: #fff;
  border: 1px solid #e8eaed;
  border-radius: 14px;
  padding: 14px 12px 12px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.rail .identity { flex-direction: column; text-align: center; border: none; padding: 8px 8px 4px; background: transparent; }
.rail .identity strong { font-size: 16px; }
.rail-nav { display: flex; flex-direction: column; gap: 2px; }
.rail-nav button, .rail-logout {
  width: 100%;
  text-align: left;
  border: none;
  background: transparent;
  border-radius: 10px;
  padding: 10px 12px;
  font: inherit;
  color: #374151;
  cursor: pointer;
}
.rail-nav button.on { background: #e8f5ec; color: #2d8a45; font-weight: 650; }
.rail-logout { color: #e74c3c; margin-top: auto; }
.identity {
  display: flex;
  align-items: center;
  gap: 14px;
  background: #fff;
  border: 1px solid #e8eaed;
  border-radius: 14px;
  padding: 16px;
  margin-bottom: 18px;
}
.identity strong { display: block; font-size: 18px; line-height: 1.3; }
.identity span { color: #6b7280; font-size: 13px; }
.avatar {
  width: 56px; height: 56px; border-radius: 50%; flex: none;
  display: grid; place-items: center;
  background: #e8f5ec; color: #246e37; font-size: 22px; font-weight: 700;
}
.pane-title, .editor h2, .group > h2 { margin: 0; }
.pane-title { font-size: 20px; margin-bottom: 12px; }
.group { margin-bottom: 18px; }
.group > h2 {
  font-size: 13px; font-weight: 650; color: #6b7280;
  margin: 0 4px 8px;
}
.inset { background: #fff; border: 1px solid #e8eaed; border-radius: 14px; overflow: hidden; }
.set-row {
  width: 100%;
  min-height: 52px;
  padding: 12px 14px;
  display: flex;
  align-items: center;
  gap: 12px;
  border: none;
  background: #fff;
  color: #1f2937;
  font: inherit;
  text-align: left;
  cursor: pointer;
}
.set-row + .set-row { box-shadow: inset 0 1px 0 #eef0f2; }
.set-row.is-static { cursor: default; }
.set-value {
  margin-left: auto;
  color: #6b7280;
  font-size: 14px;
  max-width: 58%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.set-value.good { color: #246e37; }
.chev { width: 16px; height: 16px; color: #c4c8ce; flex: none; }
.set-row.danger { color: #e74c3c; justify-content: center; font-weight: 650; }
.group-note { margin: 8px 4px 0; }
.back {
  align-self: flex-start;
  display: inline-flex;
  align-items: center;
  gap: 2px;
  border: none; background: transparent; color: #2d8a45; font: inherit; font-weight: 650;
  padding: 0 0 4px; cursor: pointer;
}
.back svg { width: 16px; height: 16px; }
.account.embedded .identity {
  flex-direction: column;
  text-align: center;
  justify-content: center;
  border: none;
  background: transparent;
  padding: 6px 8px 2px;
  margin-bottom: 6px;
}
.account.embedded .avatar { width: 72px; height: 72px; font-size: 28px; }
.account.embedded .identity strong { font-size: 22px; }
.account.embedded .set-row { min-height: 56px; padding: 14px 16px; }
.editor { display: flex; flex-direction: column; gap: 12px; max-width: 480px; }
.embedded .editor { max-width: none; }
.editor h2 { font-size: 22px; }
.field { display: flex; flex-direction: column; gap: 6px; font-size: 13px; color: #6b7280; }
.form-actions { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; }
.embedded .form-actions { flex-direction: column; align-items: stretch; }
.embedded .form-actions :deep(.el-button--primary) { width: 100%; margin-left: 0; }
.embedded .form-actions .text { align-self: center; }
.meter { display: block; height: 6px; margin-top: 6px; border-radius: 99px; background: #e8eaed; overflow: hidden; }
.meter > span { display: block; height: 100%; width: 0; background: #e5e7eb; transition: width 0.2s ease; }
.meter[data-level="1"] > span { width: 34%; background: #e74c3c; }
.meter[data-level="2"] > span { width: 68%; background: #d97706; }
.meter[data-level="3"] > span { width: 100%; background: #2d8a45; }
.form-error { color: #e74c3c; font-size: 13px; margin: 0; }
.passkey-item, .composer {
  display: flex; justify-content: space-between; align-items: center; gap: 12px;
  background: #fff; border: 1px solid #e8eaed; border-radius: 14px; padding: 12px 14px;
}
.passkey-item strong, .composer { display: block; }
.passkey-item .muted { display: block; }
.composer { flex-direction: column; align-items: stretch; }
.text, .text-danger {
  border: none; background: transparent; font: inherit; cursor: pointer; padding: 4px 0;
}
.text { color: #6b7280; }
.text-danger { color: #e74c3c; font-weight: 650; }
.empty, .note {
  margin: 0;
  background: #fff;
  border: 1px solid #e8eaed;
  border-radius: 14px;
  padding: 14px 16px;
  color: #6b7280;
  font-size: 14px;
}
.note { color: #374151; }
.status-line { display: flex; align-items: center; gap: 8px; margin: 0; }
.pill {
  display: inline-flex; align-items: center; min-height: 24px; padding: 0 8px;
  border-radius: 999px; background: #f3f4f6; color: #6b7280; font-size: 12px; font-weight: 650;
}
.pill.good { background: #e8f5ec; color: #246e37; }
.pill.idle { background: #f3f4f6; color: #6b7280; }
.kicker { margin: 0; font-size: 13px; color: #6b7280; }
.secret {
  display: block; padding: 10px 12px; background: #fff; border: 1px solid #e8eaed; border-radius: 12px;
  word-break: break-all; font-size: 14px;
}
.break { word-break: break-all; margin: 0; }
@media (max-width: 800px) {
  .account:not(.embedded) .layout { grid-template-columns: 1fr; }
  .rail { position: static; }
}
</style>

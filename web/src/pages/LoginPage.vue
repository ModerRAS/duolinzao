<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { ElMessage } from "element-plus";
import { ApiError, api, type Me } from "../api";
import { refreshSession } from "../session";
import { appVersion, loadVersion } from "../version";
import { prepareCreation, registrationToJson, unwrapPublicKey } from "../webauthn";

const props = defineProps<{ enrolling?: boolean }>();
const emit = defineEmits<{ enter: [] }>();

type Screen = "home" | "methods" | "password" | "code" | "email" | "wait" | "register" | "passkey" | "fallback";

const screen = ref<Screen>("home");
const methods = ref<string[]>(["passkey", "totp", "password"]);
const emailEnabled = ref(false);
const username = ref("");
const password = ref("");
const code = ref("");
const otpCode = ref("");
const regUser = ref("");
const regCode = ref("");
const binding = ref(false);
const fallbackPassword = ref("");
const fallbackConfirm = ref("");
const canTryPasskey = ref(typeof window !== "undefined" && !!window.PublicKeyCredential);
const wait = ref<{ id: string; poll: string; ip: string; ua: string; name: string; status: string } | null>(null);

const account = computed(() => username.value.trim() || "账号");
const avatar = computed(() => (username.value.trim().slice(0, 1) || "账"));
const showEmail = computed(() => emailEnabled.value && methods.value.includes("email_otp"));

onMounted(async () => {
  loadVersion();
  canTryPasskey.value = !!window.PublicKeyCredential;
  if (props.enrolling) screen.value = canTryPasskey.value ? "passkey" : "fallback";
  try {
    const body = await api<{ methods: string[]; email_enabled: boolean }>("/api/session/methods");
    methods.value = body.methods;
    emailEnabled.value = body.email_enabled;
  } catch {
    /* 保持默认顺序 */
  }
});

async function enter() {
  await refreshSession();
  emit("enter");
}

function goNext() {
  if (!username.value.trim()) {
    document.getElementById("login-username")?.focus();
    return;
  }
  screen.value = "methods";
}

async function passwordLogin() {
  try {
    await api("/api/session/password", {
      method: "POST",
      body: JSON.stringify({ username: username.value, password: password.value }),
    });
    await enter();
  } catch (error) {
    ElMessage.error(error instanceof Error ? error.message : "登录失败");
  }
}

async function codeLogin() {
  try {
    await api("/api/session/totp", {
      method: "POST",
      body: JSON.stringify({ username: username.value, code: code.value }),
    });
    await enter();
  } catch (error) {
    ElMessage.error(error instanceof Error ? error.message : "验证码不正确");
  }
}

async function requestEmailOtp() {
  try {
    await api("/api/session/email-otp/request", {
      method: "POST",
      body: JSON.stringify({ username: username.value }),
    });
    ElMessage.success("如果账号已绑定邮箱，验证码已发出");
  } catch (error) {
    ElMessage.error(error instanceof Error ? error.message : "无法发送");
  }
}

async function verifyEmailOtp() {
  try {
    await api("/api/session/email-otp/verify", {
      method: "POST",
      body: JSON.stringify({ username: username.value, code: otpCode.value }),
    });
    await enter();
  } catch (error) {
    ElMessage.error(error instanceof Error ? error.message : "验证码不正确");
  }
}

async function passkeyLogin(username?: string) {
  // 免填账号：直接 start（无 username），浏览器/系统弹账号选择器。
  // 传了账号则走带 allowCredentials 的命名路径，兼容非 discoverable 旧钥匙。
  const named = username?.trim() ? { username: username.trim() } : {};
  try {
    const started = await api<{ challenge_id: string; publicKey: Record<string, unknown> }>(
      "/api/session/passkey/start",
      { method: "POST", body: JSON.stringify(named) },
    );
    const publicKey = prepareRequest(unwrapPublicKey(started) as unknown as PublicKeyCredentialRequestOptions);
    let credential: PublicKeyCredential | null;
    try {
      credential = (await navigator.credentials.get({ publicKey })) as PublicKeyCredential | null;
    } catch (error) {
      // 用户取消 / 无可用凭证：都是本地行为，不打 error toast 吓人。
      if (error instanceof DOMException && (error.name === "NotAllowedError" || error.name === "AbortError")) {
        ElMessage.info("已取消通行密钥登录");
        return;
      }
      ElMessage.info("没有可用的通行密钥，请用密码登录");
      return;
    }
    if (!credential) return;
    await api("/api/session/passkey/finish", {
      method: "POST",
      body: JSON.stringify({
        ...named,
        challenge_id: started.challenge_id,
        credential: credentialToJson(credential),
      }),
    });
    await enter();
  } catch (error) {
    if (error instanceof ApiError && error.code === "passkey_none") {
      ElMessage.info("这个账号还没有通行密钥，请用下面的密码登录");
      return;
    }
    ElMessage.error(error instanceof Error ? error.message : "通行密钥登录失败");
  }
}

function prepareRequest(options: PublicKeyCredentialRequestOptions): PublicKeyCredentialRequestOptions {
  const source = options as PublicKeyCredentialRequestOptions & { challenge: string };
  return {
    ...options,
    challenge: toBuffer(source.challenge),
    allowCredentials: options.allowCredentials?.map((item) => ({
      ...item,
      id: typeof item.id === "string" ? toBuffer(item.id) : item.id,
    })),
  };
}

function toBuffer(value: string | ArrayBuffer): ArrayBuffer {
  if (value instanceof ArrayBuffer) return value;
  const padded = value.replace(/-/g, "+").replace(/_/g, "/");
  const bin = atob(padded + "=".repeat((4 - (padded.length % 4)) % 4));
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i += 1) bytes[i] = bin.charCodeAt(i);
  return bytes.buffer;
}

function credentialToJson(credential: PublicKeyCredential) {
  const response = credential.response as AuthenticatorAssertionResponse;
  return {
    id: credential.id,
    rawId: bufferToBase64url(credential.rawId),
    type: credential.type,
    response: {
      clientDataJSON: bufferToBase64url(response.clientDataJSON),
      authenticatorData: bufferToBase64url(response.authenticatorData),
      signature: bufferToBase64url(response.signature),
      userHandle: response.userHandle ? bufferToBase64url(response.userHandle) : null,
    },
  };
}

function bufferToBase64url(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let bin = "";
  bytes.forEach((byte) => {
    bin += String.fromCharCode(byte);
  });
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function passkeyUnavailable() {
  screen.value = "fallback";
}

async function register() {
  try {
    await api("/api/register", {
      method: "POST",
      body: JSON.stringify({
        username: regUser.value,
        invite_code: regCode.value,
      }),
    });
    if (!window.PublicKeyCredential) {
      passkeyUnavailable();
      return;
    }
    screen.value = "passkey";
  } catch (error) {
    ElMessage.error(error instanceof Error ? error.message : "注册失败");
  }
}

async function bindRegisteredPasskey() {
  if (!window.PublicKeyCredential) {
    passkeyUnavailable();
    return;
  }
  binding.value = true;
  try {
    const started = await api<{ challenge_id: string; publicKey: Record<string, unknown> }>(
      "/api/passkeys/register/start",
      { method: "POST" },
    );
    const publicKey = prepareCreation(unwrapPublicKey(started));
    let credential: PublicKeyCredential | null;
    try {
      credential = (await navigator.credentials.create({ publicKey })) as PublicKeyCredential | null;
    } catch {
      passkeyUnavailable();
      return;
    }
    if (!credential) {
      passkeyUnavailable();
      return;
    }
    await api("/api/passkeys/register/finish", {
      method: "POST",
      body: JSON.stringify({
        challenge_id: started.challenge_id,
        credential: registrationToJson(credential),
        friendly_name: "本机",
      }),
    });
    ElMessage.success("通行密钥已绑定");
    await enter();
  } catch {
    passkeyUnavailable();
  } finally {
    binding.value = false;
  }
}

async function finishWithPassword() {
  if ([...fallbackPassword.value].length < 8) {
    ElMessage.error("密码至少 8 个字符");
    return;
  }
  if (fallbackPassword.value !== fallbackConfirm.value) {
    ElMessage.error("两次输入的密码不一致");
    return;
  }
  binding.value = true;
  try {
    await api("/api/me/password", {
      method: "POST",
      body: JSON.stringify({ new_password: fallbackPassword.value }),
    });
    fallbackPassword.value = "";
    fallbackConfirm.value = "";
    await enter();
  } catch (error) {
    ElMessage.error(error instanceof Error ? error.message : "密码设置失败");
  } finally {
    binding.value = false;
  }
}

async function requestApproval() {
  try {
    const created = await api<{
      id: string;
      poll_token: string;
      status: string;
      ip: string;
      user_agent: string;
      friendly_name: string;
    }>("/api/login-approvals", {
      method: "POST",
      body: JSON.stringify({ username: username.value }),
    });
    wait.value = {
      id: created.id,
      poll: created.poll_token,
      ip: created.ip,
      ua: created.user_agent,
      name: created.friendly_name,
      status: created.status,
    };
    poll();
  } catch (error) {
    ElMessage.error(error instanceof Error ? error.message : "无法发起确认");
  }
}

async function poll() {
  if (!wait.value) return;
  try {
    const body = await api<{ status: string }>(
      `/api/login-approvals/${wait.value.id}?poll_token=${encodeURIComponent(wait.value.poll)}`,
    );
    wait.value.status = body.status;
    if (body.status === "allowed") {
      const me = await api<Me>("/api/me");
      if (me.id) await enter();
      return;
    }
    if (body.status === "pending") setTimeout(poll, 2000);
  } catch (error) {
    ElMessage.error(error instanceof Error ? error.message : "确认失败");
  }
}
</script>

<template>
  <div class="login-wrap">
    <div class="card">
      <div class="brand">
        <span class="brand-icon" aria-hidden="true">
          <svg viewBox="0 0 24 24" width="28" height="28" fill="none">
            <path d="M12 3c-1.8 0-3.3 1-4.1 2.4A3.5 3.5 0 0 0 4 8.8C4 10.8 5.6 12.4 7.6 12.4h8.8c2 0 3.6-1.6 3.6-3.6a3.5 3.5 0 0 0-3.9-3.4A4.6 4.6 0 0 0 12 3Z" fill="#2d8a45"/>
            <rect x="7" y="12.2" width="10" height="7.3" rx="1.2" fill="#2d8a45"/>
            <rect x="6.2" y="18.8" width="11.6" height="1.8" rx="0.6" fill="#246e37"/>
          </svg>
        </span>
        <div>
          <div class="name">多邻灶</div>
          <div class="role">食堂 / 管理横屏工作台</div>
        </div>
      </div>

      <template v-if="screen === 'home'">
        <h1>登录</h1>
        <p class="sub">使用通行密钥，或输入账号继续</p>
        <el-button type="primary" size="large" class="full" @click="passkeyLogin()">使用通行密钥登录</el-button>
        <div class="divider">或</div>
        <el-input id="login-username" v-model="username" placeholder="账号" autocomplete="username" class="mb" @keyup.enter="goNext" />
        <div class="row-actions">
          <button type="button" class="link left" @click="screen = 'register'">有邀请码，去注册</button>
          <el-button type="primary" @click="goNext">下一步</el-button>
        </div>
      </template>

      <template v-else-if="screen === 'methods'">
        <h1>选择登录方式</h1>
        <div class="chip-wrap"><div class="chip"><span class="avatar">{{ avatar }}</span><span>{{ account }}</span></div></div>
        <div class="methods">
          <button type="button" class="method" @click="passkeyLogin(username)">
            <span><span class="t">通行密钥</span><span class="d">使用该账号绑定的通行密钥登录</span></span><span class="arrow">›</span>
          </button>
          <button type="button" class="method" @click="screen = 'password'">
            <span><span class="t">密码</span><span class="d">使用账号密码登录</span></span><span class="arrow">›</span>
          </button>
          <button type="button" class="method" @click="screen = 'code'">
            <span><span class="t">验证码</span><span class="d">Authenticator 应用中的 6 位动态码</span></span><span class="arrow">›</span>
          </button>
          <button v-if="showEmail" type="button" class="method" @click="screen = 'email'">
            <span><span class="t">邮箱登录</span><span class="d">向绑定邮箱发送验证码</span></span><span class="arrow">›</span>
          </button>
          <button type="button" class="method" @click="screen = 'wait'">
            <span><span class="t">新设备确认</span><span class="d">请已登录设备在页内允许</span></span><span class="arrow">›</span>
          </button>
        </div>
        <button type="button" class="link muted" @click="screen = 'home'">使用其他账号</button>
      </template>

      <template v-else-if="screen === 'password'">
        <h1>输入密码</h1>
        <div class="chip-wrap"><div class="chip"><span class="avatar">{{ avatar }}</span><span>{{ account }}</span></div></div>
        <el-input v-model="password" type="password" show-password placeholder="密码" autocomplete="current-password" class="mb" @keyup.enter="passwordLogin" />
        <el-button type="primary" class="full" @click="passwordLogin">登录</el-button>
        <button type="button" class="link muted" @click="screen = 'methods'">换一种方式</button>
      </template>

      <template v-else-if="screen === 'code'">
        <h1>输入验证码</h1>
        <div class="chip-wrap"><div class="chip"><span class="avatar">{{ avatar }}</span><span>{{ account }}</span></div></div>
        <el-input v-model="code" placeholder="6 位动态码" inputmode="numeric" autocomplete="one-time-code" maxlength="8" class="mb" @keyup.enter="codeLogin" />
        <p class="hint">打开 Authenticator 应用查看验证码。</p>
        <el-button type="primary" class="full" @click="codeLogin">验证并登录</el-button>
        <button type="button" class="link muted" @click="screen = 'methods'">换一种方式</button>
      </template>

      <template v-else-if="screen === 'email'">
        <h1>邮箱登录</h1>
        <div class="chip-wrap"><div class="chip"><span class="avatar">{{ avatar }}</span><span>{{ account }}</span></div></div>
        <el-button class="full outline" @click="requestEmailOtp">发送验证码</el-button>
        <el-input v-model="otpCode" placeholder="6 位验证码" inputmode="numeric" autocomplete="one-time-code" class="mb" @keyup.enter="verifyEmailOtp" />
        <el-button type="primary" class="full" @click="verifyEmailOtp">验证并登录</el-button>
        <button type="button" class="link muted" @click="screen = 'methods'">换一种方式</button>
      </template>

      <template v-else-if="screen === 'wait'">
        <h1>等待设备确认</h1>
        <div class="chip-wrap"><div class="chip"><span class="avatar">{{ avatar }}</span><span>{{ account }}</span></div></div>
        <p class="sub">已向你的已登录设备发起确认，请在对方页面允许或拒绝。</p>
        <el-button class="full outline" @click="requestApproval">重新发起确认</el-button>
        <p v-if="wait" class="hint">{{ wait.name }} · {{ wait.ip }} · {{ wait.status === "pending" ? "等待确认" : wait.status }}</p>
        <button type="button" class="link muted" @click="screen = 'methods'">换一种方式</button>
      </template>

      <template v-else-if="screen === 'passkey'">
        <h1>绑定通行密钥</h1>
        <p class="sub">要先绑定通行密钥才能进入。这台设备可以创建通行密钥时，不能改用密码跳过。</p>
        <el-button type="primary" class="full" :disabled="binding" @click="bindRegisteredPasskey">绑定通行密钥</el-button>
      </template>

      <template v-else-if="screen === 'fallback'">
        <h1>设置密码</h1>
        <p class="sub">通行密钥现在用不了（设备不支持、已取消或创建失败）。请设置至少 8 位密码后再进入。</p>
        <el-input v-model="fallbackPassword" type="password" show-password placeholder="密码（至少 8 位）" autocomplete="new-password" class="mb" />
        <el-input v-model="fallbackConfirm" type="password" show-password placeholder="再输入一次密码" autocomplete="new-password" class="mb" @keyup.enter="finishWithPassword" />
        <el-button type="primary" class="full" :disabled="binding" @click="finishWithPassword">设置密码并进入</el-button>
        <button v-if="canTryPasskey" type="button" class="link" :disabled="binding" @click="screen = 'passkey'">重试通行密钥</button>
      </template>

      <template v-else>
        <h1>注册</h1>
        <el-input v-model="regUser" placeholder="账号" autocomplete="username" class="mb" />
        <el-input v-model="regCode" placeholder="邀请码" class="mb" />
        <el-button type="primary" class="full" @click="register">注册并登录</el-button>
        <button type="button" class="link" @click="screen = 'home'">返回登录</button>
      </template>
    </div>
    <p class="foot">新设备将等待已登录设备页内确认 · 不扫码</p>
    <p v-if="appVersion" class="ver">Powered by 多邻灶 · Version: {{ appVersion }}</p>
  </div>
</template>

<style scoped>
.login-wrap {
  min-height: 100vh; display: flex; flex-direction: column; align-items: center; justify-content: center;
  background: #f3f5f7; padding: 24px;
}
.card {
  width: 100%; max-width: 400px; background: #fff; border-radius: 14px; padding: 24px 24px 18px;
  border: 1px solid #eef0f2; box-shadow: 0 1px 4px rgba(16,24,40,.06);
}
.brand { display: flex; align-items: center; gap: 10px; margin-bottom: 18px; }
.name { font-size: 20px; font-weight: 700; }
.role { font-size: 12px; color: #9ca3af; }
h1 { margin: 0 0 8px; font-size: 22px; text-align: center; font-weight: 700; }
.sub { margin: 0 0 16px; text-align: center; font-size: 13px; color: #6b7280; }
.full { width: 100%; margin: 0 0 10px; }
.outline { --el-button-border-color: #2d8a45; --el-button-text-color: #2d8a45; }
.divider { display: flex; align-items: center; gap: 12px; margin: 12px 0; color: #9ca3af; font-size: 12px; }
.divider::before, .divider::after { content: ""; flex: 1; height: 1px; background: #f0f1f3; }
.row-actions { display: flex; justify-content: space-between; align-items: center; gap: 10px; }
.chip-wrap { text-align: center; }
.chip {
  display: inline-flex; align-items: center; gap: 8px; margin: 0 auto 16px; padding: 5px 10px 5px 5px;
  border: 1px solid #eef0f2; border-radius: 999px; font-size: 13px; color: #374151; background: #fafafa;
}
.avatar {
  width: 26px; height: 26px; border-radius: 50%; background: #e8f5ec; color: #2d8a45;
  display: grid; place-items: center; font-weight: 700; font-size: 12px;
}
.methods { display: grid; gap: 8px; }
.method {
  width: 100%; text-align: left; padding: 12px;
  border: 1px solid #eef0f2; border-radius: 10px; background: #fff; cursor: pointer; font: inherit;
  display: flex; align-items: center; justify-content: space-between; gap: 12px;
}
.method:hover { border-color: #2d8a45; background: #f4faf6; }
.method .t { font-weight: 600; display: block; font-size: 14px; }
.method .d { font-size: 12px; color: #9ca3af; display: block; margin-top: 2px; }
.method .arrow { color: #9ca3af; }
.hint { margin: 4px 0 12px; font-size: 12px; color: #9ca3af; text-align: center; }
.foot { margin-top: 14px; font-size: 12px; color: #9ca3af; }
.ver { margin-top: 6px; font-size: 11px; color: #b6bcc6; }
.link { display: block; width: 100%; margin-top: 12px; border: none; background: transparent; color: #2d8a45; cursor: pointer; font-size: 14px; }
.link:disabled { opacity: 0.5; cursor: default; }
.link.muted { color: #6b7280; }
.link.left { width: auto; margin-top: 0; text-align: left; }
.mb { margin-bottom: 10px; }
</style>

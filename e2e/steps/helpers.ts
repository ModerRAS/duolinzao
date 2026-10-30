import { expect, type Page } from "@playwright/test";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";

// features/*.feature 的共享薄封装。
// 选择器与流程对照 web/src/pages/*.vue、LoginPage.vue；
// 邀请码与食堂由 scripts/e2e.sh 调 e2e/seed.sql + setup-roles.mjs 预置，不需要 mock。
export const password = "E2e-passw0rd!";
export const inviteCode = process.env.DLZ_E2E_INVITE_CODE ?? "DLZ-E2E-INVITE";
export const canteen = process.env.DLZ_E2E_CANTEEN ?? "一食堂";
export const canteen2 = process.env.DLZ_E2E_CANTEEN2 ?? "二食堂";
export const superUser = process.env.DLZ_SUPER_USER ?? "root";
export const superPass = process.env.DLZ_SUPER_PASS ?? "root-password-1";
export const canteenUser = process.env.DLZ_E2E_CANTEEN_USER ?? "e2e_canteen";
export const canteenPass = process.env.DLZ_E2E_CANTEEN_PASS ?? "E2e-canteen-pass1!";
export const adminUser = process.env.DLZ_E2E_ADMIN_USER ?? "e2e_admin";
export const adminPass = process.env.DLZ_E2E_ADMIN_PASS ?? "E2e-admin-pass1!";
export const baseURL = process.env.DLZ_BASE_URL ?? "http://localhost:8080";

let seq = 0;
let currentUser = "";

// 场景跑在各自进程里（模块状态不共享），本轮注册的账号写文件共享。
// DB 跨轮持久化：旧文件指向的账号若还能登录就复用。
const userFile = process.env.DLZ_E2E_USER_FILE ?? ".tmp/e2e-user";

function readSharedUser(): string {
  try {
    return readFileSync(userFile, "utf8").trim();
  } catch {
    return "";
  }
}

export async function openTab(page: Page, name: string) {
  await page.locator(".van-tabbar-item", { hasText: name }).click();
}

// 向总管 API 现开一张未使用邀请码；失败时退回 seed 的 DLZ_E2E_INVITE_CODE。
// 单次码：full suite / 重试 / 多次 ensureLoggedIn 都会再开一张，避免 used_at 卡住。
export async function mintInviteCode(): Promise<string> {
  try {
    const s = await rootSession();
    const r = await s.call("/api/admin/invites", { method: "POST" });
    if ((r.status === 201 || r.status === 200) && typeof r.json?.code === "string" && r.json.code) {
      return r.json.code as string;
    }
  } catch {
    // 总管登录或发码失败：用 seed 兜底。
  }
  return inviteCode;
}

// 注册一个全新账号并登录（用户名保证唯一，套件可重复跑）。
export async function registerNewUser(page: Page) {
  currentUser = `e2e_${Date.now()}_${seq++}`;
  // 先清会话：同 page 可能还登着 canteen/admin，不清会进不了注册页或注册被会话挡住。
  await page.context().clearCookies();
  const code = await mintInviteCode();
  await page.goto("/");
  await page.getByRole("button", { name: "有邀请码，去注册" }).click();
  await page.getByPlaceholder("账号").fill(currentUser);
  await page.getByPlaceholder("邀请码").fill(code);
  await page.getByPlaceholder("密码（至少 8 位）").fill(password);
  await page.getByRole("button", { name: "注册并登录" }).click();
  await expect(page.locator(".emp")).toBeVisible();
  mkdirSync(".tmp", { recursive: true });
  writeFileSync(userFile, currentUser);
}

// 保证已登录：优先用本轮已注册账号（文件共享，跨进程），
// 登录不上（单独跑某场景、旧账号不可用）则现场注册。套件里只有注册场景消费邀请码。
export async function ensureLoggedIn(page: Page) {
  // 先清会话：同一 page 可能还登着 canteen/admin（跨角色断言场景），不清就找不到登录入口。
  await page.context().clearCookies();
  const saved = currentUser || readSharedUser();
  if (saved) {
    currentUser = saved;
    await page.goto("/");
    await page.getByPlaceholder("账号").fill(currentUser);
    await page.getByRole("button", { name: "下一步" }).click();
    await page.getByRole("button", { name: /使用账号密码登录/ }).click();
    await page.getByPlaceholder("密码").fill(password);
    await page.getByRole("button", { name: "登录", exact: true }).click();
    try {
      await expect(page.locator(".emp")).toBeVisible({ timeout: 8000 });
      return;
    } catch {
      // 登录不上，继续往下现场注册。
    }
  }
  await registerNewUser(page);
}

// 以指定账号密码登录（横屏/竖屏共用同一登录页）。先清会话，避免串号。
export async function loginAs(page: Page, username: string, pwd: string) {
  await page.context().clearCookies();
  await page.goto("/");
  await page.getByPlaceholder("账号").fill(username);
  await page.getByRole("button", { name: "下一步" }).click();
  await page.getByRole("button", { name: /使用账号密码登录/ }).click();
  await page.getByPlaceholder("密码").fill(pwd);
  await page.getByRole("button", { name: "登录", exact: true }).click();
}

export async function ensureSuper(page: Page) {
  await loginAs(page, superUser, superPass);
  await expect(page.locator(".side")).toBeVisible({ timeout: 10000 });
}

export async function ensureCanteen(page: Page) {
  await loginAs(page, canteenUser, canteenPass);
  await expect(page.locator(".side")).toBeVisible({ timeout: 10000 });
}

export async function ensureAdmin(page: Page) {
  await loginAs(page, adminUser, adminPass);
  await expect(page.locator(".side")).toBeVisible({ timeout: 10000 });
}

// 跨 feature 共享的背景状态：canteen/admin 的「该账号已登录横屏工作台」文本相同，
// 由前置的范围步骤决定角色，单个定义分支登录，避免 bddgen 重复定义。
export const bgRole: { name: string } = { name: "" };

// 跨 feature 共享的拒绝断言：「修改被拒绝」在 admin/employee 文本相同，
// 由各自的 When 写入状态，单个定义断言，避免重复定义。
export const lastDenial: { status: number } = { status: 0 };

// 定长唯一名：毫秒时间戳 + 两位进程内计数。同长字符串之间子串即相等，
// 表格行的 hasText 定位不再撞到其它行；用户名重复注册也不会冲突。
let tagSeq = 0;
export function uniqueTag(prefix: string): string {
  tagSeq = (tagSeq + 1) % 100;
  return `${prefix}${Date.now()}${tagSeq.toString().padStart(2, "0")}`;
}

// 点击横屏左侧栏（App.vue .side .nav-btn）。
export async function openSide(page: Page, label: string) {
  await page.locator(".side").getByRole("button", { name: label }).click();
}

// 改单页行内翻转：点「改为…」后轮询接口直到翻转，最多点 3 次（表格重渲染可能吞掉一次点击）。
export async function flipOverrideRow(
  page: Page,
  username: string,
  date: string,
  slot: string,
  canteenId: string,
  accountId: string,
): Promise<void> {
  const row = page.locator(".el-table__row", { hasText: username }).first();
  await expect(row).toBeVisible({ timeout: 15000 });
  await row.scrollIntoViewIfNeeded();
  const btn = row.getByRole("button", { name: /改为/ });
  const label = (await btn.textContent()) ?? "";
  const wantOrdering = !label.includes("不报");
  const check = async () => {
    const s = await rootSession();
    const list = await s.call(`/api/admin/meals?date=${date}&canteen_id=${canteenId}`);
    const found = (list.json.items ?? []).find(
      (x: any) => x.account_id === accountId && x.slot === slot,
    );
    expect(found?.ordering).toBe(wantOrdering);
  };
  for (let i = 0; i < 3; i += 1) {
    // 日历浮层偶发盖住行内按钮：先 Escape，真点击超短时就对同一按钮派发点击（同一 handler）。
    await page.keyboard.press("Escape");
    try {
      await btn.click({ timeout: 8000 });
    } catch {
      await btn.dispatchEvent("click");
    }
    try {
      await expect(check).toPass({ timeout: 5000 });
      return;
    } catch {
      // 没翻转就再点一次。
    }
  }
  await expect(check).toPass({ timeout: 5000 });
}

// 改单页的日期选择器：先关残留下拉层，再打字 + 回车提交，确认值已落定。
export async function pickAdminDate(page: Page, date: string) {
  await page.keyboard.press("Escape");
  // 注意：el-date-picker 渲染根 class 是 el-date-editor，不是 el-date-picker。
  const input = page.locator(".el-date-editor input").first();
  await expect(input).toBeVisible({ timeout: 15000 });
  // 不经过聚焦（聚焦会弹开日历浮层盖住表格）：直接设值并派发 input/change，走同一 v-model 链路。
  await input.evaluate(
    (el, v) => {
      const target = el as HTMLInputElement;
      target.value = v;
      target.dispatchEvent(new Event("input", { bubbles: true }));
      target.dispatchEvent(new Event("change", { bubbles: true }));
    },
    date,
  );
  await expect(input).toHaveValue(date, { timeout: 10000 });
}

// 保证已绑定至少一个食堂（已绑定则跳过，避免重复点导致反选解绑）。
export async function ensureBound(page: Page) {
  await ensureLoggedIn(page);
  await openTab(page, "报餐");
  await page.getByRole("button", { name: "手动改报餐" }).click();
  try {
    await expect(page.getByRole("button", { name: canteen, exact: true }).first()).toBeVisible({
      timeout: 4000,
    });
    return;
  } catch {
    // 还没绑定，走绑定流程。
  }
  await openTab(page, "我的");
  await page.getByText("绑定食堂", { exact: true }).click();
  await page.getByText(canteen, { exact: true }).click();
  await page.getByRole("button", { name: "保存", exact: true }).click();
  await expect(page.getByText("已绑定")).toBeVisible();
}

// 选择上二休四并保存（重复保存同一预设是幂等的）。
export async function chooseShift(page: Page) {
  await ensureBound(page);
  await openTab(page, "报餐");
  await page.getByRole("button", { name: "选择班制" }).click();
  const panel = page.locator("section.panel", { hasText: "上二休四" });
  await panel.locator('input[type="date"]').fill("2025-01-06");
  await panel.getByRole("button", { name: "保存", exact: true }).click();
  await expect(page.getByText("已设为上二休四")).toBeVisible();
}

// ---- 上海时区日期工具（与 web/src/api.ts 同口径） ----
export function shanghaiToday(): string {
  return new Intl.DateTimeFormat("en-CA", {
    timeZone: "Asia/Shanghai",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).format(new Date());
}

export function addDaysStr(iso: string, days: number): string {
  const [y, m, d] = iso.split("-").map(Number);
  const dt = new Date(Date.UTC(y, m - 1, d));
  dt.setUTCDate(dt.getUTCDate() + days);
  return dt.toISOString().slice(0, 10);
}

// ---- 不经过浏览器会话的直调 API（setup/断言用，不污染 page 的 cookie） ----
export interface Session {
  cookie: string;
  call: (path: string, init?: { method?: string; body?: unknown }) => Promise<{ status: number; json: any }>;
}

export async function loginSession(username: string, pwd: string): Promise<Session> {
  const res = await fetch(`${baseURL}/api/session/password`, {
    method: "POST",
    headers: { "content-type": "application/json", "x-dlz-requested": "1" },
    body: JSON.stringify({ username, password: pwd }),
  });
  const setCookie = res.headers.get("set-cookie") ?? "";
  const token = (setCookie.match(/dlz_session=([^;]+)/) ?? [])[1];
  if (res.status !== 200 || !token) {
    throw new Error(`login failed ${username}: ${res.status}`);
  }
  const cookie = `dlz_session=${token}`;
  return {
    cookie,
    call: async (path, init = {}) => {
      const r = await fetch(`${baseURL}${path}`, {
        method: init.method ?? "GET",
        headers: {
          "x-dlz-requested": "1",
          ...(init.body !== undefined ? { "content-type": "application/json" } : {}),
          cookie,
        },
        body: init.body === undefined ? undefined : JSON.stringify(init.body),
      });
      const text = await r.text();
      let json: any = null;
      try {
        json = text ? JSON.parse(text) : null;
      } catch {
        json = { _raw: text };
      }
      return { status: r.status, json };
    },
  };
}

export async function rootSession(): Promise<Session> {
  return loginSession(superUser, superPass);
}

export async function canteenIdByName(s: Session, name: string): Promise<string> {
  const r = await s.call("/api/canteens");
  const item = (r.json?.items ?? []).find((c: any) => c.name === name);
  if (!item) throw new Error(`no canteen ${name}`);
  return item.id;
}

export async function accountIdByName(s: Session, username: string): Promise<string> {
  const r = await s.call("/api/admin/accounts");
  const item = (r.json?.items ?? []).find((a: any) => a.username === username);
  if (!item) throw new Error(`no account ${username}`);
  return item.id;
}

// 确保某食堂每类至少一道启用菜（幂等，按名称复用）。返回按类分组的 id。
export async function ensureDishes(
  s: Session,
  canteenId: string,
  tag = "e2e",
): Promise<Record<string, string[]>> {
  const listed = await s.call(`/api/canteens/${canteenId}/dishes`);
  const groups = listed.json?.groups ?? { 菜: [], 饭: [], 汤: [] };
  const out: Record<string, string[]> = { 菜: [], 饭: [], 汤: [] };
  for (const kind of ["菜", "饭", "汤"] as const) {
    const live = (groups[kind] ?? []).filter((d: any) => d.enabled);
    if (live.length > 0) {
      out[kind] = live.map((d: any) => d.id);
      continue;
    }
    const name = `${tag}${kind}${Date.now() % 100000}`;
    const created = await s.call(`/api/canteens/${canteenId}/dishes`, {
      method: "POST",
      body: { name, kind, taste_tags: ["酸"], enabled: true },
    });
    if (created.status !== 201 && created.status !== 200) {
      throw new Error(`create dish ${kind} failed: ${created.status} ${JSON.stringify(created.json)}`);
    }
    out[kind] = [created.json.id ?? created.json.dish?.id];
  }
  return out;
}

// 当餐选定保存（幂等覆盖）。
export async function ensureSelection(
  s: Session,
  canteenId: string,
  date: string,
  slot: string,
  dishIds: string[],
) {
  const r = await s.call(`/api/canteens/${canteenId}/selection/${date}/${slot}`, {
    method: "PUT",
    body: { dish_ids: dishIds },
  });
  if (r.status !== 200) throw new Error(`selection failed: ${r.status} ${JSON.stringify(r.json)}`);
}

// 管理员/总管改单（可写锁定餐）。
export async function adminOverride(
  s: Session,
  accountId: string,
  date: string,
  slot: string,
  ordering: boolean,
  canteenId: string,
) {
  const r = await s.call(`/api/admin/meals/${accountId}/${date}/${slot}`, {
    method: "PUT",
    body: { ordering, canteen_id: canteenId },
  });
  if (r.status !== 200) throw new Error(`override failed: ${r.status} ${JSON.stringify(r.json)}`);
  return r.json;
}

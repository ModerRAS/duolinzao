// @scale 旅程步骤：按花名册（e2e/scale/roster.<smoke|full>.json）走完全程 Playwright UI。
// 约定：业务操作（建食堂/建菜/排菜/发码/注册/绑定/报餐/评分/看汇总）一律点 UI；
// API 只用于断言与 setup（发邀请 underlying 仍走 UI）。时间拨钟走仅测试环境入口
// POST /api/test/clock（总管 super_admin + DLZ_ALLOW_TEST_CLOCK=1），这是时间缝。
import { execSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { expect, type Page } from "@playwright/test";
import { createBdd, test } from "playwright-bdd";
import {
  accountIdByName,
  addDaysStr,
  baseURL,
  ensureSuper,
  flipOverrideRow,
  lastDenial,
  loginAs,
  loginSession,
  openSide,
  openTab,
  pickAdminDate,
  rootSession,
} from "./helpers";

const { Given, When, Then } = createBdd(test);

const SCALE = process.env.DLZ_SCALE === "full" ? "full" : "smoke";
const SLOT_EN: Record<string, string> = { 早: "breakfast", 中: "lunch", 晚: "dinner" };
const SLOT_CN: Record<string, string> = { breakfast: "早餐", lunch: "午餐", dinner: "晚餐" };
const KIND_TABS = ["菜", "饭", "汤"] as const;

interface Roster {
  meta: { scale: string; seed: number; run_tag: string; target_date: string };
  slices: Record<string, { now: string; desc: string }>;
  canteens: { key: string; name: string; canteen_user: string; canteen_pass: string; dishes: Record<string, string[]> }[];
  admins: { user: string; pass: string; canteen_keys: string[] }[];
  employees: { user: string; pass: string; bind_canteen_keys: string[]; shift: string; meals: any[]; ratings: any[] }[];
  expect: { headcount: Record<string, Record<string, Record<string, number>>>; rating_visible: Record<string, string[]> };
}

let cached: Roster | null = null;
function roster(): Roster {
  if (!cached) {
    const path = `scale/roster.${SCALE}.json`;
    if (!existsSync(path)) {
      // 开跑时生成：e2e.sh 已做；手动单跑时兜底现生成。
      execSync("node scale/roster.mjs", { env: { ...process.env, DLZ_SCALE: SCALE } });
    }
    cached = JSON.parse(readFileSync(path, "utf8"));
    expect(cached!.meta.scale).toBe(SCALE);
  }
  loadState();
  return cached!;
}

const keyToId: Record<string, string> = {};
const keyToName: Record<string, string> = {};
// T0 之后落盘（key→id/name 映射 + 发放的邀请码），供重试/单跑后续场景时恢复。
// run_tag 随每次生成变化，run_tag 对不上就不用旧状态。
const STATE_PATH = ".tmp/scale-state.json";
function saveState() {
  mkdirSync(".tmp", { recursive: true });
  writeFileSync(STATE_PATH, JSON.stringify({ run_tag: roster().meta.run_tag, keyToId, keyToName }));
}
let stateLoaded = false;
function loadState() {
  if (stateLoaded) return;
  stateLoaded = true;
  try {
    if (!existsSync(STATE_PATH)) return;
    const saved = JSON.parse(readFileSync(STATE_PATH, "utf8"));
    if (saved.run_tag !== roster().meta.run_tag) return;
    Object.assign(keyToId, saved.keyToId);
    Object.assign(keyToName, saved.keyToName);
  } catch {
    // 无旧状态就走正常 T0 流程。
  }
}
let inviteCodes: string[] = [];
const denial: Record<string, { status: number; error?: string }> = {};

// 花名册员工 UI 登录（密码兜底）。
async function loginEmp(page: Page, user: string, pass: string) {
  await page.context().clearCookies();
  await page.goto("/");
  await page.getByRole("button", { name: /密码登录（兜底）/ }).click();
  await page.getByPlaceholder("账号").fill(user);
  await page.getByPlaceholder("密码").fill(pass);
  await page.getByRole("button", { name: "登录", exact: true }).click();
  await expect(page.locator(".emp")).toBeVisible({ timeout: 15000 });
}

// 选食堂 pill（员工本周报餐 / 管理侧 canteen-row 二选一由调用方定）。
async function pickMealCanteen(page: Page, name: string) {
  await page.getByRole("button", { name, exact: true }).first().click();
}

// 翻到目标周：点 下一周 直到出现 MM-DD 面板（目标日在未来，至多点 5 次）。
async function gotoWeekOf(page: Page, date: string) {
  const md = date.slice(5);
  for (let i = 0; i < 6; i += 1) {
    if ((await page.locator("article.panel", { hasText: md }).count()) > 0) return;
    await page.getByRole("button", { name: "下一周" }).click();
    await page.waitForTimeout(800);
  }
  throw new Error(`找不到 ${date} 所在周`);
}

// 把某日某餐设为 报/不报（点 UI pill，断言 class 落定）。
async function setMealUI(page: Page, date: string, slotCn: string, wantYes: boolean) {
  const md = date.slice(5);
  const panel = page.locator("article.panel", { hasText: md }).first();
  await expect(panel).toBeVisible({ timeout: 15000 });
  const pill = panel.getByRole("button", { name: slotCn, exact: true });
  await expect(pill).toBeEnabled({ timeout: 15000 });
  // 三态：unset→点1次=报→再点=不报。点到目标态为止（至多 3 次）。
  const cls = wantYes ? /yes/ : /no/;
  for (let i = 0; i < 4; i += 1) {
    try {
      await expect(pill).toHaveClass(cls, { timeout: 3000 });
      return;
    } catch {
      await pill.click();
    }
  }
  await expect(pill).toHaveClass(cls, { timeout: 5000 });
}

// ---- 背景 ----
Given("测试时钟可用", async () => {
  const s = await rootSession();
  const r = await s.call("/api/test/clock");
  expect(r.status, JSON.stringify(r.json)).toBe(200);
});

Given("已按 DLZ_SCALE 生成花名册", async () => {
  const r = roster();
  expect(r.canteens.length).toBeGreaterThan(0);
  expect(r.employees.length).toBeGreaterThan(0);
  expect(r.slices.T0?.now).toBeTruthy();
});

// ---- T0 ----
When("总管按花名册创建全部食堂", async ({ page }) => {
  await ensureSuper(page);
  const s = await rootSession();
  for (const c of roster().canteens) {
    // 幂等：retry/重跑时同名已存在直接复用（食堂名无唯一约束，不可盲建，否则下拉框重名）。
    let id: string | null = null;
    try {
      id = await canteenIdOf(s, c.name);
    } catch {
      id = null;
    }
    if (id === null) {
      await page.getByPlaceholder("新食堂名称").fill(c.name);
      await page.getByRole("button", { name: "新建食堂" }).click();
      // 成功信号用输入框被清空：成功分支 await api 落库后才清空；toast 会残留导致误判
      //（CI 曾 2.4s 误过陈旧 toast，查询跑赢提交报“无食堂”）。
      await expect(page.getByPlaceholder("新食堂名称")).toHaveValue("", { timeout: 15000 });
      id = await canteenIdOf(s, c.name);
    }
    keyToId[c.key] = id;
    keyToName[c.key] = c.name;
  }
});

async function canteenIdOf(s: Awaited<ReturnType<typeof rootSession>>, name: string) {
  const r = await s.call("/api/canteens");
  const item = (r.json?.items ?? []).find((c: any) => c.name === name);
  if (!item) throw new Error(`无食堂 ${name}`);
  return item.id as string;
}

async function ensureAccount(user: string, pass: string): Promise<string> {
  const s = await rootSession();
  try {
    return await accountIdByName(s, user);
  } catch {
    // 无则走邀请码注册（账号准备，非业务报餐）。
    const inv = await s.call("/api/admin/invites", { method: "POST" });
    expect(inv.status, `mint invite: ${inv.status} ${JSON.stringify(inv.json)}`).toBe(201);
    const reg = await fetch(`${baseURL}/api/register`, {
      method: "POST",
      // 写操作 CSRF 闸（master #5 起）：无头 403 csrf_required；与 helpers.Session.call / 前端 api() 对齐。
      headers: { "content-type": "application/json", "x-dlz-requested": "1" },
      body: JSON.stringify({ username: user, invite_code: inv.json.code, password: pass }),
    });
    const regBody = await reg.text();
    expect(reg.status, `register ${user}: ${reg.status} ${regBody}`).toBe(201);
    return accountIdByName(s, user);
  }
}

When("总管为每个食堂挂上食堂账号角色与范围", async () => {
  const s = await rootSession();
  for (const c of roster().canteens) {
    const account_id = await ensureAccount(c.canteen_user, c.canteen_pass);
    const r = await s.call("/api/admin/role-assignments", {
      method: "POST",
      body: { account_id, role: "canteen", canteen_ids: [keyToId[c.key]] },
    });
    expect(r.status).toBe(200);
  }
});

When("总管按花名册挂上管理员及食堂范围", async () => {
  const s = await rootSession();
  for (const a of roster().admins) {
    const account_id = await ensureAccount(a.user, a.pass);
    const r = await s.call("/api/admin/role-assignments", {
      method: "POST",
      body: { account_id, role: "admin", canteen_ids: a.canteen_keys.map((k) => keyToId[k]) },
    });
    expect(r.status).toBe(200);
  }
});

async function dishExists(
  s: Awaited<ReturnType<typeof rootSession>>,
  canteenId: string,
  name: string,
) {
  const r = await s.call(`/api/canteens/${canteenId}/dishes`);
  const groups = r.json?.groups ?? {};
  return KIND_TABS.some((k) => (groups[k] ?? []).some((d: any) => d.name === name));
}

When("各食堂账号登录并创建本食堂菜饭汤", async ({ page }) => {
  const s = await rootSession();
  for (const c of roster().canteens) {
    await loginAs(page, c.canteen_user, c.canteen_pass);
    await expect(page.locator(".side")).toBeVisible({ timeout: 15000 });
    await page.locator(".canteen-row .el-select").click();
    await page.getByRole("option", { name: c.name }).click();
    await openSide(page, "菜谱");
    const canteenId = keyToId[c.key] ?? (await canteenIdOf(s, c.name));
    for (const kind of KIND_TABS) {
      for (const dish of c.dishes[kind] ?? []) {
        // 幂等：retry 时同名已存在直接跳过，否则一餐选出 8 张卡打爆“已选 4 道”。
        if (await dishExists(s, canteenId, dish)) continue;
        await page.getByRole("button", { name: "新建菜品" }).click();
        await expect(page.getByText("菜谱 / 新建菜")).toBeVisible();
        await page.getByPlaceholder("请输入菜品名称").fill(dish);
        await page.locator(`.form-card label:has(input[value="${kind}"])`).click();
        await page.getByRole("button", { name: "保存" }).click();
        // 落定信号用列表出现该菜名（toast 会残留误判，同食堂创建一节）。
        // 注意先点对应页签：列表默认停留在“菜”，饭/汤行被过滤不可见。
        await openSide(page, "菜谱");
        await page.getByRole("tab", { name: kind }).click();
        await expect(page.locator(".el-table__row", { hasText: dish }).first()).toBeVisible({
          timeout: 15000,
        });
      }
    }
  }
});

When("各食堂为花名册目标日三餐完成选菜勾选", async ({ page }) => {
  const target = roster().meta.target_date;
  for (const c of roster().canteens) {
    await loginAs(page, c.canteen_user, c.canteen_pass);
    await expect(page.locator(".side")).toBeVisible({ timeout: 15000 });
    await page.locator(".canteen-row .el-select").click();
    await page.getByRole("option", { name: c.name }).click();
    await openSide(page, "选菜优化");
    await expect(page.getByText("周一到周日 · 点某一餐进入选定")).toBeVisible();
    // 翻到目标周（时钟在 T0，目标日在下周）。
    for (let i = 0; i < 4; i += 1) {
      if ((await page.locator(".day-col", { hasText: target.slice(5) }).count()) > 0) break;
      await page.getByRole("button", { name: "下一周" }).click();
      await page.waitForTimeout(800);
    }
    const col = page.locator(".day-col", { hasText: target.slice(5) }).first();
    await expect(col).toBeVisible({ timeout: 15000 });
    const cells = col.locator("button.slot-cell");
    expect(await cells.count()).toBe(3);
    const slotEns = ["breakfast", "lunch", "dinner"];
    for (let i = 0; i < 3; i += 1) {
      // 先挂好网络等待再点格子：选定视图 loadPick 是异步的，必须等目标餐
      // (date, slot) 的数据落定后再点卡。否则撞上旧餐残留——卡片数与“已选
      // N 道”都是上一个餐的，循环直接跳过、断言照过，当前餐实际没选
      // （CI 曾静默漏掉整格，T4 才暴露）。
      const loaded = page.waitForResponse(
        (r) => r.url().includes(`/selection?date=${target}&slot=${slotEns[i]}`) && r.ok(),
        { timeout: 15000 },
      );
      await cells.nth(i).click();
      await loaded;
      await expect(page.getByText("勾选即保存")).toBeVisible();
      // 等候选卡片加载（本食堂 4 道：2 菜 1 饭 1 汤），再全选。
      await expect(page.locator(".dish-card")).toHaveCount(4, { timeout: 15000 });
      const cards = page.locator(".dish-card:not(.selected)");
      // 全选本食堂 4 道（2 菜 1 饭 1 汤）。
      for (let k = 0; k < 4; k += 1) {
        if ((await cards.count()) === 0) break;
        await cards.first().click();
        await page.waitForTimeout(600);
      }
      await expect(page.getByText("已选 4 道")).toBeVisible({ timeout: 10000 });
      await page.getByRole("button", { name: "返回周视图" }).click();
      await expect(page.getByText("周一到周日 · 点某一餐进入选定")).toBeVisible();
    }
  }
});

When("管理员或总管发放足够邀请码", async ({ page }) => {
  const n = roster().employees.length;
  const a = roster().admins[0];
  inviteCodes = [];
  await loginAs(page, a.user, a.pass);
  await expect(page.locator(".side")).toBeVisible({ timeout: 15000 });
  await openSide(page, "邀请码");
  for (let i = 0; i < n; i += 1) {
    await page.getByRole("button", { name: "发放邀请码" }).click();
    const code = page.locator(".code");
    await expect(code).toBeVisible({ timeout: 10000 });
    inviteCodes.push(((await code.textContent()) ?? "").trim());
    await page.getByRole("button", { name: "关闭", exact: true }).click();
  }
  expect(inviteCodes.every((c) => c.length > 0)).toBe(true);
});

When("花名册中每位员工完成邀请码注册", async ({ page }) => {
  const list = roster().employees;
  for (let i = 0; i < list.length; i += 1) {
    const e = list[i];
    // 幂等：retry 时已注册直接跳过（撞账号唯一约束）。
    // 注意：必须用只读查询判存在，绝不能用登录试探——不存在用户的失败登录会记
    // login_attempt，同 IP 15 分钟超 20 次就 429，后续一切登录雪崩（CI 实锤）。
    let exists = false;
    try {
      const s = await rootSession();
      await accountIdByName(s, e.user);
      exists = true;
    } catch {
      exists = false;
    }
    if (exists) continue;
    await page.context().clearCookies();
    await page.goto("/");
    await page.getByRole("button", { name: "有邀请码，去注册" }).click();
    await page.getByPlaceholder("账号").fill(e.user);
    await page.getByPlaceholder("邀请码").fill(inviteCodes[i]);
    await page.getByPlaceholder("密码（至少 8 位）").fill(e.pass);
    await page.getByRole("button", { name: "注册并登录" }).click();
    await expect(page.locator(".emp")).toBeVisible({ timeout: 15000 });
  }
});

When("每位员工按花名册绑定食堂并选择班制", async ({ page }) => {
  const anchor = addDaysStr(roster().meta.target_date, -14);
  for (const e of roster().employees) {
    await loginEmp(page, e.user, e.pass);
    // 绑定：只点尚未绑定的（重复跑幂等）。
    const bound: string[] = await page.request
      .get("/api/me/canteens")
      .then((r) => r.json())
      .then((b) => (b.items ?? []).map((x: any) => x.id));
    const want = e.bind_canteen_keys.map((k) => keyToId[k]);
    await openTab(page, "我的");
    await page.getByText("绑定食堂", { exact: true }).click();
    const missing = want.filter((id) => !bound.includes(id));
    for (const id of missing) {
      const name = roster().canteens.find((c) => keyToId[c.key] === id)?.name ?? "";
      await page.locator(".van-cell", { hasText: name }).first().click();
    }
    if (missing.length) {
      await page.getByRole("button", { name: "保存", exact: true }).click();
      // 落定信号走接口轮询（toast 会残留误判，同食堂创建一节）。
      await expect(async () => {
        const ids: string[] = await page.request
          .get("/api/me/canteens")
          .then((r) => r.json())
          .then((b) => (b.items ?? []).map((x: any) => x.id));
        expect(want.every((id) => ids.includes(id))).toBe(true);
      }).toPass({ timeout: 15000 });
    }
    // 班制：上二休四。
    await openTab(page, "报餐");
    await page.getByRole("button", { name: "选择班制" }).click();
    const panel = page.locator("section.panel", { hasText: "上二休四" });
    await panel.locator('input[type="date"]').fill(anchor);
    await panel.getByRole("button", { name: "保存", exact: true }).click();
    await expect(page.getByText("已设为上二休四").first()).toBeVisible({ timeout: 10000 });
  }
  saveState();
});

Then("花名册中每个食堂在界面可访问", async ({ page }) => {
  for (const c of roster().canteens) {
    await loginAs(page, c.canteen_user, c.canteen_pass);
    await expect(page.locator(".side")).toBeVisible({ timeout: 15000 });
    await page.locator(".canteen-row .el-select").click();
    await expect(page.getByRole("option", { name: c.name })).toBeVisible();
    await page.keyboard.press("Escape");
  }
});

Then("每位员工仅能在其绑定食堂范围内报餐", async ({ page }) => {
  for (const e of roster().employees) {
    await loginEmp(page, e.user, e.pass);
    await openTab(page, "报餐");
    await page.getByRole("button", { name: "手动改报餐" }).click();
    const names = e.bind_canteen_keys.map((k) => keyToName[k]);
    for (const name of names) {
      await expect(page.getByRole("button", { name, exact: true }).first()).toBeVisible({ timeout: 10000 });
    }
    // 范围外食堂不出现。
    const outside = roster().canteens.map((c) => c.name).filter((n) => !names.includes(n));
    for (const name of outside) {
      await expect(page.getByRole("button", { name, exact: true })).toHaveCount(0);
    }
  }
});

// ---- 时钟 ----
When("测试时钟拨到切片 {word}", async ({}, slice: string) => {
  const s = await rootSession();
  const r = await s.call("/api/test/clock", {
    method: "POST",
    body: { now: roster().slices[slice].now },
  });
  expect(r.status, JSON.stringify(r.json)).toBe(200);
});

// ---- T1/T2 报餐 ----
// NOTE: loginEmp 需密码；rows 循环里用 empOf 取 pass。
async function doSliceMealsFixed(page: Page, slice: "T1" | "T2") {
  const target = roster().meta.target_date;
  for (const e of roster().employees) {
    const rows = e.meals.filter((m: any) => m.at_slice === slice);
    if (!rows.length) continue;
    await loginEmp(page, e.user, e.pass);
    await openTab(page, "报餐");
    await page.getByRole("button", { name: "手动改报餐" }).click();
    await gotoWeekOf(page, target);
    for (const m of rows) {
      if (m.intent === "报") await pickMealCanteen(page, keyToName[m.canteen_key]);
      else await pickMealCanteen(page, keyToName[e.bind_canteen_keys[0]]);
      await setMealUI(page, m.date, m.slot, m.intent === "报");
    }
  }
}

When("员工按花名册在 at_slice 为 {word} 的餐次执行报或不报", async ({ page }, slice: string) => {
  expect(["T1", "T2"]).toContain(slice);
  await doSliceMealsFixed(page, slice as "T1" | "T2");
});

async function expectSliceMeals(slice: "T1" | "T2") {
  const s = await rootSession();
  const target = roster().meta.target_date;
  for (const c of roster().canteens) {
    const r = await s.call(`/api/admin/meals?date=${target}&canteen_id=${keyToId[c.key]}`);
    expect(r.status).toBe(200);
    for (const e of roster().employees) {
      for (const m of e.meals.filter((x: any) => x.at_slice === slice && x.canteen_key === c.key)) {
        const row = (r.json.items ?? []).find(
          (x: any) => x.username === e.user && x.slot === SLOT_EN[m.slot],
        );
        expect(row, `${e.user} ${m.slot}`).toBeTruthy();
        expect(row.ordering, `${e.user} ${m.slot}`).toBe(m.intent === "报");
      }
    }
  }
}

Then("提前很久的报餐全部成功", async () => {
  await expectSliceMeals("T1");
});

Then("锁定前夕修改仍然成功", async () => {
  await expectSliceMeals("T2");
});

Then("同一未锁餐次的后报覆盖先报", async ({ page }) => {
  const target = roster().meta.target_date;
  const e = roster().employees.find((x) => x.meals.some((m: any) => m.intent === "报"))!;
  const m = e.meals.find((x: any) => x.intent === "报")!;
  await loginEmp(page, e.user, e.pass);
  await openTab(page, "报餐");
  await page.getByRole("button", { name: "手动改报餐" }).click();
  await gotoWeekOf(page, target);
  await pickMealCanteen(page, keyToName[m.canteen_key]);
  // 翻转再翻回：后报覆盖先报，最终态与花名册一致。
  await setMealUI(page, m.date, m.slot, false);
  await setMealUI(page, m.date, m.slot, true);
});

Then("各食堂人数总览与花名册 expect.headcount 一致", async ({ page }) => {
  const target = roster().meta.target_date;
  const want = roster().expect.headcount;
  for (const c of roster().canteens) {
    await loginAs(page, c.canteen_user, c.canteen_pass);
    await expect(page.locator(".side")).toBeVisible({ timeout: 15000 });
    await page.locator(".canteen-row .el-select").click();
    await page.getByRole("option", { name: c.name }).click();
    await openSide(page, "人数总览");
    await pickAdminDate(page, target);
    await page.waitForTimeout(1000);
    const nums = page.locator(".stat .num");
    expect(await nums.count()).toBe(3);
    const got = [await nums.nth(0).textContent(), await nums.nth(1).textContent(), await nums.nth(2).textContent()];
    const slots = ["早", "中", "晚"];
    for (let i = 0; i < 3; i += 1) {
      const expected = want[c.key]?.[target]?.[slots[i]] ?? 0;
      expect(Number((got[i] ?? "").trim()), `${c.key} ${slots[i]}`).toBe(expected);
    }
  }
});

Then("管理员仅能看到其管辖食堂的人数", async ({ page }) => {
  const a = roster().admins[0];
  const names = a.canteen_keys.map((k) => keyToName[k]);
  const outside = roster().canteens.map((c) => c.name).filter((n) => !names.includes(n));
  await loginAs(page, a.user, a.pass);
  await expect(page.locator(".side")).toBeVisible({ timeout: 15000 });
  await page.locator(".canteen-row .el-select").click();
  for (const n of names) await expect(page.getByRole("option", { name: n })).toBeVisible();
  for (const n of outside) await expect(page.getByRole("option", { name: n })).toHaveCount(0);
  await page.keyboard.press("Escape");
  // 接口侧：范围外 403。
  const admin = await loginSession(a.user, a.pass);
  const s = await rootSession();
  for (const c of roster().canteens.filter((x) => !a.canteen_keys.includes(x.key))) {
    const r = await admin.call(
      `/api/canteens/${await canteenIdOf(s, c.name)}/headcount?date=${roster().meta.target_date}`,
    );
    expect(r.status).toBe(403);
  }
});

// ---- T3 ----
When("员工尝试修改已锁定餐次", async ({ page }) => {
  const target = roster().meta.target_date;
  const e = roster().employees[0];
  const m = e.meals[0];
  await loginEmp(page, e.user, e.pass);
  await openTab(page, "报餐");
  await page.getByRole("button", { name: "手动改报餐" }).click();
  await gotoWeekOf(page, target);
  // UI 已锁：按钮变「X 已锁定」且 disabled。
  const md = target.slice(5);
  const panel = page.locator("article.panel", { hasText: md }).first();
  await expect(panel.getByRole("button", { name: `${m.slot} 已锁定` })).toBeVisible({ timeout: 15000 });
  // 接口尝试：403 meal_locked。
  const emp = await loginSession(e.user, e.pass);
  const s = await rootSession();
  const r = await emp.call(`/api/meals/${target}/${SLOT_EN[m.slot]}`, {
    method: "PUT",
    body: { ordering: m.intent !== "报", canteen_id: await canteenIdOf(s, keyToName[m.canteen_key]) },
  });
  denial.employee = { status: r.status, error: r.json?.error };
});

Then("员工修改被拒绝", async () => {
  expect(denial.employee?.status).toBe(403);
  expect(denial.employee?.error).toBe("meal_locked");
});

When("管理员在范围内修改已锁定报餐", async ({ page }) => {
  const target = roster().meta.target_date;
  const a = roster().admins[0];
  const ck = a.canteen_keys[0];
  const e = roster().employees.find((x) => x.meals.some((m: any) => m.canteen_key === ck))!;
  const m = e.meals.find((x: any) => x.canteen_key === ck)!;
  const s = await rootSession();
  const before = await s.call(`/api/admin/meals?date=${target}&canteen_id=${keyToId[ck]}`);
  const row = (before.json.items ?? []).find(
    (x: any) => x.username === e.user && x.slot === SLOT_EN[m.slot],
  );
  expect(row).toBeTruthy();
  // 幂等：retry 时若已是翻转态则跳过 UI 翻转（否则来回翻，Then 口径漂移）。
  // 翻转目标态 = 与花名册 intent 相反。
  const flipped = !(m.intent === "报");
  if (row.ordering !== flipped) {
    await loginAs(page, a.user, a.pass);
    await expect(page.locator(".side")).toBeVisible({ timeout: 15000 });
    await page.locator(".canteen-row .el-select").click();
    await page.getByRole("option", { name: keyToName[ck] }).click();
    await openSide(page, "改单");
    await expect(page.getByText("截止后仅管理员可改")).toBeVisible({ timeout: 15000 });
    await pickAdminDate(page, target);
    await flipOverrideRow(
      page,
      e.user,
      target,
      SLOT_EN[m.slot],
      keyToId[ck],
      await accountIdByName(s, e.user),
    );
  }
});

Then("范围内改单成功", async () => {
  const target = roster().meta.target_date;
  const a = roster().admins[0];
  const ck = a.canteen_keys[0];
  const e = roster().employees.find((x) => x.meals.some((m: any) => m.canteen_key === ck))!;
  const m = e.meals.find((x: any) => x.canteen_key === ck)!;
  const s = await rootSession();
  const r = await s.call(`/api/admin/meals?date=${target}&canteen_id=${keyToId[ck]}`);
  const row = (r.json.items ?? []).find(
    (x: any) => x.username === e.user && x.slot === SLOT_EN[m.slot],
  );
  // 翻转过一次：与花名册 intent 相反。
  expect(row?.ordering).toBe(!(m.intent === "报"));
  // 还原，避免污染 T4/人数口径。
  const admin = await loginSession(a.user, a.pass);
  const back = await admin.call(`/api/admin/meals/${row.account_id}/${target}/${SLOT_EN[m.slot]}`, {
    method: "PUT",
    body: { ordering: m.intent === "报", canteen_id: keyToId[ck] },
  });
  expect(back.status).toBe(200);
});

When("管理员尝试修改范围外食堂的已锁定报餐", async () => {
  const target = roster().meta.target_date;
  const a = roster().admins[0];
  const out = roster().canteens.find((c) => !a.canteen_keys.includes(c.key))!;
  const e = roster().employees.find((x) => x.meals.some((m: any) => m.canteen_key === out.key))!;
  const m = e.meals.find((x: any) => x.canteen_key === out.key)!;
  const s = await rootSession();
  const admin = await loginSession(a.user, a.pass);
  const r = await admin.call(
    `/api/admin/meals/${await accountIdByName(s, e.user)}/${target}/${SLOT_EN[m.slot]}`,
    { method: "PUT", body: { ordering: !(m.intent === "报"), canteen_id: keyToId[out.key] } },
  );
  expect(r.status).toBe(403);
  lastDenial.status = r.status;
});

When("食堂账号尝试截止后修改他人报餐", async () => {
  const target = roster().meta.target_date;
  const c = roster().canteens[0];
  const e = roster().employees.find((x) => x.meals.some((m: any) => m.canteen_key === c.key))!;
  const m = e.meals.find((x: any) => x.canteen_key === c.key)!;
  const s = await rootSession();
  const staff = await loginSession(c.canteen_user, c.canteen_pass);
  const r = await staff.call(
    `/api/admin/meals/${await accountIdByName(s, e.user)}/${target}/${SLOT_EN[m.slot]}`,
    { method: "PUT", body: { ordering: !(m.intent === "报"), canteen_id: keyToId[c.key] } },
  );
  expect(r.status).toBe(403);
  lastDenial.status = r.status;
});

// ---- T4 ----
async function submitRatingUI(page: Page, user: string, pass: string, r: any) {
  await loginEmp(page, user, pass);
  await openTab(page, "评分");
  await page.locator('.filters input[type="date"]').fill(r.date);
  await page.getByRole("button", { name: SLOT_CN[SLOT_EN[r.slot]], exact: true }).click();
  let commented = false;
  for (const kind of KIND_TABS) {
    await page.locator(".van-tab", { hasText: kind }).click();
    console.log(`scale T4: ${user} ${r.slot} ${kind} tab`);
    for (;;) {
      const dishes = page.locator("article.panel.dish:visible");
      let acted = false;
      const n = await dishes.count();
      for (let i = 0; i < n; i += 1) {
        const dish = dishes.nth(i);
        if (((await dish.textContent()) ?? "").includes("已评分")) continue;
        const value = String(r.scores[kind]);
        await dish.getByRole("button", { name: value, exact: true }).click();
        if (r.comment && !commented) {
          await dish.getByPlaceholder("可选评语，随这条评分一起提交").fill(r.comment);
          commented = true;
        }
        if (r.anonymous) await dish.getByText("匿名，管理端不展示账号").click();
        await dish.getByRole("button", { name: "提交" }).click();
        await expect(dish.getByText("已评分，不能再改")).toBeVisible({ timeout: 10000 });
        acted = true;
        break;
      }
      if (!acted) break;
    }
  }
  // 完整性终态校验：该餐选定菜应全已评（当次评出或之前已评均可；选定缺失则失败）。
  // retry 安全：重跑时无动作也不报错。
  const tg = await page.request
    .get(`/api/me/score-targets?date=${r.date}&slot=${SLOT_EN[r.slot]}`)
    .then((x) => x.json());
  const dishes = [...tg.groups.菜, ...tg.groups.饭, ...tg.groups.汤];
  expect(dishes.length, `${user} ${r.date} ${r.slot} 选定为空`).toBeGreaterThan(0);
  expect(
    dishes.every((d: any) => d.scored),
    `${user} ${r.date} ${r.slot} 有未评`,
  ).toBe(true);
}

When("已报餐员工按花名册 ratings 在 T4 提交评分与可选评语", async ({ page }) => {
  const rows = roster().employees.flatMap((e) => e.ratings.filter((x: any) => x.at_slice === "T4").map((r: any) => ({ e, r })));
  console.log(`scale T4: ${rows.length} 条待评`);
  let done = 0;
  for (const e of roster().employees) {
    for (const r of e.ratings.filter((x: any) => x.at_slice === "T4")) {
      await submitRatingUI(page, e.user, e.pass, r);
      done += 1;
      if (done % 5 === 0) console.log(`scale T4: ${done}/${rows.length}`);
    }
  }
});

Then("花名册 T4 评分落库成功", async () => {
  // 每条 rating 行：该餐 4 道选定菜应全有该员工的评分。
  const s = await rootSession();
  for (const e of roster().employees) {
    for (const r of e.ratings.filter((x: any) => x.at_slice === "T4")) {
      const sel = await s.call(
        `/api/canteens/${keyToId[r.canteen_key]}/selection?date=${r.date}&slot=${SLOT_EN[r.slot]}`,
      );
      const ids: string[] = Object.values(sel.json.groups ?? {}).flat().map((d: any) => d.dish_id ?? d.id);
      expect(ids.length).toBeGreaterThan(0);
      // 按人查：该员工对这几道菜都有分（走该员工口径的 targets）。
      const emp = await loginSession(e.user, e.pass);
      const targets = await emp.call(`/api/me/score-targets?date=${r.date}&slot=${SLOT_EN[r.slot]}`);
      const scored = [...targets.json.groups.菜, ...targets.json.groups.饭, ...targets.json.groups.汤].filter(
        (d: any) => d.scored,
      ).length;
      expect(scored, `${e.user} ${r.slot}`).toBe(ids.length);
    }
  }
});

Then("未报餐员工不能对应该餐评分", async ({ page }) => {
  const target = roster().meta.target_date;
  const e = roster().employees.find((x) => x.meals.some((m: any) => m.intent === "不报"))!;
  const m = e.meals.find((x: any) => x.intent === "不报")!;
  await loginEmp(page, e.user, e.pass);
  await openTab(page, "评分");
  await page.locator('.filters input[type="date"]').fill(target);
  await page.getByRole("button", { name: SLOT_CN[SLOT_EN[m.slot]], exact: true }).click();
  await expect(page.getByText("这一类没有可评的餐品").first()).toBeVisible({ timeout: 10000 });
  // 接口侧：400 score_not_ordered。
  const s = await rootSession();
  const sel = await s.call(
    `/api/canteens/${keyToId[m.canteen_key]}/selection?date=${target}&slot=${SLOT_EN[m.slot]}`,
  );
  const dishId = (Object.values(sel.json.groups ?? {}).flat() as any[])[0]?.dish_id;
  expect(dishId).toBeTruthy();
  const emp = await loginSession(e.user, e.pass);
  const r = await emp.call("/api/scores", {
    method: "POST",
    body: { meal_date: target, slot: SLOT_EN[m.slot], dish_id: dishId, value: 1, anonymous: true },
  });
  expect(r.status).toBe(400);
  expect(r.json.error).toBe("score_not_ordered");
});

Then("各食堂评分汇总与匿名评语仅含本范围数据", async ({ page }) => {
  for (const c of roster().canteens) {
    const own: string[] = [...c.dishes.菜, ...c.dishes.饭, ...c.dishes.汤];
    const other: string[] = roster().canteens.filter((x) => x.key !== c.key).flatMap((x) => [...x.dishes.菜, ...x.dishes.饭, ...x.dishes.汤]);
    await loginAs(page, c.canteen_user, c.canteen_pass);
    await expect(page.locator(".side")).toBeVisible({ timeout: 15000 });
    await page.locator(".canteen-row .el-select").click();
    await page.getByRole("option", { name: c.name }).click();
    await openSide(page, "评分");
    for (const kind of KIND_TABS) {
      await page.getByRole("tab", { name: kind }).click();
      await page.waitForTimeout(500);
    }
    const body = (await page.locator(".main").textContent()) ?? "";
    for (const name of other) expect(body).not.toContain(name);
    await openSide(page, "评语");
    for (const kind of KIND_TABS) {
      await page.getByRole("tab", { name: kind }).click();
      await page.waitForTimeout(500);
    }
    const comments = (await page.locator(".main").textContent()) ?? "";
    for (const name of other) expect(comments).not.toContain(name);
    const staff = await loginSession(c.canteen_user, c.canteen_pass);
    const s = await rootSession();
    for (const o of roster().canteens.filter((x) => x.key !== c.key)) {
      const r1 = await staff.call(`/api/admin/score-summaries?canteen_id=${await canteenIdOf(s, o.name)}`);
      expect(r1.status).toBe(403);
      const r2 = await staff.call(`/api/admin/comments?canteen_id=${await canteenIdOf(s, o.name)}`);
      expect(r2.status).toBe(403);
    }
  }
});

Then("管理员评分与评语只读其分配范围", async ({ page }) => {
  const a = roster().admins[0];
  const admin = await loginSession(a.user, a.pass);
  const s = await rootSession();
  for (const ck of a.canteen_keys) {
    const r = await admin.call(`/api/admin/score-summaries?canteen_id=${keyToId[ck]}`);
    expect(r.status).toBe(200);
  }
  const out = roster().canteens.filter((c) => !a.canteen_keys.includes(c.key));
  expect(out.length).toBeGreaterThan(0);
  for (const c of out) {
    const r = await admin.call(`/api/admin/score-summaries?canteen_id=${keyToId[c.key]}`);
    expect(r.status).toBe(403);
  }
  // UI：管辖内汇总可见。
  await loginAs(page, a.user, a.pass);
  await expect(page.locator(".side")).toBeVisible({ timeout: 15000 });
  await page.locator(".canteen-row .el-select").click();
  await page.getByRole("option", { name: keyToName[a.canteen_keys[0]] }).click();
  await openSide(page, "评分");
  await expect(page.getByText("可见均分").first()).toBeVisible({ timeout: 10000 });
});

// ---- T5 ----
When("员工尝试对已过窗餐次再次评分", async () => {
  // 找一餐已报但 T4 未评的（生成器留约 30% 未评，供超窗断言，避免撞 score_exists）。
  const target = roster().meta.target_date;
  let found: { e: any; m: any } | null = null;
  for (const e of roster().employees) {
    for (const m of e.meals.filter((x: any) => x.intent === "报")) {
      const rated = e.ratings.some((r: any) => r.date === m.date && r.slot === m.slot);
      if (!rated) {
        found = { e, m };
        break;
      }
    }
    if (found) break;
  }
  expect(found, "花名册应至少留一餐已报未评").toBeTruthy();
  const { e, m } = found!;
  const s = await rootSession();
  const sel = await s.call(
    `/api/canteens/${keyToId[m.canteen_key]}/selection?date=${target}&slot=${SLOT_EN[m.slot]}`,
  );
  const dishId = (Object.values(sel.json.groups ?? {}).flat() as any[])[0]?.dish_id;
  expect(dishId).toBeTruthy();
  const emp = await loginSession(e.user, e.pass);
  const r = await emp.call("/api/scores", {
    method: "POST",
    body: { meal_date: target, slot: SLOT_EN[m.slot], dish_id: dishId, value: 1, anonymous: true },
  });
  denial.t5 = { status: r.status, error: r.json?.error };
});

Then("评分被拒绝", async () => {
  expect(denial.t5?.status).toBe(400);
  expect(denial.t5?.error).toBe("score_window_closed");
});

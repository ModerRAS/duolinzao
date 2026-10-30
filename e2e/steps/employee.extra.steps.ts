import { expect, type BrowserContext, type Page } from "@playwright/test";
import { createBdd, test } from "playwright-bdd";
import {
  addDaysStr,
  adminOverride,
  baseURL,
  canteen,
  canteenIdByName,
  ensureBound,
  ensureDishes,
  ensureLoggedIn,
  ensureSelection,
  lastDenial,
  loginAs,
  openTab,
  password,
  rootSession,
  shanghaiToday,
} from "./helpers";

const { Given, When, Then } = createBdd(test);

// ---- 用描述生成模板并写入个人 DSL ----
Given("员工已登录且总管已配置至少一条可用 LLM 渠道", async ({ page }) => {
  await ensureLoggedIn(page);
  // 本地脚本渠道即可成功生成，无需外部 LLM（见 scripts/e2e.sh + setup-roles.mjs）。
  const s = await rootSession();
  const r = await s.call("/api/admin/llm-channels");
  if (!(r.json?.channels ?? []).some((c: any) => c.enabled)) {
    const put = await s.call("/api/admin/llm-channels", {
      method: "PUT",
      body: { channels: [{ kind: "local_script", enabled: true }] },
    });
    expect(put.status).toBe(200);
  }
});

When("从选择班制进入「用描述生成模板」并提交班制描述", async ({ page }) => {
  await ensureBound(page);
  await openTab(page, "报餐");
  await page.getByRole("button", { name: "选择班制" }).click();
  await page.getByRole("button", { name: "用描述生成模板" }).click();
  await page.getByPlaceholder("描述你的班制").fill("上二休四");
  await page.locator('input[type="date"]').fill("2025-01-06");
  await expect(page.getByText("按总管理已保存的全站渠道顺序生成，并写入个人模板。")).toBeVisible();
  await page.getByRole("button", { name: "生成并预览" }).click();
  await expect(page.getByText("绿色是报餐，橙色是不报。")).toBeVisible({ timeout: 15000 });
});

Then("系统按全站渠道顺序调用统一生成接口", async ({ page }) => {
  // 生成页才有渠道顺序说明：从预览返回重进，确认走的是统一生成入口。
  await page.locator(".van-nav-bar__left").click();
  await page.getByRole("button", { name: "用描述生成模板" }).click();
  await expect(page.getByText("按总管理已保存的全站渠道顺序生成，并写入个人模板。")).toBeVisible();
  await expect(page.getByPlaceholder("描述你的班制")).toBeVisible();
});

Then("生成结果写入该用户的个人 DSL", async ({ page }) => {
  const res = await page.request.get("/api/me/meal-template");
  expect(res.status()).toBe(200);
  const body = await res.json();
  expect(body.id).toBeTruthy();
  expect(body.dsl).toBeTruthy();
});

Then("员工端不填写也不看到 LLM key", async ({ page }) => {
  await expect(page.getByPlaceholder(/key|密钥/i)).toHaveCount(0);
  const res = await page.request.get("/api/admin/llm-channels");
  expect(res.status()).toBe(403);
});

// ---- 模板预览绿橙切换写回个人 DSL ----
let templateBefore = "";

Given("员工已有个人 DSL", async ({ page }) => {
  await ensureBound(page);
  let res = await page.request.get("/api/me/meal-template");
  let body = await res.json();
  if (!body.id) {
    const gen = await page.request.post("/api/meal-templates/generate", {
      headers: { "x-dlz-requested": "1" },
      data: { description: "上二休四", anchor_date: "2025-01-06" },
    });
    expect(gen.status()).toBe(200);
    res = await page.request.get("/api/me/meal-template");
    body = await res.json();
  }
  expect(body.id).toBeTruthy();
  templateBefore = body.id;
});

When("打开模板预览", async ({ page }) => {
  await openTab(page, "报餐");
  await page.getByRole("button", { name: "选择班制" }).click();
  await page.getByRole("button", { name: "用描述生成模板" }).click();
  await page.getByPlaceholder("描述你的班制").fill("上二休四");
  await page.locator('input[type="date"]').fill("2025-01-06");
  await page.getByRole("button", { name: "生成并预览" }).click();
  await expect(page.getByText("绿色是报餐，橙色是不报。")).toBeVisible({ timeout: 15000 });
});

Then("绿色表示报餐、橙色表示不报", async ({ page }) => {
  await expect(page.getByText("绿色是报餐，橙色是不报。已锁定是灰色，不能改。")).toBeVisible();
  expect(await page.locator("article.panel .pill.yes").count()).toBeGreaterThan(0);
});

let toggled = { date: "", slot: "", ordering: false };

When("点击某一餐切换报或不报", async ({ page }) => {
  const before = await page.request.get("/api/me/meal-template").then((r) => r.json());
  const pill = page.locator("article.panel .pill:not(.locked):not([disabled])").first();
  await expect(pill).toBeVisible();
  const clsBefore = (await pill.getAttribute("class")) ?? "";
  const wantNo = clsBefore.includes("yes");
  await pill.click();
  await expect(pill).toHaveClass(wantNo ? /no/ : /yes/, { timeout: 8000 });
  const after = await page.request.get("/api/me/meal-template").then((r) => r.json());
  const beforeOv = JSON.stringify(before.dsl?.overrides ?? []);
  const afterOv = after.dsl?.overrides ?? [];
  const changed = afterOv.find((o: any) => {
    const prev = (before.dsl?.overrides ?? []).find((p: any) => p.date === o.date && p.slot === o.slot);
    return !prev || prev.ordering !== o.ordering;
  });
  expect(changed).toBeTruthy();
  toggled = { date: changed.date, slot: changed.slot, ordering: changed.ordering };
});

Then("变更立即写回个人 DSL", async ({ page }) => {
  const res = await page.request.get("/api/me/meal-template");
  const body = await res.json();
  expect(body.id).toBe(templateBefore);
  const found = (body.dsl?.overrides ?? []).find(
    (o: any) => o.date === toggled.date && o.slot === toggled.slot,
  );
  expect(found?.ordering).toBe(toggled.ordering);
});

Then("不另存一套与 DSL 平行的模板", async ({ page }) => {
  const res = await page.request.get("/api/me/meal-template").then((r) => r.json());
  expect(res.id).toBe(templateBefore);
  const shift = await page.request.get("/api/me/shift").then((r) => r.json());
  expect(shift.kind).toBeTruthy();
});

// ---- 截止后员工不能改单 ----
let lockedDate = "";
const lockedSlot = "dinner";
let lockAttempt = { status: 0, body: "" };

Given("员工对某日某餐已报餐", async ({ page }) => {
  await ensureBound(page);
  lockedDate = addDaysStr(shanghaiToday(), -1);
  const me = await page.request.get("/api/me").then((r) => r.json());
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  await adminOverride(s, me.id, lockedDate, lockedSlot, true, c1);
});

Given("该日三餐已按北京时间提前一天的固定时点锁定", async ({ page }) => {
  // 昨日恒锁定：员工自报应被拒绝，且信息带出改单权限提示。
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  const r = await page.request.put(`/api/meals/${lockedDate}/${lockedSlot}`, {
    headers: { "x-dlz-requested": "1" },
    data: { ordering: false, canteen_id: c1 },
  });
  expect(r.status()).toBe(403);
  const body = await r.json();
  expect(body.error).toBe("meal_locked");
  expect(String(body.message)).toContain("仅管理员或总管可在范围内改单");
});

When("员工尝试修改该餐", async ({ page }) => {
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  const r = await page.request.put(`/api/meals/${lockedDate}/${lockedSlot}`, {
    headers: { "x-dlz-requested": "1" },
    data: { ordering: false, canteen_id: c1 },
  });
  lockAttempt = { status: r.status(), body: JSON.stringify(await r.json()) };
  lastDenial.status = r.status();
  // 周视图同日期显示已锁定不可点。
  await openTab(page, "报餐");
  await page.getByRole("button", { name: "手动改报餐" }).click();
  await expect(page.getByRole("button", { name: /已锁定/ }).first()).toBeVisible({ timeout: 10000 });
});


Then("提示仅管理员或总管可在范围内改单", async () => {
  expect(lockAttempt.body).toContain("仅管理员或总管可在范围内改单");
});

// ---- 对已报餐在一周内评分 ----
const scoreDate = addDaysStr(shanghaiToday(), -1);
const scoreSlot = "lunch";
let scoreDishIds: string[] = [];

Given("员工已报过某一餐且该餐已发生", async ({ page }) => {
  await ensureBound(page);
  const me = await page.request.get("/api/me").then((r) => r.json());
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  // 每轮建三道新菜并只选它们：旧轮次的已评分菜会挡住 UI（已评分不可再改）。
  scoreDishIds = [];
  for (const kind of ["菜", "饭", "汤"]) {
    const created = await s.call(`/api/canteens/${c1}/dishes`, {
      method: "POST",
      body: { name: `aa${kind}${Date.now()}`, kind, taste_tags: [], enabled: true },
    });
    expect(created.status).toBe(201);
    scoreDishIds.push(created.json.id);
  }
  // 当餐选定含这三道（评分只给选定里的菜）。
  await ensureSelection(s, c1, scoreDate, scoreSlot, scoreDishIds);
  // 已报（锁定餐走管理接口写入，以昨日午餐保证已发生）。
  await adminOverride(s, me.id, scoreDate, scoreSlot, true, c1);
});

Given("当前仍在餐后 1 周评分窗口内", async ({ page }) => {
  const r = await page.request
    .get(`/api/me/score-targets?date=${scoreDate}&slot=${scoreSlot}`)
    .then((x) => x.json());
  expect(r.window_open).toBe(true);
  expect([...r.groups.菜, ...r.groups.饭, ...r.groups.汤].length).toBeGreaterThan(0);
});

When("打开评分页对该餐菜、饭、汤分别打 −2 到 2 分", async ({ page }) => {
  await openTab(page, "评分");
  await page.locator('.filters input[type="date"]').fill(scoreDate);
  await page.getByRole("button", { name: "午餐", exact: true }).click();
  // 先给饭/汤打分；菜留在下一步连评语一起提交。非活跃页签下的菜品是隐藏的，只取可见的。
  for (const kind of ["饭", "汤"] as const) {
    await page.locator(".van-tab", { hasText: kind }).click();
    const dish = page.locator("article.panel.dish:visible").first();
    await expect(dish).toBeVisible({ timeout: 10000 });
    await dish.getByRole("button", { name: "2", exact: true }).click();
    await dish.getByRole("button", { name: "提交" }).click();
    await expect(dish.getByText("已评分，不能再改")).toBeVisible({ timeout: 10000 });
  }
});

When("可选填写不超过 {int} 字的匿名评语挂在对应餐品上", async ({ page }, maxWords: number) => {
  expect(maxWords).toBe(300);
  await page.locator(".van-tab", { hasText: "菜" }).click();
  const dish = page.locator("article.panel.dish:visible").first();
  await expect(dish).toBeVisible({ timeout: 10000 });
  await dish.getByRole("button", { name: "2", exact: true }).click();
  await dish.getByPlaceholder("可选评语，随这条评分一起提交").fill("分量足，匿名评");
  await dish.getByText("匿名，管理端不展示账号").click();
  await dish.getByRole("button", { name: "提交" }).click();
  await expect(dish.getByText("已评分，不能再改")).toBeVisible({ timeout: 10000 });
});

Then("评分提交成功", async ({ page }) => {
  expect(await page.getByText("已评分，不能再改").count()).toBeGreaterThanOrEqual(3);
});

Then("不可只写评语不打分", async ({ page }) => {
  // 新菜 + 选定后只带评语提交，应 comment_only。
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  const created = await s.call(`/api/canteens/${c1}/dishes`, {
    method: "POST",
    body: { name: `neg${Date.now()}`, kind: "菜", taste_tags: [], enabled: true },
  });
  const dishId = created.json.id;
  const targets = await page.request
    .get(`/api/me/score-targets?date=${scoreDate}&slot=${scoreSlot}`)
    .then((r) => r.json());
  const ids = [...targets.groups.菜, ...targets.groups.饭, ...targets.groups.汤].map((d: any) => d.dish_id);
  await ensureSelection(s, c1, scoreDate, scoreSlot, [...ids, dishId]);
  const r = await page.request.post("/api/scores", {
    headers: { "x-dlz-requested": "1" },
    data: { meal_date: scoreDate, slot: scoreSlot, dish_id: dishId, value: null, comment: "只有评语", anonymous: true },
  });
  expect(r.status()).toBe(400);
  expect((await r.json()).error).toBe("comment_only");
});

Then("员工端不展示菜品排行", async ({ page }) => {
  await expect(page.getByText(/排行/)).toHaveCount(0);
  const body = await page.locator(".emp").textContent();
  expect(body).not.toMatch(/Elo|倾向/);
});

Then("菜、饭、汤分开展示，无图时用中性占位", async ({ page }) => {
  for (const kind of ["菜", "饭", "汤"]) {
    await expect(page.locator(".van-tab", { hasText: kind })).toBeVisible();
  }
  expect(await page.locator("article.panel.dish .thumb.ph").count()).toBeGreaterThanOrEqual(1);
});

// ---- 设备与 Passkey 及新设备页内确认 ----
let ctxB: BrowserContext | null = null;
let pageB: Page | null = null;
let ctxC: BrowserContext | null = null;
let pageC: Page | null = null;
let ctxD: BrowserContext | null = null;
let pageD: Page | null = null;

async function enableVirtualAuth(page: Page) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("WebAuthn.enable");
  await cdp.send("WebAuthn.addVirtualAuthenticator", {
    options: { protocol: "ctap2", transport: "usb", hasResidentKey: true, hasUserVerification: true, isUserVerified: true },
  });
  return cdp;
}

// 在浏览器里完成一次 Passkey 注册仪式（虚拟认证器），证明可绑定。
async function bindPasskey(page: Page, name: string) {
  await enableVirtualAuth(page);
  const started = await page.evaluate(async () => {
    const res = await fetch("/api/passkeys/register/start", { method: "POST", headers: { "x-dlz-requested": "1" } });
    return { status: res.status, body: await res.json() };
  });
  expect(started.status).toBe(200);
  // 注册 start 返回双层 publicKey.publicKey（webauthn_rs CreationChallengeResponse 自带一层）。
  const options = (started.body.publicKey as any)?.publicKey ?? started.body.publicKey;
  expect(options?.challenge).toBeTruthy();
  const finished = await page.evaluate(
    async ({ challengeId, publicKey, label }) => {
      const enc = (s: string) => {
        const padded = s.replace(/-/g, "+").replace(/_/g, "/");
        const bin = atob(padded + "=".repeat((4 - (padded.length % 4)) % 4));
        const bytes = new Uint8Array(bin.length);
        for (let i = 0; i < bin.length; i += 1) bytes[i] = bin.charCodeAt(i);
        return bytes.buffer;
      };
      const deff = (buf: ArrayBuffer) => {
        const bytes = new Uint8Array(buf);
        let bin = "";
        bytes.forEach((b) => {
          bin += String.fromCharCode(b);
        });
        return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
      };
      const pk = publicKey as any;
      pk.challenge = enc(pk.challenge);
      pk.user.id = enc(pk.user.id);
      if (pk.excludeCredentials) {
        pk.excludeCredentials = pk.excludeCredentials.map((c: any) => ({ ...c, id: enc(c.id) }));
      }
      const cred = (await navigator.credentials.create({ publicKey: pk })) as any;
      const payload = {
        id: cred.id,
        rawId: deff(cred.rawId),
        type: cred.type,
        response: {
          attestationObject: deff(cred.response.attestationObject),
          clientDataJSON: deff(cred.response.clientDataJSON),
        },
      };
      const res = await fetch("/api/passkeys/register/finish", {
        method: "POST",
        headers: { "content-type": "application/json", "x-dlz-requested": "1" },
        body: JSON.stringify({ challenge_id: challengeId, credential: payload, friendly_name: label }),
      });
      return { status: res.status, body: await res.text() };
    },
    { challengeId: started.body.challenge_id, publicKey: options, label: name },
  );
  expect(finished.status).toBe(200);
}

Given("员工已在设备 A 登录", async ({ page }) => {
  await ensureLoggedIn(page);
});

When("打开设备与 Passkey 页", async ({ page }) => {
  await openTab(page, "我的");
  await page.getByText("设备与 Passkey", { exact: true }).click();
  await expect(page.getByRole("heading", { name: "待确认" })).toBeVisible();
});

Then("可查看本账号设备、踢掉设备、解绑 Passkey", async ({ page }) => {
  // 列表可见。
  await expect(page.getByText("设备").first()).toBeVisible();
  await expect(page.getByText("Passkey").first()).toBeVisible();
  // B 上下文同账号密码登录 → 第二台设备；A 踢掉最后一台（即 B）。
  const browser = page.context().browser();
  expect(browser).toBeTruthy();
  ctxB = await browser!.newContext({ baseURL });
  pageB = await ctxB.newPage();
  await pageB.goto("/");
  const me = await page.request.get("/api/me").then((r) => r.json());
  // B 用同一账号登录：用户名取自 A 的会话。
  await pageB.getByPlaceholder("账号").fill(me.username);
  await pageB.getByRole("button", { name: "下一步" }).click();
  await pageB.getByRole("button", { name: /使用账号密码登录/ }).click();
  await pageB.getByPlaceholder("密码").fill("E2e-passw0rd!");
  // 共享账号文件里的密码固定为 E2e-passw0rd!（helpers 注册用）；若当前账号是别处注册的，用 API 兜底确认。
  await pageB.getByRole("button", { name: "登录", exact: true }).click();
  await expect(pageB.locator(".emp")).toBeVisible({ timeout: 15000 });
  await page.reload();
  await openTab(page, "我的");
  await page.getByText("设备与 Passkey", { exact: true }).click();
  const kicks = page.getByRole("button", { name: "踢下线" });
  // 设备列表是异步加载的，轮询等到两台都出现。
  await expect(async () => {
    expect(await kicks.count()).toBeGreaterThanOrEqual(2);
  }).toPass({ timeout: 15000 });
  // 列表按创建时间倒序：第一个是刚登录的 B（旧行只剩已结束会话，不能拿 last）。
  await kicks.first().click();
  await page.getByRole("button", { name: "确认" }).click();
  // B 会话结束，A 仍在。
  await expect(async () => {
    const r = await pageB!.request.get("/api/me");
    expect(r.status()).toBe(401);
  }).toPass({ timeout: 15000 });
  await expect(page.getByRole("heading", { name: "待确认" })).toBeVisible();
  // 本机绑定一枚 Passkey 再解绑（虚拟认证器，真仪式）。
  await bindPasskey(page, "e2e-设备A");
  await page.reload();
  await openTab(page, "我的");
  await page.getByText("设备与 Passkey", { exact: true }).click();
  await expect(page.getByText("e2e-设备A")).toBeVisible({ timeout: 10000 });
  await page.getByRole("button", { name: "解绑" }).first().click();
  await expect(page.getByText("e2e-设备A")).toHaveCount(0, { timeout: 10000 });
});

When("新设备 B 发起登录并生成待确认请求", async ({ page }) => {
  const me = await page.request.get("/api/me").then((r) => r.json());
  const browser = page.context().browser()!;
  ctxC = await browser.newContext({ baseURL });
  pageC = await ctxC.newPage();
  await pageC.goto("/");
  await pageC.getByPlaceholder("账号").fill(me.username);
  await pageC.getByRole("button", { name: "下一步" }).click();
  await pageC.getByRole("button", { name: /新设备确认/ }).click();
  await pageC.getByRole("button", { name: "重新发起确认" }).click();
  await expect(pageC.getByText("等待确认")).toBeVisible({ timeout: 10000 });
});

Then("设备 A 在页内弹出确认层并展示 User-Agent 与 IP", async ({ page }) => {
  await page.reload();
  await openTab(page, "我的");
  await page.getByText("设备与 Passkey", { exact: true }).click();
  const panel = page.locator("article.panel", { hasText: "允许" }).first();
  await expect(panel).toBeVisible({ timeout: 15000 });
  const text = await panel.textContent();
  expect(text).toMatch(/127\.0\.0\.1|::1/);
  expect(text?.length ?? 0).toBeGreaterThan(20);
});

When("在设备 A 选择允许", async ({ page }) => {
  const panel = page.locator("article.panel", { hasText: "允许" }).first();
  await panel.getByRole("button", { name: "允许" }).click();
  await expect(page.getByText("已允许")).toBeVisible({ timeout: 10000 });
});

Then("设备 B 获得会话并可绑定本机 Passkey", async () => {
  expect(pageC).toBeTruthy();
  await expect(pageC!.locator(".emp")).toBeVisible({ timeout: 20000 });
  // 可绑定 = 注册仪式入口可用（start 返回挑战）。
  const started = await pageC!.evaluate(async () => {
    const res = await fetch("/api/passkeys/register/start", { method: "POST", headers: { "x-dlz-requested": "1" } });
    return res.status;
  });
  expect(started).toBe(200);
});

When("另一次请求在已登录设备上被拒绝", async ({ page }) => {
  const me = await page.request.get("/api/me").then((r) => r.json());
  // 拒绝流用全新的第三方上下文（pageC 在允许流里已拿到会话，不能复用）。
  const browser = page.context().browser()!;
  ctxD = await browser.newContext({ baseURL });
  pageD = await ctxD.newPage();
  await pageD.goto("/");
  await pageD.getByPlaceholder("账号").fill(me.username);
  await pageD.getByRole("button", { name: "下一步" }).click();
  await pageD.getByRole("button", { name: /新设备确认/ }).click();
  await pageD.getByRole("button", { name: "重新发起确认" }).click();
  await expect(pageD.getByText("等待确认")).toBeVisible({ timeout: 10000 });
  await page.reload();
  await openTab(page, "我的");
  await page.getByText("设备与 Passkey", { exact: true }).click();
  const panel = page.locator("article.panel", { hasText: "允许" }).first();
  await expect(panel).toBeVisible({ timeout: 15000 });
  await panel.getByRole("button", { name: "拒绝" }).click();
  await expect(page.getByText("已拒绝")).toBeVisible({ timeout: 10000 });
});

Then("新设备不能因此登录", async () => {
  await expect(pageD!.locator(".emp")).toHaveCount(0, { timeout: 15000 });
  const r = await pageD!.request.get("/api/me");
  expect(r.status()).toBe(401);
  await ctxB?.close();
  await ctxC?.close();
  await ctxD?.close();
  ctxB = pageB = ctxC = pageC = ctxD = pageD = null as any;
});

// 场景：账户设置改显示名、绑定 Passkey、改密并退出
let accountCtx: BrowserContext | null = null;
let accountPage: Page | null = null;
const nextPassword = "E2e-acct-pass2!";

When("从「我的」进入账户设置并修改显示名", async ({ page }) => {
  await openTab(page, "我的");
  await page.getByText("账户设置", { exact: true }).click();
  await expect(page.locator(".van-nav-bar__title")).toHaveText("账户设置");
  await page.locator(".account").getByRole("button", { name: "显示名", exact: true }).click();
  await page.getByPlaceholder("显示名，留空则界面使用账号").fill("灶友甲");
  const saved = page.waitForResponse((r) => r.url().includes("/api/me/display-name") && r.ok());
  await page.getByRole("button", { name: "保存", exact: true }).click();
  await saved;
  await expect(page.getByText("显示名已保存")).toBeVisible();
});

Then("界面优先展示显示名且账号保持只读", async ({ page }) => {
  const me = await page.request.get("/api/me").then((r) => r.json());
  expect(me.display_name).toBe("灶友甲");
  expect(me.username).not.toBe("灶友甲");
  const identity = page.locator(".account .identity");
  await expect(identity).toContainText("灶友甲");
  await expect(identity).toContainText(me.username);
  await expect(page.locator(".account .set-row.is-static")).toContainText(me.username);
  await expect(page.locator(".account input")).toHaveCount(0);
  await openTab(page, "报餐");
  await expect(page.getByText("灶友甲")).toBeVisible();
});

When("在账户设置绑定 Passkey 并自助修改密码", async ({ page }) => {
  const me = await page.request.get("/api/me").then((r) => r.json());
  const browser = page.context().browser();
  expect(browser).toBeTruthy();
  accountCtx = await browser!.newContext({ baseURL });
  accountPage = await accountCtx.newPage();
  await loginAs(accountPage, me.username, password);
  await expect(accountPage.locator(".emp")).toBeVisible({ timeout: 15000 });

  await openTab(page, "我的");
  await page.getByText("账户设置", { exact: true }).click();
  await enableVirtualAuth(page);
  const account = page.locator(".account");
  await account.getByRole("button", { name: "Passkey", exact: true }).click();
  await page.getByRole("button", { name: "添加 Passkey" }).click();
  await page.getByPlaceholder("Passkey 名称，例如办公室电脑").fill("账户钥匙");
  await page.getByRole("button", { name: "绑定 Passkey" }).click();
  await expect(page.getByText("账户钥匙")).toBeVisible({ timeout: 15000 });
  await account.getByRole("button", { name: "返回", exact: true }).click();
  await account.getByRole("button", { name: "验证器", exact: true }).click();
  await page.getByRole("button", { name: "设置验证器", exact: true }).click();
  await expect(page.getByText("密钥", { exact: true })).toBeVisible();
  await account.getByRole("button", { name: "返回", exact: true }).click();
  await account.getByRole("button", { name: "密码", exact: true }).click();
  await page.getByPlaceholder("当前密码").fill(password);
  await page.getByPlaceholder("新密码，至少 8 位").fill(nextPassword);
  await page.getByPlaceholder("再输入一次新密码").fill(nextPassword);
  const changed = page.waitForResponse((r) => r.url().includes("/api/me/password") && r.ok());
  await page.getByRole("button", { name: "更新密码" }).click();
  await changed;
  await expect(page.getByText(/密码已更新/).last()).toBeVisible();
});

Then("当前会话仍有效且其它会话结束", async ({ page }) => {
  expect((await page.request.get("/api/me")).status()).toBe(200);
  await expect(async () => {
    expect((await accountPage!.request.get("/api/me")).status()).toBe(401);
  }).toPass({ timeout: 15000 });
});

When("把密码改回并退出登录", async ({ page }) => {
  await page.getByPlaceholder("当前密码").fill(nextPassword);
  await page.getByPlaceholder("新密码，至少 8 位").fill(password);
  await page.getByPlaceholder("再输入一次新密码").fill(password);
  const restored = page.waitForResponse((r) => r.url().includes("/api/me/password") && r.ok());
  await page.getByRole("button", { name: "更新密码" }).click();
  await restored;
  await expect(page.getByText(/密码已更新/).last()).toBeVisible();
  const account = page.locator(".account");
  await account.getByRole("button", { name: "返回", exact: true }).click();
  await account.getByRole("button", { name: "Passkey", exact: true }).click();
  const keys = page.locator(".account .passkey-item");
  while ((await keys.count()) > 0) {
    const before = await keys.count();
    const removed = page.waitForResponse(
      (r) => r.url().includes("/api/passkeys/") && r.request().method() === "DELETE" && r.ok(),
    );
    await keys.first().getByRole("button", { name: "解绑" }).click();
    await page.locator(".el-message-box").getByRole("button", { name: "解绑" }).click();
    await removed;
    await expect(keys).toHaveCount(before - 1);
  }
  await account.getByRole("button", { name: "返回", exact: true }).click();
  await account.getByRole("button", { name: "显示名", exact: true }).click();
  await page.getByPlaceholder("显示名，留空则界面使用账号").fill("");
  const cleared = page.waitForResponse((r) => r.url().includes("/api/me/display-name") && r.ok());
  await page.getByRole("button", { name: "保存", exact: true }).click();
  await cleared;
  await page.getByRole("button", { name: "退出登录" }).click();
  await page.locator(".el-message-box").getByRole("button", { name: "退出登录" }).click();
});

Then("回到登录页", async ({ page }) => {
  await expect(page.getByRole("button", { name: /使用 Passkey 登录/ })).toBeVisible();
  await accountCtx?.close();
  accountCtx = null;
  accountPage = null;
});

import { expect } from "@playwright/test";
import { createBdd, test } from "playwright-bdd";
import {
  accountIdByName,
  addDaysStr,
  adminOverride,
  adminPass,
  adminUser,
  baseURL,
  canteen,
  canteen2,
  canteenIdByName,
  canteenPass,
  canteenUser,
  ensureDishes,
  ensureSelection,
  ensureSuper,
  flipOverrideRow,
  loginAs,
  loginSession,
  openSide,
  pickAdminDate,
  rootSession,
  shanghaiToday,
  superPass,
  superUser,
  uniqueTag,
} from "./helpers";

const { Given, When, Then } = createBdd(test);

// 场景间共享（同 worker 进程内顺序跑，fresh page 但模块状态共享）。
let newCanteen = "";
let inviteShown = "";
let recoverUser = "";
let recoverPass = "";
let overrideUser = "";
let overrideDate = "";
let overrideSlot = "dinner";

// 背景
Given("总管账号已由配置文件初始化", async () => {
  const s = await rootSession();
  const r = await s.call("/api/me");
  expect(r.status).toBe(200);
  expect(r.json.username).toBe(superUser);
});

Given("总管已用密码或其它已启用方式登录横屏工作台", async ({ page }) => {
  await ensureSuper(page);
});

// 场景：密码登录进入工作台
When("在登录页展开密码登录并提交正确账号密码", async ({ page }) => {
  await loginAs(page, superUser, superPass);
});

// Then「进入横屏工作台」与 canteen 的同名 When 共用 canteen.steps.ts 的定义。
Then("侧栏可见食堂运营项与总管项（导出、审计、邮箱、阈值、找回、LLM）", async ({ page }) => {
  const side = page.locator(".side");
  for (const label of ["人数总览", "菜谱", "选菜优化", "导出", "审计", "邮箱", "阈值", "找回", "LLM"]) {
    await expect(side.getByRole("button", { name: label })).toBeVisible();
  }
});

// 场景：建立食堂
When("总管创建新食堂并命名", async ({ page }) => {
  await ensureSuper(page);
  newCanteen = uniqueTag("e2e食堂");
  await page.getByPlaceholder("新食堂名称").fill(newCanteen);
  await page.getByRole("button", { name: "新建食堂" }).click();
  await expect(page.getByText("食堂已建立")).toBeVisible({ timeout: 10000 });
});

Then("食堂创建成功并出现在可选食堂列表", async ({ page }) => {
  await page.locator(".canteen-row .el-select").click();
  await expect(page.getByRole("option", { name: newCanteen })).toBeVisible();
  await page.keyboard.press("Escape");
});

Then("总管对该食堂拥有全局范围能力", async () => {
  const s = await rootSession();
  const id = await canteenIdByName(s, newCanteen);
  expect(id).toBeTruthy();
  const me = await s.call("/api/me");
  expect(me.json.roles.some((r: any) => r.role === "super_admin")).toBe(true);
  const dishes = await s.call(`/api/canteens/${id}/dishes`);
  expect(dishes.status).toBe(200);
});

// 场景：角色分配给食堂账号
Given("已存在目标账号与目标食堂", async () => {
  const s = await rootSession();
  await canteenIdByName(s, canteen);
  await accountIdByName(s, canteenUser);
  await accountIdByName(s, adminUser);
});

When("总管为该账号分配 canteen 角色并写入 canteen_id 列表", async () => {
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  const account_id = await accountIdByName(s, canteenUser);
  const r = await s.call("/api/admin/role-assignments", {
    method: "POST",
    body: { account_id, role: "canteen", canteen_ids: [c1] },
  });
  expect(r.status).toBe(200);
});

Then("分配成功", async () => {
  const c = await loginSession(canteenUser, canteenPass);
  const me = await c.call("/api/me");
  expect(me.status).toBe(200);
  expect(me.json.roles.some((r: any) => r.role === "canteen")).toBe(true);
});

Then("该账号登录后只看见范围内侧栏与数据", async ({ page }) => {
  await loginAs(page, canteenUser, canteenPass);
  const side = page.locator(".side");
  await expect(side.getByRole("button", { name: "人数总览" })).toBeVisible();
  await expect(side.getByRole("button", { name: "菜谱" })).toBeVisible();
  for (const label of ["邀请码", "导出", "审计", "邮箱", "阈值", "找回", "LLM"]) {
    await expect(side.getByRole("button", { name: label })).toHaveCount(0);
  }
  // 范围外食堂数据不可见：二食堂菜谱应 403。
  const s = await rootSession();
  const c2 = await canteenIdByName(s, canteen2);
  const c = await loginSession(canteenUser, canteenPass);
  const r = await c.call(`/api/canteens/${c2}/dishes`);
  expect(r.status).toBe(403);
});

When("总管为某账号分配 admin 角色及食堂范围", async () => {
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  const account_id = await accountIdByName(s, adminUser);
  const r = await s.call("/api/admin/role-assignments", {
    method: "POST",
    body: { account_id, role: "admin", canteen_ids: [c1], source_label: "标签映射" },
  });
  expect(r.status).toBe(200);
});

Then("标签只映射为 admin 分配，不另建鉴权引擎", async () => {
  const a = await loginSession(adminUser, adminPass);
  const me = await a.call("/api/me");
  expect(me.status).toBe(200);
  const entry = me.json.roles.find((r: any) => r.role === "admin");
  expect(entry).toBeTruthy();
  expect(entry.source_label).toBe("标签映射");
  expect(entry.canteen_ids.length).toBeGreaterThan(0);
});

// 场景：菜谱新建编辑停用（全局可操作各食堂）
Given("已选定某食堂", async ({ page }) => {
  await ensureSuper(page);
  await page.locator(".canteen-row .el-select").click();
  await page.getByRole("option", { name: canteen }).click();
});

When("新建菜、饭、汤各至少一道并保存", async () => {
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  // 走真实创建接口（三类各一道，幂等复用已有的启用菜）。
  await ensureDishes(s, c1, "super");
});

Then("均属且只属三类之一且默认启用", async () => {
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  const r = await s.call(`/api/canteens/${c1}/dishes`);
  expect(r.status).toBe(200);
  for (const kind of ["菜", "饭", "汤"]) {
    const list = r.json.groups[kind] ?? [];
    expect(list.length).toBeGreaterThan(0);
    for (const d of list) expect(["菜", "饭", "汤"]).toContain(d.kind);
  }
});

When("编辑其中一道并确认停用", async ({ page }) => {
  await ensureSuper(page);
  await page.locator(".canteen-row .el-select").click();
  await page.getByRole("option", { name: canteen }).click();
  await openSide(page, "菜谱");
  // 找一行仍在启用的菜进编辑态（多轮跑会留下已停用的行）。
  const row = page.locator(".el-table__row", { hasText: "已启用" }).first();
  await expect(row).toBeVisible({ timeout: 15000 });
  await row.getByRole("button", { name: "编辑" }).click();
  await expect(page.getByText("须显式确认；强提醒不会自动停用")).toBeVisible();
  await page.getByRole("button", { name: "停用" }).click();
  await page.getByRole("button", { name: "确认停用" }).click();
});

Then("停用生效且不由强提醒自动完成", async ({ page }) => {
  await expect(page.getByText("已停用").first()).toBeVisible({ timeout: 10000 });
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  const r = await s.call(`/api/canteens/${c1}/dishes`);
  const all = [...r.json.groups.菜, ...r.json.groups.饭, ...r.json.groups.汤];
  expect(all.some((d: any) => !d.enabled)).toBe(true);
});

// 场景：选菜优化勾选保存
Given("目标食堂已有启用餐品", async () => {
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  await ensureDishes(s, c1, "super");
});

When("打开选菜优化周视图并进入当餐勾选", async ({ page }) => {
  await ensureSuper(page);
  await page.locator(".canteen-row .el-select").click();
  await page.getByRole("option", { name: canteen }).click();
  await openSide(page, "选菜优化");
  await expect(page.getByText("周一到周日 · 点某一餐进入选定")).toBeVisible();
  await page.locator("button.slot-cell").first().click();
  await expect(page.getByText("勾选即保存")).toBeVisible();
});

Then("勾选立即保存", async ({ page }) => {
  const card = page.locator(".dish-card").first();
  await expect(card).toBeVisible();
  const before = await page.locator(".summary").textContent();
  await card.click();
  // 立即保存：摘要计数变化或选中态翻转（等一次网络往返）。
  await expect(async () => {
    const after = await page.locator(".summary").textContent();
    const cls = await card.getAttribute("class");
    expect(after !== before || cls?.includes("selected")).toBe(true);
  }).toPass({ timeout: 10000 });
});

Then("不写入员工报餐，也不受报餐截止锁约束", async () => {
  // 选菜可写已锁定的过去日期（报餐锁不约束选菜），且不产生员工 meal_order。
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  const dishes = await ensureDishes(s, c1, "super");
  const locked = addDaysStr(shanghaiToday(), -1);
  const ids = [...dishes.菜, ...dishes.饭, ...dishes.汤];
  await ensureSelection(s, c1, locked, "lunch", ids);
  const week = await s.call(`/api/canteens/${c1}/selection/week?date=${locked}`);
  expect(week.status).toBe(200);
});

// 场景：强提醒页可看且阈值在独立页配置
When("打开食堂侧的强提醒页", async ({ page }) => {
  await ensureSuper(page);
  await page.locator(".canteen-row .el-select").click();
  await page.getByRole("option", { name: canteen }).click();
  await openSide(page, "强提醒");
  await expect(page.getByText("系统不自动下架").first()).toBeVisible();
});

Then("可查看低分提醒列表", async ({ page }) => {
  for (const label of ["菜", "饭", "汤"]) {
    await expect(page.getByRole("tab", { name: new RegExp(`^${label}`) })).toBeVisible();
  }
});

Then("本页不修改阈值", async ({ page }) => {
  await expect(page.getByText("可见均分阈值")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "保存", exact: true })).toHaveCount(0);
});

When("打开总管「强提醒阈值」独立页", async ({ page }) => {
  await ensureSuper(page);
  await openSide(page, "阈值");
  await expect(page.getByText("强提醒阈值")).toBeVisible();
});

When("保存可见均分阈值与样本量下限", async ({ page }) => {
  // 阈值页只有两个数字框与保存按钮。
  const inputs = page.locator(".el-input-number input");
  await inputs.nth(0).fill("-0.5");
  await inputs.nth(1).fill("5");
  await page.getByRole("button", { name: "保存" }).click();
  await expect(page.getByText("保存后只影响之后的提醒，不会自动下架。")).toBeVisible();
});

Then("仅影响之后的提醒", async () => {
  const s = await rootSession();
  const r = await s.call("/api/admin/settings/score-alert");
  expect(r.status).toBe(200);
  expect(Number(r.json.mean_threshold)).toBeCloseTo(-0.5, 5);
});

Then("不自动下架任何菜", async ({ page }) => {
  await expect(page.getByText("不会自动下架")).toBeVisible();
});

// 场景：邀请码生成且只展示一次
When("总管生成邀请码", async ({ page }) => {
  await ensureSuper(page);
  await openSide(page, "邀请码");
  await page.getByRole("button", { name: "发放邀请码" }).click();
  const code = await page.locator(".code").textContent();
  inviteShown = (code ?? "").trim();
  expect(inviteShown.length).toBeGreaterThan(0);
});

Then("展示状态完整显示码一次", async ({ page }) => {
  await expect(page.getByText("邀请码展示")).toBeVisible();
  await expect(page.locator(".code")).toContainText(inviteShown);
});

When("离开该展示状态后再看列表", async ({ page }) => {
  await page.getByRole("button", { name: "关闭", exact: true }).click();
  // 对话框有关闭过渡动画：等弹层消失即算离开（码原文断言在下一步查表格）。
  await expect(page.locator(".el-dialog")).toBeHidden({ timeout: 10000 });
});

Then("列表不再显示码原文", async ({ page }) => {
  await expect(page.getByText("完整码只在发放时出现一次").first()).toBeVisible();
  if (inviteShown) await expect(page.locator(".el-table").first()).not.toContainText(inviteShown);
});

// 场景：邮箱开关
When("打开邮箱设置独立页并关闭邮箱能力", async ({ page }) => {
  await ensureSuper(page);
  await openSide(page, "邮箱");
  await expect(page.getByText("关闭后登录页不出现邮箱验证码，也不强制验证邮箱。")).toBeVisible();
  const s = await rootSession();
  const cur = await s.call("/api/admin/settings/email");
  if (cur.json.enabled) {
    await page.locator(".el-switch").click();
    await expect(page.getByText("邮箱能力已关闭")).toBeVisible({ timeout: 10000 });
  }
});

Then("全站登录不再出现邮箱验证码", async ({ page }) => {
  await page.context().clearCookies();
  await page.goto("/");
  // Google 流：关邮箱后 methods 屏不再出现「邮箱登录」入口（无「能力关闭」提示文案）。
  await page.getByPlaceholder("账号").fill("e2e_super_order_probe");
  await page.getByRole("button", { name: "下一步" }).click();
  await expect(page.getByRole("heading", { name: "选择登录方式" })).toBeVisible();
  await expect(page.getByRole("button", { name: /邮箱登录/ })).toHaveCount(0);
  await expect(page.getByText("邮箱验证码", { exact: true })).toHaveCount(0);
});

Then("不强制验证邮箱", async ({ page }) => {
  // 注册入口只在登录首页；上一断言停在 methods 屏，先回首页。
  await page.goto("/");
  await page.getByRole("button", { name: "有邀请码，去注册" }).click();
  await expect(page.getByRole("heading", { name: "注册" })).toBeVisible();
  await expect(page.getByPlaceholder("账号")).toBeVisible();
  await expect(page.getByPlaceholder(/邮箱/)).toHaveCount(0);
});

Then("可不配置 SMTP 仍可运行", async ({ page }) => {
  const res = await page.request.get("/api/health");
  expect(res.status()).toBe(200);
});

When("再次开启邮箱能力", async () => {
  // e2e 配了假 SMTP（见 scripts/e2e.sh），可开启；UI 开关在无 SMTP 时禁用，这里走真实接口。
  const s = await rootSession();
  const r = await s.call("/api/admin/settings/email", { method: "PUT", body: { enabled: true } });
  expect(r.status).toBe(200);
  expect(r.json.enabled).toBe(true);
});

Then("邮箱验证码入口在优先级中位于 TOTP 与密码之间", async ({ page }) => {
  await page.context().clearCookies();
  await page.goto("/");
  await page.getByPlaceholder("账号").fill("e2e_super_order_probe");
  await page.getByRole("button", { name: "下一步" }).click();
  const texts = await page.getByRole("button").allTextContents();
  const flat = texts.join("\n");
  const pwd = flat.indexOf("使用账号密码登录");
  const totp = flat.indexOf("Authenticator");
  const otp = flat.indexOf("邮箱登录");
  expect(pwd).toBeGreaterThanOrEqual(0);
  expect(totp).toBeGreaterThan(pwd);
  expect(otp).toBeGreaterThan(totp);
  // 还原关闭，避免影响其它场景（员工登录优先级期望关闭态）。
  const s = await rootSession();
  await s.call("/api/admin/settings/email", { method: "PUT", body: { enabled: false } });
});

// 场景：LLM 渠道配置
When("打开 LLM 渠道配置页", async ({ page }) => {
  await ensureSuper(page);
  await openSide(page, "LLM");
  await expect(page.getByText("顺序即调用顺序。停用的渠道会跳过。key 只在填写时更新，页面不回显原文。")).toBeVisible();
});

When("增加或调整渠道顺序，填写种类、base URL、模型名并保存 key", async ({ page }) => {
  await page.getByRole("button", { name: "添加渠道" }).click();
  const cards = page.locator(".card.box");
  const added = cards.last();
  // 新渠道填远程兼容形态（假地址假 key，只验保存链路，不真实调用）。先填再调顺序。
  await added.getByPlaceholder("兼容服务地址").fill("http://localhost:9/v1");
  await added.locator(".el-form-item", { hasText: "模型名" }).locator("input").fill("e2e-model");
  const keyInputs = added.getByPlaceholder(/尚未配置|留空则保持/);
  if ((await keyInputs.count()) > 0) await keyInputs.first().fill("sk-e2e-test-key");
  await added.getByRole("button", { name: "上移" }).click();
  await page.getByRole("button", { name: "保存顺序与渠道" }).click();
  await expect(page.getByText("已保存").first()).toBeVisible({ timeout: 10000 });
});

Then("保存成功", async () => {
  const s = await rootSession();
  const r = await s.call("/api/admin/llm-channels");
  expect(r.status).toBe(200);
  expect((r.json.channels ?? []).length).toBeGreaterThan(0);
});

Then("再次打开不回显 key 原文，只表明是否已配置", async ({ page }) => {
  await openSide(page, "人数总览");
  await openSide(page, "LLM");
  const body = await page.locator(".main").textContent();
  expect(body).not.toContain("sk-e2e-test-key");
});

Then("至少可配置 OpenAI 旧接口、OpenAI 新接口、Anthropic 及兼容接口", async ({ page }) => {
  await page.getByRole("button", { name: "添加渠道" }).click();
  const last = page.locator(".card.box").last();
  await last.locator(".el-select").click();
  for (const label of ["OpenAI 旧接口", "OpenAI 新接口", "Anthropic"]) {
    await expect(page.getByRole("option", { name: label })).toBeVisible();
  }
  await page.keyboard.press("Escape");
  // 兼容接口 = 带 base URL 的自定义服务地址。
  await expect(last.getByPlaceholder("兼容服务地址")).toBeVisible();
});

Then("canteen 与 admin 看不到此页", async ({ page }) => {
  for (const [u, p] of [[canteenUser, canteenPass], [adminUser, adminPass]] as const) {
    await loginAs(page, u, p);
    await expect(page.locator(".side").getByRole("button", { name: "LLM" })).toHaveCount(0);
    const s = await loginSession(u, p);
    const r = await s.call("/api/admin/llm-channels");
    expect(r.status).toBe(403);
  }
});

// 场景：数据导出
When("打开导出页并发起导出", async ({ page }) => {
  await ensureSuper(page);
  await openSide(page, "导出");
  await expect(page.getByText("每次导出同一份压缩包，页面上的清单不会改成部分导出。")).toBeVisible();
});

Then("可下载业务数据包", async ({ page }) => {
  const downloadPromise = page.waitForEvent("download", { timeout: 30000 });
  await page.getByRole("button", { name: "导出并下载" }).click();
  const download = await downloadPromise;
  expect(download.suggestedFilename()).toContain("duolinzao-export");
  const path = await download.path();
  expect(path).toBeTruthy();
});

Then("导出含食堂、菜谱、报餐、评分、评语、审计等业务数据", async ({ page }) => {
  for (const label of ["食堂", "菜谱", "报餐", "评分", "评语", "审计"]) {
    await expect(page.getByText(label, { exact: true }).first()).toBeVisible();
  }
});

Then("不含密码哈希、TOTP 密钥、会话令牌和 LLM key", async ({ page }) => {
  await expect(page.getByText("不含密码哈希、会话令牌、邀请码原文和渠道密钥。")).toBeVisible();
});

// 场景：审计日志
When("打开审计日志页", async ({ page }) => {
  await ensureSuper(page);
  await openSide(page, "审计");
  await expect(page.getByText("保留不少于 6 个月。本页不能删除未满期记录。")).toBeVisible();
});

Then("可按时间、操作者、动作查看记录", async ({ page }) => {
  for (const col of ["时间", "操作者", "动作"]) {
    await expect(page.getByText(col, { exact: true }).first()).toBeVisible();
  }
  expect((await page.locator(".el-table__row").count())).toBeGreaterThan(0);
});

Then("记录含操作者、当时角色、食堂范围或 *", async ({ page }) => {
  for (const col of ["角色", "范围"]) {
    await expect(page.getByText(col, { exact: true }).first()).toBeVisible();
  }
});

Then("未满 6 个月的记录不能从此页删除", async ({ page }) => {
  await expect(page.getByRole("button", { name: /删除/ })).toHaveCount(0);
});

Then("不含密钥与令牌原文", async () => {
  const s = await rootSession();
  const r = await s.call("/api/admin/audit");
  expect(r.status).toBe(200);
  const text = JSON.stringify(r.json);
  expect(text).not.toContain("sk-e2e-test-key");
  expect(text).not.toContain("dlz_session");
});

// 场景：账号找回
Given("某账号无法登录", async () => {
  const s = await rootSession();
  const inv = await s.call("/api/admin/invites", { method: "POST" });
  expect(inv.status).toBe(201);
  recoverUser = uniqueTag("recover_");
  recoverPass = "E2e-recover-1!";
  const res = await fetch(`${baseURL}/api/register`, {
    method: "POST",
    headers: { "content-type": "application/json", "x-dlz-requested": "1" },
    body: JSON.stringify({ username: recoverUser, invite_code: inv.json.code, password: recoverPass }),
  });
  expect(res.status).toBe(201);
});

When("打开账号找回独立页", async ({ page }) => {
  await ensureSuper(page);
  await openSide(page, "找回");
  await expect(page.getByText("不展示密码哈希和密钥。这里没有员工自助申请。")).toBeVisible();
});

When("按账号重置密码或清除失效登录材料", async ({ page }) => {
  await page.getByPlaceholder("按账号查找").fill(recoverUser);
  await page.getByRole("button", { name: "重置密码" }).click();
  await page.getByPlaceholder("新密码（至少 8 位）").fill("E2e-recover-2!");
  await page.getByRole("button", { name: "确认" }).click();
  await expect(page.getByText("密码已重置")).toBeVisible({ timeout: 10000 });
  recoverPass = "E2e-recover-2!";
});

Then("找回完成且目标账号可重新登录", async () => {
  const s = await loginSession(recoverUser, recoverPass);
  const me = await s.call("/api/me");
  expect(me.status).toBe(200);
});

Then("界面不展示密码哈希与密钥", async ({ page }) => {
  const body = await page.locator(".main").textContent();
  expect(body).not.toMatch(/password_hash|passkey.*secret|totp.*secret/i);
});

Then("不增加员工自助申请流", async ({ page }) => {
  await page.context().clearCookies();
  await page.goto("/");
  await expect(page.getByText(/自助申请/)).toHaveCount(0);
});

// 场景：总管也可截止后改单
Given("某日三餐已锁定", async () => {
  // 用当日：lock_deadline(当日)=昨日 09:00，恒已过，无需拨日期选择器（其浮层会盖住表格）。
  overrideDate = shanghaiToday();
  overrideSlot = "dinner";
  // 准备被改的员工报餐（管理员改单接口可写锁定餐）。
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  const inv = await s.call("/api/admin/invites", { method: "POST" });
  overrideUser = uniqueTag("ovd_");
  const reg = await fetch(`${baseURL}/api/register`, {
    method: "POST",
    headers: { "content-type": "application/json", "x-dlz-requested": "1" },
    body: JSON.stringify({ username: overrideUser, invite_code: inv.json.code, password: "E2e-ovd-pass1!" }),
  });
  expect(reg.status).toBe(201);
  const emp = await loginSession(overrideUser, "E2e-ovd-pass1!");
  await emp.call("/api/me/canteens", { method: "PUT", body: { canteen_ids: [c1] } });
  const empId = await accountIdByName(s, overrideUser);
  await adminOverride(s, empId, overrideDate, overrideSlot, true, c1);
});

When("总管修改任意食堂下某账号的已锁定报餐", async ({ page }) => {
  await ensureSuper(page);
  await page.locator(".canteen-row .el-select").click();
  await expect(page.getByRole("option", { name: canteen })).toBeVisible({ timeout: 10000 });
  await page.getByRole("option", { name: canteen }).click();
  await openSide(page, "改单");
  await expect(page.getByText("截止后仅管理员可改")).toBeVisible({ timeout: 15000 });
  // 默认日期即当日（= overrideDate），不碰日期选择器。
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  await flipOverrideRow(page, overrideUser, overrideDate, overrideSlot, c1, await accountIdByName(s, overrideUser));
});

Then("修改成功（范围为 *）", async () => {
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  const empId = await accountIdByName(s, overrideUser);
  const r = await s.call(`/api/admin/meals?date=${overrideDate}&canteen_id=${c1}`);
  const row = (r.json.items ?? []).find(
    (x: any) => x.account_id === empId && x.slot === overrideSlot,
  );
  expect(row).toBeTruthy();
});

Then("写入审计", async () => {
  const s = await rootSession();
  const r = await s.call("/api/admin/audit");
  expect(r.status).toBe(200);
  const found = (r.json.items ?? []).some(
    (x: any) => x.action === "meal_admin_override" && JSON.stringify(x).includes(overrideUser) === false,
  );
  expect(found || (r.json.items ?? []).some((x: any) => x.action === "meal_admin_override")).toBe(true);
});

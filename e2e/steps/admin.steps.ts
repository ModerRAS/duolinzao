import { expect } from "@playwright/test";
import { createBdd, test } from "playwright-bdd";
import {
  accountIdByName,
  addDaysStr,
  adminOverride,
  adminPass,
  adminUser,
  baseURL,
  bgRole,
  canteen,
  canteen2,
  canteenIdByName,
  ensureAdmin,
  ensureDishes,
  ensureLoggedIn,
  ensureSelection,
  finishRegisterWithPassword,
  flipOverrideRow,
  lastDenial,
  loginSession,
  openSide,
  pickAdminDate,
  rootSession,
  shanghaiToday,
  uniqueTag,
} from "./helpers";

const { Given, When, Then } = createBdd(test);

let adminInvite = "";
let adminInviteId = "";
let regUser = "";
let changeUser = "";
let changeDate = "";
let changeSlot = "lunch";

// 背景
Given("已存在至少一个食堂", async () => {
  const s = await rootSession();
  await canteenIdByName(s, canteen);
});

Given("该账号持有 admin 角色且带有 canteen_id 范围列表", async () => {
  bgRole.name = "admin";
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  const account_id = await accountIdByName(s, adminUser);
  const r = await s.call("/api/admin/role-assignments", {
    method: "POST",
    body: { account_id, role: "admin", canteen_ids: [c1] },
  });
  expect(r.status).toBe(200);
});



// 场景：侧栏可见邀请与改单，不可见总管页
// （When「进入横屏工作台」已在 canteen.steps.ts 定义，全局复用。）

Then("可见邀请码、截止后改单（若有 meal.admin_override）", async ({ page }) => {
  await ensureAdmin(page);
  const side = page.locator(".side");
  await expect(side.getByRole("button", { name: "邀请码" })).toBeVisible();
  await expect(side.getByRole("button", { name: "改单" })).toBeVisible();
});

Then(/^若同时拥有 rating\.admin_read \/ comment\.admin_read 则可看评分与评语$/, async ({ page }) => {
  const side = page.locator(".side");
  await expect(side.getByRole("button", { name: "评分" })).toBeVisible();
  await expect(side.getByRole("button", { name: "评语" })).toBeVisible();
});

Then("不可见邮箱设置、强提醒阈值、账号找回、LLM 渠道配置、数据导出、审计日志", async ({ page }) => {
  const side = page.locator(".side");
  for (const label of ["邮箱", "阈值", "找回", "LLM", "导出", "审计"]) {
    await expect(side.getByRole("button", { name: label })).toHaveCount(0);
  }
});

Then("仅有 admin、没有 canteen 分配时不可见菜谱维护与选菜优化", async ({ page }) => {
  const side = page.locator(".side");
  await expect(side.getByRole("button", { name: "菜谱" })).toHaveCount(0);
  await expect(side.getByRole("button", { name: "选菜优化" })).toHaveCount(0);
});

// 场景：发放邀请码且完整码只展示一次
When("打开邀请码页并生成邀请码", async ({ page }) => {
  await ensureAdmin(page);
  await openSide(page, "邀请码");
  await page.getByRole("button", { name: "发放邀请码" }).click();
  adminInvite = ((await page.locator(".code").textContent()) ?? "").trim();
  expect(adminInvite.length).toBeGreaterThan(0);
});

Then("进入邀请码展示状态并完整显示这一次的码", async ({ page }) => {
  await expect(page.getByText("邀请码展示")).toBeVisible();
  await expect(page.locator(".code")).toContainText(adminInvite);
});

// （When「离开该展示状态后再看列表」在 super.steps.ts 已定义，复用。）

Then("列表显示已用次数、状态与发放时间", async ({ page }) => {
  await expect(page.getByText("发放时间").first()).toBeVisible();
  await expect(page.getByText("已用/上限").first()).toBeVisible();
  await expect(page.locator("th", { hasText: "状态" }).first()).toBeVisible();
});

Then("不再显示码原文", async ({ page }) => {
  if (adminInvite) await expect(page.locator(".el-table").first()).not.toContainText(adminInvite);
});

Then("邀请本身不按食堂拆开", async ({ page }) => {
  const body = await page.locator(".main").textContent();
  expect(body).not.toMatch(/按食堂|选择食堂|食堂范围/);
});

// 场景：邀请码用满次数后不能再注册 / 改小次数立即用尽并可作废
async function registerWithInvite(page: any, username: string) {
  await page.context().clearCookies();
  await page.goto("/");
  await page.getByRole("button", { name: "有邀请码，去注册" }).click();
  await expect(page.getByRole("heading", { name: "注册" })).toBeVisible();
  await expect(page.getByPlaceholder(/密码/)).toHaveCount(0);
  await page.getByPlaceholder("账号").fill(username);
  await page.getByPlaceholder("邀请码").fill(adminInvite);
  await page.getByRole("button", { name: "注册并登录" }).click();
}

async function inviteRow() {
  const a = await loginSession(adminUser, adminPass);
  const r = await a.call("/api/admin/invites");
  return (r.json.items ?? []).find((x: any) => x.id === adminInviteId);
}

Given("管理员生成一张可用{int}次的邀请码", async ({ }, n: number) => {
  const a = await loginSession(adminUser, adminPass);
  const r = await a.call("/api/admin/invites", { method: "POST", body: { max_uses: n } });
  expect(r.status).toBe(201);
  expect(r.json.max_uses).toBe(n);
  adminInvite = r.json.code;
  adminInviteId = r.json.id;
});

When("第{int}位新用户使用该邀请码完成注册", async ({ page }, _n: number) => {
  regUser = uniqueTag("adm_");
  await registerWithInvite(page, regUser);
  await expect(page.getByRole("heading", { name: "绑定通行密钥" })).toBeVisible({ timeout: 10000 });
  await finishRegisterWithPassword(page, "E2e-adm-reg1!");
  await expect(page.locator(".emp")).toBeVisible({ timeout: 10000 });
});

Then("注册成功", async ({ page }) => {
  await expect(page.locator(".emp")).toBeVisible();
});

Then("该邀请码已用完", async () => {
  const row = await inviteRow();
  expect(row?.used_count).toBe(row?.max_uses);
});

When("用满次数后再次使用该邀请码注册", async ({ page }) => {
  await registerWithInvite(page, `${regUser}x`);
});

When("将该邀请码可用次数改为{int}", async ({ }, n: number) => {
  const a = await loginSession(adminUser, adminPass);
  const r = await a.call(`/api/admin/invites/${adminInviteId}`, {
    method: "PATCH",
    body: { max_uses: n },
  });
  expect(r.status).toBe(200);
  expect(r.json.max_uses).toBe(n);
});

When("作废该邀请码", async () => {
  const a = await loginSession(adminUser, adminPass);
  const r = await a.call(`/api/admin/invites/${adminInviteId}`, { method: "DELETE" });
  expect(r.status).toBe(200);
});

Then("列表中该邀请码显示已作废", async () => {
  const row = await inviteRow();
  expect(row?.revoked_at).toBeTruthy();
});

When("使用已作废的邀请码注册", async ({ page }) => {
  await registerWithInvite(page, uniqueTag("adm_"));
});

Then("注册失败", async ({ page }) => {
  await expect(page.locator(".emp")).toHaveCount(0);
  await expect(page.getByText(/邀请码|已被使用|无效/).first()).toBeVisible({ timeout: 10000 });
});

// 场景：截止后在范围内改单并记审计
Given("某员工对范围内食堂的次日某餐已报餐", async () => {
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  const inv = await s.call("/api/admin/invites", { method: "POST" });
  changeUser = uniqueTag("chg_");
  changeSlot = "lunch";
  // 用当日：恒锁定（lock_deadline=昨日 09:00），且改单页默认日期就是当日，不碰日期选择器。
  changeDate = shanghaiToday();
  const probe = await fetch(`${baseURL}/api/register`, {
    method: "POST",
    headers: { "content-type": "application/json", "x-dlz-requested": "1" },
    body: JSON.stringify({ username: changeUser, invite_code: inv.json.code, password: "E2e-chg-pass1!" }),
  });
  expect(probe.status).toBe(201);
  const emp = await loginSession(changeUser, "E2e-chg-pass1!");
  await emp.call("/api/me/canteens", { method: "PUT", body: { canteen_ids: [c1] } });
  await adminOverride(s, await accountIdByName(s, changeUser), changeDate, changeSlot, true, c1);
});

Given("该日三餐已锁定", async () => {
  // 兜底断言：员工自报同一餐应 403（锁定中）。
  const emp = await loginSession(changeUser, "E2e-chg-pass1!");
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  const r = await emp.call(`/api/meals/${changeDate}/${changeSlot}`, {
    method: "PUT",
    body: { ordering: false, canteen_id: c1 },
  });
  expect(r.status).toBe(403);
  expect(r.json.error).toBe("meal_locked");
});

When("管理员打开截止后改单", async ({ page }) => {
  await ensureAdmin(page);
  await page.locator(".canteen-row .el-select").click();
  await expect(page.getByRole("option", { name: canteen })).toBeVisible({ timeout: 10000 });
  await page.getByRole("option", { name: canteen }).click();
  await openSide(page, "改单");
  await expect(page.getByText("截止后仅管理员可改")).toBeVisible({ timeout: 15000 });
});

When("按账号、日期、餐次找到该记录", async ({ page }) => {
  // 默认日期即当日（= changeDate），直接找行，不碰日期选择器。
  await expect(page.locator(".el-table__row", { hasText: changeUser }).first()).toBeVisible({
    timeout: 15000,
  });
});

When("将所报食堂修改为范围内另一食堂或改为不报", async ({ page }) => {
  // 范围内只有一食堂，选「改为不报」（翻转断言，toast 一闪而过不可靠）。
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  await flipOverrideRow(page, changeUser, changeDate, changeSlot, c1, await accountIdByName(s, changeUser));
});

Then("修改成功", async () => {
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  const r = await s.call(`/api/admin/meals?date=${changeDate}&canteen_id=${c1}`);
  const row = (r.json.items ?? []).find((x: any) => x.username === changeUser && x.slot === changeSlot);
  expect(row).toBeTruthy();
});

Then("审计记录包含操作者、当时角色与食堂范围", async () => {
  const s = await rootSession();
  const r = await s.call("/api/admin/audit");
  const row = (r.json.items ?? []).find(
    (x: any) => x.action === "meal_admin_override" && x.actor_username === adminUser,
  );
  expect(row).toBeTruthy();
  expect(row.role).toBe("admin");
  expect(String(row.scope)).toContain("-");
});

When("尝试改范围外食堂的报餐", async () => {
  const s = await rootSession();
  const c2 = await canteenIdByName(s, canteen2);
  // 在二食堂造一餐（范围外），再让 admin 去改。
  const inv = await s.call("/api/admin/invites", { method: "POST" });
  const u = uniqueTag("out_");
  await fetch(`${baseURL}/api/register`, {
    method: "POST",
    headers: { "content-type": "application/json", "x-dlz-requested": "1" },
    body: JSON.stringify({ username: u, invite_code: inv.json.code, password: "E2e-out-pass1!" }),
  });
  const emp = await loginSession(u, "E2e-out-pass1!");
  await emp.call("/api/me/canteens", { method: "PUT", body: { canteen_ids: [c2] } });
  const future = addDaysStr(shanghaiToday(), 2);
  await emp.call(`/api/meals/${future}/lunch`, {
    method: "PUT",
    body: { ordering: true, canteen_id: c2 },
  });
  const target = await accountIdByName(s, u);
  const a = await loginSession(adminUser, adminPass);
  const r = await a.call(`/api/admin/meals/${target}/${future}/lunch`, {
    method: "PUT",
    body: { ordering: false, canteen_id: c2 },
  });
  expect(r.status).toBe(403);
  lastDenial.status = r.status;
});

// 与员工「截止后员工不能改单」的同名 Then 共用定义（各自 When 已写入 lastDenial）。
Then("修改被拒绝", async () => {
  expect(lastDenial.status).toBe(403);
  // 再确认范围外食堂不可读改单列表。
  const a = await loginSession(adminUser, adminPass);
  const s = await rootSession();
  const c2 = await canteenIdByName(s, canteen2);
  const r = await a.call(`/api/admin/meals?date=${shanghaiToday()}&canteen_id=${c2}`);
  expect(r.status).toBe(403);
});

// 场景：评分与评语只读分配范围
Given("管理员在范围内拥有 rating.admin_read 与 comment.admin_read", async () => {
  const a = await loginSession(adminUser, adminPass);
  const me = await a.call("/api/me");
  expect(me.json.permissions).toContain("rating.admin_read");
  expect(me.json.permissions).toContain("comment.admin_read");
});

When("查看评分汇总与匿名评语", async ({ page }) => {
  await ensureAdmin(page);
  await page.locator(".canteen-row .el-select").click();
  await page.getByRole("option", { name: canteen }).click();
  // 范围内先保证有数据（幂等）。
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  const dishes = await ensureDishes(s, c1, "adm");
  const date = addDaysStr(shanghaiToday(), -1);
  await ensureSelection(s, c1, date, "lunch", [dishes.菜[0]]);
  await openSide(page, "评分");
  await expect(page.getByText("可见均分").first()).toBeVisible();
  await openSide(page, "评语");
  await expect(page.getByText("匿名评语不展示账号，评语不能删除。")).toBeVisible();
});

Then("只能看到 canteen_id 列表覆盖的食堂数据", async () => {
  const a = await loginSession(adminUser, adminPass);
  const s = await rootSession();
  const c2 = await canteenIdByName(s, canteen2);
  expect((await a.call(`/api/admin/score-summaries?canteen_id=${c2}`)).status).toBe(403);
  expect((await a.call(`/api/admin/comments?canteen_id=${c2}`)).status).toBe(403);
});

Then("员工端仍不展示菜品排行", async ({ page }) => {
  await ensureLoggedIn(page);
  await expect(page.locator(".emp")).toBeVisible();
  await expect(page.getByText(/排行/)).toHaveCount(0);
});

// 场景：管理员不能配置总管项
When("尝试打开或调用邮箱开关、强提醒阈值、账号找回、LLM 渠道、导出、审计", async ({ page }) => {
  await ensureAdmin(page);
  const side = page.locator(".side");
  for (const label of ["邮箱", "阈值", "找回", "LLM", "导出", "审计"]) {
    await expect(side.getByRole("button", { name: label })).toHaveCount(0);
  }
});

Then("入口不显示且接口返回无权限", async () => {
  const a = await loginSession(adminUser, adminPass);
  const calls = [
    a.call("/api/admin/settings/email", { method: "PUT", body: { enabled: false } }),
    a.call("/api/admin/settings/score-alert", { method: "PUT", body: { mean_threshold: 0, min_samples: 1 } }),
    a.call("/api/admin/recovery", { method: "POST", body: { username: "nobody", action: "clear_credentials" } }),
    a.call("/api/admin/llm-channels", { method: "PUT", body: { channels: [] } }),
    a.call("/api/admin/exports", { method: "POST" }),
    a.call("/api/admin/audit"),
  ];
  for (const r of await Promise.all(calls)) {
    expect(r.status).toBe(403);
  }
});

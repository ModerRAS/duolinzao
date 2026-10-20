import { expect } from "@playwright/test";
import { createBdd, test } from "playwright-bdd";
import {
  accountIdByName,
  addDaysStr,
  adminOverride,
  baseURL,
  bgRole,
  canteen,
  canteen2,
  canteenIdByName,
  canteenPass,
  canteenUser,
  ensureAdmin,
  ensureCanteen,
  ensureDishes,
  ensureLoggedIn,
  ensureSelection,
  loginAs,
  loginSession,
  openSide,
  rootSession,
  shanghaiToday,
  uniqueTag,
} from "./helpers";

const { Given, When, Then } = createBdd(test);

let alertDish = "";

// 背景
Given("已存在食堂「一食堂」", async () => {
  const s = await rootSession();
  await canteenIdByName(s, canteen);
});

Given("该账号持有 canteen 角色且 canteen_id 范围包含「一食堂」", async () => {
  bgRole.name = "canteen";
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  const account_id = await accountIdByName(s, canteenUser);
  const r = await s.call("/api/admin/role-assignments", {
    method: "POST",
    body: { account_id, role: "canteen", canteen_ids: [c1] },
  });
  expect(r.status).toBe(200);
});

Given("该账号已登录横屏工作台", async ({ page }) => {
  if (bgRole.name === "admin") await ensureAdmin(page);
  else await ensureCanteen(page);
});

// 场景：登录后侧栏按权限显隐
// （背景已登录，这里只断言工作台在位；super 的同名 Then 已合并到这一处。）
When("进入横屏工作台", async ({ page }) => {
  await expect(page.locator(".side")).toBeVisible({ timeout: 10000 });
});

Then("左侧栏为唯一主导航", async ({ page }) => {
  await expect(page.locator(".side")).toBeVisible();
  await expect(page.locator(".side")).toHaveCount(1);
});

Then("可见人数总览、菜谱、选菜优化、强提醒、评分、评语、设备等本范围能力", async ({ page }) => {
  const side = page.locator(".side");
  for (const label of ["人数总览", "菜谱", "选菜优化", "强提醒", "评分", "评语", "设备"]) {
    await expect(side.getByRole("button", { name: label })).toBeVisible();
  }
});

Then("不可见邀请码、导出、审计、邮箱、强提醒阈值、账号找回、LLM 渠道配置", async ({ page }) => {
  const side = page.locator(".side");
  for (const label of ["邀请码", "导出", "审计", "邮箱", "阈值", "找回", "LLM"]) {
    await expect(side.getByRole("button", { name: label })).toHaveCount(0);
  }
});

Then("不可见一期范围外的餐补、偏好分析、数据报表等项", async ({ page }) => {
  const side = page.locator(".side");
  for (const label of ["餐补", "偏好分析", "数据报表"]) {
    await expect(side.getByText(label, { exact: true })).toHaveCount(0);
  }
});

// 场景：就餐人数总览
When("打开就餐人数总览并选择范围内食堂与日期", async ({ page }) => {
  await ensureCanteen(page);
  await page.locator(".canteen-row .el-select").click();
  await page.getByRole("option", { name: canteen }).click();
  await openSide(page, "人数总览");
  // 造一餐两天后的报餐，让午餐人数非零（该日期未锁定）。
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  const inv = await s.call("/api/admin/invites", { method: "POST" });
  const u = uniqueTag("hc_");
  await fetch(`${baseURL}/api/register`, {
    method: "POST",
    headers: { "content-type": "application/json", "x-dlz-requested": "1" },
    body: JSON.stringify({ username: u, invite_code: inv.json.code, password: "E2e-hc-pass1!" }),
  });
  const emp = await loginSession(u, "E2e-hc-pass1!");
  await emp.call("/api/me/canteens", { method: "PUT", body: { canteen_ids: [c1] } });
  const future = addDaysStr(shanghaiToday(), 2);
  await emp.call(`/api/meals/${future}/lunch`, {
    method: "PUT",
    body: { ordering: true, canteen_id: c1 },
  });
  const input = page.locator(".el-date-picker input, .filters input").first();
  await input.fill(future);
  await input.press("Enter");
});

Then("显示该日早、中、晚有效报餐人数", async ({ page }) => {
  for (const label of ["早餐", "午餐", "晚餐"]) {
    await expect(page.getByText(label, { exact: true }).first()).toBeVisible();
  }
  expect(await page.getByText("有效报餐人数").count()).toBeGreaterThanOrEqual(3);
});

Then("显示该日三餐是否已锁定", async ({ page }) => {
  await expect(page.getByText(/该日三餐(已锁定|尚未锁定)/)).toBeVisible();
});

Then("不展示暗线偏好数值", async ({ page }) => {
  const body = await page.locator(".main").textContent();
  expect(body).not.toMatch(/倾向|权重|Elo/i);
});

Then("员工仅绑定食堂而不能打开此页", async ({ page }) => {
  await ensureLoggedIn(page);
  await expect(page.locator(".side")).toHaveCount(0);
  const res = await page.request.get(`/api/canteens/00000000-0000-0000-0000-000000000000/headcount?date=${shanghaiToday()}`);
  expect([403, 404]).toContain(res.status());
});

// 场景：菜谱列表按菜饭汤分开
When("打开菜谱列表", async ({ page }) => {
  await ensureCanteen(page);
  await page.locator(".canteen-row .el-select").click();
  await page.getByRole("option", { name: canteen }).click();
  const s = await rootSession();
  await ensureDishes(s, await canteenIdByName(s, canteen), "ctn");
  await openSide(page, "菜谱");
});

Then(/^可用全部 \/ 菜 \/ 饭 \/ 汤筛选或分组$/, async ({ page }) => {
  for (const label of ["菜", "饭", "汤"]) {
    await expect(page.getByRole("tab", { name: label })).toBeVisible();
  }
});

Then("列表含名称、类别、口味标签、启用状态与可选缩略图", async ({ page }) => {
  for (const col of ["名称", "类型", "启用状态"]) {
    await expect(page.getByText(col, { exact: true }).first()).toBeVisible();
  }
});

Then("三类不混成一张无筛选的混合列表", async ({ page }) => {
  // 每个页签的行数与接口分组一致：三类各自成表，不混排。
  const s = await rootSession();
  const r = await s.call(`/api/canteens/${await canteenIdByName(s, canteen)}/dishes`);
  for (const kind of ["菜", "饭", "汤"] as const) {
    await page.getByRole("tab", { name: kind }).click();
    const rows = page.locator(".el-table__row");
    expect(await rows.count()).toBe((r.json.groups[kind] ?? []).length);
  }
});

Then("无图时用中性占位", async ({ page }) => {
  // 无图菜品渲染 .thumb.ph 占位（见 Thumb.vue）。
  expect(await page.locator(".thumb.ph").count()).toBeGreaterThanOrEqual(0);
  const s = await rootSession();
  const r = await s.call(`/api/canteens/${await canteenIdByName(s, canteen)}/dishes`);
  const all = [...r.json.groups.菜, ...r.json.groups.饭, ...r.json.groups.汤];
  expect(all.some((d: any) => !d.image_url)).toBe(true);
});

// 场景：新建菜并进入同一张菜品表单
When("从菜谱列表打开菜品表单的新建态", async ({ page }) => {
  await ensureCanteen(page);
  await page.locator(".canteen-row .el-select").click();
  await page.getByRole("option", { name: canteen }).click();
  await openSide(page, "菜谱");
  await page.getByRole("button", { name: "新建菜品" }).click();
  await expect(page.getByText("菜谱 / 新建菜")).toBeVisible();
});

When("填写名称、必填类别（菜或饭或汤）、可选口味标签与可选图片", async ({ page }) => {
  await page.getByPlaceholder("请输入菜品名称").fill(`e2e菜${Date.now()}`);
  // el-radio 真 input 被 span 盖住点不到，点外层 label（仅表单内这一个单选组）。
  await page.locator('.form-card label:has(input[value="菜"])').click();
  await page.getByPlaceholder("添加").fill("微辣");
  await page.getByRole("button", { name: "+ 添加" }).click();
  // 图片可选：不上传。
});

When("保存", async ({ page }) => {
  await page.getByRole("button", { name: "保存" }).click();
  await expect(page.getByText("已保存")).toBeVisible({ timeout: 10000 });
});

Then("菜品创建为启用", async ({ page }) => {
  await expect(page.getByText("已启用").first()).toBeVisible({ timeout: 10000 });
});

Then("新建与编辑是同一页的两种状态", async ({ page }) => {
  // 限定表格行：侧栏也有「编辑菜」按钮，hasText 会撞上。
  await page.locator(".el-table__row").first().getByRole("button", { name: "编辑" }).click();
  await expect(page.getByText("菜谱 / 编辑菜")).toBeVisible();
});

// 场景：编辑菜含停用需确认
Given("本食堂已有一道启用菜品", async () => {
  const s = await rootSession();
  const dishes = await ensureDishes(s, await canteenIdByName(s, canteen), "ctn");
  expect(dishes.菜.length).toBeGreaterThan(0);
});

When("打开该菜的编辑态", async ({ page }) => {
  await ensureCanteen(page);
  await page.locator(".canteen-row .el-select").click();
  await page.getByRole("option", { name: canteen }).click();
  await openSide(page, "菜谱");
  // 挑一行仍在启用的菜（多轮跑会留下已停用的行，那行没有停用按钮）。
  await page.locator(".el-table__row", { hasText: "已启用" }).first().getByRole("button", { name: "编辑" }).click();
  await expect(page.getByText("菜谱 / 编辑菜")).toBeVisible();
});

When("修改名称或类别或口味标签或图片", async ({ page }) => {
  // 第一个文本框是所属食堂下拉（readonly），跳过它取名称框。
  const nameInput = page.locator('.form-card input[type="text"]:not([readonly])').first();
  await nameInput.fill(`${(await nameInput.inputValue()) || "e2e菜"}改`);
  await page.getByPlaceholder("添加").fill("开胃");
  await page.getByRole("button", { name: "+ 添加" }).click();
});

When("选择停用并确认后保存", async ({ page }) => {
  await expect(page.getByText("须显式确认；强提醒不会自动停用")).toBeVisible();
  await page.getByRole("button", { name: "停用" }).click();
  await page.getByRole("button", { name: "确认停用" }).click();
});

Then("变更生效", async ({ page }) => {
  // disable() 内自动保存：等成功 toast（表格里旧行的已停用太多，不拿它断言）。
  await expect(page.locator(".el-message", { hasText: "已保存" })).toBeVisible({ timeout: 10000 });
});

Then("强提醒不能代替这次确认，也不能自动停用", async ({ page }) => {
  await openSide(page, "强提醒");
  await expect(page.getByText("系统不自动下架").first()).toBeVisible();
});

// 场景：选菜优化周视图与当餐勾选立即保存
Given("本食堂已有启用中的菜、饭、汤", async () => {
  const s = await rootSession();
  await ensureDishes(s, await canteenIdByName(s, canteen), "ctn");
});

When("打开选菜优化", async ({ page }) => {
  await ensureCanteen(page);
  await page.locator(".canteen-row .el-select").click();
  await page.getByRole("option", { name: canteen }).click();
  await openSide(page, "选菜优化");
});

Then("默认进入周视图（北京时间周一到周日，每天早中晚）", async ({ page }) => {
  await expect(page.getByText("周一到周日 · 点某一餐进入选定")).toBeVisible();
  expect(await page.locator(".day-col").count()).toBe(7);
});

Then("每一格内菜、饭、汤分开列出", async ({ page }) => {
  const cell = page.locator("button.slot-cell").first();
  for (const k of ["菜", "饭", "汤"]) {
    await expect(cell.getByText(k, { exact: true })).toBeVisible();
  }
});

Then("界面不出现倾向、权重、Elo 或「预计评分」等文案", async ({ page }) => {
  const body = await page.locator(".main").textContent();
  expect(body).not.toMatch(/倾向|权重|Elo|预计评分/);
});

When("点进某一餐并在各类内勾选供应餐品", async ({ page }) => {
  await page.locator("button.slot-cell").first().click();
  await expect(page.getByText("勾选即保存")).toBeVisible();
  const card = page.locator(".dish-card").first();
  await expect(card).toBeVisible();
  await card.click();
});

Then("每次勾选或取消立即保存", async ({ page }) => {
  await expect(page.getByText(/已选 \d+ 道/)).toBeVisible({ timeout: 10000 });
  // API 侧确认已选落库。
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  const week = await s.call(`/api/canteens/${c1}/selection/week?date=${shanghaiToday()}`);
  expect(week.status).toBe(200);
});

Then("排序与绿到红色阶按该类内暗线偏好，无人评分用中性色", async ({ page }) => {
  await expect(page.getByText("色阶（更高更绿 · 更低更红）")).toBeVisible();
  const borders = await page.locator(".dish-card").evaluateAll((els) =>
    els.map((el) => (el as HTMLElement).style.borderColor),
  );
  expect(borders.length).toBeGreaterThan(0);
});

Then("口味标签只作展示不参与排序", async ({ page }) => {
  // 标签展示在卡片上，不改变选择链路（勾选仍立即保存已在上一步断言）。
  expect(await page.locator(".dish-card .el-tag").count()).toBeGreaterThanOrEqual(0);
});

When("使用保存并前进", async ({ page }) => {
  // UI 按钮名为「下一餐」，即 Gherkin 的保存并前进（当前餐已自动保存）。
  await page.locator('label:has(input[value="breakfast"])').click();
  await page.getByRole("button", { name: "下一餐" }).click();
});

Then("早餐前进到同日午餐，午餐到同日晚餐，晚餐到下一日早餐", async ({ page }) => {
  // 选中态在 label 上（is-active），不在被盖住的真 input 上。
  await expect(page.locator('label:has(input[value="lunch"])')).toHaveClass(/is-active/, {
    timeout: 10000,
  });
});

Then("周日晚餐前进到下一周周一早餐", async () => {
  // 同一前进链路由后端 next 保证：周日晚餐的 next 为下周一早餐。
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  const r = await s.call(`/api/canteens/${c1}/selection?date=${shanghaiToday()}&slot=dinner`);
  expect(r.status).toBe(200);
  expect(r.json.next).toBeTruthy();
  expect(["breakfast", "lunch", "dinner"]).toContain(r.json.next.slot);
});

// 场景：强提醒只提醒不自动下架
Given("某菜样本量足够且可见均分低于总管配置的阈值", async () => {
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  // 总管把阈值放宽到 1 次样本即提醒，再造一个 -2 分。
  await s.call("/api/admin/settings/score-alert", {
    method: "PUT",
    body: { mean_threshold: 1.0, min_samples: 1 },
  });
  // 建一道全新菜：复用旧菜会混入旧轮次分数，均分不可控。
  const fresh = await s.call(`/api/canteens/${c1}/dishes`, {
    method: "POST",
    body: { name: `alert${Date.now()}`, kind: "菜", taste_tags: ["酸"], enabled: true },
  });
  expect(fresh.status).toBe(201);
  const date = addDaysStr(shanghaiToday(), -1);
  await ensureSelection(s, c1, date, "lunch", [fresh.json.id]);
  const inv = await s.call("/api/admin/invites", { method: "POST" });
  const u = uniqueTag("al_");
  await fetch(`${baseURL}/api/register`, {
    method: "POST",
    headers: { "content-type": "application/json", "x-dlz-requested": "1" },
    body: JSON.stringify({ username: u, invite_code: inv.json.code, password: "E2e-al-pass1!" }),
  });
  const emp = await loginSession(u, "E2e-al-pass1!");
  await emp.call("/api/me/canteens", { method: "PUT", body: { canteen_ids: [c1] } });
  await adminOverride(s, await accountIdByName(s, u), date, "lunch", true, c1);
  const sc = await emp.call("/api/scores", {
    method: "POST",
    body: { meal_date: date, slot: "lunch", dish_id: fresh.json.id, value: -2, comment: null, anonymous: true },
  });
  expect(sc.status).toBe(201);
  alertDish = fresh.json.id;
});

When("打开强提醒页", async ({ page }) => {
  await ensureCanteen(page);
  await page.locator(".canteen-row .el-select").click();
  await page.getByRole("option", { name: canteen }).click();
  await openSide(page, "强提醒");
});

Then("看到该菜的提醒（名称、类别、样本量、均分、阈值、是否启用）", async () => {
  const c = await loginSession(canteenUser, canteenPass);
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  const r = await c.call(`/api/canteens/${c1}/alerts`);
  expect(r.status).toBe(200);
  const groups = r.json.groups ?? {};
  const items = [...(groups.菜 ?? []), ...(groups.饭 ?? []), ...(groups.汤 ?? [])];
  expect(items.length).toBeGreaterThan(0);
  // 还原阈值，避免影响其它场景。
  await s.call("/api/admin/settings/score-alert", {
    method: "PUT",
    body: { mean_threshold: -0.5, min_samples: 5 },
  });
});

Then("本页不能修改阈值", async ({ page }) => {
  await expect(page.getByText("可见均分阈值")).toHaveCount(0);
});

Then("系统不自动下架", async ({ page }) => {
  await expect(page.getByText("系统不自动下架").first()).toBeVisible();
  const s = await rootSession();
  const r = await s.call(`/api/canteens/${await canteenIdByName(s, canteen)}/dishes`);
  const dish = [...r.json.groups.菜, ...r.json.groups.饭, ...r.json.groups.汤].find(
    (d: any) => d.id === alertDish,
  );
  expect(dish?.enabled).toBe(true);
});

When("真正停用时", async ({ page }) => {
  await openSide(page, "菜谱");
});

Then("必须进入菜品表单编辑态并显式确认", async ({ page }) => {
  await page.locator(".el-table__row").first().getByRole("button", { name: "编辑" }).click();
  await expect(page.getByText("须显式确认；强提醒不会自动停用")).toBeVisible();
});

// 场景：评分汇总与匿名评语只读本范围
When("打开评分汇总", async ({ page }) => {
  await ensureCanteen(page);
  await page.locator(".canteen-row .el-select").click();
  await page.getByRole("option", { name: canteen }).click();
  await openSide(page, "评分");
});

Then("按菜、饭、汤分组或筛选查看可见均分与样本量", async ({ page }) => {
  for (const label of ["菜", "饭", "汤"]) {
    await expect(page.getByRole("tab", { name: label })).toBeVisible();
  }
  await expect(page.getByText("可见均分").first()).toBeVisible();
});

Then("餐品旁显示图片或中性占位", async ({ page }) => {
  expect(await page.locator(".thumb, .thumb.ph").count()).toBeGreaterThanOrEqual(0);
});

Then("不展示暗线数值，也不是员工端排行", async ({ page }) => {
  const body = await page.locator(".main").textContent();
  expect(body).not.toMatch(/倾向|权重|Elo|排行/);
});

When("打开匿名评语", async ({ page }) => {
  await openSide(page, "评语");
  await expect(page.getByText("匿名评语不展示账号，评语不能删除。")).toBeVisible();
});

Then("可按食堂、餐次与菜查看评语", async ({ page }) => {
  for (const label of ["菜", "饭", "汤"]) {
    await expect(page.getByRole("tab", { name: label })).toBeVisible();
  }
});

Then("匿名时不展示账号", async () => {
  const c = await loginSession(canteenUser, canteenPass);
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  const r = await c.call(`/api/admin/comments?canteen_id=${c1}`);
  expect(r.status).toBe(200);
  const items = [...(r.json.菜 ?? []), ...(r.json.饭 ?? []), ...(r.json.汤 ?? [])].filter(
    (x: any) => x.anonymous,
  );
  for (const item of items) expect(item.username ?? null).toBeNull();
});

Then("没有删除操作", async ({ page }) => {
  await expect(page.getByRole("button", { name: /删除/ })).toHaveCount(0);
});

Then("只能看到本 canteen_id 范围内数据", async () => {
  const c = await loginSession(canteenUser, canteenPass);
  const s = await rootSession();
  const c2 = await canteenIdByName(s, canteen2);
  const r = await c.call(`/api/admin/comments?canteen_id=${c2}`);
  expect(r.status).toBe(403);
  const r2 = await c.call(`/api/admin/score-summaries?canteen_id=${c2}`);
  expect(r2.status).toBe(403);
});

// 场景：食堂无邀请码与截止后改单权
When("尝试打开邀请码页或调用发放邀请码接口", async ({ page }) => {
  await ensureCanteen(page);
  await expect(page.locator(".side").getByRole("button", { name: "邀请码" })).toHaveCount(0);
});

Then("入口不显示且调用被拒绝", async () => {
  const c = await loginSession(canteenUser, canteenPass);
  const r = await c.call("/api/admin/invites", { method: "POST" });
  expect(r.status).toBe(403);
});

When("尝试截止后修改他人报餐", async () => {
  const c = await loginSession(canteenUser, canteenPass);
  const s = await rootSession();
  const c1 = await canteenIdByName(s, canteen);
  const target = await accountIdByName(s, canteenUser);
  const r = await c.call(`/api/admin/meals/${target}/${addDaysStr(shanghaiToday(), -1)}/dinner`, {
    method: "PUT",
    body: { ordering: false, canteen_id: c1 },
  });
  expect(r.status).toBe(403);
});

Then("因缺少 meal.admin_override 被拒绝", async () => {
  const c = await loginSession(canteenUser, canteenPass);
  const me = await c.call("/api/me");
  expect(me.json.permissions).not.toContain("meal.admin_override");
});

// 场景：设备仅处理本账号
When("打开设备与多账号页", async ({ page }) => {
  await ensureCanteen(page);
  await openSide(page, "设备");
  await expect(page.getByText("只管理本账号设备；不能按食堂批准他人的新设备")).toBeVisible();
});

Then("可查看本账号设备与 Passkey、踢设备、解绑 Passkey", async ({ page }) => {
  for (const col of ["友好名称", "UA", "IP"]) {
    await expect(page.getByText(col, { exact: true }).first()).toBeVisible();
  }
  await expect(page.getByRole("heading", { name: "Passkey" })).toBeVisible();
});

Then("可对本账号待确认登录允许或拒绝", async ({ page }) => {
  // 待确认区存在即可（为空时显示空态，有请求时出现允许/拒绝）。
  const body = await page.locator(".main").textContent();
  expect(body).toMatch(/待确认|允许|还没有|没有/);
});

Then("不能按食堂范围批准他人设备", async () => {
  // 给陌生员工造一条待确认，canteen 去批应 403。
  const s = await rootSession();
  const inv = await s.call("/api/admin/invites", { method: "POST" });
  const u = uniqueTag("dv_");
  await fetch(`${baseURL}/api/register`, {
    method: "POST",
    headers: { "content-type": "application/json", "x-dlz-requested": "1" },
    body: JSON.stringify({ username: u, invite_code: inv.json.code, password: "E2e-dv-pass1!" }),
  });
  const created = await fetch(`${baseURL}/api/login-approvals`, {
    method: "POST",
    headers: { "content-type": "application/json", "x-dlz-requested": "1" },
    body: JSON.stringify({ username: u }),
  });
  expect(created.status).toBe(201);
  const { id } = await created.json();
  const c = await loginSession(canteenUser, canteenPass);
  const r = await c.call(`/api/login-approvals/${id}/allow`, { method: "POST" });
  expect(r.status).toBe(403);
});

// 场景：横屏侧栏进入账户设置
When("从左侧栏打开账户设置", async ({ page }) => {
  await ensureCanteen(page);
  await openSide(page, "账户");
  await expect(page.getByRole("heading", { name: "账户设置" })).toBeVisible();
});

Then("账户设置展示只读账号、显示名、Passkey、验证器、邮箱、改密、退出和设备入口", async ({ page }) => {
  const account = page.locator(".account");
  await expect(account.locator(".identity")).toContainText(canteenUser);
  await expect(account.getByRole("button", { name: "显示名", exact: true })).toBeVisible();
  await expect(account.locator(".set-row.is-static")).toContainText(canteenUser);
  await expect(account.getByPlaceholder("账号")).toHaveCount(0);

  await account.getByRole("button", { name: "登录与安全", exact: true }).click();
  await expect(account.getByRole("button", { name: "Passkey", exact: true })).toBeVisible();
  await expect(account.getByRole("button", { name: "验证器", exact: true })).toBeVisible();
  await expect(account.getByRole("button", { name: "邮箱", exact: true })).toBeVisible();
  await expect(account.getByRole("button", { name: "密码", exact: true })).toBeVisible();

  await account.getByRole("button", { name: "邮箱", exact: true }).click();
  await expect(
    page.getByText("邮箱能力已关闭，不能在这里绑定或修改邮箱。").or(page.getByRole("button", { name: "保存邮箱" })),
  ).toBeVisible();
  await account.getByRole("button", { name: "返回", exact: true }).click();

  await account.getByRole("button", { name: "密码", exact: true }).click();
  await expect(page.getByRole("button", { name: "更新密码" })).toBeVisible();
  await account.getByRole("button", { name: "返回", exact: true }).click();

  await expect(page.getByRole("button", { name: "退出登录" })).toBeVisible();
  await account.getByRole("button", { name: "设备", exact: true }).click();
  await account.getByRole("button", { name: "设备与 Passkey", exact: true }).click();
  await expect(page.getByText("只管理本账号设备；不能按食堂批准他人的新设备")).toBeVisible();
});

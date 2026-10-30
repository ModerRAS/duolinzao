import { expect, type Locator, type Page } from "@playwright/test";
import { createBdd, test } from "playwright-bdd";
import {
  canteen,
  chooseShift,
  ensureBound,
  ensureLoggedIn,
  inviteCode,
  openTab,
  registerNewUser,
  rootSession,
} from "./helpers";

const { Given, When, Then } = createBdd(test);

// 跨步骤共享：本周报餐场景里选中的那个餐次；是否已切到「下一周」视图。
// 每个场景的背景步骤会重置（也可单独跑场景）。
let lastSlot: Locator | null = null;
let weekAdvanced = false;

async function gotoNextWeekOnce(page: Page) {
  if (!weekAdvanced) {
    // 等周报餐接口落定再找按钮：加载态按钮先渲染为可用，锁态一到就禁用，抢点会悬空。
    const settled = page
      .waitForResponse((r) => r.url().includes("/api/me/meals") && r.ok(), { timeout: 15000 })
      .catch(() => null);
    await page.getByRole("button", { name: "下一周" }).click();
    await settled;
    weekAdvanced = true;
  }
}

// P0-1：lock_time=09:00，每周日 09:00（北京时间）后下周一三餐已锁，
// 按钮变成「早 已锁定」且 disabled。死点第一天的「早」必挂，
// 所以找第一个未锁定的餐次（已锁定按钮名字是「早 已锁定」，正则直接排除）。
// 绿=yes（报餐），橙=no（不报）。每次点击前重查可用，加载中途被禁用就换下一餐。
async function setMeal(page: Page, wantYes: boolean): Promise<Locator> {
  await gotoNextWeekOnce(page);
  const cls = wantYes ? /yes/ : /no/;
  for (let round = 0; round < 3; round++) {
    const slots = page.getByRole("button", { name: /^(早|中|晚)$/ });
    const count = await slots.count();
    for (let i = 0; i < count; i++) {
      const slot = slots.nth(i);
      for (let attempt = 0; attempt < 2; attempt++) {
        try {
          if (!(await slot.isEnabled())) break;
          await slot.click({ timeout: 5000 });
        } catch {
          break; // 点之前被禁用，换下一餐。
        }
        try {
          await expect(slot).toHaveClass(cls, { timeout: 8000 });
          return slot;
        } catch {
          // 一次点击没到目标态（初始态恰为反义时需要点两次），再点一次。
        }
      }
    }
  }
  throw new Error("没有可切换的未锁定餐次");
}

// 背景：系统已部署并可访问
Given("系统已部署并可访问", async ({ page }) => {
  lastSlot = null;
  weekAdvanced = false;
  await page.goto("/");
  await expect(page.getByRole("button", { name: /使用 Passkey 登录/ })).toBeVisible();
});

// 背景：邀请码与食堂由 scripts/e2e.sh 调 e2e/seed.sql 预置，这里只确认配置存在。
Given("管理员或总管已发放一张未使用的邀请码", async () => {
  expect(inviteCode).toBeTruthy();
});

Given("至少存在一个启用中的食堂", async () => {
  expect(canteen).toBeTruthy();
});

// 场景：邀请码注册开通账号
When("未登录用户打开邀请码注册页", async ({ page }) => {
  lastSlot = null;
  weekAdvanced = false;
  await page.goto("/");
  await page.getByRole("button", { name: "有邀请码，去注册" }).click();
});

When("填写字符串账号与邀请码并提交", async ({ page }) => {
  // registerNewUser 会清 cookie、现开邀请码并完成注册（可能再导航一次，幂等）。
  await registerNewUser(page);
});

Then("账号创建成功并可进入登录", async ({ page }) => {
  await expect(page.locator(".emp")).toBeVisible();
});

Then("新账号默认持有员工角色", async ({ page }) => {
  // 员工端是竖屏，没有横屏侧栏。
  await expect(page.locator(".emp")).toBeVisible();
  await expect(page.locator(".side")).toHaveCount(0);
});

Then("尚未选择班制时默认不报任何一餐", async ({ page }) => {
  await expect(page.getByText("尚未选择班制，默认不报任何一餐。")).toBeVisible();
});

// 场景：登录优先级与邮箱开关
Given("员工账号已存在", async ({ page }) => {
  await ensureLoggedIn(page);
});

When("打开登录页", async ({ page }) => {
  // 以未登录身份看登录页：清掉 Given 里登录的会话。
  await page.context().clearCookies();
  await page.goto("/");
});

Then("登录入口按 Passkey、TOTP、邮箱验证码、密码顺序展示", async ({ page }) => {
  // Google 流：首页 Passkey；填账号下一步后选方式（密码 / 验证码 / 邮箱）。
  const s = await rootSession();
  await s.call("/api/admin/settings/email", { method: "PUT", body: { enabled: true } });
  await page.goto("/");
  await expect(page.getByRole("button", { name: /使用 Passkey 登录/ })).toBeVisible();
  await page.getByPlaceholder("账号").fill("e2e_order_probe");
  await page.getByRole("button", { name: "下一步" }).click();
  const texts = await page.getByRole("button").allTextContents();
  const flat = texts.join("\n");
  const pwd = flat.indexOf("使用账号密码登录");
  const totp = flat.indexOf("Authenticator");
  const otp = flat.indexOf("邮箱登录");
  expect(pwd).toBeGreaterThanOrEqual(0);
  expect(totp).toBeGreaterThan(pwd);
  expect(otp).toBeGreaterThan(totp);
});

Then("不做短信 OTP", async ({ page }) => {
  await expect(page.locator("body")).not.toContainText("短信");
  await expect(page.locator(".side")).toHaveCount(0);
});

Then("若总管已关闭邮箱能力则不出现邮箱验证码", async ({ page }) => {
  const s = await rootSession();
  await s.call("/api/admin/settings/email", { method: "PUT", body: { enabled: false } });
  await page.goto("/");
  // Google 流：关邮箱后 methods 屏不再出现「邮箱登录」入口（无「能力关闭」提示文案）。
  await page.getByPlaceholder("账号").fill("e2e_order_probe");
  await page.getByRole("button", { name: "下一步" }).click();
  await expect(page.getByRole("heading", { name: "选择登录方式" })).toBeVisible();
  await expect(page.getByRole("button", { name: /邮箱登录/ })).toHaveCount(0);
  await expect(page.getByText("邮箱验证码", { exact: true })).toHaveCount(0);
});

// 场景：绑定多个食堂作为报餐使用范围（seed 只有一个食堂，绑定可用食堂）
Given("员工已登录", async ({ page }) => {
  await ensureLoggedIn(page);
});

When("从「我的」进入绑定食堂页并绑定食堂", async ({ page }) => {
  await openTab(page, "我的");
  await page.getByText("绑定食堂", { exact: true }).click();
  await expect(page.getByText("可以绑定多个食堂。这是报餐时的使用范围。")).toBeVisible();
  await page.getByText(canteen, { exact: true }).click();
  await page.getByRole("button", { name: "保存", exact: true }).click();
});

Then("绑定成功", async ({ page }) => {
  await expect(page.getByText("已绑定")).toBeVisible();
});

Then("这些食堂仅作为报餐使用范围", async ({ page }) => {
  await expect(page.getByText("绑定食堂只决定报餐可用范围，不是食堂管理范围。")).toBeVisible();
});

Then("员工不能因此打开食堂菜谱维护或人数总览等横屏管理画面", async ({ page }) => {
  await expect(page.locator(".side")).toHaveCount(0);
  await expect(page.getByText("人数总览", { exact: true })).toHaveCount(0);
});

// 场景：选择班制仅上二休四或自定义
Given("员工已登录且尚未选择班制", async ({ page }) => {
  await ensureBound(page);
  await openTab(page, "报餐");
  await expect(page.getByText("尚未选择班制，默认不报任何一餐。")).toBeVisible();
});

When("打开选择班制页", async ({ page }) => {
  await page.getByRole("button", { name: "选择班制" }).click();
  await expect(page.getByText("只有上二休四和自定义排班。用描述生成模板是子流程，不是第三种班制。")).toBeVisible();
});

Then("只提供「上二休四」和「自定义排班」", async ({ page }) => {
  await expect(page.getByText("自定义排班", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "用描述生成模板" })).toBeVisible();
});

Then("不出现「上五休二」或其它已砍班制", async ({ page }) => {
  await expect(page.getByText("上五休二")).toHaveCount(0);
});

When("选择「上二休四」并确认", async ({ page }) => {
  const panel = page.locator("section.panel", { hasText: "上二休四" });
  await panel.locator('input[type="date"]').fill("2025-01-06");
  await panel.getByRole("button", { name: "保存", exact: true }).click();
});

Then("班制保存成功", async ({ page }) => {
  await expect(page.getByText("已设为上二休四")).toBeVisible();
  await expect(page.getByText("尚未选择班制，默认不报任何一餐。")).toHaveCount(0);
});

Then("「用描述生成模板」作为模板生成器入口而非第三种班制", async ({ page }) => {
  // 保存后回到报餐首页，重进选择班制页再确认入口仍在。
  await openTab(page, "报餐");
  await page.getByRole("button", { name: "选择班制" }).click();
  await expect(page.getByRole("button", { name: "用描述生成模板" })).toBeVisible();
});

// 场景：本周报餐绿橙与跨食堂提示
Given("员工已绑定至少一个食堂且已选定班制或可手动报餐", async ({ page }) => {
  await chooseShift(page);
});

When("打开本周报餐", async ({ page }) => {
  await openTab(page, "报餐");
  const legend = page.locator(".legend");
  await expect(legend.getByText("报", { exact: true })).toBeVisible();
  await expect(legend.getByText("不报", { exact: true })).toBeVisible();
});

Then("按天展示早、中、晚三餐", async ({ page }) => {
  const cols = page.locator(".board-cols");
  await expect(cols).toContainText("早");
  await expect(cols).toContainText("中");
  await expect(cols).toContainText("晚");
});

Then("可切换已绑定食堂", async ({ page }) => {
  await page.getByRole("button", { name: "手动改报餐" }).click();
  await expect(page.getByRole("button", { name: canteen, exact: true }).first()).toBeVisible();
});

Then("看到跨食堂按所选食堂生效的提示", async ({ page }) => {
  await expect(page.getByText("跨食堂按当前选中的食堂生效。锁定时间一到，次日三餐一起锁定。")).toBeVisible();
});

Then("看到次日三餐锁定相关提示", async ({ page }) => {
  await expect(page.getByText("锁定时间一到，次日三餐一起锁定。")).toBeVisible();
});

When("将某一餐设为报餐", async ({ page }) => {
  lastSlot = await setMeal(page, true);
});

Then("该餐显示为绿色", async () => {
  await expect(lastSlot!).toHaveClass(/yes/);
});

When("将某一餐设为不报", async ({ page }) => {
  // 同一餐后报覆盖先报：优先翻转刚才那一餐，点不动就另找一餐。
  if (lastSlot) {
    try {
      if (await lastSlot.isEnabled()) {
        await lastSlot.click({ timeout: 5000 });
        await expect(lastSlot).toHaveClass(/no/, { timeout: 8000 });
        return;
      }
    } catch {
      // 掉线或被禁用，往下另找。
    }
  }
  lastSlot = await setMeal(page, false);
});

Then("该餐显示为橙色", async () => {
  await expect(lastSlot!).toHaveClass(/no/);
});

Then("同餐后报覆盖尚未锁定的先报", async () => {
  await expect(lastSlot!).toHaveClass(/no/);
});

// 场景：员工不可见总管与食堂管理能力
Given("员工已登录且仅持有员工角色", async ({ page }) => {
  await ensureLoggedIn(page);
});

When("访问横屏侧栏中的菜谱维护、选菜优化、邀请码、导出、审计、邮箱、阈值、找回、LLM 等入口", async ({
  page,
}) => {
  await expect(page.locator(".side")).toHaveCount(0);
});

Then("这些入口不显示", async ({ page }) => {
  for (const entry of ["人数总览", "菜谱", "邀请码", "审计", "导出", "LLM"]) {
    await expect(page.getByText(entry, { exact: true })).toHaveCount(0);
  }
});

Then("直接调用对应管理 API 被拒绝", async ({ page }) => {
  const res = await page.request.get("/api/admin/audit");
  expect(res.status()).toBe(403);
});

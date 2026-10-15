# 多邻灶原生客户端设计（Android）

状态：**已拍板**（确认前仍不写业务代码）  
范围：与一期 Web 共用同一 Axum API；**本期只做 Android**；自托管站点分发，不上应用商店  
权威需求：[一期需求](../requirements-phase1.md)  
UI 参考：[docs/ui](../ui/README.md)

**已拍板**

| 项 | 决定 |
|----|------|
| 平台 | **只做 Android**（Kotlin + Jetpack Compose） |
| 鸿蒙 | **暂缓 / 本期忽略**；不建 `clients/harmony/`，不排期纯血 HarmonyOS NEXT |
| 第一版范围 | **只做员工端（竖屏）**，按**完整功能**交付：登录 / 邀请注册 / 报餐 / 班制 / 我的食堂（可改）/ 评分 / 站内通知 / 设备与 Passkey / 新设备确认 / 应用内更新 |
| App 地位 | 与 Web **平级的一等公民**：App 内可绑定 Passkey；邀请注册时同步绑定 Passkey |
| 工作台 | 食堂 / 管理员 / 总管工作台**不做横屏**；日后另行设计为**竖屏工作台**，届时补充本文 |
| 登录方式 | 与 Web **同步**：Passkey / **认证器 TOTP** / **邮箱 OTP**（按站点开关）/ 密码，Passkey 默认优先；邮箱登录**仅**验证码，**不再**使用 magic link / App Links；邮箱 OTP 有效 **10 分钟**，一次性靠**记下已用 TOTP 时间窗（step/timestamp）**——同窗再验拒绝；旧 magic link **consume-on-use**；`email-otp/request` **一律 accepted**（无账号/邮箱不发信） |
| 账户设置 | 与 Web **同步**：绑定邮箱、TOTP 设置等 |
| 注册 | `/api/register` 成功即返回登录会话（**需改服务端**，§4.5）；Web 与 App 相同：随后进入 Passkey 绑定（可暂时跳过 / 不支持自动跳过）；**不走**新设备确认 |
| 改包方式 | 修改**已构建的通用 APK**（替换 `assets/server.json` → zipalign → apksigner 用部署方 keystore 重签 → 生成 `version.json`），**不从源码重编**；`versionCode` / `versionName` **沿用通用 APK 原值**，部署方不覆盖 |
| 最低版本 | **minSdk 30（Android 11）**，理由见 §8.4 |
| Passkey | 使用 **Android Credential Manager / 平台 API**，**不用 WebView 套 WebAuthn 页** |
| 登录优先级 | **默认 Passkey**；无可用凭据 / 取消 / 失败后再密码（备用入口可手动切密码） |
| 分发 | **只从各部署自己的服务器分发**：部署方改包内 Base URL 后重签，经站点 `/app/` 下发；**不用 GitHub Releases，暂不上应用商店** |
| 应用内更新 | 第一版即包含：从自家服务器同源拉 `{baseUrl}/app/version.json`；服务端配套支持（§4.4） |
| 卓易通 | 不作为交付目标，但保持兼容：核心流程不依赖仅 GMS 才有的能力（§8.2） |

---

## 0. 目标与非目标

### 0.1 目标

1. 提供与 Web **同一后端**、与 Web 平级的 Android 客户端；第一版完整覆盖员工端（竖屏）。食堂 / 管理工作台日后另行设计为竖屏。
2. **通用 APK**：包内可读服务端 Base URL；自托管方在发版流程里改配置并重新签名，用户从**自家站点**下载，连的就是自家 API。
3. 第一版：**登录默认走 Passkey**（平台 API）；无可用凭据、无凭据提供方 / GMS、用户取消或失败时自动回退密码；员工端功能完整（见 §2.2），权限与 Web 一致。
4. 开源仓库示例域名仅用 `https://example.com`，禁止私有部署信息入仓。

### 0.2 非目标（本期）

- **纯血鸿蒙 / HarmonyOS NEXT / ArkTS 独立工程**（暂缓，见 §8.1）。
- 上架 Google Play / 华为等应用商店（暂不做）。
- 通过 GitHub Releases 分发 APK（**不用**；只从各部署自己的服务器 `/app/` 分发）。
- 本期不做食堂 / 管理工作台（日后另行设计竖屏版）。
- 在 Swarm / CD 里强制编 APK（可后续接；设计先定产物与改包流程）。
- 另起原生专用 API；短信 OTP、支付、餐补。
- 用 WebView 加载站点来「假装」原生 Passkey。

---

## 1. 角色与用户逻辑

一期四个角色与 Web 相同。同一账号可持有多条角色分配；客户端按 `GET /api/me` 返回的 `roles` / `permissions` **显隐导航与拦截写操作**，不在本地发明权限。

| 角色 | 代码 | 客户端形态 | 核心用户逻辑 |
|------|------|------------|--------------|
| 员工 | `employee` | **竖屏** App 主壳 | 选班制 / 个人 DSL、本周报餐（绿报 / 橙不报）、绑定多个食堂（使用范围）、餐后评分、设备与 Passkey、站内通知 |
| 食堂 / 物业 | `canteen` | 工作台（**待日后竖屏设计**，第一版不做） | 人数总览、菜谱、选菜优化、强提醒、评分汇总、评语；范围限 `canteen_ids` |
| 管理员 | `admin` | 同上（待设计） | 邀请码、截止后改单、评分/评语（范围食堂）；侧栏按权限显隐 |
| 总管理 | `super_admin` | 同上（待设计） | 全局范围：邮箱开关、强提醒阈值、账号找回、LLM 渠道、审计、导出、建食堂与角色分配等 |

### 1.1 登录后路由规则

1. 调用 `GET /api/me`。
2. 第一版一律进入**员工竖屏** IA（持有工作台角色的账号也一样）。
3. 持有工作台角色账号的入口与切换，在下一阶段（竖屏工作台）处理。
4. 会话失效（401）：回登录并清本地 Cookie / 会话状态。

### 1.2 员工逻辑要点（与需求对齐）

- **新用户默认不报餐**：未选班制且未完成报餐前，不自动生成报餐。
- 班制仅 **上二休四** / **自定义**；「用描述生成模板」是模板生成器，不是第三种班制。
- 预览与本周报餐：**绿色 = 报，橙色 = 不报**；切换即写回。
- 绑定多食堂是**报餐使用范围**，不是管理范围。
- 截止后本人不可改；仅 `meal.admin_override`（及超管）可改。
- 员工端**不展示**菜品排行；评分 −2～+2，菜/饭/汤分开。

### 1.3 食堂 / 总管逻辑要点（供日后竖屏工作台设计参考）

- 布局（竖屏）待日后设计，本文暂不规定。
- 菜、饭、汤分组，禁止混成一条列表。
- 选菜优化：周视图 → 当餐选定；勾选立即保存；色阶表示暗线偏好，界面不展示偏好数值 / Elo。
- 强提醒不自动下架；停用须在菜品表单确认。

---

## 2. 信息架构（IA）

视觉参考见 `docs/ui/`，原生不要求像素级一致，**信息结构对齐**。

### 2.1 员工竖屏（底部三 Tab：报餐 | 评分 | 我的）

```
登录（**默认 Passkey**，密码备用；见 §3.2）/ 注册（邀请码 + 同步绑定 Passkey）
 └─ 员工壳
      ├─ 报餐
      │    ├─ 首页
      │    ├─ 选择班制 / 描述生成模板 / 模板预览
      │    └─ 本周报餐（按日早中晚；可切换已绑食堂）
      ├─ 评分
      └─ 我的
           ├─ 我的食堂（查看 / 修改绑定）
           ├─ 设备与 Passkey（绑定 / 解绑 Passkey、设备列表）
           ├─ 新设备确认（批准 / 拒绝其他设备登录）
           ├─ 站内通知
           ├─ 检查更新
           └─ 退出登录
```

### 2.2 第一版范围（仅 Android 员工端，完整功能）

1. **登录**：与 Web 同步（Passkey / 认证器 TOTP / 邮箱 OTP / 密码）；默认 Passkey（Credential Manager）；无可用凭据 / 无凭据提供方或 GMS / 取消 / 失败时自动回退密码（§3.2 / §3.2.4）
2. **邀请注册**：注册成功即登录，随后直接绑定 Passkey（§3.2.3）
2a. **账户设置**：与 Web 同步（绑定邮箱 `/api/me/email`、TOTP 设置 `/api/me/totp/*` 等）
3. **设备与 Passkey**：App 内绑定、解绑 Passkey；设备列表与移除（`/api/devices*`）
4. **新设备确认**：新设备登录走确认流程；App 内可批准 / 拒绝其他设备的登录请求（`/api/login-approvals*`）
5. **首页 / 本周报餐**：`/api/me/home`、`/api/me/meals`，切换报 / 不报（`PUT /api/meals/{date}/{slot}`）
6. **班制**：上二休四 / 自定义；描述生成模板、模板预览与应用（`/api/me/shift`、`/api/meal-templates*`）
7. **我的食堂**：查看并修改绑定（`GET/PUT /api/me/canteens`）
8. **评分**：餐后评分，菜 / 饭 / 汤分开（`/api/me/score-targets`、`POST /api/scores`）
9. **站内通知**：列表与已读（`/api/notifications*`）
10. **应用内更新**：从自家服务器同源拉 `version.json`（§4.4）
11. 通用：401 → 回登录；写请求带 `X-DLZ-Requested: 1`；Cookie 持久化；包内 `server.json` + inject 脚本 + 可选 `useMock`

### 2.3 工作台（日后另行设计，竖屏）

食堂 / 管理员 / 总管工作台**不做横屏**。日后另行设计**竖屏工作台**，届时补充本文（IA、导航、权限显隐、编码顺序）。第一版 APK 不含工作台。

---

## 3. 与 Web API 对齐

### 3.1 传输约定

| 项 | 约定 |
|----|------|
| Base URL | 包内配置（§4），启动时规范化 |
| 会话 | Cookie `dlz_session`；OkHttp CookieJar（或等价）持久化到应用私有存储 |
| CSRF | 所有 POST/PUT/PATCH/DELETE（含登录）带 **`X-DLZ-Requested: 1`** |
| JSON | `Content-Type: application/json`（除图片 multipart） |
| 错误 | 非 2xx + `{ error, message }`；业务 401 清会话回登录 |
| 时钟 | 日期 / 周视图按**北京时间** |

对照：`web/src/api.ts`、`src/routes.rs`。

### 3.2 登录与 Passkey（Android 平台 API）

全站方式优先级与 Web 一致：**Passkey → 认证器 TOTP → 邮箱 OTP（可关）→ 密码**。邮箱登录**只**发验证码，不再提供 magic link。

**登录流（已确认）：默认 Passkey；密码仅备用 / 回退。**

```
打开登录页
  ├─ 主路径：发起 Passkey（Credential Manager）
  │     ├─ 成功 → 建立会话（cookie）→ 进 App
  │     ├─ 无可用凭据 / 平台不支持 / 用户取消 / 断言失败
  │     │     → 自动切换到其他方式（密码为默认备用）
  ├─ 其他方式：认证器 TOTP（`POST /api/session/totp`）；邮箱 OTP（按站点开关，§3.2.4）；与 Web 一致；可用方式以 `GET /api/session/methods` 为准（邮箱开启时返回 `email_otp`，**不再**返回 `magic_link`）
  └─ 次要入口：用户主动点「使用密码」也可直接进密码表单（不强制先失败一次）
```


#### 3.2.1 Passkey（主路径，非 WebView）

后端 WebAuthn 风格接口（与 Web 共用）：

| 步骤 | 方法 | 路径 |
|------|------|------|
| 登录开始 | POST | `/api/session/passkey/start` |
| 登录完成 | POST | `/api/session/passkey/finish` |
| 绑定开始 | POST | `/api/passkeys/register/start`（需已登录） |
| 绑定完成 | POST | `/api/passkeys/register/finish` |
| 解绑 | DELETE | `/api/passkeys/:id` |

**Android 实现要求：**

1. 使用 **Credential Manager**（`androidx.credentials`）及系统 **publickey / passkey** 流程：服务端 challenge → 平台 API → 断言结果 POST 到 finish。
2. **禁止**用 WebView 打开站点登录页完成 Passkey。
3. RP ID / origin 与包内 `baseUrl` 的 host、服务端 WebAuthn 配置一致；自托管改包时三者同域。
4. 无可用凭据、设备无凭据提供方 / 无 GMS（如卓易通）、取消或失败时，**自动回退密码**（不阻塞登录）；不要静默重试死循环。
5. **绑定页**：「我的 → 设备与 Passkey」内可新增 Passkey（register start/finish）与解绑，与 Web 能力一致。
6. **新设备确认**（`/api/login-approvals*`）按需求 §3.1 完整实现：
   - 本机为新设备时：发起确认（`POST /api/login-approvals`），轮询状态（`GET /api/login-approvals/:id`）；
   - 本机为已登录设备时：查看待批准请求（`GET /api/login-approvals/pending`），批准 / 拒绝（`POST .../:id/allow`、`.../:id/deny`）。

#### 3.2.2 密码（备用 / 回退）

- 仅作备用：无 Passkey、用户选择密码、或 Passkey 失败后使用。
- `POST /api/session/password`，body `{ "username", "password" }` → Set-Cookie。
- 登录页可保留「使用密码」次要入口，但**默认主按钮 / 首屏动作是 Passkey**，不得把密码表单做成唯一默认主路径。

#### 3.2.3 邀请注册（同步绑定 Passkey；Web 与 App 相同）

Web 与 App 共用同一流程与同一服务端改动：

1. 填用户名、邀请码、密码 → `POST /api/register`（body `{ username, invite_code, password }`）。密码同时作为备用登录方式。
2. 注册成功即返回登录会话（Set-Cookie；**需改服务端**，见 §4.5），客户端直接进入 Passkey 绑定。
3. **不走新设备确认**（`/api/login-approvals*`）：注册自动登录时还没有其他可批准的设备。
4. 设备 / 浏览器支持 Passkey：以绑定 Passkey 为主 / 默认动作（`/api/passkeys/register/start` → 平台 API → `/finish`），同时提供可见的「暂时跳过」。
5. 不支持 Passkey：自动跳过此步。
6. 跳过的用户之后可在「设备与 Passkey」补绑。

#### 3.2.4 邮箱 OTP 登录（Web 与 App；取代 magic link）

邮箱能力开启时，**唯一**的邮箱登录方式是一次性验证码（OTP）。**不再**提供 magic link，也**不**为登录配置 App Links / `assetlinks.json`。

流程（Web 与 App 相同）：

1. 用户输入用户名 → `POST /api/session/email-otp/request` → 向已绑定邮箱发送验证码。
2. 用户填入验证码 → `POST /api/session/email-otp/verify` → 校验成功后建立会话（Set-Cookie）。
3. 邮箱能力关闭、未绑邮箱：不可用（与现有邮箱开关一致）。
4. **认证器 TOTP**（`/api/session/totp`）仍是独立登录方式，与邮箱 OTP 分开。

**验证码算法与窗口（已拍板默认）：**

| 项 | 决定 |
|----|------|
| 算法风格 | 与认证器 TOTP **同一套生成风格**（现实现：HMAC-SHA1、**6 位**数字；与 `totp_rs` / `/api/me/totp/*` 一致的码型） |
| 有效窗口 | **10 分钟**（邮箱 OTP 专用；认证器 TOTP 仍为其自身的 30s 步长，互不影响） |
| 重发间隔 | **≥ 60 秒**（未到间隔的重发请求拒绝） |
| 失败锁定 | 连续 **5 次**校验失败 → 锁定该账号的邮箱 OTP 校验 **15 分钟** |
| 一次性 | 校验成功后**持久化该码对应的 TOTP 时间窗（step / timestamp）**；之后同一已存时间窗再验一律拒绝（邮箱 OTP 与认证器 TOTP 同此规则）。算法本身不保证一次性，靠服务端记「已用时间窗」 |
| 防枚举 | `POST /api/session/email-otp/request` **一律返回 accepted**（与账号是否存在、是否已绑邮箱无关）；仅当账号存在且已绑邮箱时才发信；不存在则假装已发 |

说明：邮箱 OTP 的密钥 / 挑战由服务端按次签发并邮寄，不是用户认证器里那枚长期 secret；「同一算法风格」指码的生成与位数与认证器 TOTP 对齐，有效期与限流按上表。

**切换期残留的 magic link token（若仍有未消费记录）：** consume-on-use（用过即作废）。**不**再设更长的双轨接受窗口；邮箱登录以 OTP 为唯一方式（有效期仍 10 分钟）。邮箱 OTP / 认证器 TOTP 的防重放：只存已成功使用的时间窗，同窗拒绝。

**`GET /api/session/methods`：**

- 邮箱开启时返回 **`email_otp`**（以及既有 `passkey`、`totp`、`password` 等）。
- **不再**返回 `magic_link`。

**服务端现状与改动：** 现有 `/api/session/magic-link/request|consume` 废弃并移除（或改为不可用）；新增 `email-otp` 的 request / verify。详见 §4.5。一期需求中的「不做短信 OTP」不变——此处仅为**邮箱**验证码。

### 3.3 员工端 API（第一版必接）

| 能力 | 方法 | 路径 | 备注 |
|------|------|------|------|
| 当前用户 | GET | `/api/me` | |
| 首页 | GET | `/api/me/home` | |
| 区间报餐 | GET | `/api/me/meals?start=&end=` | |
| 改一餐 | PUT | `/api/meals/{date}/{slot}` | body: `{ ordering, canteen_id }`；`slot`: breakfast/lunch/dinner |
| 我的食堂 | GET/PUT | `/api/me/canteens` | 可查看、可修改 |
| 班制 | GET/PUT | `/api/me/shift` | |
| 模板 | GET/POST/PUT | `/api/me/meal-template`、`/api/meal-templates/generate`、`/api/meal-templates/:id`（`/slots`、`/apply`） | |
| 评分 | GET / POST | `/api/me/score-targets`、`/api/scores` | −2～+2，菜/饭/汤分开 |
| 通知 | GET / POST | `/api/notifications`、`/api/notifications/:id/read` | |
| 注册 | POST | `/api/register` | 成功即登录，见 §3.2.3 |
| 登录方式 | GET / POST | `/api/session/methods`（含 `email_otp`）、`/api/session/totp`、`/api/session/email-otp/request`、`/api/session/email-otp/verify`（**取代** magic-link） | 与 Web 同步；§3.2.4 / §4.5 |
| 账户设置 | PUT / POST | `/api/me/email`、`/api/me/totp/setup`、`/api/me/totp/confirm` | 与 Web 同步 |
| 设备 | GET / DELETE | `/api/devices`、`/api/devices/:id` | |
| 新设备确认 | POST / GET | `/api/login-approvals*` | 见 §3.2.1 |

### 3.4 食堂 / 管理 API（日后竖屏工作台使用）

与 Web 共用（`/api/canteens/...`、`/api/admin/...` 等）。无权限则隐藏入口；不新增权限码。

---

## 4. 包内服务端 URL 与「服务端改包」分发

### 4.1 配置形态

APK 内例如 `assets/server.json`：

```json
{
  "baseUrl": "https://example.com",
  "useMock": false
}
```

- 开源默认 **`https://example.com`**。
- 启动读取；非法 URL 则提示「安装包未绑定有效服务器」，不发起业务请求。
- **正式包不提供**让终端用户随意改 Base URL 的设置项（防钓鱼）；仅 debug / 开发者选项可例外。

### 4.2 服务端（部署方）改包再分发

目标：**谁家站点提供的 APK，就连谁家 API。**

推荐流程（由部署流水线或管理员在构建机执行，**不必**在业务容器里现场编 Android）：

1. 取得**已构建的通用 APK**（`baseUrl` 为 `https://example.com`）；**不从源码重编**。
2. 运行 `clients/android/scripts/inject-server-url.sh https://<自有域名> <apk>`：替换 APK 内 `assets/server.json`。
3. `zipalign` 对齐。
4. `apksigner` 使用**部署方自己的** keystore 重签（密钥不进 git）。
5. 生成 `version.json`：`versionCode` / `versionName` **从通用 APK 原样读取**（部署方不覆盖），并写入 `url`、`sha256`。
6. 将最终 APK 放到站点静态目录，例如：
   - `GET {baseUrl}/app/duolinzao.apk`
   - `GET {baseUrl}/app/version.json`：供应用内更新（§4.4），由同一 inject / 重签步骤写出

说明：

- 「服务端改包」指**部署方掌控的发版步骤**（脚本 + 对象存储 / 站点 `/app/`），不是让 Axum 进程在请求时动态改二进制。
- 业务镜像可只负责**托管** `/app/*.apk`；改包可在 CI（传入 `DLZ_DOMAIN` 一类**部署私有**变量，勿写入开源仓库）完成后再 `scp`/上传。
- 用户只从自己打开的站点下载，从而自然绑定该站点的 API。

### 4.3 只从服务端分发

- 不上架时依赖「未知来源安装」；文档说明风险。
- 应用内更新只请求**当前 baseUrl 同源**的 `version.json`。
- **不使用 GitHub Releases** 分发 APK；暂不上应用商店。

### 4.4 应用内更新（第一版包含）

客户端：

1. 启动时及「我的 → 检查更新」时请求 `GET {baseUrl}/app/version.json`（仅同源）。
2. 格式：`{ "versionCode", "versionName", "url", "sha256" }`；`url` 必须与 baseUrl 同源（一般为 `/app/duolinzao.apk`）。
3. `versionCode` 大于本机时提示更新；下载后校验 **sha256**、**包名**与**签名证书指纹**（与当前安装包一致）均通过才调起系统安装。
4. 任一校验失败则丢弃安装包并提示，不安装。

服务端需配套的工作：

1. 站点托管 `/app/` 静态目录：`/app/duolinzao.apk` 与 `/app/version.json`（可走现有静态文件服务或独立静态路由；需确保 `/app/*` 不被 SPA 回退到 `index.html`，缺失文件应返回 404）。
2. `version.json` 由 inject / 重签步骤生成：`versionCode` / `versionName` 从通用 APK 原样读取（部署方不覆盖），并写入 `url`、`sha256`。
3. 响应头：`version.json` 不缓存或短缓存；APK 使用正确的 `Content-Type`（`application/vnd.android.package-archive`）。
4. 部署文档说明 `/app/` 目录的挂载 / 上传方式（具体路径、域名属部署私有信息，不入仓）。

### 4.5 其他服务端改动（第一版必需）

1. **`POST /api/register` 成功即登录**：注册成功时建立会话并返回 Set-Cookie（`dlz_session`），与登录接口一致；Web 与 App 共用。注册自动登录**不**进入新设备确认流。
2. **注册后 Passkey 绑定步骤（Web 与 App）**：注册成功进绑定页；支持则默认绑定且可「暂时跳过」，不支持则自动跳过（产品流，主要改前端；服务端已有 passkey register 接口）。
3. **邮箱登录改为 OTP（取代 magic link）**：
   - 新增 `POST /api/session/email-otp/request` 与 `POST /api/session/email-otp/verify`（路径可微调，须 Web / App 共用）；参数见 §3.2.4（6 位、与认证器 TOTP 同算法风格、有效 10 分钟、重发 ≥60s、5 次失败锁 15 分钟）。
   - `GET /api/session/methods`：邮箱开启时返回 **`email_otp`**，**不再**返回 `magic_link`。
   - **废弃并移除**（或永久关闭）`/api/session/magic-link/request` 与 `/consume`；登录不再发含链接的邮件，也不再为登录准备 App Links / `assetlinks.json`。
   - **Web 登录页**与 App 一并改为「邮箱验证码」UI，去掉 magic link 入口。
   - **认证器 TOTP**（`/api/session/totp`、`/api/me/totp/*`）保持不变，仍是独立登录方式。
   - 仍**不做短信 OTP**。
   - **防枚举**：`email-otp/request` 一律返回 accepted；仅账号存在且已绑邮箱时发信。
   - **一次性**：校验成功后持久化该码对应的 TOTP 时间窗（step / timestamp）；之后对同一已存时间窗的 verify 一律拒绝。邮箱 OTP 有效期仍为 10 分钟。切换期残留 magic link token：consume-on-use，不另设双轨窗口。

---

## 5. 安全与隐私

1. 开源禁止：真实域名、节点名、密钥、内网 IP、私人邮箱等。
2. 本地：Cookie 与必要缓存；密码不落盘。
3. 证书钉扎默认关（自托管证书多样）；部署方可文档化开启方式。
4. 正式包日志不打印 token / cookie / 密码。
5. Passkey 私钥不出系统实现边界；应用只持有断言结果。

---

## 6. 分阶段交付（仅 Android 员工端）

| 阶段 | 内容 | 出口 |
|------|------|------|
| D0 | 本设计评审 | 确认后开工 |
| D1 | `clients/android` 骨架（minSdk 30）+ `server.json` + inject 脚本 + mock 开关；服务端 `/app/` 托管与 `version.json` 生成 | assembleDebug；站点可下载 APK |
| D2 | 登录（Passkey 优先；认证器 TOTP / 邮箱 OTP / 密码与 Web 同步）+ 邀请注册（服务端注册即登录，不走新设备确认）后绑定 Passkey + 账户设置 + 设备与 Passkey 页 + 新设备确认（含 App 内批准）；服务端以 email-otp 取代 magic-link | 演示环境跑通 |
| D3 | 首页 / 本周报餐 + 班制与模板 + 我的食堂（可改） | 报餐主路径对齐 Web |
| D4 | 评分 + 站内通知 + 应用内更新 | **第一版完整发布**（员工端） |
| 后续 | 竖屏工作台：另行设计后补充本文 | 待设计 |

---

## 7. 仓库布局（确认后编码）

```
clients/
  README.md
  android/                    # 唯一本期客户端工程
    app/src/main/assets/server.json
    scripts/inject-server-url.sh
    README.md
docs/clients/
  README.md
  design.md                   # 本文
```

- **不**创建 `clients/harmony/`（鸿蒙暂缓）。
- 根 README 指向本文；不把无 SDK 的强制 Android 编译加入会红的主 CI job。

---

## 8. 平台说明

### 8.1 鸿蒙：暂缓

本期**不做**纯血鸿蒙（HarmonyOS NEXT / ArkTS）客户端，不维护双端产品逻辑对照表，不占用排期。若未来恢复，另开设计修订，再议是否复用本 Android 的 IA / API 映射。

### 8.2 卓易通（可选兼容备注，非交付目标）

部分运行环境可通过**卓易通**等方案在鸿蒙设备上运行 Android 应用。这属于**兼容层 / 旁路运行**，不是原生鸿蒙工程：

- 社区实测（非官方）容器报 **Android 12 / API 31**；minSdk 30 可正常安装。
- **无完整 GMS**：Android 11–13 上 Passkey 通常依赖 GMS，因此卓易通内 **Passkey 大概率不可用**，默认走密码回退。
- 以上**未经验证**，须在真机上实测。
- 不作为交付目标、不纳入验收标准，但**保持兼容**：核心流程（登录、注册、报餐、更新）不得依赖仅 GMS 才有的能力（如 FCM 推送、Play 服务内更新等）。
- 若用户自行用卓易通安装本 APK，视为「Android 包在兼容层上的尽力而为」，问题优先在标准 Android 设备上复现。

### 8.3 Android 实现选型（摘要）

| 点 | 选择 |
|----|------|
| 语言 / UI | Kotlin + Jetpack Compose |
| 网络 | OkHttp（CookieJar）+ Retrofit 或等价 |
| Passkey | Credential Manager 等平台 API |
| 配置 | `assets/server.json` |
| 产物 | `.apk`；站点 `/app/duolinzao.apk` |
| minSdk | 30（Android 11） |

### 8.4 最低版本：Android 11（API 30）

覆盖率（minSdk 30 及以上）：

- Google Play 活跃设备约 **83%**（composables.com，2025-12）
- 全球流量约 **87%**（StatCounter，经 apilevels.com，2026-04）
- 中国大陆粗算约 **94%**（StatCounter；偏流量统计，且鸿蒙设备 UA 未必计入 Android，仅供参考）

为何不要求 Android 14：14 新增的主要是**系统级第三方凭据提供方**（可直接用 1Password、Bitwarden 等存的 Passkey）；11–13 上 Credential Manager 经 Google Play 服务即可使用 Passkey（基本限 Google 密码管理器）。属锦上添花，非必需；抬到 14 覆盖率将降到约一半。

---

## 9. 已确认

1. 只做 **Android**；鸿蒙暂缓。
2. 第一版 = **员工端（竖屏）完整功能**（§2.2）；工作台日后另行设计为竖屏。
3. 登录流：**默认 Passkey**；无可用凭据 / 无提供方或 GMS / 取消 / 失败自动回退密码；允许次要「使用密码」入口。
4. App 与 Web 平级：App 内可绑定 Passkey；邀请注册同步绑定 Passkey。
5. 新设备确认完整实现，含 App 内批准其他设备。
6. 正式包禁止用户改 Base URL；由部署方改已构建 APK（替换 server.json → zipalign → apksigner 重签 → 生成 version.json）。
7. 只从各部署自己的服务器 `/app/` 分发；不用 GitHub Releases，暂不上商店。
8. 应用内更新第一版即包含，服务端配套支持（§4.4）。
9. minSdk 30（Android 11）。
10. 卓易通不作交付目标，但核心流程不依赖仅 GMS 能力。
11. 登录方式与账户设置与 Web 同步；邮箱登录**仅** OTP（Web / App 相同），废除 magic link / App Links 登录；认证器 TOTP 仍独立保留。
12. 注册接口成功即登录（需改服务端）；Web 与 App 相同：随后 Passkey 绑定（可暂时跳过 / 不支持自动跳过）；注册自动登录不走新设备确认。
13. 持有工作台角色的账号：第一版进员工端，下一阶段（竖屏工作台）处理。
14. `version.json` 的 `versionCode` / `versionName` 沿用通用 APK，部署方不覆盖。
15. 邮箱 OTP（有效 10 分钟）/ 认证器 TOTP：校验成功后记下已用 TOTP 时间窗（step / timestamp），同窗再验拒绝；切换期残留 magic link token 为 consume-on-use。`email-otp/request` 一律 accepted（防枚举），无账号或未绑邮箱不发信。

---

## 10. 修订记录

| 日期 | 说明 |
|------|------|
| 2026-10-13 | 初稿：Android + 鸿蒙并列 |
| 2026-10-13 | 修订：只做 Android；鸿蒙暂缓；MVP 含 Passkey（Credential Manager，非 WebView）；强化服务端改包分发；卓易通仅可选备注 |
| 2026-10-13 | 修订：登录流确认——默认 Passkey，密码仅备用/回退 |
| 2026-10-14 | 修订：第一版为员工端完整功能（注册同步绑定 Passkey、App 内绑定页、新设备确认含 App 内批准、我的食堂可改、班制、评分、通知）；工作台改为日后竖屏设计；只从自家服务器分发（不用 GitHub Releases）；应用内更新进第一版并补服务端支持；minSdk 30 及覆盖率依据；卓易通兼容说明 |
| 2026-10-14 | 修订：改包定为修改已构建 APK（替换 server.json + zipalign + apksigner + 生成 version.json）；注册接口成功即登录（服务端改动）；注册时 Passkey 绑定可暂时跳过 / 不支持自动跳过；登录方式与账户设置与 Web 同步；工作台角色账号下一阶段处理 |
| 2026-10-15 | 修订：Web 注册与 App 同流程（绑定 Passkey 可跳过）；注册自动登录不走新设备确认；version.json 版本号沿用通用 APK |
| 2026-10-15 | 修订：邮箱登录改为**仅 OTP**（废除 magic link / App Links）；码型对齐认证器 TOTP、有效 10 分钟；重发 ≥60s、5 次失败锁 15 分钟；`methods` 返回 `email_otp`；Web 登录 UI 同步改 OTP |
| 2026-10-15 | 修订：邮箱 OTP 与切换期残留 magic token 均为用后即废（无更长双轨窗口）；email-otp/request 一律返回 accepted，仅存在且已绑邮箱才发信 |
| 2026-10-15 | 修订：一次性实现为记录已用 TOTP 时间窗（同窗拒绝），非仅依赖算法 |
| 2026-10-15 | 修订：明确一次性=持久化已用 TOTP 时间窗（step/timestamp），同窗 verify 拒绝；邮箱 OTP 有效期仍 10 分钟；magic link consume-on-use；request 一律 accepted |

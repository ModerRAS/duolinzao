# Android 交付方向变更：系统 WebView 壳 + PWA（2026-11-04）

状态：**已定方向**。PWA 与长会话已落地；WebView 壳待实现。本文替代
[`design.md`](./design.md) 中「原生 Compose 员工端 + 工作台」的 UI 交付方式；
账号、权限、API、分发、签名流程不变（仍共用同一 Axum API，仍从自家站点 `/app/` 分发）。

## 1. 为什么改

原生 Compose 端与 Web 端是两套 UI/交互，真机审计发现员工端 14 项不一致、食堂竖屏壳
13 项缺失（`audit/ui-alignment.md`），持续维护双端不划算。改为：

- **一套 UI**：Android 壳直接加载站点，Web 改一次两端同时生效；
- **PWA**：浏览器用户可安装、可离线打开应用壳；
- **长会话**：服务端 180 天会话 + 持久 Cookie，WebView 的 CookieManager 落盘，账号很久不用重登；
- **系统 Passkey**：WebView 通过 AndroidX WebKit 调用系统 Credential Manager，不走 WebView 里的假 WebAuthn 页面。

## 2. Passkey 在 WebView 里可行吗？——可行（有前提）

Android 官方文档《Authenticate users with WebView》说明：**Credential Manager 自
`androidx.webkit:webkit:1.12.0` 起原生支持 WebView**，要求：

1. 依赖 `androidx.credentials:credentials`（+ `credentials-play-services-auth`）与 `androidx.webkit:webkit`（建议 1.14+）；
2. App 与站点做 **Digital Asset Links**（站点 `/.well-known/assetlinks.json` 的
   `package_name=com.duolinzao.app` + APK 签名证书 SHA-256）。站点已按签名后的 APK
   自动生成该文件（`version.json` 的 `certSha256`），壳沿用同一包名/签名即可；
3. 运行时判断 `WebViewFeature.isFeatureSupported(WebViewFeature.WEB_AUTHENTICATION)`，
   支持时调用：

```kotlin
if (WebViewFeature.isFeatureSupported(WebViewFeature.WEB_AUTHENTICATION)) {
    WebSettingsCompat.setWebAuthenticationSupport(
        webView.settings,
        WebSettingsCompat.WEB_AUTHENTICATION_SUPPORT_FOR_APP,
    )
}
```

已知限制（官方明确）：

- WebKit 不支持 `mediation: "conditional"`。本项目 Web 端是显式「使用通行密钥登录」按钮，
  `navigator.credentials.get({publicKey})`，**未使用 conditional**，不受影响。
- 依赖系统 Credential Manager：**无 GMS 的设备可能没有可用凭据提供方**，必须保留密码/TOTP/邮箱 OTP 兜底。
- 旧 ROM 的 WebView 可能不支持 `WEB_AUTHENTICATION`：壳必须做能力检测，失败时回到密码登录（Web 端已有 `passkeyUnavailable` 回退）。

参考：
- <https://developer.android.com/identity/sign-in/credential-manager-webview>
- <https://developer.android.com/jetpack/androidx/releases/webkit#1.12.0>

## 3. 壳的职责（只做最小系统集成）

| 职责 | 做法 |
|---|---|
| 加载站点 | 从包内 `assets/server.json` 读 `baseUrl`（沿用现有 inject 改包流程），`loadUrl(baseUrl + "/")` |
| 同源限制 | 站内链接在 WebView 内；其它 `http(s)` 链接用 Custom Tabs / 系统浏览器打开（不继承会话） |
| Passkey | 见 §2；登录/注册/账户绑定都由站点页面发起 |
| Cookie 持久化 | `CookieManager.setAcceptCookie(true)`；不启用第三方 Cookie；`onPause` 时 `flush()`；持久 Cookie 由服务端 `Max-Age` 保证 |
| 文件选择 | `WebChromeClient.onShowFileChooser`（菜品图片/证件等上传） |
| 返回键 | 先 `webView.canGoBack()`；SPA 无路由时需 Web 侧补 `history.pushState`/`popstate`（见风险） |
| 下载/更新 | 站点 `/app/*.apk` 下载交给系统下载器（带 Cookie 头），或引导到「应用签名」页；不再内嵌原生更新器 |
| 权限 | 只在需要时处理 `onPermissionRequest`（相机/麦克风）；默认拒绝 |
| 安全 | 禁 `file://` 访问、禁 mixed content、不注入任何 JS 桥；域名白名单限定 baseUrl 主机 |
| 外观 | 全屏 WebView + 系统栏主题色；不需要原生导航栏（Web 自带侧栏/底栏） |

## 4. 会话持久化（已实现）

- 服务端：`SESSION_DAYS = 180`（`src/ops/mod.rs`）；登录下发的 `dlz_session` 现在是
  **持久 Cookie**：`HttpOnly; SameSite=Lax; Secure; Path=/; Max-Age=15552000`（`src/routes.rs::session_cookie`）。
  浏览器/WebView 进程退出后仍保留；吊销仍走「我的 → 设备与通行密钥 → 移除」或超管找回。
- WebView：CookieManager 默认把持久 Cookie 落盘；后台时 `flush()`。
- 可选增强（未做）：滑动续期（剩余 < 30 天时刷新 Cookie）、记住我开关；Chrome/WebView 对单个 Cookie 上限约 400 天。

## 5. PWA（已实现）

- `web/public/manifest.webmanifest`：名称/图标（192/512/maskable/Apple touch）/standalone/主题色 `#2D8A45`。
- `web/public/sw.js`：只缓存同源静态壳（导航网络优先，失败回退缓存的 `index.html`；`/assets/*` 缓存优先）；
  **不缓存 `/api/`、`/app/`、`/healthz`、`assetlinks`**。升级改 `CACHE` 版本号。
- `web/index.html`：manifest / theme-color / apple meta。
- `web/src/main.ts`：仅生产构建注册 SW（`import.meta.env.PROD`）。
- PWA 安装适用于浏览器（Chrome/Edge/桌面）；WebView 内不弹安装，但 SW 缓存仍生效。

## 6. 迁移步骤

1. **壳 MVP**：新 `MainActivity`（WebView）+ `androidx.webkit`，保留包名 `com.duolinzao.app`、`minSdk 30`、`assets/server.json`、CI 版本注入与 `/app/` 分发；开启 passkey、Cookie、文件选择、外链、返回。
2. **Web 补返回历史**：`App.vue` 的页面切换写 `history.pushState`，`popstate` 回到上一页（否则返回键直接退出）。
3. **真机回归**：登录（密码/passkey）、报餐、评分、食堂壳、图片上传、长会话（杀进程重开仍在登录态）；无 WebAuthn 的旧 WebView 走密码兜底。
4. **删除原生 UI**：`ui/` 下的 Compose 页面、`SmokeUiTest`、相关依赖；保留 `assets/server.json` 与签名/分发脚本。
5. **文档与 CI**：更新 `clients/android/README.md`、`design.md`（标注被本文替代）、CI 增加壳的 assemble/upgrade 验证；APK 仍由服务端签名发布。

## 7. 风险与待决

- **旧 WebView 碎片化**：`WEB_AUTHENTICATION` 不可用时 Passkey 按钮会失败；必须保证 Web 端回退密码且提示清楚。
- **无 GMS 设备**：Credential Manager 无凭据提供方；Passkey 不可用，密码/TOTP/邮箱 OTP 兜底。
- **SPA 返回键**：不补 history 会一按就退出；列为壳 MVP 的必做项。
- **PWA 更新**：SW 缓存可能让用户看到旧壳；导航网络优先 + 版本化 CACHE 缓解。
- **安全**：WebView 不加 JS 桥；外链不继承会话；`/app/` 仍要登录；Cookie HttpOnly/Secure。
- **TWA 备选**：若未来只面向有 Chrome 的设备，可换 Trusted Web Activity（Chrome 原生支持 Passkey/PWA 安装），代价是依赖 Chrome 与 `handle_all_urls` assetlinks，且不可定制 UI。

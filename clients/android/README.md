# 多邻灶 Android 员工端

员工竖屏完整功能（第一版）：登录（Passkey 默认 / TOTP / 邮箱 OTP / 密码兜底）、邀请注册后绑 Passkey、
账户设置、设备与 Passkey、新设备确认、首页与本周报餐、班制与模板、我的食堂、评分、通知、应用内更新。
食堂/管理横屏工作台本期不做（日后竖屏工作台另行设计）。

设计见 [`docs/clients/design.md`](../../docs/clients/design.md)，与 Web 共用同一 Axum API。

## 构建

前置：JDK 17+、Android SDK（含 platform 35、build-tools 35），设 `ANDROID_HOME`（或 `ANDROID_SDK_ROOT`），
或在 `clients/android/local.properties` 写 `sdk.dir`（该文件不进仓库）。

```bash
cd clients/android
./gradlew :app:assembleDebug        # 调试包（含 http 明文、开发设置页）
./gradlew :app:assembleRelease      # 通用包底版（未签名，部署前走 inject 脚本重签）
```

产物：`app/build/outputs/apk/{debug,release}/app-*.apk`。`versionCode=1` / `versionName=0.1.2`
与仓库基线（Cargo `0.1.2`）对齐；inject 脚本原样沿用这两个值，部署方不覆盖。

## 运行（模拟器 / 真机）

1. 启动 Android 模拟器（API ≥ 30）或连真机（`adb devices` 可见）。
2. 安装：`adb install -r app/build/outputs/apk/debug/app-debug.apk`。
3. 包内默认 `assets/server.json` 为 `{"baseUrl":"https://example.com","useMock":true}`：
   离线直接进 mock，可走完登录→三 Tab→报餐切换主路径。
4. 对接真后端：`useMock=false`（inject 脚本自动关），`baseUrl` 指向自家站点；
   模拟器连宿主机后端可用 `http://10.0.2.2:端口`（仅 debug 包允许明文 http，正式包只认 https 同源）；
   debug 包可在「我的 → 开发设置」里临时覆盖 Base URL，正式包无此入口。

## inject 改包（部署方在构建机执行，不在业务容器里现场编）

```bash
# 需要 unzip + zipalign + apksigner/apkanalyzer（或 aapt）+ python；Linux 部署机同样可用。
# KEYSTORE 等经环境传入，密钥不进仓库。
export ANDROID_HOME=/path/to/android-sdk
KEYSTORE=/path/to/site.jks KEYSTORE_PASS=... KEY_ALIAS=... KEY_PASS=... \
  scripts/inject-server-url.sh https://app.example.com universal.apk duolinzao.apk
# 产物：duolinzao.apk + 同目录 version.json（versionCode/Name 沿用通用包原值）
# 放到站点静态目录 {static_dir}/app/：/app/duolinzao.apk 与 /app/version.json
```

未设 `KEYSTORE` 时输出未签名包（仅供检查 `server.json`，发布前必须重签）。

## 应用内更新

App 只拉**当前 baseUrl 同源**的 `/app/version.json`
(`{versionCode, versionName, url, sha256}`)，`versionCode` 更大才提示；
下载后校验 sha256 + 包名（签名指纹尽力比对），任一失败丢弃。
服务端需保证 `/app/*` 缺文件返回 404（不能回退 index.html）。

## 真机验证记录（HK1_RBOX_K8，Android 13 / API 33，无 GMS）

- `./gradlew :app:assembleDebug` / `assembleRelease` 成功。
- 安装运行：登录页正常渲染（mock 下 methods 含 passkey/totp/email_otp）。
- 空账号点 Passkey：提示填账号并回退密码；有账号点 Passkey：无凭据提供方→
  「Passkey 不可用，已切换到密码登录」，不卡死（符合§3.2 无 GMS 回退）。
- mock 密码登录→员工壳三 Tab；本周报餐 10-21 早餐「不报→报餐」切换写回成功（橙→绿）。
- mock 邀请注册→直接进壳（不走新设备确认）；班制选择与模板生成预览正常。
- 我的页：食堂绑定/显示名/邮箱/改密入口、设备与 Passkey、新设备确认、通知、检查更新齐备。
- 已知限制：该电视盒的 `adb input tap` 对评分卡内小字按钮无响应（同构写入已由报餐切换验证），
  触屏真机复测；对接真后端（master #16 已齐备）待联调验证。

## 与服务端版本的对应

- `useMock=false` 时 App 调用的邮箱 OTP（`/api/session/email-otp/*`）与注册即登录（Set-Cookie）
  已在 master 落地（#16）；注册兼容旧服务端（无会话时自动回退一次密码登录），邮箱 OTP 入口按 methods 隐藏。
- 写请求统一带 `X-DLZ-Requested: 1`；会话走 `dlz_session` Cookie（应用私有存储持久化，密码不落盘）。

# 多邻灶 Android 员工端

员工竖屏完整功能（第一版）：登录（通行密钥默认 / TOTP / 邮箱 OTP / 密码兜底）、邀请注册后必须绑定通行密钥（设备不能创建时才设置密码）、
账户设置、设备与 Passkey、新设备确认、首页与本周报餐、班制与模板、我的食堂、评分、通知、应用内更新。
食堂/管理横屏工作台本期不做（日后竖屏工作台另行设计）。

设计见 [`docs/clients/design.md`](../../docs/clients/design.md)，与 Web 共用同一 Axum API。

## 构建

前置：JDK 17+、Android SDK（含 platform 35、build-tools 35），设 `ANDROID_HOME`（或 `ANDROID_SDK_ROOT`），
或在 `clients/android/local.properties` 写 `sdk.dir`（该文件不进仓库）。

```bash
cd clients/android
./gradlew :app:assembleDebug        # 调试包（含 http 明文、开发设置页）
./gradlew :app:assembleRelease      # 未签通用包（正式签名在服务端完成，见下）
```

产物：`app/build/outputs/apk/{debug,release}/app-*.apk`。版本号平时不用管：
CI 自动注入（`versionName` = Gitea 风格串 `scripts/ci-version.sh`，
`versionCode` = `git rev-list --count HEAD` 单调递增）；本地不传 Gradle 属性时
默认 `versionCode=1` / `versionName=0.1.2`。inject 脚本原样沿用构建值，部署方不覆盖。
`version.json` 另带 `signing` 标注（`release` 正式 keystore / `ephemeral` CI 一次性 /
`unsigned` 未签名），App 端只做展示，不参与更新判断。

## 运行（模拟器 / 真机）

1. 启动 Android 模拟器（API ≥ 30）或连真机（`adb devices` 可见）。
2. 安装：`adb install -r app/build/outputs/apk/debug/app-debug.apk`。
3. 包内默认 `assets/server.json` 为 `{"baseUrl":"https://example.com","useMock":true}`：
   离线直接进 mock，可走完登录→三 Tab→报餐切换主路径。
4. 对接真后端：`useMock=false`（inject 脚本自动关），`baseUrl` 指向自家站点；
   模拟器连宿主机后端可用 `http://10.0.2.2:端口`（仅 debug 包允许明文 http，正式包只认 https 同源）；
   debug 包可在「我的 → 开发设置」里临时覆盖 Base URL，正式包无此入口。

## UI 测试（Compose 冒烟，mock）

- 测试代码：`app/src/androidTest/java/com/duolinzao/app/SmokeUiTest.kt`，
  用 `createAndroidComposeRule<MainActivity>` 走密码兜底登录→断言底部三 Tab
  （`assets/server.json` 默认 `useMock:true`，不连真后端；不依赖 Passkey）。
- 本地：先起模拟器（API ≥ 30，`adb devices` 可见），再跑：

```bash
cd clients/android
./gradlew :app:connectedDebugAndroidTest   # 需模拟器/真机
./gradlew :app:assembleDebugAndroidTest    # 仅编译测试包，无需设备
```

- CI：在 `.github/workflows/android.yml` 的 `ui-test` job（API 30 google_apis 模拟器），
  与 `assemble` 并行、不进主 `ci.yml` 的 needs，不挡后端主 CI。

## inject 改包（CI 里执行，只改 baseUrl，不签名）

```bash
# 需要 unzip + zipalign + apkanalyzer（或 aapt）+ python。
# 签名材料只活在应用数据库，不走本脚本、不进 GitHub、不进部署机文件。
export ANDROID_HOME=/path/to/android-sdk
scripts/inject-server-url.sh https://app.example.com universal.apk duolinzao.apk
# 产物：duolinzao.apk（未签，zipalign 已做）+ 同目录 version.json
# （versionCode/Name 沿用通用包原值，signing 标 unsigned；CI 打进镜像，签名发布时服务端重写）
```

正式签名在服务端完成：超管在站点「应用签名」页把 keystore 存进数据库后点
「签名发布」，服务端用 `apksigner` 签当前包、重算 sha256 并写出正式 `version.json`
（`signing` 为 `release`）。密钥永不进仓库，见 `docs/deploy.md` §7。

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
- 真后端联调（staging，#16 之后）：邀请注册即登录、新旧密码登录、报餐切换、食堂绑定、
  上二休四（含锚点日）、评分提交（管理端汇总均分 2.0）、显示名、改密（保留当前会话）、
  邮箱 OTP 请求 accepted/错码 400、设备列表、通知空态、更新检查（站点缺 /app/version.json
  时红字提示而非崩溃）均通过；服务端双端核验一致。
- 已知限制：该电视盒 `adb input tap` 偶发点不上小按钮/底部按钮（登出按钮等；同类写入链路
  均已由服务端 state 证实），触屏真机复测；`PUT /shift` 需锚点日+已绑食堂已在 App 内补齐；
  站点 `/app/*` 缺文件回退 index.html（设计要求真 404）是服务端缺口，待服务端修。

## 与服务端版本的对应

- 邀请注册：`POST /api/register` 传账号和邀请码，密码可省略。成功后的会话用来绑定通行密钥；能创建时必须绑定成功才进入员工端。设备不支持、用户取消或创建失败时，用 `POST /api/me/password` 设置至少 8 位密码，不传当前密码。通行密钥成功则密码可为空。界面写「通行密钥」。上面真机记录里的「mock 邀请注册→直接进壳」是当时的结果，不是这条契约。
- 邮箱 OTP（`/api/session/email-otp/*`）与注册即发会话已在 master 落地。邮箱 OTP 入口按 methods 隐藏。TOTP 在账户设置里后绑，注册时不采集。
- 写请求统一带 `X-DLZ-Requested: 1`；会话走 `dlz_session` Cookie（应用私有存储持久化，密码不落盘）。

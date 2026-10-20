# 多邻灶（duolinzao）

部门内部报餐网页。仓库 / crate：`duolinzao`。

技术栈：**Vue3**（前端）+ **Rust Axum**（后端）。

Android 原生客户端设计见 [docs/clients/](docs/clients/README.md)；源码目录约定见 [clients/](clients/README.md)（鸿蒙暂缓；设计确认前不落业务代码）。

一期需求已锁定，见 [docs/requirements-phase1.md](docs/requirements-phase1.md)。

用户故事、技术方案与实施计划见 [specs/phase1/requirements.md](specs/phase1/requirements.md)。

界面参考见 [docs/ui/README.md](docs/ui/README.md)：员工竖屏，以及食堂横屏（左侧栏为主）。食堂端画面清单在一期需求第 11 节。

## 本地运行

1. 准备 PostgreSQL，并复制配置：

```bash
cp config.example.toml config.toml
```

2. 填掉所有 `REPLACE_ME`。截止钟点、三餐餐后时刻、均分阈值、样本量下限、图片目录、单张大小上限和本地加密密钥都没有代码默认值，缺一项进程会拒绝启动。邮箱能力默认关闭，此时可以不填 SMTP。`local_key` 是 32 字节密钥的 base64。

3. 启动：

```bash
export DATABASE_URL=postgres://duolinzao:duolinzao@127.0.0.1:5432/duolinzao
# 把 config.toml 里的 database.url 指到同一个库
cargo run
```

数据库迁移在进程启动时自动执行。也可以只用数据库容器：`docker compose up db`。业务镜像见 `Dockerfile`，Postgres 不打进业务镜像，餐品图片目录需要挂载。

## Docker / compose

```bash
cp config.docker.example.toml config.docker.toml   # 再填掉 REPLACE_ME
docker compose build            # 打镜像，名字与 CI 推的 GHCR 一致：ghcr.io/moderras/duolinzao
docker compose up               # Postgres 旁路 + app（静态前端由 app 自己出）
```

`config.docker.example.toml` 与 `config.example.toml` 相同，只把主机名和路径换成容器里的：`database.url` 用 `db`，`static_dir=/app/static`，餐品图片在 `duolinzao-images` 卷里（容器以 nobody 运行，镜像已把该目录的属主交给它）。只需要数据库时：`docker compose up db`。

生产 Cookie（Swarm + Traefik）：`server.cookie_secure` 保持 `true`。TLS 在 Traefik 终结、后端容器只听 HTTP 也没关系——浏览器只看自己访问的是不是 HTTPS，照样回送 Secure Cookie。仅本地 HTTP 直连开发或 e2e（`http://localhost` 明文）时才改为 `false`；缺省即 `true`。

写接口 CSRF 第二道闸：所有 POST/PUT/PATCH/DELETE 必须带请求头 `X-DLZ-Requested: 1`（前端 `api()`/`apiBlob()` 已统一带上），缺头返回 403 `csrf_required`。GET/HEAD/OPTIONS 不受影响；登录/注册等无会话端点同样要求该头，无豁免。

真实客户端 IP（Traefik 后面）：`server.trust_proxy` 缺省 `false`，直接取对端地址。生产在 Traefik 后面才配 `true`，此时只解析一层 `X-Forwarded-For` 首段 / `Forwarded` 首个 `for=`，用于登录限流与审计记 IP；开关关闭时绝不读这些头，防客户端伪造。Traefik 侧必须设置 `X-Forwarded-For`（`entryPoints.proxyProtocol` 或 `forwardedHeaders`），否则所有请求都会记成网关 IP，登录限流会误伤。`config.docker.example.toml` 默认 `true`，本地示例默认 `false`。

会话与找回：`POST /api/admin/recovery` 的 `reset_password` 成功后吊销该账号全部会话并作废未消费 magic link（旧 cookie 立刻 401）。自助改密是 `POST /api/me/password`：校验当前密码，保留当前会话，吊销其它会话，并作废未消费 magic link。`username` 只读；`PUT /api/me/display-name` 改显示名，`GET /api/me` 带上 `display_name`。`POST /api/session/logout` 吊销当前会话并清 cookie。TOTP 与 Passkey 登录和密码共用同一失败计数窗（用户键 + IP 键，15 分钟 20 次），自助改密的当前密码错误也计入该窗，失败也落库，超限同返 429 `login_rate_limited`。

LLM 出站收紧：`base_url` 只允许 `https`（`http` 仅放行回环 `localhost`/`127/8`/`::1` 测试例外），字面私网/链路本地/组播/文档地址一律拒绝，含凭据的 URL 拒绝；出站客户端最多跟 2 跳重定向。`PUT /api/admin/llm-channels` 的每次变更都写审计（记 kind + base_url + model，不记 key 明文）。

前端在 `web/`。开发时 Vite 把 `/api` 代理到 `127.0.0.1:8080`：

```bash
cd web && npm install && npm run dev
```

集成测试：

```bash
cargo test
```

测试会在本机 Postgres 上创建 `duolinzao_test`。默认走本机 peer 套接字 `/var/run/postgresql`，管理库连接可用 `DATABASE_URL` 指定；测试库本身用 `TEST_DATABASE_URL` 覆盖（CI 里没有套接字，走 `postgres://postgres:postgres@127.0.0.1:5432/duolinzao_test`）。

## 浏览器测试（Playwright）

`e2e/` 下是 Playwright 规格，对照根目录 `features/*.feature`（Gherkin 中文场景）覆盖员工主路径：登录入口顺序与无短信 OTP、邀请码注册、员工竖屏、绑定食堂、班制只有上二休四/自定义、本周报餐绿（报）/橙（不报）切换。

```bash
# 需要先有一个可用 Postgres；脚本会生成临时配置、起 app、预置食堂与邀请码，再跑 Playwright
make e2e                 # = web 构建 + cargo build + npx playwright install + bash scripts/e2e.sh
```

失败时截图 / trace / video 和 HTML 报告留在 `e2e/test-results`、`e2e/playwright-report`，后端日志在 `e2e/app.log`。对着 Vite dev server 调试时可设 `DLZ_BASE_URL=http://127.0.0.1:5173`（此时脚本不再自己起 app）。

## CI / 交付

`.github/workflows/ci.yml`：

- **每次 PR**、以及推 `master` / `v*` tag：先跑 `cargo test`（Postgres service）与 Playwright 浏览器测试；两者都过了才 build 镜像。
- **只有**推到 `master` 或 `v*` tag 才 push 到 GHCR：`ghcr.io/moderras/duolinzao`（镜像名必须全小写）。PR 只 build，绝不 push。
- 失败时上传 `playwright-report`（截图 / trace）与 `e2e-app-log` 产物。
- CI 只用测试用配置与 seed 数据，不调用任何外部 LLM（模板生成器的渠道在运行期由总管配置，测试不碰）。

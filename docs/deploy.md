# 多邻灶生产部署（Docker Swarm）

目标拓扑：两个节点，一个跑应用（role=app），一个跑 Postgres（role=db）；入口是挂 label 的 Traefik，外部网络名可配（默认 `traefik-public`）。发布方式：CI 在 master 测试全绿后构建镜像推 GHCR，再由 deploy job SSH 到 manager 做 `docker stack deploy`（start-first，失败自动 rollback）。**服务器上永远不 build。**

仓库是开源的，只放通用配置：域名、entrypoint、certresolver、外部网络名、部署路径全部变量化，见 `deploy/stack.yml` 头部注释。本文件不出现任何私有主机名/域名。

## 0. 与代码的关系

- 数据库迁移在**应用启动时自动运行**（`src/main.rs` 里 `sqlx::migrate!`），无需手工 `sqlx migrate run`；回滚镜像不会回滚迁移（迁移只向前，见第 5 节）。
- 健康探针分两层，别混：
  - `GET /healthz`（生产公网端口 8080，纯文本 `ok`）**只证明进程在听**，不查库、不登录、免 CSRF；Traefik/CI 的公网健康检查用它，`GET /api/health` 仍在。
  - `GET /readyz` **只挂管理回环端口** `server.admin_bind`（缺省 `127.0.0.1:8090`，可用 `DLZ_ADMIN_BIND` 或 config 的 `[server] admin_bind` 覆盖）：真查库（`SELECT 1`，2s 短超时；DB 断/超时回 503 `{"error":"db_unavailable"}`）。该端口只在容器回环监听、不 publish、不经 Traefik；**公网没有 `/readyz` 路由**，公网打过去会被 SPA fallback 吃掉：body 是 `index.html`（`Content-Type: text/html`），状态码 404（tower-http 的 `ServeDir::not_found_service` 固定 404）——那跟就绪无关，不要拿它当探针。
  - 生产 `deploy/stack.yml` 的 app healthcheck **本次仍是 `/healthz`**（有意：避免 DB 短暂维护时整个 app 被 Swarm 判死；防复发先覆盖 staging）。staging `deploy/stack.staging.yml` 的 healthcheck 已改为容器内 `127.0.0.1:8090/readyz`，DB 不可用即 unhealthy。
  - `server.admin_bind` 若与 `server.bind` 相同，进程启动即报错（双监听撞车）；显式配成非回环地址时启动会打 `tracing::warn`（等于把查库探针暴露到该网卡，需确认有意）。
- 敏感配置全部走 external docker secret，应用从 `DLZ_*_FILE`（即 `/run/secrets/*`）读取，相关单测在 `src/config.rs`（`parse_with_env`）。

## 1. 首次准备（manager 上，一次性）

### 1.1 节点打标

stack 用 `node.labels.duolinzao.role` 做 placement（`app` 跑应用，`db` 跑 Postgres），先核对并打标。

> **拓扑前提**：本仓库的部署与 CI 检查假定 **manager 节点就是 app 节点**（`role=app`）。staging 冒烟与 db 健康判据都靠在 manager 上 `docker exec` app 容器完成（`docker exec` 只作用于本节点，见 §6.4）。把 app 调度到别的节点而不同步改 CI，这两步会失败（日志显式打 `CTR_MISSING`/找不到容器）。

```bash
docker node ls
docker node update --label-add duolinzao.role=app <应用节点名>
docker node update --label-add duolinzao.role=db <数据库节点名>
docker node inspect <节点> --format '{{.Spec.Labels}}'
```

### 1.1b 持久数据目录（bind mount）

持久数据不用命名卷，直接落宿主机目录。**每个会运行 app 或 db 的节点上，只需要事先建好同一个 `DEPLOY_PATH` 目录**（root 所有、权限 `755` 即可，例如 `sudo mkdir -p -m 755 /opt/duolinzao`）。子目录由容器启动时自动创建，CI 不需要 SSH 到 worker：

- db：把 `DEPLOY_PATH` 挂到 `/srv/duolinzao`，`PGDATA=/srv/duolinzao/pgdata`。官方 postgres 镜像的 entrypoint 以 root 启动，会自建 `pgdata/` 并设为 `999`、`0700`，首次启动自动 initdb。
- app：同样挂 `DEPLOY_PATH` → `/srv/duolinzao`。容器以 root 启动，只执行 `mkdir -p /srv/duolinzao/images && chown 65534:65534`，随即用 `setpriv` 降权到 nobody（65534）运行应用，菜品图写入 `/srv/duolinzao/images`（`deploy/config.toml` 的 `dish_image.dir`）。

`DEPLOY_PATH` 必须对容器内用户可遍历（至少 `755`），否则 postgres 初始化会报 `Permission denied`。app 以 nobody 运行时读不到 `pgdata/`（0700），两者互不影响。

### 1.2 网络

- 外部网（默认 `traefik-public`，可由 `TRAEFIK_NETWORK` 改名）：已存在、Traefik 在用，stack 里只引用不创建。
- `duolinzao-internal`：overlay + internal，由 stack 自动创建，db 只连它。

### 1.3 docker secret（5 个，全 external，stack 不创建）

```bash
DBPW='<强随机密码>'; DBURL="postgres://duolinzao:${DBPW}@db:5432/duolinzao"
printf '%s' "$DBPW"  | docker secret create dlz_db_password -
printf '%s' "$DBURL" | docker secret create dlz_database_url -
printf '%s' '<超管密码>' | docker secret create dlz_super_password -
# 32 字节 base64（LLM key 加密根密钥，丢了则已存渠道 key 解不开）：
openssl rand -base64 32 | tr -d '\n' | docker secret create dlz_local_key -
# 不用邮箱时填占位（stack 引用了 external secret，不存在会部署失败）：
printf '%s' 'unused' | docker secret create dlz_smtp_password -
docker secret ls
```

注意 `dlz_database_url` 里的密码必须和 `dlz_db_password` 是**同一个**（db 容器用后者设密码，app 用前者连库）。改密码时两个一起轮换。

### 1.4 swap（小内存节点建议加 1G）

```bash
fallocate -l 1G /swapfile && chmod 600 /swapfile && mkswap /swapfile && swapon /swapfile
echo '/swapfile none swap sw 0 0' >> /etc/fstab
free -h
```

### 1.5 GHCR 拉取

- 镜像默认**公开**最省事（GHCR package 设为 public），manager 无需登录；`GHCR_PULL_TOKEN` 可空着。
- 若设为私有：建一个只有 `read:packages` 的 token，填进仓库 secret `GHCR_PULL_TOKEN`，deploy 会先 `docker login ghcr.io`（用户名取仓库 owner，token 经 stdin 传入，不进远端命令行）。token 用 `[A-Za-z0-9_]` 字符。

## 2. GitHub 侧配置

### 2.1 Variables（Settings → Secrets and variables → Actions → Variables）

| 名称 | 默认值（未设时 stack 用） | 说明 |
|---|---|---|
| `DLZ_DOMAIN` | `example.com` | 域名；同时用于 Traefik Host 规则与 webauthn rp_id/origin |
| `TRAEFIK_ENTRYPOINT` | `web` | Traefik 入口点名 |
| `TRAEFIK_CERTRESOLVER` | `letsencrypt` | 证书解析器名 |
| `TRAEFIK_NETWORK` | `traefik-public` | 外部网络实际名（stack 引用 + `traefik.docker.network` 取同一值） |

### 2.2 Secrets（同位置 Secrets 页）

| 名称 | 说明 |
|---|---|
| `DEPLOY_HOST` | manager IP/域名 |
| `DEPLOY_PORT` | SSH 端口（可空，默认 22） |
| `DEPLOY_USER` | SSH 用户（能跑 docker，需在 docker 组） |
| `DEPLOY_SSH_KEY` | 私钥全文（只用于部署，建议单独建 deploy 专用 key） |
| `DEPLOY_KNOWN_HOSTS` | `ssh-keyscan -p 端口 HOST` 的输出；SSH 强制校验，防中间人 |
| `DEPLOY_PATH` | manager 上放 stack 的目录，如 `/opt/duolinzao` |
| `GHCR_PULL_TOKEN` | 可选；见 1.5 |

### 2.3 Environments

仓库 Settings → Environments 建 `production`，开 **Required reviewers**（至少一人点 Approve 才能发布）。deploy job 声明 `environment: production`，没过审批不会连服务器。

### 2.4 触发规则

deploy job 条件：主仓 + `refs/heads/master` + `push` 事件。fork 的 master push、任何 PR 都不会触发（另有 `environment` 二次把关）。master 每次 push 跑全量测试 → 镜像推送（`sha-<commit>` + `latest`）→ deploy 串行（`concurrency: deploy-production`）。

## 3. 发布流程（自动）

1. `test` / `e2e` / `e2e-scale` 全绿 → `image` 构建并推 `ghcr.io/moderras/duolinzao:sha-<commit>`（+ `latest`）。
2. `deploy`：算 `CONFIG_VERSION`（`deploy/config.toml` 的 sha256 前 12 位，配置一改名一变即触发服务更新），scp `stack.yml` + `config.toml` 到 `DEPLOY_PATH`；`IMAGE_TAG`/`DLZ_DOMAIN`/Traefik 四变量/`CONFIG_VERSION` 注入后 `docker stack deploy -c stack.yml --with-registry-auth duolinzao`。
3. 先轮询 `duolinzao_db`，**三个条件同时满足才算就绪**：`UpdateStatus.State` 为空或 `completed`（`updating` 不算完成——2026-11-10 事故就是 update=updating 时被放行的）、至少 1 个 Running 任务、且 DB 健康（等价于 `docker ps --filter health=healthy`；两节点拓扑下 db 在 worker、manager 的 `docker ps` 看不到它的容器，改用 app 容器回环查库接口 `/api/session/methods` 返回 200 判定，语义同 `pg_isready`，`/healthz` 不查库不能替）。rollback/paused 立即失败，约 2 分钟超时即失败；再轮询 `duolinzao_app`：spec 或运行中任务的镜像以新 tag 为前缀即可（允许 Swarm 解析后的 `@sha256:...`），且有 Running 任务、`UpdateStatus` 非 rollback/paused（10 分钟超时即失败；`failure_action: rollback` 已由 Swarm 自动回滚）。生产 db 仍是 start-first（新 db 起来后再停旧的，overlay 上 `db` VIP 不断），否则与 app 同次发版时 app 会因 `db` 解析失败（Name or service not known）直接 exit 1；**staging 的 db 已改 stop-first**（同机 bind PGDATA，start-first 会让两个 postgres 打开同一目录；代价是更新期间短暂断库，自动部署可接受，见 §6.4）；app 两环境都保持 stop-first（内存限制）。
4. runner `curl https://域名/healthz` 重试 5 分钟。
5. `docker image prune -f` 清旧镜像。

## 4. 回滚

- 自动：更新失败 Swarm 按 `update_config.failure_action: rollback` 回滚到上一版（`rollback_config` 已配）。
- 手动：`docker service rollback duolinzao_app`。
- 定版回滚：`DEPLOY_PATH=<同部署路径> IMAGE_TAG=sha-<旧commit> DLZ_DOMAIN=... TRAEFIK_ENTRYPOINT=... TRAEFIK_CERTRESOLVER=... TRAEFIK_NETWORK=... CONFIG_VERSION=... docker stack deploy -c stack.yml --with-registry-auth duolinzao`（DEPLOY_PATH 必须传，bind 路径靠它拼），或在 Actions 里对旧 commit 重跑 workflow（tag 不可变，旧镜像还在）。
- **回滚到不带 8090 管理端口的旧镜像**（本 PR 之前的 image）时要连 stack 文件一起回退，否则 staging 的 healthcheck 调 `127.0.0.1:8090/readyz` 必失败、Swarm 判 unhealthy 反复重启：
  - `docker service rollback duolinzao-staging_app` 回的是上一份服务 spec（含当时的 healthcheck），镜像/栈文件天然配套，一般可直接用；
  - 但「定版回滚」用当前 `stack.yml` + 旧 `IMAGE_TAG` 时，会把新 healthcheck(/readyz) 和旧镜像（无 8090）拼在一起 → 不健康。此时要同时把 manager 上的 `stack.yml` 换成与旧镜像同期的版本，或临时把该服务 healthcheck 改回 `/healthz` 再发。生产 healthcheck 仍是 `/healthz`，不受此影响。
- 注意：**迁移不回滚**。应用启动只向前迁移；回滚代码前确认旧代码兼容新表结构（一期 schema 只增不改，安全）。

## 5. 日常运维速查

```bash
docker stack services duolinzao
docker service ps duolinzao_app --no-trunc
docker service logs -f duolinzao_app
curl -fsS https://<域名>/healthz
# 就绪探针只在容器回环管理端口（公网没有）：DB 断时返回 503，是查库探针
docker exec $(docker ps -qf name=duolinzao_app | head -1) curl -fsS http://127.0.0.1:8090/readyz
docker volume ls | grep duolinzao   # 正常应无 duolinzao 卷：持久数据走 DEPLOY_PATH 下 bind mount
```

db 数据在宿主机目录 `DEPLOY_PATH/pgdata`（bind mount，随 db 节点 placement 固定）。备份即 tar 文件夹：`tar czf /backup/pg-$(date +%F).tgz -C <DEPLOY_PATH> pgdata`（停服或接受 crash-consistent）。恢复：停 db 服务后解压回去，保证属主仍是 UID 999。

**db 更新前先备份**：只要某次发版会动 db 服务的 spec（镜像/配置/资源），建议先做一次上面这行 tar（低峰期发版）。事故（2026-11-10）里 db 的 WAL/PGDATA 损坏，恢复成本远高于一次备份；staging 也建议保持这个习惯。

**生产 db start-first 的风险与待办（本次未改）**：`deploy/stack.yml` 的 db 仍是 `start-first` + 同机 bind `PGDATA`。理论上新任务先起、旧任务后停，但一旦调度时序异常或新实例失败，两个 postgres 可能打开同一 PGDATA，是本次事故的潜在触发点之一。本次只把 staging 改为 stop-first 并收紧 CI 门禁；**生产改 stop-first 要单独安排维护窗口**（更新期间 db 有 10~30s 不可用、app 会短暂报错），列入待办，不要在普通发版里顺手改。

## 6. 测试服（staging）

同一集群、独立 stack `duolinzao-staging`：自带独立小配额 Postgres（192M limit，`shared_buffers=32MB`，`max_connections=20`）与 app（192M limit），与生产零共享（service/router/secret/config/网络/数据目录全独立）。master 全绿后 CI 自动部署，无需审批；生产 `deploy` 保持人工审批不变。

### 6.1 DNS

staging 域名建议 `staging.<生产域名>`，例如生产是 `example.com` 则 staging 用 `staging.example.com`，解析到同一 Traefik 入口。证书由同一 certresolver 签发（通配或按需申请，Traefik 侧配置不变）。

### 6.2 docker secret（5 个，全 external，CI 自动确保，无需手工建）

staging 的 5 个 secret（名字带 `stg` 前缀，不与生产冲突）由 `deploy-staging` job 在每次部署前幂等确保，**缺才建、绝不重建已有**：

| secret | 来源 |
|---|---|
| `dlz_stg_super_password` | GitHub `staging` environment 的 secret `STAGING_SUPER_PASSWORD`（经 stdin 传，不进命令行） |
| `dlz_stg_db_password` / `dlz_stg_database_url` | 首次部署时在 manager 上用 `openssl rand -hex 24` 生成，拼成 `postgres://duolinzao:<密码>@db:5432/duolinzao` 一起建 |
| `dlz_stg_local_key` | 首次部署时在 manager 上 `openssl rand -base64 32` 生成 |
| `dlz_stg_smtp_password` | 占位 `unused`（log 模式不连 SMTP；stack 引用了 external secret，不存在会部署失败） |

注意：`docker secret` 不可变，已存在的 db 密码永远不会被覆盖（否则和已有 `pgdata` 对不上）。若 `dlz_stg_db_password` 与 `dlz_stg_database_url` 一有一无（人为误删其中一个），job 会直接报错，要求按 6.6 重置，不自动生成新的。`@db:5432` 中的主机名就是 `db`：staging 内部网络独立，service 发现只在栈内有效，app 照样用短名连自己的库。

### 6.3 仓库侧配置

建 environment `staging`（**不加** Required reviewers，自动部署）。注意作用域不一样：

- `STAGING_DLZ_DOMAIN` 必须设成**仓库级 Variables**（`Settings → Secrets and variables → Actions → Variables`），
  **不要**只设在 environment 里。`android-apk` job 没有声明 environment（它不该拿 staging 的 secret），
  因此**看不见 environment 级变量**；只设在 environment 时它会当成“没配”，再也别静默回落到
  `https://example.com`——现在缺域名就跳过 staging 包并打 warning（见 7.1）。
- 两个 secret 设在 environment `staging` 里（只给部署 job 用）。

`deploy-staging` job 声明了 `environment: staging`，而 GitHub 的变量解析是
**environment → repository → organization** 依次回退，所以仓库级 `STAGING_DLZ_DOMAIN`
部署 job 照样读得到。任一缺失 `deploy-staging` 就**跳过并打 notice**，master 保持绿色。

| 名称 | 位置 | 示例 | 说明 |
|---|---|---|---|
| `STAGING_DLZ_DOMAIN` | **repository var** | `staging.example.com` | staging 域名；同时用于 Traefik Host 规则、webauthn rp_id/origin 与 **staging APK 的 baseUrl** |
| `STAGING_DEPLOY_PATH` | environment secret | 如 `/opt/duolinzao-staging` | manager 上放 staging stack 的目录，**不得与生产 `DEPLOY_PATH` 相同**（CI 会校验，相同直接拒绝部署；生产路径在 staging 环境不可见时放过） |
| `STAGING_SUPER_PASSWORD` | environment secret | 强随机密码 | 测试服超管密码，只在 secret 缺失时建一次 |

一行设好（`gh` 已登录时；换成自己的 staging 域名，本仓不自建站点时留空即可）：

```bash
gh variable set STAGING_DLZ_DOMAIN --repo <owner>/duolinzao --body 'staging.<生产域名>'
```

其余 SSH 凭据（`DEPLOY_HOST/PORT/USER/SSH_KEY/KNOWN_HOSTS`、`GHCR_PULL_TOKEN`）与 Traefik 三变量复用生产的。

manager 上仍需手工做两件事：建好 staging 数据目录（各会跑 app/db 的节点都要建，权限 `755`，例如 `sudo mkdir -p -m 755 /opt/duolinzao-staging`）和 DNS（见 6.1）。这两样没好之前 job 照样跳过，不会把 master 搞红。

### 6.4 部署与验证

每次 master push 全绿后自动：检查配置 → 确保 5 个 `dlz_stg_*` secret（缺才建）→ scp `stack.staging.yml` + `config.staging.toml`（随即在远端改名为 `stack.yml` / `config.toml`，与生产同形；stack 文件里的 `file: ./config.toml` 与改名后一致）→ `docker stack deploy ... duolinzao-staging` → 先等 `duolinzao-staging_db` 就绪（`UpdateStatus.State` 为空/ `completed` + 至少 1 个 Running + DB 健康，判据见 §3 第 3 步）再轮询 `duolinzao-staging_app` 收敛 → `curl https://staging 域名/healthz` → **业务冒烟（查库）**。

**业务冒烟（本次补的 P0 洞）**：`/healthz` 只证明进程活着，DB 断时 staging 照样全绿。这一步在 runner 上 SSH 到 manager，`docker exec` 进 `duolinzao-staging_app` 容器，调容器回环 `http://127.0.0.1:8080/api/session/methods`（匿名 GET、必查库 `email_enabled`），要求 200，30×10s 重试，失败即 fail job 并保留 `docker service ps/logs`。用的是容器内回环、不经 Traefik 公网，也**没有**给 `/readyz` 开公网入口。**它依赖 app 容器在 manager 节点上**（文档拓扑即 app 在 manager）；容器不在本节点时日志显式打 `http=CTR_MISSING`，不会静默变红。

staging 与生产有两处有意差异：db 更新 order（staging 已 stop-first，生产仍 start-first，原因与待办见 §5）、app healthcheck（staging 用 `/readyz`，生产仍 `/healthz`，见 §0）。除收紧 db 门禁判据外，生产发布流程不变。

```bash
docker stack services duolinzao-staging
docker service logs -f duolinzao-staging_app
curl -fsS https://staging.example.com/healthz
# 手动复核两个探针（都在容器回环，公网没有）：readyz 查库，methods 是冒烟同款
docker exec $(docker ps -qf name=duolinzao-staging_app | head -1) curl -fsS http://127.0.0.1:8090/readyz
docker exec $(docker ps -qf name=duolinzao-staging_app | head -1) curl -sS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8080/api/session/methods
```

### 6.5 邮件验证码怎么看（log 模式）

staging 的 `[email]` 配的是 `enabled = true` + `delivery = "log"`：不连 SMTP，验证码直接打进应用日志，行里带 `[email:log-only]` 标记：

```bash
docker service logs duolinzao-staging_app 2>&1 | grep 'email:log-only'
```

从该行取 6 位码，在登录页走“邮箱验证码”登录即可。生产默认仍是 `delivery = "smtp"` 真发，行为不变。

### 6.6 改超管密码与清库重置

改测试服超管密码：docker secret 不可变，**只改 GitHub 上 `staging` 环境的 `STAGING_SUPER_PASSWORD` 不会自动生效**。步骤：`docker stack rm duolinzao-staging` → `docker secret rm dlz_stg_super_password` → 重新跑 CI（re-run `deploy-staging` 或等下次 master push），新密码会被建进去。不要动生产的 `dlz_super_password`。

彻底重置（含换库密码）：

```bash
docker stack rm duolinzao-staging
# 等 service 全退（docker stack services duolinzao-staging 为空）后再删：
docker secret rm dlz_stg_db_password dlz_stg_database_url dlz_stg_super_password dlz_stg_local_key dlz_stg_smtp_password
rm -rf <STAGING_DEPLOY_PATH>/pgdata <STAGING_DEPLOY_PATH>/images
# 再重新跑 CI（re-run deploy-staging 或下次 master push）：secret 与库都会重建。
```

只删 `dlz_stg_*`，**绝不动生产 `dlz_*`**。

## 7. Android APK 自动发布（站点 `/app/*` 直出最新包）

模型：CI 只构建**未签名**通用包（按域名各出一份）打进镜像；**正式签名在服务端完成**，
签名材料（keystore + 密码 + 别名）只活在**应用数据库**（`app_signing` 单例行，
经 `local_key` AES-GCM 加密），由超管在「应用签名」页上传/替换、点「签名发布」。
密钥不进 git、不进 GitHub Secrets、不进部署机文件。

每次 CI（PR 照跑）`android-apk` job 自动：未签通用包 → 按域名各出一份 →
打进服务端镜像 → `/app/*` 直出（此时是未签包 + `"signing":"unsigned"` 的临时
`version.json`）。App 只拉**当前 baseUrl 同源**的 `/app/version.json`
（`versionCode` 更大才提示，`url` 必须同源，校验 `sha256`），服务端缺文件返回真 404。

### 7.1 为什么一份镜像要装两份包

APK 的 `baseUrl` 写死在包内，且更新器要求同源；生产和测试服共用同一镜像，
**一个 APK 不可能两边都用**。所以：

- `prod/duolinzao.apk`（baseUrl=`https://<DLZ_DOMAIN>`）→ 镜像 `/app/static/app/`
  （生产默认 `{static_dir}/app`，不动）。
- `staging/duolinzao.apk`（baseUrl=`https://<STAGING_DLZ_DOMAIN>`）
  → 镜像 `/app/static/app-staging/`，staging stack 设
  `DLZ_APP_DIR=/app/static/app-staging` 把 `/app/*` 路由指向它（见 `deploy/stack.staging.yml`）。
- 路由仍是同一个 `/app/*`，靠服务端目录隔离；`version.json` 的 `url` 都是
  `/app/duolinzao.apk`（同源，冒烟步骤会校验）。
- **缺 `STAGING_DLZ_DOMAIN` 时 CI 不生成 staging 包**（`::warning` 跳过，绝不静默把
  `https://example.com` 写进包）：连不上服务器、通行密钥仪式起不来的故障比缺包难查得多。
  这时镜像里 `/app/static/app-staging/` 只有 `.gitkeep`，部署后 staging 的
  `/app/version.json` 是 404（一眼可见）；补上变量重跑 master 即可。prod 包仍按开源默认
  用 `https://example.com` 占位（不自建站点的人 `cargo run` / 本地测都不受影响）。
- `inject-server-url.sh` 自己也会拒绝空/非 `http(s)://` 的 URL（包已打出来后再查出
  baseUrl 是空的成本太高）。

### 7.2 版本号

- `versionName` = Gitea 风格串（`scripts/ci-version.sh`，与页脚版本同算法；
  `image` job 与 `android-apk` job 共用这一份，避免两处漂移）。
- `versionCode` = `git rev-list --count HEAD`（单调递增，保证每次构建都比上大）。
- CI 经 Gradle 属性传入（`-PdlzVersionCode=` / `-PdlzVersionName=`）；
  本地不传保留默认值（`1` / `0.1.2`），本地构建不破。

### 7.3 超管一次性准备（两环境各做一次）

1. 本地生成长期 keystore（**占位值，换成自己的**；jks 文件**绝不进仓库、绝不进 GitHub**，
   生成后只留离线备份，见 7.4）：

```bash
keytool -genkeypair -keystore site.jks -storepass '<强随机密码>' -keypass '<强随机密码>' \
  -alias duolinzao -keyalg RSA -keysize 2048 -validity 3650 -dname "CN=example.com"
```

2. 用超管账号登录站点 → 「应用签名」页 → 选择 jks 文件、填三个值 → 「保存到数据库」。
   也可以跳过这步：空库时直接点「签名发布」，服务端会自动生成一把 release 级 RSA
   keystore 存进数据库再签名（`keytool -genkeypair`，密码为服务端随机串）；之后仍可随时在
   同页覆盖成自己的 key（覆盖前页面会警告换 key 后果）。
   页面只回显别名与 keystore 指纹（sha256），密码永不回显、不进审计、不出现在任何列表响应。
   应用 API 不提供 keystore 导出/下载（防泄漏）；备份靠数据库（`app_signing` 行密文随库备份），
   恢复数据库即还原签名能力。
3. 点「签名发布」：服务端用库里 keystore 给当前镜像里的包签名（`apksigner`，镜像自带），
   重算 sha256 并写出正式 `version.json`（`"signing":"release"`）。
4. 验证（需已登录会话；匿名会 401）：浏览器打开站点登录后访问 `/app/version.json`，
   或 `curl -fsS -b 'dlz_session=<token>' https://example.com/app/version.json | jq .`
   （把域名换成自己的；期望 `signing` 为 `release`）。
5. **每次部署新镜像后都要再点一次「签名发布」**（镜像更新会重置 `/app/*` 为未签包）。

staging 与 production 各有自己的数据库 → 各存各的 keystore。
若希望测试机上 staging 包能覆盖安装生产包（同包名升级不卸载），两环境上传**同一把**
keystore；否则分开，文档在此写清，超管自己选。

DB 尚未配置 keystore 时：staging/CI 保持绿（`/app/*` 是 CI 未签包，App 下载后因无签名
丢弃更新，不崩）；生产首次必须先做上面第 2–3 步再对外发包。

### 7.4 keystore 一旦启用不能换

**换 keystore = 已装设备无法更新，必须卸载重装**（Android 签名机制，签名指纹比对失败
即丢弃更新）。所以：keystore 建好后**离线备份**（至少两处，如加密 U 盘 + 密码管理器附件），
密码同样备份；**数据库备份即包含签名密钥，备份必须同等保护**；丢了只能让所有人重装。

### 7.5 首次装包与更新门禁

- **`GET /app/*`（apk + version.json）要有效会话**，匿名 401，防刷流/随便下。
- **首次/对外下载入口只在超管「应用签名」页**（复制链接 / 下载 APK）。员工「我的」、
  食堂「更多」、登录页、页脚都不挂入口。
- 超管在本页下载后可线下分发；若复制链接给别人，对方也必须已登录本站，否则 401。
- **已登录用户**可在 App 内「检查更新」拉同源 `/app/version.json` 与 APK（请求带会话 cookie）。
- 镜像体积：两份 APK 都是 MB 级（具体数字见 PR 描述），加上 JRE + build-tools
 （服务端签名用，约数百 MB）；**没开 R8/混淆**（会改 App 行为），除非确认安全才考虑。

### 7.6 启动自动签名（A 方案，缺省开）

- 行为：进程就绪后（监听端口 bind 成功、健康检查已可过，后台任务）自检一次
  `{app_dir}/version.json`：暂存包有效（`versionCode>0`）且 `signing` 不是 `release`
  → 若库里已有 keystore，用同一签名代码路径自动签名发布（签 `duolinzao.apk`、
  重算 sha256、写出 `signing: "release"` 的 `version.json`，记一条无 actor 审计）；
  若库里还没有 keystore，**默认只告警、不自动生成**（超管至少手动点一次「签名发布」，
  建 key 的动作永远要人确认）。
- 已发布（`signing: "release"`）时不动手，只做 sha 自洽检查：`version.json` 与
  `duolinzao.apk` 对不上就记一条 warn 日志，请超管手动点「签名发布」修复。
- 开关：`[server] apk_auto_sign`（缺省 `true`），`DLZ_APK_AUTO_SIGN=false` 可关；
  关了就回到老流程（每次部署新镜像后手动点一次）。手动「签名发布」入口原样保留，
  随时可点；空库点它仍会先自动生成一把 keystore 再签（7.3 第 2–3 步）。
- 安全：keystore/密码不进 HTTP API、不进日志明文（沿用原有约束，只回显别名与指纹），
  不给 CI 签名权（CI 仍只打未签包进镜像）。
- 并发/目录：stop-first 单副本，原地 rename 写同一 `{app_dir}`，与手动路径一致，
  不引入新目录；启动钩子失败只记日志（`apk auto sign ...`），绝不阻塞启动。
- 可观测：日志关键字 `apk auto signed on boot`（成功，带 versionCode/sha256）/
  `apk auto sign skipped|failed`（跳过/失败原因）；「应用签名」页新增「已发布」一行，
  直接显示 `version.json` 实况（versionCode + signing），超管一眼确认自签是否生效。
- 验证：部署新镜像后，用已登录会话访问 `/app/version.json`
  （期望 `signing` 为 `release`，sha256 与刚构建的包一致），外加服务端日志一条成功记录；
  空库场景应看到 skip 告警、且 `app_signing` 表仍为空（不自动建 key）。

### 7.7 `/.well-known/assetlinks.json`（通行密钥的 Digital Asset Links）

App 用**站点域名当 RP ID** 调系统通行密钥（Credential Manager），Android 会去

```
https://<域名>/.well-known/assetlinks.json
```

核对「这个域名 ↔ `com.duolinzao.app` + 证书指纹」的关联（`delegate_permission/common.get_login_creds`）。
路径必须 **200 + `application/json`**，且不能被 SPA 回退成 `index.html`：拿到 HTML 会被判成
「无关联」，App 内通行密钥注册/登录直接失败（而且失败得很安静）。

- 由后端**直出**（显式路由，优先于 SPA fallback），**免会话**（系统校验器不带 cookie）。
- 指纹不写死在仓库里：`certSha256` 由「签名发布」时 `apksigner verify --print-certs` 从
  **实际发放的包**读出（大写冒号分隔），与服务端当前 release 证书永远一致；换 keystore 后重签即自动更新。
- 未签名（CI 未签包、还没点「签名发布」）时回 `[]`：合法 JSON =「没有已关联的 App」，
  不是 404、也不是 HTML。
- 与登录邮件用的 App Links **无关**（那条线已废弃，见 `docs/clients/design.md` §3.2.4）。
- 验证（staging 与生产各一次）：

```bash
curl -fsS -D- https://example.com/.well-known/assetlinks.json | head -20   # 200 + application/json
curl -fsS https://example.com/.well-known/assetlinks.json | jq .          # 签名后应有 package_name + 指纹
```

  期望与 `curl -fsS https://example.com/app/version.json | jq .certSha256` 的值一致；
  App 内通行密钥仍失败时，先用
  `adb shell pm get-app-links com.duolinzao.app` / `adb logcat | grep -i assetlinks` 看系统侧结论。

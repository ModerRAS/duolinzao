# 多邻灶生产部署（Docker Swarm）

目标拓扑：两个节点，一个跑应用（role=app），一个跑 Postgres（role=db）；入口是挂 label 的 Traefik，外部网络名可配（默认 `traefik-public`）。发布方式：CI 在 master 测试全绿后构建镜像推 GHCR，再由 deploy job SSH 到 manager 做 `docker stack deploy`（start-first，失败自动 rollback）。**服务器上永远不 build。**

仓库是开源的，只放通用配置：域名、entrypoint、certresolver、外部网络名、部署路径全部变量化，见 `deploy/stack.yml` 头部注释。本文件不出现任何私有主机名/域名。

## 0. 与代码的关系

- 数据库迁移在**应用启动时自动运行**（`src/main.rs` 里 `sqlx::migrate!`），无需手工 `sqlx migrate run`；回滚镜像不会回滚迁移（迁移只向前，见第 5 节）。
- 健康探针 `GET /healthz`（纯文本 `ok`，不查库、不登录、免 CSRF），Traefik/CI 只看它；`GET /api/health` 仍在。
- 敏感配置全部走 external docker secret，应用从 `DLZ_*_FILE`（即 `/run/secrets/*`）读取，相关单测在 `src/config.rs`（`parse_with_env`）。

## 1. 首次准备（manager 上，一次性）

### 1.1 节点打标

stack 用 `node.labels.duolinzao.role` 做 placement（`app` 跑应用，`db` 跑 Postgres），先核对并打标：

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
3. 轮询 `duolinzao_app`：spec 或运行中任务的镜像以新 tag 为前缀即可（允许 Swarm 解析后的 `@sha256:...`），且有 Running 任务、`UpdateStatus` 非 rollback/paused（10 分钟超时即失败；`failure_action: rollback` 已由 Swarm 自动回滚）。
4. runner `curl https://域名/healthz` 重试 5 分钟。
5. `docker image prune -f` 清旧镜像。

## 4. 回滚

- 自动：更新失败 Swarm 按 `update_config.failure_action: rollback` 回滚到上一版（`rollback_config` 已配）。
- 手动：`docker service rollback duolinzao_app`。
- 定版回滚：`DEPLOY_PATH=<同部署路径> IMAGE_TAG=sha-<旧commit> DLZ_DOMAIN=... TRAEFIK_ENTRYPOINT=... TRAEFIK_CERTRESOLVER=... TRAEFIK_NETWORK=... CONFIG_VERSION=... docker stack deploy -c stack.yml --with-registry-auth duolinzao`（DEPLOY_PATH 必须传，bind 路径靠它拼），或在 Actions 里对旧 commit 重跑 workflow（tag 不可变，旧镜像还在）。
- 注意：**迁移不回滚**。应用启动只向前迁移；回滚代码前确认旧代码兼容新表结构（一期 schema 只增不改，安全）。

## 5. 日常运维速查

```bash
docker stack services duolinzao
docker service ps duolinzao_app --no-trunc
docker service logs -f duolinzao_app
curl -fsS https://<域名>/healthz
docker volume ls | grep duolinzao   # 正常应无 duolinzao 卷：持久数据走 DEPLOY_PATH 下 bind mount
```

db 数据在宿主机目录 `DEPLOY_PATH/pgdata`（bind mount，随 db 节点 placement 固定）。备份即 tar 文件夹：`tar czf /backup/pg-$(date +%F).tgz -C <DEPLOY_PATH> pgdata`（停服或接受 crash-consistent）。恢复：停 db 服务后解压回去，保证属主仍是 UID 999。

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

建 environment `staging`（**不加** Required reviewers，自动部署），在该 environment 里设以下三项（**都是 environment 级**，任一缺失 `deploy-staging` job 就**跳过并打 notice**，master 保持绿色）：

| 名称 | 位置 | 示例 | 说明 |
|---|---|---|---|
| `STAGING_DLZ_DOMAIN` | environment var | `staging.example.com` | staging 域名；同时用于 Traefik Host 规则与 webauthn rp_id/origin |
| `STAGING_DEPLOY_PATH` | environment secret | 如 `/opt/duolinzao-staging` | manager 上放 staging stack 的目录，**不得与生产 `DEPLOY_PATH` 相同**（CI 会校验，相同直接拒绝部署；生产路径在 staging 环境不可见时放过） |
| `STAGING_SUPER_PASSWORD` | environment secret | 强随机密码 | 测试服超管密码，只在 secret 缺失时建一次 |

其余 SSH 凭据（`DEPLOY_HOST/PORT/USER/SSH_KEY/KNOWN_HOSTS`、`GHCR_PULL_TOKEN`）与 Traefik 三变量复用生产的。

manager 上仍需手工做两件事：建好 staging 数据目录（各会跑 app/db 的节点都要建，权限 `755`，例如 `sudo mkdir -p -m 755 /opt/duolinzao-staging`）和 DNS（见 6.1）。这两样没好之前 job 照样跳过，不会把 master 搞红。

### 6.4 部署与验证

每次 master push 全绿后自动：检查配置 → 确保 5 个 `dlz_stg_*` secret（缺才建）→ scp `stack.staging.yml` + `config.staging.toml`（随即在远端改名为 `stack.yml` / `config.toml`，与生产同形；stack 文件里的 `file: ./config.toml` 与改名后一致）→ `docker stack deploy ... duolinzao-staging` → 轮询 `duolinzao-staging_app` 收敛 → `curl https://staging 域名/healthz`。生产发布流程不变。

```bash
docker stack services duolinzao-staging
docker service logs -f duolinzao-staging_app
curl -fsS https://staging.example.com/healthz
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
   页面只回显别名与 keystore 指纹（sha256），密码永不回显、不进审计、不出现在任何列表响应。
3. 点「签名发布」：服务端用库里 keystore 给当前镜像里的包签名（`apksigner`，镜像自带），
   重算 sha256 并写出正式 `version.json`（`"signing":"release"`）。
4. 验证：`curl -fsS https://example.com/app/version.json | jq .`
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

### 7.5 首次装包

- 生产手机浏览器打开 `https://example.com/app/duolinzao.apk`（把域名换成自己的）
  下载安装；之后 App 内「检查更新」会自动提示。
- 镜像体积：两份 APK 都是 MB 级（具体数字见 PR 描述），加上 JRE + build-tools
 （服务端签名用，约数百 MB）；**没开 R8/混淆**（会改 App 行为），除非确认安全才考虑。

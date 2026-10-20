# AGENTS.md — 多邻灶（duolinzao）

部门内部报餐系统：约 300 人、≤10 食堂、三餐/天、无支付。产品名「多邻灶」，仓库 / crate：`duolinzao`（`ModerRAS/duolinzao`）。

一期需求与界面清单以 `docs/` 为准；改产品行为前先读文档，勿凭常识扩 scope。

## Stack

- 前端：Vue 3
- 后端：Rust + Axum（Tokio）
- 数据库：PostgreSQL + SQLx（写 SQL；`query!` / migrations；CI 用 `cargo sqlx prepare` 离线元数据）
- 密码哈希：Argon2id（勿用 SHA 系列存密码）
- 交付：Docker；部署与备案自理

## Docs map

- `docs/requirements-phase1.md` — 一期需求（权威）
- `docs/` 下 EARS 用户故事与任务分解 — 实现拆解
- `docs/ui/` — 主视觉参考图（员工竖屏；食堂与总管横屏以左侧栏为主，含强提醒、导出、审计、邮箱、阈值、找回、LLM）。新建和编辑是同一张菜品表单。评语和改单的旧稿导航不当主导航。班制图里的上五休二禁止实现

## Architecture notes（一期）

- 鉴权：Passkey → TOTP → 邮箱 magic link → 密码兜底；无短信 OTP。邮箱相关能力（magic link / 验证）可由超管在后台关闭，做成类似 Gitea / Vaultwarden 的可选项，不强制验证邮箱
- 账户：`username` 只读；`display_name` 可改，界面优先显示名。自助改密校验当前密码，保留当前会话并吊销其它会话。超管找回仍吊销该账号全部会话。Passkey 可后绑，TOTP / 绑邮箱 / 退出在账户设置页。不新增权限码
- 新设备：已登录设备页内批准（可挂起；展示 UA/IP）；不依赖系统推送；扫码非主路径
- 授权：**Scoped RBAC**（`employee` / `canteen` / `admin` / `super_admin` + 食堂范围）；标签映射为 admin 任职，不是另一套引擎
- 班制：仅「上二休四」+「自定义」；**统一 DSL**（预设=写好的 DSL；用户轻度改餐次后存个人 DSL；按 DSL 生成每日报餐；套用前预览，绿=报餐 / 橙=不报，可点切换）
- 评分：明线 −2～2（餐后一周）→ 暗线人–菜偏好；选菜优化与强提醒吃暗线；员工端不展示排行。选菜色阶严格是偏好高低（更高更绿，更低更红），口味标签只作辅助，不替代排序和色阶
- 菜谱：新建 / 编辑可上传图片（库内 `image_path`，接口 `image_url`）。列表缩略图可选；无图用中性占位。口味标签挂在菜品上，食堂用 `menu.write` 维护
- 模板生成器的渠道由总管理在「LLM 渠道配置」维护（`system.super`，不新增权限码）。key 加密且界面不回显原文。员工只调用统一生成接口
- 截止：北京时间提前一天锁次日三餐；之后仅管理员可改

## Commands（骨架阶段按实际补全）

- 后端：`cargo run` / `cargo test` / `cargo clippy`
- 前端：`pnpm dev` / `pnpm test` / `pnpm lint`（包管理器以仓库锁定为准）
- DB：Docker Compose 起 Postgres；`sqlx migrate run`；改查询后 `cargo sqlx prepare`

命令以 README / Makefile 最终稿为准，本处过时则更新本文件。

## Conventions

- 业务资源尽量带或可推导 `canteen_id`；权限先鉴权再查数，避免查后再滤漏权
- 领域规则（截止、评分窗口）与权限码分开实现
- 敏感操作写审计（actor、角色/任职、食堂范围、权限码、结果），保留 ≥6 个月
- 中文产品名「多邻灶」；标识符 `duolinzao`
- UI：员工端竖屏；食堂/管理端横屏。工作台统一使用左侧栏，登录页无侧栏，不并存顶栏主导航，菜单只列一期画面

## Version / Release（页脚版本号，Gitea 风格）

- 版本串规则（严格跟 Gitea Makefile，不插入 `+dev-`）：
  - 正好落在 `vX.Y.Z` tag 上：显示 `X.Y.Z`（去掉 `v`，不附 commit）
  - 非 tag / 开发构建：`git describe --tags --always`，把**第一个** `-` 换成 `+`，再去掉开头 `v`（如 `0.1.0+12-gc7d0119`）
  - 仓库尚无 `v*` tag 时 `--always` 只给出短 sha：拼 `{Cargo.toml 版本}+0-g{sha}`（如 `0.1.0+0-gc7d0119`）；无 git 时回退 `{版本}+unknown`
- 展示位置（全局同一文案）：登录页 + 横屏侧栏底 + 员工竖屏底，文案 `Powered by 多邻灶 · Version: <ver>`
- API：`GET /api/health` JSON 带 `version` 字段；`GET /healthz` 保持纯 `ok`、不加版本（部署探针用）
- 注入方式：`build.rs` 编译期注入（`DLZ_VERSION` 显式值优先，原样用；否则抓 `git describe` 原始串进 `DLZ_GIT_DESCRIBE`，由 `src/version.rs::describe_to_version` 归一化）；归一化逻辑在 `src/version.rs`，单测覆盖
- 镜像 tag 关系：GHCR 的 `sha-<commit>` / `latest` 只做部署寻址（Swarm 按不可变 tag 拉镜像），页脚主文案用 describe 串，不直接展示 `sha-…` 前缀串；Docker 构建经 `ARG DLZ_VERSION` + `ARG GITHUB_SHA` 透传版本（`.dockerignore` 忽略 `.git`，容器内不跑 `git describe`），CI 在 image job 算好串后以 build-args 传入
- 日常 bump：助手可按规则自行 bump `Cargo.toml`，必要时同步 `web/package.json`（当前基线均为 `0.1.0`）；不要为页脚功能单独打 git tag

## Don't

- 不要把 SHA/MD5 当密码存储；不要上短信 OTP
- 不要恢复已砍需求：上五休二、多班叠加、按食堂发不同班制、支付
- 不要在员工端展示菜品排行或「Elo/倾向」等暗线说法
- 不要无范围的「食堂管理员能改所有食堂」
- 不要把评语做成可只评不打分；不要开放硬删评语接口（软删）
- 一期范围外功能先记文档再做

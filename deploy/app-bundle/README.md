# `deploy/app-bundle/`：CI 填充的未签 APK 产物目录（不进 git）

`android-apk` job 在每次 CI 构建出未签通用包（按域名各出一份），`image` job 用
`actions/download-artifact` 拉到这里，再由 Dockerfile 拷进镜像：

- staging 份只在仓库级 var `STAGING_DLZ_DOMAIN` 存在时生成；缺失时 CI 跳过并打
  warning（绝不静默写 `example.com`），目录里只留 `.gitkeep` → 部署后 staging 的
  `/app/version.json` 是 404，一眼能看出来。
- `prod/duolinzao.apk` + `prod/version.json`（`"signing":"unsigned"`）
  → 镜像 `/app/static/app/`（生产默认 `{static_dir}/app`，baseUrl 用仓库级 `DLZ_DOMAIN`）。
- `staging/duolinzao.apk` + `staging/version.json` → 镜像 `/app/static/app-staging/`
  （staging stack 设 `DLZ_APP_DIR=/app/static/app-staging`，baseUrl 用仓库级
  `STAGING_DLZ_DOMAIN`；两份包同镜像、路由同为 `/app/*`，靠目录隔离）。

正式签名在服务端完成，不在 CI：超管把 keystore 存进应用数据库（`app_signing` 表，
「应用签名」页）后点「签名发布」，服务端用镜像自带的 `apksigner` 签名、重算 sha256
并写出正式 `version.json`（`"signing":"release"`）。密钥不进 GitHub、不进部署机文件。
每次部署新镜像后都要再点一次签名发布。详见 `docs/deploy.md` §7。

本地 `docker build` 时这里只有 `.gitkeep`（见 `.gitignore`），两个目录都拷空壳，
构建照样成功，`/app/version.json` 缺文件即 404。

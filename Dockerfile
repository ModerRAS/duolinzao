# 多邻灶一期镜像：Axum 二进制 + Vue 构建产物。数据库由部署方单独运行。
# 餐品图片放在挂载目录，不打进镜像。许可证 AGPL-3.0。

FROM node:22-bookworm-slim AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ .
RUN npm run build

FROM rust:1.88-bookworm AS rust
WORKDIR /src
# 页脚版本：优先 CI 传入的 DLZ_VERSION（Gitea 风格归一化串）；无则 build.rs
# 用 GITHUB_SHA 拼 `{cargo_ver}+0-g{sha}`，本地则回退 `{cargo_ver}+unknown`。
# .dockerignore 默认忽略 .git，构建上下文里没有 git 历史，不做 git describe。
ARG DLZ_VERSION=""
ARG GITHUB_SHA=""
ENV DLZ_VERSION=${DLZ_VERSION} GITHUB_SHA=${GITHUB_SHA}
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY migrations ./migrations
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl default-jre-headless unzip \
    && rm -rf /var/lib/apt/lists/*
# 应用签名在服务端完成：JRE 跑 apksigner（只取 build-tools 35.0.1，不装整个 SDK；
# --version 即校验布局）。
RUN set -eux; \
    curl -fsSL -o /tmp/build-tools.zip https://dl.google.com/android/repository/build-tools_r35.0.1_linux.zip; \
    mkdir -p /tmp/bt /opt/android-sdk; \
    unzip -q /tmp/build-tools.zip -d /tmp/bt; \
    mv /tmp/bt/* /opt/android-sdk/; \
    rm -rf /tmp/bt /tmp/build-tools.zip; \
    SIGNERDIR=$(dirname "$(find /opt/android-sdk -maxdepth 2 -name apksigner -type f | head -1)"); \
    test -n "$SIGNERDIR" && test -x "$SIGNERDIR/apksigner"; \
    ln -s "$SIGNERDIR" /opt/android-bt; \
    du -sh /opt/android-sdk; \
    /opt/android-bt/apksigner --version
ENV PATH="/opt/android-bt:${PATH}"
WORKDIR /app
COPY LICENSE /app/LICENSE
COPY --from=rust /src/target/release/duolinzao /app/duolinzao
COPY --from=web /src/web/dist /app/static
# CI 由 android-apk job 填充未签包（deploy/app-bundle/，本地构建时只有 .gitkeep，
# 拷空壳也成功；staging 份走 DLZ_APP_DIR 指向 app-staging，见 deploy/stack.staging.yml）。
# 正式签名在服务端完成（超管上传 keystore 到数据库后点「签名发布」，见 docs/deploy.md §7）。
COPY deploy/app-bundle/prod/ /app/static/app/
COPY deploy/app-bundle/staging/ /app/static/app-staging/
RUN rm -f /app/static/app/.gitkeep /app/static/app-staging/.gitkeep
# 镜像以 nobody 运行：先把餐品图片目录建好并交给它，命名卷才会继承属主。
# /app/static/app* 同样交给 nobody，签名发布时服务端要重写 APK 与 version.json。
RUN mkdir -p /var/lib/duolinzao/images /app/static/app /app/static/app-staging \
    && chown -R 65534:65534 /var/lib/duolinzao /app/static/app /app/static/app-staging
ENV DUOLINZAO_CONFIG=/config/config.toml
EXPOSE 8080
USER 65534:65534
ENTRYPOINT ["/app/duolinzao"]

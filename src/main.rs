use std::net::SocketAddr;
use std::time::Duration;

use duolinzao::config::AppConfig;
use duolinzao::ops::meal;
use duolinzao::ops::{app_signing, session};
use duolinzao::{admin_router, router};
use duolinzao::state::AppState;
use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "duolinzao=info,tower_http=info".into()),
        )
        .init();

    let path = std::env::var("DUOLINZAO_CONFIG").unwrap_or_else(|_| "config.toml".into());
    let config = AppConfig::load_path(&path).unwrap_or_else(|err| {
        eprintln!("{err}");
        std::process::exit(1);
    });
    // 双监听撞车是配置错误：两个 TcpListener 绑同一地址时管理面必定起不来，
    // 探针静默落空；启动即报错，比带病运行好排查。
    if config.bind == config.admin_bind {
        anyhow::bail!(
            "server.bind 与 server.admin_bind 不能相同（{}）：业务面与管理面必须分开监听",
            config.bind
        );
    }
    // admin_bind 缺省回环即边界；显式改成非回环会把 /readyz 暴露到该网卡，必须留痕。
    if !config.admin_bind.ip().is_loopback() {
        tracing::warn!(
            admin_bind = %config.admin_bind,
            "server.admin_bind 不是回环地址：/readyz 会暴露到该网卡，确认这是有意的"
        );
    }
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    let state = AppState::production(pool, config)?;
    session::bootstrap(&state).await?;
    let app = router(state.clone());
    let admin_app = admin_router(state.clone());
    let bind = state.config.bind;
    let admin_bind = state.config.admin_bind;
    let maintenance = state.clone();
    tokio::spawn(async move {
        loop {
            if let Err(err) = meal::maintenance(&maintenance).await {
                tracing::warn!(error = %err.message, "maintenance");
            }
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    });
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let admin_listener = tokio::net::TcpListener::bind(admin_bind).await?;
    tracing::info!(%bind, %admin_bind, "多邻灶已启动");
    // A 方案：就绪后再自检签名（后台跑，不阻塞健康检查；失败只记日志）。
    let auto_sign = state.clone();
    tokio::spawn(async move {
        app_signing::auto_sign_on_boot(&auto_sign).await;
    });
    // 管理面与业务面同进程双监听：就绪探针只走回环 admin_bind，不经公网端口；任一面异常退出都直接返回错误，避免半活进程。
    tokio::try_join!(
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_signal()),
        axum::serve(admin_listener, admin_app.into_make_service())
            .with_graceful_shutdown(shutdown_signal())
    )?;
    Ok(())
}

/// 优雅退出信号：容器/编排器停容器发 SIGTERM，本地终端 Ctrl-C 发 SIGINT，两者都要接。
/// 业务面与管理面各注册一份（tokio 允许多个 listener 监听同一信号，任一面收到都会停）。
async fn shutdown_signal() {
    let ctrl_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            tracing::error!("注册 Ctrl-C 监听失败，等待 SIGTERM 兜底");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(err) => {
                tracing::error!(error = %err, "注册 SIGTERM 监听失败");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("收到 Ctrl-C，开始优雅关闭"),
        _ = terminate => tracing::info!("收到 SIGTERM，开始优雅关闭"),
    }
}

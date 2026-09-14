//! pardon-daemon 库面：供集成测试（tests/）与 main 复用。

pub mod clip;
pub mod handler;
pub mod http;
pub mod state;
/// 测试假实现。常驻编译（不能 cfg(test)——集成测试以普通依赖编译 lib，
/// cfg(test) 模块对外不可见）；doc(hidden) 使其不出现在文档。
#[doc(hidden)]
pub mod testing;

use anyhow::Context;
use clap::Parser;
use std::sync::Arc;

#[derive(Parser)]
#[command(
    name = "pardond",
    version,
    about = "pardon daemon — clipboard translation"
)]
pub struct Args {
    /// 通知写日志而非桌面通知（headless 调试）
    #[arg(long)]
    pub log_notify: bool,
}

/// 启动 daemon。config 只在启动时读一次（引擎名 Box::leak 会随 Pipeline
/// 重建累积，热重载被有意排除——改配置请重启服务，spec M2 约束）。
pub async fn run(args: Args) -> anyhow::Result<()> {
    let cfg = pardon_core::config::load().map_err(|e| {
        anyhow::anyhow!(
            "config ({}): {e}",
            pardon_core::config::config_path().display()
        )
    })?;
    let bind = pardon_core::config::effective_http_bind(&cfg)?;

    let pipeline = pardon_core::pipeline::Pipeline::from_config(&cfg)?;
    log::info!(
        "pipeline ready (engine chain default: {})",
        cfg.default_engine
    );
    let translator = Arc::new(state::PipelineTranslator {
        pipeline: Arc::new(tokio::sync::Mutex::new(pipeline)),
    });

    let notifier_timeout = cfg.daemon.notify_timeout_ms;
    let notifier: Arc<dyn pardon_platform::Notifier> = if args.log_notify {
        Arc::new(pardon_platform::LogNotifier)
    } else {
        Arc::new(pardon_platform::linux::DesktopNotifier::new(
            notifier_timeout,
        ))
    };

    // wl-clipboard 缺失 → 降级占位（触发口返回 503），不退出
    let clipboard: Arc<dyn pardon_platform::ClipboardAccess> =
        match pardon_platform::linux::WaylandClipboard::new() {
            Ok(c) => Arc::new(c),
            Err(e) => {
                log::warn!("clipboard unavailable, degraded mode: {e:#}");
                Arc::new(pardon_platform::UnavailableClipboard {
                    reason: format!("{e:#}"),
                })
            }
        };

    let state = Arc::new(state::DaemonState {
        guard: tokio::sync::Mutex::new(pardon_core::loopguard::LoopGuard::new(
            std::time::Duration::from_millis(cfg.daemon.dedup_window_ms),
        )),
        cfg,
        started: std::time::Instant::now(),
        translator,
        notifier,
        clipboard,
        counters: Default::default(),
        clipboard_watching: std::sync::atomic::AtomicBool::new(false),
        shutdown: Arc::new(tokio::sync::Notify::new()),
    });

    if state.cfg.daemon.auto_translate {
        match pardon_platform::linux::WaylandMonitor::new() {
            Ok(m) => {
                clip::start(state.clone(), Box::new(m)).await?;
                state
                    .clipboard_watching
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                log::info!("clipboard watching enabled");
            }
            Err(e) => {
                log::warn!("clipboard watching disabled: {e:#} (http trigger still available)")
            }
        }
    } else {
        log::info!("auto_translate off");
    }

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("bind {bind} (port in use? another pardond may be running)"))?;
    let actual = listener.local_addr()?;
    println!(
        "pardond {} listening on http://{}",
        pardon_core::VERSION,
        actual
    );

    let shutdown_state = state.clone();
    axum::serve(listener, http::router(state))
        .with_graceful_shutdown(shutdown_signal(shutdown_state))
        .await?;
    log::info!("pardond stopped");
    Ok(())
}

async fn shutdown_signal(state: Arc<state::DaemonState>) {
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("install SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = term.recv() => {},
        _ = state.shutdown.notified() => {},
    }
}

//! pardon-daemon 库面：供集成测试（tests/）与 main 复用。

pub mod clip;
pub mod events;
pub mod handler;
pub mod http;
pub mod state;
pub mod status;
/// 测试假实现。常驻编译（不能 cfg(test)——集成测试以普通依赖编译 lib，
/// cfg(test) 模块对外不可见）；doc(hidden) 使其不出现在文档。
#[doc(hidden)]
pub mod testing;
pub mod trigger;
pub mod uds;
pub mod watcher;

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

/// 启动 daemon。`[daemon]` 配置段支持热重载（[`apply_config`]）；引擎字段
/// （default_engine/[llm]）不热应用——引擎名 Box::leak 会随 Pipeline 重建
/// 累积，引擎变更需重启服务（spec M2 约束）。
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

    // 翻译历史：打开失败 → None 降级（不影响翻译）
    let history = match pardon_core::history::History::open(&pardon_core::history::history_path()?)
    {
        Ok(h) => Some(Arc::new(tokio::sync::Mutex::new(h))),
        Err(e) => {
            log::warn!("history unavailable: {e:#}");
            None
        }
    };
    // 事件广播：不留占位接收者——send 在无 UDS 订阅者时报错，正是
    // popup 回退桌面通知的判据（Task 5 起每个 GUI 连接订阅一路）。
    let (events_tx, _) = tokio::sync::broadcast::channel::<String>(64);
    let (watcher_stop, _) = tokio::sync::watch::channel(false);

    let state = Arc::new(state::DaemonState {
        guard: tokio::sync::Mutex::new(pardon_core::loopguard::LoopGuard::new(
            std::time::Duration::from_millis(cfg.daemon.dedup_window_ms),
        )),
        cfg: std::sync::RwLock::new(cfg),
        started: std::time::Instant::now(),
        translator,
        notifier,
        clipboard,
        counters: Default::default(),
        clipboard_watching: std::sync::atomic::AtomicBool::new(false),
        shutdown: Arc::new(tokio::sync::Notify::new()),
        watcher_stop,
        history,
        events: events_tx,
    });

    // 剪贴板监听开关统一走 watcher 入口（wl-clipboard 缺失 → 降级 warn，
    // HTTP 触发口仍可用）
    let auto = state.cfg_snapshot().daemon.auto_translate;
    match watcher::set_enabled(&state, auto).await {
        Ok(()) if auto => log::info!("clipboard watching enabled"),
        Ok(()) => log::info!("auto_translate off"),
        Err(e) => log::warn!("clipboard watching disabled: {e:#} (http trigger still available)"),
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

    // UDS IPC（GUI）：绑定失败只 error 日志降级，daemon 不退——HTTP 口仍
    // 可用（占用场景是另一 pardond 或残留套接字，与 HTTP bind 失败不同源）
    let uds_state = state.clone();
    let uds_shutdown = state.shutdown.clone();
    tokio::spawn(async move {
        if let Err(e) = uds::serve(uds_state, uds_shutdown).await {
            log::error!("ipc server: {e:#}");
        }
    });

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
        // 信号路径也要广播：uds::serve 在等同一 Notify（清理套接字文件）
        _ = tokio::signal::ctrl_c() => state.shutdown.notify_waiters(),
        _ = term.recv() => state.shutdown.notify_waiters(),
        _ = state.shutdown.notified() => {},
    }
}

/// 应用磁盘新配置：`[daemon]` 字段热生效（含监听开关）；引擎字段
/// （default_engine/[llm]）不热应用，返回 restart_required=true。
pub async fn apply_config(state: &Arc<state::DaemonState>) -> anyhow::Result<bool> {
    let fresh = pardon_core::config::load()?;
    // 写锁收口在同步段（不跨 await；set_enabled 内部自己拿 cfg_snapshot）
    let (restart_required, auto_fresh) = {
        let mut cur = state.cfg.write().expect("cfg lock poisoned");
        let restart_required = fresh.default_engine != cur.default_engine || fresh.llm != cur.llm;
        let auto_fresh = fresh.daemon.auto_translate;
        cur.daemon = fresh.daemon;
        (restart_required, auto_fresh)
    };
    let auto_now = state
        .clipboard_watching
        .load(std::sync::atomic::Ordering::Acquire);
    if auto_fresh != auto_now {
        if let Err(e) = watcher::set_enabled(state, auto_fresh).await {
            log::warn!("watcher toggle after reload failed: {e:#}");
        }
    }
    Ok(restart_required)
}

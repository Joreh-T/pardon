//! 状态快照（HTTP /status 与 UDS `status` 方法共用）。

use crate::state::DaemonState;
use serde::Serialize;
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[derive(Serialize)]
pub struct StatusResponse {
    pub version: &'static str,
    pub uptime_s: u64,
    pub clipboard_watching: bool,
    pub auto_translate: bool,
    pub default_engine: String,
    pub counters: StatusCounters,
    /// UDS 事件订阅连接数（GUI 实例数）。
    pub gui_connected: usize,
    /// 词卡是否显示学习徽章行（GUI 词卡渲染依据；通知路径 handler 直接读 cfg）。
    pub show_word_badge: bool,
}

#[derive(Serialize)]
pub struct StatusCounters {
    pub clipboard_events: u64,
    pub translations: u64,
    pub notifications: u64,
    pub triggers: u64,
}

pub fn snapshot(state: &Arc<DaemonState>) -> StatusResponse {
    let cfg = state.cfg_snapshot();
    StatusResponse {
        version: pardon_core::VERSION,
        uptime_s: state.started.elapsed().as_secs(),
        clipboard_watching: state.clipboard_watching.load(Ordering::Relaxed),
        auto_translate: cfg.daemon.auto_translate,
        default_engine: cfg.default_engine.clone(),
        counters: StatusCounters {
            clipboard_events: state.counters.clipboard_events.load(Ordering::Relaxed),
            translations: state.counters.translations.load(Ordering::Relaxed),
            notifications: state.counters.notifications.load(Ordering::Relaxed),
            triggers: state.counters.triggers.load(Ordering::Relaxed),
        },
        gui_connected: state.events.receiver_count(),
        show_word_badge: cfg.daemon.show_word_badge,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::DaemonState;
    use crate::testing::*;
    use pardon_core::config::AppConfig;
    use std::sync::atomic::AtomicBool;

    /// 纯构造 state（快照测试不触翻译/通知/剪贴板，假实现即可）。
    fn state(cfg: AppConfig) -> Arc<DaemonState> {
        Arc::new(DaemonState {
            guard: tokio::sync::Mutex::new(pardon_core::loopguard::LoopGuard::new(
                std::time::Duration::from_secs(10),
            )),
            cfg: std::sync::RwLock::new(cfg),
            started: std::time::Instant::now(),
            translator: Arc::new(FakeTranslator::new()),
            notifier: Arc::new(FakeNotifier::default()),
            clipboard: Arc::new(FakeClipboard::default()),
            counters: Default::default(),
            clipboard_watching: AtomicBool::new(false),
            shutdown: Arc::new(tokio::sync::Notify::new()),
            watcher_stop: tokio::sync::watch::channel(false).0,
            history: None,
            events: tokio::sync::broadcast::channel(64).0,
        })
    }

    /// status 快照携带 show_word_badge（GUI 词卡渲染依据）。
    #[test]
    fn snapshot_exposes_show_word_badge() {
        let mut cfg = AppConfig::default();
        cfg.daemon.show_word_badge = true;
        assert!(snapshot(&state(cfg)).show_word_badge);
        let mut cfg = AppConfig::default();
        cfg.daemon.show_word_badge = false;
        assert!(!snapshot(&state(cfg)).show_word_badge);
    }
}

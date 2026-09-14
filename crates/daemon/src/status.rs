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
    }
}

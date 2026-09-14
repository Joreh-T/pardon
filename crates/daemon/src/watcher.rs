//! 剪贴板监听实时开关（托盘勾选 / UDS reload 共用）。

use crate::clip;
use crate::state::DaemonState;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// 生产入口：monitor 构造固定为 WaylandMonitor。
pub async fn set_enabled(state: &Arc<DaemonState>, enabled: bool) -> anyhow::Result<()> {
    set_enabled_with_monitor(state, enabled, make_wayland_monitor).await
}

/// WaylandMonitor 无 new_boxed → 私有适配 fn（trait object 装箱）。
fn make_wayland_monitor() -> anyhow::Result<Box<dyn pardon_platform::ClipboardMonitor>> {
    Ok(Box::new(pardon_platform::linux::WaylandMonitor::new()?))
}

/// 可测内核。true：启动监听桥（若未运行）；false：请求消费循环退出。
/// 幂等：状态已达标时 no-op。
pub async fn set_enabled_with_monitor(
    state: &Arc<DaemonState>,
    enabled: bool,
    make_monitor: impl Fn() -> anyhow::Result<Box<dyn pardon_platform::ClipboardMonitor>>,
) -> anyhow::Result<()> {
    let running = state.clipboard_watching.load(Ordering::Acquire);
    if enabled == running {
        return Ok(());
    }
    if enabled {
        // 先复位 false：新消费循环订阅到的初值必须是「运行中」，
        // 再构造 monitor 并启动桥，最后置位 watching
        state.watcher_stop.send_replace(false);
        let monitor = make_monitor()?;
        clip::start(state.clone(), monitor).await?;
        state.clipboard_watching.store(true, Ordering::Release);
    } else {
        state.watcher_stop.send_replace(true);
        state.clipboard_watching.store(false, Ordering::Release);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::state::DaemonState;
    use crate::testing::*;
    use pardon_core::config::AppConfig;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    /// 字面构造 DaemonState（无辅助构造器）：默认配置 + 假实现。
    async fn bare_state(auto: bool) -> Arc<DaemonState> {
        let mut cfg = AppConfig::default();
        cfg.daemon.auto_translate = auto;
        Arc::new(DaemonState {
            guard: tokio::sync::Mutex::new(pardon_core::loopguard::LoopGuard::new(
                std::time::Duration::from_millis(cfg.daemon.dedup_window_ms),
            )),
            cfg: std::sync::RwLock::new(cfg),
            started: std::time::Instant::now(),
            translator: Arc::new(FakeTranslator::new()),
            notifier: Arc::new(FakeNotifier::default()),
            clipboard: Arc::new(FakeClipboard::default()),
            counters: Default::default(),
            clipboard_watching: std::sync::atomic::AtomicBool::new(false),
            shutdown: Arc::new(tokio::sync::Notify::new()),
            watcher_stop: tokio::sync::watch::channel(false).0,
            history: None,
            events: tokio::sync::broadcast::channel(64).0,
        })
    }

    #[tokio::test]
    async fn stop_when_not_running_is_noop() {
        let s = bare_state(false).await;
        super::set_enabled(&s, false).await.unwrap(); // 未运行 → no-op 不报错
        assert!(!s.clipboard_watching.load(Ordering::Relaxed));
    }

    /// Drop 探针假 monitor：next_event 永久挂起；被 drop 时置位探针
    /// （桥任务退出 = monitor 被drop = wl-paste 子进程被 kill_on_drop 收掉）。
    struct ProbeMonitor {
        dropped: Arc<std::sync::atomic::AtomicBool>,
    }
    impl Drop for ProbeMonitor {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::Relaxed);
        }
    }
    #[async_trait::async_trait]
    impl pardon_platform::ClipboardMonitor for ProbeMonitor {
        async fn next_event(&mut self) -> anyhow::Result<String> {
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn enable_starts_and_disable_stops_bridge_and_reaps_monitor() {
        let s = bare_state(false).await;
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let probe = dropped.clone();
        super::set_enabled_with_monitor(&s, true, move || {
            Ok(Box::new(ProbeMonitor {
                dropped: probe.clone(),
            }))
        })
        .await
        .unwrap();
        assert!(
            s.clipboard_watching.load(Ordering::Relaxed),
            "启用后 watching=true"
        );

        // 停用走真实路径（send_replace(true) + store(false)）：桥任务 select
        // 到停止信号 → 退出并 drop monitor（不等下一次剪贴板事件）
        super::set_enabled_with_monitor(&s, false, || unreachable!())
            .await
            .unwrap();
        let reaped = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while !dropped.load(Ordering::Relaxed) {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(
            reaped.is_ok(),
            "1s 内桥任务应退出并 drop monitor（wl-paste 子进程被收）"
        );
        assert!(!s.clipboard_watching.load(Ordering::Relaxed));
    }
}

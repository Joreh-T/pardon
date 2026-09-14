//! 剪贴板事件桥：monitor（断线自动重启）→ channel → 合并消费。

use crate::handler::{handle_text, Origin};
use crate::state::DaemonState;
use pardon_platform::ClipboardMonitor;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// 启动监听桥与消费循环（默认 3s 退避）。返回消费任务句柄。
pub async fn start(
    state: Arc<DaemonState>,
    monitor: Box<dyn ClipboardMonitor>,
) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    start_with_backoff(state, monitor, Duration::from_secs(3)).await
}

pub async fn start_with_backoff(
    state: Arc<DaemonState>,
    mut monitor: Box<dyn ClipboardMonitor>,
    backoff: Duration,
) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    let (tx, mut rx) = mpsc::channel::<String>(64);

    // 桥任务：watcher 断线 → 退避重启（合成器重连、Niri 重启场景自愈）；
    // 同时 select watcher_stop——停用时立即退出并 drop monitor（其内部
    // session 的 kill_on_drop 随之收掉 wl-paste 子进程，无需等下一次
    // 剪贴板事件）。spawn 前订阅，确保停用方的信号翻转必被捕获。
    let mut bridge_stop = state.watcher_stop.subscribe();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                changed = bridge_stop.changed() => {
                    let ended = changed.is_err() || *bridge_stop.borrow();
                    if ended { break; }
                }
                ev = monitor.next_event() => {
                    match ev {
                        Ok(text) => {
                            if tx.send(text).await.is_err() {
                                break; // 消费端已退出（daemon 关停）
                            }
                        }
                        Err(e) => {
                            log::warn!("clipboard monitor ended: {e:#}; retrying in {backoff:?}");
                            tokio::time::sleep(backoff).await;
                        }
                    }
                }
            }
        }
    });

    // 消费任务：串行处理；处理期间堆积的事件只保留最新（快速连续复制合并）。
    // select watcher_stop：托盘/reload 实时停用监听时请求退出（spawn 前订阅，
    // 确保停用方在 start 之后的信号翻转必被捕获；值仍为 false 时继续循环）。
    let mut stop = state.watcher_stop.subscribe();
    let consumer = tokio::spawn(async move {
        loop {
            tokio::select! {
                changed = stop.changed() => {
                    let ended = changed.is_err() || *stop.borrow();
                    if ended { break; }
                }
                maybe = rx.recv() => {
                    let Some(mut text) = maybe else { break };
                    while let Ok(newer) = rx.try_recv() {
                        text = newer;
                    }
                    let _ = handle_text(&state, &text, Origin::Auto).await;
                }
            }
        }
    });
    Ok(consumer)
}

#[cfg(test)]
mod tests {
    use crate::state::DaemonState;
    use crate::testing::*;
    use pardon_core::config::AppConfig;
    use pardon_platform::ClipboardMonitor;
    use std::sync::Arc;
    use std::time::Duration;

    /// 脚本化假监听：每次 next_event 延时后吐下一条；耗尽后永久挂起。
    struct FakeMonitor {
        script: Vec<(u64, String)>,
    }

    #[async_trait::async_trait]
    impl ClipboardMonitor for FakeMonitor {
        async fn next_event(&mut self) -> anyhow::Result<String> {
            match self.script.first() {
                Some((delay, text)) => {
                    let (delay, text) = (*delay, text.clone());
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    self.script.remove(0);
                    Ok(text)
                }
                None => std::future::pending::<anyhow::Result<String>>().await,
            }
        }
    }

    async fn state_with(
        translator: Arc<FakeTranslator>,
        notifier: Arc<FakeNotifier>,
    ) -> Arc<DaemonState> {
        let cfg = AppConfig::default();
        Arc::new(DaemonState {
            guard: tokio::sync::Mutex::new(pardon_core::loopguard::LoopGuard::new(
                Duration::from_millis(cfg.daemon.dedup_window_ms),
            )),
            cfg: std::sync::RwLock::new(cfg),
            started: std::time::Instant::now(),
            translator,
            notifier,
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
    async fn burst_coalesces_to_latest() {
        // 翻译耗时 150ms；事件 0/40/80ms 到达 → 只处理首条与最后一条？
        // 不：处理首条期间后两条入队，处理完只消费最新（第三条）→ 恰 2 次翻译、
        // 2 条通知，第二条通知文本 = 第三条事件的译文。
        let tr = Arc::new(FakeTranslator::delayed(150, |t| {
            FakeTranslator::sentence(t, "（译）", "glm")
        }));
        let no = Arc::new(FakeNotifier::default());
        let state = state_with(tr.clone(), no.clone()).await;
        let monitor = FakeMonitor {
            script: vec![
                (0, "first text".into()),
                (40, "second text".into()),
                (80, "third text".into()),
            ],
        };
        super::start(state, Box::new(monitor)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        let sent = no.sent.lock().unwrap();
        assert_eq!(sent.len(), 2, "coalesce: {sent:?}");
        assert!(sent[1].0.contains("third text"), "{:?}", sent[1]);
        assert_eq!(tr.calls.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn monitor_error_is_retried_not_fatal() {
        // 第一轮立即失败（watcher 退出），退避后第二轮吐事件。
        // 退避 3s 太长会拖慢测试 → 把退避做成可注入：
        // start_with_backoff(state, monitor, backoff: Duration)；
        // start() = start_with_backoff(.., Duration::from_secs(3))。
        let tr = Arc::new(FakeTranslator::default());
        let no = Arc::new(FakeNotifier::default());
        let state = state_with(tr, no.clone()).await;
        struct FlakyThenOk {
            first: bool,
        }
        #[async_trait::async_trait]
        impl ClipboardMonitor for FlakyThenOk {
            async fn next_event(&mut self) -> anyhow::Result<String> {
                if self.first {
                    self.first = false;
                    anyhow::bail!("watcher exited");
                }
                Ok("after retry".into())
            }
        }
        super::start_with_backoff(
            state,
            Box::new(FlakyThenOk { first: true }),
            Duration::from_millis(10),
        )
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(no.sent.lock().unwrap().len(), 1);
    }
}

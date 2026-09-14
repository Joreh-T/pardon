//! daemon 共享状态：配置、翻译服务、平台能力、防回环、计数器。

use pardon_core::config::AppConfig;
use pardon_core::dict::WordCard;
use pardon_core::loopguard::LoopGuard;
use pardon_core::pipeline::{Pipeline, Translation};
use pardon_platform::{ClipboardAccess, Notifier};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;
use std::time::Instant;

/// 翻译服务抽象：daemon 逻辑只依赖此 trait（测试用假实现替换真实 Pipeline，
/// 避免 LLM/词典依赖）。
#[async_trait::async_trait]
pub trait Translator: Send + Sync {
    /// 完整分流翻译（词→词卡文本，句→引擎链），语义同 `pardon translate`。
    async fn translate(&self, text: &str) -> Translation;
    /// 查词卡（未收录通知取 suggestions 用）。
    async fn lookup(&self, word: &str) -> WordCard;
}

/// 真实实现。rusqlite `Connection` 非 Sync → 持 `&Pipeline` 跨 await 的
/// future 非 Send，无法装进 `async_trait` 的 Send boxed future（tokio Mutex
/// 只解决 guard 本身跨 await，解决不了 `&Pipeline`）；故 `translate` 在
/// blocking 线程上用 `Handle::block_on` 跑完再交还——依赖多线程 runtime
/// 驱动 IO/timer（Task 10 以 rt-multi-thread 启动 daemon）。tokio Mutex
/// 保证并发调用对 Pipeline 的互斥。
pub struct PipelineTranslator {
    pub pipeline: Arc<tokio::sync::Mutex<Pipeline>>,
}

#[async_trait::async_trait]
impl Translator for PipelineTranslator {
    async fn translate(&self, text: &str) -> Translation {
        let pipeline = Arc::clone(&self.pipeline);
        let text = text.to_string();
        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            handle.block_on(async {
                let pipeline = pipeline.lock().await;
                pipeline.translate(&text).await
            })
        })
        .await
        .expect("pipeline translate task panicked")
    }

    async fn lookup(&self, word: &str) -> WordCard {
        self.pipeline.lock().await.lookup(word)
    }
}

#[derive(Default)]
pub struct Counters {
    pub clipboard_events: AtomicU64,
    pub translations: AtomicU64,
    pub notifications: AtomicU64,
    pub triggers: AtomicU64,
}

pub struct DaemonState {
    /// 配置：读多写少（每事件读快照）；reload 只写 daemon 字段。
    pub cfg: std::sync::RwLock<AppConfig>,
    pub started: Instant,
    pub translator: Arc<dyn Translator>,
    pub notifier: Arc<dyn Notifier>,
    pub clipboard: Arc<dyn ClipboardAccess>,
    /// 防回环 + 同内容去重（Auto 事件与译文写回共用一把锁）。
    pub guard: tokio::sync::Mutex<LoopGuard>,
    pub counters: Counters,
    /// 剪贴板监听是否启动（wl-clipboard 缺失时 false，HTTP 触发口仍可用）。
    pub clipboard_watching: AtomicBool,
    /// `/shutdown` 与 SIGTERM/SIGINT 共用的关停信号。
    pub shutdown: Arc<tokio::sync::Notify>,
    /// 剪贴板监听停止信号（watch：true=请求停止；托盘/reload 实时开关用）。
    pub watcher_stop: tokio::sync::watch::Sender<bool>,
    /// 翻译历史（打开失败 → None 降级，不影响翻译）。
    pub history: Option<Arc<tokio::sync::Mutex<pardon_core::history::History>>>,
    /// 事件广播（events.rs 的 JSON 行）→ UDS 订阅者（GUI）。
    pub events: tokio::sync::broadcast::Sender<String>,
}

impl DaemonState {
    /// 配置快照：读锁 + clone，调用点一行拿到一致视图。
    pub fn cfg_snapshot(&self) -> AppConfig {
        self.cfg.read().expect("cfg lock poisoned").clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::*;

    /// PARDON_CONFIG 是进程级全局 → 篡改它的测试共用串行锁
    /// （testing::CONFIG_ENV_LOCK；tray.rs 的持久化测试也持有它）。
    #[tokio::test]
    async fn apply_config_live_fields_and_restart_flag() {
        let _lock = crate::testing::CONFIG_ENV_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("config.toml");
        std::fs::write(&cfg_path, "[daemon]\nauto_translate = false\n").unwrap();
        std::env::set_var("PARDON_CONFIG", &cfg_path);
        // 初始：auto_translate=true / show_word_badge=true / default_engine=youdao，
        // watching 未启动（false）→ reload 翻转只改 cfg，无需启动监听
        let mut cfg = AppConfig::default();
        cfg.daemon.show_word_badge = true;
        let state = Arc::new(DaemonState {
            guard: tokio::sync::Mutex::new(LoopGuard::new(std::time::Duration::from_millis(
                cfg.daemon.dedup_window_ms,
            ))),
            cfg: std::sync::RwLock::new(cfg),
            started: Instant::now(),
            translator: Arc::new(FakeTranslator::new()),
            notifier: Arc::new(FakeNotifier::default()),
            clipboard: Arc::new(FakeClipboard::default()),
            counters: Default::default(),
            clipboard_watching: AtomicBool::new(false),
            shutdown: Arc::new(tokio::sync::Notify::new()),
            watcher_stop: tokio::sync::watch::channel(false).0,
            history: None,
            events: tokio::sync::broadcast::channel(64).0,
        });
        let restart = crate::apply_config(&state).await.unwrap();
        assert!(!restart, "daemon 字段变更不需要重启");
        assert!(!state.cfg_snapshot().daemon.auto_translate);
        assert!(
            !state.cfg_snapshot().daemon.show_word_badge,
            "daemon 段整体替换"
        );
        // 引擎字段变更 → restart_required
        std::fs::write(
            &cfg_path,
            "default_engine = \"llm\"\n[llm]\ndefault_provider=\"x\"\n[[llm.providers]]\nid=\"x\"\ntype=\"openai\"\nbase_url=\"http://x\"\nmodel=\"m\"\n[daemon]\nauto_translate = false\n",
        )
        .unwrap();
        let restart = crate::apply_config(&state).await.unwrap();
        assert!(restart);
        std::env::remove_var("PARDON_CONFIG");
    }
}

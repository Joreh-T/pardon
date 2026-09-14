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
    pub cfg: AppConfig,
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
}

//! HTTP 层集成测试：真实 bind 127.0.0.1:0 + reqwest。

use pardon_core::config::AppConfig;
use pardon_core::pipeline::Translation;
use pardon_daemon::http::router;
use pardon_daemon::state::DaemonState;
use pardon_daemon::testing::{FakeClipboard, FakeNotifier, FakeTranslator};
use std::sync::Arc;
use std::time::Duration;

fn translation(text: &str, out: &str, engine: &str) -> Translation {
    Translation {
        source_lang: pardon_core::lang::Lang::En,
        target_lang: pardon_core::lang::Lang::Zh,
        text: text.into(),
        translation: out.into(),
        engine: engine.into(),
    }
}

/// 起 router 于随机端口，返回 (base_url, state)——shutdown 测试需要
/// 拿 state.shutdown 断言 notify 触发。
async fn spawn_router(
    translator: Arc<FakeTranslator>,
    notifier: Arc<FakeNotifier>,
    clipboard: Arc<FakeClipboard>,
) -> (String, Arc<DaemonState>) {
    let cfg = AppConfig::default();
    let state = Arc::new(DaemonState {
        guard: tokio::sync::Mutex::new(pardon_core::loopguard::LoopGuard::new(
            Duration::from_millis(cfg.daemon.dedup_window_ms),
        )),
        cfg: std::sync::RwLock::new(cfg),
        started: std::time::Instant::now(),
        translator,
        notifier,
        clipboard,
        counters: Default::default(),
        clipboard_watching: std::sync::atomic::AtomicBool::new(true),
        shutdown: Arc::new(tokio::sync::Notify::new()),
        watcher_stop: tokio::sync::watch::channel(false).0,
        history: None,
        events: tokio::sync::broadcast::channel(64).0,
    });
    let app = router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), state)
}

#[tokio::test]
async fn translate_returns_translation_json() {
    let tr = Arc::new(FakeTranslator::with_map(vec![(
        "hello world".into(),
        translation("hello world", "你好世界", "glm"),
    )]));
    let (base, _state) = spawn_router(
        tr,
        Arc::new(FakeNotifier::default()),
        Arc::new(FakeClipboard::default()),
    )
    .await;
    let resp: serde_json::Value = reqwest::Client::new()
        .post(format!("{base}/translate"))
        .json(&serde_json::json!({ "text": "hello world" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["translation"], "你好世界");
    assert_eq!(resp["engine"], "glm");
}

#[tokio::test]
async fn translate_empty_text_is_400() {
    let (base, _state) = spawn_router(
        Arc::new(FakeTranslator::default()),
        Arc::new(FakeNotifier::default()),
        Arc::new(FakeClipboard::default()),
    )
    .await;
    let resp = reqwest::Client::new()
        .post(format!("{base}/translate"))
        .json(&serde_json::json!({ "text": "   " }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn trigger_selection_notifies_and_returns_translation() {
    let tr = Arc::new(FakeTranslator::with_map(vec![(
        "primary text".into(),
        translation("primary text", "主文本", "glm"),
    )]));
    let no = Arc::new(FakeNotifier::default());
    let mut clip = FakeClipboard::default();
    clip.primary_result = Ok("primary text".into());
    let (base, _state) = spawn_router(tr, no.clone(), Arc::new(clip)).await;
    let resp: serde_json::Value = reqwest::get(format!("{base}/trigger/selection"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["notified"], true);
    assert_eq!(resp["translation"]["translation"], "主文本");
    assert_eq!(no.sent.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn trigger_read_empty_maps_to_no_text() {
    let mut clip = FakeClipboard::default();
    clip.primary_result = Err(anyhow::anyhow!(
        "wl-paste failed: Clipboard content is not available as requested type \"text/plain\""
    ));
    let (base, _state) = spawn_router(
        Arc::new(FakeTranslator::default()),
        Arc::new(FakeNotifier::default()),
        Arc::new(clip),
    )
    .await;
    let resp: serde_json::Value = reqwest::get(format!("{base}/trigger/selection"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["notified"], false);
    assert_eq!(resp["reason"], "no text available");
    assert_eq!(resp["translation"], serde_json::Value::Null);
}

#[tokio::test]
async fn trigger_hard_error_is_503() {
    let mut clip = FakeClipboard::default();
    clip.primary_result = Err(anyhow::anyhow!("compositor connection refused"));
    let (base, _state) = spawn_router(
        Arc::new(FakeTranslator::default()),
        Arc::new(FakeNotifier::default()),
        Arc::new(clip),
    )
    .await;
    let resp = reqwest::get(format!("{base}/trigger/selection"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 503);
}

#[tokio::test]
async fn status_reports_counters_and_watching() {
    let (base, _state) = spawn_router(
        Arc::new(FakeTranslator::default()),
        Arc::new(FakeNotifier::default()),
        Arc::new(FakeClipboard::default()),
    )
    .await;
    let resp: serde_json::Value = reqwest::get(format!("{base}/status"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["clipboard_watching"], true);
    assert_eq!(resp["version"], pardon_core::VERSION);
    assert!(resp["counters"]["translations"].is_u64());
}

#[tokio::test]
async fn shutdown_fires_notify() {
    let (base, state) = spawn_router(
        Arc::new(FakeTranslator::default()),
        Arc::new(FakeNotifier::default()),
        Arc::new(FakeClipboard::default()),
    )
    .await;
    let resp = reqwest::Client::new()
        .post(format!("{base}/shutdown"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    tokio::time::timeout(Duration::from_secs(1), state.shutdown.notified())
        .await
        .expect("shutdown notify should fire within 1s");
}

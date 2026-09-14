//! UDS IPC 集成测试：真实 serve 循环 + interprocess 客户端往返。
//! PARDON_IPC_SOCK 是进程级全局 → 两个测试经 SOCK_LOCK 串行
//! （tokio Mutex：守卫需跨 await 持有，std 锁会触发 clippy::await_holding_lock）。

use pardon_core::config::AppConfig;
use pardon_daemon::state::DaemonState;
use pardon_daemon::testing::{FakeClipboard, FakeNotifier, FakeTranslator};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

static SOCK_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 字面构造 DaemonState（照 handler tests 模式，含 Task 4 新字段）。
fn test_state(
    cfg: AppConfig,
    history: Option<Arc<tokio::sync::Mutex<pardon_core::history::History>>>,
) -> Arc<DaemonState> {
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
        history,
        events: tokio::sync::broadcast::channel(64).0,
    })
}

/// 起 serve 并等套接字文件就绪（1s 超时）。
async fn spawn_serve(state: &Arc<DaemonState>, sock: std::path::PathBuf) {
    let s2 = state.clone();
    let shutdown = Arc::new(tokio::sync::Notify::new());
    tokio::spawn(async move {
        let _ = pardon_daemon::uds::serve(s2, shutdown).await;
    });
    for _ in 0..100 {
        if sock.exists() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("ipc socket not created within 1s: {}", sock.display());
}

/// interprocess 客户端连接（Linux 上 fs-name 本地套接字即 UDS）。
async fn connect(sock: &std::path::Path) -> interprocess::local_socket::tokio::Stream {
    // tokio::prelude 才带 tokio Stream trait（connect）；sync prelude 不行
    use interprocess::local_socket::tokio::prelude::*;
    let name = sock
        .to_str()
        .unwrap()
        .to_fs_name::<interprocess::local_socket::GenericFilePath>()
        .unwrap();
    interprocess::local_socket::tokio::Stream::connect(name)
        .await
        .unwrap()
}

/// 每连接一次 `tokio::io::split` + BufReader lines，`send`/`recv` 复用到底
/// （每请求重建 BufReader 会丢缓冲）。
async fn send(w: &mut tokio::io::WriteHalf<interprocess::local_socket::tokio::Stream>, line: &str) {
    w.write_all(line.as_bytes()).await.unwrap();
    w.write_all(b"\n").await.unwrap();
    w.flush().await.unwrap();
}

async fn recv(
    lines: &mut tokio::io::Lines<
        BufReader<tokio::io::ReadHalf<interprocess::local_socket::tokio::Stream>>,
    >,
) -> serde_json::Value {
    let line = lines
        .next_line()
        .await
        .unwrap()
        .expect("connection closed before response");
    serde_json::from_str(&line).unwrap_or_else(|e| panic!("bad json line ({e}): {line}"))
}

#[tokio::test]
async fn ping_status_translate_lookup_roundtrip() {
    let _l = SOCK_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("ipc.sock");
    std::env::set_var("PARDON_IPC_SOCK", &sock);
    let hist = Arc::new(tokio::sync::Mutex::new(
        pardon_core::history::History::open(&dir.path().join("h.sqlite")).unwrap(),
    ));
    let state = test_state(AppConfig::default(), Some(hist));
    spawn_serve(&state, sock.clone()).await;

    let stream = connect(&sock).await;
    let (r, mut w) = tokio::io::split(stream);
    let mut lines = BufReader::new(r).lines();

    send(&mut w, r#"{"id":1,"method":"ping","params":{}}"#).await;
    let v = recv(&mut lines).await;
    assert_eq!(v["id"], 1);
    assert!(v["ok"].as_bool().unwrap());
    assert_eq!(v["result"]["version"], pardon_core::VERSION);

    send(&mut w, r#"{"id":2,"method":"status","params":{}}"#).await;
    let v = recv(&mut lines).await;
    assert!(v["ok"].as_bool().unwrap(), "{v}");
    assert_eq!(v["result"]["gui_connected"], 0, "{v}");
    assert_eq!(v["result"]["clipboard_watching"], false, "{v}");

    send(
        &mut w,
        r#"{"id":3,"method":"translate","params":{"text":"hello world"}}"#,
    )
    .await;
    let v = recv(&mut lines).await;
    assert!(v["ok"].as_bool().unwrap(), "{v}");
    assert_eq!(v["result"]["translation"], "（译文）"); // FakeTranslator 默认

    // 空文本 → 错误响应
    send(
        &mut w,
        r#"{"id":4,"method":"translate","params":{"text":"   "}}"#,
    )
    .await;
    let v = recv(&mut lines).await;
    assert!(!v["ok"].as_bool().unwrap(), "{v}");

    send(
        &mut w,
        r#"{"id":5,"method":"lookup","params":{"word":"run"}}"#,
    )
    .await;
    let v = recv(&mut lines).await;
    assert_eq!(v["result"]["found"], false); // FakeTranslator 默认 miss 卡

    // 刚才的 translate 落历史（origin "gui"）
    send(&mut w, r#"{"id":6,"method":"history","params":{}}"#).await;
    let v = recv(&mut lines).await;
    assert!(v["ok"].as_bool().unwrap(), "{v}");
    let entries = v["result"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "{v}");
    assert_eq!(entries[0]["origin"], "gui");
    assert_eq!(entries[0]["text"], "hello world");

    // trigger 空剪贴板（FakeClipboard 默认 Ok("")）→ no text available
    send(
        &mut w,
        r#"{"id":7,"method":"trigger","params":{"source":"clipboard"}}"#,
    )
    .await;
    let v = recv(&mut lines).await;
    assert!(v["ok"].as_bool().unwrap(), "{v}");
    assert_eq!(v["result"]["notified"], false, "{v}");
    assert_eq!(v["result"]["reason"], "no text available", "{v}");

    // trigger 非法 source → 错误响应
    send(
        &mut w,
        r#"{"id":8,"method":"trigger","params":{"source":"bogus"}}"#,
    )
    .await;
    let v = recv(&mut lines).await;
    assert!(!v["ok"].as_bool().unwrap(), "{v}");

    send(&mut w, r#"{"id":9,"method":"badmethod","params":{}}"#).await;
    let v = recv(&mut lines).await;
    assert!(!v["ok"].as_bool().unwrap());

    std::env::remove_var("PARDON_IPC_SOCK");
}

#[tokio::test]
async fn subscriber_receives_popup_event() {
    let _l = SOCK_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("ipc.sock");
    std::env::set_var("PARDON_IPC_SOCK", &sock);
    let mut cfg = AppConfig::default();
    cfg.daemon.popup = true;
    let state = test_state(cfg, None);
    spawn_serve(&state, sock.clone()).await;

    let stream = connect(&sock).await;
    let (r, mut w) = tokio::io::split(stream);
    let mut lines = BufReader::new(r).lines();

    send(&mut w, r#"{"id":1,"method":"subscribe","params":{}}"#).await;
    let v = recv(&mut lines).await;
    assert!(v["ok"].as_bool().unwrap(), "{v}");
    assert_eq!(v["result"], serde_json::json!({}));

    // 触发一次翻译（Trigger 路径；popup=true → 广播事件行）
    pardon_daemon::handler::handle_text(&state, "hello", pardon_daemon::handler::Origin::Trigger)
        .await;

    // 下一行应是 popup 事件
    let ev = tokio::time::timeout(std::time::Duration::from_secs(5), recv(&mut lines))
        .await
        .expect("popup event within 5s");
    assert_eq!(ev["event"], "popup");
    assert_eq!(ev["params"]["translation"]["text"], "hello");
    std::env::remove_var("PARDON_IPC_SOCK");
}

/// 订阅连接断开后：广播 Receiver（gui_connected）回到 0、无接收者时
/// events.send 返回 Err（popup 回退桌面通知的判据）——转发任务不得泄漏。
#[tokio::test]
async fn disconnecting_subscriber_releases_receiver_and_send_fails() {
    let _l = SOCK_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("ipc.sock");
    std::env::set_var("PARDON_IPC_SOCK", &sock);
    let state = test_state(AppConfig::default(), None);
    spawn_serve(&state, sock.clone()).await;

    // 连接 A：subscribe
    let stream = connect(&sock).await;
    let (r, mut w) = tokio::io::split(stream);
    let mut lines = BufReader::new(r).lines();
    send(&mut w, r#"{"id":1,"method":"subscribe","params":{}}"#).await;
    let v = recv(&mut lines).await;
    assert!(v["ok"].as_bool().unwrap(), "{v}");
    assert_eq!(state.events.receiver_count(), 1, "{v}");

    // 断开 A（两半都 drop 才关流 → 服务端读到 EOF）
    drop(w);
    drop(lines);

    // receiver_count 应回 0（转发任务被 abort；泄漏时永不归零）
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    while state.events.receiver_count() > 0 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "subscriber receiver leaked after disconnect"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    // 无接收者 → send Err：死订阅者不得让 popup 误判「已送达」
    assert!(state
        .events
        .send(r#"{"event":"popup","params":{}}"#.to_string())
        .is_err());

    // 契约面复核：新连接查 status，gui_connected=0
    let stream = connect(&sock).await;
    let (r, mut w) = tokio::io::split(stream);
    let mut lines = BufReader::new(r).lines();
    send(&mut w, r#"{"id":2,"method":"status","params":{}}"#).await;
    let v = recv(&mut lines).await;
    assert_eq!(v["result"]["gui_connected"], 0, "{v}");
    std::env::remove_var("PARDON_IPC_SOCK");
}

//! UDS JSONL IPC 服务器（spec §5.4，GUI 契约）。帧协议见任务 Interfaces：
//! 请求 `{"id":i64,"method":str,"params":obj}`，应答 `{"id":…,"ok":…}`，
//! 事件行 = [`crate::events::Event::to_line`]。每连接：读循环解析请求 →
//! dispatch → 应答行；`subscribe` 后另起转发任务把 events 广播写入同连接
//! （应答与事件共用一个写任务，天然有序）。

use crate::state::DaemonState;
use anyhow::Context;
use interprocess::local_socket::tokio::prelude::*;
use interprocess::local_socket::{GenericFilePath, ListenerOptions, Name};
use serde::Deserialize;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Deserialize)]
struct Request {
    id: i64,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

pub async fn serve(
    state: Arc<DaemonState>,
    shutdown: Arc<tokio::sync::Notify>,
) -> anyhow::Result<()> {
    let path = pardon_core::ipc::socket_path();
    // 父目录仅在本方新建时收紧权限（已存在的目录如 /tmp、XDG_RUNTIME_DIR
    // 不动——chmod 它们会破坏系统约定）
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("mkdir {}", parent.display()))?;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                .with_context(|| format!("chmod 0700 {}", parent.display()))?;
        }
    }
    // 残留套接字：连得上 = 活 daemon 占用；连不上 = 陈旧文件，清掉重绑
    if path.exists() {
        let alive = match fs_name(&path) {
            Ok(n) => interprocess::local_socket::tokio::Stream::connect(n)
                .await
                .is_ok(),
            Err(_) => false,
        };
        if alive {
            anyhow::bail!(
                "ipc socket {} is owned by a running pardond",
                path.display()
            );
        }
        std::fs::remove_file(&path)
            .with_context(|| format!("remove stale socket {}", path.display()))?;
    }
    let listener = ListenerOptions::new()
        .name(fs_name(&path)?)
        .create_tokio()
        .with_context(|| format!("bind ipc socket {}", path.display()))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("chmod 0600 {}", path.display()))?;
    log::info!("ipc listening on {}", path.display());

    loop {
        tokio::select! {
            _ = shutdown.notified() => break,
            accepted = listener.accept() => match accepted {
                Ok(stream) => {
                    let st = state.clone();
                    tokio::spawn(async move { handle_conn(st, stream).await });
                }
                Err(e) => {
                    log::warn!("ipc accept failed: {e}");
                    break;
                }
            },
        }
    }
    let _ = std::fs::remove_file(&path); // 关停清理（best-effort）
    Ok(())
}

fn fs_name(path: &std::path::Path) -> anyhow::Result<Name<'static>> {
    let s = path.to_string_lossy().into_owned();
    Ok(s.to_fs_name::<GenericFilePath>()?)
}

async fn handle_conn(state: Arc<DaemonState>, stream: interprocess::local_socket::tokio::Stream) {
    let (read, write) = tokio::io::split(stream);
    let mut lines = BufReader::new(read).lines();
    // 单写任务：应答与事件行共用，保证同连接有序
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        let mut write = write;
        while let Some(line) = rx.recv().await {
            if write.write_all(line.as_bytes()).await.is_err()
                || write.write_all(b"\n").await.is_err()
                || write.flush().await.is_err()
            {
                break;
            }
        }
    });
    // subscribe 的转发任务句柄：转发任务持有 tx 克隆与 broadcast Receiver，
    // 断连后必须由本方 abort（见读循环后的收尾）——它自己不会退出
    let mut forward: Option<tokio::task::JoinHandle<()>> = None;
    while let Ok(Some(line)) = lines.next_line().await {
        let resp: String = match serde_json::from_str::<Request>(&line) {
            Ok(req) => dispatch(&state, &tx, &mut forward, req).await,
            Err(e) => err_line(0, &format!("bad request: {e}")),
        };
        let _ = tx.send(resp);
    }
    // 断连（EOF）：先 abort 转发任务——任务丢弃即释放 broadcast Receiver
    // （gui_connected 立减）与 tx 克隆；再丢本方 tx → 写任务 recv 返回
    // None 退出 → await 收尾。不 abort 则三任务 + 流两半泄漏，直到其后
    // 两个广播事件（写失败、tx.send 失败）才逐个解体
    if let Some(h) = forward.take() {
        h.abort();
    }
    drop(tx);
    let _ = writer.await;
}

async fn dispatch(
    state: &Arc<DaemonState>,
    tx: &tokio::sync::mpsc::UnboundedSender<String>,
    forward: &mut Option<tokio::task::JoinHandle<()>>,
    req: Request,
) -> String {
    match req.method.as_str() {
        "ping" => ok_line(
            req.id,
            serde_json::json!({ "version": pardon_core::VERSION }),
        ),
        "status" => ok_line(
            req.id,
            serde_json::to_value(crate::status::snapshot(state)).unwrap_or_default(),
        ),
        "translate" => {
            let text = req
                .params
                .get("text")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or("");
            if text.is_empty() {
                return err_line(req.id, "empty text");
            }
            let tr = state.translator.translate(text).await;
            state
                .counters
                .translations
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            record_gui_history(state, &tr).await;
            ok_line(req.id, serde_json::to_value(&tr).unwrap_or_default())
        }
        "lookup" => {
            let word = req
                .params
                .get("word")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or("");
            if word.is_empty() {
                return err_line(req.id, "empty word");
            }
            let card = state.translator.lookup(word).await;
            ok_line(req.id, serde_json::to_value(&card).unwrap_or_default())
        }
        "trigger" => {
            let source = req
                .params
                .get("source")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if source != "selection" && source != "clipboard" {
                return err_line(req.id, "source must be selection|clipboard");
            }
            match crate::trigger::run(state, source == "selection").await {
                crate::trigger::TriggerOutcome::NoText => ok_line(
                    req.id,
                    serde_json::json!({ "notified": false, "reason": "no text available" }),
                ),
                crate::trigger::TriggerOutcome::Handled {
                    notified,
                    translation,
                } => ok_line(
                    req.id,
                    serde_json::json!({ "notified": notified, "translation": translation }),
                ),
                crate::trigger::TriggerOutcome::Failed(msg) => err_line(req.id, &msg),
            }
        }
        "history" => {
            let limit = req
                .params
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(20) as u32;
            match &state.history {
                Some(h) => {
                    let list = h.lock().await.list(limit).unwrap_or_default();
                    ok_line(req.id, serde_json::to_value(list).unwrap_or_default())
                }
                None => err_line(req.id, "history unavailable"),
            }
        }
        "reload" => match crate::apply_config(state).await {
            Ok(restart_required) => ok_line(
                req.id,
                serde_json::json!({ "restart_required": restart_required }),
            ),
            Err(e) => err_line(req.id, &format!("{e:#}")),
        },
        "subscribe" => {
            // 重复 subscribe：abort 旧转发任务再换新（句柄只有一个，不留泄漏路径）
            if let Some(old) = forward.take() {
                old.abort();
            }
            let mut rx = state.events.subscribe();
            let tx = tx.clone();
            *forward = Some(tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(line) => {
                            if tx.send(line).is_err() {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
            }));
            ok_line(req.id, serde_json::json!({}))
        }
        other => err_line(req.id, &format!("unknown method {other:?}")),
    }
}

/// GUI 主窗口翻译记历史（origin "gui"；空译文跳过）。
async fn record_gui_history(state: &Arc<DaemonState>, tr: &pardon_core::pipeline::Translation) {
    use pardon_core::history::HistoryEntry;
    if tr.translation.is_empty() {
        return;
    }
    let Some(h) = &state.history else { return };
    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let e = HistoryEntry {
        ts_ms,
        text: tr.text.clone(),
        translation: tr.translation.clone(),
        engine: tr.engine.clone(),
        origin: "gui".into(),
    };
    if let Err(err) = h.lock().await.record(&e) {
        log::warn!("history record failed: {err:#}");
    }
}

fn ok_line(id: i64, result: serde_json::Value) -> String {
    serde_json::to_string(&serde_json::json!({ "id": id, "ok": true, "result": result }))
        .unwrap_or_else(|_| r#"{"id":0,"ok":false,"error":"serialize"}"#.into())
}

fn err_line(id: i64, msg: &str) -> String {
    serde_json::to_string(&serde_json::json!({ "id": id, "ok": false, "error": msg }))
        .unwrap_or_else(|_| r#"{"id":0,"ok":false,"error":"serialize"}"#.into())
}

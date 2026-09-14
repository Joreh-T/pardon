//! UDS IPC 客户端：一次性连接-per-request + 常驻订阅连接（自动重连）。
//! 协议契约见 docs（README M3）；路径规则与 daemon 侧 pardon_core::ipc
//! 保持一致（此文件有意不依赖 pardon-core——GUI 是可替换客户端）。
//! 帧协议（与 crates/daemon/src/uds.rs 逐字一致）：
//! 请求 `{"id":i64,"method":str,"params":obj}`，
//! 应答 `{"id":…,"ok":true,"result":…}` / `{"id":…,"ok":false,"error":…}`，
//! 事件行 `{"event":str,"params":obj}`（如 `{"event":"popup",…}`）。

use interprocess::local_socket::tokio::prelude::*;
use serde_json::Value;
use std::path::PathBuf;
use tauri::Emitter;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("PARDON_IPC_SOCK") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(base) if !base.is_empty() => PathBuf::from(base).join("pardon").join("ipc.sock"),
        _ => std::env::temp_dir().join("pardon").join("ipc.sock"),
    }
}

async fn connect() -> anyhow::Result<interprocess::local_socket::tokio::Stream> {
    let name = socket_path()
        .to_string_lossy()
        .into_owned()
        .to_fs_name::<interprocess::local_socket::GenericFilePath>()?;
    Ok(interprocess::local_socket::tokio::Stream::connect(name).await?)
}

/// 单次请求-响应（每请求新建连接：UDS connect 微秒级，免去连接管理）。
pub async fn request(method: &str, params: Value) -> Result<Value, String> {
    let resp = request_raw(method, params)
        .await
        .map_err(|e| format!("cannot reach pardond: {e:#}; is it running?"))?;
    if resp["ok"].as_bool().unwrap_or(false) {
        Ok(resp["result"].clone())
    } else {
        Err(resp["error"]
            .as_str()
            .unwrap_or("unknown ipc error")
            .to_string())
    }
}

async fn request_raw(method: &str, params: Value) -> anyhow::Result<Value> {
    let mut stream = connect().await?;
    let line = serde_json::to_string(&serde_json::json!({
        "id": 1,
        "method": method,
        "params": params,
    }))?;
    stream.write_all(line.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.flush().await?;
    let (r, _w) = tokio::io::split(stream);
    let mut lines = BufReader::new(r).lines();
    let resp = lines
        .next_line()
        .await?
        .ok_or_else(|| anyhow::anyhow!("ipc closed before response"))?;
    Ok(serde_json::from_str(&resp)?)
}

/// 事件行解析：`{"event":…,"params":…}` → (事件名, params)。
pub fn parse_event_line(line: &str) -> Option<(String, Value)> {
    let v: Value = serde_json::from_str(line).ok()?;
    let name = v["event"].as_str()?.to_string();
    Some((name, v["params"].clone()))
}

/// 常驻订阅（断线 3s 重连）：事件转发为同名 tauri 事件；
/// 连接状态变化发 `ipc-up`（true/false）供主窗口状态指示。
pub async fn subscribe_loop(handle: tauri::AppHandle) {
    loop {
        let _ = handle.emit("ipc-up", false);
        match subscribe_once(&handle).await {
            Ok(()) => {}
            Err(e) => log_line(&format!("ipc subscribe ended: {e:#}")),
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
}

async fn subscribe_once(handle: &tauri::AppHandle) -> anyhow::Result<()> {
    let mut stream = connect().await?;
    let line = serde_json::to_string(&serde_json::json!({
        "id": 1, "method": "subscribe", "params": {}
    }))?;
    stream.write_all(line.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.flush().await?;
    let _ = handle.emit("ipc-up", true);
    let (r, _w) = tokio::io::split(stream);
    let mut lines = BufReader::new(r).lines();
    while let Some(line) = lines.next_line().await? {
        if let Some((name, params)) = parse_event_line(&line) {
            let _ = handle.emit(&name, params);
        }
    }
    anyhow::bail!("event stream ended")
}

fn log_line(msg: &str) {
    eprintln!("pardon-gui: {msg}");
}

#[cfg(test)]
mod tests {
    use super::*;

    static SOCK_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// std UnixListener 假 daemon：回显固定响应行（Linux 上 fs-name 本地
    /// 套接字即 UDS，与 interprocess 客户端互通）。
    /// 守卫跨 await 是有意的：PARDON_IPC_SOCK 必须在整个异步期间独占，
    /// 测试进程内不会因此阻塞别的 runtime 线程。
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn request_roundtrips_against_fake_daemon() {
        let _l = SOCK_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("ipc.sock");
        std::env::set_var("PARDON_IPC_SOCK", &sock);
        let listener = tokio::net::UnixListener::bind(&sock).unwrap();
        let srv = tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
            let (s, _) = listener.accept().await.unwrap();
            let (r, mut w) = tokio::io::split(s);
            let mut lines = BufReader::new(r).lines();
            if let Some(_req) = lines.next_line().await.unwrap() {
                w.write_all(br#"{"id":1,"ok":true,"result":{"version":"test"}}"#)
                    .await
                    .unwrap();
                w.write_all(b"\n").await.unwrap();
                w.flush().await.unwrap();
            }
        });
        let v = request("ping", serde_json::json!({})).await.unwrap();
        assert_eq!(v["version"], "test");
        srv.await.unwrap();
        std::env::remove_var("PARDON_IPC_SOCK");
    }

    #[test]
    fn parse_event_line_extracts_name_and_params() {
        let (name, params) =
            parse_event_line(r#"{"event":"popup","params":{"translation":{"text":"hi"}}}"#)
                .unwrap();
        assert_eq!(name, "popup");
        assert_eq!(params["translation"]["text"], "hi");
        assert!(parse_event_line(r#"{"id":1,"ok":true}"#).is_none());
        assert!(parse_event_line("not json").is_none());
    }

    #[test]
    fn socket_path_env_override() {
        let _l = SOCK_LOCK.lock().unwrap();
        std::env::set_var("PARDON_IPC_SOCK", "/tmp/x.sock");
        assert_eq!(socket_path(), std::path::PathBuf::from("/tmp/x.sock"));
        std::env::remove_var("PARDON_IPC_SOCK");
    }
}

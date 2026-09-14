//! `pardon trigger`：通知 daemon 翻译 primary selection / 剪贴板。

/// HTTP 超时：翻译可能较慢（引擎链串行尝试多个引擎），给足余量。
const TRIGGER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

pub async fn run(source: &str, addr: std::net::SocketAddr) -> anyhow::Result<i32> {
    if source != "selection" && source != "clipboard" {
        anyhow::bail!("unknown source {source:?}, expected selection|clipboard");
    }
    let url = format!("http://{addr}/trigger/{source}");
    // 带超时的 client：连接/读超时等一切传输错误统一走「无法连接」提示
    let client = reqwest::Client::builder()
        .timeout(TRIGGER_TIMEOUT)
        .build()?;
    let fetched = async {
        let resp = client.get(&url).send().await?;
        let status = resp.status();
        let body = resp.text().await?;
        Ok::<_, reqwest::Error>((status, body))
    }
    .await;
    let (status, body) = fetched.map_err(|e| {
        anyhow::anyhow!(
            "cannot reach pardond at {addr}: {e}; is it running? try `pardon daemon start`"
        )
    })?;
    println!("{body}");
    Ok(if status.is_success() { 0 } else { 2 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unknown_source_is_rejected() {
        let err = run("bogus", "127.0.0.1:1".parse().unwrap())
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("selection|clipboard"));
    }

    #[tokio::test]
    async fn unreachable_daemon_is_an_error_with_hint() {
        // 127.0.0.1:1 几乎必然 connection refused
        let err = run("selection", "127.0.0.1:1".parse().unwrap())
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("pardon daemon start"));
    }

    /// 一切传输错误（含将来的读超时类）都归一到「无法连接」提示——用
    /// IPv6 回环 refused 作快速代表路径（真 60s 超时等待会让套件变慢，不做）。
    #[tokio::test]
    async fn transport_error_including_read_maps_to_hint() {
        let err = run("clipboard", "[::1]:1".parse().unwrap())
            .await
            .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("cannot reach pardond"), "msg: {msg}");
        assert!(msg.contains("pardon daemon start"), "msg: {msg}");
    }
}

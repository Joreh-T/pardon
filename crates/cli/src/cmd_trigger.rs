//! `pardon trigger`：通知 daemon 翻译 primary selection / 剪贴板。

pub async fn run(source: &str, addr: std::net::SocketAddr) -> anyhow::Result<i32> {
    if source != "selection" && source != "clipboard" {
        anyhow::bail!("unknown source {source:?}, expected selection|clipboard");
    }
    let url = format!("http://{addr}/trigger/{source}");
    let resp = reqwest::get(&url).await.map_err(|e| {
        anyhow::anyhow!(
            "cannot reach pardond at {addr}: {e}; is it running? try `pardon daemon start`"
        )
    })?;
    let status = resp.status();
    let body = resp.text().await?;
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
}

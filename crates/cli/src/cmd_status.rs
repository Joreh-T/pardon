//! `pardon status`：pardond 运行状态与计数器。

pub async fn run(addr: std::net::SocketAddr) -> anyhow::Result<i32> {
    let url = format!("http://{addr}/status");
    match reqwest::get(&url).await {
        Ok(resp) => {
            let body = resp.text().await?;
            // pretty 输出（已是 JSON）；解析失败则原样打印
            match serde_json::from_str::<serde_json::Value>(&body) {
                Ok(v) => println!("{}", serde_json::to_string_pretty(&v)?),
                Err(_) => println!("{body}"),
            }
            Ok(0)
        }
        Err(_) => {
            println!("pardond is not running");
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unreachable_reports_not_running_with_zero() {
        let code = run("127.0.0.1:1".parse().unwrap()).await.unwrap();
        assert_eq!(code, 0);
    }
}

//! `pardon status`：pardond 运行状态与计数器。

/// HTTP 超时：状态查询应即时完成。
const STATUS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

pub async fn run(addr: std::net::SocketAddr) -> anyhow::Result<i32> {
    let url = format!("http://{addr}/status");
    // 带超时的 client：连接/读超时等一切传输错误都视为「未运行」
    let client = reqwest::Client::builder().timeout(STATUS_TIMEOUT).build()?;
    let fetched = async {
        let resp = client.get(&url).send().await?;
        resp.text().await
    }
    .await;
    match fetched {
        Ok(body) => {
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

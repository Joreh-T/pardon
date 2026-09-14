//! 真 pardond 进程 e2e：空词典 + 无 Wayland 环境下验证
//! HTTP 服务、词路由离线 miss、降级 503 与优雅关停。
//! 全程无外网（词路由不触引擎；默认配置无 LLM provider）。

use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::time::{timeout, Duration};

#[tokio::test]
async fn daemon_serves_offline_translate_and_shuts_down() {
    let home = tempfile::tempdir().unwrap();
    let cfg_path = home.path().join("config.toml");
    std::fs::write(&cfg_path, "").unwrap(); // 默认配置

    // 词路由靠 ecdict 命中可达（router::classify 以词典命中区分词/句，
    // 全空词典会把一切文本送进引擎链 → 触外网）。播种仅含 "hello" 的
    // 最小词典（空释义卡片）→ 词路由 → 不触引擎，离线安全；
    // 卡片无释义 → translation 空串，engine = "ecdict"。
    let dict_dir = home.path().join("dict");
    std::fs::create_dir_all(&dict_dir).unwrap();
    let conn = rusqlite::Connection::open(dict_dir.join("ecdict.sqlite")).unwrap();
    let row = format!("hello{}\n", ",".repeat(12)); // 13 列 ECDICT CSV，仅 word 非空
    pardon_core::dict::ecdict::import::import_csv(row.as_bytes(), &conn).unwrap();

    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_pardond"))
        .arg("--log-notify")
        .env("PARDON_HOME", home.path())
        .env("PARDON_CONFIG", &cfg_path)
        .env("PARDON_HTTP_BIND", "127.0.0.1:0") // 随机端口，从 stdout 读实际值
        .env("PATH", "") // wl-paste/wl-copy 不可用 → 验证降级路径
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();

    // 读启动行拿实际端口
    let stdout = child.stdout.take().unwrap();
    let mut lines = BufReader::new(stdout).lines();
    let line = timeout(Duration::from_secs(15), lines.next_line())
        .await
        .expect("startup line timeout")
        .expect("io error")
        .expect("eof");
    assert!(line.contains("listening on http://"), "got: {line}");
    let addr: &str = line.rsplit("http://").next().unwrap().trim();

    // 1) POST /translate 单词 → 空词典 miss：engine=ecdict、translation 空（离线安全）
    let resp: serde_json::Value = reqwest::Client::new()
        .post(format!("http://{addr}/translate"))
        .json(&serde_json::json!({ "text": "hello" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["engine"], "ecdict");
    assert_eq!(resp["translation"], "");

    // 2) trigger/clipboard 在降级模式（wl-paste 不可用）→ 503
    let resp = reqwest::get(format!("http://{addr}/trigger/clipboard"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 503);

    // 3) status 可用
    let resp: serde_json::Value = reqwest::get(format!("http://{addr}/status"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["clipboard_watching"], false, "PATH 清空后监听应降级");

    // 4) POST /shutdown → 优雅退出 0
    reqwest::Client::new()
        .post(format!("http://{addr}/shutdown"))
        .send()
        .await
        .unwrap();
    let status = timeout(Duration::from_secs(10), child.wait())
        .await
        .expect("shutdown timeout")
        .unwrap();
    assert!(status.success());
}

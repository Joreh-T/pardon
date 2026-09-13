//! Golden 契约测试：`pardon translate` 的 stdout JSONL 与退出码（spec §5.3）。
//!
//! 环境同 Task 18：`PARDON_HOME` → core 测试仓库内 pip_home（进 git 的
//! mini fixture 导入产物），`PARDON_CONFIG` → 不存在路径（缺失 → 内置默认，
//! 即无 LLM 配置）。引擎 base 指向测试内 wiremock；逐行 `serde_json` 解析
//! 后按字段断言（delta 切分依引擎而定，不比对整行字符串）。

use assert_cmd::Command;
use serde_json::Value;
use std::time::Duration;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SENTENCE: &str = "please give me the book";
const SENTENCE_ZH: &str = "请给我那本书";
/// pip_home ecdict 中 "run" 的卡片文本（pos 行以 '\n' 连接，gloss 以 '；' 连接）。
const RUN_CARD: &str = "n. 跑步\nv. 跑；运转";

fn pardon() -> Command {
    let mut c = Command::cargo_bin("pardon").unwrap();
    let core_tests = env!("CARGO_MANIFEST_DIR").to_string() + "/../core/tests";
    c.env("PARDON_HOME", core_tests.clone() + "/pip_home");
    c.env("PARDON_CONFIG", core_tests + "/pip_home/no-such-config.toml");
    c
}

/// 引擎 base 全部指向 mock：不慎走到 bing/真实网络时得到 404 而非外网请求。
fn mocked(mock: &MockServer) -> Command {
    let mut c = pardon();
    c.env("PARDON_YOUDAO_BASE", mock.uri());
    c.env("PARDON_BING_BASE", mock.uri());
    c
}

/// 有道 mock：`i=input` 的 GET → 单段译文 output。
async fn mount_youdao(server: &MockServer, input: &str, output: &str) {
    Mock::given(method("GET"))
        .and(path("/translate"))
        .and(query_param("i", input))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"translateResult": [[{"src": input, "tgt": output}]]}),
        ))
        .mount(server)
        .await;
}

/// stdout 按行解析为 JSON 值序列。
fn jsonl(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("行不是合法 JSON（{e}）：{line}")))
        .collect()
}

/// 1. 无 stream：--json 单行 Result JSON，engine=="youdao"，exit 0。
#[tokio::test]
async fn translate_sentence_plain_json_result_youdao() {
    let server = MockServer::start().await;
    mount_youdao(&server, SENTENCE, SENTENCE_ZH).await;

    let out = mocked(&server).args(["translate", "--json", SENTENCE]).unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "--json 应恰好一行，实际 stdout：{stdout}");
    let v: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(v["type"], "result");
    assert_eq!(v["source"], "en");
    assert_eq!(v["target"], "zh");
    assert_eq!(v["engine"], "youdao");
    assert_eq!(v["text"], SENTENCE);
    assert_eq!(v["translation"], SENTENCE_ZH);
}

/// 2. --stream 词路由：meta → 单 delta（卡片全文）→ result（engine=="ecdict"）。
#[tokio::test]
async fn stream_word_run_meta_delta_result_ecdict() {
    // 词路由只查词典，不触引擎；mock server 兜底防真实网络
    let server = MockServer::start().await;
    let out = mocked(&server).args(["translate", "--stream", "run"]).unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let events = jsonl(&String::from_utf8_lossy(&out.stdout));
    assert_eq!(events.len(), 3, "应恰三行（meta/delta/result），实际：{events:?}");

    let meta = &events[0];
    assert_eq!(meta["type"], "meta");
    assert_eq!(meta["source"], "en");
    assert_eq!(meta["target"], "zh");
    assert_eq!(meta["engine"], "auto", "meta.engine 应为 --engine 旗标值");

    let delta = &events[1];
    assert_eq!(delta["type"], "delta");
    assert_eq!(delta["text"], RUN_CARD);

    let result = &events[2];
    assert_eq!(result["type"], "result");
    assert_eq!(result["source"], "en");
    assert_eq!(result["target"], "zh");
    assert_eq!(result["engine"], "ecdict");
    assert_eq!(result["text"], "run");
    assert_eq!(result["translation"], RUN_CARD);
}

/// 3. --stream 句子（无 LLM 配置）：meta → 单 delta（链式全文）→ result
///    engine=="youdao"（Task 17：无 LLM 时链式结果作单次 delta）。
#[tokio::test]
async fn stream_sentence_without_llm_single_delta_youdao() {
    let server = MockServer::start().await;
    mount_youdao(&server, SENTENCE, SENTENCE_ZH).await;

    let out = mocked(&server).args(["translate", "--stream", SENTENCE]).unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let events = jsonl(&String::from_utf8_lossy(&out.stdout));
    assert_eq!(events.len(), 3, "无 LLM 时应恰三行，实际：{events:?}");

    assert_eq!(events[0]["type"], "meta");
    assert_eq!(events[0]["engine"], "auto");
    assert_eq!(events[1]["type"], "delta");
    assert_eq!(events[1]["text"], SENTENCE_ZH);

    assert_eq!(events[2]["type"], "result");
    assert_eq!(events[2]["engine"], "youdao");
    assert_eq!(events[2]["text"], SENTENCE);
    assert_eq!(events[2]["translation"], SENTENCE_ZH);
}

/// 4. --timeout 1 且 mock 延迟 3s → exit 124，stderr 单行 Error JSONL
///    code=="timeout"。
#[tokio::test]
async fn timeout_exits_124_with_timeout_error_json() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/translate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(
                    serde_json::json!({"translateResult": [[{"src": SENTENCE, "tgt": SENTENCE_ZH}]]}),
                )
                .set_delay(Duration::from_secs(3)),
        )
        .mount(&server)
        .await;

    let assert = mocked(&server)
        .args(["translate", "--timeout", "1", SENTENCE])
        .assert()
        .code(124);
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    let v: Value = serde_json::from_str(stderr.trim()).unwrap();
    assert_eq!(v["type"], "error");
    assert_eq!(v["code"], "timeout");
    assert!(
        v["message"].as_str().unwrap_or_default().contains("1s"),
        "message 应含超时秒数：{stderr}"
    );
}

/// 5. --engine bing（显式单引擎，不降级）：两步 mock 成功 → result
///    engine=="bing"，exit 0（复用 Task 14 的 token 抓取 mock 模式）。
#[tokio::test]
async fn explicit_engine_bing_two_step_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/translator"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            b"<html><head><script>var _G={IG:\"AB12CD\",ST:(!e&&123)};</script></head>\
              <body><div id=\"tta_outGDCont\" data-iid=\"translator.5023\"></div></body></html>"
                .to_vec(),
            "text/html",
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/ttranslatev3"))
        .and(query_param("isVertical", "1"))
        .and(query_param("IG", "AB12CD"))
        .and(query_param("IID", "translator.5023"))
        .and(wiremock::matchers::body_partial_json(serde_json::json!({
            "fromLang": "auto-detect",
            "to": "zh-Hans",
            "text": SENTENCE
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!([{"translations": [{"text": SENTENCE_ZH, "to": "zh-Hans"}]}]),
        ))
        .mount(&server)
        .await;

    let out = mocked(&server)
        .args(["translate", "--json", "--engine", "bing", SENTENCE])
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(v["type"], "result");
    assert_eq!(v["engine"], "bing");
    assert_eq!(v["source"], "en");
    assert_eq!(v["target"], "zh");
    assert_eq!(v["text"], SENTENCE);
    assert_eq!(v["translation"], SENTENCE_ZH);
}

/// 6. 空文本（无参数、无 --stdin）→ exit 2。
#[test]
fn empty_text_no_args_no_stdin_exits_2() {
    pardon().args(["translate"]).assert().code(2);
}

/// 6b. 全链失败（译文与引擎皆空）→ exit 2，stdout 不留空 Result，错误
/// 只走 stderr（code "engine"）。词路由无释义不算失败（见 6c）。
#[tokio::test]
async fn total_chain_failure_exits_2_stderr_only() {
    // 无任何 mock 挂载：youdao/bing 全部 404 → 全链失败
    let server = MockServer::start().await;
    let assert = mocked(&server)
        .args(["translate", "--json", SENTENCE])
        .assert()
        .code(2);
    let out = assert.get_output();
    assert!(
        out.stdout.is_empty(),
        "失败路径 stdout 应为空，实际：{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let v: Value = serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(v["type"], "error");
    assert_eq!(v["code"], "engine");
    // 全链失败时给出可行动的提示（Fix 7；timeout 路径不带）
    assert!(
        v["message"].as_str().unwrap_or_default().contains("hint: configure an llm provider"),
        "message 应含 LLM 配置提示：{v}"
    );
}

/// 6c. 词路由命中但无释义（如 "gave" 只有 exchange 无 gloss）：译文空、
/// engine=="ecdict" → 不是错误，照常输出 exit 0。
#[test]
fn gloss_less_word_is_not_an_error() {
    let out = pardon().args(["translate", "--json", "gave"]).unwrap();
    assert!(out.status.success());
    let v: Value = serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(v["type"], "result");
    assert_eq!(v["engine"], "ecdict");
    assert_eq!(v["translation"], "");
}

/// 7. --stdin 读全部 stdin（含换行）：zh 文本 → 有道 type 参数为
///    ZH_CN_EN，result.text 原样保留多行内容。
#[tokio::test]
async fn stdin_multiline_zh_requests_zh_cn_en_and_preserves_text() {
    let server = MockServer::start().await;
    let input = "你好世界\n第二行";
    Mock::given(method("GET"))
        .and(path("/translate"))
        .and(query_param("type", "ZH_CN_EN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"translateResult": [[{"src": input, "tgt": "hello world"}]]}),
        ))
        .mount(&server)
        .await;

    let out = mocked(&server)
        .args(["translate", "--stdin", "--json"])
        .write_stdin(input)
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "--json 应恰好一行，实际 stdout：{stdout}");
    let v: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(v["type"], "result");
    assert_eq!(v["source"], "zh");
    assert_eq!(v["target"], "en");
    assert_eq!(v["engine"], "youdao");
    assert_eq!(v["text"], input, "stdin 多行内容应原样保留");
    assert_eq!(v["translation"], "hello world");
}

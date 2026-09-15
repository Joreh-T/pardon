//! 集成测试：Pipeline 组装词典 + 引擎链 + 流式。
//!
//! `PARDON_HOME` / `PARDON_YOUDAO_BASE` / `PARDON_GOOGLE_BASE` 是进程级
//! 环境变量且测试默认并行：所有触碰它们的测试经 `ENV_LOCK` 串行
//! （`TestEnv` 持锁期间 set，Drop 时还原）。

use pardon_core::config::{load_from_str, AppConfig, LlmConfig, ProviderConfig, ProviderType};
use pardon_core::dict::cedict::CedictDb;
use pardon_core::dict::ecdict::import::import_csv;
use pardon_core::engine::TranslateRequest;
use pardon_core::lang::Lang;
use pardon_core::pipeline::{card_text, LlmEngine, Pipeline};
use rusqlite::Connection;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use wiremock::matchers::{method, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SENTENCE: &str = "please give me the book";
const SENTENCE_ZH: &str = "请给我那本书";

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// 测试期内设置 `PARDON_HOME` / `PARDON_YOUDAO_BASE` / `PARDON_GOOGLE_BASE`；
/// Drop 还原并释放锁。
struct TestEnv {
    old_home: Option<String>,
    old_youdao: Option<String>,
    old_google: Option<String>,
    _lock: MutexGuard<'static, ()>,
}

fn set_env(home: &Path, youdao_base: Option<&str>, google_base: Option<&str>) -> TestEnv {
    let lock = ENV_LOCK.lock().unwrap();
    let old_home = std::env::var("PARDON_HOME").ok();
    let old_youdao = std::env::var("PARDON_YOUDAO_BASE").ok();
    let old_google = std::env::var("PARDON_GOOGLE_BASE").ok();
    std::env::set_var("PARDON_HOME", home);
    match youdao_base {
        Some(base) => std::env::set_var("PARDON_YOUDAO_BASE", base),
        None => std::env::remove_var("PARDON_YOUDAO_BASE"),
    }
    match google_base {
        Some(base) => std::env::set_var("PARDON_GOOGLE_BASE", base),
        None => std::env::remove_var("PARDON_GOOGLE_BASE"),
    }
    TestEnv {
        old_home,
        old_youdao,
        old_google,
        _lock: lock,
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        match self.old_home.take() {
            Some(h) => std::env::set_var("PARDON_HOME", h),
            None => std::env::remove_var("PARDON_HOME"),
        }
        match self.old_youdao.take() {
            Some(b) => std::env::set_var("PARDON_YOUDAO_BASE", b),
            None => std::env::remove_var("PARDON_YOUDAO_BASE"),
        }
        match self.old_google.take() {
            Some(b) => std::env::set_var("PARDON_GOOGLE_BASE", b),
            None => std::env::remove_var("PARDON_GOOGLE_BASE"),
        }
    }
}

/// 临时 PARDON_HOME：fixture CSV / .u8 导入为 dict/ecdict.sqlite + cedict.sqlite。
fn fixture_home() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let dict_dir = dir.path().join("dict");
    std::fs::create_dir_all(&dict_dir).unwrap();
    let conn = Connection::open(dict_dir.join("ecdict.sqlite")).unwrap();
    import_csv(include_str!("fixtures/ecdict_mini.csv").as_bytes(), &conn).unwrap();
    drop(conn);
    CedictDb::import_to_path(
        include_str!("fixtures/cedict_mini.u8").as_bytes(),
        &dict_dir.join("cedict.sqlite"),
    )
    .unwrap();
    dir
}

/// 有道 mock：SENTENCE → SENTENCE_ZH。
async fn mount_youdao(server: &MockServer) {
    Mock::given(method("GET"))
        .and(query_param("i", SENTENCE))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "translateResult": [[{"src": SENTENCE, "tgt": SENTENCE_ZH}]]
        })))
        .mount(server)
        .await;
}

/// google mock：SENTENCE → `[SENTENCE_ZH]`（首元素即译文）。
async fn mount_google(server: &MockServer) {
    Mock::given(method("GET"))
        .and(query_param("q", SENTENCE))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([SENTENCE_ZH])))
        .mount(server)
        .await;
}

/// default_engine 可选、provider id 固定 "mock" 的 LLM 配置。
/// base_url 故意带尾部 '/'：构造时应 trim 掉（Task 11 review note）。
fn cfg_llm(base: &str, default_engine: &str) -> AppConfig {
    load_from_str(&format!(
        r#"
default_engine = "{default_engine}"
[llm]
default_provider = "mock"

[[llm.providers]]
id = "mock"
type = "openai"
base_url = "{base}/"
api_key = "test-key"
model = "m"
"#
    ))
    .unwrap()
}

fn req_en2zh() -> TranslateRequest {
    TranslateRequest {
        text: SENTENCE.into(),
        from: Lang::En,
        to: Lang::Zh,
    }
}

#[test]
fn lookup_en_uses_ecdict_and_fills_suggestions_on_miss() {
    let home = fixture_home();
    let _env = set_env(home.path(), None, None);
    let p = Pipeline::from_config(&AppConfig::default()).unwrap();

    let card = p.lookup("run");
    assert!(card.found);
    assert_eq!(card.source, "ecdict");

    let card = p.lookup("helo");
    assert!(!card.found);
    assert!(
        card.suggestions.contains(&"hello".to_string()),
        "got {:?}",
        card.suggestions
    );
}

#[test]
fn lookup_zh_uses_cedict() {
    let home = fixture_home();
    let _env = set_env(home.path(), None, None);
    let p = Pipeline::from_config(&AppConfig::default()).unwrap();

    let card = p.lookup("你好");
    assert!(card.found);
    assert_eq!(card.source, "cedict");
}

#[tokio::test]
async fn translate_word_route_returns_card_text_from_ecdict() {
    let home = fixture_home();
    let _env = set_env(home.path(), None, None);
    let p = Pipeline::from_config(&AppConfig::default()).unwrap();

    let t = p.translate("run").await;
    assert_eq!(t.engine, "ecdict");
    assert_eq!(t.translation, "n. 跑步\nv. 跑；运转");
    assert_eq!(t.text, "run");
    assert_eq!((t.source_lang, t.target_lang), (Lang::En, Lang::Zh));
}

#[tokio::test]
async fn translate_sentence_without_llm_uses_google_first() {
    let home = fixture_home();
    let server = MockServer::start().await;
    mount_google(&server).await;
    mount_youdao(&server).await;
    let _env = set_env(
        home.path(),
        Some(server.uri().as_str()),
        Some(server.uri().as_str()),
    );
    let p = Pipeline::from_config(&AppConfig::default()).unwrap();

    let t = p.translate(SENTENCE).await;
    assert_eq!(t.engine, "google", "默认链 google 应排第一");
    assert_eq!(t.translation, SENTENCE_ZH);
}

/// google 失败（死端口）→ 降级到链内下一位 youdao。
#[tokio::test]
async fn google_failure_falls_back_to_youdao() {
    let home = fixture_home();
    let server = MockServer::start().await;
    mount_youdao(&server).await;
    let _env = set_env(
        home.path(),
        Some(server.uri().as_str()),
        Some("http://127.0.0.1:1"),
    );
    let p = Pipeline::from_config(&AppConfig::default()).unwrap();

    let t = p.translate(SENTENCE).await;
    assert_eq!(t.engine, "youdao");
    assert_eq!(t.translation, SENTENCE_ZH);
}

#[tokio::test]
async fn translate_stream_word_emits_single_delta_with_full_card_text() {
    let home = fixture_home();
    let _env = set_env(home.path(), None, None);
    let p = Pipeline::from_config(&AppConfig::default()).unwrap();

    let mut deltas = Vec::new();
    let t = p
        .translate_stream("run", |d| deltas.push(d.to_string()))
        .await
        .unwrap();
    let expected = card_text(&p.lookup("run"));
    assert_eq!(
        deltas,
        vec![expected.clone()],
        "word 路由应恰好一次完整卡片文本 delta"
    );
    assert_eq!(t.engine, "ecdict");
    assert_eq!(t.translation, expected);
}

#[tokio::test]
async fn from_config_with_missing_dicts_degrades_to_empty() {
    let dir = tempfile::tempdir().unwrap();
    let _env = set_env(dir.path(), None, None);
    let p = Pipeline::from_config(&AppConfig::default()).expect("词典缺失时应以空库降级构造成功");

    let card = p.lookup("run");
    assert!(!card.found);
    assert!(card.suggestions.is_empty());

    let card = p.lookup("你好");
    assert!(!card.found);
    assert!(card.suggestions.is_empty());
}

#[tokio::test]
async fn translate_stream_sentence_without_llm_emits_single_delta() {
    let home = fixture_home();
    let server = MockServer::start().await;
    mount_google(&server).await;
    let _env = set_env(
        home.path(),
        Some(server.uri().as_str()),
        Some(server.uri().as_str()),
    );
    let p = Pipeline::from_config(&AppConfig::default()).unwrap();

    let mut deltas = Vec::new();
    let t = p
        .translate_stream(SENTENCE, |d| deltas.push(d.to_string()))
        .await
        .unwrap();
    assert_eq!(deltas, vec![SENTENCE_ZH]);
    assert_eq!(t.engine, "google");
    assert_eq!(t.translation, SENTENCE_ZH);
}

#[tokio::test]
async fn translate_stream_sentence_via_llm_forwards_deltas_and_skips_empty() {
    let home = fixture_home();
    let server = MockServer::start().await;
    // SSE 增量：请 / ""（空串，应跳过不转发）/ 书
    let sse = "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"请\"}}]}\n\n\
               data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"\"}}]}\n\n\
               data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"书\"}}]}\n\n\
               data: [DONE]\n\n";
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
        .mount(&server)
        .await;
    let _env = set_env(
        home.path(),
        Some(server.uri().as_str()),
        Some(server.uri().as_str()),
    );
    // default_engine 仍为 youdao：链 = [google, youdao, bing]，但流式优先走 LLM
    let p = Pipeline::from_config(&cfg_llm(&server.uri(), "youdao")).unwrap();

    let mut deltas = Vec::new();
    let t = p
        .translate_stream(SENTENCE, |d| deltas.push(d.to_string()))
        .await
        .unwrap();
    assert_eq!(deltas, vec!["请", "书"], "空串 delta 不应转发");
    assert_eq!(t.translation, "请书");
    assert_eq!(t.engine, "mock");
}

#[tokio::test]
async fn translate_stream_llm_failure_falls_back_to_chain() {
    let home = fixture_home();
    let server = MockServer::start().await;
    // LLM 端点 500 → 流式失败 → 回退引擎链（google mock 兜底，链首）
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    mount_google(&server).await;
    let _env = set_env(
        home.path(),
        Some(server.uri().as_str()),
        Some(server.uri().as_str()),
    );
    let p = Pipeline::from_config(&cfg_llm(&server.uri(), "youdao")).unwrap();

    let mut deltas = Vec::new();
    let t = p
        .translate_stream(SENTENCE, |d| deltas.push(d.to_string()))
        .await
        .unwrap();
    assert_eq!(deltas, vec![SENTENCE_ZH], "回退后完整译文应作单次 delta");
    assert_eq!(t.engine, "google");
    assert_eq!(t.translation, SENTENCE_ZH);
}

#[tokio::test]
async fn translate_with_default_engine_llm_puts_llm_first_in_chain() {
    let home = fixture_home();
    let server = MockServer::start().await;
    // 只 mock LLM 端点：若链序错误（youdao 在前）则 GET 无 mock → 404 → 失败
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "created": 1,
            "model": "m",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": SENTENCE_ZH},
                         "finish_reason": "stop"}]
        })))
        .mount(&server)
        .await;
    let _env = set_env(
        home.path(),
        Some(server.uri().as_str()),
        Some(server.uri().as_str()),
    );
    let p = Pipeline::from_config(&cfg_llm(&server.uri(), "llm")).unwrap();

    let t = p.translate(SENTENCE).await;
    assert_eq!(t.engine, "mock");
    assert_eq!(t.translation, SENTENCE_ZH);
}

#[tokio::test]
async fn chain_with_builds_single_engine_chain() {
    let home = fixture_home();
    let server = MockServer::start().await;
    mount_google(&server).await;
    mount_youdao(&server).await;
    let _env = set_env(
        home.path(),
        Some(server.uri().as_str()),
        Some(server.uri().as_str()),
    );
    let p = Pipeline::from_config(&AppConfig::default()).unwrap();

    let chain = p.chain_with("google").unwrap();
    let (out, name) = chain.translate(&req_en2zh()).await.unwrap();
    assert_eq!((out.as_str(), name), (SENTENCE_ZH, "google"));

    let chain = p.chain_with("youdao").unwrap();
    let (out, name) = chain.translate(&req_en2zh()).await.unwrap();
    assert_eq!((out.as_str(), name), (SENTENCE_ZH, "youdao"));

    // 未配置 provider → llm 单链报错；未知引擎名报错
    assert!(p.chain_with("llm").is_err());
    assert!(p.chain_with("nope").is_err());
}

/// ollama provider 的 base_url / 提示词覆盖应透传到引擎，而非被
/// `for_ollama` 的 11434 缺省覆盖（Fix 4）。
#[test]
fn ollama_provider_honors_base_url_and_prompt_overrides() {
    let dir = tempfile::tempdir().unwrap();
    let _env = set_env(dir.path(), None, None);
    let cfg = load_from_str(
        r#"
default_engine = "youdao"
[llm]
default_provider = "local"

[[llm.providers]]
id = "local"
type = "ollama"
base_url = "http://192.168.1.5:11434/v1/"
model = "qwen2.5:7b"
system_prompt = "custom system"
user_prompt_template = "TRANSLATE {text}"
"#,
    )
    .unwrap();
    let p = Pipeline::from_config(&cfg).unwrap();
    p.chain_with("llm")
        .expect("ollama provider should back the \"llm\" engine");
    match p
        .llm
        .as_deref()
        .expect("ollama provider should build an llm engine")
    {
        LlmEngine::OpenAi(e) => {
            let c = e.config();
            // 尾部 '/' 被构造时 trim；其余字段按 provider 行透传
            assert_eq!(c.base_url, "http://192.168.1.5:11434/v1");
            assert_eq!(c.id, "local");
            assert_eq!(c.model, "qwen2.5:7b");
            assert_eq!(c.system_prompt.as_deref(), Some("custom system"));
            assert_eq!(c.user_prompt_template.as_deref(), Some("TRANSLATE {text}"));
        }
        other => panic!(
            "ollama should map to the OpenAI-compatible engine, got {}",
            other.id()
        ),
    }
}

/// base_url 留空（结构直构绕过校验；load_from_str 会拒绝空 base_url）→
/// 回退本机 11434 缺省，引擎仍可构造。
#[test]
fn ollama_provider_with_empty_base_url_falls_back_to_11434() {
    let dir = tempfile::tempdir().unwrap();
    let _env = set_env(dir.path(), None, None);
    let cfg = AppConfig {
        llm: LlmConfig {
            default_provider: "local".into(),
            providers: vec![ProviderConfig {
                id: "local".into(),
                provider_type: ProviderType::Ollama,
                base_url: String::new(),
                model: "qwen2.5:7b".into(),
                api_key: None,
                api_key_env: None,
                system_prompt: None,
                user_prompt_template: None,
            }],
        },
        ..AppConfig::default()
    };
    let p = Pipeline::from_config(&cfg).unwrap();
    match p
        .llm
        .as_deref()
        .expect("empty base_url should still build the engine")
    {
        LlmEngine::OpenAi(e) => {
            assert_eq!(e.config().base_url, "http://127.0.0.1:11434/v1");
        }
        other => panic!(
            "ollama should map to the OpenAI-compatible engine, got {}",
            other.id()
        ),
    }
}

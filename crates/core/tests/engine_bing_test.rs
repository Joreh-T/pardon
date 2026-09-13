use pardon_core::engine::bing::BingEngine;
use pardon_core::engine::{Engine, EngineError, TranslateRequest};
use pardon_core::lang::Lang;
use wiremock::{matchers, Mock, MockServer, ResponseTemplate};

fn req() -> TranslateRequest {
    TranslateRequest { text: "hello".into(), from: Lang::En, to: Lang::Zh }
}

/// 含 `IG:"AB12CD"` 与 `data-iid="translator.5023"` 的假 translator 页面。
const TOKEN_HTML: &str = "<html><head><script>var _G={IG:\"AB12CD\",ST:(!e&&123)};</script>\
                          </head><body><div id=\"tta_outGDCont\" data-iid=\"translator.5023\">\
                          </div></body></html>";

#[tokio::test]
async fn translates_with_scraped_tokens() {
    let server = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/translator"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(TOKEN_HTML.as_bytes().to_vec(), "text/html"),
        )
        .mount(&server)
        .await;
    // IG/IID query 参数与 JSON body 不完全匹配时 mock 不命中（请求落空 → 报错），
    // 因此本用例同时验证了抓取 token 的透传。
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/ttranslatev3"))
        .and(matchers::query_param("isVertical", "1"))
        .and(matchers::query_param("IG", "AB12CD"))
        .and(matchers::query_param("IID", "translator.5023"))
        .and(matchers::body_partial_json(serde_json::json!({
            "fromLang": "auto-detect",
            "to": "zh-Hans",
            "text": "hello"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!([{"translations": [{"text": "你好", "to": "zh-Hans"}]}]),
        ))
        .mount(&server)
        .await;
    let engine = BingEngine::new_with_base(server.uri());
    let out = engine.translate(&req()).await.unwrap();
    assert_eq!(out, "你好");
    assert_eq!(engine.name(), "bing");
}

#[tokio::test]
async fn wrong_token_params_do_not_match() {
    let server = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/translator"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(TOKEN_HTML.as_bytes().to_vec(), "text/html"),
        )
        .mount(&server)
        .await;
    // POST mock 只接受 IG=FFFF00；引擎实际发送抓取到的 AB12CD → 不命中 → 404。
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/ttranslatev3"))
        .and(matchers::query_param("IG", "FFFF00"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!([{"translations": [{"text": "不应到达", "to": "zh-Hans"}]}]),
        ))
        .mount(&server)
        .await;
    let engine = BingEngine::new_with_base(server.uri());
    let err = engine.translate(&req()).await.unwrap_err();
    assert!(matches!(err, EngineError::Api(_)), "got: {err:?}");
}

#[tokio::test]
async fn missing_ig_token_is_parse_error() {
    let server = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/translator"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            b"<html><body>under maintenance</body></html>".to_vec(),
            "text/html",
        ))
        .mount(&server)
        .await;
    let engine = BingEngine::new_with_base(server.uri());
    let err = engine.translate(&req()).await.unwrap_err();
    assert!(
        matches!(err, EngineError::Parse(ref m) if m.contains("IG")),
        "got: {err:?}"
    );
}

/// 真网冒烟（可选）：Bing 改版导致 token 抓取失效时不阻塞，见引擎 doc 注释。
#[tokio::test]
#[ignore = "hits real cn.bing.com; may fail when Bing changes its page layout"]
async fn real_network_smoke() {
    let engine = BingEngine::new();
    let out = engine.translate(&req()).await.unwrap();
    assert!(!out.is_empty(), "translated text should be non-empty");
}

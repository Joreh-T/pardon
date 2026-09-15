//! Google 引擎 wiremock 契约测试：`GET /translate_a/t?client=dict-chrome-ex
//! &sl=…&tl=…&q=…`，响应 JSON 数组首元素为译文（string 直取；嵌套数组
//! 各元素 join("\n")）。

use pardon_core::engine::google::GoogleEngine;
use pardon_core::engine::{Engine, TranslateRequest};
use pardon_core::lang::Lang;
use wiremock::{
    matchers::{method, path, query_param},
    Mock, MockServer, ResponseTemplate,
};

const SENTENCE_EN: &str = "The quick brown fox jumps over the lazy dog.";
const SENTENCE_ZH: &str = "敏捷的棕色狐狸跳过了那只懒狗。";

/// google mock：`q=input` 的 GET → body 首元素为译文。
async fn mount_google(server: &MockServer, sl: &str, tl: &str, q: &str, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/translate_a/t"))
        .and(query_param("client", "dict-chrome-ex"))
        .and(query_param("sl", sl))
        .and(query_param("tl", tl))
        .and(query_param("q", q))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

fn req(text: &str, from: Lang, to: Lang) -> TranslateRequest {
    TranslateRequest {
        text: text.into(),
        from,
        to,
    }
}

#[tokio::test]
async fn translates_en_to_zh() {
    let server = MockServer::start().await;
    mount_google(
        &server,
        "en",
        "zh",
        SENTENCE_EN,
        serde_json::json!([SENTENCE_ZH]),
    )
    .await;
    let e = GoogleEngine::new_with_base(server.uri());
    let out = e
        .translate(&req(SENTENCE_EN, Lang::En, Lang::Zh))
        .await
        .unwrap();
    assert_eq!(out, SENTENCE_ZH);
}

#[tokio::test]
async fn translates_zh_to_en() {
    let server = MockServer::start().await;
    mount_google(
        &server,
        "zh",
        "en",
        SENTENCE_ZH,
        serde_json::json!([SENTENCE_EN]),
    )
    .await;
    let e = GoogleEngine::new_with_base(server.uri());
    let out = e
        .translate(&req(SENTENCE_ZH, Lang::Zh, Lang::En))
        .await
        .unwrap();
    assert_eq!(out, SENTENCE_EN);
}

#[tokio::test]
async fn nested_first_element_joins_parts_with_newline() {
    let server = MockServer::start().await;
    mount_google(
        &server,
        "en",
        "zh",
        "line one\nline two",
        serde_json::json!([["第一行", "第二行"]]),
    )
    .await;
    let e = GoogleEngine::new_with_base(server.uri());
    let out = e
        .translate(&req("line one\nline two", Lang::En, Lang::Zh))
        .await
        .unwrap();
    assert_eq!(out, "第一行\n第二行");
}

#[tokio::test]
async fn non_200_maps_to_engine_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let e = GoogleEngine::new_with_base(server.uri());
    let err = e
        .translate(&req("hi", Lang::En, Lang::Zh))
        .await
        .unwrap_err();
    assert!(
        matches!(err, pardon_core::engine::EngineError::Api(_)),
        "非 200 应映射 Api 错误，got {err:?}"
    );
}

#[tokio::test]
async fn invalid_json_maps_to_engine_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw("<html>oops".as_bytes().to_vec(), "text/html"),
        )
        .mount(&server)
        .await;
    let e = GoogleEngine::new_with_base(server.uri());
    let err = e
        .translate(&req("hi", Lang::En, Lang::Zh))
        .await
        .unwrap_err();
    assert!(
        matches!(err, pardon_core::engine::EngineError::Parse(_)),
        "非法 JSON 应映射 Parse 错误，got {err:?}"
    );
}

#[tokio::test]
async fn unexpected_first_element_type_is_parse_error() {
    let server = MockServer::start().await;
    mount_google(&server, "en", "zh", "hi", serde_json::json!([42])).await;
    let e = GoogleEngine::new_with_base(server.uri());
    let err = e
        .translate(&req("hi", Lang::En, Lang::Zh))
        .await
        .unwrap_err();
    assert!(
        matches!(err, pardon_core::engine::EngineError::Parse(_)),
        "首元素非 string/数组应映射 Parse 错误，got {err:?}"
    );
}

#[tokio::test]
async fn network_error_maps_to_engine_error() {
    let e = GoogleEngine::new_with_base("http://127.0.0.1:1".into());
    assert!(e.translate(&req("x", Lang::En, Lang::Zh)).await.is_err());
}

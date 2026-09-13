use pardon_core::engine::youdao::YoudaoEngine;
use pardon_core::engine::{Engine, TranslateRequest};
use pardon_core::lang::Lang;
use wiremock::{matchers::{method, query_param}, Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn translates_and_joins_segments() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(query_param("i", "hello world"))
        .and(query_param("type", "EN2ZH_CN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "translateResult": [[{"src":"hello","tgt":"你好"},{"src":"world","tgt":"世界"}]]
        })))
        .mount(&server)
        .await;
    let e = YoudaoEngine::new_with_base(server.uri());
    let out = e.translate(&TranslateRequest {
        text: "hello world".into(), from: Lang::En, to: Lang::Zh }).await.unwrap();
    assert_eq!(out, "你好世界");
}

#[tokio::test]
async fn zh_to_en_uses_zh_cn_en_type() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(query_param("type", "ZH_CN_EN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "translateResult": [[{"src":"你好","tgt":"hello"}]]
        }))).mount(&server).await;
    let e = YoudaoEngine::new_with_base(server.uri());
    let out = e.translate(&TranslateRequest {
        text: "你好".into(), from: Lang::Zh, to: Lang::En }).await.unwrap();
    assert_eq!(out, "hello");
}

#[tokio::test]
async fn network_error_maps_to_engine_error() {
    let e = YoudaoEngine::new_with_base("http://127.0.0.1:1".into());
    assert!(e.translate(&TranslateRequest {
        text: "x".into(), from: Lang::En, to: Lang::Zh }).await.is_err());
}

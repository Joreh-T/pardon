use pardon_core::engine::anthropic::{AnthropicConfig, AnthropicEngine};
use pardon_core::engine::{Engine, EngineError, TranslateRequest};
use pardon_core::lang::Lang;
use wiremock::{matchers, Mock, MockServer, ResponseTemplate};

fn req() -> TranslateRequest {
    TranslateRequest { text: "hello".into(), from: Lang::En, to: Lang::Zh }
}

fn engine(base_url: String) -> AnthropicEngine {
    AnthropicEngine::new(AnthropicConfig {
        id: "mock".into(),
        base_url,
        api_key: "k".into(),
        model: "claude-sonnet-4-5".into(),
        system_prompt: None,
        user_prompt_template: None,
    })
}

#[tokio::test]
async fn non_stream_translation() {
    let server = MockServer::start().await;
    // 同时校验请求头：缺少 x-api-key / anthropic-version 时 mock 不匹配
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/messages"))
        .and(matchers::header("x-api-key", "k"))
        .and(matchers::header("anthropic-version", "2023-06-01"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "msg_1",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": "你好"}],
            "model": "claude-sonnet-4-5",
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 1, "output_tokens": 2}
        })))
        .mount(&server)
        .await;
    let engine = engine(server.uri());
    let out = engine.translate(&req()).await.unwrap();
    assert_eq!(out, "你好");
    assert_eq!(engine.name(), "mock");
}

#[tokio::test]
async fn stream_deltas_concatenated() {
    let server = MockServer::start().await;
    let sse = "event: message_start\ndata: {\"type\":\"message_start\"}\n\n\
               event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"你\"}}\n\n\
               event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"好\"}}\n\n\
               event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
    Mock::given(matchers::method("POST"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_raw(sse.as_bytes().to_vec(), "text/event-stream"))
        .mount(&server)
        .await;
    let engine = engine(server.uri());
    let mut deltas = Vec::new();
    let full = engine
        .translate_stream(&req(), |d| deltas.push(d.to_string()))
        .await
        .unwrap();
    assert_eq!(full, "你好");
    assert_eq!(deltas, vec!["你", "好"]);
}

#[tokio::test]
async fn api_error_maps_to_engine_error_with_status() {
    let server = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_json(
            serde_json::json!({"type": "error", "error": {"type": "authentication_error", "message": "invalid x-api-key"}})))
        .mount(&server)
        .await;
    let engine = engine(server.uri());
    let err = engine.translate(&req()).await.unwrap_err();
    assert!(matches!(err, EngineError::Api(ref m) if m.contains("401")), "got: {err:?}");
}

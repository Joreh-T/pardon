use pardon_core::engine::openai::{OpenAiConfig, OpenAiEngine};
use pardon_core::engine::{Engine, TranslateRequest};
use pardon_core::lang::Lang;
use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

fn req() -> TranslateRequest {
    TranslateRequest {
        text: "hello".into(),
        from: Lang::En,
        to: Lang::Zh,
    }
}

#[tokio::test]
async fn non_stream_translation() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "created": 1,
            "model": "m",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "你好"},
                         "finish_reason": "stop"}]
        })))
        .mount(&server)
        .await;
    let engine = OpenAiEngine::new(OpenAiConfig {
        id: "mock".into(),
        base_url: server.uri(),
        api_key: "k".into(),
        model: "m".into(),
        system_prompt: None,
        user_prompt_template: None,
    });
    let out = engine.translate(&req()).await.unwrap();
    assert_eq!(out, "你好");
}

#[tokio::test]
async fn stream_deltas_concatenated() {
    let server = MockServer::start().await;
    // async-openai 0.28 的 CreateChatCompletionStreamResponse 要求
    // id/object/created/model/index 字段存在，故比裸 delta 多补齐这些字段
    let sse = "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"你\"}}]}\n\n\
               data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"好\"}}]}\n\n\
               data: [DONE]\n\n";
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
        .mount(&server)
        .await;
    let engine = OpenAiEngine::new(OpenAiConfig {
        id: "mock".into(),
        base_url: server.uri(),
        api_key: "k".into(),
        model: "m".into(),
        system_prompt: None,
        user_prompt_template: None,
    });
    let mut deltas = Vec::new();
    let full = engine
        .translate_stream(&req(), |d| deltas.push(d.to_string()))
        .await
        .unwrap();
    assert_eq!(full, "你好");
    assert_eq!(deltas, vec!["你", "好"]);
}

#[tokio::test]
async fn api_error_maps_to_engine_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(serde_json::json!({"error": {"message": "bad key"}})),
        )
        .mount(&server)
        .await;
    let engine = OpenAiEngine::new(OpenAiConfig {
        id: "mock".into(),
        base_url: server.uri(),
        api_key: "k".into(),
        model: "m".into(),
        system_prompt: None,
        user_prompt_template: None,
    });
    let err = engine.translate(&req()).await.unwrap_err();
    assert!(
        matches!(err, pardon_core::engine::EngineError::Api(_)),
        "got: {err:?}"
    );
}

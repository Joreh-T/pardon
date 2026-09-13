//! Anthropic Messages API 引擎：手写 `/v1/messages` 调用，不引第三方 SDK。
//! 非流式取 `content[0].text`；流式解析 SSE，仅关心 `content_block_delta`
//! 携带的 `delta.text` 增量。

use super::prompt::{render_user_prompt, DEFAULT_SYSTEM_PROMPT, DEFAULT_USER_TEMPLATE};
use super::{Engine, EngineError, TranslateRequest};
use eventsource_stream::Eventsource;
use futures::StreamExt;

/// Anthropic（或兼容 Messages API 端点）配置。
pub struct AnthropicConfig {
    /// provider id，如 "anthropic"
    pub id: String,
    /// 如 `https://api.anthropic.com`
    pub base_url: String,
    pub api_key: String,
    /// 如 `claude-sonnet-4-5`
    pub model: String,
    pub system_prompt: Option<String>,
    pub user_prompt_template: Option<String>,
}

pub struct AnthropicEngine {
    /// `Engine::name()` 需返回 `&'static str`，而 id 是运行时 String，
    /// 故在 `new` 中 `Box::leak` 一次性泄漏（同 `OpenAiEngine` 的做法：
    /// 每个引擎仅构造一次、数量有界，泄漏总量可控）。
    name: &'static str,
    cfg: AnthropicConfig,
    http: reqwest::Client,
}

impl AnthropicEngine {
    pub fn new(cfg: AnthropicConfig) -> Self {
        let name = Box::leak(cfg.id.clone().into_boxed_str());
        Self { name, cfg, http: reqwest::Client::new() }
    }

    /// 组装 (system, user) 提示词：Task 10 的常量为默认值，配置可覆盖。
    fn prompts(&self, req: &TranslateRequest) -> (String, String) {
        let system = self.cfg.system_prompt.as_deref().unwrap_or(DEFAULT_SYSTEM_PROMPT);
        let user = render_user_prompt(
            self.cfg.user_prompt_template.as_deref().unwrap_or(DEFAULT_USER_TEMPLATE),
            &req.text,
            req.from,
            req.to,
        );
        (system.to_owned(), user)
    }

    /// 发起 `/v1/messages` 请求；`stream` 为 `true` 时返回原始响应供 SSE 解析。
    /// 发送失败 → `Network`；HTTP 非 2xx → `Api`（reqwest 的错误消息含状态码）。
    async fn post_messages(
        &self,
        body: serde_json::Value,
    ) -> Result<reqwest::Response, EngineError> {
        self.http
            .post(format!("{}/v1/messages", self.cfg.base_url))
            .header("x-api-key", &self.cfg.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .map_err(|e| EngineError::Network(e.to_string()))?
            .error_for_status()
            .map_err(|e| EngineError::Api(e.to_string()))
    }

    fn body(&self, req: &TranslateRequest, stream: bool) -> serde_json::Value {
        let (system, user) = self.prompts(req);
        let mut body = serde_json::json!({
            "model": self.cfg.model,
            "max_tokens": 4096,
            "system": system,
            "messages": [{"role": "user", "content": user}],
        });
        if stream {
            body["stream"] = serde_json::Value::Bool(true);
        }
        body
    }

    async fn translate_once(&self, req: &TranslateRequest) -> Result<String, EngineError> {
        let resp = self.post_messages(self.body(req, false)).await?;
        let resp: serde_json::Value =
            resp.json().await.map_err(|e| EngineError::Parse(e.to_string()))?;
        resp["content"][0]["text"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| EngineError::Parse("no text in content[0]".into()))
            .map(|s| s.to_owned())
    }

    /// 流式翻译：每收到一个 `content_block_delta` 的文本增量调用一次
    /// `on_delta`，返回完整译文。其余事件（message_start、content_block_start、
    /// message_delta、message_stop、ping 等）及非 JSON data 一律忽略。
    pub async fn translate_stream(
        &self,
        req: &TranslateRequest,
        mut on_delta: impl FnMut(&str) + Send,
    ) -> Result<String, EngineError> {
        let resp = self.post_messages(self.body(req, true)).await?;
        let mut out = String::new();
        let mut stream = resp.bytes_stream().eventsource();
        while let Some(ev) = stream.next().await {
            let ev = ev.map_err(|e| EngineError::Network(e.to_string()))?;
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&ev.data) else { continue };
            if v["type"] == "content_block_delta" {
                if let Some(t) = v["delta"]["text"].as_str() {
                    on_delta(t);
                    out.push_str(t);
                }
            }
        }
        if out.is_empty() {
            return Err(EngineError::Parse("empty content".into()));
        }
        Ok(out)
    }
}

#[async_trait::async_trait]
impl Engine for AnthropicEngine {
    fn name(&self) -> &'static str {
        self.name
    }

    async fn translate(&self, req: &TranslateRequest) -> Result<String, EngineError> {
        self.translate_once(req).await
    }
}

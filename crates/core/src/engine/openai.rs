//! OpenAI 兼容引擎：适用于任何暴露 `/chat/completions` 的端点
//! （OpenAI、DeepSeek 等），Ollama 的 OpenAI 兼容层亦复用它。

use super::prompt::{render_user_prompt, DEFAULT_SYSTEM_PROMPT, DEFAULT_USER_TEMPLATE};
use super::{Engine, EngineError, TranslateRequest};
use async_openai::config::OpenAIConfig as AsyncOpenAiConfig;
use async_openai::error::OpenAIError;
use async_openai::types::{
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
    ChatCompletionRequestSystemMessageContent, ChatCompletionRequestUserMessageArgs,
    ChatCompletionRequestUserMessageContent, CreateChatCompletionRequest,
    CreateChatCompletionRequestArgs,
};
use async_openai::Client;
use futures::StreamExt;

/// 通用「OpenAI 兼容端点」配置；Ollama 也用它（`base_url` 指 `/v1`）。
pub struct OpenAiConfig {
    /// provider id，如 "deepseek"
    pub id: String,
    /// 如 `https://api.deepseek.com/v1`
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub system_prompt: Option<String>,
    pub user_prompt_template: Option<String>,
}

/// Ollama OpenAI 兼容层的缺省端点（provider `base_url` 留空时使用）。
pub const OLLAMA_DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434/v1";

impl OpenAiConfig {
    /// Ollama 的 OpenAI 兼容端点（`base_url` [`OLLAMA_DEFAULT_BASE_URL`]，key "ollama"）。
    pub fn for_ollama(model: &str) -> Self {
        Self {
            id: "ollama".into(),
            base_url: OLLAMA_DEFAULT_BASE_URL.into(),
            api_key: "ollama".into(),
            model: model.into(),
            system_prompt: None,
            user_prompt_template: None,
        }
    }
}

pub struct OpenAiEngine {
    /// `Engine::name()` 需返回 `&'static str`，而 id 是运行时 String，
    /// 故在 `new` 中 `Box::leak` 一次性泄漏。每个引擎仅构造一次、provider
    /// 数量有限且有界，泄漏总量可控。
    name: &'static str,
    cfg: OpenAiConfig,
    client: Client<AsyncOpenAiConfig>,
}

impl OpenAiEngine {
    pub fn new(cfg: OpenAiConfig) -> Self {
        let sdk_cfg = AsyncOpenAiConfig::new()
            .with_api_base(cfg.base_url.clone())
            .with_api_key(cfg.api_key.clone());
        let name = Box::leak(cfg.id.clone().into_boxed_str());
        Self { name, cfg, client: Client::with_config(sdk_cfg) }
    }

    /// 引擎实际生效的配置（组装正确性的只读断言用，如 pipeline 测试）。
    pub fn config(&self) -> &OpenAiConfig {
        &self.cfg
    }

    fn build_request(&self, req: &TranslateRequest) -> Result<CreateChatCompletionRequest, EngineError> {
        let system = self.cfg.system_prompt.as_deref().unwrap_or(DEFAULT_SYSTEM_PROMPT);
        let user = render_user_prompt(
            self.cfg.user_prompt_template.as_deref().unwrap_or(DEFAULT_USER_TEMPLATE),
            &req.text,
            req.from,
            req.to,
        );
        let messages = vec![
            ChatCompletionRequestMessage::System(
                ChatCompletionRequestSystemMessageArgs::default()
                    .content(ChatCompletionRequestSystemMessageContent::Text(
                        system.to_owned(),
                    ))
                    .build()
                    .map_err(map_openai_error)?,
            ),
            ChatCompletionRequestMessage::User(
                ChatCompletionRequestUserMessageArgs::default()
                    .content(ChatCompletionRequestUserMessageContent::Text(user))
                    .build()
                    .map_err(map_openai_error)?,
            ),
        ];
        CreateChatCompletionRequestArgs::default()
            .model(self.cfg.model.as_str())
            .messages(messages)
            .temperature(0.1)
            .build()
            .map_err(map_openai_error)
    }

    /// 流式翻译：每收到一个增量内容调用一次 `on_delta`，返回完整译文。
    /// （`create_stream` 内部会自动设置 `stream: true`，无需手动设置。）
    pub async fn translate_stream(
        &self,
        req: &TranslateRequest,
        mut on_delta: impl FnMut(&str) + Send,
    ) -> Result<String, EngineError> {
        let request = self.build_request(req)?;
        let mut stream = self
            .client
            .chat()
            .create_stream(request)
            .await
            .map_err(map_openai_error)?;
        let mut out = String::new();
        while let Some(ev) = stream.next().await {
            let ev = ev.map_err(map_openai_error)?;
            if let Some(delta) = ev.choices.first().and_then(|c| c.delta.content.as_deref()) {
                on_delta(delta);
                out.push_str(delta);
            }
        }
        if out.is_empty() {
            return Err(EngineError::Parse("empty content".into()));
        }
        Ok(out)
    }

    async fn translate_once(&self, req: &TranslateRequest) -> Result<String, EngineError> {
        let request = self.build_request(req)?;
        let resp = self.client.chat().create(request).await.map_err(map_openai_error)?;
        let content = resp
            .choices
            .first()
            .and_then(|c| c.message.content.as_deref())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| EngineError::Parse("empty content".into()))?;
        Ok(content.to_owned())
    }
}

/// async-openai 错误 → `EngineError`：
/// 传输层错误 → `Network`；API 错误（HTTP 非 2xx，SDK 不保留状态码，
/// 以其错误消息/响应体呈现）→ `Api`；响应反序列化失败 → `Parse`。
fn map_openai_error(err: OpenAIError) -> EngineError {
    match err {
        OpenAIError::Reqwest(e) => EngineError::Network(e.to_string()),
        OpenAIError::StreamError(msg) => EngineError::Network(msg),
        OpenAIError::ApiError(e) => EngineError::Api(e.to_string()),
        OpenAIError::JSONDeserialize(e) => EngineError::Parse(e.to_string()),
        other => EngineError::Api(other.to_string()),
    }
}

#[async_trait::async_trait]
impl Engine for OpenAiEngine {
    fn name(&self) -> &'static str {
        self.name
    }

    async fn translate(&self, req: &TranslateRequest) -> Result<String, EngineError> {
        self.translate_once(req).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn for_ollama_sets_endpoint_and_key() {
        let cfg = OpenAiConfig::for_ollama("qwen2.5:7b");
        assert_eq!(cfg.id, "ollama");
        assert_eq!(cfg.base_url, "http://127.0.0.1:11434/v1");
        assert_eq!(cfg.api_key, "ollama");
        assert_eq!(cfg.model, "qwen2.5:7b");
        assert!(cfg.system_prompt.is_none());
        assert!(cfg.user_prompt_template.is_none());
    }
}

//! Google 翻译引擎：Chrome 词典扩展（dict-chrome-ex）同款非官方免费端点
//! `GET {base}/translate_a/t?client=dict-chrome-ex&sl=…&tl=…&q=…`，
//! 无需鉴权，带普通浏览器 User-Agent。
//!
//! 响应是 JSON 数组，实测多种形态都以首元素承载译文：首元素为 string 时
//! 即译文（多行文本内嵌 `\n`）；为嵌套数组时各元素按序 join("\n")；其余
//! 形态视为解析失败。
//!
//! 非官方端点，随时可能失效/限流（作默认兜底链第一位；显式
//! `--engine google` 亦可用），质量不如 LLM。

use super::{Engine, EngineError, TranslateRequest};
use crate::lang::Lang;

pub struct GoogleEngine {
    base: String,
    http: reqwest::Client,
}

const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36";

impl GoogleEngine {
    /// 默认 base `https://clients5.google.com`，可经环境变量
    /// `PARDON_GOOGLE_BASE` 覆盖（测试/代理场景用）。
    pub fn new() -> Self {
        let base = std::env::var("PARDON_GOOGLE_BASE")
            .unwrap_or_else(|_| "https://clients5.google.com".into());
        Self::new_with_base(base)
    }

    /// 直接注入 base（单测用 wiremock mock 服务）。
    pub fn new_with_base(base: String) -> Self {
        Self {
            base,
            http: reqwest::Client::new(),
        }
    }

    /// Lang → google 语言码（实测 en/zh；勿用 zh-CN）。
    fn lang_code(l: Lang) -> &'static str {
        match l {
            Lang::En => "en",
            Lang::Zh => "zh",
        }
    }
}

impl Default for GoogleEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Engine for GoogleEngine {
    fn name(&self) -> &'static str {
        "google"
    }

    async fn translate(&self, req: &TranslateRequest) -> Result<String, EngineError> {
        // `.query` 负责对 `q` 做 URL 编码（空格、换行、CJK 等）。
        let resp = self
            .http
            .get(format!("{}/translate_a/t", self.base))
            .query(&[
                ("client", "dict-chrome-ex"),
                ("sl", Self::lang_code(req.from)),
                ("tl", Self::lang_code(req.to)),
                ("q", req.text.as_str()),
            ])
            .header("User-Agent", USER_AGENT)
            .send()
            .await
            .map_err(|e| EngineError::Network(e.to_string()))?
            .error_for_status()
            .map_err(|e| EngineError::Api(e.to_string()))?;
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| EngineError::Parse(e.to_string()))?;
        // 防御性解析：只信首元素，string 直取；数组 join("\n")；其他报错。
        match body.as_array().and_then(|a| a.first()) {
            Some(serde_json::Value::String(s)) => Ok(s.clone()),
            Some(serde_json::Value::Array(parts)) => {
                let lines: Option<Vec<&str>> = parts.iter().map(|p| p.as_str()).collect();
                let joined = lines
                    .ok_or_else(|| EngineError::Parse("non-string part in nested array".into()))?
                    .join("\n");
                if joined.is_empty() {
                    return Err(EngineError::Parse("empty translation".into()));
                }
                Ok(joined)
            }
            other => Err(EngineError::Parse(format!(
                "unexpected first element: {other:?}"
            ))),
        }
    }
}

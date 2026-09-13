//! 有道翻译 web 免费接口引擎：`GET {base}/translate?doctype=json&type=…&i=…`，
//! 无需鉴权。响应形如
//! `{"translateResult":[[{"src":"hello","tgt":"你好"}], …]}`，
//! 所有 `tgt` 按序直接拼接（无分隔符）作为译文。

use super::{Engine, EngineError, TranslateRequest};
use crate::lang::Lang;

pub struct YoudaoEngine {
    base: String,
    http: reqwest::Client,
}

/// 有道 web 接口响应：`translateResult[i][j].tgt` 为译文分段。
#[derive(serde::Deserialize)]
struct YoudaoResponse {
    #[serde(rename = "translateResult")]
    translate_result: Option<Vec<Vec<Segment>>>,
}

#[derive(serde::Deserialize)]
struct Segment {
    tgt: String,
}

impl YoudaoEngine {
    /// 默认 base `https://fanyi.youdao.com`，可经环境变量
    /// `PARDON_YOUDAO_BASE` 覆盖（测试/代理场景用）。
    pub fn new() -> Self {
        let base = std::env::var("PARDON_YOUDAO_BASE")
            .unwrap_or_else(|_| "https://fanyi.youdao.com".into());
        Self::new_with_base(base)
    }

    /// 直接注入 base（单测用 wiremock mock 服务）。
    pub fn new_with_base(base: String) -> Self {
        Self { base, http: reqwest::Client::new() }
    }

    /// 目标语言 → 有道 `type` 参数。
    fn youdao_type(to: Lang) -> &'static str {
        match to {
            Lang::Zh => "EN2ZH_CN",
            Lang::En => "ZH_CN_EN",
        }
    }
}

impl Default for YoudaoEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Engine for YoudaoEngine {
    fn name(&self) -> &'static str {
        "youdao"
    }

    async fn translate(&self, req: &TranslateRequest) -> Result<String, EngineError> {
        // `.query` 负责对 `i` 做 URL 编码（空格、CJK 等）。
        let resp = self
            .http
            .get(format!("{}/translate", self.base))
            .query(&[
                ("doctype", "json"),
                ("type", Self::youdao_type(req.to)),
                ("i", req.text.as_str()),
            ])
            .send()
            .await
            .map_err(|e| EngineError::Network(e.to_string()))?
            .error_for_status()
            .map_err(|e| EngineError::Api(e.to_string()))?;
        let parsed: YoudaoResponse =
            resp.json().await.map_err(|e| EngineError::Parse(e.to_string()))?;
        let mut out = String::new();
        for seg in parsed.translate_result.into_iter().flatten().flatten() {
            out.push_str(&seg.tgt);
        }
        if out.is_empty() {
            return Err(EngineError::Parse("empty translateResult".into()));
        }
        Ok(out)
    }
}

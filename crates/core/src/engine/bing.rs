//! 必应翻译引擎（experimental）：非官方 web 接口，两步请求。
//! ① `GET {base}/translator` 抓取页面内的 `IG` / `IID` token；
//! ② `POST {base}/ttranslatev3` 携带 token 完成翻译。
//!
//! 响应形如 `[{"translations":[{"text":"你好","to":"zh-Hans"}], …}]`，
//! 取 `[0].translations[0].text` 作为译文。

use std::sync::LazyLock;

use super::{Engine, EngineError, TranslateRequest};
use crate::lang::Lang;
use regex::Regex;

/// Experimental：token 抓取依赖页面结构（`IG` / `data-iid`），
/// Bing 改版可能随时失效（spec §9 风险 2），仅作 fallback 链中的实验性引擎。
/// 2026-09 真网复核：`ttranslatev3` 已返回 401 captcha，上游免费端点确认
/// 失效；引擎保留作请求契约与未来官方签名 API 适配的基础，勿作主力引擎。
pub struct BingEngine {
    http: reqwest::Client,
    base: String,
}

/// `_G={IG:"…",…}` 中的 IG（十六进制串）。
static IG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"IG:"([0-9A-Fa-f]+)""#).expect("static IG regex"));
/// `data-iid="…"` 属性中的 IID。
static IID_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-iid="([^"]+)""#).expect("static IID regex"));

/// ttranslatev3 响应数组元素：`translations[0].text` 为译文。
#[derive(serde::Deserialize)]
struct BingResponse {
    translations: Vec<BingTranslation>,
}

#[derive(serde::Deserialize)]
struct BingTranslation {
    text: String,
}

const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64)";

impl BingEngine {
    /// 默认 base `https://cn.bing.com`，可经环境变量
    /// `PARDON_BING_BASE` 覆盖（测试/代理场景用）。
    pub fn new() -> Self {
        let base =
            std::env::var("PARDON_BING_BASE").unwrap_or_else(|_| "https://cn.bing.com".into());
        Self::new_with_base(base)
    }

    /// 直接注入 base（单测用 wiremock mock 服务）。
    pub fn new_with_base(base: String) -> Self {
        Self { http: reqwest::Client::new(), base }
    }

    /// 目标语言 → ttranslatev3 `to` 参数。
    fn to_code(to: Lang) -> &'static str {
        match to {
            Lang::Zh => "zh-Hans",
            Lang::En => "en",
        }
    }
}

impl Default for BingEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Engine for BingEngine {
    fn name(&self) -> &'static str {
        "bing"
    }

    async fn translate(&self, req: &TranslateRequest) -> Result<String, EngineError> {
        // ① 抓 token：不带 UA 可能拿到精简页，带上更接近真实浏览器。
        let html = self
            .http
            .get(format!("{}/translator", self.base))
            .header("User-Agent", USER_AGENT)
            .send()
            .await
            .map_err(|e| EngineError::Network(e.to_string()))?
            .error_for_status()
            .map_err(|e| EngineError::Api(e.to_string()))?
            .text()
            .await
            .map_err(|e| EngineError::Network(e.to_string()))?;
        let token = IG_RE.captures(&html).and_then(|c| c.get(1).map(|m| m.as_str()));
        let iid = IID_RE.captures(&html).and_then(|c| c.get(1).map(|m| m.as_str()));
        let (ig, iid) = match (token, iid) {
            (Some(ig), Some(iid)) => (ig, iid),
            _ => return Err(EngineError::Parse("missing IG/IID token".into())),
        };

        // ② 翻译：`.json` 负责 text 的转义与 Content-Type。
        let resp = self
            .http
            .post(format!("{}/ttranslatev3", self.base))
            .query(&[("isVertical", "1"), ("IG", ig), ("IID", iid)])
            .header("User-Agent", USER_AGENT)
            .header("Referer", format!("{}/translator", self.base))
            .json(&serde_json::json!({
                "fromLang": "auto-detect",
                "to": Self::to_code(req.to),
                "text": req.text,
            }))
            .send()
            .await
            .map_err(|e| EngineError::Network(e.to_string()))?
            .error_for_status()
            .map_err(|e| EngineError::Api(e.to_string()))?;
        let parsed: Vec<BingResponse> =
            resp.json().await.map_err(|e| EngineError::Parse(e.to_string()))?;
        parsed
            .into_iter()
            .next()
            .and_then(|r| r.translations.into_iter().next())
            .map(|t| t.text)
            .ok_or_else(|| EngineError::Parse("empty translations".into()))
    }
}

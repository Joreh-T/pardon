use crate::lang::Lang;

pub struct TranslateRequest {
    pub text: String,
    pub from: Lang,
    pub to: Lang,
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("network error: {0}")]
    Network(String),
    #[error("engine api error: {0}")]
    Api(String),
    #[error("response parse error: {0}")]
    Parse(String),
}

#[async_trait::async_trait]
pub trait Engine: Send + Sync {
    fn name(&self) -> &'static str;
    async fn translate(&self, req: &TranslateRequest) -> Result<String, EngineError>;
}

pub struct Chain {
    pub engines: Vec<std::sync::Arc<dyn Engine>>,
}

impl Chain {
    /// 按序尝试，返回 (译文, 成功引擎名)；全部失败返回最后一个 Err
    pub async fn translate(&self, req: &TranslateRequest)
        -> Result<(String, &'static str), EngineError>
    {
        let mut last: Option<EngineError> = None;
        for e in &self.engines {
            match e.translate(req).await {
                Ok(text) => return Ok((text, e.name())),
                Err(err) => last = Some(err),
            }
        }
        Err(last.unwrap_or(EngineError::Api("no engines configured".into())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::Lang;
    use std::sync::Arc;

    struct OkEngine(&'static str);
    #[async_trait::async_trait]
    impl Engine for OkEngine {
        fn name(&self) -> &'static str { self.0 }
        async fn translate(&self, _r: &TranslateRequest) -> Result<String, EngineError> {
            Ok(format!("translated-by-{}", self.0))
        }
    }
    struct FailEngine(&'static str);
    #[async_trait::async_trait]
    impl Engine for FailEngine {
        fn name(&self) -> &'static str { self.0 }
        async fn translate(&self, _r: &TranslateRequest) -> Result<String, EngineError> {
            Err(EngineError::Api(format!("boom-{}", self.0)))
        }
    }

    fn req() -> TranslateRequest {
        TranslateRequest { text: "hi".into(), from: Lang::En, to: Lang::Zh }
    }

    #[tokio::test]
    async fn chain_returns_first_success() {
        let chain = Chain { engines: vec![Arc::new(OkEngine("a")), Arc::new(OkEngine("b"))] };
        let (out, engine) = chain.translate(&req()).await.unwrap();
        assert_eq!((out.as_str(), engine), ("translated-by-a", "a"));
    }

    #[tokio::test]
    async fn chain_falls_through_failures() {
        let chain = Chain { engines: vec![Arc::new(FailEngine("x")), Arc::new(OkEngine("y"))] };
        let (out, engine) = chain.translate(&req()).await.unwrap();
        assert_eq!((out.as_str(), engine), ("translated-by-y", "y"));
    }

    #[tokio::test]
    async fn chain_all_fail_returns_last_error() {
        let chain = Chain { engines: vec![Arc::new(FailEngine("x")), Arc::new(FailEngine("z"))] };
        let err = chain.translate(&req()).await.unwrap_err();
        assert!(matches!(err, EngineError::Api(m) if m == "boom-z"), "应返回最后一个错误");
    }

    #[tokio::test]
    async fn empty_chain_errors() {
        let chain = Chain { engines: vec![] };
        assert!(chain.translate(&req()).await.is_err());
    }
}

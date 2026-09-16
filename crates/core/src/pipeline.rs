//! Pipeline 顶层组装：词典（ECDICT / CEDICT）+ 引擎链 + 流式翻译。
//!
//! `from_config` 容错加载词典：sqlite 缺失时空内存库降级（stderr 提示导入
//! 命令），CLI 仍可翻译；引擎链按 `default_engine` 组装（llm 可用时
//! [llm, google, youdao, bing]，否则 [google, youdao, bing]）。

use crate::config::{AppConfig, ProviderType};
use crate::dict::cedict::CedictDb;
use crate::dict::ecdict_query::EcdictDb;
use crate::dict::{DictProvider, WordCard};
use crate::engine::anthropic::{AnthropicConfig, AnthropicEngine};
use crate::engine::bing::BingEngine;
use crate::engine::google::GoogleEngine;
use crate::engine::openai::{OpenAiConfig, OpenAiEngine, OLLAMA_DEFAULT_BASE_URL};
use crate::engine::youdao::YoudaoEngine;
use crate::engine::{Chain, Engine, EngineError, TranslateRequest};
use crate::lang::{self, Lang};
use crate::router::{self, Route};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 已构造的默认 LLM 引擎：OpenAI 兼容端点或 Anthropic Messages API。
pub enum LlmEngine {
    OpenAi(OpenAiEngine),
    Anthropic(AnthropicEngine),
}

impl LlmEngine {
    /// provider id（构造时已 `Box::leak` 为 `&'static str`）。
    pub fn id(&self) -> &str {
        match self {
            Self::OpenAi(e) => e.name(),
            Self::Anthropic(e) => e.name(),
        }
    }

    /// 流式翻译：每个文本增量回调一次 `on_delta`，返回完整译文。
    pub async fn translate_stream(
        &self,
        req: &TranslateRequest,
        on_delta: impl FnMut(&str) + Send,
    ) -> Result<String, EngineError> {
        match self {
            Self::OpenAi(e) => e.translate_stream(req, on_delta).await,
            Self::Anthropic(e) => e.translate_stream(req, on_delta).await,
        }
    }
}

/// LLM 引擎同时可作非流式引擎进入链（两个底层引擎均已实现 `Engine`）。
#[async_trait::async_trait]
impl Engine for LlmEngine {
    fn name(&self) -> &'static str {
        match self {
            Self::OpenAi(e) => e.name(),
            Self::Anthropic(e) => e.name(),
        }
    }

    async fn translate(&self, req: &TranslateRequest) -> Result<String, EngineError> {
        match self {
            Self::OpenAi(e) => e.translate(req).await,
            Self::Anthropic(e) => e.translate(req).await,
        }
    }
}

/// 一次翻译结果（CLI JSON 契约的句式输出）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Translation {
    pub source_lang: Lang,
    pub target_lang: Lang,
    pub text: String,
    pub translation: String,
    /// 成功引擎名（"ecdict"/"cedict"/"google"/"youdao"/"bing"/provider id）；
    /// 全链失败时为空串（`translation` 同为空串）。
    pub engine: String,
}

/// 词条卡片渲染为纯文本：每词性一行「pos gloss1；gloss2」（'；' 连接），
/// 行间 '\n'；无词性前缀的行只列释义。供 word 路由的 `Translation.translation`
/// 与流式单次 delta 使用。
pub fn card_text(card: &WordCard) -> String {
    card.pos
        .iter()
        .map(|pg| {
            if pg.pos.is_empty() {
                pg.gloss.join("；")
            } else {
                format!("{} {}", pg.pos, pg.gloss.join("；"))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub struct Pipeline {
    pub ecdict: EcdictDb,
    pub cedict: CedictDb,
    /// 默认引擎链：`default_engine == "llm"` 且 provider 可用时
    /// [llm, google, youdao, bing]，否则 [google, youdao, bing]。
    /// Google 是活的免费端点、排第一兜底；youdao/bing 端点已死但保留
    /// 殿后（显式 `--engine youdao/bing` 仍可用）。
    pub chain: Chain,
    /// 默认 LLM provider 构造出的引擎；未指定 default_provider 时 None。
    /// `Arc` 使同一实例同时进入 chain 与流式路径。
    pub llm: Option<Arc<LlmEngine>>,
}

impl Pipeline {
    /// `$PARDON_HOME/dict/`（缺省 `dirs::data_dir()/pardon`）下加载词典，
    /// 按 config 组装引擎链。词典文件缺失 → 空库降级 + stderr 提示；
    /// 存在但打不开 → 报错。
    pub fn from_config(cfg: &AppConfig) -> anyhow::Result<Self> {
        let dict_dir = pardon_home()?.join("dict");
        let ecdict = load_ecdict(&dict_dir.join("ecdict.sqlite"))?;
        let cedict = load_cedict(&dict_dir.join("cedict.sqlite"))?;

        let llm = build_llm(cfg);
        // Google 免费（非官方）端点当前可用，作链首兜底；youdao/bing 已死仍殿后
        let mut engines: Vec<Arc<dyn Engine>> = vec![
            Arc::new(GoogleEngine::new()),
            Arc::new(YoudaoEngine::new()),
            Arc::new(BingEngine::new()),
        ];
        if cfg.default_engine == "llm" {
            if let Some(llm) = &llm {
                engines.insert(0, llm.clone());
            }
        }
        Ok(Self {
            ecdict,
            cedict,
            chain: Chain { engines },
            llm,
        })
    }

    /// 查词：zh → CEDICT，en → ECDICT。未命中返回 `found:false` 卡片并填
    /// suggestions（en 用 ECDICT 编辑距离推荐；zh 无推荐来源，留空）。
    /// 输入先过 [`router::sanitize`]（零宽字符等会让整串词典失配）。
    pub fn lookup(&self, word: &str) -> WordCard {
        let word = router::sanitize(word);
        let lang = lang::detect(&word);
        let miss = || WordCard {
            found: false,
            word: word.to_string(),
            phonetic: None,
            pos: Vec::new(),
            definition: Vec::new(),
            exchange: None,
            collins: None,
            oxford: false,
            tags: Vec::new(),
            source: if lang == Lang::Zh { "cedict" } else { "ecdict" }.to_string(),
            suggestions: if lang == Lang::En {
                self.ecdict.suggest(&word)
            } else {
                Vec::new()
            },
        };
        match lang {
            Lang::Zh => self.cedict.lookup(&word).unwrap_or_else(miss),
            Lang::En => self.ecdict.lookup(&word).unwrap_or_else(miss),
        }
    }

    /// 非流式翻译。word 路由 → 词典卡片文本（engine = 卡片来源）；
    /// sentence 路由 → 引擎链（全链失败时 translation/engine 为空串）。
    pub async fn translate(&self, text: &str) -> Translation {
        match router::classify(text, &self.ecdict, &self.cedict) {
            Route::Word(w) => {
                let card = self.lookup(&w);
                let (source_lang, target_lang) = lang::direction(&w);
                Translation {
                    source_lang,
                    target_lang,
                    text: w,
                    translation: card_text(&card),
                    engine: card.source,
                }
            }
            Route::Sentence(t) => {
                let (source_lang, target_lang) = lang::direction(&t);
                let req = TranslateRequest {
                    text: t.clone(),
                    from: source_lang,
                    to: target_lang,
                };
                let (translation, engine): (String, &str) =
                    self.chain.translate(&req).await.unwrap_or_default();
                Translation {
                    source_lang,
                    target_lang,
                    text: t,
                    translation,
                    engine: engine.to_string(),
                }
            }
        }
    }

    /// 流式翻译。word 路由：`on_delta` 一次性收到整张卡片文本（--stream 下
    /// word 也呈现 meta → 单 delta → result 的统一节奏）。sentence 路由：
    /// 有 LLM → 逐增量转发（空串 delta 跳过），失败回退引擎链并把完整译文
    /// 作单次 delta；无 LLM → 引擎链 + 单次完整 delta。链也失败 → Err。
    pub async fn translate_stream(
        &self,
        text: &str,
        mut on_delta: impl FnMut(&str) + Send,
    ) -> Result<Translation, EngineError> {
        match router::classify(text, &self.ecdict, &self.cedict) {
            Route::Word(w) => {
                let card = self.lookup(&w);
                let translation = card_text(&card);
                on_delta(&translation);
                let (source_lang, target_lang) = lang::direction(&w);
                Ok(Translation {
                    source_lang,
                    target_lang,
                    text: w,
                    engine: card.source,
                    translation,
                })
            }
            Route::Sentence(t) => {
                let (source_lang, target_lang) = lang::direction(&t);
                let req = TranslateRequest {
                    text: t.clone(),
                    from: source_lang,
                    to: target_lang,
                };
                if let Some(llm) = &self.llm {
                    // 部分端点会发空串增量：跳过不转发
                    let streamed = llm
                        .translate_stream(&req, |d| {
                            if !d.is_empty() {
                                on_delta(d)
                            }
                        })
                        .await;
                    if let Ok(translation) = streamed {
                        return Ok(Translation {
                            source_lang,
                            target_lang,
                            text: t,
                            translation,
                            engine: llm.id().to_string(),
                        });
                    }
                    // LLM 失败 → 引擎链兜底（完整译文一次性 delta）
                }
                let (translation, engine) = self.chain.translate(&req).await?;
                on_delta(&translation);
                Ok(Translation {
                    source_lang,
                    target_lang,
                    text: t,
                    translation,
                    engine: engine.to_string(),
                })
            }
        }
    }

    /// 显式单引擎链（CLI `-e` 用）：google / youdao / bing / llm（须已配置
    /// provider）；未知引擎名或 llm 未配置 → Err。
    pub fn chain_with(&self, engine_name: &str) -> anyhow::Result<Chain> {
        let engine: Arc<dyn Engine> = match engine_name {
            "google" => Arc::new(GoogleEngine::new()),
            "youdao" => Arc::new(YoudaoEngine::new()),
            "bing" => Arc::new(BingEngine::new()),
            "llm" => self.llm.clone().ok_or_else(|| {
                anyhow::anyhow!("engine \"llm\" requested but no llm provider is configured")
            })?,
            other => {
                return Err(anyhow::anyhow!(
                    "unknown engine {other:?}, expected one of llm/google/youdao/bing"
                ))
            }
        };
        Ok(Chain {
            engines: vec![engine],
        })
    }
}

/// 数据根目录：`PARDON_HOME` 环境变量优先（非空时）；缺省
/// `dirs::data_dir()/pardon`。词典在 `<home>/dict`，TTS 缓存在 `<home>/tts`。
pub fn pardon_home() -> anyhow::Result<PathBuf> {
    if let Ok(home) = std::env::var("PARDON_HOME") {
        if !home.is_empty() {
            return Ok(PathBuf::from(home));
        }
    }
    dirs::data_dir()
        .map(|d| d.join("pardon"))
        .ok_or_else(|| anyhow::anyhow!("PARDON_HOME unset and XDG data dir unavailable"))
}

/// 词典存在则只读打开；缺失则空库降级 + stderr 提示导入命令。
fn load_ecdict(path: &Path) -> anyhow::Result<EcdictDb> {
    if path.exists() {
        return EcdictDb::open(path).with_context(|| format!("open ecdict at {}", path.display()));
    }
    eprintln!(
        "pardon: ecdict dictionary not found at {}; run the importer (see dicts/README.md)",
        path.display()
    );
    EcdictDb::empty()
}

fn load_cedict(path: &Path) -> anyhow::Result<CedictDb> {
    if path.exists() {
        return CedictDb::open(path).with_context(|| format!("open cedict at {}", path.display()));
    }
    eprintln!(
        "pardon: cedict dictionary not found at {}; run the importer (see dicts/README.md)",
        path.display()
    );
    CedictDb::empty()
}

/// 由 `llm.default_provider` 指向的 provider 构造 LLM 引擎；未指定或
/// 找不到 provider → None。base_url 去掉尾部 '/'（避免拼出 `//…`）。
/// openai 无 key → "none"（部分本地端点不校验）；anthropic 无 key → 空串
/// 照常构造（请求 401 后由引擎链兜底）；ollama → OpenAI 兼容层，同样
/// 透传 provider 的 base_url / 提示词覆盖（base_url 留空才用本机 11434
/// 缺省，见 [`OpenAiConfig::for_ollama`]）。
fn build_llm(cfg: &AppConfig) -> Option<Arc<LlmEngine>> {
    let id = cfg.llm.default_provider.as_str();
    if id.is_empty() {
        return None;
    }
    let p = cfg.llm.providers.iter().find(|p| p.id == id)?;
    let base_url = p.base_url.trim_end_matches('/').to_string();
    let engine = match p.provider_type {
        ProviderType::Openai => LlmEngine::OpenAi(OpenAiEngine::new(OpenAiConfig {
            id: p.id.clone(),
            base_url,
            api_key: p
                .resolve_api_key(std::env::var)
                .unwrap_or_else(|| "none".into()),
            model: p.model.clone(),
            system_prompt: p.system_prompt.clone(),
            user_prompt_template: p.user_prompt_template.clone(),
        })),
        ProviderType::Anthropic => LlmEngine::Anthropic(AnthropicEngine::new(AnthropicConfig {
            id: p.id.clone(),
            base_url,
            api_key: p.resolve_api_key(std::env::var).unwrap_or_default(),
            model: p.model.clone(),
            system_prompt: p.system_prompt.clone(),
            user_prompt_template: p.user_prompt_template.clone(),
        })),
        ProviderType::Ollama => {
            let base_url = if base_url.is_empty() {
                OLLAMA_DEFAULT_BASE_URL.to_string()
            } else {
                base_url
            };
            LlmEngine::OpenAi(OpenAiEngine::new(OpenAiConfig {
                id: p.id.clone(),
                base_url,
                // 无 key 来源时回退 "ollama"（部分 OpenAI 兼容层要求非空 key）
                api_key: p
                    .resolve_api_key(std::env::var)
                    .unwrap_or_else(|| "ollama".into()),
                model: p.model.clone(),
                system_prompt: p.system_prompt.clone(),
                user_prompt_template: p.user_prompt_template.clone(),
            }))
        }
    };
    Some(Arc::new(engine))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dict::PosGloss;

    fn card(pos: Vec<PosGloss>) -> WordCard {
        WordCard {
            found: true,
            word: "w".into(),
            phonetic: None,
            pos,
            definition: Vec::new(),
            exchange: None,
            collins: None,
            oxford: false,
            tags: Vec::new(),
            source: "ecdict".into(),
            suggestions: Vec::new(),
        }
    }

    #[test]
    fn card_text_renders_pos_lines_joined_by_fullwidth_semicolon() {
        let c = card(vec![
            PosGloss {
                pos: "n.".into(),
                gloss: vec!["跑步".into()],
            },
            PosGloss {
                pos: "v.".into(),
                gloss: vec!["跑".into(), "运转".into()],
            },
        ]);
        assert_eq!(card_text(&c), "n. 跑步\nv. 跑；运转");
    }

    #[test]
    fn card_text_bare_glosses_have_no_pos_prefix() {
        let c = card(vec![PosGloss {
            pos: "".into(),
            gloss: vec!["you (informal)".into(), "hello".into()],
        }]);
        assert_eq!(card_text(&c), "you (informal)；hello");
    }

    #[test]
    fn card_text_empty_card_is_empty() {
        assert_eq!(card_text(&card(Vec::new())), "");
    }
}

//! TOML 配置：加载、校验与默认配置写入。

use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    /// "llm" | "youdao" | "bing"；缺省 "youdao"（免配置可用）。
    #[serde(default = "default_engine_youdao")]
    pub default_engine: String,
    #[serde(default)]
    pub llm: LlmConfig,
}

fn default_engine_youdao() -> String {
    "youdao".into()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self { default_engine: "youdao".into(), llm: LlmConfig::default() }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LlmConfig {
    /// 对应 `providers[].id`；空 = 不启用 LLM。
    #[serde(default)]
    pub default_provider: String,
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    #[serde(rename = "type")]
    pub provider_type: ProviderType,
    pub base_url: String,
    pub model: String,
    /// 明文 key（用户自担风险）；优先于 `api_key_env`。
    #[serde(default)]
    pub api_key: Option<String>,
    /// 环境变量名；`api_key` 缺失时从该变量读取。
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub user_prompt_template: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderType {
    Openai,
    Anthropic,
    Ollama,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config invalid: {field}: {reason}")]
    Invalid { field: String, reason: String },
    #[error("config io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("config parse error: {0}")]
    Parse(String),
}

/// `$XDG_CONFIG_HOME/pardon/config.toml`（`dirs::config_dir`）；拿不到
/// 配置根目录时退化为相对路径 `pardon/config.toml`。
pub fn default_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("pardon")
        .join("config.toml")
}

/// 读取配置：`PARDON_CONFIG` 环境变量覆盖路径；文件不存在 → 内置默认。
pub fn load() -> Result<AppConfig, ConfigError> {
    let path = std::env::var("PARDON_CONFIG")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    match std::fs::read_to_string(&path) {
        Ok(text) => load_from_str(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AppConfig::default()),
        Err(e) => Err(ConfigError::Io(e)),
    }
}

/// 解析 + 校验（测试友好）。
pub fn load_from_str(s: &str) -> Result<AppConfig, ConfigError> {
    let cfg: AppConfig = toml::from_str(s).map_err(|e| ConfigError::Parse(e.to_string()))?;
    cfg.validate()?;
    Ok(cfg)
}

/// 写出带注释的默认配置文档（`config init` 用）；父目录不存在则创建。
pub fn write_default(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(path, DEFAULT_CONFIG_TOML)?;
    Ok(())
}

const DEFAULT_CONFIG_TOML: &str = r#"# pardon 配置文件
# 路径：~/.config/pardon/config.toml（可用 PARDON_CONFIG 环境变量覆盖）
#
# 默认引擎：llm | youdao | bing。缺省 youdao，开箱即用。
default_engine = "youdao"

[llm]
# default_provider 指向下方 providers[].id；留空 = 不启用 LLM。
default_provider = ""
providers = []

# LLM provider 示例（取消注释并按需修改）：
#
# [[llm.providers]]
# id = "ds"                            # 唯一标识，default_provider 引用它
# type = "openai"                      # openai | anthropic | ollama
# base_url = "https://api.deepseek.com/v1"
# model = "deepseek-chat"
# # key 二选一：api_key 明文（自担风险）优先；api_key_env 指向环境变量名。
# api_key_env = "DEEPSEEK_API_KEY"
# # system_prompt / user_prompt_template 可选，覆盖内置提示词。
#
# [[llm.providers]]
# id = "local"
# type = "ollama"                      # Ollama 无需 api_key
# base_url = "http://127.0.0.1:11434/v1"
# model = "qwen2.5:7b"
"#;

impl AppConfig {
    /// 校验配置；失败时 `field` 携带字段路径（如 `llm.providers[0].id`）。
    pub fn validate(&self) -> Result<(), ConfigError> {
        const KNOWN_ENGINES: [&str; 3] = ["llm", "youdao", "bing"];
        if !KNOWN_ENGINES.contains(&self.default_engine.as_str()) {
            return Err(ConfigError::Invalid {
                field: "default_engine".into(),
                reason: format!(
                    "unknown engine {:?}, expected one of llm/youdao/bing",
                    self.default_engine
                ),
            });
        }

        // 每个 provider：id 非空且唯一、base_url/model 非空。
        // （api_key 可省略：type="ollama" 引擎侧回退为 "ollama"，校验不强求。）
        let mut seen_ids: Vec<&str> = Vec::new();
        for (i, p) in self.llm.providers.iter().enumerate() {
            if p.id.is_empty() {
                return Err(ConfigError::Invalid {
                    field: format!("llm.providers[{i}].id"),
                    reason: "id must be non-empty".into(),
                });
            }
            if seen_ids.contains(&p.id.as_str()) {
                return Err(ConfigError::Invalid {
                    field: format!("llm.providers[{i}].id"),
                    reason: format!("duplicate provider id {:?}", p.id),
                });
            }
            seen_ids.push(&p.id);
            if p.base_url.is_empty() {
                return Err(ConfigError::Invalid {
                    field: format!("llm.providers[{i}].base_url"),
                    reason: "base_url must be non-empty".into(),
                });
            }
            if p.model.is_empty() {
                return Err(ConfigError::Invalid {
                    field: format!("llm.providers[{i}].model"),
                    reason: "model must be non-empty".into(),
                });
            }
        }

        // default_engine = "llm" 时 default_provider 必须能解析到某个 provider。
        if self.default_engine == "llm" {
            if self.llm.default_provider.is_empty() {
                return Err(ConfigError::Invalid {
                    field: "llm.default_provider".into(),
                    reason: "must name a providers[].id when default_engine = \"llm\"".into(),
                });
            }
            if !self.llm.providers.iter().any(|p| p.id == self.llm.default_provider) {
                return Err(ConfigError::Invalid {
                    field: "llm.default_provider".into(),
                    reason: format!(
                        "no provider with id {:?}",
                        self.llm.default_provider
                    ),
                });
            }
        }
        Ok(())
    }
}

impl ProviderConfig {
    /// `api_key` 有则用之；否则读 `api_key_env` 指定的环境变量（getter 注入以便测试）。
    pub fn resolve_api_key(
        &self,
        get_env: impl Fn(String) -> Result<String, std::env::VarError>,
    ) -> Option<String> {
        if let Some(key) = &self.api_key {
            return Some(key.clone());
        }
        let var = self.api_key_env.as_deref()?;
        get_env(var.to_string()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPENAI_TOML: &str = r#"
default_engine = "llm"
[llm]
default_provider = "ds"
[[llm.providers]]
id = "ds"
type = "openai"
base_url = "https://api.deepseek.com/v1"
api_key_env = "DEEPSEEK_API_KEY"
model = "deepseek-chat"
"#;

    fn provider(id: &str) -> ProviderConfig {
        ProviderConfig {
            id: id.into(),
            provider_type: ProviderType::Openai,
            base_url: "https://api.deepseek.com/v1".into(),
            model: "deepseek-chat".into(),
            api_key: None,
            api_key_env: None,
            system_prompt: None,
            user_prompt_template: None,
        }
    }

    #[test]
    fn builtin_defaults_are_out_of_box_usable() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.default_engine, "youdao");
        assert_eq!(cfg.llm.default_provider, "");
        assert!(cfg.llm.providers.is_empty());
    }

    #[test]
    fn empty_toml_parses_to_builtin_defaults() {
        assert_eq!(load_from_str("").unwrap(), AppConfig::default());
    }

    #[test]
    fn parses_openai_provider_with_api_key_env() {
        let cfg = load_from_str(OPENAI_TOML).unwrap();
        assert_eq!(cfg.default_engine, "llm");
        assert_eq!(cfg.llm.default_provider, "ds");
        assert_eq!(cfg.llm.providers.len(), 1);
        let p = &cfg.llm.providers[0];
        assert_eq!(p.id, "ds");
        assert_eq!(p.provider_type, ProviderType::Openai);
        assert_eq!(p.base_url, "https://api.deepseek.com/v1");
        assert_eq!(p.model, "deepseek-chat");
        assert_eq!(p.api_key_env.as_deref(), Some("DEEPSEEK_API_KEY"));
        assert!(p.api_key.is_none());
        assert!(p.system_prompt.is_none());
        assert!(p.user_prompt_template.is_none());
    }

    #[test]
    fn unknown_default_engine_is_invalid() {
        let err = load_from_str(r#"default_engine = "google""#).unwrap_err();
        match &err {
            ConfigError::Invalid { field, reason } => {
                assert_eq!(field, "default_engine");
                assert!(reason.contains("google"), "reason: {reason}");
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn llm_engine_requires_resolvable_default_provider() {
        let err = load_from_str(
            r#"
default_engine = "llm"
[llm]
default_provider = "missing"
[[llm.providers]]
id = "ds"
type = "openai"
base_url = "https://api.deepseek.com/v1"
model = "deepseek-chat"
"#,
        )
        .unwrap_err();
        match &err {
            ConfigError::Invalid { field, .. } => assert_eq!(field, "llm.default_provider"),
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn llm_engine_with_empty_default_provider_is_invalid() {
        let err = load_from_str("default_engine = \"llm\"\n").unwrap_err();
        match &err {
            ConfigError::Invalid { field, .. } => assert_eq!(field, "llm.default_provider"),
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_provider_ids_are_invalid() {
        let err = load_from_str(
            r#"
default_engine = "bing"
[[llm.providers]]
id = "ds"
type = "openai"
base_url = "https://api.deepseek.com/v1"
model = "deepseek-chat"
[[llm.providers]]
id = "ds"
type = "openai"
base_url = "https://api.deepseek.com/v1"
model = "deepseek-chat"
"#,
        )
        .unwrap_err();
        match &err {
            ConfigError::Invalid { field, reason } => {
                assert_eq!(field, "llm.providers[1].id");
                assert!(reason.contains("ds"), "reason: {reason}");
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn empty_provider_fields_are_invalid() {
        let id_missing = load_from_str(
            r#"
[[llm.providers]]
id = ""
type = "openai"
base_url = "https://api.deepseek.com/v1"
model = "deepseek-chat"
"#,
        )
        .unwrap_err();
        match &id_missing {
            ConfigError::Invalid { field, .. } => assert_eq!(field, "llm.providers[0].id"),
            other => panic!("expected Invalid, got {other:?}"),
        }

        let base_missing = load_from_str(
            r#"
[[llm.providers]]
id = "ds"
type = "openai"
base_url = ""
model = "deepseek-chat"
"#,
        )
        .unwrap_err();
        match &base_missing {
            ConfigError::Invalid { field, .. } => assert_eq!(field, "llm.providers[0].base_url"),
            other => panic!("expected Invalid, got {other:?}"),
        }

        let model_missing = load_from_str(
            r#"
[[llm.providers]]
id = "ds"
type = "openai"
base_url = "https://api.deepseek.com/v1"
model = ""
"#,
        )
        .unwrap_err();
        match &model_missing {
            ConfigError::Invalid { field, .. } => assert_eq!(field, "llm.providers[0].model"),
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn ollama_provider_may_omit_api_key() {
        let cfg = load_from_str(
            r#"
[[llm.providers]]
id = "local"
type = "ollama"
base_url = "http://127.0.0.1:11434/v1"
model = "qwen2.5:7b"
"#,
        )
        .unwrap();
        let p = &cfg.llm.providers[0];
        assert_eq!(p.provider_type, ProviderType::Ollama);
        // 无 key 来源 → None；引擎构造时对 ollama 回退为 "ollama"
        assert!(p.resolve_api_key(|_| Ok("unused".into())).is_none());
    }

    #[test]
    fn bad_toml_syntax_is_parse_error() {
        let err = load_from_str("default_engine = [unclosed").unwrap_err();
        assert!(matches!(err, ConfigError::Parse(_)), "got {err:?}");
    }

    #[test]
    fn resolve_api_key_prefers_inline_over_env() {
        let p = ProviderConfig {
            api_key: Some("sk-plain".into()),
            api_key_env: Some("SOME_VAR".into()),
            ..provider("ds")
        };
        // getter 返回值必须被忽略：api_key 优先
        let key = p.resolve_api_key(|_| Ok("from-env".into())).unwrap();
        assert_eq!(key, "sk-plain");
    }

    #[test]
    fn resolve_api_key_reads_env_var_by_name() {
        let p = ProviderConfig {
            api_key_env: Some("DEEPSEEK_API_KEY".into()),
            ..provider("ds")
        };
        let key = p
            .resolve_api_key(|name| {
                (name == "DEEPSEEK_API_KEY")
                    .then(|| "sekret".to_string())
                    .ok_or(std::env::VarError::NotPresent)
            })
            .unwrap();
        assert_eq!(key, "sekret");
        // 变量不存在 → None
        assert!(p.resolve_api_key(|_| Err(std::env::VarError::NotPresent)).is_none());
    }

    #[test]
    fn resolve_api_key_none_when_no_source() {
        let p = provider("ds");
        assert!(p.resolve_api_key(|_| Ok("unused".into())).is_none());
        // 签名兼容 std::env::var（K=String 推断），不依赖真实环境
        let _compiles: Option<String> = p.resolve_api_key(std::env::var);
    }

    #[test]
    fn default_config_path_points_into_pardon_dir() {
        let p = default_config_path();
        assert!(p.ends_with(Path::new("pardon").join("config.toml")));
    }

    #[test]
    fn write_default_roundtrips_to_builtin_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        write_default(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains('#'), "默认配置应带注释");
        assert_eq!(load_from_str(&text).unwrap(), AppConfig::default());
    }
}

//! TOML 配置：加载、校验与默认配置写入。

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    /// "llm" | "youdao" | "bing"；缺省 "youdao"。
    /// 注意：有道/Bing 的免费 web 端点已于 2026-09 失效，句子翻译需配置 LLM provider（见 README）。
    #[serde(default = "default_engine_youdao")]
    pub default_engine: String,
    #[serde(default)]
    pub llm: LlmConfig,
    #[serde(default)]
    pub daemon: DaemonConfig,
}

fn default_engine_youdao() -> String {
    "youdao".into()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            default_engine: "youdao".into(),
            llm: LlmConfig::default(),
            daemon: DaemonConfig::default(),
        }
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DaemonConfig {
    /// HTTP 触发口绑定地址；只允许回环地址（PARDON_HTTP_BIND 环境变量可覆盖）。
    #[serde(default = "default_http_bind")]
    pub http_bind: String,
    /// 「复制即翻译」总开关（剪贴板自动监听）。
    #[serde(default = "default_true")]
    pub auto_translate: bool,
    /// 自动翻译的文本字节上限，超过则忽略（spec §6 默认 5KB）。
    #[serde(default = "default_max_text_bytes")]
    pub max_text_bytes: usize,
    /// 同内容去重时间窗毫秒（spec §5.5 防回环）。
    #[serde(default = "default_dedup_window_ms")]
    pub dedup_window_ms: u64,
    /// 译文自动写回剪贴板（写入前登记防回环哈希）。
    #[serde(default)]
    pub copy_translation: bool,
    /// 桌面通知显示时长毫秒。
    #[serde(default = "default_notify_timeout_ms")]
    pub notify_timeout_ms: u32,
}

fn default_http_bind() -> String {
    "127.0.0.1:7377".into()
}
fn default_true() -> bool {
    true
}
fn default_max_text_bytes() -> usize {
    5120
}
fn default_dedup_window_ms() -> u64 {
    10_000
}
fn default_notify_timeout_ms() -> u32 {
    5000
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            http_bind: default_http_bind(),
            auto_translate: default_true(),
            max_text_bytes: default_max_text_bytes(),
            dedup_window_ms: default_dedup_window_ms(),
            copy_translation: false,
            notify_timeout_ms: default_notify_timeout_ms(),
        }
    }
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

/// 生效的配置文件路径：`PARDON_CONFIG` 环境变量（非空时）覆盖；缺省
/// [`default_config_path`]。读取（[`load`]）与 `config --init` 共用此解析。
pub fn config_path() -> PathBuf {
    std::env::var("PARDON_CONFIG")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path)
}

/// 生效的 HTTP 绑定地址：`PARDON_HTTP_BIND`（非空）覆盖配置值；
/// 两者都必须是合法 SocketAddr 且为回环地址（localhost-only 契约）。
pub fn effective_http_bind(cfg: &AppConfig) -> Result<SocketAddr, ConfigError> {
    let raw = std::env::var("PARDON_HTTP_BIND")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| cfg.daemon.http_bind.clone());
    let invalid = |reason: String| ConfigError::Invalid {
        field: "daemon.http_bind".into(),
        reason,
    };
    let addr: SocketAddr = raw
        .parse()
        .map_err(|e| invalid(format!("not a valid socket address ({e})")))?;
    if !addr.ip().is_loopback() {
        return Err(invalid(
            "must be a loopback address (127.x.x.x / ::1)".into(),
        ));
    }
    Ok(addr)
}

/// 读取配置：路径见 [`config_path`]；文件不存在 → 内置默认。
pub fn load() -> Result<AppConfig, ConfigError> {
    let path = config_path();
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
# 默认引擎：llm | youdao | bing。注意：有道/Bing 的免费 web 端点已于
# 2026-09 失效，句子翻译需配置 LLM provider（见下方示例）；
# 查词（离线词典）与发音不受影响。
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
            if !self
                .llm
                .providers
                .iter()
                .any(|p| p.id == self.llm.default_provider)
            {
                return Err(ConfigError::Invalid {
                    field: "llm.default_provider".into(),
                    reason: format!("no provider with id {:?}", self.llm.default_provider),
                });
            }
        }

        let d = &self.daemon;
        match d.http_bind.parse::<SocketAddr>() {
            Ok(addr) if addr.ip().is_loopback() => {}
            Ok(_) => {
                return Err(ConfigError::Invalid {
                    field: "daemon.http_bind".into(),
                    reason: "must be a loopback address (127.x.x.x / ::1)".into(),
                })
            }
            Err(e) => {
                return Err(ConfigError::Invalid {
                    field: "daemon.http_bind".into(),
                    reason: format!("not a valid socket address ({e})"),
                })
            }
        }
        if d.max_text_bytes == 0 {
            return Err(ConfigError::Invalid {
                field: "daemon.max_text_bytes".into(),
                reason: "must be >= 1".into(),
            });
        }
        if d.dedup_window_ms == 0 {
            return Err(ConfigError::Invalid {
                field: "daemon.dedup_window_ms".into(),
                reason: "must be >= 1".into(),
            });
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

    // 串行化会改动 PARDON_HTTP_BIND 的测试，避免并行互踩。
    static HTTP_BIND_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
        assert!(p
            .resolve_api_key(|_| Err(std::env::VarError::NotPresent))
            .is_none());
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

    #[test]
    fn daemon_section_defaults() {
        let cfg = load_from_str("").unwrap();
        assert_eq!(cfg.daemon.http_bind, "127.0.0.1:7377");
        assert!(cfg.daemon.auto_translate);
        assert_eq!(cfg.daemon.max_text_bytes, 5120);
        assert_eq!(cfg.daemon.dedup_window_ms, 10_000);
        assert!(!cfg.daemon.copy_translation);
        assert_eq!(cfg.daemon.notify_timeout_ms, 5000);
    }

    #[test]
    fn daemon_section_overrides() {
        let cfg = load_from_str(
            r#"
[daemon]
http_bind = "127.0.0.1:9999"
auto_translate = false
max_text_bytes = 100
dedup_window_ms = 2000
copy_translation = true
notify_timeout_ms = 3000
"#,
        )
        .unwrap();
        let d = &cfg.daemon;
        assert_eq!(d.http_bind, "127.0.0.1:9999");
        assert!(!d.auto_translate);
        assert_eq!(d.max_text_bytes, 100);
        assert_eq!(d.dedup_window_ms, 2000);
        assert!(d.copy_translation);
        assert_eq!(d.notify_timeout_ms, 3000);
    }

    #[test]
    fn non_loopback_http_bind_is_invalid() {
        let err = load_from_str(
            r#"[daemon]
http_bind = "0.0.0.0:7377"
"#,
        )
        .unwrap_err();
        match &err {
            ConfigError::Invalid { field, .. } => assert_eq!(field, "daemon.http_bind"),
            other => panic!("expected Invalid, got {other:?}"),
        }
        let err = load_from_str(
            r#"[daemon]
http_bind = "not an addr"
"#,
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::Invalid { .. }), "got {err:?}");
    }

    #[test]
    fn zero_limits_are_invalid() {
        for bad in ["max_text_bytes = 0", "dedup_window_ms = 0"] {
            let toml = format!("[daemon]\n{bad}\n");
            assert!(load_from_str(&toml).is_err(), "{bad} should be invalid");
        }
    }

    #[test]
    fn effective_http_bind_resolves_config_and_env() {
        let _lock = HTTP_BIND_LOCK.lock().unwrap();
        let cfg = load_from_str("").unwrap();
        // 无环境变量 → 配置默认
        std::env::remove_var("PARDON_HTTP_BIND");
        let addr = effective_http_bind(&cfg).unwrap();
        assert_eq!(addr.port(), 7377);
        assert!(addr.ip().is_loopback());
        // 环境变量覆盖（含 port 0，测试/调试用）
        std::env::set_var("PARDON_HTTP_BIND", "127.0.0.1:0");
        assert_eq!(effective_http_bind(&cfg).unwrap().port(), 0);
        // 非回环拒绝
        std::env::set_var("PARDON_HTTP_BIND", "192.168.1.5:7377");
        assert!(matches!(
            effective_http_bind(&cfg),
            Err(ConfigError::Invalid { .. })
        ));
        std::env::remove_var("PARDON_HTTP_BIND");
    }
}

//! 集成测试：config 模块经真实文件 + `PARDON_CONFIG` 环境变量加载。
//!
//! `std::env::set_var` 影响整个进程，且测试默认并行：所有触碰
//! `PARDON_CONFIG` 的操作收进单个测试函数内顺序执行，其余测试不读该变量，
//! 以此避免竞争。

use pardon_core::config::{self, AppConfig, ProviderConfig, ProviderType};
use std::env::VarError;

const FILE_TOML: &str = r#"
default_engine = "llm"

[llm]
default_provider = "ds"

[[llm.providers]]
id = "ds"
type = "openai"
base_url = "https://api.deepseek.com/v1"
api_key_env = "DEEPSEEK_API_KEY"
model = "deepseek-chat"

[[llm.providers]]
id = "local"
type = "ollama"
base_url = "http://127.0.0.1:11434/v1"
model = "qwen2.5:7b"
"#;

#[test]
fn load_via_env_var_then_missing_file_falls_back_to_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("config.toml");
    std::fs::write(&cfg_path, FILE_TOML).unwrap();

    // PARDON_CONFIG 覆盖默认查找路径 → 加载成功
    std::env::set_var("PARDON_CONFIG", &cfg_path);
    let cfg = config::load().unwrap();
    assert_eq!(cfg.default_engine, "llm");
    assert_eq!(cfg.llm.default_provider, "ds");
    assert_eq!(cfg.llm.providers.len(), 2);
    assert_eq!(cfg.llm.providers[0].provider_type, ProviderType::Openai);
    assert_eq!(cfg.llm.providers[1].provider_type, ProviderType::Ollama);
    assert_eq!(
        cfg.llm.providers[0].api_key_env.as_deref(),
        Some("DEEPSEEK_API_KEY")
    );

    // 指向不存在的文件 → 内置默认（免配置可用）
    let missing = dir.path().join("missing.toml");
    std::env::set_var("PARDON_CONFIG", &missing);
    assert_eq!(config::load().unwrap(), AppConfig::default());

    std::env::remove_var("PARDON_CONFIG");
}

#[test]
fn resolve_api_key_prefers_inline_then_env_then_none() {
    let p = ProviderConfig {
        id: "ds".into(),
        provider_type: ProviderType::Openai,
        base_url: "https://api.deepseek.com/v1".into(),
        model: "deepseek-chat".into(),
        api_key: None,
        api_key_env: None,
        system_prompt: None,
        user_prompt_template: None,
    };

    // 两处都没有 → None（getter 不应被调用）
    assert!(p.resolve_api_key(|_| Ok("unexpected".into())).is_none());

    // 仅 api_key_env：getter 命中 → Some
    let p = ProviderConfig { api_key_env: Some("DEEPSEEK_API_KEY".into()), ..p };
    let key = p
        .resolve_api_key(|name| {
            (name == "DEEPSEEK_API_KEY")
                .then(|| "env-value".to_string())
                .ok_or(VarError::NotPresent)
        })
        .unwrap();
    assert_eq!(key, "env-value");

    // getter 未命中（变量不存在）→ None
    assert!(p.resolve_api_key(|_| Err(VarError::NotPresent)).is_none());

    // api_key 明文优先于 api_key_env
    let p = ProviderConfig { api_key: Some("sk-plain".into()), ..p };
    assert_eq!(
        p.resolve_api_key(|_| Err(VarError::NotPresent)).as_deref(),
        Some("sk-plain")
    );
}

#[test]
fn write_default_creates_parseable_commented_config() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pardon").join("config.toml");
    config::write_default(&path).unwrap();
    assert!(path.is_file());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains('#'), "默认配置应带注释");
    assert_eq!(config::load_from_str(&text).unwrap(), AppConfig::default());
}

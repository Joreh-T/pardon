//! `pardon speak <text> [--lang auto|en|zh]`：朗读文本（TTS）。
//!
//! 成功时**无任何 stdout 输出**（朗读即是输出）；参数或 TTS 错误上抛走
//! 统一错误出口（stderr JSON + exit 2）。缓存目录 `$PARDON_HOME/tts`。
//! 测试开关：`PARDON_TTS_DISABLE=1` 时 `Tts::speak` 直接 Ok（不下载不播放）。

use pardon_core::lang::{self, Lang};

/// 语言旗标 → Lang：`auto` → 文本检测；`en`/`zh` → 显式（解析复用
/// translate 的 [`crate::cmd_translate::parse_lang`]，同一错误消息契约）。
fn resolve_lang(flag: &str, text: &str) -> anyhow::Result<Lang> {
    match flag {
        "auto" => Ok(lang::detect(text)),
        other => crate::cmd_translate::parse_lang(other),
    }
}

/// 朗读 `text`。返回进程退出码：0 成功 / 2 参数或 TTS 错误。
/// 空 / 纯空白文本是参数错误：直接 exit 2，不做无意义的 TTS 往返。
pub async fn run(text: &str, lang_flag: &str) -> anyhow::Result<i32> {
    if text.trim().is_empty() {
        anyhow::bail!("empty text: nothing to speak");
    }
    let lang = resolve_lang(lang_flag, text)?;
    let cache_dir = pardon_core::pipeline::pardon_home()?.join("tts");
    pardon_core::tts::Tts::new(cache_dir)
        .speak(text, lang)
        .await?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_lang_auto_detects_from_text() {
        assert_eq!(resolve_lang("auto", "hello").unwrap(), Lang::En);
        assert_eq!(resolve_lang("auto", "你好").unwrap(), Lang::Zh);
    }

    #[test]
    fn resolve_lang_explicit_overrides_detection() {
        assert_eq!(resolve_lang("zh", "hello").unwrap(), Lang::Zh);
        assert_eq!(resolve_lang("en", "你好").unwrap(), Lang::En);
    }

    #[test]
    fn resolve_lang_rejects_unknown() {
        let err = resolve_lang("fr", "hello").unwrap_err();
        assert!(err.to_string().contains("unknown language"), "{err}");
    }
}

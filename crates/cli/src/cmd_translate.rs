//! `pardon translate [TEXT…|--stdin]`：词/句翻译 + JSONL 流式契约（spec §5.3）。
//!
//! 引擎选择：`--engine auto`（默认）→ pipeline 默认链（词典路由 + 引擎兜底）；
//! `llm|youdao|bing` → [`Pipeline::chain_with`] 单引擎链（无兜底，失败即错，
//! 也不做词典路由）。退出码裁决：全链失败（`translation` 与 `engine` 皆空）
//! 或显式引擎失败 → stderr Error JSONL（code "engine"）+ exit 2；超时 →
//! code "timeout" + exit 124；词路由命中但无释义（译文空、引擎非空）不是
//! 错误 → 照常输出 exit 0。

use crate::output::{self, StreamEvent};
use crate::TranslateArgs;
use pardon_core::engine::{Chain, EngineError, TranslateRequest};
use pardon_core::lang::{self, Lang};
use pardon_core::pipeline::{Pipeline, Translation};
use std::io::{Read, Write};
use std::time::Duration;

/// 翻译并输出。返回进程退出码：0 成功 / 2 引擎或参数错误 / 124 超时。
pub async fn run(args: &TranslateArgs) -> anyhow::Result<i32> {
    let text = collect_text(args)?;
    let (source, target) = resolve_langs(&args.source, &args.target, &text)?;
    validate_engine(&args.engine)?;

    let cfg = pardon_core::config::load()?;
    let pipeline = Pipeline::from_config(&cfg)?;
    // 显式引擎 → 单引擎链（不降级）；auto → None，走 pipeline 默认链
    let single = if args.engine == "auto" {
        None
    } else {
        Some(pipeline.chain_with(&args.engine)?)
    };
    let dur = Duration::from_secs(args.timeout);

    if args.stream {
        run_stream(args, &pipeline, single.as_ref(), &text, source, target, dur).await
    } else {
        run_plain(args, &pipeline, single.as_ref(), &text, source, target, dur).await
    }
}

/// `--stream`：meta → delta* → result，stdout 逐行 flush；Meta.engine 为
/// 旗标值（auto/llm/youdao/bing），实际引擎以 Result 事件为准。
async fn run_stream(
    args: &TranslateArgs,
    pipeline: &Pipeline,
    single: Option<&Chain>,
    text: &str,
    source: Lang,
    target: Lang,
    dur: Duration,
) -> anyhow::Result<i32> {
    emit(&StreamEvent::Meta {
        source: output::lang_str(source).into(),
        target: output::lang_str(target).into(),
        engine: args.engine.clone(),
    })?;
    let outcome = match single {
        // 单引擎链无流式回调：成功后把完整译文作单次 delta（统一 meta →
        // delta → result 节奏，与 pipeline 兜底行为一致）
        Some(chain) => {
            tokio::time::timeout(dur, translate_via_chain(chain, to_request(text, source, target)))
                .await
        }
        None => {
            tokio::time::timeout(dur, async {
                pipeline
                    .translate_stream(text, |d| {
                        let _ = emit(&StreamEvent::Delta { text: d.to_string() });
                    })
                    .await
            })
            .await
        }
    };
    let t = exit_on_outcome(outcome, args.timeout);
    // 终局裁决先行：全链失败时不发 Result 事件（stdout 保持 meta/delta 序列，
    // 错误只走 stderr）
    finish(&t)?;
    if single.is_some() {
        emit(&StreamEvent::Delta { text: t.translation.clone() })?;
    }
    emit(&result_event(&t))?;
    Ok(0)
}

/// 非 `--stream`：单个 Result JSON（`--json` 紧凑单行，否则 pretty）。
async fn run_plain(
    args: &TranslateArgs,
    pipeline: &Pipeline,
    single: Option<&Chain>,
    text: &str,
    source: Lang,
    target: Lang,
    dur: Duration,
) -> anyhow::Result<i32> {
    let outcome = match single {
        Some(chain) => {
            tokio::time::timeout(dur, translate_via_chain(chain, to_request(text, source, target)))
                .await
        }
        // pipeline.translate 非 Result：全链失败以「译文与引擎皆空」表达
        None => tokio::time::timeout(dur, async { Ok(pipeline.translate(text).await) }).await,
    };
    let t = exit_on_outcome(outcome, args.timeout);
    // 同 stream：先裁决再输出，全链失败时 stdout 不留空 Result
    finish(&t)?;
    let ev = result_event(&t);
    let out = if args.json {
        serde_json::to_string(&ev)?
    } else {
        serde_json::to_string_pretty(&ev)?
    };
    println!("{out}");
    Ok(0)
}

/// Elapsed → timeout/exit 124；EngineError → engine/exit 2；Ok 原样返回。
fn exit_on_outcome(
    outcome: Result<Result<Translation, EngineError>, tokio::time::error::Elapsed>,
    timeout_secs: u64,
) -> Translation {
    match outcome {
        Err(_) => output::event_exit(
            "timeout",
            format!("translate timed out after {timeout_secs}s"),
            124,
        ),
        Ok(Err(e)) => output::event_exit("engine", e.to_string(), 2),
        Ok(Ok(t)) => t,
    }
}

/// 退出码裁决：只有全链失败（译文与引擎皆空）才是错误（stderr Error
/// JSONL + exit 2）；词路由命中但无释义（engine 非空）不是，放行。
fn finish(t: &Translation) -> anyhow::Result<()> {
    if t.translation.is_empty() && t.engine.is_empty() {
        output::event_exit("engine", "all translation engines failed".into(), 2);
    }
    Ok(())
}

/// 单引擎链翻译 → Translation；无兜底，Err 直接上抛。
async fn translate_via_chain(
    chain: &Chain,
    req: TranslateRequest,
) -> Result<Translation, EngineError> {
    let (translation, engine) = chain.translate(&req).await?;
    Ok(Translation {
        source_lang: req.from,
        target_lang: req.to,
        text: req.text,
        translation,
        engine: engine.to_string(),
    })
}

fn to_request(text: &str, from: Lang, to: Lang) -> TranslateRequest {
    TranslateRequest { text: text.to_string(), from, to }
}

/// Result 事件：字段名对齐 core `Translation`，语言为小写码。
fn result_event(t: &Translation) -> StreamEvent {
    StreamEvent::Result {
        source: output::lang_str(t.source_lang).into(),
        target: output::lang_str(t.target_lang).into(),
        engine: t.engine.clone(),
        text: t.text.clone(),
        translation: t.translation.clone(),
    }
}

/// 单行 JSONL 到 stdout 并立即 flush（流式契约：消费方可逐行读）。
fn emit(ev: &StreamEvent) -> anyhow::Result<()> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{}", serde_json::to_string(ev)?)?;
    out.flush()?;
    Ok(())
}

/// 文本来源：argv 参数以空格连接优先，其次 `--stdin` 读全部；两者皆空 →
/// 错误（exit 2）。空白-only 视为空。
fn collect_text(args: &TranslateArgs) -> anyhow::Result<String> {
    let text = if !args.text.is_empty() {
        args.text.join(" ")
    } else if args.stdin {
        let mut buf = String::new();
        std::io::stdin().lock().read_to_string(&mut buf)?;
        buf
    } else {
        anyhow::bail!("no text given: pass TEXT arguments or --stdin");
    };
    if text.trim().is_empty() {
        anyhow::bail!("empty text: nothing to translate");
    }
    Ok(text)
}

/// 方向解析：auto → `lang::direction(text)`；显式 en/zh → 覆盖（其他值报错）。
fn resolve_langs(source: &str, target: &str, text: &str) -> anyhow::Result<(Lang, Lang)> {
    let (auto_from, auto_to) = lang::direction(text);
    let from = if source == "auto" { auto_from } else { parse_lang(source)? };
    let to = if target == "auto" { auto_to } else { parse_lang(target)? };
    Ok((from, to))
}

fn parse_lang(s: &str) -> anyhow::Result<Lang> {
    match s {
        "en" => Ok(Lang::En),
        "zh" => Ok(Lang::Zh),
        other => anyhow::bail!("unknown language {other:?}, expected \"auto\", \"en\" or \"zh\""),
    }
}

fn validate_engine(engine: &str) -> anyhow::Result<()> {
    match engine {
        "auto" | "llm" | "youdao" | "bing" => Ok(()),
        other => anyhow::bail!("unknown engine {other:?}, expected one of auto/llm/youdao/bing"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: Vec<String>) -> TranslateArgs {
        TranslateArgs {
            text,
            stdin: false,
            source: "auto".into(),
            target: "auto".into(),
            engine: "auto".into(),
            json: false,
            stream: false,
            timeout: 30,
        }
    }

    #[test]
    fn collect_text_joins_args_by_space() {
        let got = collect_text(&args(vec!["hello".into(), "world".into()])).unwrap();
        assert_eq!(got, "hello world");
    }

    #[test]
    fn collect_text_rejects_missing_and_blank() {
        let err = collect_text(&args(vec![])).unwrap_err();
        assert!(err.to_string().contains("no text given"), "{err}");
        let err = collect_text(&args(vec!["   ".into()])).unwrap_err();
        assert!(err.to_string().contains("empty text"), "{err}");
    }

    #[test]
    fn resolve_langs_auto_follows_detection() {
        assert_eq!(
            resolve_langs("auto", "auto", "hello").unwrap(),
            (Lang::En, Lang::Zh)
        );
        assert_eq!(
            resolve_langs("auto", "auto", "你好").unwrap(),
            (Lang::Zh, Lang::En)
        );
    }

    #[test]
    fn resolve_langs_explicit_overrides_and_rejects_unknown() {
        assert_eq!(resolve_langs("zh", "en", "hello").unwrap(), (Lang::Zh, Lang::En));
        assert!(resolve_langs("fr", "auto", "hello").is_err());
        assert!(resolve_langs("auto", "ja", "hello").is_err());
    }

    #[test]
    fn validate_engine_accepts_known_rejects_other() {
        for ok in ["auto", "llm", "youdao", "bing"] {
            validate_engine(ok).unwrap();
        }
        assert!(validate_engine("google").is_err());
    }

    #[test]
    fn lang_str_is_lowercase_code() {
        assert_eq!(output::lang_str(Lang::En), "en");
        assert_eq!(output::lang_str(Lang::Zh), "zh");
    }
}

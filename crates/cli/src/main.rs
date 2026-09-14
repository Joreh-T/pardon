//! pardon CLI 入口：clap 子命令分发。
//!
//! 统一错误出口 [`output::error_exit`]：任何内部错误 → stderr 单行
//! `{"type":"error","code":"internal","message":"…"}` + exit 2。
//! `translate` 的超时/引擎错误走 [`output::event_exit`]（code
//! timeout/engine，exit 124/2）。

mod cmd_daemon;
mod cmd_history;
mod cmd_lookup;
mod cmd_speak;
mod cmd_status;
mod cmd_translate;
mod cmd_trigger;
mod output;

use clap::{Args, Parser, Subcommand};

/// `pardon translate` 参数：文本来自 argv（空格连接）或 `--stdin`。
#[derive(Args)]
struct TranslateArgs {
    /// Text to translate; multiple arguments joined by spaces
    text: Vec<String>,
    /// Read the full text from standard input (used when no TEXT given)
    #[arg(long)]
    stdin: bool,
    /// Source language: auto | en | zh (auto = detect from text) (only effective with an explicit --engine; with --engine auto the direction is auto-detected)
    #[arg(long, default_value = "auto")]
    source: String,
    /// Target language: auto | en | zh (auto = detected from text) (only effective with an explicit --engine; with --engine auto the direction is auto-detected)
    #[arg(long, default_value = "auto")]
    target: String,
    /// Engine: auto | llm | youdao | bing (single engine gets no fallback)
    #[arg(long, default_value = "auto")]
    engine: String,
    /// Compact single-line JSON (non-stream output)
    #[arg(long)]
    json: bool,
    /// Stream JSONL events: meta, then deltas, then result
    #[arg(long)]
    stream: bool,
    /// Whole-operation timeout in seconds
    #[arg(long, default_value = "30")]
    timeout: u64,
}

#[derive(Parser)]
#[command(name = "pardon", version, about = "Pardon my French — a translator")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Offline dictionary lookup (word card)
    Lookup {
        word: String,
        /// compact single-line JSON
        #[arg(long)]
        json: bool,
    },
    /// Translate text (word or sentence)
    Translate(TranslateArgs),
    /// Show translation history
    History(cmd_history::HistoryArgs),
    /// Speak text aloud (TTS; no stdout output on success)
    Speak {
        /// Text to speak
        text: String,
        /// Language: auto | en | zh (auto = detect from text)
        #[arg(long, default_value = "auto")]
        lang: String,
    },
    /// Show config path, or write the default config (--init)
    Config {
        /// Write the default config to the config path (refuses if it exists)
        #[arg(long)]
        init: bool,
    },
    /// Trigger a translation via the daemon (for window-manager keybindings)
    Trigger {
        /// What to translate: selection (primary selection) or clipboard
        #[arg(default_value = "selection")]
        source: String,
    },
    /// Show pardond status (running, counters)
    Status,
    /// Manage the pardond daemon (start | stop)
    Daemon {
        /// start | stop
        action: String,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let code = match cli.cmd {
        Cmd::Lookup { word, json } => cmd_lookup::run(&word, json),
        Cmd::Translate(args) => cmd_translate::run(&args).await,
        Cmd::History(args) => cmd_history::run(&args).await,
        Cmd::Speak { text, lang } => cmd_speak::run(&text, &lang).await,
        Cmd::Config { init } => run_config(init),
        Cmd::Trigger { source } => run_trigger(&source).await,
        Cmd::Status => run_status().await,
        Cmd::Daemon { action } => run_daemon(&action).await,
    }
    .unwrap_or_else(|e| output::error_exit(e));
    std::process::exit(code);
}

/// `pardon config [--init]`：`--init` 把默认配置写到生效路径
/// （[`pardon_core::config::config_path`]，`PARDON_CONFIG` 可覆盖）；文件已
/// 存在 → 报错 exit 2（消息含路径），不覆盖。成功与裸 `config` 都打印
/// 路径、exit 0。
fn run_config(init: bool) -> anyhow::Result<i32> {
    let path = pardon_core::config::config_path();
    if init && path.exists() {
        anyhow::bail!(
            "config already exists: {} (edit it in place, or delete it first)",
            path.display()
        );
    }
    if init {
        pardon_core::config::write_default(&path)?;
    }
    println!("{}", path.display());
    Ok(0)
}

/// `pardon trigger <source>`：读配置定位 pardond，再通知它翻译
/// selection / clipboard。
async fn run_trigger(source: &str) -> anyhow::Result<i32> {
    let cfg = pardon_core::config::load()?;
    let addr = pardon_core::config::effective_http_bind(&cfg)?;
    cmd_trigger::run(source, addr).await
}

/// `pardon status`：读配置定位 pardond，查询其运行状态。
async fn run_status() -> anyhow::Result<i32> {
    let cfg = pardon_core::config::load()?;
    let addr = pardon_core::config::effective_http_bind(&cfg)?;
    cmd_status::run(addr).await
}

/// `pardon daemon <action>`：读配置定位 pardond，执行 start | stop。
async fn run_daemon(action: &str) -> anyhow::Result<i32> {
    let cfg = pardon_core::config::load()?;
    let addr = pardon_core::config::effective_http_bind(&cfg)?;
    cmd_daemon::run(action, addr).await
}

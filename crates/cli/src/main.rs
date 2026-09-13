//! pardon CLI 入口：clap 子命令分发。
//!
//! 统一错误出口 [`output::error_exit`]：任何内部错误 → stderr 单行
//! `{"type":"error","code":"internal","message":"…"}` + exit 2。
//! `translate` 的超时/引擎错误走 [`output::event_exit`]（code
//! timeout/engine，exit 124/2）。

mod cmd_lookup;
mod cmd_translate;
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
    /// Speak text aloud (TTS)
    Speak { text: String },
    /// Show or write default config
    Config { #[arg(long)] init: bool },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let code = match cli.cmd {
        Cmd::Lookup { word, json } => cmd_lookup::run(&word, json),
        Cmd::Translate(args) => cmd_translate::run(&args).await,
        // 未实现子命令：走统一错误出口（stderr JSON + exit 2）
        Cmd::Speak { .. } => Err(anyhow::anyhow!("todo: `pardon speak` is not implemented yet")),
        Cmd::Config { .. } => Err(anyhow::anyhow!("todo: `pardon config` is not implemented yet")),
    }
    .unwrap_or_else(|e| output::error_exit(e));
    std::process::exit(code);
}

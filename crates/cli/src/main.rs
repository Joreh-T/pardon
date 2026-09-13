//! pardon CLI 入口：clap 子命令分发。
//!
//! 统一错误出口 [`output::error_exit`]：任何内部错误 → stderr 单行
//! `{"type":"error","code":"internal","message":"…"}` + exit 2。

mod cmd_lookup;
mod output;

use clap::{Args, Parser, Subcommand};

// `pardon translate` 参数：Task 19 填充，本任务为空占位（doc 注释会泄入
// `--help` 的 about 文案，故用普通注释）。
#[derive(Args)]
struct TranslateArgs {}

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
        // 未实现子命令：走统一错误出口（stderr JSON + exit 2）
        Cmd::Translate(_) => Err(anyhow::anyhow!("todo: `pardon translate` is not implemented yet")),
        Cmd::Speak { .. } => Err(anyhow::anyhow!("todo: `pardon speak` is not implemented yet")),
        Cmd::Config { .. } => Err(anyhow::anyhow!("todo: `pardon config` is not implemented yet")),
    }
    .unwrap_or_else(|e| output::error_exit(e));
    std::process::exit(code);
}

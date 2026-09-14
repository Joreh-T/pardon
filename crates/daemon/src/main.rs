//! pardond 入口（逻辑在 pardon_daemon::run，便于集成测试复用 lib）。

use clap::Parser;

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = pardon_daemon::Args::parse();
    if let Err(e) = pardon_daemon::run(args).await {
        eprintln!("pardond: {e:#}");
        std::process::exit(1);
    }
}

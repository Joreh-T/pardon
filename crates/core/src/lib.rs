pub mod config;
pub mod dict;
pub mod engine;
pub mod history;
pub mod ipc;
pub mod lang;
pub mod loopguard;
pub mod pipeline;
pub mod router;
pub mod tts;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

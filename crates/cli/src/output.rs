//! CLI 输出契约：错误的统一呈现。

use anyhow::Error;
use serde::Serialize;

/// stderr 错误 JSON 的字段序（serde 声明序）即对外契约：
/// `{"type":"error","code":"internal","message":"…"}`。
#[derive(Serialize)]
struct ErrorJson<'a> {
    r#type: &'a str,
    code: &'a str,
    message: String,
}

/// 内部错误出口：stderr 单行 JSON，退出码 2。序列化保证 message 转义
/// （含换行/引号）后仍为单行。
pub fn error_exit(e: Error) -> ! {
    let err = ErrorJson {
        r#type: "error",
        code: "internal",
        message: e.to_string(),
    };
    eprintln!("{}", serde_json::to_string(&err).unwrap_or_else(|_| {
        r#"{"type":"error","code":"internal","message":"unserializable error"}"#.into()
    }));
    std::process::exit(2)
}

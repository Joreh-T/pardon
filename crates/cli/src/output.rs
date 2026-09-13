//! CLI 输出契约：错误的统一呈现 + `translate` 的 JSONL 流事件（spec §5.3）。

use anyhow::Error;
use pardon_core::lang::Lang;
use serde::Serialize;

/// `pardon translate --stream` 的 stdout JSONL 事件 + stderr 错误体，
/// 内部标签 `type` 小写、字段按声明序输出：
///
/// ```json
/// {"type":"meta","source":"en","target":"zh","engine":"auto"}
/// {"type":"delta","text":"…"}
/// {"type":"result","source":"en","target":"zh","engine":"youdao","text":"…","translation":"…"}
/// {"type":"error","code":"engine","message":"…"}
/// ```
///
/// stdout 只出现 Meta/Delta/Result（保持逐行可解析）；Error 仅经 stderr
/// 呈现（code：internal / engine / timeout）。
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum StreamEvent {
    /// 首行：解析出的方向与用户选择的引擎（"auto" = 链式，实际引擎见 Result）。
    Meta {
        source: String,
        target: String,
        engine: String,
    },
    /// 译文增量。已发增量不回撤（引擎中途失败时兜底全文另作 delta）；
    /// 终态以 Result 事件为准。
    Delta {
        text: String,
    },
    /// 终行：字段名对齐 core `Translation`（source/target 为小写语言码）。
    Result {
        source: String,
        target: String,
        engine: String,
        text: String,
        translation: String,
    },
    /// 仅 stderr：机器可读错误（code ∈ internal / engine / timeout）。
    Error {
        code: String,
        message: String,
    },
}

/// 内部错误出口：stderr 单行 error JSON，退出码 2。序列化保证 message 转义
/// （含换行/引号）后仍为单行。
pub fn error_exit(e: Error) -> ! {
    event_exit("internal", e.to_string(), 2)
}

/// stderr 单行 Error JSONL + 指定退出码（`translate` 的 engine/timeout
/// 契约出口：exit 2 / 124）。序列化理论上不会失败（纯字符串字段），
/// 兜底串保契约形状。
pub fn event_exit(code: &str, message: String, exit_code: i32) -> ! {
    let err = StreamEvent::Error { code: code.to_string(), message };
    eprintln!(
        "{}",
        serde_json::to_string(&err).unwrap_or_else(|_| {
            format!(r#"{{"type":"error","code":"{code}","message":"unserializable error"}}"#)
        })
    );
    std::process::exit(exit_code)
}

/// Lang → JSONL 契约的小写语言码（与 core `Lang` 的 serde 表示一致）。
pub fn lang_str(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "en",
        Lang::Zh => "zh",
    }
}

//! daemon → GUI 订阅事件（UDS 上按行转发，见 uds.rs）。

use pardon_core::dict::WordCard;
use pardon_core::pipeline::Translation;
use serde::Serialize;

/// Popup 携带完整词卡（数百字节）远大于 ShowWindow——事件构造后立即
/// 序列化为 JSON 行丢弃，不长期持有也不批量存储，无需装箱压缩。
#[allow(clippy::large_enum_variant)]
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "event", content = "params", rename_all = "snake_case")]
pub enum Event {
    /// 翻译结果弹窗（card 为词典命中/词形建议卡；句子翻译为 None）。
    Popup {
        translation: Translation,
        card: Option<WordCard>,
    },
    /// 托盘请求打开窗口。
    ShowWindow { kind: ShowWindowKind },
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ShowWindowKind {
    Main,
    Settings,
}

impl Event {
    /// 序列化为单行 JSON（UDS JSONL 帧协议）。
    pub fn to_line(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pardon_core::pipeline::Translation;

    fn tr(text: &str) -> Translation {
        Translation {
            source_lang: pardon_core::lang::Lang::En,
            target_lang: pardon_core::lang::Lang::Zh,
            text: text.into(),
            translation: "译".into(),
            engine: "glm".into(),
        }
    }

    #[test]
    fn popup_serializes_as_tagged_event_line() {
        let line = Event::Popup {
            translation: tr("hello"),
            card: None,
        }
        .to_line()
        .unwrap();
        assert!(
            line.starts_with(r#"{"event":"popup","params":{"translation":"#),
            "{line}"
        );
        assert!(line.contains(r#""engine":"glm""#), "{line}");
        assert!(line.contains(r#""card":null"#), "{line}");
    }

    #[test]
    fn show_window_serializes_snake_case_kind() {
        let line = Event::ShowWindow {
            kind: ShowWindowKind::Settings,
        }
        .to_line()
        .unwrap();
        assert!(line.contains(r#""event":"show_window""#), "{line}");
        assert!(line.contains(r#""kind":"settings""#), "{line}");
    }
}

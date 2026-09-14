use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    En,
    Zh,
}

/// CJK 统一表意文字 + 扩展A + CJK 标点
pub fn is_cjk_char(c: char) -> bool {
    matches!(c as u32,
        0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0x3000..=0x303F | 0xFF00..=0xFFEF)
}

pub fn detect(text: &str) -> Lang {
    if text.chars().any(is_cjk_char) {
        Lang::Zh
    } else {
        Lang::En
    }
}

pub fn direction(text: &str) -> (Lang, Lang) {
    if detect(text) == Lang::Zh {
        (Lang::Zh, Lang::En)
    } else {
        (Lang::En, Lang::Zh)
    }
}

pub fn display(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "English",
        Lang::Zh => "Chinese (Simplified)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Lang::{En, Zh};

    #[test]
    fn detect_pure() {
        assert_eq!(detect("hello world"), En);
        assert_eq!(detect("run"), En);
        assert_eq!(detect("你好"), Zh);
        assert_eq!(detect("这是一个句子。"), Zh);
    }

    #[test]
    fn detect_mixed_counts_as_zh() {
        assert_eq!(detect("这个 API 设计很好"), Zh);
        assert_eq!(detect("start 开始"), Zh);
    }

    #[test]
    fn detect_cjk_punctuation_only_is_zh() {
        assert_eq!(detect("你好，世界！"), Zh);
    }

    #[test]
    fn detect_empty_defaults_en() {
        assert_eq!(detect(""), En);
    }

    #[test]
    fn direction_is_reverse_pair() {
        assert_eq!(direction("hello"), (En, Zh));
        assert_eq!(direction("你好"), (Zh, En));
    }

    #[test]
    fn display_names() {
        assert_eq!(display(En), "English");
        assert_eq!(display(Zh), "Chinese (Simplified)");
    }
}

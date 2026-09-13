pub mod ecdict;
pub mod ecdict_query;

use serde::{Deserialize, Serialize};

/// ECDICT translation 字段中出现的词性缩写（行首前缀）。
/// 注意 ECDICT 的 pos 数据列不可靠（spec §3.2），词性一律从此处解析。
const POS_ABBRS: &[&str] = &[
    "n.", "v.", "vt.", "vi.", "adj.", "adv.", "art.", "aux.v.", "aux.", "conj.", "prep.",
    "pron.", "int.", "interj.", "num.", "abbr.",
];

/// 单个词条（CLI JSON 契约的顶层结构）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct WordCard {
    pub found: bool,
    pub word: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phonetic: Option<Phonetic>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pos: Vec<PosGloss>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub definition: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exchange: Option<Exchange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collins: Option<u8>,
    pub oxford: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    pub source: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<String>,
}

/// 音标；M1 只有 uk（ECDICT）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Phonetic {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uk: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub us: Option<String>,
}

/// 一个词性下的释义组；`pos` 为空串表示无前缀行。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PosGloss {
    pub pos: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gloss: Vec<String>,
}

/// ECDICT exchange 词形变化。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Exchange {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub past: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ing: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub third: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comparative: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superlative: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plural: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lemma: Option<String>,
}

/// 词典后端抽象（ECDICT SQLite / CEDICT / 后续引擎均实现此 trait）。
pub trait DictProvider {
    fn lookup(&self, word: &str) -> Option<WordCard>;
    fn suggest(&self, word: &str) -> Vec<String>;
}

/// 把 ECDICT 的 translation 字段按词性前缀分组成 [`PosGloss`] 列表。
pub fn parse_translation_lines(raw: &str) -> Vec<PosGloss> {
    let mut out: Vec<PosGloss> = Vec::new();
    for line in raw.split('\n') {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (pos, rest) = match POS_ABBRS.iter().find(|a| line.starts_with(*a)) {
            Some(a) => (*a, line[a.len()..].trim_start()),
            None => ("", line),
        };
        for part in rest.split(['；', ';']) {
            let g = part.trim();
            if g.is_empty() {
                continue;
            }
            match out.iter_mut().find(|pg| pg.pos == pos) {
                Some(pg) => pg.gloss.push(g.to_string()),
                None => out.push(PosGloss { pos: pos.to_string(), gloss: vec![g.to_string()] }),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_with_pos_prefixes() {
        let out = parse_translation_lines("n. 火药\nv. 使爆炸；配制\nvt. 感知");
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], PosGloss { pos: "n.".into(), gloss: vec!["火药".into()] });
        assert_eq!(out[1], PosGloss { pos: "v.".into(), gloss: vec!["使爆炸".into(), "配制".into()] });
        assert_eq!(out[2], PosGloss { pos: "vt.".into(), gloss: vec!["感知".into()] });
    }

    #[test]
    fn parse_lines_without_prefix_group_into_empty_pos() {
        let out = parse_translation_lines("你好\n感叹词");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], PosGloss { pos: "".into(), gloss: vec!["你好".into(), "感叹词".into()] });
    }

    #[test]
    fn parse_mixed() {
        let out = parse_translation_lines("adj. 高的\n高级的");
        assert_eq!(out.len(), 2);
        assert_eq!(out[1], PosGloss { pos: "".into(), gloss: vec!["高级的".into()] });
    }

    #[test]
    fn parse_empty_and_whitespace() {
        assert!(parse_translation_lines("").is_empty());
        assert!(parse_translation_lines("  \n \n").is_empty());
    }

    #[test]
    fn parse_splits_on_semicolon_and_fullwidth() {
        let out = parse_translation_lines("n. 火药；炸药");
        assert_eq!(out[0].gloss, vec!["火药", "炸药"]);
    }
}

use super::{Exchange, Phonetic, WordCard, parse_translation_lines};

/// 解析 ECDICT exchange 字段，如 "d:perceived/p:perceived/3:perceives/i:perceiving" → [`Exchange`]。
pub fn parse_exchange(raw: &str) -> Option<Exchange> {
    if raw.trim().is_empty() {
        return None;
    }
    let mut ex = Exchange {
        past: None,
        pp: None,
        ing: None,
        third: None,
        comparative: None,
        superlative: None,
        plural: None,
        lemma: None,
    };
    let mut any = false;
    for pair in raw.split('/') {
        let Some((k, v)) = pair.split_once(':') else { continue };
        let v = v.trim().to_string();
        if v.is_empty() {
            continue;
        }
        any = true;
        match k.trim() {
            "p" => ex.past = Some(v),
            "d" => ex.pp = Some(v),
            "i" => ex.ing = Some(v),
            "3" => ex.third = Some(v),
            "r" => ex.comparative = Some(v),
            "t" => ex.superlative = Some(v),
            "s" => ex.plural = Some(v),
            "0" | "1" => ex.lemma = Some(v),
            _ => {}
        }
    }
    if any { Some(ex) } else { None }
}

/// ECDICT CSV 列（13 列，无表头，顺序见 spec §3.2）→ [`WordCard`]（found=true）。
///
/// 列序：word, phonetic, definition, translation, pos, collins, oxford, tag, bnc, frq, exchange, detail, audio。
/// pos/detail/audio 列忽略（pos 不可靠；detail/audio 为空）。
pub fn row_to_card(row: &[&str]) -> Option<WordCard> {
    if row.len() < 11 {
        return None;
    }
    Some(WordCard {
        found: true,
        word: row[0].to_string(),
        phonetic: if row[1].is_empty() {
            None
        } else {
            Some(Phonetic { uk: Some(row[1].to_string()), us: None })
        },
        pos: parse_translation_lines(row[3]),
        definition: row[2]
            .split('\n')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
        exchange: parse_exchange(row[10]),
        collins: row[5].parse().ok(),
        oxford: row[6] == "1",
        tags: row[7]
            .split_whitespace()
            .map(String::from)
            .filter(|t| !t.is_empty())
            .collect(),
        source: "ecdict".into(),
        suggestions: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exchange_full() {
        let ex = parse_exchange("d:perceived/p:perceived/3:perceives/i:perceiving/0:perceive").unwrap();
        assert_eq!(ex.pp.as_deref(), Some("perceived"));
        assert_eq!(ex.past.as_deref(), Some("perceived"));
        assert_eq!(ex.third.as_deref(), Some("perceives"));
        assert_eq!(ex.ing.as_deref(), Some("perceiving"));
        assert_eq!(ex.lemma.as_deref(), Some("perceive"));
        assert!(ex.comparative.is_none() && ex.plural.is_none());
    }

    #[test]
    fn exchange_empty_is_none() {
        assert!(parse_exchange("").is_none());
    }

    #[test]
    fn exchange_plural_and_comparative() {
        let ex = parse_exchange("s:boxes/0:box").unwrap();
        assert_eq!(ex.plural.as_deref(), Some("boxes"));
        let ex = parse_exchange("r:bigger/t:biggest/0:big").unwrap();
        assert_eq!(ex.comparative.as_deref(), Some("bigger"));
        assert_eq!(ex.superlative.as_deref(), Some("biggest"));
    }

    // 13 列 fixture：word,phonetic,definition,translation,pos,collins,oxford,tag,bnc,frq,exchange,detail,audio
    // 注意：与 Task 5 的 ecdict_mini.csv 中 run 行完全一致（权威数据源）
    fn run_row() -> Vec<&'static str> {
        vec!["run","rʌn","move fast\noperate","n. 跑步\nv. 跑；运转","n:46/v:54","3","1","zk gk cet4","1234","567","p:ran/d:run/i:running/3:runs/0:run","",""]
    }

    #[test]
    fn row_to_card_assembles() {
        let card = row_to_card(&run_row()).unwrap();
        assert!(card.found);
        assert_eq!(card.word, "run");
        assert_eq!(card.phonetic.as_ref().unwrap().uk.as_deref(), Some("rʌn"));
        assert_eq!(card.pos.len(), 2);
        assert_eq!(card.pos[0].pos, "n.");
        assert_eq!(card.pos[1].gloss, vec!["跑", "运转"]);
        assert_eq!(card.definition, vec!["move fast", "operate"]);
        assert_eq!(card.exchange.as_ref().unwrap().past.as_deref(), Some("ran"));
        assert_eq!(card.collins, Some(3));
        assert!(card.oxford);
        assert_eq!(card.tags, vec!["zk", "gk", "cet4"]);
        assert_eq!(card.source, "ecdict");
        assert!(card.suggestions.is_empty());
    }

    #[test]
    fn row_to_card_minimal() {
        let card = row_to_card(&["hello","həˈləu","","int. 你好","","","","","","","","",""]).unwrap();
        assert_eq!(card.pos[0].gloss, vec!["你好"]);
        assert_eq!(card.definition.len(), 0);
        assert!(card.exchange.is_none());
    }

    #[test]
    fn row_with_too_few_columns_is_none() {
        assert!(row_to_card(&["only","two"]).is_none());
    }
}

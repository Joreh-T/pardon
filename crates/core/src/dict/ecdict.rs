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

pub mod import {
    use super::row_to_card;
    use rusqlite::Connection;
    use std::io::Read;

    pub fn create_schema(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS entries (
                word TEXT PRIMARY KEY,
                phonetic TEXT, definition TEXT, translation TEXT,
                pos_json TEXT, exchange_json TEXT,
                collins INTEGER, oxford INTEGER, tags TEXT, bnc INTEGER, frq INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_entries_word ON entries(word COLLATE NOCASE);
            CREATE TABLE IF NOT EXISTS wordforms (form TEXT PRIMARY KEY, lemma TEXT NOT NULL);",
        )
    }

    /// 返回本次新插入的词条数（已存在的词条跳过）。
    pub fn import_csv(reader: impl Read, conn: &Connection) -> anyhow::Result<u64> {
        use anyhow::Context;
        create_schema(conn).context("create schema")?;
        let mut rdr = csv::ReaderBuilder::new()
            .has_headers(false)
            .flexible(true)
            .from_reader(reader);
        conn.execute_batch("BEGIN")?;
        let mut inserted = 0u64;
        let result = (|| -> anyhow::Result<()> {
            for rec in rdr.records() {
                let rec = rec.context("read csv record")?;
                let row: Vec<&str> = rec.iter().collect();
                inserted += insert_row(conn, &row)?;
            }
            Ok(())
        })();
        conn.execute_batch("COMMIT")?;
        result?;
        Ok(inserted)
    }

    /// 从官方 ECDICT SQLite 发布包导入（表 `stardict`，含 id/sw 附加列，
    /// 其余 13 列与 CSV 列序一致；collins/oxford/bnc/frq 为 INTEGER，NULL 视为空）。
    pub fn import_sqlite(src: &std::path::Path, conn: &Connection) -> anyhow::Result<u64> {
        use anyhow::Context;
        create_schema(conn).context("create schema")?;
        let src_conn = Connection::open_with_flags(
            src, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ).with_context(|| format!("open official ecdict sqlite {}", src.display()))?;
        let mut stmt = src_conn.prepare(
            "SELECT word, phonetic, definition, translation, pos, collins, oxford, tag, \
             bnc, frq, exchange, detail, audio FROM stardict",
        ).context("select from stardict (官方库缺少 stardict 表？)")?;
        let mut rows = stmt.query([])?;
        conn.execute_batch("BEGIN")?;
        let mut inserted = 0u64;
        let result = (|| -> anyhow::Result<()> {
            while let Some(r) = rows.next()? {
                // 全部列转字符串（INTEGER 列 NULL → ""），复用 CSV 路径的行处理
                let row: Vec<String> = (0..13)
                    .map(|i| {
                        r.get_ref(i).map(|v| match v {
                            rusqlite::types::ValueRef::Null => String::new(),
                            rusqlite::types::ValueRef::Integer(n) => n.to_string(),
                            rusqlite::types::ValueRef::Text(t) => {
                                String::from_utf8_lossy(t).into_owned()
                            }
                            _ => String::new(),
                        })
                    })
                    .collect::<Result<_, _>>()?;
                let refs: Vec<&str> = row.iter().map(String::as_str).collect();
                inserted += insert_row(conn, &refs)?;
            }
            Ok(())
        })();
        conn.execute_batch("COMMIT")?;
        result?;
        Ok(inserted)
    }

    /// 插入单条 13 列行（返回 0=已存在跳过，1=新插入）；负责 entries + wordforms。
    fn insert_row(conn: &Connection, row: &[&str]) -> anyhow::Result<u64> {
        let Some(card) = row_to_card(row) else { return Ok(0) };
        let pos_json = serde_json::to_string(&card.pos).unwrap();
        let ex_json = card
            .exchange
            .as_ref()
            .map(|e| serde_json::to_string(e).unwrap());
        let changed = conn.execute(
            "INSERT OR IGNORE INTO entries VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            rusqlite::params![
                card.word,
                row.get(1).copied().unwrap_or(""),
                row.get(2).copied().unwrap_or(""),
                row.get(3).copied().unwrap_or(""),
                pos_json,
                ex_json,
                card.collins,
                card.oxford as i64,
                card.tags.join(" "),
                row.get(8).and_then(|s| s.parse().ok()).unwrap_or(0i64),
                row.get(9).and_then(|s| s.parse().ok()).unwrap_or(0i64),
            ],
        )?;
        if changed == 1 {
            if let Some(ex) = &card.exchange {
                for form in [
                    ex.past.as_deref(),
                    ex.pp.as_deref(),
                    ex.ing.as_deref(),
                    ex.third.as_deref(),
                    ex.comparative.as_deref(),
                    ex.superlative.as_deref(),
                    ex.plural.as_deref(),
                ]
                .into_iter()
                .flatten()
                {
                    let _ = conn.execute(
                        "INSERT OR IGNORE INTO wordforms VALUES (?1, ?2)",
                        rusqlite::params![form, card.word],
                    );
                }
            }
            Ok(1)
        } else {
            Ok(0)
        }
    }
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

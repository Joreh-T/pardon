use crate::dict::{DictProvider, Exchange, Phonetic, PosGloss, WordCard};
use rusqlite::Connection;
use std::io::Read;
use std::path::Path;

pub struct EcdictDb { conn: Connection }

fn card_from_row(row: &rusqlite::Row) -> rusqlite::Result<WordCard> {
    let word: String = row.get(0)?;
    let phonetic: Option<String> = row.get(1)?;
    let definition: String = row.get(2).unwrap_or_default();
    // translation 列不再单独读取：词性/释义已在导入时解析进 pos_json
    let _translation: String = row.get(3).unwrap_or_default();
    let pos_json: String = row.get(4).unwrap_or_else(|_| "[]".into());
    let exchange_json: Option<String> = row.get(5)?;
    let collins: Option<i64> = row.get(6)?;
    let oxford: i64 = row.get(7)?;
    let tags: String = row.get(8).unwrap_or_default();
    Ok(WordCard {
        found: true,
        word,
        phonetic: phonetic.map(|p| Phonetic { uk: Some(p), us: None }),
        pos: serde_json::from_str::<Vec<PosGloss>>(&pos_json).unwrap_or_default(),
        definition: definition.split('\n').map(str::trim)
            .filter(|s| !s.is_empty()).map(String::from).collect(),
        exchange: exchange_json.and_then(|j| serde_json::from_str::<Exchange>(&j).ok()),
        collins: collins.map(|c| c as u8),
        oxford: oxford == 1,
        tags: tags.split_whitespace().map(String::from).collect(),
        source: "ecdict".into(),
        suggestions: Vec::new(),
    })
}

const CARD_COLS: &str = "word, phonetic, definition, translation, pos_json, exchange_json, collins, oxford, tags";

impl EcdictDb {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        Ok(Self { conn: Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)? })
    }
    pub fn from_reader(reader: impl Read) -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        super::ecdict::import::import_csv(reader, &conn)?;
        Ok(Self { conn })
    }
    fn query_word(&self, w: &str) -> Option<WordCard> {
        self.conn.query_row(
            &format!("SELECT {CARD_COLS} FROM entries WHERE word = ?1 COLLATE NOCASE"),
            [w], |r| card_from_row(r)).ok()
    }
}

impl DictProvider for EcdictDb {
    fn lookup(&self, word: &str) -> Option<WordCard> {
        self.query_word(word).or_else(|| {
            let lemma: Option<String> = self.conn.query_row(
                "SELECT lemma FROM wordforms WHERE form = ?1", [word.to_lowercase()],
                |r| r.get(0)).ok();
            lemma.and_then(|l| self.query_word(&l))
        })
    }

    fn suggest(&self, word: &str) -> Vec<String> {
        let prefix: String = word.chars().take(2).collect();
        let mut stmt = match self.conn.prepare(
            "SELECT word FROM entries WHERE word LIKE ?1 || '%' LIMIT 800") { Ok(s) => s, Err(_) => return vec![] };
        let cands: Vec<String> = stmt.query_map([prefix], |r| r.get::<_, String>(0))
            .map(|rows| rows.flatten().collect()).unwrap_or_default();
        let mut scored: Vec<(usize, String)> = cands.into_iter()
            .map(|c| (levenshtein(word, &c), c))
            .filter(|(d, _)| *d <= 2)
            .collect();
        scored.sort_by_key(|(d, _)| *d);
        scored.into_iter().take(5).map(|(_, c)| c).collect()
    }
}

pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1)
                .min(prev[j - 1] + usize::from(a[i - 1] != b[j - 1]));
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

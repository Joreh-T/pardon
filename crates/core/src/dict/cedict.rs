use crate::dict::{DictProvider, Phonetic, PosGloss, WordCard};
use rusqlite::Connection;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

/// 元音声调标记表：(基字符, [无声调, 一声, 二声, 三声, 四声])
const TONE_MARKS: &[(char, [char; 5])] = &[
    ('a', ['a', 'ā', 'á', 'ǎ', 'à']),
    ('e', ['e', 'ē', 'é', 'ě', 'è']),
    ('i', ['i', 'ī', 'í', 'ǐ', 'ì']),
    ('o', ['o', 'ō', 'ó', 'ǒ', 'ò']),
    ('u', ['u', 'ū', 'ú', 'ǔ', 'ù']),
    ('ü', ['ü', 'ǖ', 'ǘ', 'ǚ', 'ǜ']),
];

/// 单音节 "fei3" → "fěi"；规则：有 a 或 e 标其上；有 ou 标 o；否则标最后一个元音。
fn syllable_display(syl: &str) -> String {
    let tone = syl.chars().last().and_then(|c| c.to_digit(10)).unwrap_or(0) as usize;
    let base: String = syl.chars().filter(|c| !c.is_ascii_digit()).collect();
    if tone == 0 || tone > 4 {
        return base;
    }
    let chars: Vec<char> = base.chars().collect();
    let idx = ['a', 'e']
        .iter()
        .find_map(|t| chars.iter().position(|c| c.eq_ignore_ascii_case(t)))
        .or_else(|| chars.windows(2).position(|w| w[0] == 'o' && w[1] == 'u'))
        .unwrap_or_else(|| {
            chars
                .iter()
                .rposition(|c| "aeiouüv".contains(c.to_ascii_lowercase()))
                .unwrap_or(0)
        });
    let mut out = chars.clone();
    for (base_ch, marks) in TONE_MARKS {
        if out[idx].eq_ignore_ascii_case(base_ch) {
            let m = marks[tone];
            out[idx] = if out[idx].is_uppercase() {
                m.to_uppercase().next().unwrap()
            } else {
                m
            };
            break;
        }
    }
    out.into_iter().collect()
}

/// 数字调拼音串 "fei1 chang2" → "fēi cháng"；v 视作 ü（lv4 → lǜ）。
/// 纯字母 token（无声调音节或非拼音词）原样保留。
pub fn pinyin_display(numeric: &str) -> String {
    numeric
        .split_whitespace()
        .map(|s| {
            let s = s.replace('v', "ü");
            if s.chars().all(|c| c.is_ascii_alphabetic()) {
                s
            } else {
                syllable_display(&s)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// token 是否为带调拼音音节："ni3" / "Zhong1" / "lv4" → true；
/// 释义 token（"you"、"(informal)/hello"、"covid19"）→ false。
fn is_syllable_token(t: &str) -> bool {
    match t.chars().next_back() {
        Some(last) => {
            last.is_ascii_digit() && t.chars().rev().skip(1).all(|c| c.is_ascii_alphabetic())
        }
        None => false,
    }
}

/// 解析一行 CEDICT：`繁體 简体 [pin1 yin1] /gloss1/gloss2/`；`#` 开头为注释。
/// 真实 CEDICT 的拼音包在 `[ ]` 里（解析前剥掉）、释义包在 `/…/` 里：
/// 开头的 `/` 使该 token 不匹配音节，split('/') 后空段被滤掉。
/// 去括号的简写格式 `繁體 简体 pin1 yin1 gloss1/gloss2` 同样兼容。
pub fn parse_line(
    line: &str,
) -> Option<(
    String,      /*simp*/
    String,      /*trad*/
    String,      /*pinyin数字*/
    Vec<String>, /*glosses*/
)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let line = line.replace(['[', ']'], "");
    let mut tokens = line.split_whitespace();
    let trad = tokens.next()?;
    let simp = tokens.next()?;
    let rest: Vec<&str> = tokens.collect();
    let split_at = rest.iter().position(|t| !is_syllable_token(t))?;
    if split_at == 0 {
        return None;
    }
    let pinyin = rest[..split_at].join(" ");
    let glosses = rest[split_at..]
        .join(" ")
        .split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    Some((simp.into(), trad.into(), pinyin, glosses))
}

pub struct CedictDb {
    conn: Connection,
}

impl CedictDb {
    /// `IF NOT EXISTS`：对已有库重复导入（dicts/README.md 承诺的覆盖语义）
    /// 走 INSERT OR REPLACE 刷新，而不是报 table already exists。
    const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS cedict (simplified TEXT PRIMARY KEY, traditional TEXT, pinyin TEXT, glosses TEXT);";

    /// 解析 CEDICT 流并写入 `conn`；返回导入条数（from_reader / import_to_path 共用）。
    fn fill(conn: &Connection, reader: impl Read) -> anyhow::Result<u64> {
        conn.execute_batch(Self::SCHEMA)?;
        conn.execute_batch("BEGIN")?;
        let mut n = 0u64;
        for line in BufReader::new(reader).lines() {
            let line = line?;
            if let Some((simp, trad, py, glosses)) = parse_line(&line) {
                conn.execute(
                    "INSERT OR REPLACE INTO cedict VALUES (?1,?2,?3,?4)",
                    rusqlite::params![simp, trad, py, serde_json::to_string(&glosses)?],
                )?;
                n += 1;
            }
        }
        conn.execute_batch("COMMIT")?;
        anyhow::ensure!(n > 0, "no cedict entries parsed");
        Ok(n)
    }

    /// 全量导入到内存库（内嵌 / 测试用）。
    pub fn from_reader(reader: impl Read) -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::fill(&conn, reader)?;
        Ok(Self { conn })
    }

    /// 全量导入到磁盘库（`pardon-import --cedict <cedict.u8> <out.sqlite>` 用）；返回导入条数。
    pub fn import_to_path(reader: impl Read, path: &Path) -> anyhow::Result<u64> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "OFF")?;
        conn.pragma_update(None, "synchronous", "OFF")?;
        Self::fill(&conn, reader)
    }

    /// 只读打开磁盘上已导入的 CEDICT 库。
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            conn: Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?,
        })
    }

    /// 空内存库（词典缺失时的降级）：建 schema 不导数据，lookup 永远 miss。
    pub fn empty() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(Self::SCHEMA)?;
        Ok(Self { conn })
    }
}

impl DictProvider for CedictDb {
    fn lookup(&self, word: &str) -> Option<WordCard> {
        let r: rusqlite::Result<(String, String)> = self.conn.query_row(
            "SELECT pinyin, glosses FROM cedict WHERE simplified = ?1",
            [word],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        );
        match r {
            Ok((py, glosses)) => Some(WordCard {
                found: true,
                word: word.to_string(),
                phonetic: Some(Phonetic {
                    uk: Some(pinyin_display(&py)),
                    us: None,
                }),
                pos: vec![PosGloss {
                    pos: "".into(),
                    gloss: serde_json::from_str(&glosses).unwrap_or_default(),
                }],
                definition: vec![],
                exchange: None,
                collins: None,
                oxford: false,
                tags: vec![],
                source: "cedict".into(),
                suggestions: vec![],
            }),
            Err(_) => None,
        }
    }

    fn suggest(&self, _word: &str) -> Vec<String> {
        Vec::new()
    }
}

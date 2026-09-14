//! 翻译历史：SQLite 追加 + 查询 + 清空 + 封顶裁剪。
//! CLI（`pardon history`）与 daemon（剪贴板/触发翻译）共写同一库文件，
//! busy_timeout 容忍两端并发。

use anyhow::Context;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 历史保留上限（record 后裁剪更旧的行）。
pub const HISTORY_MAX_ROWS: usize = 1000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Unix 毫秒时间戳。
    pub ts_ms: i64,
    pub text: String,
    pub translation: String,
    /// 成功引擎（"ecdict"/"cedict"/provider id…）。
    pub engine: String,
    /// "auto" | "trigger" | "cli" | "gui"。
    pub origin: String,
}

pub struct History {
    conn: Connection,
}

impl History {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(path)
            .with_context(|| format!("open history db at {}", path.display()))?;
        conn.busy_timeout(std::time::Duration::from_secs(2))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS history (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 ts_ms INTEGER NOT NULL,
                 text TEXT NOT NULL,
                 translation TEXT NOT NULL,
                 engine TEXT NOT NULL,
                 origin TEXT NOT NULL
             );",
        )?;
        Ok(Self { conn })
    }

    /// 追加一条并按 [`HISTORY_MAX_ROWS`] 封顶。
    pub fn record(&mut self, e: &HistoryEntry) -> anyhow::Result<()> {
        self.record_with_cap(e, HISTORY_MAX_ROWS)
    }

    /// 追加 + 裁剪到 cap 条（测试可注入小 cap）。
    pub fn record_with_cap(&mut self, e: &HistoryEntry, cap: usize) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO history (ts_ms, text, translation, engine, origin)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![e.ts_ms, e.text, e.translation, e.engine, e.origin],
        )?;
        self.conn.execute(
            "DELETE FROM history WHERE id NOT IN
             (SELECT id FROM history ORDER BY id DESC LIMIT ?1)",
            rusqlite::params![cap as i64],
        )?;
        Ok(())
    }

    /// 最近 limit 条（新→旧）。
    pub fn list(&self, limit: u32) -> anyhow::Result<Vec<HistoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT ts_ms, text, translation, engine, origin
             FROM history ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![limit], |row| {
            Ok(HistoryEntry {
                ts_ms: row.get(0)?,
                text: row.get(1)?,
                translation: row.get(2)?,
                engine: row.get(3)?,
                origin: row.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// 清空，返回删除行数。
    pub fn clear(&mut self) -> anyhow::Result<usize> {
        let n = self.conn.execute("DELETE FROM history", [])?;
        Ok(n)
    }
}

/// 默认库路径：`<pardon_home>/history.sqlite`（PARDON_HOME 可覆盖）。
pub fn history_path() -> anyhow::Result<PathBuf> {
    Ok(crate::pipeline::pardon_home()?.join("history.sqlite"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(text: &str, ts: i64) -> HistoryEntry {
        HistoryEntry {
            ts_ms: ts,
            text: text.into(),
            translation: format!("译-{text}"),
            engine: "glm".into(),
            origin: "cli".into(),
        }
    }

    #[test]
    fn open_creates_schema_and_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nested").join("history.sqlite");
        let h = History::open(&p).unwrap();
        drop(h);
        assert!(p.exists(), "db file should be created");
    }

    #[test]
    fn record_then_list_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = History::open(&dir.path().join("h.sqlite")).unwrap();
        h.record(&entry("first", 100)).unwrap();
        h.record(&entry("second", 200)).unwrap();
        h.record(&entry("third", 300)).unwrap();
        let list = h.list(2).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].text, "third", "最新在前");
        assert_eq!(list[1].text, "second");
        assert_eq!(list[0].ts_ms, 300);
    }

    #[test]
    fn record_with_cap_prunes_oldest() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = History::open(&dir.path().join("h.sqlite")).unwrap();
        for i in 0..5 {
            h.record_with_cap(&entry(&format!("e{i}"), i), 3).unwrap();
        }
        let list = h.list(100).unwrap();
        assert_eq!(list.len(), 3, "封顶 3 条");
        assert_eq!(list[2].text, "e2", "最旧的 e0/e1 被裁剪");
        assert_eq!(list[0].text, "e4");
    }

    #[test]
    fn clear_empties_and_reports_count() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = History::open(&dir.path().join("h.sqlite")).unwrap();
        h.record(&entry("a", 1)).unwrap();
        h.record(&entry("b", 2)).unwrap();
        assert_eq!(h.clear().unwrap(), 2);
        assert!(h.list(10).unwrap().is_empty());
        assert_eq!(h.clear().unwrap(), 0);
    }

    #[test]
    fn list_on_empty_db_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let h = History::open(&dir.path().join("h.sqlite")).unwrap();
        assert!(h.list(20).unwrap().is_empty());
    }
}

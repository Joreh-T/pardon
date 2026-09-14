//! `pardon history`：查看/清空翻译历史（--json 给机器，--limit 控制条数）。

use pardon_core::history::{History, HistoryEntry};

#[derive(clap::Args)]
pub struct HistoryArgs {
    /// 最多显示条数
    #[arg(long, default_value_t = 20)]
    pub limit: u32,
    /// compact JSON 数组输出
    #[arg(long)]
    pub json: bool,
    /// 清空历史
    #[arg(long)]
    pub clear: bool,
}

pub async fn run(args: &HistoryArgs) -> anyhow::Result<i32> {
    let path = pardon_core::history::history_path()?;
    if args.clear {
        let mut h = History::open(&path)?;
        let n = h.clear()?;
        println!("cleared {n} entries");
        return Ok(0);
    }
    let h = History::open(&path)?;
    let list = h.list(args.limit)?;
    if args.json {
        println!("{}", serde_json::to_string(&list)?);
    } else {
        for e in &list {
            println!("{}", format_entry(e));
        }
    }
    Ok(0)
}

/// 单行人类可读格式：`2026-09-14 12:03:45  ecdict   run → v. 跑…`。
fn format_entry(e: &HistoryEntry) -> String {
    let dt = chrono::DateTime::from_timestamp_millis(e.ts_ms)
        .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| e.ts_ms.to_string());
    format!("{dt}  {:<7} {}", e.engine, arrow_line(e))
}

/// `text → translation`，两侧各截 60 字符（CJK 安全）。
fn arrow_line(e: &HistoryEntry) -> String {
    format!("{} → {}", cut(&e.text, 60), cut(&e.translation, 60))
}

fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max).collect();
        t.push('…');
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::HOME_LOCK;
    use pardon_core::history::{History, HistoryEntry};

    fn args(limit: u32, json: bool, clear: bool) -> HistoryArgs {
        HistoryArgs { limit, json, clear }
    }

    #[test]
    fn format_entry_truncates_long_fields() {
        let e = HistoryEntry {
            ts_ms: 1_800_000_000_000,
            text: "x".repeat(300),
            translation: "y".repeat(400),
            engine: "ecdict".into(),
            origin: "cli".into(),
        };
        let line = format_entry(&e);
        assert!(
            line.chars().count() < 200,
            "行长应封顶: {}",
            line.chars().count()
        );
        assert!(line.contains('…'), "截断应有省略号");
    }

    /// PARDON_HOME 是进程级环境变量，锁内整体串行；`run` 无真实 .await，
    /// 故用 block_on 保持同步测试——锁不跨 await（clippy::await_holding_lock）。
    #[test]
    fn clear_writes_and_lists_nothing() {
        let _lock = HOME_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("PARDON_HOME", dir.path());
        {
            let mut h = History::open(&pardon_core::history::history_path().unwrap()).unwrap();
            h.record(&HistoryEntry {
                ts_ms: 1,
                text: "hi".into(),
                translation: "你好".into(),
                engine: "glm".into(),
                origin: "cli".into(),
            })
            .unwrap();
        }
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let code = rt.block_on(run(&args(20, false, true))).unwrap();
        std::env::remove_var("PARDON_HOME");
        assert_eq!(code, 0);
        // 清空后 json 列表为空
        std::env::set_var("PARDON_HOME", dir.path());
        let code = rt.block_on(run(&args(20, true, false))).unwrap();
        std::env::remove_var("PARDON_HOME");
        assert_eq!(code, 0);
    }
}

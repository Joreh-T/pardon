//! 用法: pardon-import <ecdict.csv> <out.sqlite>
use std::io::BufReader;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let csv_path = args.next().expect("usage: pardon-import <ecdict.csv> <out.sqlite>");
    let out_path = args.next().expect("usage: pardon-import <ecdict.csv> <out.sqlite>");
    let f = std::fs::File::open(&csv_path)?;
    let conn = rusqlite::Connection::open(&out_path)?;
    conn.pragma_update(None, "journal_mode", "OFF")?;
    conn.pragma_update(None, "synchronous", "OFF")?;
    let n = pardon_core::dict::ecdict::import::import_csv(BufReader::new(f), &conn)?;
    println!("imported {n} entries into {out_path}");
    Ok(())
}

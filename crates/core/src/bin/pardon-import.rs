//! 用法: pardon-import <ecdict.csv> <out.sqlite>
//!       pardon-import --ecdict-sqlite <官方ecdict.db> <out.sqlite>   （推荐：release 的 sqlite 版）
//!       pardon-import --cedict <cedict.u8> <out.sqlite>
use std::io::BufReader;

fn usage() -> &'static str {
    "usage: pardon-import [--ecdict-sqlite | --cedict] <input> <out.sqlite>"
}

fn need(arg: Option<String>) -> String {
    arg.unwrap_or_else(|| {
        eprintln!("{}", usage());
        std::process::exit(2);
    })
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let first = need(args.next());
    if first == "--cedict" {
        let cedict_path = need(args.next());
        let out_path = need(args.next());
        let f = std::fs::File::open(&cedict_path)?;
        let n = pardon_core::dict::cedict::CedictDb::import_to_path(
            f,
            std::path::Path::new(&out_path),
        )?;
        println!("imported {n} entries into {out_path}");
        return Ok(());
    }
    if first == "--ecdict-sqlite" {
        let src_path = need(args.next());
        let out_path = need(args.next());
        let conn = rusqlite::Connection::open(&out_path)?;
        conn.pragma_update(None, "journal_mode", "OFF")?;
        conn.pragma_update(None, "synchronous", "OFF")?;
        let n = pardon_core::dict::ecdict::import::import_sqlite(
            std::path::Path::new(&src_path),
            &conn,
        )?;
        println!("imported {n} entries into {out_path}");
        return Ok(());
    }
    let out_path = need(args.next());
    let f = std::fs::File::open(&first)?;
    let conn = rusqlite::Connection::open(&out_path)?;
    conn.pragma_update(None, "journal_mode", "OFF")?;
    conn.pragma_update(None, "synchronous", "OFF")?;
    let n = pardon_core::dict::ecdict::import::import_csv(BufReader::new(f), &conn)?;
    println!("imported {n} entries into {out_path}");
    Ok(())
}

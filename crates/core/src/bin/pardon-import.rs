//! 用法: pardon-import <ecdict.csv> <out.sqlite>
//!       pardon-import --cedict <cedict.u8> <out.sqlite>
use std::io::BufReader;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let first = args.next().expect("usage: pardon-import [--cedict] <input> <out.sqlite>");
    if first == "--cedict" {
        let cedict_path =
            args.next().expect("usage: pardon-import --cedict <cedict.u8> <out.sqlite>");
        let out_path =
            args.next().expect("usage: pardon-import --cedict <cedict.u8> <out.sqlite>");
        let f = std::fs::File::open(&cedict_path)?;
        let n = pardon_core::dict::cedict::CedictDb::import_to_path(
            f, std::path::Path::new(&out_path))?;
        println!("imported {n} entries into {out_path}");
        return Ok(());
    }
    let out_path = args.next().expect("usage: pardon-import <ecdict.csv> <out.sqlite>");
    let f = std::fs::File::open(&first)?;
    let conn = rusqlite::Connection::open(&out_path)?;
    conn.pragma_update(None, "journal_mode", "OFF")?;
    conn.pragma_update(None, "synchronous", "OFF")?;
    let n = pardon_core::dict::ecdict::import::import_csv(BufReader::new(f), &conn)?;
    println!("imported {n} entries into {out_path}");
    Ok(())
}

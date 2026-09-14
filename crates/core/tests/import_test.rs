use pardon_core::dict::ecdict::import::{import_csv, import_sqlite};
use rusqlite::Connection;

fn setup() -> (Connection, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let conn = Connection::open(dir.path().join("t.sqlite")).unwrap();
    (conn, dir)
}

#[test]
fn imports_rows_and_builds_wordforms() {
    let (conn, _d) = setup();
    let n = import_csv(include_str!("fixtures/ecdict_mini.csv").as_bytes(), &conn).unwrap();
    assert_eq!(n, 7);
    let (w, pos_json): (String, String) = conn
        .query_row(
            "SELECT word, pos_json FROM entries WHERE word='run'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(w, "run");
    assert!(pos_json.contains("\"n.\""));
    // wordforms 反向表：gave → give
    let lemma: String = conn
        .query_row("SELECT lemma FROM wordforms WHERE form='gave'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(lemma, "give");
    let lemma: String = conn
        .query_row("SELECT lemma FROM wordforms WHERE form='boxes'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(lemma, "box");
}

#[test]
fn import_is_idempotent() {
    let (conn, _d) = setup();
    import_csv(include_str!("fixtures/ecdict_mini.csv").as_bytes(), &conn).unwrap();
    let n2 = import_csv(include_str!("fixtures/ecdict_mini.csv").as_bytes(), &conn).unwrap();
    assert_eq!(n2, 0, "重复导入相同词条应跳过");
    let cnt: i64 = conn
        .query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))
        .unwrap();
    assert_eq!(cnt, 7);
}

#[test]
fn perceive_row_columns_not_drifted() {
    let (conn, _d) = setup();
    import_csv(include_str!("fixtures/ecdict_mini.csv").as_bytes(), &conn).unwrap();
    let (collins, tags, ex_json): (i64, String, Option<String>) = conn
        .query_row(
            "SELECT collins, tags, exchange_json FROM entries WHERE word='perceive'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(collins, 2);
    assert_eq!(tags, "cet6 ky");
    assert!(ex_json.unwrap().contains("\"third\":\"perceives\""));
    // give up 行 definition 应为 surrender（列 3）
    let def: String = conn
        .query_row(
            "SELECT definition FROM entries WHERE word='give up'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(def, "surrender");
}

/// 构造一个官方 ECDICT SQLite 布局（stardict 表，含 id/sw 附加列）的 mini 源库，
/// 数据与 ecdict_mini.csv 一致（多行字段即含 \n 的字符串）。
fn official_minidb(path: &std::path::Path) {
    let s = Connection::open(path).unwrap();
    s.execute_batch(
        r#"CREATE TABLE stardict (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            word VARCHAR(64) NOT NULL, sw VARCHAR(64) NOT NULL,
            phonetic VARCHAR(64), definition TEXT, translation TEXT,
            pos VARCHAR(16), collins INTEGER DEFAULT 0, oxford INTEGER DEFAULT 0,
            tag VARCHAR(64), bnc INTEGER, frq INTEGER,
            exchange TEXT, detail TEXT, audio TEXT);"#,
    )
    .unwrap();
    // (word, phonetic, definition, translation, pos, collins, oxford, tag, bnc, frq, exchange)
    type Row<'a> = (
        &'a str,
        &'a str,
        &'a str,
        &'a str,
        &'a str,
        i64,
        i64,
        &'a str,
        i64,
        i64,
        &'a str,
    );
    let rows: Vec<Row> = vec![
        (
            "run",
            "rʌn",
            "move fast\noperate",
            "n. 跑步\nv. 跑；运转",
            "n:46/v:54",
            3,
            1,
            "zk gk cet4",
            1234,
            567,
            "p:ran/d:run/i:running/3:runs/0:run",
        ),
        ("gave", "ɡeɪv", "", "", "", 0, 0, "", 0, 0, "0:give"),
        (
            "give",
            "ɡɪv",
            "",
            "vt. 给予",
            "",
            3,
            1,
            "cet4",
            0,
            0,
            "d:given/p:gave/i:giving/3:gives/0:give",
        ),
        (
            "hello",
            "həˈləu",
            "",
            "int. 你好",
            "",
            0,
            0,
            "zk",
            1234,
            1,
            "",
        ),
        (
            "give up",
            "",
            "surrender",
            "放弃；认输",
            "",
            0,
            0,
            "",
            1234,
            10,
            "0:give up",
        ),
        (
            "box",
            "bɒks",
            "",
            "n. 盒子",
            "n:100",
            1,
            0,
            "zk",
            998,
            88,
            "s:boxes/0:box",
        ),
        (
            "perceive",
            "pəˈsiːv",
            "sense\nbecome aware of",
            "vt. 感知；察觉",
            "v:54",
            2,
            0,
            "cet6 ky",
            2500,
            4000,
            "d:perceived/p:perceived/3:perceives/i:perceiving/0:perceive",
        ),
    ];
    for r in rows {
        s.execute(
            "INSERT INTO stardict (word, sw, phonetic, definition, translation, pos, collins, oxford, tag, bnc, frq, exchange) \
             VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7, r.8, r.9, r.10],
        )
        .unwrap();
    }
}

#[test]
fn import_from_official_sqlite_layout() {
    let (conn, _d) = setup();
    let src_dir = tempfile::tempdir().unwrap();
    let src = src_dir.path().join("official.db");
    official_minidb(&src);

    let n = import_sqlite(&src, &conn).unwrap();
    assert_eq!(n, 7, "官方 sqlite 全部 7 条应导入");

    // 与 CSV 导入关键行为等价
    let (w, pos_json, collins): (String, String, i64) = conn
        .query_row(
            "SELECT word, pos_json, collins FROM entries WHERE word='run'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(w, "run");
    assert!(pos_json.contains("\"n.\"") && pos_json.contains("\"v.\""));
    assert_eq!(collins, 3);
    let lemma: String = conn
        .query_row("SELECT lemma FROM wordforms WHERE form='gave'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(lemma, "give");
    let def: String = conn
        .query_row(
            "SELECT definition FROM entries WHERE word='give up'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(def, "surrender");
}

#[test]
fn import_from_official_sqlite_is_idempotent() {
    let (conn, _d) = setup();
    let src_dir = tempfile::tempdir().unwrap();
    let src = src_dir.path().join("official.db");
    official_minidb(&src);
    import_sqlite(&src, &conn).unwrap();
    let n2 = import_sqlite(&src, &conn).unwrap();
    assert_eq!(n2, 0);
    let cnt: i64 = conn
        .query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))
        .unwrap();
    assert_eq!(cnt, 7);
}

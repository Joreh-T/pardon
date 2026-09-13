use pardon_core::dict::ecdict::import::import_csv;
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
    let (w, pos_json): (String, String) = conn.query_row(
        "SELECT word, pos_json FROM entries WHERE word='run'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!(w, "run");
    assert!(pos_json.contains("\"n.\""));
    // wordforms 反向表：gave → give
    let lemma: String = conn.query_row(
        "SELECT lemma FROM wordforms WHERE form='gave'", [], |r| r.get(0)).unwrap();
    assert_eq!(lemma, "give");
    let lemma: String = conn.query_row(
        "SELECT lemma FROM wordforms WHERE form='boxes'", [], |r| r.get(0)).unwrap();
    assert_eq!(lemma, "box");
}

#[test]
fn import_is_idempotent() {
    let (conn, _d) = setup();
    import_csv(include_str!("fixtures/ecdict_mini.csv").as_bytes(), &conn).unwrap();
    let n2 = import_csv(include_str!("fixtures/ecdict_mini.csv").as_bytes(), &conn).unwrap();
    assert_eq!(n2, 0, "重复导入相同词条应跳过");
    let cnt: i64 = conn.query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0)).unwrap();
    assert_eq!(cnt, 7);
}

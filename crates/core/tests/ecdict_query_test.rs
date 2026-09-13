use pardon_core::dict::ecdict_query::EcdictDb;
use pardon_core::dict::{DictProvider, WordCard};

fn db() -> EcdictDb {
    EcdictDb::from_reader(include_str!("fixtures/ecdict_mini.csv").as_bytes()).unwrap()
}

#[test]
fn exact_lookup_case_insensitive() {
    let card: WordCard = db().lookup("Run").unwrap();
    assert_eq!(card.word, "run");
    assert!(card.found);
    assert_eq!(card.phonetic.unwrap().uk.unwrap(), "rʌn");
    assert_eq!(card.exchange.unwrap().past.unwrap(), "ran");
}

#[test]
fn phrase_lookup() {
    assert!(db().lookup("give up").is_some());
}

#[test]
fn lemma_fallback_boxes_to_box() {
    // fixture 里没有 boxes 词条，只能走 wordforms 反向表还原 lemma
    let card = db().lookup("boxes").unwrap();
    assert_eq!(card.word, "box");
}

#[test]
fn gave_hits_entry_with_empty_pos() {
    // mini fixture 中 gave 行 translation 为空：词条本身入库，pos_json 为 "[]"
    let card = db().lookup("gave").unwrap();
    assert_eq!(card.word, "gave");
    assert!(card.pos.is_empty());
}

#[test]
fn miss_returns_none() {
    assert!(db().lookup("nonexistentword").is_none());
}

#[test]
fn suggest_edit_distance() {
    let s = db().suggest("helo");
    assert!(s.contains(&"hello".to_string()), "got {s:?}");
    assert!(s.len() <= 5);
}

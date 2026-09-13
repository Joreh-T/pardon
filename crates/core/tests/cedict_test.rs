use pardon_core::dict::cedict::{parse_line, pinyin_display, CedictDb};
use pardon_core::dict::DictProvider;

#[test]
fn parse_valid_line() {
    let (simp, trad, py, glosses) = parse_line("你好 你好 ni3 hao3 you (informal)/hello").unwrap();
    assert_eq!((simp.as_str(), trad.as_str(), py.as_str()), ("你好", "你好", "ni3 hao3"));
    assert_eq!(glosses, vec!["you (informal)", "hello"]);
}

#[test]
fn parse_skips_comments_and_blank() {
    assert!(parse_line("# comment").is_none());
    assert!(parse_line("").is_none());
}

#[test]
fn pinyin_tone_marks() {
    assert_eq!(pinyin_display("ni3 hao3"), "nǐ hǎo");
    assert_eq!(pinyin_display("Zhong1 guo2"), "Zhōng guó");
    assert_eq!(pinyin_display("fan1 yi4"), "fān yì");
    assert_eq!(pinyin_display("lv4"), "lǜ");
    assert_eq!(pinyin_display("fei1 chang2 hao3"), "fēi cháng hǎo");
}

#[test]
fn lookup_zh_entry() {
    let db = CedictDb::from_reader(include_str!("fixtures/cedict_mini.u8").as_bytes()).unwrap();
    let card = db.lookup("你好").unwrap();
    assert_eq!(card.word, "你好");
    assert_eq!(card.phonetic.unwrap().uk.unwrap(), "nǐ hǎo");
    assert!(card.pos[0].gloss.contains(&"hello".to_string()));
    assert_eq!(card.source, "cedict");
    // 3 字词条也能整串命中
    assert!(db.lookup("非常好").is_some());
    assert!(db.lookup("不存在的词组xyz").is_none());
}

#[test]
fn import_to_path_then_open_roundtrip() {
    // pardon-import --cedict 落盘路径：import_to_path 写文件库，open 只读回读
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cedict.sqlite");
    let n = CedictDb::import_to_path(
        std::io::Cursor::new(include_str!("fixtures/cedict_mini.u8")), &path).unwrap();
    assert_eq!(n, 5);
    let db = CedictDb::open(&path).unwrap();
    let card = db.lookup("绿").unwrap();
    assert_eq!(card.phonetic.unwrap().uk.unwrap(), "lǜ");
    assert_eq!(card.pos[0].gloss, vec!["green"]);
    // 繁简异形的词条按简体命中（CEDICT 行首列为繁體，lookup 走 simplified 列）
    assert!(db.lookup("翻译").is_some());
    assert!(db.lookup("中国").is_some());
}

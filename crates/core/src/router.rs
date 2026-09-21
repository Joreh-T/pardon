use crate::dict::DictProvider;
use crate::lang;

pub enum Route {
    /// 查词：携带实际用于查询的词（可能是输入的首词）
    Word(String),
    Sentence(String),
}

/// 是否为应剥离的隐形字符：零宽/格式字符（Cf）与控制字符（保留 \t\n 等
/// Unicode 空白）。PDF 复制常带零宽空格 U+200B/软连字符 U+00AD——它们
/// 不是 Unicode 空白，`trim()` 不认，会让「单词+零宽字符」整串查词典
/// 失配而误判成句子路由。
fn is_invisible(c: char) -> bool {
    matches!(c,
        '\u{00AD}'                 // soft hyphen（PDF 断字）
        | '\u{200B}'..='\u{200F}'  // 零宽空格/ZWNJ/ZWJ/方向标记
        | '\u{2060}'               // word joiner
        | '\u{FEFF}'               // BOM
        | '\u{202A}'..='\u{202E}'  // 双向嵌入控制
        | '\u{2066}'..='\u{2069}'  // 双向隔离
    ) || (c.is_control() && !c.is_whitespace())
}

/// 分流/查词前的文本清洗：剥隐形字符 + 首尾空白。内部保留 \t\n 等
/// 正常空白（多行句子照常进句路由）。
pub fn sanitize(text: &str) -> String {
    text.chars()
        .filter(|c| !is_invisible(*c))
        .collect::<String>()
        .trim()
        .to_string()
}

pub fn classify(text: &str, en: &dyn DictProvider, zh: &dyn DictProvider) -> Route {
    let t = &sanitize(text);
    if t.is_empty() {
        return Route::Sentence(t.into());
    }
    if lang::detect(t) == lang::Lang::Zh {
        if t.chars().count() <= 6 && zh.lookup(t).is_some() {
            return Route::Word(t.to_string());
        }
        return Route::Sentence(t.into());
    }
    if en.lookup(t).is_some() {
        return Route::Word(t.to_string());
    }
    let words: Vec<&str> = t.split_whitespace().collect();
    if words.len() <= 2 {
        // 逐词回退只服务「一个真词 + 干扰词」的选区（如 "run §1"）：
        // 恰好一个词命中词典时查该词。两个词都命中词典的是真短语
        // （"word query"）——整短语未收录时只查首词会丢掉后半内容
        // （错误分词），改走句路由整体翻译。
        let mut hit: Option<&str> = None;
        let mut hits = 0;
        for w in &words {
            if en.lookup(w).is_some() {
                hits += 1;
                hit = Some(w);
            }
        }
        if hits == 1 {
            return Route::Word(hit.unwrap().to_string());
        }
    }
    Route::Sentence(t.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dict::{DictProvider, WordCard};

    struct FakeDict {
        words: &'static [&'static str],
    }
    impl DictProvider for FakeDict {
        fn lookup(&self, w: &str) -> Option<WordCard> {
            // 对齐真实 ECDICT 的 COLLATE NOCASE（不区分大小写）
            self.words
                .iter()
                .copied()
                .find(|k| k.eq_ignore_ascii_case(w))
                .map(|k| WordCard {
                    found: true,
                    word: k.into(),
                    phonetic: None,
                    pos: vec![],
                    definition: vec![],
                    exchange: None,
                    collins: None,
                    oxford: false,
                    tags: vec![],
                    source: "fake".into(),
                    suggestions: vec![],
                })
        }
        fn suggest(&self, _: &str) -> Vec<String> {
            vec![]
        }
    }

    const EN: FakeDict = FakeDict {
        words: &["run", "give", "give up", "gave", "word", "query"],
    };
    const ZH: FakeDict = FakeDict {
        words: &["你好", "翻译", "非常好"],
    };

    #[test]
    fn en_word_hit() {
        assert!(matches!(classify("run", &EN, &ZH), Route::Word(_)));
    }

    #[test]
    fn en_phrase_whole_hit() {
        let r = classify("give up", &EN, &ZH);
        assert!(matches!(r, Route::Word(w) if w == "give up"));
    }

    #[test]
    fn en_two_words_fallback_to_first_word() {
        let r = classify("give quickly", &EN, &ZH);
        assert!(
            matches!(r, Route::Word(w) if w == "give"),
            "两词短语未收录时逐词回退"
        );
    }

    /// 真机复现：查 "word query" 曾只查首词 word、丢弃 query（错误分词）。
    /// 两个词都在词典里的是真短语：整短语未收录时应走句路由整体翻译，
    /// 而不是静默只取首词。
    #[test]
    fn en_two_dict_words_phrase_is_sentence() {
        assert!(matches!(
            classify("word query", &EN, &ZH),
            Route::Sentence(_)
        ));
        assert!(matches!(
            classify("Word Query", &EN, &ZH),
            Route::Sentence(_)
        ));
    }

    #[test]
    fn en_sentence() {
        assert!(matches!(
            classify("please give me the book quickly", &EN, &ZH),
            Route::Sentence(_)
        ));
    }

    #[test]
    fn zh_word_short_hit() {
        assert!(matches!(classify("你好", &EN, &ZH), Route::Word(_)));
        assert!(matches!(classify("非常好", &EN, &ZH), Route::Word(_)));
    }

    #[test]
    fn zh_over_six_chars_is_sentence() {
        assert!(matches!(
            classify("今天的天气非常好啊朋友们", &EN, &ZH),
            Route::Sentence(_)
        ));
    }

    #[test]
    fn zh_short_but_no_entry_is_sentence() {
        assert!(matches!(classify("吃饭了", &EN, &ZH), Route::Sentence(_)));
    }

    #[test]
    fn mixed_text_is_sentence() {
        assert!(matches!(
            classify("这个 API 设计", &EN, &ZH),
            Route::Sentence(_)
        ));
    }

    #[test]
    fn empty_is_sentence() {
        assert!(matches!(classify("   ", &EN, &ZH), Route::Sentence(_)));
    }

    /// 隐形字符清洗：单词 + 零宽空格/软连字符/BOM 应命中词典（词路由），
    /// 而非整串失配落到句路由（真机复现：U+200B 曾被 LLM 翻译）。
    #[test]
    fn invisible_chars_are_stripped_before_word_lookup() {
        assert!(matches!(classify("run\u{200B}", &EN, &ZH), Route::Word(w) if w == "run"));
        assert!(matches!(classify("run\u{FEFF}", &EN, &ZH), Route::Word(w) if w == "run"));
        assert!(matches!(classify("ru\u{00AD}n", &EN, &ZH), Route::Word(w) if w == "run"));
        assert!(matches!(classify(" run\u{2060} ", &EN, &ZH), Route::Word(w) if w == "run"));
    }

    /// 清洗保留正文语义：句中零宽字符剥除后仍按句子路由（不吞内容）。
    #[test]
    fn sanitize_strips_invisible_but_keeps_text() {
        assert_eq!(sanitize("hello\u{200B} world\u{00AD}"), "hello world");
        assert_eq!(sanitize("多行\n句子\t保持"), "多行\n句子\t保持");
        assert_eq!(sanitize("\u{200B}"), "");
    }
}

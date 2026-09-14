use crate::dict::DictProvider;
use crate::lang;

pub enum Route {
    /// 查词：携带实际用于查询的词（可能是输入的首词）
    Word(String),
    Sentence(String),
}

pub fn classify(text: &str, en: &dyn DictProvider, zh: &dyn DictProvider) -> Route {
    let t = text.trim();
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
        for w in &words {
            if en.lookup(w).is_some() {
                return Route::Word((*w).to_string());
            }
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
            self.words.contains(&w).then(|| WordCard {
                found: true,
                word: w.into(),
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
        words: &["run", "give", "give up", "gave"],
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
}

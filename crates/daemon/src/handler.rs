//! 文本处理核心：自动剪贴板事件与显式触发共用
//! （防回环/长度上限 → 分流翻译 → 通知 → 可选译文写回）。

use crate::state::DaemonState;
use pardon_core::pipeline::Translation;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// 事件来源：Auto = 剪贴板监听（防回环+长度上限生效）；
/// Trigger = 显式触发（快捷键/HTTP，全部绕过，用户意图明确）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Auto,
    Trigger,
}

pub enum HandleResult {
    Handled {
        translation: Translation,
        notified: bool,
    },
    /// "empty" | "oversize" | "duplicate"
    Skipped(&'static str),
}

/// 通知摘要里原文截断长度（字符，CJK 安全）。
const SUMMARY_MAX_CHARS: usize = 30;
/// 通知正文最大长度（字符）。
const BODY_MAX_CHARS: usize = 1200;
/// 未收录建议最多展示几个。
const MAX_SUGGESTIONS: usize = 3;

/// 核心处理链：trim → （Auto）长度上限 + 防回环 → 翻译 → 通知 →
/// （可选）译文写回剪贴板（写回前登记防回环哈希）。
pub async fn handle_text(state: &DaemonState, raw: &str, origin: Origin) -> HandleResult {
    let text = raw.trim();
    if text.is_empty() {
        return HandleResult::Skipped("empty");
    }
    if origin == Origin::Auto {
        if text.len() > state.cfg.daemon.max_text_bytes {
            log::debug!("skip oversize text ({} bytes)", text.len());
            return HandleResult::Skipped("oversize");
        }
        let now = std::time::Instant::now();
        if !state.guard.lock().await.admit(text, now) {
            return HandleResult::Skipped("duplicate");
        }
        state
            .counters
            .clipboard_events
            .fetch_add(1, Ordering::Relaxed);
    } else {
        state.counters.triggers.fetch_add(1, Ordering::Relaxed);
    }

    let translation = state.translator.translate(text).await;
    state.counters.translations.fetch_add(1, Ordering::Relaxed);

    // 词典命中（Word 路由）→ 完整词卡通知（音标+词形+可选徽章）；
    // 非词典结果 → 通用通知；文本为英文词形（≤2 词）且词典建议非空时，
    // 把「未收录；试试：…」追加为译文后的尾行（LLM 纠错译文与建议共存）。
    let dict_sourced = translation.engine == "ecdict" || translation.engine == "cedict";
    let (summary, body) = if dict_sourced && !translation.translation.is_empty() {
        let card = state.translator.lookup(&translation.text).await;
        format_word_notification(&card, state.cfg.daemon.show_word_badge)
    } else {
        let miss_hint = if is_word_shaped_en(&translation.text) {
            let card = state.translator.lookup(&translation.text).await;
            suggestion_hint(&card)
        } else {
            None
        };
        format_notification(&translation, miss_hint.as_deref())
    };
    // notify 是同步 D-Bus 往返（notify-rust 超时可至 ~25s）→ 挪到 blocking
    // 线程，避免卡住 runtime worker 与串行剪贴板消费循环（join 失败视同
    // notify 失败，语义同 http.rs 触发口读剪贴板）。
    let notifier = Arc::clone(&state.notifier);
    let notified = tokio::task::spawn_blocking(move || notifier.notify(&summary, &body))
        .await
        .unwrap_or_else(|e| Err(anyhow::anyhow!("notify task failed: {e}")))
        .inspect_err(|e| log::warn!("notify failed: {e:#}"))
        .is_ok();
    if notified {
        state.counters.notifications.fetch_add(1, Ordering::Relaxed);
    }

    // 译文写回剪贴板：先登记（防回环），再写入（write_clipboard 会 spawn
    // 子进程并等待，同样走 blocking 线程）
    if state.cfg.daemon.copy_translation && !translation.translation.is_empty() {
        let now = std::time::Instant::now();
        state
            .guard
            .lock()
            .await
            .register_write(&translation.translation, now);
        let clipboard = Arc::clone(&state.clipboard);
        let text = translation.translation.clone();
        let write = tokio::task::spawn_blocking(move || clipboard.write_clipboard(&text))
            .await
            .unwrap_or_else(|e| Err(anyhow::anyhow!("clipboard write task failed: {e}")));
        if let Err(e) = write {
            log::warn!("copy translation back failed: {e:#}");
        }
    }

    HandleResult::Handled {
        translation,
        notified,
    }
}

/// 英文词形判定：非中文且 ≤2 个空白分隔词（对齐 router::classify 的
/// 单词/两词回退边界；中文侧 CEDICT 无建议来源，直接排除）。
fn is_word_shaped_en(text: &str) -> bool {
    pardon_core::lang::detect(text) != pardon_core::lang::Lang::Zh
        && text.split_whitespace().count() <= 2
}

/// 词卡建议行：`未收录；试试：a、b`（最多 MAX_SUGGESTIONS 个）；无建议 None。
fn suggestion_hint(card: &pardon_core::dict::WordCard) -> Option<String> {
    if card.suggestions.is_empty() {
        return None;
    }
    let shown: Vec<String> = card
        .suggestions
        .iter()
        .take(MAX_SUGGESTIONS)
        .cloned()
        .collect();
    Some(format!("未收录；试试：{}", shown.join("、")))
}

/// 通知文案（句子/通用）：摘要 = 原文截断；正文 = 译文（词形
/// 未收录时追加「试试」尾行）、失败或未收录提示。摘要不带前缀——
/// 通知标题行只承载内容本身，品牌名不与单词/译文混排。
pub fn format_notification(tr: &Translation, miss_hint: Option<&str>) -> (String, String) {
    let summary = truncate_chars(&tr.text, SUMMARY_MAX_CHARS);
    let mut body = if !tr.translation.is_empty() {
        truncate_chars(&tr.translation, BODY_MAX_CHARS)
    } else if tr.engine.is_empty() {
        "翻译失败（引擎链全部失败）".to_string()
    } else {
        miss_hint.unwrap_or("未收录").to_string()
    };
    if !tr.translation.is_empty() {
        if let Some(hint) = miss_hint {
            body.push('\n');
            body.push_str(hint);
        }
    }
    (summary, body)
}

/// 词卡命中通知（词典风格分层）：标题 = 单词；正文 = 音标行（CEDICT 侧
/// 为拼音）→ 词性释义行 → 词形变化行。
pub fn format_word_notification(
    card: &pardon_core::dict::WordCard,
    show_badge: bool,
) -> (String, String) {
    // 音标优先英式、缺省美式（CEDICT 侧 uk 字段存的是拼音）
    let phonetic = card
        .phonetic
        .as_ref()
        .and_then(|p| p.uk.as_deref().or(p.us.as_deref()))
        .filter(|p| !p.is_empty());
    let mut lines: Vec<String> = Vec::new();
    if let Some(p) = phonetic {
        lines.push(format!("/{p}/"));
    }
    let text = pardon_core::pipeline::card_text(card);
    if !text.is_empty() {
        lines.push(text);
    }
    if let Some(forms) = exchange_line(&card.exchange) {
        lines.push(forms);
    }
    if show_badge {
        if let Some(badge) = badge_line(card) {
            lines.push(badge);
        }
    }
    (
        truncate_chars(&card.word, SUMMARY_MAX_CHARS),
        truncate_chars(&lines.join("\n"), BODY_MAX_CHARS),
    )
}

/// 学习优先级徽章行：`柯林斯 ★★★★ · 牛津核心 · 高考 · 雅思`。
/// 星级/牛津/考试标签全部缺席时返回 None（不加空行）。
fn badge_line(card: &pardon_core::dict::WordCard) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(c) = card.collins {
        if c > 0 {
            parts.push(format!("柯林斯 {}", "★".repeat((c.min(5)) as usize)));
        }
    }
    if card.oxford {
        parts.push("牛津核心".into());
    }
    parts.extend(card.tags.iter().map(|t| exam_tag_label(t)));
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

/// ECDICT 考试标签 → 中文；未识别的标签原样透出（新增词表不至于丢信息）。
fn exam_tag_label(tag: &str) -> String {
    match tag {
        "zk" => "中考".into(),
        "gk" => "高考".into(),
        "cet4" => "四级".into(),
        "cet6" => "六级".into(),
        "ky" => "考研".into(),
        "toefl" => "托福".into(),
        "ielts" => "雅思".into(),
        "gre" => "GRE".into(),
        other => other.to_string(),
    }
}

/// 词形变化一行（去重保序、封顶 4 个）：`词形：ran · running · runs`。
fn exchange_line(ex: &Option<pardon_core::dict::Exchange>) -> Option<String> {
    let ex = ex.as_ref()?;
    let mut seen: Vec<&str> = Vec::new();
    for form in [
        &ex.past,
        &ex.pp,
        &ex.ing,
        &ex.third,
        &ex.comparative,
        &ex.superlative,
        &ex.plural,
    ] {
        if let Some(form) = form.as_deref() {
            if !form.is_empty() && !seen.contains(&form) {
                seen.push(form);
            }
        }
    }
    if seen.is_empty() {
        None
    } else {
        let shown: Vec<&str> = seen.iter().take(4).copied().collect();
        Some(format!("词形：{}", shown.join(" · ")))
    }
}

/// 按字符截断（CJK 安全），超长追加省略号。
pub fn truncate_chars(s: &str, max: usize) -> String {
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
    use crate::state::DaemonState;
    use crate::testing::*;
    use pardon_core::config::AppConfig;
    use pardon_core::dict::WordCard;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};

    fn word_card_suggestions(v: &[&str]) -> WordCard {
        WordCard {
            found: false,
            word: String::new(),
            phonetic: None,
            pos: vec![],
            definition: vec![],
            exchange: None,
            collins: None,
            oxford: false,
            tags: vec![],
            source: "ecdict".into(),
            suggestions: v.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// 组装测试 state：默认 AppConfig + 注入的假实现。
    fn test_state(
        tr: Arc<FakeTranslator>,
        no: Arc<FakeNotifier>,
        cb: Arc<FakeClipboard>,
        cfg: AppConfig,
    ) -> DaemonState {
        use std::time::Duration;
        DaemonState {
            guard: tokio::sync::Mutex::new(pardon_core::loopguard::LoopGuard::new(
                Duration::from_millis(cfg.daemon.dedup_window_ms),
            )),
            cfg,
            started: std::time::Instant::now(),
            translator: tr,
            notifier: no,
            clipboard: cb,
            counters: Default::default(),
            clipboard_watching: std::sync::atomic::AtomicBool::new(false),
            shutdown: Arc::new(tokio::sync::Notify::new()),
        }
    }

    fn base_cfg() -> AppConfig {
        let mut cfg = AppConfig::default();
        cfg.daemon.dedup_window_ms = 10_000; // 测试不等待真实过期
        cfg
    }

    /// 1. 空白文本（Auto）→ Skipped("empty")，翻译器零调用。
    #[tokio::test]
    async fn empty_text_is_skipped() {
        let ft = Arc::new(FakeTranslator::new());
        let s = test_state(
            ft.clone(),
            Arc::new(FakeNotifier::default()),
            Arc::new(FakeClipboard::default()),
            base_cfg(),
        );
        let r = handle_text(&s, "  \n ", Origin::Auto).await;
        assert!(matches!(r, HandleResult::Skipped("empty")));
        assert!(ft.calls.lock().unwrap().is_empty());
    }

    /// 2. 超长文本 Auto → Skipped("oversize")；Trigger 绕过上限 → Handled。
    #[tokio::test]
    async fn oversize_auto_skipped_but_trigger_bypasses() {
        let mut cfg = base_cfg();
        cfg.daemon.max_text_bytes = 10;
        let s = test_state(
            Arc::new(FakeTranslator::new()),
            Arc::new(FakeNotifier::default()),
            Arc::new(FakeClipboard::default()),
            cfg,
        );
        let text = "aaaaaaaaaaaaaaaaaaaa"; // 20 bytes > 10
        let r = handle_text(&s, text, Origin::Auto).await;
        assert!(matches!(r, HandleResult::Skipped("oversize")));
        let r = handle_text(&s, text, Origin::Trigger).await;
        assert!(matches!(r, HandleResult::Handled { .. }));
    }

    /// 3. 同文本连续两次 Auto：第二次命中去重窗 → Skipped("duplicate")。
    #[tokio::test]
    async fn duplicate_auto_event_deduped() {
        let s = test_state(
            Arc::new(FakeTranslator::new()),
            Arc::new(FakeNotifier::default()),
            Arc::new(FakeClipboard::default()),
            base_cfg(),
        );
        let first = handle_text(&s, "hello world", Origin::Auto).await;
        assert!(matches!(first, HandleResult::Handled { .. }));
        let second = handle_text(&s, "hello world", Origin::Auto).await;
        assert!(matches!(second, HandleResult::Skipped("duplicate")));
    }

    /// 4a. 词形 LLM 结果（拼错词）→ 通知正文 = 译文 + 「试试」尾行。
    #[tokio::test]
    async fn word_shaped_llm_result_appends_suggestions() {
        let ft = FakeTranslator::new();
        ft.results.lock().unwrap().push((
            "runnign".into(),
            FakeTranslator::sentence("runnign", "(LLM 认为你想说 running)", "glm"),
        ));
        *ft.lookup_card.lock().unwrap() = word_card_suggestions(&["running", "run"]);
        let no = Arc::new(FakeNotifier::default());
        let s = test_state(
            Arc::new(ft),
            no.clone(),
            Arc::new(FakeClipboard::default()),
            base_cfg(),
        );
        handle_text(&s, "runnign", Origin::Trigger).await;
        let sent = no.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert!(
            sent[0].1.contains("(LLM 认为你想说 running)"),
            "译文仍在: {}",
            sent[0].1
        );
        assert!(
            sent[0].1.ends_with("未收录；试试：running、run"),
            "尾行应为建议: {}",
            sent[0].1
        );
    }

    /// 4b. 句子（>2 词）不查建议、无尾行。
    #[tokio::test]
    async fn sentence_result_has_no_suggestion_line() {
        let ft = Arc::new(FakeTranslator::new());
        ft.results.lock().unwrap().push((
            "please give me the book quickly".into(),
            FakeTranslator::sentence("please give me the book quickly", "请把书快给我", "glm"),
        ));
        *ft.lookup_card.lock().unwrap() = word_card_suggestions(&["running"]);
        let no = Arc::new(FakeNotifier::default());
        let s = test_state(
            ft.clone(),
            no.clone(),
            Arc::new(FakeClipboard::default()),
            base_cfg(),
        );
        handle_text(&s, "please give me the book quickly", Origin::Auto).await;
        let sent = no.sent.lock().unwrap();
        assert_eq!(sent[0].1, "请把书快给我", "不应有建议尾行: {}", sent[0].1);
        // lookup 不应被调用（句子不做建议查询）
        assert!(ft.lookup_card_calls.load(Ordering::Relaxed) == 0);
    }

    /// 4c. 中文词形不走建议（CEDICT 无 suggest 来源）。
    #[tokio::test]
    async fn chinese_text_skips_suggestion_lookup() {
        let ft = Arc::new(FakeTranslator::new());
        ft.results.lock().unwrap().push((
            "吃饭了".into(),
            FakeTranslator::sentence("吃饭了", "have eaten", "glm"),
        ));
        let no = Arc::new(FakeNotifier::default());
        let s = test_state(
            ft.clone(),
            no.clone(),
            Arc::new(FakeClipboard::default()),
            base_cfg(),
        );
        handle_text(&s, "吃饭了", Origin::Auto).await;
        let sent = no.sent.lock().unwrap();
        assert_eq!(sent[0].1, "have eaten");
        assert!(ft.lookup_card_calls.load(Ordering::Relaxed) == 0);
    }

    /// 5. 引擎链全失败（译文与引擎皆空）→ 通知正文「翻译失败」。
    #[tokio::test]
    async fn sentence_failure_shows_fallback_body() {
        let ft = FakeTranslator::new();
        ft.results.lock().unwrap().push((
            "hello world".into(),
            FakeTranslator::sentence("hello world", "", ""),
        ));
        let no = Arc::new(FakeNotifier::default());
        let s = test_state(
            Arc::new(ft),
            no.clone(),
            Arc::new(FakeClipboard::default()),
            base_cfg(),
        );
        let r = handle_text(&s, "hello world", Origin::Auto).await;
        assert!(matches!(r, HandleResult::Handled { .. }));
        let sent = no.sent.lock().unwrap();
        assert!(sent[0].1.contains("翻译失败"), "body: {}", sent[0].1);
    }

    /// 6. 成功翻译 → notified=true，translations/notifications 计数各 1。
    #[tokio::test]
    async fn successful_translation_notifies_and_counts() {
        let s = test_state(
            Arc::new(FakeTranslator::new()),
            Arc::new(FakeNotifier::default()),
            Arc::new(FakeClipboard::default()),
            base_cfg(),
        );
        let r = handle_text(&s, "hello world", Origin::Auto).await;
        match r {
            HandleResult::Handled { notified, .. } => assert!(notified),
            HandleResult::Skipped(w) => panic!("unexpected Skipped({w})"),
        }
        assert_eq!(s.counters.translations.load(Ordering::Relaxed), 1);
        assert_eq!(s.counters.notifications.load(Ordering::Relaxed), 1);
    }

    /// 7. 通知失败 → notified=false，但仍 Handled（不视为错误）。
    #[tokio::test]
    async fn notify_failure_still_returns_handled() {
        let s = test_state(
            Arc::new(FakeTranslator::new()),
            Arc::new(FakeNotifier {
                sent: Mutex::new(vec![]),
                fail: true,
            }),
            Arc::new(FakeClipboard::default()),
            base_cfg(),
        );
        let r = handle_text(&s, "hello world", Origin::Auto).await;
        match r {
            HandleResult::Handled { notified, .. } => assert!(!notified),
            HandleResult::Skipped(w) => panic!("unexpected Skipped({w})"),
        }
    }

    /// 8. copy_translation 开 → 译文写回剪贴板；回声译文再触发被防回环拦截。
    #[tokio::test]
    async fn copy_translation_writes_back_and_blocks_echo() {
        let mut cfg = base_cfg();
        cfg.daemon.copy_translation = true;
        let ft = FakeTranslator::new();
        ft.results.lock().unwrap().push((
            "hello".into(),
            FakeTranslator::sentence("hello", "你好", "glm"),
        ));
        let cb = Arc::new(FakeClipboard::default());
        let s = test_state(
            Arc::new(ft),
            Arc::new(FakeNotifier::default()),
            cb.clone(),
            cfg,
        );
        let r = handle_text(&s, "hello", Origin::Auto).await;
        match r {
            HandleResult::Handled {
                translation,
                notified,
            } => {
                assert!(notified);
                assert_eq!(translation.translation, "你好");
            }
            HandleResult::Skipped(w) => panic!("unexpected Skipped({w})"),
        }
        assert_eq!(*cb.written.lock().unwrap(), vec!["你好".to_string()]);
        // 用户/回声再复制译文 → LoopGuard 拦截
        let echo = handle_text(&s, "你好", Origin::Auto).await;
        assert!(matches!(echo, HandleResult::Skipped("duplicate")));
    }

    /// 9. 按字符截断（CJK 安全）：4 字后加省略号；短串原样返回。
    #[tokio::test]
    async fn truncate_chars_cjk_safe() {
        assert_eq!(truncate_chars("你好世界！！！", 4), "你好世界…");
        assert_eq!(truncate_chars("abc", 10), "abc");
        assert_eq!(truncate_chars("", 3), "");
    }

    /// 10. 句子/LLM 结果：summary = `pardon · 原文`，body = 译文（无音标）。
    #[tokio::test]
    async fn format_notification_sentence_has_no_phonetic() {
        let tr = FakeTranslator::sentence("hello world", "你好世界", "glm");
        let (summary, body) = format_notification(&tr, None);
        assert_eq!(summary, "hello world");
        assert!(!summary.contains('/'), "LLM 摘要不应有音标: {summary}");
        assert_eq!(body, "你好世界");
    }

    /// 11. 词卡命中通知：音标进摘要、词形变化进正文（去重封顶）。
    #[tokio::test]
    async fn word_hit_notification_has_phonetic_and_exchange() {
        let card = WordCard {
            found: true,
            word: "run".into(),
            phonetic: Some(pardon_core::dict::Phonetic {
                uk: Some("rʌn".into()),
                us: None,
            }),
            pos: vec![pardon_core::dict::PosGloss {
                pos: "v.".into(),
                gloss: vec!["跑".into(), "运转".into()],
            }],
            definition: vec![],
            exchange: Some(pardon_core::dict::Exchange {
                past: Some("ran".into()),
                pp: Some("ran".into()), // 与 past 相同 → 去重
                ing: Some("running".into()),
                third: Some("runs".into()),
                comparative: None,
                superlative: None,
                plural: None,
                lemma: None,
            }),
            collins: Some(4),
            oxford: true,
            tags: vec!["gk".into(), "ielts".into()],
            source: "ecdict".into(),
            suggestions: vec![],
        };
        let (summary, body) = format_word_notification(&card, true);
        assert_eq!(summary, "run");
        assert!(body.starts_with("/rʌn/\n"), "音标应为正文首行: {body}");
        assert!(body.contains("v. 跑；运转"), "body: {body}");
        assert!(body.contains("词形：ran · running · runs"), "body: {body}");
        assert!(
            body.ends_with("柯林斯 ★★★★ · 牛津核心 · 高考 · 雅思"),
            "badge line: {body}"
        );
        // 开关关闭 → 徽章行不出现
        let (_, body_off) = format_word_notification(&card, false);
        assert!(!body_off.contains("柯林斯"), "body_off: {body_off}");
    }

    /// 12. 词卡无音标：摘要只有词；无词形：正文只有释义行。
    #[tokio::test]
    async fn word_notification_without_phonetic_or_exchange() {
        let card = WordCard {
            found: true,
            word: "gave".into(),
            phonetic: None,
            pos: vec![],
            definition: vec![],
            exchange: None,
            collins: None,
            oxford: false,
            tags: vec![],
            source: "ecdict".into(),
            suggestions: vec![],
        };
        let (summary, body) = format_word_notification(&card, true);
        assert_eq!(summary, "gave");
        assert_eq!(body, "");
    }

    /// 13. handle_text 走词典命中路径：通知来自词卡（音标可见）。
    #[tokio::test]
    async fn handle_text_word_hit_uses_rich_card_notification() {
        let ft = FakeTranslator::new();
        ft.results.lock().unwrap().push((
            "run".into(),
            FakeTranslator::sentence("run", "v. 跑", "ecdict"),
        ));
        *ft.lookup_card.lock().unwrap() = WordCard {
            found: true,
            word: "run".into(),
            phonetic: Some(pardon_core::dict::Phonetic {
                uk: Some("rʌn".into()),
                us: None,
            }),
            pos: vec![pardon_core::dict::PosGloss {
                pos: "v.".into(),
                gloss: vec!["跑".into()],
            }],
            definition: vec![],
            exchange: None,
            collins: Some(3),
            oxford: true,
            tags: vec!["cet4".into()],
            source: "ecdict".into(),
            suggestions: vec![],
        };
        let no = Arc::new(FakeNotifier::default());
        let s = test_state(
            Arc::new(ft),
            no.clone(),
            Arc::new(FakeClipboard::default()),
            base_cfg(),
        );
        let r = handle_text(&s, "run", Origin::Trigger).await;
        assert!(matches!(r, HandleResult::Handled { .. }));
        let sent = no.sent.lock().unwrap();
        assert_eq!(sent[0].0, "run");
        assert!(sent[0].1.starts_with("/rʌn/"), "body: {}", sent[0].1);
        assert!(sent[0].1.contains("v. 跑"), "body: {}", sent[0].1);
        // 默认配置 show_word_badge=false → 徽章不出现（词卡数据有 3★/牛津/四级）
        assert!(!sent[0].1.contains("柯林斯"), "body: {}", sent[0].1);
    }
}

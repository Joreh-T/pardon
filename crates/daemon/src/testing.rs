//! 测试专用假实现（lib.rs 以 `#[doc(hidden)] pub mod` 常驻挂载，供单元与
//! 集成测试共用）：翻译器/通知器/剪贴板。

use crate::state::Translator;
use pardon_core::dict::WordCard;
use pardon_core::pipeline::Translation;
use pardon_platform::{ClipboardAccess, Notifier};
use std::sync::Mutex;

/// 记录调用的假翻译器：translate 返回预设表（文本→结果），未命中返回
/// 标准成功句；lookup 返回预设词卡。`delayed` 构造的实例改用映射函数
/// 生成译文（带延迟，剪贴板合并消费测试用）。
/// 延迟映射函数类型（clippy type_complexity 规避别名）。
type Mapper = Box<dyn Fn(&str) -> Translation + Send + Sync>;

pub struct FakeTranslator {
    pub(crate) results: Mutex<Vec<(String, Translation)>>,
    pub(crate) lookup_card: Mutex<WordCard>,
    pub(crate) delay_ms: u64,
    pub(crate) calls: Mutex<Vec<String>>,
    /// lookup 调用次数（handler 建议查询可达性断言用）。
    pub(crate) lookup_card_calls: std::sync::atomic::AtomicUsize,
    mapper: Option<Mapper>,
}

impl FakeTranslator {
    pub fn new() -> Self {
        Self {
            results: Mutex::new(vec![]),
            lookup_card: Mutex::new(WordCard {
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
                suggestions: vec![],
            }),
            delay_ms: 0,
            calls: Mutex::new(vec![]),
            lookup_card_calls: std::sync::atomic::AtomicUsize::new(0),
            mapper: None,
        }
    }

    /// 预设「文本→译文」映射表构造（未命中回落标准成功句）。
    pub fn with_map(map: Vec<(String, Translation)>) -> Self {
        Self {
            results: Mutex::new(map),
            ..Self::new()
        }
    }

    /// 带翻译延迟 + 逐文本映射函数的实例（忽略 results 预设表）。
    pub fn delayed(
        delay_ms: u64,
        mapper: impl Fn(&str) -> Translation + Send + Sync + 'static,
    ) -> Self {
        Self {
            delay_ms,
            mapper: Some(Box::new(mapper)),
            ..Self::new()
        }
    }

    /// 标准成功句构造（Lang: En→Zh）。
    pub fn sentence(text: &str, translation: &str, engine: &str) -> Translation {
        Translation {
            source_lang: pardon_core::lang::Lang::En,
            target_lang: pardon_core::lang::Lang::Zh,
            text: text.into(),
            translation: translation.into(),
            engine: engine.into(),
        }
    }
}

impl Default for FakeTranslator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Translator for FakeTranslator {
    async fn translate(&self, text: &str) -> Translation {
        if self.delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        }
        self.calls.lock().unwrap().push(text.to_string());
        if let Some(mapper) = &self.mapper {
            return mapper(text);
        }
        let r = self
            .results
            .lock()
            .unwrap()
            .iter()
            .find(|(t, _)| t == text)
            .map(|(_, tr)| tr.clone());
        r.unwrap_or_else(|| Self::sentence(text, "（译文）", "glm"))
    }
    async fn lookup(&self, word: &str) -> WordCard {
        self.lookup_card_calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut c = self.lookup_card.lock().unwrap().clone();
        c.word = word.to_string();
        c
    }
}

#[derive(Default)]
pub struct FakeNotifier {
    /// 已发通知 (summary, body)。pub：集成测试直接断言。
    pub sent: Mutex<Vec<(String, String)>>,
    pub(crate) fail: bool,
}
impl Notifier for FakeNotifier {
    fn notify(&self, summary: &str, body: &str) -> anyhow::Result<()> {
        if self.fail {
            anyhow::bail!("no notification daemon");
        }
        self.sent
            .lock()
            .unwrap()
            .push((summary.into(), body.into()));
        Ok(())
    }
}

pub struct FakeClipboard {
    pub(crate) written: Mutex<Vec<String>>,
    /// `read_primary` 的返回值（HTTP 触发口测试用；默认空串成功）。
    pub primary_result: anyhow::Result<String>,
    /// `read_clipboard` 的返回值。
    pub clipboard_result: anyhow::Result<String>,
}
impl Default for FakeClipboard {
    fn default() -> Self {
        // anyhow::Error 未实现 Default → 手写（derive 不可用）
        Self {
            written: Mutex::new(vec![]),
            primary_result: Ok(String::new()),
            clipboard_result: Ok(String::new()),
        }
    }
}
impl ClipboardAccess for FakeClipboard {
    fn read_clipboard(&self) -> anyhow::Result<String> {
        match &self.clipboard_result {
            Ok(s) => Ok(s.clone()),
            // anyhow::Error 非 Clone → 以链式文案重建（anyhow Display alternate）
            Err(e) => Err(anyhow::anyhow!("{e:#}")),
        }
    }
    fn read_primary(&self) -> anyhow::Result<String> {
        match &self.primary_result {
            Ok(s) => Ok(s.clone()),
            Err(e) => Err(anyhow::anyhow!("{e:#}")),
        }
    }
    fn write_clipboard(&self, text: &str) -> anyhow::Result<()> {
        self.written.lock().unwrap().push(text.to_string());
        Ok(())
    }
}

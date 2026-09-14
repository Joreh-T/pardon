//! TTS：有道 dictvoice 下载 + mp3 本地缓存 + espeak-ng 离线兜底。
//!
//! 朗读流程（`Tts::speak`）：
//! 1. `PARDON_TTS_DISABLE=1`（测试开关）→ 不下载不播放直接 Ok；
//! 2. 命中缓存（`sha1(voice|text).mp3`；0 字节视为未命中）→ 直接播放；
//! 3. 未命中 → 下载 dictvoice mp3（5s 超时）→ 原子写缓存（tmp+rename）→ 播放；
//! 4. 下载失败 → 把原文 UTF-8 字节交给播放链，
//!    合成型播放器（espeak-ng）朗读文本，非合成播放器（rodio）解码失败跳过。

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use sha1::{Digest, Sha1};

use crate::lang::Lang;

const YOUDAO_HOST: &str = "dict.youdao.com";

/// 测试开关判定：`PARDON_TTS_DISABLE` 精确等于 `"1"` 才禁用。
/// 纯函数（不读 env），便于零竞态单测；真实读取在 [`tts_disabled`]。
fn parse_tts_disable(v: Option<&str>) -> bool {
    v == Some("1")
}

/// 读 env 判定测试开关。置于 [`Tts::speak`] 入口：置 1 时不下载不播放
/// 直接 Ok——集成测试借此跳过真实网络与音频设备。
fn tts_disabled() -> bool {
    parse_tts_disable(std::env::var("PARDON_TTS_DISABLE").ok().as_deref())
}

/// 发音（对应 dictvoice 的 `type` / `le` 参数）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Voice {
    Uk,
    Us,
    Zh,
}

impl Voice {
    /// 缓存键等内部用途的短标记。
    fn tag(self) -> &'static str {
        match self {
            Voice::Uk => "uk",
            Voice::Us => "us",
            Voice::Zh => "zh",
        }
    }
}

/// 百分号编码：RFC 3986 unreserved 之外全部 `%XX`（空格→`%20`），
/// 非 ASCII 按 UTF-8 字节编码（`你` → `%E4%BD%A0`）。
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 有道 dictvoice 下载 URL：
/// `Uk` → `?audio=..&type=1`，`Us` → `?audio=..&type=2`，`Zh` → `?audio=..&le=zh`（无 type）。
pub fn youdao_voice_url(text: &str, voice: Voice) -> String {
    let audio = urlencode(text);
    match voice {
        Voice::Uk => format!("https://{YOUDAO_HOST}/dictvoice?audio={audio}&type=1"),
        Voice::Us => format!("https://{YOUDAO_HOST}/dictvoice?audio={audio}&type=2"),
        Voice::Zh => format!("https://{YOUDAO_HOST}/dictvoice?audio={audio}&le=zh"),
    }
}

/// 播放器抽象。
///
/// 入参通常是 mp3 音频数据；**在线下载失败时**收到的是待朗读的原始
/// UTF-8 文本字节——可合成的播放器（如 `EspeakPlayer`）据此直接朗读，
/// 仅能播放的 `RodioPlayer` 解码失败返回 Err 并落到链上下一个。
pub trait AudioPlayer: Send + Sync {
    fn play(&self, mp3: &[u8]) -> anyhow::Result<()>;
}

/// rodio 播放器：解码 mp3 并阻塞播放至结束（无默认输出设备则 Err）。
pub struct RodioPlayer;

impl AudioPlayer for RodioPlayer {
    fn play(&self, mp3: &[u8]) -> anyhow::Result<()> {
        // OutputStream 须存活到播放结束，故绑定 `_stream` 不掉。
        let (_stream, handle) =
            rodio::OutputStream::try_default().context("打开默认音频输出设备失败")?;
        let sink = rodio::Sink::try_new(&handle).context("创建 rodio sink 失败")?;
        // Sink::append 要求 Source: 'static，故拷贝一份 Owned 数据。
        let source =
            rodio::Decoder::new(std::io::Cursor::new(mp3.to_vec())).context("mp3 解码失败")?;
        sink.append(source);
        sink.sleep_until_end();
        Ok(())
    }
}

/// espeak-ng 合成播放器：把入参当 UTF-8 文本朗读（英文 `-v en`，中文 `-v zh`）。
/// 二进制缺席或退出非 0 返回 Err。
pub struct EspeakPlayer;

impl AudioPlayer for EspeakPlayer {
    fn play(&self, mp3: &[u8]) -> anyhow::Result<()> {
        let text = std::str::from_utf8(mp3).context("espeak 载荷不是 UTF-8 文本")?;
        let voice = match crate::lang::detect(text) {
            Lang::Zh => "zh",
            Lang::En => "en",
        };
        let out = std::process::Command::new("espeak-ng")
            .arg("-v")
            .arg(voice)
            .arg(text)
            .output()
            .context("espeak-ng 不可用")?;
        if !out.status.success() {
            anyhow::bail!(
                "espeak-ng 退出码 {:?}: {}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(())
    }
}

pub struct Tts {
    http: reqwest::Client,
    cache_dir: PathBuf,
    fallback: Vec<Box<dyn AudioPlayer>>,
}

impl Tts {
    /// 默认播放链：rodio 播 mp3，espeak-ng 朗读文本兜底。
    pub fn new(cache_dir: PathBuf) -> Self {
        Self::new_with_players(
            cache_dir,
            vec![Box::new(RodioPlayer), Box::new(EspeakPlayer)],
        )
    }

    pub fn new_with_players(cache_dir: PathBuf, players: Vec<Box<dyn AudioPlayer>>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("build tts http client");
        Self {
            http,
            cache_dir,
            fallback: players,
        }
    }

    /// 注入自定义 http client（测试把 dict.youdao.com 解析到不可达地址）。
    #[cfg(test)]
    fn with_client(
        cache_dir: PathBuf,
        players: Vec<Box<dyn AudioPlayer>>,
        http: reqwest::Client,
    ) -> Self {
        Self {
            http,
            cache_dir,
            fallback: players,
        }
    }

    /// 缓存路径：`sha1(tag|text).mp3`——同 (text, voice) 稳定，跨 voice 不同。
    pub fn cache_path(&self, text: &str, voice: Voice) -> PathBuf {
        let mut hasher = Sha1::new();
        hasher.update(voice.tag().as_bytes());
        // 分隔符避免 tag 与 text 交界处的哈希歧义
        hasher.update(b"|");
        hasher.update(text.as_bytes());
        let hex: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        self.cache_dir.join(format!("{hex}.mp3"))
    }

    /// 朗读 `text`：`En`→Uk、`Zh`→Zh（M1 英文固定取英音）。
    /// 测试开关：环境变量 `PARDON_TTS_DISABLE=1` 时直接 Ok，不下载不播放。
    pub async fn speak(&self, text: &str, lang: Lang) -> anyhow::Result<()> {
        if tts_disabled() {
            return Ok(());
        }
        let voice = match lang {
            Lang::En => Voice::Uk,
            Lang::Zh => Voice::Zh,
        };
        let path = self.cache_path(text, voice);
        let audio = match std::fs::read(&path) {
            // 0 字节视为未命中（崩溃/写满残留），走下载重试
            Ok(cached) if !cached.is_empty() => cached,
            _ => match self.download(&youdao_voice_url(text, voice)).await {
                Ok(bytes) => {
                    // 缓存写失败不阻断播放
                    let _ = self.write_cache(&path, &bytes);
                    bytes
                }
                Err(_) => {
                    // 在线失败：把原文交给合成型播放器（espeak-ng）
                    return self.play_bytes(text.as_bytes());
                }
            },
        };
        self.play_bytes(&audio)
    }

    /// 下载 mp3（client 自带 5s 超时）。
    async fn download(&self, url: &str) -> anyhow::Result<Vec<u8>> {
        let resp = self
            .http
            .get(url)
            .send()
            .await
            .context("dictvoice 请求失败")?;
        let resp = resp.error_for_status().context("dictvoice HTTP 状态错误")?;
        let bytes = resp.bytes().await.context("dictvoice 响应读取失败")?;
        Ok(bytes.to_vec())
    }

    /// 原子写缓存（含建目录）：先写 `.tmp` 再 rename，失败时不会留下
    /// 截断/0 字节的最终文件，下次朗读会重试下载。
    fn write_cache(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.cache_dir)?;
        let tmp = path.with_extension("mp3.tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, path)
    }

    /// 依次尝试播放链，首个 Ok 即返回；全部失败报最后一个错误。
    fn play_bytes(&self, bytes: &[u8]) -> anyhow::Result<()> {
        let mut last_err = None;
        for player in &self.fallback {
            match player.play(bytes) {
                Ok(()) => return Ok(()),
                Err(e) => last_err = Some(e),
            }
        }
        match last_err {
            Some(e) => Err(e.context("所有播放器均失败")),
            None => Err(anyhow::anyhow!("未配置任何播放器")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};

    /// 记录每次 play 收到的字节并返回 Ok。
    struct RecordingPlayer(Arc<Mutex<Vec<Vec<u8>>>>);

    impl AudioPlayer for RecordingPlayer {
        fn play(&self, mp3: &[u8]) -> anyhow::Result<()> {
            self.0.lock().unwrap().push(mp3.to_vec());
            Ok(())
        }
    }

    /// 记录字节但总是 Err（模拟无声卡的 rodio）。
    struct RejectingPlayer(Arc<Mutex<Vec<Vec<u8>>>>);

    impl AudioPlayer for RejectingPlayer {
        fn play(&self, mp3: &[u8]) -> anyhow::Result<()> {
            self.0.lock().unwrap().push(mp3.to_vec());
            anyhow::bail!("cannot play")
        }
    }

    /// 把 dict.youdao.com 解析到 127.0.0.1:1（连接即拒）的 Tts。
    fn unreachable_tts(cache_dir: PathBuf, players: Vec<Box<dyn AudioPlayer>>) -> Tts {
        let addr: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .resolve(YOUDAO_HOST, addr)
            .build()
            .unwrap();
        Tts::with_client(cache_dir, players, http)
    }

    #[test]
    fn tts_disable_switch_is_exact_1() {
        // 纯函数判定：不触碰进程环境（env 是进程全局的，核心单测并行跑，
        // 改 env 会与其他 speak 用例竞态）。真实读取 env 的路径由 CLI 集成
        // 测试（独立子进程 + PARDON_TTS_DISABLE=1）覆盖。
        assert!(!parse_tts_disable(None));
        assert!(!parse_tts_disable(Some("")));
        assert!(!parse_tts_disable(Some("0")));
        assert!(!parse_tts_disable(Some("true")));
        assert!(parse_tts_disable(Some("1")));
    }

    #[test]
    fn voice_url_exact_strings() {
        assert_eq!(
            youdao_voice_url("hello", Voice::Uk),
            "https://dict.youdao.com/dictvoice?audio=hello&type=1"
        );
        assert_eq!(
            youdao_voice_url("hello", Voice::Us),
            "https://dict.youdao.com/dictvoice?audio=hello&type=2"
        );
        assert_eq!(
            youdao_voice_url("你好", Voice::Uk),
            "https://dict.youdao.com/dictvoice?audio=%E4%BD%A0%E5%A5%BD&type=1"
        );
        // Zh：le=zh 且无 type 参数
        assert_eq!(
            youdao_voice_url("你好", Voice::Zh),
            "https://dict.youdao.com/dictvoice?audio=%E4%BD%A0%E5%A5%BD&le=zh"
        );
        assert_eq!(
            youdao_voice_url("hello", Voice::Zh),
            "https://dict.youdao.com/dictvoice?audio=hello&le=zh"
        );
    }

    #[test]
    fn cache_path_stable_distinct_mp3() {
        let dir = tempfile::tempdir().unwrap();
        let tts = Tts::new_with_players(dir.path().to_path_buf(), vec![]);
        let uk = tts.cache_path("hello", Voice::Uk);
        assert_eq!(
            uk,
            tts.cache_path("hello", Voice::Uk),
            "同 text+voice 应稳定"
        );
        assert_eq!(uk.extension().unwrap(), "mp3");
        assert!(uk.starts_with(dir.path()));
        assert_ne!(uk, tts.cache_path("hello", Voice::Us));
        assert_ne!(uk, tts.cache_path("hello", Voice::Zh));
        assert_ne!(uk, tts.cache_path("hellp", Voice::Uk), "不同 text 应不同");
    }

    #[tokio::test]
    async fn speak_cache_hit_plays_cached_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let tts = unreachable_tts(
            dir.path().to_path_buf(),
            vec![Box::new(RecordingPlayer(calls.clone()))],
        );
        std::fs::write(tts.cache_path("hello", Voice::Uk), b"FAKE_MP3").unwrap();

        tts.speak("hello", Lang::En).await.unwrap();

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "应恰好播放一次");
        assert_eq!(calls[0].as_slice(), b"FAKE_MP3".as_slice());
    }

    #[tokio::test]
    async fn speak_download_failure_falls_back_to_text() {
        let dir = tempfile::tempdir().unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let tts = unreachable_tts(
            dir.path().to_path_buf(),
            vec![Box::new(RecordingPlayer(calls.clone()))],
        );

        tts.speak("hello", Lang::En).await.unwrap();

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].as_slice(),
            b"hello".as_slice(),
            "应收到原文而非 mp3"
        );
    }

    #[tokio::test]
    async fn speak_download_failure_falls_through_rejecting_player() {
        let dir = tempfile::tempdir().unwrap();
        let rej = Arc::new(Mutex::new(Vec::new()));
        let ok = Arc::new(Mutex::new(Vec::new()));
        let tts = unreachable_tts(
            dir.path().to_path_buf(),
            vec![
                Box::new(RejectingPlayer(rej.clone())),
                Box::new(RecordingPlayer(ok.clone())),
            ],
        );

        tts.speak("hi", Lang::En).await.unwrap();

        assert_eq!(rej.lock().unwrap().len(), 1, "第一个 player 先被尝试");
        let ok = ok.lock().unwrap();
        assert_eq!(ok.len(), 1);
        assert_eq!(ok[0].as_slice(), b"hi".as_slice());
    }

    #[tokio::test]
    async fn speak_all_players_fail_is_err() {
        let dir = tempfile::tempdir().unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let tts = unreachable_tts(
            dir.path().to_path_buf(),
            vec![Box::new(RejectingPlayer(calls))],
        );
        assert!(tts.speak("hello", Lang::En).await.is_err());
    }

    #[tokio::test]
    async fn speak_maps_lang_to_voice() {
        let dir = tempfile::tempdir().unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let tts = unreachable_tts(
            dir.path().to_path_buf(),
            vec![Box::new(RecordingPlayer(calls.clone()))],
        );
        std::fs::write(tts.cache_path("hello", Voice::Uk), b"UK_MP3").unwrap();
        std::fs::write(tts.cache_path("你好", Voice::Zh), b"ZH_MP3").unwrap();

        tts.speak("hello", Lang::En).await.unwrap();
        tts.speak("你好", Lang::Zh).await.unwrap();

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].as_slice(), b"UK_MP3".as_slice(), "En 应取 Uk 缓存");
        assert_eq!(calls[1].as_slice(), b"ZH_MP3".as_slice(), "Zh 应取 Zh 缓存");
    }

    #[tokio::test]
    async fn zero_byte_cache_file_is_miss_and_retries_download() {
        // 预写 0 字节缓存文件；http 指向不可达地址 → 应视为 miss 走下载（失败）→ 文本兜底
        let dir = tempfile::tempdir().unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let tts = unreachable_tts(
            dir.path().to_path_buf(),
            vec![Box::new(RecordingPlayer(calls.clone()))],
        );
        let path = tts.cache_path("hello", Voice::Uk);
        std::fs::write(&path, b"").unwrap();

        let res = tts.speak("hello", Lang::En).await;

        assert!(res.is_ok(), "文本兜底应成功");
        let calls = calls.lock().unwrap();
        assert_eq!(
            calls[0].as_slice(),
            b"hello".as_slice(),
            "0 字节缓存应视为 miss，player 收到原文而非空字节"
        );
    }

    #[test]
    fn write_cache_creates_missing_dir() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("tts-cache"); // 尚不存在
        let tts = Tts::new_with_players(cache_dir, vec![]);
        let path = tts.cache_path("hello", Voice::Uk);

        tts.write_cache(&path, b"MP3DATA").unwrap();

        assert_eq!(
            std::fs::read(&path).unwrap().as_slice(),
            b"MP3DATA".as_slice()
        );
    }

    #[tokio::test]
    #[ignore = "真实网络+音频设备；手动冒烟：cargo test -p pardon-core tts -- --ignored"]
    async fn real_speak_hello() {
        let dir = tempfile::tempdir().unwrap();
        let tts = Tts::new(dir.path().to_path_buf());
        tts.speak("hello", Lang::En).await.unwrap();
    }
}

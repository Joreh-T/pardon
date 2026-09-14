//! wl-paste / wl-copy 子进程实现（Niri 等 ext-data-control-v1 合成器）。
//!
//! 不用 wl-clipboard-rs 原生 API 的原因：该 crate 0.9 无 watch 模块
//! （2026-09-14 核实，见 docs/m2-wayland-spike.md），事件监听只能依赖
//! `wl-paste --watch`；读写统一走同一套二进制 → 零 wayland 链接依赖，
//! 且可用注入路径的假脚本做无头测试（`with_bin`/`with_bins`）。
//!
//! 帧协议：watch 命令 `sh -c 'base64 -w0; echo'` 保证每个剪贴板事件
//! 恰好输出一行 base64（无内嵌换行歧义）；读本侧解码、lossy 容错。
//!
//! 两个 spike 实证行为（docs/m2-wayland-spike.md，wl-clipboard 2.2.1）：
//! 1. `--watch` 吞掉其后全部 argv → 选项（--type）必须放在 --watch **之前**；
//! 2. watcher 启动即对既有剪贴板内容吐一帧「启动帧」→ 会话起始时间窗内的
//!    非空帧丢弃，窗口外才是真实变更事件。

use crate::{ClipboardAccess, ClipboardMonitor};
use anyhow::{bail, Context};
use base64::Engine as _;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader, Lines};
use tokio::process::{Child, ChildStdout, Command as AsyncCommand};

/// 文本 MIME 候选（按优先级回退）。真机发现（2026-09-14）：WezTerm 等应用
/// 只通告 `text/plain;charset=utf-8`，精确请求 `text/plain` 会「无文本」；
/// wl-copy 通告全套，X11 应用常只有 `UTF8_STRING`。
const TEXT_MIMES: [&str; 3] = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"];
/// watch 命令模板：监听任意类型的变更，命令内按 TEXT_MIMES 回退取文本；
/// 无文本类型时输出空行（被解码层过滤）。每事件恰一行 base64 + '\n'。
/// 注意 `$(...)` 会吞尾部换行——上游本就按 trim 语义处理，无碍。
const WATCH_CMD: &str = "for t in \"text/plain;charset=utf-8\" text/plain UTF8_STRING; do if out=$(wl-paste -t \"$t\" 2>/dev/null); then printf '%s' \"$out\" | base64 -w0; echo; exit 0; fi; done; echo";
/// 启动帧丢弃窗口（毫秒）：watcher 启动后立即回显既有剪贴板内容，实测
/// 亚百毫秒到达；窗口取 300ms 留裕量。窗口内到达的非空帧一律丢弃。
const INITIAL_FRAME_WINDOW_MS: u64 = 300;

/// 在给定目录序列中查找 `name`：必须是带执行位的普通文件（unix 惯例，
/// 与 shell 的 PATH 搜索一致——无 x 位的同名文件会被跳过而非命中）。
/// `find_bin` 的可测内核：测试直接喂目录列表，不动进程 PATH。
fn find_bin_in(dirs: impl IntoIterator<Item = PathBuf>, name: &str) -> Option<PathBuf> {
    for dir in dirs {
        let p = dir.join(name);
        let Ok(md) = std::fs::metadata(&p) else {
            continue;
        };
        if md.is_file() && md.permissions().mode() & 0o111 != 0 {
            return Some(p);
        }
    }
    None
}

fn find_bin(name: &str) -> anyhow::Result<PathBuf> {
    let path = std::env::var_os("PATH").context("PATH is not set")?;
    find_bin_in(std::env::split_paths(&path), name)
        .with_context(|| format!("{name} not found in PATH (install the wl-clipboard package)"))
}

/// 事件文本解码：空行跳过（空剪贴板/图片事件被 --type 过滤后的空输出），
/// 非法 base64 / 非 UTF-8 跳过（返回 None）。
fn decode_event_line(line: &str) -> Option<String> {
    if line.is_empty() {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(line.trim_end())
        .ok()?;
    let text = String::from_utf8(bytes).ok()?;
    Some(text)
}

/// 持久 watcher 会话：跨 next_event 调用复用同一 `wl-paste --watch` 进程
///（每次调用都重启 watcher 会导致启动帧无限循环：spike 发现 2）。
struct MonitorSession {
    child: Child,
    lines: Lines<BufReader<ChildStdout>>,
    started: Instant,
}

pub struct WaylandMonitor {
    wl_paste: PathBuf,
    initial_window: Duration,
    session: Option<MonitorSession>,
}

impl WaylandMonitor {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            wl_paste: find_bin("wl-paste")?,
            initial_window: Duration::from_millis(INITIAL_FRAME_WINDOW_MS),
            session: None,
        })
    }

    /// 测试注入：指定 wl-paste（可执行假脚本）与启动帧丢弃窗口。
    pub fn with_bin(wl_paste: PathBuf, initial_window: Duration) -> Self {
        Self {
            wl_paste,
            initial_window,
            session: None,
        }
    }

    async fn ensure_session(&mut self) -> anyhow::Result<&mut MonitorSession> {
        if self.session.is_none() {
            let mut child = AsyncCommand::new(&self.wl_paste)
                // 参数顺序关键：--watch 吞掉其后全部 argv（spike 发现 1）。
                // 不在外层限定 --type：类型协商交给 WATCH_CMD 内层，避免
                // WezTerm 等只通告 charset 变体的应用被整类过滤。
                .args(["--watch", "sh", "-c", WATCH_CMD])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .with_context(|| format!("spawn {}", self.wl_paste.display()))?;
            let stdout = child.stdout.take().context("piped stdout")?;
            self.session = Some(MonitorSession {
                child,
                lines: BufReader::new(stdout).lines(),
                started: Instant::now(),
            });
        }
        Ok(self.session.as_mut().expect("session ensured above"))
    }
}

#[async_trait::async_trait]
impl ClipboardMonitor for WaylandMonitor {
    async fn next_event(&mut self) -> anyhow::Result<String> {
        let window = self.initial_window;
        loop {
            let session = self.ensure_session().await?;
            match session.lines.next_line().await {
                Ok(Some(line)) => {
                    let Some(text) = decode_event_line(&line) else {
                        continue;
                    };
                    if text.trim().is_empty() {
                        continue;
                    }
                    // 启动帧：会话起始窗口内到达的是既有剪贴板内容回显，丢弃
                    if session.started.elapsed() < window {
                        continue;
                    }
                    return Ok(text);
                }
                Ok(None) | Err(_) => {
                    // watcher 退出（合成器重连等）：终止并丢弃会话，Err 由
                    // 上层（clip.rs 桥）退避后重试——重试会开新会话并重新
                    // 丢弃新会话的启动帧
                    if let Some(mut s) = self.session.take() {
                        let _ = s.child.start_kill();
                    }
                    bail!("wl-paste --watch session ended");
                }
            }
        }
    }
}

pub struct WaylandClipboard {
    wl_paste: PathBuf,
    wl_copy: PathBuf,
}

impl WaylandClipboard {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            wl_paste: find_bin("wl-paste")?,
            wl_copy: find_bin("wl-copy")?,
        })
    }

    /// 测试注入。
    pub fn with_bins(wl_paste: PathBuf, wl_copy: PathBuf) -> Self {
        Self { wl_paste, wl_copy }
    }

    fn read(&self, primary: bool) -> anyhow::Result<String> {
        // 按候选顺序回退请求（见 TEXT_MIMES：WezTerm 只通告 charset 变体）。
        let mut last_err = String::new();
        for mime in TEXT_MIMES {
            let mut cmd = Command::new(&self.wl_paste);
            cmd.args(["--no-newline", "--type", mime]);
            if primary {
                cmd.arg("--primary");
            }
            let out = cmd
                .output()
                .with_context(|| format!("spawn {}", self.wl_paste.display()))?;
            if out.status.success() {
                return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
            }
            last_err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        }
        // 全部候选失败 =「无文本/空剪贴板」或真实错误；daemon 侧按 stderr
        // 关键字（not available as requested type 等，spike 发现 5）
        // 区分为「无文本」而非 503。
        bail!("wl-paste failed (no text type offered): {last_err}")
    }
}

impl ClipboardAccess for WaylandClipboard {
    fn read_clipboard(&self) -> anyhow::Result<String> {
        self.read(false)
    }

    fn read_primary(&self) -> anyhow::Result<String> {
        self.read(true)
    }

    fn write_clipboard(&self, text: &str) -> anyhow::Result<()> {
        let mut child = Command::new(&self.wl_copy)
            .args(["--type", "text/plain"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("spawn {}", self.wl_copy.display()))?;
        // stdin 全量写入后 drop 关闭，让 wl-copy 完成服务
        let mut stdin = child.stdin.take().context("piped stdin")?;
        stdin.write_all(text.as_bytes())?;
        drop(stdin);
        let out = child.wait_with_output()?;
        if !out.status.success() {
            bail!(
                "wl-copy failed ({}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    // 本机内核对「写完脚本立刻 exec」可能返回 ETXTBSY（Text file busy）：
    // 并行测试下实测 ~14% 触发率，且与并发负载正相关（sleep 退避反而更糟，
    // fsync 只能缓解）；单线程实测 0 触发。故把所有「写脚本 → exec」路径
    // 串行化——锁只存在于测试内，生产代码不受影响。
    static EXEC_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// 写一个可执行假脚本；$1 起为透传参数（脚本内容自行匹配）。
    fn fake_bin(dir: &Path, name: &str, script: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    /// find_bin_in 只认带执行位的普通文件：0644 的 wl-paste 不算命中。
    #[test]
    fn find_bin_in_requires_exec_bit() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("wl-paste");
        std::fs::write(&p, "#!/bin/sh\n:\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(find_bin_in([dir.path().to_path_buf()], "wl-paste"), None);
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(find_bin_in([dir.path().to_path_buf()], "wl-paste"), Some(p));
    }

    /// find_bin_in 按序扫描：跳过不含目标的目录，命中后面的目录。
    #[test]
    fn find_bin_in_scans_dirs_in_order() {
        let empty = tempfile::tempdir().unwrap();
        let has = tempfile::tempdir().unwrap();
        let bin = fake_bin(has.path(), "wl-copy", ":");
        assert_eq!(
            find_bin_in(
                [empty.path().to_path_buf(), has.path().to_path_buf()],
                "wl-copy"
            ),
            Some(bin)
        );
    }

    #[test]
    fn read_clipboard_captures_exact_text() {
        let _exec = EXEC_LOCK.blocking_lock();
        let dir = tempfile::tempdir().unwrap();
        let paste = fake_bin(
            dir.path(),
            "wl-paste",
            r#"printf 'hello world'; printf 'no-newline set' >&2"#,
        );
        let copy = fake_bin(dir.path(), "wl-copy", ":");
        let c = WaylandClipboard::with_bins(paste, copy);
        assert_eq!(c.read_clipboard().unwrap(), "hello world");
    }

    #[test]
    fn read_primary_appends_primary_flag() {
        let _exec = EXEC_LOCK.blocking_lock();
        let dir = tempfile::tempdir().unwrap();
        // 假脚本把收到的参数写进输出，验证 --primary 确实传了
        let paste = fake_bin(
            dir.path(),
            "wl-paste",
            r#"case " $* " in *" --primary "*) printf 'PRIMARY';; *) printf 'REGULAR';; esac"#,
        );
        let copy = fake_bin(dir.path(), "wl-copy", ":");
        let c = WaylandClipboard::with_bins(paste, copy);
        assert_eq!(c.read_primary().unwrap(), "PRIMARY");
        assert_eq!(c.read_clipboard().unwrap(), "REGULAR");
    }

    #[test]
    fn read_failure_carries_stderr() {
        let _exec = EXEC_LOCK.blocking_lock();
        let dir = tempfile::tempdir().unwrap();
        let paste = fake_bin(
            dir.path(),
            "wl-paste",
            r#"echo 'No suitable type of copied clipboard' >&2; exit 1"#,
        );
        let copy = fake_bin(dir.path(), "wl-copy", ":");
        let c = WaylandClipboard::with_bins(paste, copy);
        let e = c.read_clipboard().unwrap_err();
        let msg = format!("{e:#}");
        assert!(msg.contains("No suitable type"), "msg: {msg}");
    }

    /// WezTerm 式通告：charset 变体报错 → 回退到 text/plain 命中。
    #[test]
    fn read_falls_back_across_text_mimes() {
        let _exec = EXEC_LOCK.blocking_lock();
        let dir = tempfile::tempdir().unwrap();
        let paste = fake_bin(
            dir.path(),
            "wl-paste",
            r#"case " $* " in *"charset=utf-8"*) echo 'nope' >&2; exit 1;; *) printf 'via-plain';; esac"#,
        );
        let copy = fake_bin(dir.path(), "wl-copy", ":");
        let c = WaylandClipboard::with_bins(paste, copy);
        assert_eq!(c.read_clipboard().unwrap(), "via-plain");
    }

    /// X11 兼容通告：前两个候选失败，UTF8_STRING 命中。
    #[test]
    fn read_falls_back_to_x11_string_mime() {
        let _exec = EXEC_LOCK.blocking_lock();
        let dir = tempfile::tempdir().unwrap();
        let paste = fake_bin(
            dir.path(),
            "wl-paste",
            r#"case " $* " in *"charset=utf-8"*|*" text/plain "*) exit 1;; *) printf 'via-x11';; esac"#,
        );
        let copy = fake_bin(dir.path(), "wl-copy", ":");
        let c = WaylandClipboard::with_bins(paste, copy);
        assert_eq!(c.read_clipboard().unwrap(), "via-x11");
    }

    #[test]
    fn write_pipes_text_to_stdin() {
        let _exec = EXEC_LOCK.blocking_lock();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("copied.txt");
        let out_str = out.display().to_string();
        let paste = fake_bin(dir.path(), "wl-paste", ":");
        let copy = fake_bin(dir.path(), "wl-copy", &format!("cat > {out_str}"));
        let c = WaylandClipboard::with_bins(paste, copy);
        c.write_clipboard("你好\nsecond line").unwrap();
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "你好\nsecond line");
    }

    #[tokio::test]
    async fn monitor_skips_initial_frame_and_streams_changes() {
        let _exec = EXEC_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        // 单进程假 watcher（持久会话）：启动帧 hello 立即输出（落入 150ms 窗口，
        // 应被丢弃——窗口取宽以容忍慢机上 spawn→echo 的延迟；帧只会被延迟读取，
        // 延迟只会把后续帧推出窗外，方向安全）；窗口后两帧真实变更（你好 / world）
        let paste = fake_bin(
            dir.path(),
            "wl-paste",
            r#"echo 'aGVsbG8='; sleep 0.3; echo '5L2g5aW9'; sleep 0.1; echo 'd29ybGQ='; sleep 60"#,
        );
        let mut m = WaylandMonitor::with_bin(paste, std::time::Duration::from_millis(150));
        // 启动帧被丢弃；同一进程跨两次调用持续供帧（持久会话）
        assert_eq!(m.next_event().await.unwrap(), "你好");
        assert_eq!(m.next_event().await.unwrap(), "world");
    }

    #[tokio::test]
    async fn monitor_session_end_returns_err() {
        let _exec = EXEC_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        // 假 watcher 供完一帧即退出：EOF → Err（上层退避重启语义）
        let paste = fake_bin(
            dir.path(),
            "wl-paste",
            r#"echo 'aGVsbG8='; sleep 0.3; echo '5L2g5aW9'; exit 0"#,
        );
        let mut m = WaylandMonitor::with_bin(paste, std::time::Duration::from_millis(150));
        assert_eq!(m.next_event().await.unwrap(), "你好");
        assert!(m.next_event().await.is_err());
    }
}

//! 平台能力抽象（spec §4.1）：剪贴板监听/读写、通知。
//! trait 是跨平台契约；实现按平台分模块（当前仅 linux）。

use async_trait::async_trait;

#[cfg(target_os = "linux")]
pub mod linux;

/// 剪贴板文本变化事件源（事件驱动，非轮询，spec §3.4）。
#[async_trait]
pub trait ClipboardMonitor: Send {
    /// 等待下一个文本事件。非文本/空内容由实现内部跳过（继续等待下一个
    /// 事件）；仅在监听通道致命错误时返回 Err（调用方负责退避重启）。
    async fn next_event(&mut self) -> anyhow::Result<String>;
}

/// 剪贴板与 primary selection 的同步读写（调用方在异步上下文中应经
/// `spawn_blocking` 调用，单次开销毫秒级）。
pub trait ClipboardAccess: Send + Sync {
    fn read_clipboard(&self) -> anyhow::Result<String>;
    fn read_primary(&self) -> anyhow::Result<String>;
    fn write_clipboard(&self, text: &str) -> anyhow::Result<()>;
}

/// 桌面通知输出（mako/dunst 等实现了 freedesktop 通知服务的环境）。
/// M3 的 GUI 弹窗将以另一个实现替换此 trait。
pub trait Notifier: Send + Sync {
    fn notify(&self, summary: &str, body: &str) -> anyhow::Result<()>;
}

/// 通知后端：仅写日志（headless 调试 / `pardond --log-notify`）。
pub struct LogNotifier;

impl Notifier for LogNotifier {
    fn notify(&self, summary: &str, body: &str) -> anyhow::Result<()> {
        log::info!("[notify] {summary}\n{body}");
        Ok(())
    }
}

/// 平台能力不可用时的占位：所有读/写返回构造时给定的错误
/// （daemon 在无 Wayland/无 wl-clipboard 环境下降级运行，触发口返回 503）。
pub struct UnavailableClipboard {
    pub reason: String,
}

impl ClipboardAccess for UnavailableClipboard {
    fn read_clipboard(&self) -> anyhow::Result<String> {
        anyhow::bail!("{}", self.reason)
    }
    fn read_primary(&self) -> anyhow::Result<String> {
        anyhow::bail!("{}", self.reason)
    }
    fn write_clipboard(&self, _text: &str) -> anyhow::Result<()> {
        anyhow::bail!("{}", self.reason)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_notifier_always_succeeds() {
        assert!(LogNotifier.notify("s", "b").is_ok());
    }

    #[test]
    fn unavailable_clipboard_errors_with_reason() {
        let c = UnavailableClipboard {
            reason: "no wayland".into(),
        };
        let e = c.read_clipboard().unwrap_err();
        assert!(format!("{e:#}").contains("no wayland"));
        assert!(c.read_primary().is_err());
        assert!(c.write_clipboard("x").is_err());
    }
}

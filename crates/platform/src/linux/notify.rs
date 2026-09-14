//! freedesktop 桌面通知（D-Bus，经 notify-rust；mako/dunst 等服务）。
//! 注意：该实现依赖会话内存在通知服务，无法无头单测；
//! 验证路径 = Task 12 README 手动测试矩阵（`pardond --log-notify` 对照）。

use crate::Notifier;
use notify_rust::{Notification, Timeout};

pub struct DesktopNotifier {
    timeout_ms: u32,
}

impl DesktopNotifier {
    pub fn new(timeout_ms: u32) -> Self {
        Self { timeout_ms }
    }
}

impl Notifier for DesktopNotifier {
    fn notify(&self, summary: &str, body: &str) -> anyhow::Result<()> {
        Notification::new()
            .summary(summary)
            .body(body)
            .timeout(Timeout::Milliseconds(self.timeout_ms))
            .show()
            .map(|_| ())
            .map_err(|e| anyhow::anyhow!("desktop notification failed: {e}"))
    }
}

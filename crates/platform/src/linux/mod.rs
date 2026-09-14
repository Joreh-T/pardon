//! Linux 实现：Wayland 子进程剪贴板 + freedesktop 通知。

pub mod notify;
pub mod wayland;

pub use notify::DesktopNotifier;
pub use wayland::{WaylandClipboard, WaylandMonitor};

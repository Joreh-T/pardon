//! Linux 实现：Wayland 子进程剪贴板 + freedesktop 通知。

pub mod notify;

pub use notify::DesktopNotifier;

//! UDS IPC 套接字路径（daemon 与 gui 各自实现同一约定；协议 JSON 是契约，
//! 改路径规则需两侧同步）。测试与调试用 `PARDON_IPC_SOCK` 覆盖。

use std::path::PathBuf;

pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("PARDON_IPC_SOCK") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(base) if !base.is_empty() => PathBuf::from(base).join("pardon").join("ipc.sock"),
        _ => std::env::temp_dir().join("pardon").join("ipc.sock"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    static SOCK_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn env_override_wins() {
        let _l = SOCK_LOCK.lock().unwrap();
        std::env::set_var("PARDON_IPC_SOCK", "/tmp/my.sock");
        assert_eq!(socket_path(), PathBuf::from("/tmp/my.sock"));
        std::env::set_var("PARDON_IPC_SOCK", "");
        std::env::remove_var("PARDON_IPC_SOCK");
    }

    #[test]
    fn default_under_xdg_runtime_dir() {
        let _l = SOCK_LOCK.lock().unwrap();
        std::env::remove_var("PARDON_IPC_SOCK");
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
        assert_eq!(
            socket_path(),
            PathBuf::from("/run/user/1000/pardon/ipc.sock")
        );
        std::env::remove_var("XDG_RUNTIME_DIR");
    }

    /// 空 PARDON_IPC_SOCK 视为未设置（契约：非空才覆盖）。
    #[test]
    fn empty_override_falls_back_to_xdg() {
        let _l = SOCK_LOCK.lock().unwrap();
        std::env::set_var("PARDON_IPC_SOCK", "");
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
        assert_eq!(
            socket_path(),
            PathBuf::from("/run/user/1000/pardon/ipc.sock")
        );
        std::env::remove_var("PARDON_IPC_SOCK");
        std::env::remove_var("XDG_RUNTIME_DIR");
    }
}

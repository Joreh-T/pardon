//! `pardon daemon start|stop`：无 systemd 环境的进程管理（推荐 systemd，
//! 见 README M2 章节）。

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// pardond 应与 pardon 同目录安装。
pub fn daemon_bin_for(pardon_exe: &Path) -> PathBuf {
    pardon_exe
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("pardond")
}

/// 后台启动 pardond：日志追加到 log_path，独立进程组（终端关闭不牵连）。
pub fn start_bin(bin: &Path, log_path: &Path) -> anyhow::Result<u32> {
    if !bin.exists() {
        anyhow::bail!(
            "pardond not found at {} (install it next to pardon)",
            bin.display()
        );
    }
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    let child = Command::new(bin)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .process_group(0)
        .spawn()?;
    Ok(child.id())
}

pub async fn run(action: &str, addr: std::net::SocketAddr) -> anyhow::Result<i32> {
    match action {
        "start" => {
            let exe = std::env::current_exe()?;
            let bin = daemon_bin_for(&exe);
            let log = pardon_core::pipeline::pardon_home()?
                .join("log")
                .join("pardond.log");
            let pid = start_bin(&bin, &log)?;
            println!("pardond started (pid {}); log: {}", pid, log.display());
            println!("tip: a systemd user service is the recommended way (see README)");
            Ok(0)
        }
        "stop" => {
            let url = format!("http://{addr}/shutdown");
            match reqwest::Client::new().post(&url).send().await {
                Ok(_) => {
                    println!("pardond stopped");
                    Ok(0)
                }
                Err(_) => {
                    println!("pardond is not running");
                    Ok(0)
                }
            }
        }
        other => anyhow::bail!("unknown action {other:?}, expected start|stop"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_bin_is_sibling_of_pardon() {
        let p = daemon_bin_for(Path::new("/home/u/.cargo/bin/pardon"));
        assert_eq!(p, PathBuf::from("/home/u/.cargo/bin/pardond"));
    }

    #[test]
    fn start_bin_missing_binary_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let e = start_bin(&dir.path().join("pardond"), &dir.path().join("l.log")).unwrap_err();
        assert!(format!("{e:#}").contains("pardond not found"));
    }
}

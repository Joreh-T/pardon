//! ksni 系统托盘（纯 D-Bus，spec §4.2）。回调只发命令到 channel，
//! 异步动作（watcher 开关/持久化/关停）由 [`spawn_tray`] 的命令循环
//! 执行——回调闭包是同步的，不假设运行时上下文。

use crate::events::{Event, ShowWindowKind};
use crate::state::DaemonState;
use crate::watcher;
use std::sync::Arc;
use tokio::sync::mpsc;

/// 托盘菜单 → 命令循环的命令。
pub enum TrayCmd {
    Open(ShowWindowKind),
    ToggleAuto(bool),
    Quit,
}

/// 托盘「打开窗口」决策：有 GUI 订阅者 → 事件；否则 spawn pardon-gui。
enum OpenPlan {
    Event,
    Spawn,
}

fn open_window_plan(gui_connected: bool) -> OpenPlan {
    if gui_connected {
        OpenPlan::Event
    } else {
        OpenPlan::Spawn
    }
}

/// ksni 托盘。`auto`（复制即翻译勾选态）由命令循环 [`spawn_tray`] 在
/// ToggleAuto 处理后回写，回调自身不读共享状态。
pub struct PardonTray {
    pub auto: bool,
    tx: mpsc::UnboundedSender<TrayCmd>,
}

impl ksni::Tray for PardonTray {
    fn id(&self) -> String {
        "pardon".into()
    }
    fn title(&self) -> String {
        "pardon".into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![tray_icon()]
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        vec![
            StandardItem {
                label: "打开主窗口".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.send(TrayCmd::Open(ShowWindowKind::Main));
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "设置".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.send(TrayCmd::Open(ShowWindowKind::Settings));
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            CheckmarkItem {
                label: "复制即翻译".into(),
                checked: self.auto,
                activate: Box::new(|t: &mut Self| {
                    t.auto = !t.auto;
                    let _ = t.tx.send(TrayCmd::ToggleAuto(t.auto));
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "退出 pardond".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.send(TrayCmd::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// 处理一条托盘命令（公开给测试）：开关 watcher + 持久化 + 开窗/退出。
pub async fn handle_cmd(state: &Arc<DaemonState>, cmd: TrayCmd) {
    match cmd {
        TrayCmd::Open(kind) => open_window(state, kind),
        TrayCmd::ToggleAuto(v) => {
            // 顺序：watcher 开关（失败仅 warn 不阻断）→ 配置持久化（失败
            // warn）→ 内存 cfg 快照生效
            if let Err(e) = watcher::set_enabled(state, v).await {
                log::warn!("tray toggle watcher: {e:#}");
            }
            if let Err(e) = pardon_core::config::set_bool(
                &pardon_core::config::config_path(),
                Some("daemon"),
                "auto_translate",
                v,
            ) {
                log::warn!("tray persist auto_translate: {e:#}");
            }
            state
                .cfg
                .write()
                .expect("cfg lock poisoned")
                .daemon
                .auto_translate = v;
        }
        // 两个等待者（axum graceful shutdown、uds::serve）都要醒：与
        // http post_shutdown 同一组合——单 notify_one 会随机饿死一个 waiter
        TrayCmd::Quit => {
            state.shutdown.notify_waiters();
            state.shutdown.notify_one();
        }
    }
}

fn open_window(state: &DaemonState, kind: ShowWindowKind) {
    match open_window_plan(state.events.receiver_count() > 0) {
        OpenPlan::Event => {
            if let Ok(line) = (Event::ShowWindow { kind }).to_line() {
                let _ = state.events.send(line);
            }
        }
        OpenPlan::Spawn => {
            let arg = match kind {
                ShowWindowKind::Main => "--main",
                ShowWindowKind::Settings => "--settings",
            };
            match std::process::Command::new("pardon-gui").arg(arg).spawn() {
                Ok(_) => log::info!("spawned pardon-gui {arg}"),
                Err(e) => log::warn!("spawn pardon-gui failed: {e} (is it installed?)"),
            }
        }
    }
}

/// 托盘图标：嵌入 PNG → RGBA → ksni ARGB32（网络字节序）。解码一次缓存
/// （[`ksni::Icon`] 实现 Clone，属性查询每次 clone 出走）。
fn tray_icon() -> ksni::Icon {
    static ICON: std::sync::OnceLock<ksni::Icon> = std::sync::OnceLock::new();
    ICON.get_or_init(|| {
        let img = image::load_from_memory_with_format(
            include_bytes!("../../../assets/pardon-tray.png"),
            image::ImageFormat::Png,
        )
        .expect("embedded tray icon decodes")
        .to_rgba8();
        let (w, h) = (img.width() as usize, img.height() as usize);
        // ksni Icon::data 为 ARGB32 网络字节序：RGBA 逐像素转为 ARGB
        let mut argb = Vec::with_capacity(w * h * 4);
        for px in img.chunks_exact(4) {
            argb.extend_from_slice(&[px[3], px[0], px[1], px[2]]);
        }
        ksni::Icon {
            width: w as i32,
            height: h as i32,
            data: argb,
        }
    })
    .clone()
}

/// 启动托盘 + 命令循环（lib.rs 调用）。托盘服务失败只告警（headless /
/// 无 D-Bus 时降级，daemon 其余功能不受影响）。
pub async fn spawn_tray(state: Arc<DaemonState>) {
    let (tx, mut rx) = mpsc::unbounded_channel::<TrayCmd>();
    let tray = PardonTray {
        auto: state.cfg_snapshot().daemon.auto_translate,
        tx,
    };
    use ksni::TrayMethods;
    let handle = match tray.spawn().await {
        Ok(h) => h,
        Err(e) => {
            log::warn!("tray unavailable: {e:#}");
            return;
        }
    };
    log::info!("tray ready");
    while let Some(cmd) = rx.recv().await {
        if let TrayCmd::ToggleAuto(v) = &cmd {
            // 菜单勾选态以命令循环处理后的事务结果为准
            let _ = handle.update(move |t: &mut PardonTray| t.auto = *v).await;
        }
        handle_cmd(&state, cmd).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::DaemonState;
    use crate::testing::CONFIG_ENV_LOCK;
    use crate::testing::{FakeClipboard, FakeNotifier, FakeTranslator};
    use pardon_core::config::AppConfig;
    use std::sync::Arc;

    /// 字面构造 DaemonState（无辅助构造器，照 watcher.rs tests 模式）。
    fn state_with_cfg(cfg: AppConfig) -> Arc<DaemonState> {
        Arc::new(DaemonState {
            guard: tokio::sync::Mutex::new(pardon_core::loopguard::LoopGuard::new(
                std::time::Duration::from_millis(cfg.daemon.dedup_window_ms),
            )),
            cfg: std::sync::RwLock::new(cfg),
            started: std::time::Instant::now(),
            translator: Arc::new(FakeTranslator::new()),
            notifier: Arc::new(FakeNotifier::default()),
            clipboard: Arc::new(FakeClipboard::default()),
            counters: Default::default(),
            clipboard_watching: std::sync::atomic::AtomicBool::new(false),
            shutdown: Arc::new(tokio::sync::Notify::new()),
            watcher_stop: tokio::sync::watch::channel(false).0,
            history: None,
            events: tokio::sync::broadcast::channel(64).0,
        })
    }

    /// PARDON_CONFIG 是进程级全局 → 与 state.rs 的 apply_config 测试共用
    /// 一把锁串行化（见 testing::CONFIG_ENV_LOCK）。
    #[tokio::test]
    async fn toggle_auto_updates_cfg_and_persists() {
        let _lock = CONFIG_ENV_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("config.toml");
        // 初始 true + ToggleAuto(false)：磁盘值翻转为 false 才证明写盘发生
        std::fs::write(&cfg_path, "[daemon]\nauto_translate = true\n").unwrap();
        std::env::set_var("PARDON_CONFIG", &cfg_path);
        let mut cfg = AppConfig::default();
        cfg.daemon.auto_translate = true;
        let state = state_with_cfg(cfg);
        handle_cmd(&state, TrayCmd::ToggleAuto(false)).await;
        assert!(!state.cfg_snapshot().daemon.auto_translate);
        let on_disk =
            pardon_core::config::load_from_str(&std::fs::read_to_string(&cfg_path).unwrap())
                .unwrap();
        assert!(!on_disk.daemon.auto_translate, "配置文件已持久化");
        std::env::remove_var("PARDON_CONFIG");
    }

    #[test]
    fn open_window_prefers_subscriber_else_spawns() {
        assert!(matches!(open_window_plan(true), OpenPlan::Event));
        assert!(matches!(open_window_plan(false), OpenPlan::Spawn));
    }
}

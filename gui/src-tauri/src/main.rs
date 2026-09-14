//! pardon-gui：Tauri 壳。纯 IPC 客户端——所有翻译/词典/历史数据经
//! UDS 向 pardond 请求（spec §4.1：GUI 只是可替换客户端）。

mod ipc;

use serde::{Deserialize, Serialize};
use tauri::Listener;
use tauri::Manager;

/// GUI 本地状态（弹窗位置记忆等）；存储于 `<data_dir>/pardon/gui-state.json`
/// （`PARDON_HOME` 非空时覆盖数据根，与 pardon_core 的数据根规则一致）。
#[derive(Serialize, Deserialize, Default)]
struct GuiState {
    popup_x: Option<i32>,
    popup_y: Option<i32>,
}

impl GuiState {
    /// 可恢复的弹窗位置：两坐标都已记忆才算有（半记忆视同未记忆）。
    fn into_pos(self) -> Option<(i32, i32)> {
        Some((self.popup_x?, self.popup_y?))
    }
}

fn gui_state_path() -> std::path::PathBuf {
    if let Some(home) = std::env::var_os("PARDON_HOME") {
        if !home.is_empty() {
            return std::path::PathBuf::from(home).join("gui-state.json");
        }
    }
    dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("pardon")
        .join("gui-state.json")
}

/// 读取失败（无文件/损坏）视同默认状态：位置记忆是纯增强，不报错。
fn read_gui_state() -> GuiState {
    std::fs::read_to_string(gui_state_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[tauri::command]
fn popup_pos_load() -> Option<(i32, i32)> {
    read_gui_state().into_pos()
}

#[tauri::command]
fn popup_pos_save(x: i32, y: i32) -> Result<(), String> {
    let mut s = read_gui_state();
    s.popup_x = Some(x);
    s.popup_y = Some(y);
    if let Some(parent) = gui_state_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(
        gui_state_path(),
        serde_json::to_string(&s).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

/// 隐藏弹窗（Esc / JS 侧触发）：hide 前记住当前窗口位置（物理像素）。
#[tauri::command]
fn hide_popup(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("popup") {
        if let Ok(pos) = w.outer_position() {
            let _ = popup_pos_save(pos.x, pos.y);
        }
        let _ = w.hide();
    }
}

/// 弹窗高度自适应（popup.ts 按内容量申请）；宽度固定 420，高度夹在
/// 120–480（LogicalSize：与 HiDPI 无关的布局像素）。
#[tauri::command]
fn popup_resize(app: tauri::AppHandle, height: f64) {
    if let Some(w) = app.get_webview_window("popup") {
        let h = height.clamp(120.0, 480.0);
        let _ = w.set_size(tauri::LogicalSize::new(420.0, h));
    }
}

#[tauri::command]
async fn ipc_request(
    method: String,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    ipc::request(&method, params).await
}

#[tauri::command]
async fn speak(text: String) -> Result<(), String> {
    std::process::Command::new("pardon")
        .arg("speak")
        .arg(&text)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("spawn `pardon speak`: {e}"))
}

/// 兜底启动：主窗口检测到未连接 pardond 时显示按钮，经 CLI 分离启动
/// （`pardon daemon start` 内部 spawn 独立进程组后立即返回，不会挂起）。
#[tauri::command]
async fn pardon_daemon_start() -> Result<String, String> {
    let out = std::process::Command::new("pardon")
        .args(["daemon", "start"])
        .output()
        .map_err(|e| format!("spawn `pardon daemon start`: {e}"))?;
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

#[tauri::command]
fn open_settings(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.set_focus();
        Ok(())
    } else {
        create_settings_window(&app).map_err(|e| e.to_string())
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
            if args.iter().any(|a| a == "--settings") {
                if let Some(w) = app.get_webview_window("settings") {
                    let _ = w.show();
                    let _ = w.set_focus();
                } else {
                    let _ = create_settings_window(app);
                }
            }
        }))
        .invoke_handler(tauri::generate_handler![
            ipc_request,
            speak,
            pardon_daemon_start,
            open_settings,
            hide_popup,
            popup_resize,
            popup_pos_load,
            popup_pos_save
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                ipc::subscribe_loop(handle).await;
            });
            // 预创建隐藏的弹窗窗口（事件到达即 show，避免冷启动闪烁）
            tauri::WebviewWindowBuilder::new(
                app,
                "popup",
                tauri::WebviewUrl::App("popup.html".into()),
            )
            .title("pardon")
            .decorations(false)
            .skip_taskbar(true)
            .always_on_top(true)
            .resizable(false)
            .visible(false)
            .inner_size(420.0, 300.0)
            .build()?;
            // daemon 的 popup 事件已由 ipc.rs 转发为同名 tauri 事件（JS 侧
            // popup.ts 监听渲染内容），Rust 侧只做窗口动作：
            // 恢复记忆位置（若有）→ show → focus。
            let popup_handle = app.handle().clone();
            app.listen("popup", move |_event| {
                let Some(w) = popup_handle.get_webview_window("popup") else {
                    return;
                };
                if let Some((x, y)) = read_gui_state().into_pos() {
                    let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
                }
                let _ = w.show();
                let _ = w.set_focus();
            });
            if std::env::args().any(|a| a == "--settings") {
                create_settings_window(app.handle())?;
            }
            Ok(())
        })
        // 弹窗失焦自动隐藏；hide 前同样尝试记位（与 hide_popup 命令幂等，
        // 两条路径互补：Esc 走命令、点别处走这里）
        .on_window_event(|window, event| {
            if window.label() == "popup" {
                if let tauri::WindowEvent::Focused(false) = event {
                    if let Ok(pos) = window.outer_position() {
                        let _ = popup_pos_save(pos.x, pos.y);
                    }
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running pardon-gui");
}

/// 创建设置窗口（单实例回调 / --settings / 主窗口按钮共用）。
pub fn create_settings_window(app: &tauri::AppHandle) -> tauri::Result<()> {
    tauri::WebviewWindowBuilder::new(
        app,
        "settings",
        tauri::WebviewUrl::App("settings.html".into()),
    )
    .title("pardon 设置")
    .inner_size(560.0, 640.0)
    .resizable(false)
    .build()?;
    Ok(())
}

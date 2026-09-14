//! pardon-gui：Tauri 壳。纯 IPC 客户端——所有翻译/词典/历史数据经
//! UDS 向 pardond 请求（spec §4.1：GUI 只是可替换客户端）。

mod ipc;

use tauri::Manager;

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
            open_settings
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
            if std::env::args().any(|a| a == "--settings") {
                create_settings_window(app.handle())?;
            }
            Ok(())
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

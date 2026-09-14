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

/// 最近一条 popup 事件负载。webview 冷启动竞态（Rust 侧 listen 先于 JS
/// 的 DOMContentLoaded 注册监听）期间到达的事件会被 JS 错过——先落缓存，
/// webview 就绪后经 [`popup_last`] 重放（webview 整个 GUI 生命周期只
/// 加载一次，不存在「旧内容复活」问题）。
pub struct PopupCache(pub std::sync::Mutex<Option<serde_json::Value>>);

/// 冷启动重放：popup.ts 就绪时拉取竞态窗口内错过的 popup 负载。
#[tauri::command]
fn popup_last(cache: tauri::State<PopupCache>) -> Option<serde_json::Value> {
    cache.0.lock().unwrap().clone()
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

/// 配置文件路径：`PARDON_CONFIG`（非空）优先，缺省
/// `~/.config/pardon/config.toml`（与 pardon_core 的 config_path 规则一致；
/// GUI 不依赖 core，此文件按协议复制路径规则）。
fn gui_config_path() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("PARDON_CONFIG") {
        if !p.is_empty() {
            return std::path::PathBuf::from(p);
        }
    }
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("pardon")
        .join("config.toml")
}

/// 读配置文本 → 脱敏 JSON（供 [`read_config`] 与测试共用）。
/// 脱敏：`llm.providers[]` 只保留 id/type/base_url/model + `has_api_key`
/// 存在性标志——**api_key/api_key_env 与模板正文绝不出现在输出**。
/// 文件不存在/解析失败视同空配置（设置页展示默认值，不报错）。
fn parse_sanitized_config(text: &str) -> Result<serde_json::Value, String> {
    let mut v: serde_json::Value = toml::from_str(text).unwrap_or(serde_json::json!({}));
    if let Some(providers) = v
        .get_mut("llm")
        .and_then(|l| l.get_mut("providers"))
        .and_then(|p| p.as_array_mut())
    {
        for p in providers.iter_mut() {
            let obj = p.as_object_mut().ok_or("provider not an object")?;
            let has_key = obj.get("api_key").is_some() || obj.get("api_key_env").is_some();
            let mut filtered = serde_json::Map::new();
            for (k, val) in obj.iter() {
                if matches!(k.as_str(), "id" | "type" | "base_url" | "model") {
                    filtered.insert(k.clone(), val.clone());
                }
            }
            filtered.insert("has_api_key".into(), serde_json::json!(has_key));
            *obj = filtered;
        }
    }
    Ok(v)
}

/// 读配置 → JSON（脱敏见 [`parse_sanitized_config`]）。
#[tauri::command]
fn read_config() -> Result<serde_json::Value, String> {
    let text = std::fs::read_to_string(gui_config_path()).unwrap_or_default();
    parse_sanitized_config(&text)
}

/// 可写键白名单：(键, 表)；`None` 为根表。守护 api_key 等敏感字段
/// 不被 GUI 的覆写路径触碰——白名单外的键一律拒绝。
const WRITABLE: [(&str, Option<&str>); 8] = [
    ("auto_translate", Some("daemon")),
    ("popup", Some("daemon")),
    ("show_word_badge", Some("daemon")),
    ("copy_translation", Some("daemon")),
    ("notify_timeout_ms", Some("daemon")),
    ("max_text_bytes", Some("daemon")),
    ("dedup_window_ms", Some("daemon")),
    ("default_engine", None),
];

fn json_to_toml(v: serde_json::Value) -> Result<toml_edit::Value, String> {
    use serde_json::Value as J;
    match v {
        J::Bool(b) => Ok(b.into()),
        J::Number(n) => n
            .as_i64()
            .map(|i| i.into())
            .or_else(|| n.as_f64().map(|f| f.into()))
            .ok_or_else(|| "unsupported number".to_string()),
        J::String(s) => Ok(s.into()),
        _ => Err("only bool/number/string values are writable".to_string()),
    }
}

/// 原位写单个值（toml_edit，保留注释；白名单校验；表/父目录缺失则创建）。
/// 与 pardon_core::config::set_value 同构（gui 有意不依赖 core）。
fn set_config_value(
    path: &std::path::Path,
    table: Option<&str>,
    key: &str,
    value: serde_json::Value,
) -> Result<(), String> {
    if !WRITABLE.iter().any(|(k, t)| *k == key && *t == table) {
        return Err(format!("refusing to set {key:?} in {table:?}: not writable from gui"));
    }
    // 读不到（含文件不存在）→ 空文档起步
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("parse config: {e}"))?;
    let target: &mut dyn toml_edit::TableLike = match table {
        Some(name) => doc
            .entry(name)
            .or_insert_with(|| {
                // 缺表建表：implicit 使纯新增场景不额外打印表头歧义
                let mut t = toml_edit::Table::new();
                t.set_implicit(true);
                toml_edit::Item::Table(t)
            })
            .as_table_mut()
            .ok_or("section is not a table")?,
        None => doc.as_table_mut(),
    };
    // 已有键且为标量值项：旧值的 decor（行尾注释/前后空白）搬到新值上再
    // 覆盖——insert 是整项替换，直接插新值会丢掉行尾注释（与
    // pardon_core::config::set_value 同构，core 侧 TDD 实证过的坑）。
    let mut new_val = json_to_toml(value)?;
    if let Some(item) = target.get_mut(key) {
        if let Some(old) = item.as_value_mut() {
            std::mem::swap(old.decor_mut(), new_val.decor_mut());
        }
    }
    target.insert(key, toml_edit::Item::Value(new_val));
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    std::fs::write(path, doc.to_string()).map_err(|e| e.to_string())
}

/// 原位写单个值（白名单见 [`WRITABLE`]；`table=None` 为根表）。
#[tauri::command]
fn config_set(
    table: Option<String>,
    key: String,
    value: serde_json::Value,
) -> Result<(), String> {
    set_config_value(&gui_config_path(), table.as_deref(), &key, value)
}

/// 重启 pardond：stop（忽略结果——可能本就没在跑）→ 稍候 → start
/// 并检查退出码与 stderr（引擎字段改动生效路径）。
#[tauri::command]
async fn restart_daemon() -> Result<(), String> {
    let _ = std::process::Command::new("pardon").args(["daemon", "stop"]).output();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let out = std::process::Command::new("pardon")
        .args(["daemon", "start"])
        .output()
        .map_err(|e| format!("spawn pardon daemon start: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).to_string());
    }
    Ok(())
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
            read_config,
            config_set,
            restart_daemon,
            hide_popup,
            popup_resize,
            popup_pos_load,
            popup_pos_save,
            popup_last
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
            // 恢复记忆位置（若有）→ show → focus。负载先落 PopupCache——
            // webview 冷启动竞态窗口内错过的事件由 popup_last 重放。
            app.manage(PopupCache(std::sync::Mutex::new(None)));
            let popup_handle = app.handle().clone();
            app.listen("popup", move |event| {
                if let Ok(params) = serde_json::from_str::<serde_json::Value>(event.payload()) {
                    if let Some(cache) = popup_handle.try_state::<PopupCache>() {
                        *cache.0.lock().unwrap() = Some(params);
                    }
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// PARDON_CONFIG 是进程级全局 → 涉及它的测试持锁串行（gui 进程内
    /// 只有本模块改它；与 ipc.rs 的 PARDON_IPC_SOCK 互不相干）。
    static CONFIG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    // ── read_config 脱敏 ────────────────────────────────────────────

    #[test]
    fn sanitized_config_strips_key_material_and_prompt_bodies() {
        let text = r#"
default_engine = "llm"

[llm]
default_provider = "glm"

[[llm.providers]]
id = "glm"
type = "openai"
base_url = "https://api.example.com"
model = "glm-4.7"
api_key = "sk-secret-value"
system_prompt = "You translate"
user_prompt_template = "{text}"

[daemon]
auto_translate = true
"#;
        let v = parse_sanitized_config(text).unwrap();
        let p = &v["llm"]["providers"][0];
        assert_eq!(p["id"], "glm");
        assert_eq!(p["type"], "openai");
        assert_eq!(p["base_url"], "https://api.example.com");
        assert_eq!(p["model"], "glm-4.7");
        assert_eq!(p["has_api_key"], true);
        // key 材料与模板正文绝不出现在输出（含 api_key_env 路径）
        let s = v.to_string();
        assert!(!s.contains("sk-secret-value"));
        assert!(!s.contains("\"api_key\""));
        assert!(!s.contains("\"api_key_env\""));
        assert!(p.get("api_key").is_none() && p.get("api_key_env").is_none());
        assert!(!s.contains("system_prompt"));
        assert!(!s.contains("user_prompt_template"));
        // 白名单外的 daemon 键原样保留（非 providers 区不做过滤）
        assert_eq!(v["daemon"]["auto_translate"], true);
    }

    #[test]
    fn sanitized_config_marks_missing_key_and_keeps_env_flag() {
        let text = r#"
[[llm.providers]]
id = "a"
type = "openai"
base_url = "u"
model = "m"
api_key_env = "PARDON_KEY"

[[llm.providers]]
id = "b"
type = "openai"
base_url = "u"
model = "m"
"#;
        let v = parse_sanitized_config(text).unwrap();
        assert_eq!(v["llm"]["providers"][0]["has_api_key"], true);
        assert_eq!(v["llm"]["providers"][1]["has_api_key"], false);
        let s = v.to_string();
        assert!(!s.contains("PARDON_KEY"));
    }

    #[test]
    fn sanitized_config_rejects_non_object_provider() {
        let v = parse_sanitized_config("[llm]\nproviders = [1]\n");
        assert!(v.is_err());
    }

    #[test]
    fn sanitized_config_missing_or_invalid_text_is_empty_object() {
        for text in ["", "not toml = = = ["] {
            let v = parse_sanitized_config(text).unwrap();
            assert_eq!(v, serde_json::json!({}));
        }
    }

    // ── config_set 白名单 + toml_edit 原位写 ───────────────────────

    #[test]
    fn config_set_preserves_trailing_comment_of_existing_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[daemon]\nauto_translate = true # 复制即翻译\n",
        )
        .unwrap();
        set_config_value(&path, Some("daemon"), "auto_translate", false.into()).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("false"), "value updated: {out}");
        assert!(
            out.contains("# 复制即翻译"),
            "trailing comment survives: {out}"
        );
    }

    #[test]
    fn config_set_rejects_non_whitelisted_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[llm]\n").unwrap();
        let err = set_config_value(
            &path,
            Some("llm"),
            "api_key",
            "sk-evil".into(),
        )
        .unwrap_err();
        assert!(err.contains("not writable"), "{err}");
        // 文件未被触碰
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[llm]\n");
    }

    #[test]
    fn config_set_creates_missing_file_and_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        set_config_value(&path, Some("daemon"), "notify_timeout_ms", 3500.into()).unwrap();
        let v: serde_json::Value =
            toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["daemon"]["notify_timeout_ms"], 3500);
    }

    #[test]
    fn config_set_writes_root_table_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "default_engine = \"youdao\"\n").unwrap();
        set_config_value(&path, None, "default_engine", "llm".into()).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains(r#"default_engine = "llm""#), "{out}");
    }

    #[test]
    fn json_to_toml_converts_scalars_and_rejects_the_rest() {
        assert_eq!(json_to_toml(true.into()).unwrap().to_string(), "true");
        assert_eq!(json_to_toml(42.into()).unwrap().to_string(), "42");
        assert_eq!(json_to_toml("llm".into()).unwrap().to_string(), "\"llm\"");
        assert!(json_to_toml(serde_json::json!({})).is_err());
        assert!(json_to_toml(serde_json::json!([])).is_err());
        assert!(json_to_toml(serde_json::Value::Null).is_err());
    }

    #[test]
    fn gui_config_path_env_override() {
        let _l = CONFIG_LOCK.lock().unwrap();
        std::env::set_var("PARDON_CONFIG", "/tmp/x-t10.toml");
        assert_eq!(gui_config_path(), std::path::PathBuf::from("/tmp/x-t10.toml"));
        std::env::remove_var("PARDON_CONFIG");
    }
}

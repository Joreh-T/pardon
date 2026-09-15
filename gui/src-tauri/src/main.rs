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
    let mut child = tokio::process::Command::new("pardon")
        .arg("speak")
        .arg(&text)
        .spawn()
        .map_err(|e| format!("spawn `pardon speak`: {e}"))?;
    // 后台收尸：不等待发音进程退出，但也不留僵尸子进程
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
    Ok(())
}

/// 兜底启动：主窗口检测到未连接 pardond 时显示按钮，经 CLI 分离启动
/// （`pardon daemon start` 内部 spawn 独立进程组后立即返回，不会挂起）。
/// 失败（非零退出）返回 stderr 供前端展示诊断。
#[tauri::command]
async fn pardon_daemon_start() -> Result<String, String> {
    let out = std::process::Command::new("pardon")
        .args(["daemon", "start"])
        .output()
        .map_err(|e| format!("spawn `pardon daemon start`: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).to_string());
    }
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

/// 递归剔除任意位置的机密键名（合法 schema 里这两键只存在于 providers 内，
/// 而那段已被置换过滤——零误伤）。
fn strip_secret_keys(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            map.remove("api_key");
            map.remove("api_key_env");
            for (_k, child) in map.iter_mut() {
                strip_secret_keys(child);
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(strip_secret_keys),
        _ => {}
    }
}

/// 读配置文本 → 脱敏 JSON（供 [`read_config`] 与测试共用）。
/// 脱敏：`llm.providers` 为数组时逐项只保留 id/type/base_url/model +
/// `has_api_key` 存在性标志；**非数组形状（表形 `[llm.providers.x]` 等）
/// 整节点置换为空数组**（core 会拒绝该形状，GUI 显示「无 providers」是
/// 正确降级）——比逐值过滤更严，key 材料不可能借非数组形状透出。
/// **api_key/api_key_env 与模板正文绝不出现在输出**。
/// 文件不存在/解析失败视同空配置（设置页展示默认值，不报错）。
fn parse_sanitized_config(text: &str) -> Result<serde_json::Value, String> {
    let mut v: serde_json::Value = toml::from_str(text).unwrap_or(serde_json::json!({}));
    match v.get_mut("llm").and_then(|l| l.get_mut("providers")) {
        Some(p) if p.is_array() => {
            for p in p.as_array_mut().expect("guarded is_array").iter_mut() {
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
        // 非数组（表形/字符串/…）：core 拒绝这种配置；整节点清空兜底，
        // 不存在任何逐值漏过的可能
        Some(p) => *p = serde_json::json!([]),
        None => {}
    }
    strip_secret_keys(&mut v);
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
const WRITABLE: [(&str, Option<&str>); 9] = [
    ("auto_translate", Some("daemon")),
    ("popup", Some("daemon")),
    ("show_word_badge", Some("daemon")),
    ("copy_translation", Some("daemon")),
    ("notify_timeout_ms", Some("daemon")),
    ("max_text_bytes", Some("daemon")),
    ("dedup_window_ms", Some("daemon")),
    ("default_engine", None),
    ("default_provider", Some("llm")),
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
        return Err(format!(
            "refusing to set {key:?} in {table:?}: not writable from gui"
        ));
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
    write_config_doc(path, &doc)
}

/// 写回配置文档（父目录缺失则创建；所有原位写路径共用）。
fn write_config_doc(path: &std::path::Path, doc: &toml_edit::DocumentMut) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    std::fs::write(path, doc.to_string()).map_err(|e| e.to_string())
}

/// 原位写单个值（白名单见 [`WRITABLE`]；`table=None` 为根表）。
#[tauri::command]
fn config_set(table: Option<String>, key: String, value: serde_json::Value) -> Result<(), String> {
    set_config_value(&gui_config_path(), table.as_deref(), &key, value)
}

/// provider 编辑输入（GUI 表单 → 命令）。api_key 写入式：
/// Some(非空) = 覆盖；None / Some("") = 保留现值（清除走 provider_clear_key）。
#[derive(Deserialize)]
pub struct ProviderInput {
    pub id: String,
    #[serde(rename = "type")]
    pub provider_type: String,
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
}

/// provider type 枚举（与 pardon_core::config::ProviderType 一致）。
const PROVIDER_TYPES: [&str; 3] = ["openai", "anthropic", "ollama"];

/// 读配置文档（读不到/空文件 → 空文档起步；解析失败报错）。
fn load_config_doc(path: &std::path::Path) -> Result<toml_edit::DocumentMut, String> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.parse().map_err(|e| format!("parse config: {e}"))
}

/// 取（缺失则建 implicit 表的）`[llm]` 表。
fn llm_table_mut(doc: &mut toml_edit::DocumentMut) -> Result<&mut toml_edit::Table, String> {
    doc.entry("llm")
        .or_insert_with(|| {
            let mut t = toml_edit::Table::new();
            t.set_implicit(true);
            toml_edit::Item::Table(t)
        })
        .as_table_mut()
        .ok_or_else(|| "`llm` is not a table".to_string())
}

/// 取（缺失则建的）`llm.providers` 数组表；形状不对（`[llm.providers.x]`
/// 表形/内联数组等，core 会拒绝的形状）→ Err。
fn providers_aot_mut(llm: &mut toml_edit::Table) -> Result<&mut toml_edit::ArrayOfTables, String> {
    llm.entry("providers")
        .or_insert_with(|| toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .ok_or_else(|| "`llm.providers` is not an array of tables".to_string())
}

/// 覆写表中一个字符串键；既有值项的 decor（行尾注释/空白）搬到新值上
/// ——insert 是整项替换，直接插会丢行尾注释（与 set_config_value 同坑）。
fn table_set_str(t: &mut toml_edit::Table, key: &str, s: &str) {
    let mut new_val = toml_edit::Value::from(s);
    if let Some(item) = t.get_mut(key) {
        if let Some(old) = item.as_value_mut() {
            std::mem::swap(old.decor_mut(), new_val.decor_mut());
        }
    }
    t.insert(key, toml_edit::Item::Value(new_val));
}

/// 新增/编辑 provider（`index=None` 追加到末尾；`Some(i)` 原位编辑）。
/// 校验与 core ProviderConfig 同规则：id 非空且不与其他条目重复、
/// base_url/model 非空、type ∈ openai/anthropic/ollama。
/// 编辑只覆写 id/type/base_url/model（api_key 按 Some(非空) 才写），其余键
/// （api_key_env/system_prompt/…）原样保留；旧 id 是 default_provider 且被
/// 改名时同步改写 default_provider。
fn provider_upsert_at(
    path: &std::path::Path,
    index: Option<usize>,
    p: &ProviderInput,
) -> Result<(), String> {
    if p.id.is_empty() {
        return Err("provider id must be non-empty".to_string());
    }
    if p.base_url.is_empty() {
        return Err("provider base_url must be non-empty".to_string());
    }
    if p.model.is_empty() {
        return Err("provider model must be non-empty".to_string());
    }
    if !PROVIDER_TYPES.contains(&p.provider_type.as_str()) {
        return Err(format!(
            "unknown provider type {:?}, expected one of openai/anthropic/ollama",
            p.provider_type
        ));
    }
    let new_key = p.api_key.as_deref().filter(|s| !s.is_empty());
    let mut doc = load_config_doc(path)?;
    let llm = llm_table_mut(&mut doc)?;
    // aot 可变借用存活期间读好的改名待办；借出后再回写 default_provider
    let mut rename_default_from: Option<String> = None;
    {
        let aot = providers_aot_mut(llm)?;
        let len = aot.len();
        let editing = match index {
            Some(i) => {
                if i >= len {
                    return Err(format!("provider index {i} out of range (len {len})"));
                }
                Some(i)
            }
            None => None,
        };
        // id 不与其他条目重复（编辑条目保留自身旧 id 不算重复）
        for (i, t) in aot.iter().enumerate() {
            if editing == Some(i) {
                continue;
            }
            if t.get("id").and_then(|v| v.as_str()) == Some(p.id.as_str()) {
                return Err(format!("duplicate provider id {:?}", p.id));
            }
        }
        match editing {
            Some(i) => {
                let t = aot.get_mut(i).expect("bounds checked above");
                let old_id = t.get("id").and_then(|v| v.as_str()).map(str::to_string);
                table_set_str(t, "id", &p.id);
                table_set_str(t, "type", &p.provider_type);
                table_set_str(t, "base_url", &p.base_url);
                table_set_str(t, "model", &p.model);
                if let Some(k) = new_key {
                    table_set_str(t, "api_key", k);
                }
                if old_id.as_deref().is_some_and(|old| old != p.id) {
                    rename_default_from = old_id;
                }
            }
            None => {
                let mut t = toml_edit::Table::new();
                table_set_str(&mut t, "id", &p.id);
                table_set_str(&mut t, "type", &p.provider_type);
                table_set_str(&mut t, "base_url", &p.base_url);
                table_set_str(&mut t, "model", &p.model);
                if let Some(k) = new_key {
                    table_set_str(&mut t, "api_key", k);
                }
                aot.push(t);
            }
        }
    }
    if let Some(old) = rename_default_from {
        if llm.get("default_provider").and_then(|v| v.as_str()) == Some(old.as_str()) {
            table_set_str(llm, "default_provider", &p.id);
        }
    }
    write_config_doc(path, &doc)
}

/// 删除第 index 个 provider；该条 id 是 `llm.default_provider` 时拒绝
/// （提示先换默认，避免留下指向不存在 provider 的悬空默认）。
fn provider_delete_at(path: &std::path::Path, index: usize) -> Result<(), String> {
    let mut doc = load_config_doc(path)?;
    let llm = llm_table_mut(&mut doc)?;
    let default = llm
        .get("default_provider")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let aot = providers_aot_mut(llm)?;
    let len = aot.len();
    if index >= len {
        return Err(format!("provider index {index} out of range (len {len})"));
    }
    let victim = aot
        .get(index)
        .and_then(|t| t.get("id"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    if let Some(v) = victim.as_deref() {
        if default.as_deref() == Some(v) {
            return Err(format!(
                "provider {v:?} is llm.default_provider; switch default_provider before deleting"
            ));
        }
    }
    aot.remove(index);
    write_config_doc(path, &doc)
}

/// 清除第 index 个 provider 的 api_key 与 api_key_env 两个键
/// （读取/显示侧永不回显；清除是机密的唯一移除路径）。
fn provider_clear_key_at(path: &std::path::Path, index: usize) -> Result<(), String> {
    let mut doc = load_config_doc(path)?;
    let aot = providers_aot_mut(llm_table_mut(&mut doc)?)?;
    let len = aot.len();
    if index >= len {
        return Err(format!("provider index {index} out of range (len {len})"));
    }
    let t = aot.get_mut(index).expect("bounds checked above");
    t.remove("api_key");
    t.remove("api_key_env");
    write_config_doc(path, &doc)
}

/// 新增/编辑 provider（语义见 [`provider_upsert_at`]）。
#[tauri::command]
fn provider_upsert(index: Option<usize>, provider: ProviderInput) -> Result<(), String> {
    provider_upsert_at(&gui_config_path(), index, &provider)
}

/// 删除第 index 个 provider（默认指向守卫见 [`provider_delete_at`]）。
#[tauri::command]
fn provider_delete(index: usize) -> Result<(), String> {
    provider_delete_at(&gui_config_path(), index)
}

/// 清除第 index 个 provider 的 api_key 与 api_key_env。
#[tauri::command]
fn provider_clear_key(index: usize) -> Result<(), String> {
    provider_clear_key_at(&gui_config_path(), index)
}

/// 重启 pardond：stop（忽略结果——可能本就没在跑）→ 稍候 → start
/// 并检查退出码与 stderr（引擎字段改动生效路径）。
#[tauri::command]
async fn restart_daemon() -> Result<(), String> {
    let _ = std::process::Command::new("pardon")
        .args(["daemon", "stop"])
        .output();
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
            provider_upsert,
            provider_delete,
            provider_clear_key,
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
            // daemon 托盘的 show_window 事件（ipc.rs 转发为同名 tauri 事件）：
            // GUI 在跑时点托盘「打开主窗口」/「设置」由此消费——聚焦已有窗口，
            // 窗口被关闭（已销毁）则重建/新建。
            let win_handle = app.handle().clone();
            app.listen("show_window", move |event| {
                let kind = serde_json::from_str::<serde_json::Value>(event.payload())
                    .ok()
                    .and_then(|v| v["kind"].as_str().map(str::to_string))
                    .unwrap_or_default();
                match kind.as_str() {
                    "settings" => {
                        if let Some(w) = win_handle.get_webview_window("settings") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        } else {
                            let _ = create_settings_window(&win_handle);
                        }
                    }
                    // "main" 及兜底：重建镜像 tauri.conf.json 的 main 定义
                    _ => {
                        if let Some(w) = win_handle.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        } else {
                            let _ = tauri::WebviewWindowBuilder::new(
                                &win_handle,
                                "main",
                                tauri::WebviewUrl::App("index.html".into()),
                            )
                            .title("pardon")
                            .inner_size(760.0, 520.0)
                            .build();
                        }
                    }
                }
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

    /// 机密键名在 providers 之外的任意位置（根表 / [llm] 直下 / [daemon] 下）
    /// 也不得透出——脱敏约束是绝对的，不因位置而豁免。
    #[test]
    fn sanitized_config_strips_secret_keys_outside_providers() {
        let text = r#"
api_key = "sk-root-secret"

[llm]
api_key_env = "PARDON_ROOT_KEY"

[daemon]
api_key = "sk-daemon-secret"
auto_translate = true
"#;
        let v = parse_sanitized_config(text).unwrap();
        let s = v.to_string();
        // 三位置各一条：值与键名都不出现
        assert!(!s.contains("sk-root-secret"), "root api_key value: {s}");
        assert!(
            !s.contains("PARDON_ROOT_KEY"),
            "[llm] api_key_env value: {s}"
        );
        assert!(
            !s.contains("sk-daemon-secret"),
            "[daemon] api_key value: {s}"
        );
        assert!(!s.contains("\"api_key\""), "{s}");
        assert!(!s.contains("\"api_key_env\""), "{s}");
        // 同层的非机密键不受影响（零误伤）
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

    /// 表形 providers（`[llm.providers.glm]`）绝不能让 key 材料透出：
    /// 整节点置换为空数组（core 会拒绝该形状，GUI 显示「无 providers」）。
    #[test]
    fn sanitized_config_neutralizes_table_shaped_providers() {
        let text = r#"
[llm]
default_provider = "glm"

[llm.providers.glm]
id = "glm"
type = "openai"
base_url = "https://api.example.invalid/v1"
model = "glm-4.7"
api_key = "sk-table-secret"
api_key_env = "PARDON_TABLE_KEY"
"#;
        let v = parse_sanitized_config(text).unwrap();
        assert_eq!(v["llm"]["providers"], serde_json::json!([]));
        let s = v.to_string();
        assert!(!s.contains("sk-table-secret"));
        assert!(!s.contains("PARDON_TABLE_KEY"));
        assert!(!s.contains("\"api_key\""));
        assert!(!s.contains("\"api_key_env\""));
        // llm 表其余键不受影响
        assert_eq!(v["llm"]["default_provider"], "glm");
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
        std::fs::write(&path, "[daemon]\nauto_translate = true # 复制即翻译\n").unwrap();
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
        let err = set_config_value(&path, Some("llm"), "api_key", "sk-evil".into()).unwrap_err();
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
        assert_eq!(
            gui_config_path(),
            std::path::PathBuf::from("/tmp/x-t10.toml")
        );
        std::env::remove_var("PARDON_CONFIG");
    }

    // ── provider 管理（toml_edit ArrayOfTables 原位编辑）─────────────

    const BASE_TOML: &str = r#"default_engine = "llm"
# 顶部注释
[llm]
default_provider = "glm"
[[llm.providers]]
id = "glm" # 行尾注释
type = "openai"
base_url = "https://open.bigmodel.cn/api/paas/v4"
model = "glm-5.3-flash"
api_key = "fake-existing"
"#;

    /// 测试 helper：重读文件数 `[[llm.providers]]` 条数。
    fn provider_count(path: &std::path::Path) -> usize {
        let doc: toml_edit::DocumentMut = std::fs::read_to_string(path).unwrap().parse().unwrap();
        doc["llm"]["providers"]
            .as_array_of_tables()
            .expect("providers is array-of-tables")
            .len()
    }

    #[test]
    fn provider_upsert_adds_new_with_comments_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.toml");
        std::fs::write(&p, BASE_TOML).unwrap();
        provider_upsert_at(
            &p,
            None,
            &ProviderInput {
                id: "ds".into(),
                provider_type: "openai".into(),
                base_url: "https://api.deepseek.com/v1".into(),
                model: "deepseek-chat".into(),
                api_key: Some("fake-new".into()),
            },
        )
        .unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("# 顶部注释"), "{text}");
        assert!(text.contains("# 行尾注释"), "{text}");
        assert!(text.contains(r#"id = "ds""#));
        assert!(text.contains(r#"api_key = "fake-new""#));
        assert_eq!(provider_count(&p), 2);
    }

    #[test]
    fn provider_upsert_edit_keeps_key_when_absent() {
        // api_key None → 既有 key 保留
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.toml");
        std::fs::write(&p, BASE_TOML).unwrap();
        provider_upsert_at(
            &p,
            Some(0),
            &ProviderInput {
                id: "glm".into(),
                provider_type: "openai".into(),
                base_url: "https://changed.example/v1".into(),
                model: "glm-5.4".into(),
                api_key: None,
            },
        )
        .unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(
            text.contains("fake-existing"),
            "None 不得清掉既有 key: {text}"
        );
        assert!(text.contains("changed.example"));
        assert_eq!(provider_count(&p), 1);
    }

    #[test]
    fn provider_upsert_rejects_bad_type_and_duplicate_id() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.toml");
        std::fs::write(&p, BASE_TOML).unwrap();
        // type 非 openai/anthropic/ollama → Err
        let err = provider_upsert_at(
            &p,
            None,
            &ProviderInput {
                id: "ds".into(),
                provider_type: "gemini".into(),
                base_url: "https://api.example/v1".into(),
                model: "m".into(),
                api_key: None,
            },
        )
        .unwrap_err();
        assert!(err.contains("type"), "{err}");
        // id 与现有重复（且非本条）→ Err
        let err = provider_upsert_at(
            &p,
            None,
            &ProviderInput {
                id: "glm".into(),
                provider_type: "openai".into(),
                base_url: "https://api.example/v1".into(),
                model: "m".into(),
                api_key: None,
            },
        )
        .unwrap_err();
        assert!(err.contains("duplicate"), "{err}");
        // 编辑条目保留自身旧 id 不算重复
        provider_upsert_at(
            &p,
            Some(0),
            &ProviderInput {
                id: "glm".into(),
                provider_type: "anthropic".into(),
                base_url: "https://api.example/v1".into(),
                model: "m".into(),
                api_key: None,
            },
        )
        .unwrap();
        // 被拒的两笔没写进文件
        assert_eq!(provider_count(&p), 1);
        assert!(!std::fs::read_to_string(&p).unwrap().contains("gemini"));
    }

    #[test]
    fn provider_upsert_renaming_default_updates_default_provider() {
        // 把 index 0 的 id 从 glm 改为 glm2，且 default_provider == "glm"
        // → default_provider 同步写为 "glm2"
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.toml");
        std::fs::write(&p, BASE_TOML).unwrap();
        provider_upsert_at(
            &p,
            Some(0),
            &ProviderInput {
                id: "glm2".into(),
                provider_type: "openai".into(),
                base_url: "https://open.bigmodel.cn/api/paas/v4".into(),
                model: "glm-5.3-flash".into(),
                api_key: None,
            },
        )
        .unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains(r#"default_provider = "glm2""#), "{text}");
        assert!(text.contains(r#"id = "glm2""#), "{text}");
        assert!(text.contains("# 行尾注释"), "编辑不得丢行尾注释: {text}");
    }

    #[test]
    fn provider_delete_refuses_when_default_points_at_it() {
        // default_provider == "glm" 时删 index 0 → Err 含 "default_provider"
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.toml");
        std::fs::write(&p, BASE_TOML).unwrap();
        let err = provider_delete_at(&p, 0).unwrap_err();
        assert!(err.contains("default_provider"), "{err}");
        assert_eq!(provider_count(&p), 1, "被拒的删除不得动文件");
    }

    #[test]
    fn provider_delete_removes_entry() {
        // default_provider 改指别的后删除成功，count-1
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.toml");
        let two = format!(
            "{BASE_TOML}[[llm.providers]]\nid = \"ds\"\ntype = \"openai\"\nbase_url = \"https://api.deepseek.com/v1\"\nmodel = \"deepseek-chat\"\n"
        );
        std::fs::write(&p, two).unwrap();
        set_config_value(&p, Some("llm"), "default_provider", "ds".into()).unwrap();
        provider_delete_at(&p, 0).unwrap();
        assert_eq!(provider_count(&p), 1);
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains(r#"id = "glm""#), "{text}");
        assert!(!text.contains("fake-existing"), "{text}");
        assert!(text.contains(r#"id = "ds""#), "{text}");
    }

    #[test]
    fn provider_clear_key_removes_both_key_fields() {
        // 同时移除 api_key 与 api_key_env（构造一个带 api_key_env 的）
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.toml");
        let with_env = BASE_TOML.replace(
            "api_key = \"fake-existing\"",
            "api_key = \"fake-existing\"\napi_key_env = \"PARDON_TEST_KEY\"",
        );
        std::fs::write(&p, with_env).unwrap();
        provider_clear_key_at(&p, 0).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("fake-existing"), "{text}");
        assert!(!text.contains("PARDON_TEST_KEY"), "{text}");
        assert!(!text.contains("api_key"), "{text}");
        assert_eq!(provider_count(&p), 1);
    }

    #[test]
    fn config_set_accepts_default_provider_llm() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.toml");
        std::fs::write(&p, BASE_TOML).unwrap();
        set_config_value(&p, Some("llm"), "default_provider", "other".into()).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains(r#"default_provider = "other""#), "{text}");
    }
}

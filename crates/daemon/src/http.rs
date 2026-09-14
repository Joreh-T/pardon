//! localhost HTTP 触发口（spec §5.4：给 niri bind curl、脚本与其他客户端）。

use crate::handler::{handle_text, HandleResult, Origin};
use crate::state::DaemonState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub fn router(state: Arc<DaemonState>) -> Router {
    Router::new()
        .route("/translate", post(post_translate))
        .route("/trigger/selection", get(trigger_selection))
        .route("/trigger/clipboard", get(trigger_clipboard))
        .route("/status", get(get_status))
        .route("/shutdown", post(post_shutdown))
        .with_state(state)
}

fn err_json(code: StatusCode, msg: &str) -> Response {
    (code, Json(serde_json::json!({ "error": msg }))).into_response()
}

#[derive(Deserialize)]
struct TranslateBody {
    text: String,
}

async fn post_translate(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<TranslateBody>,
) -> Response {
    let text = body.text.trim().to_string();
    if text.is_empty() {
        return err_json(StatusCode::BAD_REQUEST, "empty text");
    }
    let tr = state.translator.translate(&text).await;
    state.counters.translations.fetch_add(1, Ordering::Relaxed);
    Json(tr).into_response()
}

#[derive(Serialize)]
struct TriggerResponse {
    notified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    translation: Option<pardon_core::pipeline::Translation>,
}

async fn trigger(state: Arc<DaemonState>, primary: bool) -> Response {
    let clip = state.clipboard.clone();
    let read = tokio::task::spawn_blocking(move || {
        if primary {
            clip.read_primary()
        } else {
            clip.read_clipboard()
        }
    })
    .await
    .unwrap_or_else(|e| Err(anyhow::anyhow!("clipboard read task failed: {e}")));

    let text = match read {
        Ok(t) => t,
        Err(e) => {
            let msg = format!("{e:#}");
            let lower = msg.to_lowercase();
            // 「无文本」判据（spike 发现 5 实测 wl-clipboard 2.2.1 文案 +
            // 未验证分支的兜底词）：not available as requested type 为真实
            // stderr；no suitable/no selection/empty 覆盖空选区等路径
            if lower.contains("not available as requested type")
                || lower.contains("no suitable")
                || lower.contains("no selection")
                || lower.contains("empty")
            {
                return Json(TriggerResponse {
                    notified: false,
                    reason: Some("no text available".into()),
                    translation: None,
                })
                .into_response();
            }
            return err_json(StatusCode::SERVICE_UNAVAILABLE, &msg);
        }
    };

    match handle_text(&state, &text, Origin::Trigger).await {
        HandleResult::Handled {
            translation,
            notified,
        } => Json(TriggerResponse {
            notified,
            reason: None,
            translation: Some(translation),
        })
        .into_response(),
        HandleResult::Skipped(reason) => Json(TriggerResponse {
            notified: false,
            reason: Some(reason.into()),
            translation: None,
        })
        .into_response(),
    }
}

async fn trigger_selection(State(state): State<Arc<DaemonState>>) -> Response {
    trigger(state, true).await
}

async fn trigger_clipboard(State(state): State<Arc<DaemonState>>) -> Response {
    trigger(state, false).await
}

#[derive(Serialize)]
struct StatusResponse {
    version: &'static str,
    uptime_s: u64,
    clipboard_watching: bool,
    auto_translate: bool,
    default_engine: String,
    counters: StatusCounters,
}

#[derive(Serialize)]
struct StatusCounters {
    clipboard_events: u64,
    translations: u64,
    notifications: u64,
    triggers: u64,
}

async fn get_status(State(state): State<Arc<DaemonState>>) -> Response {
    Json(StatusResponse {
        version: pardon_core::VERSION,
        uptime_s: state.started.elapsed().as_secs(),
        clipboard_watching: state.clipboard_watching.load(Ordering::Relaxed),
        auto_translate: state.cfg.daemon.auto_translate,
        default_engine: state.cfg.default_engine.clone(),
        counters: StatusCounters {
            clipboard_events: state.counters.clipboard_events.load(Ordering::Relaxed),
            translations: state.counters.translations.load(Ordering::Relaxed),
            notifications: state.counters.notifications.load(Ordering::Relaxed),
            triggers: state.counters.triggers.load(Ordering::Relaxed),
        },
    })
    .into_response()
}

async fn post_shutdown(State(state): State<Arc<DaemonState>>) -> Response {
    state.shutdown.notify_one();
    Json(serde_json::json!({ "status": "shutting down" })).into_response()
}

//! localhost HTTP 触发口（spec §5.4：给 niri bind curl、脚本与其他客户端）。
//! 触发与状态逻辑抽至 trigger.rs / status.rs，与 UDS IPC 共用。

use crate::state::DaemonState;
use crate::trigger::TriggerOutcome;
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
    match crate::trigger::run(&state, primary).await {
        TriggerOutcome::NoText => Json(TriggerResponse {
            notified: false,
            reason: Some("no text available".into()),
            translation: None,
        })
        .into_response(),
        TriggerOutcome::Handled {
            notified,
            translation,
        } => Json(TriggerResponse {
            notified,
            reason: None,
            translation: Some(translation),
        })
        .into_response(),
        TriggerOutcome::Failed(msg) => err_json(StatusCode::SERVICE_UNAVAILABLE, &msg),
    }
}

async fn trigger_selection(State(state): State<Arc<DaemonState>>) -> Response {
    trigger(state, true).await
}

async fn trigger_clipboard(State(state): State<Arc<DaemonState>>) -> Response {
    trigger(state, false).await
}

async fn get_status(State(state): State<Arc<DaemonState>>) -> Response {
    Json(crate::status::snapshot(&state)).into_response()
}

async fn post_shutdown(State(state): State<Arc<DaemonState>>) -> Response {
    // 多个等待者（axum graceful shutdown、uds::serve）都要醒：notify_waiters
    // 广播；补一次 notify_one 存许可——晚到才 await notified() 的调用方
    // （如 http_test）仍能立即通过
    state.shutdown.notify_waiters();
    state.shutdown.notify_one();
    Json(serde_json::json!({ "status": "shutting down" })).into_response()
}

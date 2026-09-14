//! 触发翻译（读 selection/clipboard → handle_text），HTTP 与 UDS 共用。

use crate::handler::{handle_text, HandleResult, Origin};
use crate::state::DaemonState;
use pardon_core::pipeline::Translation;
use std::sync::Arc;

pub enum TriggerOutcome {
    /// 无文本（选区空/剪贴板空/跳过）。
    NoText,
    Handled {
        notified: bool,
        translation: Translation,
    },
    /// 剪贴板系统错误（503 语义）。
    Failed(String),
}

pub async fn run(state: &Arc<DaemonState>, primary: bool) -> TriggerOutcome {
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
                return TriggerOutcome::NoText;
            }
            return TriggerOutcome::Failed(msg);
        }
    };

    match handle_text(state, &text, Origin::Trigger).await {
        HandleResult::Handled {
            translation,
            notified,
        } => TriggerOutcome::Handled {
            notified,
            translation,
        },
        // Trigger 源只可能 "empty"（长度上限/去重仅 Auto 生效）→ 并入 NoText
        HandleResult::Skipped(_) => TriggerOutcome::NoText,
    }
}

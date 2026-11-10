//! WebSocket endpoints for live dashboard metrics.

use crate::extract::AuthUser;
use crate::state::AppState;
use axum::extract::ws::{Message, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::Response;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use synthpanel_ipc::methods;

/// Upgrade to a WebSocket that pushes a metrics snapshot every few seconds.
pub async fn dashboard_ws(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    upgrade: WebSocketUpgrade,
) -> Response {
    let _ = user;
    upgrade.on_upgrade(move |socket| async move {
        let mut socket = socket;
        let interval = Duration::from_secs(3);
        loop {
            let snapshot: Option<Value> =
                match state.core_call(methods::METRICS_SNAPSHOT, json!({})).await {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = socket
                            .send(Message::Text(
                                json!({ "type": "error", "message": error.to_string() })
                                    .to_string()
                                    .into(),
                            ))
                            .await;
                        return;
                    }
                };

            let services: Vec<Value> = state
                .core_call(methods::SERVICES_LIST, json!({}))
                .await
                .unwrap_or_default();

            let payload = json!({
                "type": "snapshot",
                "metrics": snapshot,
                "services": services,
            });
            if socket
                .send(Message::Text(payload.to_string().into()))
                .await
                .is_err()
            {
                return;
            }

            tokio::time::sleep(interval).await;
        }
    })
}

#[derive(Debug, Deserialize)]
pub struct LogTailQuery {
    #[serde(default = "default_source")]
    pub r#type: String,
    #[serde(default)]
    pub site_id: Option<String>,
    #[serde(default = "default_lines")]
    pub lines: usize,
}

fn default_source() -> String {
    "panel".to_string()
}

fn default_lines() -> usize {
    200
}

/// Stream log lines from the core over the socket.
pub async fn logs_ws(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Query(query): Query<LogTailQuery>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let _ = user;
    upgrade.on_upgrade(move |socket| async move {
        let mut socket = socket;
        let mut ticker = tokio::time::interval(Duration::from_secs(2));
        let mut offset: u64 = 0;

        loop {
            ticker.tick().await;
            let result: Value = match state
                .core_call(
                    methods::LOGS_TAIL,
                    json!({
                        "source": query.r#type,
                        "site_id": query.site_id,
                        "offset": offset,
                        "lines": query.lines,
                    }),
                )
                .await
            {
                Ok(value) => value,
                Err(error) => {
                    let _ = socket
                        .send(Message::Text(
                            json!({ "type": "error", "message": error.to_string() })
                                .to_string()
                                .into(),
                        ))
                        .await;
                    return;
                }
            };

            offset = result
                .get("offset")
                .and_then(Value::as_u64)
                .unwrap_or(offset);

            let lines = result
                .get("lines")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for line in lines {
                let Some(line) = line.as_str() else {
                    continue;
                };
                if socket
                    .send(Message::Text(
                        json!({ "type": "log", "line": line }).to_string().into(),
                    ))
                    .await
                    .is_err()
                {
                    return;
                }
            }
        }
    })
}

/// Helper used by the tests: turn a stream of values into text frames.
#[cfg(test)]
pub fn text_frame(value: &Value) -> Message {
    Message::Text(value.to_string().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_frame_serialises() {
        let frame = text_frame(&json!({"type": "snapshot"}));
        match frame {
            Message::Text(text) => assert!(text.as_str().contains("snapshot")),
            other => panic!("unexpected frame: {other:?}"),
        }
    }
}

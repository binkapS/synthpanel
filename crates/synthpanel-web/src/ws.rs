//! WebSocket endpoints for live dashboard metrics.

use crate::extract::AuthUser;
use crate::state::AppState;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocketUpgrade};
use axum::response::Response;
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

/// Stream raw log lines from a file over the socket.
pub async fn logs_ws(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    upgrade: WebSocketUpgrade,
) -> Response {
    let _ = user;
    let state_for_stream = state;
    upgrade.on_upgrade(move |socket| async move {
        let mut socket = socket;
        let mut ticker = tokio::time::interval(Duration::from_secs(2));
        let mut offset: u64 = 0;
        let path = state_for_stream.config.log_dir.join("synthpanel-web.log");

        loop {
            ticker.tick().await;
            let Ok(metadata) = tokio::fs::metadata(&path).await else {
                continue;
            };
            let len = metadata.len();
            if len < offset {
                // The file was rotated; start over.
                offset = 0;
            }
            if len == offset {
                continue;
            }
            let mut file = match tokio::fs::File::open(&path).await {
                Ok(file) => file,
                Err(_) => continue,
            };
            use tokio::io::AsyncSeekExt;
            if file.seek(std::io::SeekFrom::Start(offset)).await.is_err() {
                continue;
            }
            let mut buffer = String::new();
            {
                use tokio::io::AsyncReadExt;
                if file.read_to_string(&mut buffer).await.is_err() {
                    continue;
                }
            }
            offset = len;
            for line in buffer.lines() {
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

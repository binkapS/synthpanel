//! WebSocket endpoints for live dashboard metrics.

use crate::extract::AuthUser;
use crate::state::AppState;
use axum::extract::ws::{Message, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
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

/// Relay a terminal session between the browser and the core.
///
/// The browser sends `{"type":"input","data":"…"}` and
/// `{"type":"resize","rows":24,"cols":80}` frames; the server pushes
/// `{"type":"output","data":"…"}` frames back.
pub async fn terminal_ws(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
    upgrade: WebSocketUpgrade,
) -> Response {
    if let Err(error) = crate::api::admin::require_admin(&state, &user).await {
        return error.into_response();
    }
    upgrade.on_upgrade(move |socket| async move {
        use futures_util::{SinkExt, StreamExt};

        let (mut sink, mut stream) = socket.split();
        let state = state.clone();
        let session_id = id.clone();

        // Forward the browser input to the core.
        let writer_state = state.clone();
        let writer_id = session_id.clone();
        let writer = tokio::spawn(async move {
            while let Some(Ok(message)) = stream.next().await {
                let text = match message {
                    Message::Text(text) => text.to_string(),
                    Message::Close(_) => break,
                    Message::Ping(_) | Message::Pong(_) | Message::Binary(_) => continue,
                };
                let Ok(frame) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                match frame.get("type").and_then(Value::as_str) {
                    Some("input") => {
                        let data = frame.get("data").and_then(Value::as_str).unwrap_or("");
                        let _ = writer_state
                            .core_call::<Value>(
                                methods::TERMINAL_WRITE,
                                json!({ "id": writer_id, "data": data }),
                            )
                            .await;
                    }
                    Some("resize") => {
                        let rows = frame.get("rows").and_then(Value::as_u64).unwrap_or(24) as u16;
                        let cols = frame.get("cols").and_then(Value::as_u64).unwrap_or(80) as u16;
                        let _ = writer_state
                            .core_call::<Value>(
                                methods::TERMINAL_RESIZE,
                                json!({ "id": writer_id, "rows": rows, "cols": cols }),
                            )
                            .await;
                    }
                    _ => {}
                }
            }
        });

        // Poll the core for output.
        loop {
            let result: Value = match state
                .core_call(methods::TERMINAL_READ, json!({ "id": session_id }))
                .await
            {
                Ok(value) => value,
                Err(error) => {
                    let _ = sink
                        .send(Message::Text(
                            json!({ "type": "error", "message": error.to_string() })
                                .to_string()
                                .into(),
                        ))
                        .await;
                    break;
                }
            };
            let data = result.get("data").and_then(Value::as_str).unwrap_or("");
            if !data.is_empty()
                && sink
                    .send(Message::Text(
                        json!({ "type": "output", "data": data }).to_string().into(),
                    ))
                    .await
                    .is_err()
            {
                break;
            }
            if result
                .get("closed")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                let _ = sink
                    .send(Message::Text(
                        json!({ "type": "closed" }).to_string().into(),
                    ))
                    .await;
                break;
            }
            tokio::time::sleep(Duration::from_millis(120)).await;
        }

        writer.abort();
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

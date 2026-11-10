//! Client for the core Unix-socket IPC API.

use serde::de::DeserializeOwned;
use serde_json::Value;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("core is unavailable: {0}")]
    Unavailable(String),
    #[error("core rejected the request ({code}): {message}")]
    Remote { code: String, message: String },
    #[error("cannot decode core response: {0}")]
    Decode(String),
}

pub async fn core_call<T: DeserializeOwned>(
    socket: &Path,
    token: &str,
    method: &str,
    params: Value,
) -> Result<T, CoreError> {
    let mut client = synthpanel_ipc::Client::connect(socket, token)
        .await
        .map_err(|error| CoreError::Unavailable(error.to_string()))?;
    client
        .call::<T>(method, params)
        .await
        .map_err(|error| match error {
            synthpanel_ipc::ClientError::Protocol(error) => CoreError::Remote {
                code: error.code,
                message: error.message,
            },
            other => CoreError::Unavailable(other.to_string()),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_error_displays_code_and_message() {
        let error = CoreError::Remote {
            code: "not_found".into(),
            message: "no such user".into(),
        };
        let text = error.to_string();
        assert!(text.contains("not_found"));
        assert!(text.contains("no such user"));
    }
}

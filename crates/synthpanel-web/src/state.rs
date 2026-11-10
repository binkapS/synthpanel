//! Shared application state.

use crate::config::WebConfig;
use crate::ratelimit::RateLimiter;
use crate::session::{PendingTwoFactorStore, SessionStore};
use std::time::Instant;

#[derive(Debug)]
pub struct AppState {
    pub config: WebConfig,
    pub sessions: SessionStore,
    pub pending_2fa: PendingTwoFactorStore,
    pub rate_limiter: RateLimiter,
    pub started_at: Instant,
    pub core_socket: std::path::PathBuf,
    pub ipc_token: String,
}

impl AppState {
    pub fn new(config: WebConfig, core_socket: std::path::PathBuf, ipc_token: String) -> Self {
        Self {
            config,
            sessions: SessionStore::new(),
            pending_2fa: PendingTwoFactorStore::new(),
            rate_limiter: RateLimiter::new(),
            started_at: Instant::now(),
            core_socket,
            ipc_token,
        }
    }

    pub async fn core_call<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<T, crate::ipc::CoreError> {
        crate::ipc::core_call(&self.core_socket, &self.ipc_token, method, params).await
    }
}

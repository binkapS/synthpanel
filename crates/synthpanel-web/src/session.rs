//! In-memory session store.
//!
//! Phase 0 keeps sessions in the web process. They survive within a single
//! process lifetime; persisting them in the core database is a Phase 1
//! improvement so a web restart does not log every administrator out.

use crate::auth;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct SessionRecord {
    pub user_id: String,
    pub csrf: String,
    pub created_at: Instant,
    pub expires_at: Instant,
    pub ip: String,
    pub user_agent: Option<String>,
}

#[derive(Debug, Default)]
pub struct SessionStore {
    inner: Mutex<HashMap<String, SessionRecord>>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create(
        &self,
        user_id: String,
        ip: String,
        user_agent: Option<String>,
        ttl: Duration,
        csrf: String,
    ) -> String {
        let token = auth::generate_token(32);
        let key = auth::hash_token(&token);
        let now = Instant::now();
        let record = SessionRecord {
            user_id,
            csrf,
            created_at: now,
            expires_at: now + ttl,
            ip,
            user_agent,
        };
        let mut map = self.inner.lock().unwrap();
        map.insert(key, record);
        token
    }

    pub fn get(&self, token: &str) -> Option<SessionRecord> {
        let key = auth::hash_token(token);
        let mut map = self.inner.lock().unwrap();
        let record = map.get(&key).cloned();
        match record {
            Some(record) if record.expires_at > Instant::now() => Some(record),
            Some(_) => {
                map.remove(&key);
                None
            }
            None => None,
        }
    }

    pub fn destroy(&self, token: &str) {
        let key = auth::hash_token(token);
        self.inner.lock().unwrap().remove(&key);
    }

    pub fn revoke_all_for_user(&self, user_id: &str) {
        let mut map = self.inner.lock().unwrap();
        map.retain(|_, record| record.user_id != user_id);
    }

    pub fn cleanup(&self) {
        let now = Instant::now();
        let mut map = self.inner.lock().unwrap();
        map.retain(|_, record| record.expires_at > now);
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Pending two-factor challenges (before a session exists).
#[derive(Debug, Clone)]
pub struct PendingTwoFactor {
    pub user_id: String,
    pub expires_at: Instant,
}

#[derive(Debug, Default)]
pub struct PendingTwoFactorStore {
    inner: Mutex<HashMap<String, PendingTwoFactor>>,
}

impl PendingTwoFactorStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create(&self, user_id: String, ttl: Duration) -> String {
        let id = auth::generate_token(16);
        let record = PendingTwoFactor {
            user_id,
            expires_at: Instant::now() + ttl,
        };
        let mut map = self.inner.lock().unwrap();
        map.insert(id.clone(), record);
        id
    }

    pub fn take(&self, id: &str) -> Option<PendingTwoFactor> {
        let mut map = self.inner.lock().unwrap();
        let record = map.remove(id)?;
        if record.expires_at <= Instant::now() {
            return None;
        }
        Some(record)
    }

    pub fn cleanup(&self) {
        let now = Instant::now();
        let mut map = self.inner.lock().unwrap();
        map.retain(|_, record| record.expires_at > now);
    }
}

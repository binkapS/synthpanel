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
    /// The display name, so the audit log can name the actor.
    pub name: String,
    /// The role the account had when the session was created.
    ///
    /// A role change revokes the sessions of that account, so the cached
    /// role cannot become stale.
    pub role: String,
    pub csrf: String,
    pub created_at: Instant,
    pub expires_at: Instant,
    pub ip: String,
    pub user_agent: Option<String>,
}

/// What a session looks like to the account that owns it.
///
/// The raw token is never returned: `id` is the SHA-256 key under which the
/// session is stored, which is enough to tell sessions apart but cannot be
/// used to authenticate. Times are relative to process start because the
/// store uses `Instant` rather than wall-clock timestamps.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub ip: String,
    pub user_agent: Option<String>,
    pub age_seconds: u64,
    pub expires_in_seconds: u64,
    pub current: bool,
}

#[derive(Debug, Default)]
pub struct SessionStore {
    inner: Mutex<HashMap<String, SessionRecord>>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &self,
        user_id: String,
        name: String,
        role: String,
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
            name,
            role,
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

    /// The live sessions of one account, newest first.
    ///
    /// `current_token` is the plaintext token from the request; it is hashed
    /// so the caller can mark the session it arrived on. Passing the token of
    /// an API key (which has no session) simply marks none of them current.
    pub fn list_for_user(&self, user_id: &str, current_token: &str) -> Vec<SessionInfo> {
        let current_key = auth::hash_token(current_token);
        let now = Instant::now();
        let mut map = self.inner.lock().unwrap();
        // An expired session is not worth listing, and dropping it here keeps
        // the count in `revoke_all_others` honest.
        map.retain(|_, record| record.expires_at > now);
        let mut sessions: Vec<SessionInfo> = map
            .iter()
            .filter(|(_, record)| record.user_id == user_id)
            .map(|(key, record)| SessionInfo {
                id: key.clone(),
                ip: record.ip.clone(),
                user_agent: record.user_agent.clone(),
                age_seconds: now.saturating_duration_since(record.created_at).as_secs(),
                expires_in_seconds: record.expires_at.saturating_duration_since(now).as_secs(),
                current: key == &current_key,
            })
            .collect();
        sessions.sort_by_key(|info| info.age_seconds);
        sessions
    }

    /// Revoke every session of an account except the one making the call.
    ///
    /// Returns the number of sessions that were removed.
    pub fn revoke_all_others(&self, user_id: &str, current_token: &str) -> usize {
        let current_key = auth::hash_token(current_token);
        let now = Instant::now();
        let mut map = self.inner.lock().unwrap();
        let before = map
            .values()
            .filter(|record| record.user_id == user_id)
            .count();
        map.retain(|key, record| {
            record.expires_at > now && (record.user_id != user_id || key == &current_key)
        });
        let after = map
            .values()
            .filter(|record| record.user_id == user_id)
            .count();
        before - after
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
    /// The address the challenge was created for.
    pub ip: String,
    /// The user agent the challenge was created for.
    pub user_agent: String,
    pub expires_at: Instant,
}

#[derive(Debug, Default)]
pub struct PendingTwoFactorStore {
    inner: Mutex<HashMap<String, PendingTwoFactor>>,
    /// The last code that was accepted for an account, so that it cannot be
    /// used twice.
    used_codes: Mutex<HashMap<String, String>>,
}

impl PendingTwoFactorStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create(&self, user_id: String, ip: String, user_agent: String, ttl: Duration) -> String {
        let id = auth::generate_token(16);
        let record = PendingTwoFactor {
            user_id,
            ip,
            user_agent,
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

    /// Whether the code was already used for this account.
    pub fn code_was_used(&self, user_id: &str, code: &str) -> bool {
        let used = self.used_codes.lock().unwrap();
        used.get(user_id)
            .map(|value| value == code)
            .unwrap_or(false)
    }

    /// Remember the code, so that it cannot be used again.
    pub fn remember_code(&self, user_id: &str, code: &str) {
        let mut used = self.used_codes.lock().unwrap();
        used.insert(user_id.to_string(), code.to_string());
    }

    pub fn cleanup(&self) {
        let now = Instant::now();
        let mut map = self.inner.lock().unwrap();
        map.retain(|_, record| record.expires_at > now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_only_the_accounts_sessions_and_marks_the_current_one() {
        let store = SessionStore::new();
        let ttl = Duration::from_secs(3600);
        let current = store.create(
            "alice".into(),
            "Alice".into(),
            "user".into(),
            "1.2.3.4".into(),
            Some("browser".into()),
            ttl,
            "csrf".into(),
        );
        let other = store.create(
            "alice".into(),
            "Alice".into(),
            "user".into(),
            "5.6.7.8".into(),
            None,
            ttl,
            "csrf".into(),
        );
        store.create(
            "bob".into(),
            "Bob".into(),
            "admin".into(),
            "9.9.9.9".into(),
            None,
            ttl,
            "csrf".into(),
        );

        let sessions = store.list_for_user("alice", &current);
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions.iter().filter(|s| s.current).count(), 1);
        assert!(
            sessions
                .iter()
                .any(|s| s.id == auth::hash_token(&current) && s.current)
        );
        assert!(
            sessions
                .iter()
                .any(|s| s.id == auth::hash_token(&other) && !s.current)
        );
        assert!(sessions.iter().all(|s| s.expires_in_seconds > 0));

        assert_eq!(store.revoke_all_others("alice", &current), 1);
        let sessions = store.list_for_user("alice", &current);
        assert_eq!(sessions.len(), 1);
        assert!(sessions[0].current);
        assert_eq!(store.len(), 2, "bob's session survives");
    }

    #[test]
    fn expired_sessions_are_not_listed_or_counted() {
        let store = SessionStore::new();
        store.create(
            "alice".into(),
            "Alice".into(),
            "user".into(),
            "".into(),
            None,
            Duration::ZERO,
            "csrf".into(),
        );
        assert!(store.list_for_user("alice", "unknown-token").is_empty());
        assert_eq!(store.revoke_all_others("alice", "unknown-token"), 0);
    }
}

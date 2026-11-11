//! REST API v1.

pub mod admin;
pub mod alerts;
pub mod apikeys;
pub mod auth;
pub mod backups;
pub mod cron;
pub mod dashboard;
pub mod databases;
pub mod deployments;
pub mod files;
pub mod logs;
pub mod panel;
pub mod security;
pub mod sites;
pub mod terminal;
pub mod update;

use crate::state::AppState;
use axum::Router;
use axum::routing::{get, patch, post};
use std::sync::Arc;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/v1/auth/login", post(auth::login))
        .route("/api/v1/auth/logout", post(auth::logout))
        .route("/api/v1/auth/me", get(auth::me))
        .route("/api/v1/auth/2fa/setup", post(auth::totp_setup))
        .route("/api/v1/auth/2fa/enable", post(auth::totp_enable))
        .route("/api/v1/auth/2fa/verify", post(auth::totp_verify))
        .route("/api/v1/auth/2fa/disable", post(auth::totp_disable))
        .route("/api/v1/metrics/snapshot", get(dashboard::metrics_snapshot))
        .route("/api/v1/metrics/range", get(dashboard::metrics_range))
        .route("/api/v1/services", get(dashboard::services))
        .route("/api/v1/audit", get(dashboard::audit))
        .route(
            "/api/v1/settings",
            get(admin::settings_get).patch(admin::settings_patch),
        )
        .route(
            "/api/v1/users",
            get(admin::users_list).post(admin::users_create),
        )
        .route("/api/v1/users/{id}", patch(admin::users_update))
        .route("/api/v1/sites", get(sites::list).post(sites::create))
        .route(
            "/api/v1/databases",
            get(databases::list).post(databases::create),
        )
        .route(
            "/api/v1/databases/{id}",
            get(databases::get).delete(databases::delete),
        )
        .route(
            "/api/v1/databases/{id}/users",
            get(databases::users).post(databases::add_user),
        )
        .route(
            "/api/v1/databases/users/{id}",
            axum::routing::delete(databases::delete_user),
        )
        .route("/api/v1/security/firewall", get(security::firewall))
        .route(
            "/api/v1/security/firewall/rules",
            post(security::firewall_apply),
        )
        .route("/api/v1/security/fail2ban", get(security::fail2ban))
        .route(
            "/api/v1/security/fail2ban/unban",
            post(security::fail2ban_unban),
        )
        .route(
            "/api/v1/security/ssh-keys",
            get(security::ssh_keys).post(security::ssh_key_add),
        )
        .route(
            "/api/v1/security/ssh-keys/{id}",
            axum::routing::delete(security::ssh_key_remove),
        )
        .route("/ws/dashboard", get(crate::ws::dashboard_ws))
        .route("/ws/logs", get(crate::ws::logs_ws))
        .route("/api/v1/sites/{id}/ssl/issue", post(sites::issue_ssl))
        .route("/api/v1/sites/{id}/ssl/status", get(sites::ssl_status))
        .route("/api/v1/sites/{id}/suspend", post(sites::suspend))
        .route("/api/v1/sites/{id}/unsuspend", post(sites::unsuspend))
        .route("/api/v1/php/versions", get(sites::php_versions))
        .route("/api/v1/php/status", get(sites::php_status))
        .route(
            "/api/v1/sites/{id}",
            get(sites::get).patch(sites::update).delete(sites::delete),
        )
        .route("/api/v1/backups", get(backups::list))
        .route("/api/v1/backups/run", post(backups::run))
        .route("/api/v1/backups/schedules", post(backups::schedule_create))
        .route(
            "/api/v1/backups/schedules/{id}",
            axum::routing::delete(backups::schedule_delete),
        )
        .route(
            "/api/v1/backups/{id}",
            axum::routing::delete(backups::delete),
        )
        .route("/api/v1/backups/{id}/verify", post(backups::verify))
        .route("/api/v1/backups/{id}/restore", post(backups::restore))
        .route("/api/v1/backups/{id}/download", get(backups::download))
        .route("/api/v1/cron", get(cron::list).post(cron::create))
        .route("/api/v1/cron/describe", get(cron::describe))
        .route(
            "/api/v1/cron/{id}",
            patch(cron::update).delete(cron::delete),
        )
        .route("/api/v1/cron/{id}/run", post(cron::run))
        .route("/api/v1/cron/{id}/runs", get(cron::runs))
        .route("/api/v1/logs", get(logs::read))
        .route("/api/v1/logs/sources", get(logs::sources))
        .route("/api/v1/files", get(files::list).delete(files::delete))
        .route("/api/v1/files/content", get(files::read).put(files::write))
        .route("/api/v1/files/dir", post(files::mkdir))
        .route("/api/v1/files/rename", post(files::rename))
        .route("/api/v1/files/permissions", patch(files::chmod))
        .route("/api/v1/files/archive", post(files::archive))
        .route("/api/v1/files/extract", post(files::extract))
        .route("/api/v1/api-keys", get(apikeys::list).post(apikeys::create))
        .route(
            "/api/v1/api-keys/{id}",
            axum::routing::delete(apikeys::delete),
        )
        .route("/api/v1/alerts", get(alerts::list))
        .route("/api/v1/alerts/test", post(alerts::test))
        .route(
            "/api/v1/alerts/channels",
            get(alerts::channels).post(alerts::create_channel),
        )
        .route(
            "/api/v1/alerts/channels/{id}",
            axum::routing::delete(alerts::delete_channel),
        )
        .route(
            "/api/v1/alerts/rules",
            get(alerts::rules).post(alerts::create_rule),
        )
        .route(
            "/api/v1/alerts/rules/{id}",
            axum::routing::delete(alerts::delete_rule),
        )
        .route(
            "/api/v1/deployments",
            get(deployments::list).post(deployments::create),
        )
        .route(
            "/api/v1/deployments/{id}",
            patch(deployments::update).delete(deployments::delete),
        )
        .route("/api/v1/deployments/{id}/run", post(deployments::run))
        .route("/api/v1/deployments/{id}/runs", get(deployments::runs))
        .route("/api/v1/terminal", get(terminal::list))
        .route("/api/v1/terminal/open", post(terminal::open))
        .route(
            "/api/v1/terminal/{id}",
            axum::routing::delete(terminal::close),
        )
        .route("/api/v1/terminal/{id}/write", post(terminal::write))
        .route("/api/v1/terminal/{id}/read", get(terminal::read))
        .route("/api/v1/terminal/{id}/resize", post(terminal::resize))
        .route("/ws/terminal/{id}", get(crate::ws::terminal_ws))
        .route(
            "/.well-known/acme-challenge/{token}",
            get(crate::acme::challenge),
        )
        .route("/api/v1/panel/ssl/status", get(panel::status))
        .route("/api/v1/panel/ssl/issue", post(panel::issue))
        .route("/api/v1/update", get(update::status))
        .route("/api/v1/update/check", post(update::check))
        .route("/api/v1/update/apply", post(update::apply))
        .route("/api/v1/update/rollback", post(update::rollback))
        .with_state(state)
}

// Copyright (C) 2026 Binkap S
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The WebAssembly bridge: Slint callbacks in, API JSON out.
//!
//! Every function takes a generated `AppWindow` handle and drives its
//! properties and models. The UI is the only consumer; the server-side
//! `synthpanel-web` process is untouched.

use slint::{ComponentHandle, Model};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use crate::api::{self, http, set_global};
use crate::models;
use crate::AppWindow;

/// Wrap a plain vector in the `ModelRc` the generated setters expect.
fn model<T: Clone + 'static>(items: Vec<T>) -> slint::ModelRc<T> {
    slint::ModelRc::new(slint::VecModel::from(items))
}

/// Per-window runtime state.
pub struct Bridge {
    app: RefCell<Rc<AppWindow>>,
    /// The last dashboard metrics snapshot, kept so the next one can be
    /// converted into a network *rate* (the core reports byte counters).
    prev_snapshot: RefCell<Option<serde_json::Value>>,
}

pub fn install() -> Result<(), JsValue> {
    let app = AppWindow::new().map_err(|error| JsValue::from_str(&error.to_string()))?;
    let app = Rc::new(app);

    let dark = api::storage_get("synthpanel-theme").as_deref() == Some("dark");
    app.set_dark(dark);
    api::apply_theme(if dark { "dark" } else { "light" });

    let bridge = Bridge {
        app: RefCell::new(app.clone()),
        prev_snapshot: RefCell::new(None),
    };
    let bridge = Rc::new(bridge);

    register_callbacks(&bridge);
    register_socket_callbacks(&bridge);
    load_session(&bridge);

    app.show()
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Callbacks
// ---------------------------------------------------------------------------

fn register_callbacks(bridge: &Rc<Bridge>) {
    let app = bridge.app.borrow().clone();

    let b = Rc::clone(bridge);
    app.on_logout(move || {
        let b = Rc::clone(&b);
        let _ = slint::spawn_local(async move {
            let _ = http("POST", "/api/v1/auth/logout", None).await;
            let app = &b.app.borrow();
            app.set_loading(false);
            app.set_authenticated(false);
            app.set_current_page(crate::Page::Dashboard);
        });
    });

    let b = Rc::clone(bridge);
    app.on_toggle_theme(move || {
        let b = Rc::clone(&b);
        let dark = !b.app.borrow().get_dark();
        b.app.borrow_mut().set_dark(dark);
        api::storage_set("synthpanel-theme", if dark { "dark" } else { "light" });
        api::apply_theme(if dark { "dark" } else { "light" });
    });

    let b = Rc::clone(bridge);
    app.on_login(move |email, password| {
        let b = Rc::clone(&b);
        let app = b.app.borrow().clone();
        let email = email.to_string();
        let password = password.to_string();
        let _ = slint::spawn_local(async move {
            app.set_login_busy(true);
            let result =
                http("POST", "/api/v1/auth/login", Some(serde_json::json!({ "email": email, "password": password }))).await;
            match result {
                Ok(body) => {
                    if let Some(challenge) = body.get("challenge_id").and_then(serde_json::Value::as_str) {
                        api::storage_set("synthpanel-2fa", challenge);
                        app.set_login_needs_code(true);
                    } else {
                        finish_login(&b, &body);
                    }
                }
                Err(message) => {
                    app.set_login_error(message.into());
                    app.set_login_busy(false);
                }
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_verify(move |code| {
        let b = Rc::clone(&b);
        let app = b.app.borrow().clone();
        let code = code.to_string();
        let challenge = api::storage_get("synthpanel-2fa").unwrap_or_default();
        let _ = slint::spawn_local(async move {
            app.set_login_busy(true);
            let result =
                http("POST", "/api/v1/auth/2fa/verify", Some(serde_json::json!({ "challenge_id": challenge, "code": code }))).await;
            match result {
                Ok(_) => {
                    api::storage_set("synthpanel-2fa", "");
                    match http("GET", "/api/v1/auth/me", None).await {
                        Ok(user) => finish_login(&b, &user),
                        Err(message) => {
                            app.set_login_error(message.into());
                            app.set_login_busy(false);
                        }
                    }
                }
                Err(message) => {
                    app.set_login_error(message.into());
                    app.set_login_busy(false);
                }
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_reset_login(move || {
        b.app.borrow_mut().set_login_needs_code(false);
        b.app.borrow_mut().set_login_error("".into());
        b.app.borrow_mut().set_login_code("".into());
    });

    let b = Rc::clone(bridge);
    app.on_navigate(move |page| {
        let b = Rc::clone(&b);
        match page {
            crate::Page::Dashboard => {
                b.app.borrow_mut().set_current_page(crate::Page::Dashboard);
                load_dashboard(&b);
            }
            crate::Page::Sites => {
                b.app.borrow_mut().set_current_page(crate::Page::Sites);
                load_sites(&b);
            }
            crate::Page::SiteDetail => {
                b.app.borrow_mut().set_current_page(crate::Page::SiteDetail);
                load_site_detail(&b);
            }
            crate::Page::Databases => {
                b.app.borrow_mut().set_current_page(crate::Page::Databases);
                load_databases(&b);
            }
            crate::Page::Php => {
                b.app.borrow_mut().set_current_page(crate::Page::Php);
                load_php(&b);
            }
            crate::Page::Security => {
                b.app.borrow_mut().set_current_page(crate::Page::Security);
                load_security(&b);
            }
            crate::Page::Backups => {
                b.app.borrow_mut().set_current_page(crate::Page::Backups);
                load_backups(&b);
            }
            crate::Page::Cron => {
                b.app.borrow_mut().set_current_page(crate::Page::Cron);
                load_cron(&b);
            }
            crate::Page::Logs => {
                b.app.borrow_mut().set_current_page(crate::Page::Logs);
                load_logs(&b);
            }
            crate::Page::Files => {
                b.app.borrow_mut().set_current_page(crate::Page::Files);
                load_files(&b, "");
            }
            crate::Page::Deployments => {
                b.app.borrow_mut().set_current_page(crate::Page::Deployments);
                load_deployments(&b);
            }
            crate::Page::Alerts => {
                b.app.borrow_mut().set_current_page(crate::Page::Alerts);
                load_alerts(&b);
            }
            crate::Page::Terminal => {
                b.app.borrow_mut().set_current_page(crate::Page::Terminal);
                load_terminal(&b);
            }
            crate::Page::Users => {
                b.app.borrow_mut().set_current_page(crate::Page::Users);
                load_users(&b);
            }
            crate::Page::Settings => {
                b.app.borrow_mut().set_current_page(crate::Page::Settings);
                load_settings(&b);
            }
        }
    });

    let b = Rc::clone(bridge);
    app.on_open_site(move |id| {
        let b = Rc::clone(&b);
        b.app.borrow_mut().set_site_detail_id(id.to_string().into());
        b.app.borrow_mut().set_current_page(crate::Page::SiteDetail);
        load_site_detail(&b);
    });

    // --- Sites mutations ----------------------------------------------------
    let b = Rc::clone(bridge);
    app.on_create_site(
        move |domain, aliases, template, php_version, ssl_mode, document_root| {
            let b = Rc::clone(&b);
            let aliases: Vec<String> = aliases
                .as_str()
                .split(',')
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect();
            let body = serde_json::json!({
                "domain": domain.as_str(),
                "aliases": aliases,
                "template": template.as_str(),
                "php_version": (php_version.as_str().is_empty() || template.as_str() == "static" || template.as_str() == "node")
                    .then_some(serde_json::Value::Null),
                "ssl_mode": ssl_mode.as_str(),
                "document_root": document_root.as_str()
            });
            let _ = slint::spawn_local(async move {
                match http("POST", "/api/v1/sites", Some(body)).await {
                    Ok(_) => {
                        load_sites(&b);
                        b.app.borrow_mut().set_sites_status("".into());
                    }
                    Err(message) => b.app.borrow_mut().set_sites_status(message.into()),
                }
            });
        },
    );

    let b = Rc::clone(bridge);
    app.on_delete_site(move |id| {
        let b = Rc::clone(&b);
        let _ = slint::spawn_local(async move {
            match http("DELETE", &format!("/api/v1/sites/{id}"), None).await {
                Ok(_) => {
                    load_sites(&b);
                    b.app.borrow_mut().set_sites_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_sites_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_site_action(move |id, action| {
        let b = Rc::clone(&b);
        let action = action.to_string();
        let _ = slint::spawn_local(async move {
            match http("POST", &format!("/api/v1/sites/{id}/{action}"), None).await {
                Ok(_) => {
                    load_site_detail(&b);
                    load_sites(&b);
                }
                Err(message) => b.app.borrow_mut().set_site_error(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_site_ssl(move |id| {
        let b = Rc::clone(&b);
        let _ = slint::spawn_local(async move {
            match http("POST", &format!("/api/v1/sites/{id}/ssl/issue"), None).await {
                Ok(body) => {
                    b.app.borrow_mut().set_site_ssl_message(
                        body.get("message")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("certificate issued")
                            .to_string()
                            .into(),
                    );
                    load_site_detail(&b);
                }
                Err(message) => b.app.borrow_mut().set_site_ssl_message(message.into()),
            }
        });
    });

    // --- Databases ----------------------------------------------------------
    let b = Rc::clone(bridge);
    app.on_create_database(move |name, user, engine| {
        let b = Rc::clone(&b);
        let body = serde_json::json!({
            "name": name.as_str(),
            "username": user.as_str().to_string().is_empty().then_some(serde_json::Value::Null).or_else(|| Some(serde_json::json!(user.as_str()))),
            "engine": engine.as_str(),
        });
        let _ = slint::spawn_local(async move {
            match http("POST", "/api/v1/databases", Some(body)).await {
                Ok(_) => {
                    load_databases(&b);
                    b.app.borrow_mut().set_databases_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_databases_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_delete_database(move |id| {
        let b = Rc::clone(&b);
        let _ = slint::spawn_local(async move {
            match http("DELETE", &format!("/api/v1/databases/{id}"), None).await {
                Ok(_) => {
                    load_databases(&b);
                    b.app.borrow_mut().set_databases_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_databases_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_add_db_user(move |database, user, password, privileges| {
        let b = Rc::clone(&b);
        let body = serde_json::json!({
            "username": user.as_str(),
            "password": password.as_str().to_string().is_empty().then_some(serde_json::Value::Null).or_else(|| Some(serde_json::json!(password.as_str()))),
            "privileges": privileges.as_str().to_string().is_empty().then_some(serde_json::Value::Null).or_else(|| Some(serde_json::json!(privileges.as_str()))),
        });
        let _ = slint::spawn_local(async move {
            match http("POST", &format!("/api/v1/databases/{database}/users"), Some(body)).await {
                Ok(_) => {
                    load_databases(&b);
                    b.app.borrow_mut().set_databases_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_databases_status(message.into()),
            }
        });
    });

    // --- Users ----------------------------------------------------------------
    let b = Rc::clone(bridge);
    app.on_create_user(move |email, name, password, role, sites, dbs| {
        let b = Rc::clone(&b);
        let body = serde_json::json!({
            "email": email.as_str(),
            "name": name.as_str(),
            "password": password.as_str(),
            "role": role.as_str(),
            "quota_sites": sites,
            "quota_dbs": dbs
        });
        let _ = slint::spawn_local(async move {
            match http("POST", "/api/v1/users", Some(body)).await {
                Ok(_) => {
                    load_users(&b);
                    b.app.borrow_mut().set_users_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_users_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_update_user(move |id, email, name, role, status, totp, sites, dbs| {
        let b = Rc::clone(&b);
        let body = serde_json::json!({
            "email": email.as_str(),
            "name": name.as_str(),
            "role": role.as_str(),
            "status": status.as_str(),
            "totp_enabled": totp.as_str() == "on",
            "quota_sites": sites,
            "quota_dbs": dbs
        });
        let _ = slint::spawn_local(async move {
            match http("PATCH", &format!("/api/v1/users/{id}"), Some(body)).await {
                Ok(_) => {
                    load_users(&b);
                    b.app.borrow_mut().set_users_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_users_status(message.into()),
            }
        });
    });

    // --- Security -------------------------------------------------------------
    let b = Rc::clone(bridge);
    app.on_firewall_apply(move |port, proto, source, action, comment| {
        let b = Rc::clone(&b);
        let body = serde_json::json!({
            "port": port.as_str().parse::<u16>().unwrap_or(0),
            "proto": proto.as_str(),
            "source": source.as_str(),
            "action": action.as_str(),
            "comment": comment.as_str()
        });
        let _ = slint::spawn_local(async move {
            match http("POST", "/api/v1/security/firewall/rules", Some(body)).await {
                Ok(_) => {
                    load_security(&b);
                    b.app.borrow_mut().set_security_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_security_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_unban(move |jail, ip| {
        let b = Rc::clone(&b);
        let body = serde_json::json!({ "jail": jail.as_str(), "ip": ip.as_str() });
        let _ = slint::spawn_local(async move {
            match http("POST", "/api/v1/security/fail2ban/unban", Some(body)).await {
                Ok(_) => {
                    load_security(&b);
                    b.app.borrow_mut().set_security_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_security_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_add_ssh_key(move |label, key, user| {
        let b = Rc::clone(&b);
        let body = serde_json::json!({
            "label": label.as_str(),
            "public_key": key.as_str(),
            "user": user.as_str()
        });
        let _ = slint::spawn_local(async move {
            match http("POST", "/api/v1/security/ssh-keys", Some(body)).await {
                Ok(_) => {
                    load_security(&b);
                    b.app.borrow_mut().set_security_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_security_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_remove_ssh_key(move |id| {
        let b = Rc::clone(&b);
        let _ = slint::spawn_local(async move {
            match http("DELETE", &format!("/api/v1/security/ssh-keys/{id}"), None).await {
                Ok(_) => {
                    load_security(&b);
                    b.app.borrow_mut().set_security_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_security_status(message.into()),
            }
        });
    });

    // --- Backups ----------------------------------------------------------------
    let b = Rc::clone(bridge);
    app.on_run_backup(move |_name, kind, site| {
        let b = Rc::clone(&b);
        let body = serde_json::json!({
            "kind": kind.as_str(),
            "site_id": site.as_str().is_empty().then_some(serde_json::Value::Null).or_else(|| Some(serde_json::json!(site.as_str()))),
        });
        let _ = slint::spawn_local(async move {
            match http("POST", "/api/v1/backups/run", Some(body)).await {
                Ok(_) => {
                    load_backups(&b);
                    b.app.borrow_mut().set_backups_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_backups_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_backup_action(move |id, action, passphrase| {
        let b = Rc::clone(&b);
        let body = serde_json::json!({ "passphrase": passphrase.as_str() });
        let _ = slint::spawn_local(async move {
            match http("POST", &format!("/api/v1/backups/{id}/{action}"), Some(body)).await {
                Ok(_) => {
                    load_backups(&b);
                    b.app.borrow_mut().set_backups_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_backups_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_delete_backup(move |id| {
        let b = Rc::clone(&b);
        let _ = slint::spawn_local(async move {
            match http("DELETE", &format!("/api/v1/backups/{id}"), None).await {
                Ok(_) => {
                    load_backups(&b);
                    b.app.borrow_mut().set_backups_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_backups_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_create_schedule(move |name, cron, destination_type, path, _site| {
        let b = Rc::clone(&b);
        let body = serde_json::json!({
            "name": name.as_str(),
            "cron": cron.as_str(),
            "destination": { "type": destination_type.as_str(), "path": path.as_str() }
        });
        let _ = slint::spawn_local(async move {
            match http("POST", "/api/v1/backups/schedules", Some(body)).await {
                Ok(_) => {
                    load_backups(&b);
                    b.app.borrow_mut().set_backups_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_backups_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_delete_schedule(move |id| {
        let b = Rc::clone(&b);
        let _ = slint::spawn_local(async move {
            match http("DELETE", &format!("/api/v1/backups/schedules/{id}"), None).await {
                Ok(_) => {
                    load_backups(&b);
                    b.app.borrow_mut().set_backups_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_backups_status(message.into()),
            }
        });
    });

    // --- Cron ---------------------------------------------------------------------
    let b = Rc::clone(bridge);
    app.on_create_cron(move |expression, command, user, site, _id| {
        let b = Rc::clone(&b);
        let body = serde_json::json!({
            "expression": expression.as_str(),
            "command": command.as_str(),
            "user": user.as_str().is_empty().then_some(serde_json::Value::Null).or_else(|| Some(serde_json::json!(user.as_str()))),
            "site_id": site.as_str().is_empty().then_some(serde_json::Value::Null).or_else(|| Some(serde_json::json!(site.as_str()))),
        });
        let _ = slint::spawn_local(async move {
            match http("POST", "/api/v1/cron", Some(body)).await {
                Ok(_) => {
                    load_cron(&b);
                    b.app.borrow_mut().set_cron_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_cron_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_delete_cron(move |id| {
        let b = Rc::clone(&b);
        let _ = slint::spawn_local(async move {
            match http("DELETE", &format!("/api/v1/cron/{id}"), None).await {
                Ok(_) => {
                    load_cron(&b);
                    b.app.borrow_mut().set_cron_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_cron_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_run_cron(move |id| {
        let b = Rc::clone(&b);
        let _ = slint::spawn_local(async move {
            match http("POST", &format!("/api/v1/cron/{id}/run"), None).await {
                Ok(_) => {
                    load_cron(&b);
                    b.app.borrow_mut().set_cron_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_cron_status(message.into()),
            }
        });
    });

    // --- Logs ----------------------------------------------------------------------
    let b = Rc::clone(bridge);
    app.on_log_source(move |source| {
        let b = Rc::clone(&b);
        let source = source.to_string();
        let _ = slint::spawn_local(async move {
            b.app.borrow_mut().set_active_log_source(source.clone().into());
            load_log_tail(&b, &source, "");
        });
    });

    let b = Rc::clone(bridge);
    app.on_log_search(move |search| {
        let b = Rc::clone(&b);
        let source = b.app.borrow().get_active_log_source().to_string();
        if source.is_empty() {
            return;
        }
        let _ = slint::spawn_local(async move {
            load_log_tail(&b, &source, search.as_str());
        });
    });

    // --- Files ------------------------------------------------------------------------
    let b = Rc::clone(bridge);
    app.on_file_path(move |path| {
        let b = Rc::clone(&b);
        load_files(&b, path.as_str());
    });

    let b = Rc::clone(bridge);
    app.on_file_write(move |scope, path, content| {
        let b = Rc::clone(&b);
        let body = serde_json::json!({
            "scope": scope.as_str(),
            "path": path.as_str(),
            "content": content.as_str()
        });
        let _ = slint::spawn_local(async move {
            match http("PUT", "/api/v1/files/content", Some(body)).await {
                Ok(_) => {
                    b.app.borrow_mut().set_files_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_files_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_file_mkdir(move |scope, path| {
        let b = Rc::clone(&b);
        let body = serde_json::json!({ "scope": scope.as_str(), "path": path.as_str() });
        let _ = slint::spawn_local(async move {
            match http("POST", "/api/v1/files/dir", Some(body)).await {
                Ok(_) => {
                    load_files(&b, &b.app.borrow().get_current_file_path());
                    b.app.borrow_mut().set_files_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_files_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_file_delete(move |scope, path| {
        let b = Rc::clone(&b);
        let _ = slint::spawn_local(async move {
            let query = format!("scope={}&path={}", scope, percent_encode(&path));
            match http("DELETE", &format!("/api/v1/files?{query}"), None).await {
                Ok(_) => {
                    load_files(&b, &b.app.borrow().get_current_file_path());
                    b.app.borrow_mut().set_files_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_files_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_file_renamed(move |scope, from, to| {
        let b = Rc::clone(&b);
        let body = serde_json::json!({
            "scope": scope.as_str(),
            "from": from.as_str(),
            "to": to.as_str()
        });
        let _ = slint::spawn_local(async move {
            match http("POST", "/api/v1/files/rename", Some(body)).await {
                Ok(_) => {
                    load_files(&b, &b.app.borrow().get_current_file_path());
                    b.app.borrow_mut().set_files_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_files_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_file_chmod(move |scope, path, mode| {
        let b = Rc::clone(&b);
        let body = serde_json::json!({
            "scope": scope.as_str(),
            "path": path.as_str(),
            "mode": mode.as_str()
        });
        let _ = slint::spawn_local(async move {
            match http("PATCH", "/api/v1/files/permissions", Some(body)).await {
                Ok(_) => {
                    load_files(&b, &b.app.borrow().get_current_file_path());
                    b.app.borrow_mut().set_files_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_files_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_file_content(move |scope, path| {
        let b = Rc::clone(&b);
        let _ = slint::spawn_local(async move {
            let query = format!("scope={}&path={}", scope, percent_encode(&path));
            match http("GET", &format!("/api/v1/files/content?{query}"), None).await {
                Ok(body) => {
                    let content = body.get("content").and_then(serde_json::Value::as_str).unwrap_or_default();
                    b.app.borrow_mut().set_file_editor_content(content.to_string().into());
                }
                Err(message) => b.app.borrow_mut().set_files_status(message.into()),
            }
        });
    });

    // --- Deployments --------------------------------------------------------------------
    let b = Rc::clone(bridge);
    app.on_create_deployment(
        move |site, repo, branch, subdir, build, user, enabled, _id| {
            let b = Rc::clone(&b);
            let body = serde_json::json!({
                "site_id": site.as_str(),
                "repo_url": repo.as_str(),
                "branch": branch.as_str(),
                "target_subdir": subdir.as_str().is_empty().then_some(serde_json::Value::Null).or_else(|| Some(serde_json::json!(subdir.as_str()))),
                "build_command": build.as_str().is_empty().then_some(serde_json::Value::Null).or_else(|| Some(serde_json::json!(build.as_str()))),
                "deploy_user": user.as_str(),
                "enabled": enabled.as_str() == "true"
            });
            let _ = slint::spawn_local(async move {
                match http("POST", "/api/v1/deployments", Some(body)).await {
                    Ok(_) => {
                        load_deployments(&b);
                        b.app.borrow_mut().set_deployments_status("".into());
                    }
                    Err(message) => b.app.borrow_mut().set_deployments_status(message.into()),
                }
            });
        },
    );

    let b = Rc::clone(bridge);
    app.on_run_deployment(move |id| {
        let b = Rc::clone(&b);
        let _ = slint::spawn_local(async move {
            match http("POST", &format!("/api/v1/deployments/{id}/run"), None).await {
                Ok(_) => {
                    load_deployments(&b);
                    b.app.borrow_mut().set_deployments_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_deployments_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_delete_deployment(move |id| {
        let b = Rc::clone(&b);
        let _ = slint::spawn_local(async move {
            match http("DELETE", &format!("/api/v1/deployments/{id}"), None).await {
                Ok(_) => {
                    load_deployments(&b);
                    b.app.borrow_mut().set_deployments_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_deployments_status(message.into()),
            }
        });
    });

    // --- Alerts ----------------------------------------------------------------------------
    let b = Rc::clone(bridge);
    app.on_create_channel(move |name, kind, url, to, host| {
        let b = Rc::clone(&b);
        let host = host.as_str();
        let config = if kind.as_str() == "email" {
            let to_array: Vec<serde_json::Value> = to
                .as_str()
                .split(',')
                .map(|recipient| serde_json::json!(recipient.trim()))
                .filter(|value| !value.as_str().unwrap_or_default().is_empty())
                .collect();
            let smtp = serde_json::json!({ "host": host });
            serde_json::json!({ "to": to_array, "smtp": smtp })
        } else {
            serde_json::json!({ "url": url.as_str() })
        };
        let body = serde_json::json!({
            "name": name.as_str(),
            "kind": kind.as_str(),
            "config": config
        });
        let _ = slint::spawn_local(async move {
            match http("POST", "/api/v1/alerts/channels", Some(body)).await {
                Ok(_) => {
                    load_alerts(&b);
                    b.app.borrow_mut().set_alerts_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_alerts_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_delete_channel(move |id| {
        let b = Rc::clone(&b);
        let _ = slint::spawn_local(async move {
            match http("DELETE", &format!("/api/v1/alerts/channels/{id}"), None).await {
                Ok(_) => {
                    load_alerts(&b);
                    b.app.borrow_mut().set_alerts_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_alerts_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_create_rule(move |name, event, severity, channel, cooldown| {
        let b = Rc::clone(&b);
        let body = serde_json::json!({
            "name": name.as_str(),
            "event": event.as_str(),
            "severity": severity.as_str(),
            "channel_id": channel.as_str(),
            "cooldown_secs": cooldown
        });
        let _ = slint::spawn_local(async move {
            match http("POST", "/api/v1/alerts/rules", Some(body)).await {
                Ok(_) => {
                    load_alerts(&b);
                    b.app.borrow_mut().set_alerts_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_alerts_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_delete_rule(move |id| {
        let b = Rc::clone(&b);
        let _ = slint::spawn_local(async move {
            match http("DELETE", &format!("/api/v1/alerts/rules/{id}"), None).await {
                Ok(_) => {
                    load_alerts(&b);
                    b.app.borrow_mut().set_alerts_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_alerts_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_test_alert(move |channel, message| {
        let b = Rc::clone(&b);
        let body = serde_json::json!({
            "channel_id": channel.as_str(),
            "message": message.as_str()
        });
        let _ = slint::spawn_local(async move {
            match http("POST", "/api/v1/alerts/test", Some(body)).await {
                Ok(_) => b.app.borrow_mut().set_alerts_status("".into()),
                Err(message) => b.app.borrow_mut().set_alerts_status(message.into()),
            }
        });
    });

    // --- Terminal --------------------------------------------------------------------------
    let b = Rc::clone(bridge);
    app.on_terminal_close(move || {
        let b = Rc::clone(&b);
        let id = b.app.borrow().get_terminal_id().to_string();
        if !id.is_empty() {
            api::socket_close(2);
            let _ = slint::spawn_local(async move {
                let _ = http("DELETE", &format!("/api/v1/terminal/{id}"), None).await;
            });
        }
    });

    let b = Rc::clone(bridge);
    app.on_terminal_input(move |data| {
        let _ = &b;
        let frame = format!(
            "{{\"type\":\"input\",\"data\":{}}}",
            serde_json::to_string(data.as_str()).unwrap_or_default()
        );
        api::socket_send(2, &frame);
    });

    let b = Rc::clone(bridge);
    app.on_terminal_resize(move |rows, cols| {
        let _ = &b;
        api::socket_send(2, &format!(r#"{{"type":"resize","rows":{rows},"cols":{cols}}}"#));
    });

    // --- Settings ------------------------------------------------------------------------------
    let b = Rc::clone(bridge);
    app.on_save_settings(
        move |hostname, timezone, locale, _channel, _lifetime, _maintenance, _ssl, _enabled| {
            let b = Rc::clone(&b);
            let body = serde_json::json!({
                "hostname": hostname.as_str(),
                "timezone": timezone.as_str(),
                "locale": locale.as_str()
            });
            let _ = slint::spawn_local(async move {
                match http("PATCH", "/api/v1/settings", Some(body)).await {
                    Ok(_) => {
                        load_settings(&b);
                        b.app.borrow_mut().set_settings_status("".into());
                    }
                    Err(message) => b.app.borrow_mut().set_settings_status(message.into()),
                }
            });
        },
    );

    let b = Rc::clone(bridge);
    app.on_api_key_created(move |name, scopes| {
        let b = Rc::clone(&b);
        let scope_list: Vec<serde_json::Value> = scopes
            .as_str()
            .split(',')
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .into_iter()
            .map(serde_json::Value::String)
            .collect();
        let body = serde_json::json!({
            "name": name.as_str(),
            "scopes": scope_list
        });
        let _ = slint::spawn_local(async move {
            match http("POST", "/api/v1/api-keys", Some(body)).await {
                Ok(body) => {
                    let token = body.get("token").and_then(serde_json::Value::as_str).unwrap_or_default();
                    b.app.borrow_mut().set_api_key_reveal(token.to_string().into());
                    load_settings(&b);
                }
                Err(message) => b.app.borrow_mut().set_settings_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_api_key_deleted(move |id| {
        let b = Rc::clone(&b);
        let _ = slint::spawn_local(async move {
            match http("DELETE", &format!("/api/v1/api-keys/{id}"), None).await {
                Ok(_) => {
                    load_settings(&b);
                    b.app.borrow_mut().set_settings_status("".into());
                }
                Err(message) => b.app.borrow_mut().set_settings_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_update_check(move || {
        let b = Rc::clone(&b);
        let _ = slint::spawn_local(async move {
            match http("POST", "/api/v1/update/check", None).await {
                Ok(_) => load_settings(&b),
                Err(message) => b.app.borrow_mut().set_settings_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_update_apply(move || {
        let b = Rc::clone(&b);
        let _ = slint::spawn_local(async move {
            match http("POST", "/api/v1/update/apply", None).await {
                Ok(_) => load_settings(&b),
                Err(message) => b.app.borrow_mut().set_settings_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_panel_ssl_issue(move || {
        let b = Rc::clone(&b);
        let _ = slint::spawn_local(async move {
            match http("POST", "/api/v1/panel/ssl/issue", None).await {
                Ok(_) => load_settings(&b),
                Err(message) => b.app.borrow_mut().set_settings_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_twofa_setup(move |password| {
        let b = Rc::clone(&b);
        let body = serde_json::json!({ "password": password.as_str() });
        let _ = slint::spawn_local(async move {
            match http("POST", "/api/v1/auth/2fa/setup", Some(body)).await {
                Ok(body) => {
                    let secret = body.get("secret").and_then(serde_json::Value::as_str).unwrap_or_default();
                    let uri = body.get("otpauth_url").and_then(serde_json::Value::as_str).unwrap_or_default();
                    let qr = body.get("qr_code").and_then(serde_json::Value::as_str).unwrap_or_default();
                    b.app.borrow_mut().set_twofa_secret(secret.to_string().into());
                    b.app.borrow_mut().set_twofa_uri(uri.to_string().into());
                    b.app.borrow_mut().set_twofa_qr(qr.to_string().into());
                }
                Err(message) => b.app.borrow_mut().set_settings_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_twofa_enable(move |code| {
        let b = Rc::clone(&b);
        let body = serde_json::json!({ "code": code.as_str() });
        let _ = slint::spawn_local(async move {
            match http("POST", "/api/v1/auth/2fa/enable", Some(body)).await {
                Ok(_) => load_settings(&b),
                Err(message) => b.app.borrow_mut().set_settings_status(message.into()),
            }
        });
    });

    let b = Rc::clone(bridge);
    app.on_twofa_disable(move |password| {
        let b = Rc::clone(&b);
        let body = serde_json::json!({ "password": password.as_str() });
        let _ = slint::spawn_local(async move {
            match http("POST", "/api/v1/auth/2fa/disable", Some(body)).await {
                Ok(_) => load_settings(&b),
                Err(message) => b.app.borrow_mut().set_settings_status(message.into()),
            }
        });
    });
}

fn load_log_tail(bridge: &Rc<Bridge>, source: &str, search: &str) {
    let app = bridge.app.borrow().clone();
    let source = source.to_string();
    let search = search.to_string();
    let query = if search.is_empty() {
        format!("/api/v1/logs?type={source}&limit=200")
    } else {
        format!(
            "/api/v1/logs?type={source}&limit=200&search={}",
            percent_encode(&search)
        )
    };
    let _ = slint::spawn_local(async move {
        match http("GET", &query, None).await {
            Ok(body) => app.set_log_lines(model(models::to_log_lines(&body))),
            Err(message) => app.set_logs_status(message.into()),
        }
    });
}

fn register_socket_callbacks(bridge: &Rc<Bridge>) {
    let message_bridge = Rc::clone(bridge);
    let message = wasm_bindgen::closure::Closure::<dyn FnMut(JsValue)>::new(move |value: JsValue| {
        let array = match value.dyn_ref::<js_sys::Array>() {
            Some(array) => array,
            None => return,
        };
        let id = array.get(0).as_f64().unwrap_or(0.0) as u32;
        let data = array.get(1).as_string().unwrap_or_default();
        if let Some(bridge) = Rc::downgrade(&message_bridge).upgrade() {
            bridge.on_socket_message(id, &data);
        }
    });
    set_global("synthpanel_socket_message", message);

    let close_bridge = Rc::clone(bridge);
    let close = wasm_bindgen::closure::Closure::<dyn FnMut(JsValue)>::new(move |value: JsValue| {
        let id = value.as_f64().unwrap_or(0.0) as u32;
        if let Some(bridge) = Rc::downgrade(&close_bridge).upgrade() {
            bridge.on_socket_close(id);
        }
    });
    set_global("synthpanel_socket_close", close);
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

fn load_session(bridge: &Rc<Bridge>) {
    let bridge = Rc::clone(bridge);
    let app = bridge.app.borrow().clone();
    let _ = slint::spawn_local(async move {
        app.set_loading(true);
        match http("GET", "/api/v1/auth/me", None).await {
            Ok(user) => {
                apply_session(&bridge, &user);
            }
            Err(_) => {
                app.set_loading(false);
                app.set_authenticated(false);
            }
        }
    });
}

fn apply_session(bridge: &Rc<Bridge>, user: &serde_json::Value) {
    let session = models::user_session(user);
    let app = bridge.app.borrow_mut();
    app.set_session(session);
    app.set_login_error("".into());
    app.set_login_needs_code(false);
    app.set_authenticated(true);
    app.set_loading(false);
    app.set_current_page(crate::Page::Dashboard);
}

fn finish_login(bridge: &Rc<Bridge>, user: &serde_json::Value) {
    let session = models::user_session(user);
    let app = bridge.app.borrow_mut();
    app.set_session(session);
    app.set_login_error("".into());
    app.set_login_needs_code(false);
    app.set_login_busy(false);
    app.set_authenticated(true);
    app.set_current_page(crate::Page::Dashboard);
    drop(app);
    load_dashboard(bridge);
}

// ---------------------------------------------------------------------------
// Pages
// ---------------------------------------------------------------------------

fn load_dashboard(bridge: &Rc<Bridge>) {
    let app = bridge.app.borrow().clone();
    let _ = slint::spawn_local(async move {
        let snapshot = http("GET", "/api/v1/metrics/snapshot", None).await;
        let range = http("GET", "/api/v1/metrics/range?range=24h", None).await;
        let services = http("GET", "/api/v1/services", None).await;
        let audit = http("GET", "/api/v1/audit?limit=12", None).await;

        match snapshot {
            Ok(body) => {
                app.set_dash_snapshot(models::metric_snapshot(&body));
                let range_body = range.ok().unwrap_or(serde_json::Value::Null);
                let points = range_body
                    .get("points")
                    .and_then(serde_json::Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let cpu: Vec<f32> = points
                    .iter()
                    .filter_map(|point| point.get("cpu").and_then(serde_json::Value::as_f64).map(|value| value as f32))
                    .collect();
                let mem: Vec<f32> = points
                    .iter()
                    .filter_map(|point| {
                        let used = point.get("mem_used")?.as_f64()? as f32;
                        let total = point.get("mem_total")?.as_f64()? as f32;
                        let percent = if total > 0.0 { used / total * 100.0 } else { 0.0 };
                        Some(percent)
                    })
                    .collect();
                app.set_dash_cpu_spark(models::sparkline_path(&cpu).into());
                app.set_dash_mem_spark(models::sparkline_path(&mem).into());
                app.set_dash_status("".into());
            }
            Err(message) => app.set_dash_status(message.into()),
        }
        if let Ok(body) = services {
            app.set_dash_services(model(models::to_services(&body)));
        }
        if let Ok(body) = audit {
            app.set_dash_audit(model(models::to_audit(&body)));
        }

        // Live updates: WebSocket first, polling as the fallback.
        api::socket_open(1, "/ws/dashboard");
    });
}

fn load_sites(bridge: &Rc<Bridge>) {
    let app = bridge.app.borrow().clone();
    let _ = slint::spawn_local(async move {
        match http("GET", "/api/v1/sites", None).await {
            Ok(body) => {
                app.set_sites(model(models::to_sites(&body)));
                app.set_sites_status("".into());
            }
            Err(message) => app.set_sites_status(message.into()),
        }
    });
}

fn load_site_detail(bridge: &Rc<Bridge>) {
    let id = bridge.app.borrow().get_site_detail_id().to_string();
    let app = bridge.app.borrow().clone();
    let _ = slint::spawn_local(async move {
        match http("GET", &format!("/api/v1/sites/{id}"), None).await {
            Ok(body) => {
                // /api/v1/sites/{id} returns the bare site object.
                app.set_site_detail(models::to_site_detail(&body));
                app.set_site_error("".into());
                app.set_site_ssl_message("".into());
                if let Ok(ssl) = http("GET", &format!("/api/v1/sites/{id}/ssl/status"), None).await {
                    app.set_site_ssl_status(models::to_ssl(&ssl));
                }
            }
            Err(message) => app.set_site_error(message.into()),
        }
    });
}

fn load_databases(bridge: &Rc<Bridge>) {
    let app = bridge.app.borrow().clone();
    let _ = slint::spawn_local(async move {
        match http("GET", "/api/v1/databases", None).await {
            Ok(body) => {
                app.set_databases(model(models::to_databases(&body)));
                app.set_database_engines(model(models::to_engines(&body)));
                app.set_databases_status("".into());
            }
            Err(message) => app.set_databases_status(message.into()),
        }
    });
}

fn load_php(bridge: &Rc<Bridge>) {
    let app = bridge.app.borrow().clone();
    let _ = slint::spawn_local(async move {
        match http("GET", "/api/v1/php/status", None).await {
            Ok(body) => {
                app.set_php_versions(model(models::to_php_versions(&body)));
                app.set_php_installed(body.get("installed").is_some());
                app.set_php_status("".into());
            }
            Err(message) => app.set_php_status(message.into()),
        }
    });
}

fn load_security(bridge: &Rc<Bridge>) {
    let app = bridge.app.borrow().clone();
    let _ = slint::spawn_local(async move {
        let firewall = http("GET", "/api/v1/security/firewall", None).await;
        let fail2ban = http("GET", "/api/v1/security/fail2ban", None).await;
        let keys = http("GET", "/api/v1/security/ssh-keys", None).await;
        if let Ok(body) = firewall {
            app.set_firewall(crate::FirewallState {
                available: body.get("available").and_then(serde_json::Value::as_bool).unwrap_or(false),
                status: body.get("status").and_then(serde_json::Value::as_str).unwrap_or_default().to_string().into(),
            });
        }
        if let Ok(body) = fail2ban {
            app.set_jails(model(models::to_jails(&body)));
        }
        if let Ok(body) = keys {
            app.set_ssh_keys(model(models::to_ssh_keys(&body)));
        }
        app.set_security_status("".into());
    });
}

fn load_backups(bridge: &Rc<Bridge>) {
    let app = bridge.app.borrow().clone();
    let _ = slint::spawn_local(async move {
        match http("GET", "/api/v1/backups", None).await {
            Ok(body) => {
                app.set_backups(model(models::to_backups(&body)));
                app.set_backup_schedules(model(models::to_schedules(&body)));
                app.set_backups_status("".into());
            }
            Err(message) => app.set_backups_status(message.into()),
        }
    });
}

fn load_cron(bridge: &Rc<Bridge>) {
    let app = bridge.app.borrow().clone();
    let _ = slint::spawn_local(async move {
        match http("GET", "/api/v1/cron", None).await {
            Ok(body) => {
                let jobs = models::to_cron_jobs(&body);
                let mut with_runs = Vec::with_capacity(jobs.len());
                for mut job in jobs {
                    if let Ok(runs) = http(
                        "GET",
                        &format!("/api/v1/cron/{}/runs?limit=10", job.id),
                        None,
                    )
                    .await
                    {
                        job.runs = model(models::to_cron_runs(&runs));
                    }
                    with_runs.push(job);
                }
                app.set_cron_jobs(model(with_runs));
                app.set_cron_status("".into());
            }
            Err(message) => app.set_cron_status(message.into()),
        }
    });
}

fn load_logs(bridge: &Rc<Bridge>) {
    let app = bridge.app.borrow().clone();
    let _ = slint::spawn_local(async move {
        match http("GET", "/api/v1/logs/sources", None).await {
            Ok(body) => {
                app.set_log_sources(model(models::to_log_sources(&body)));
                app.set_logs_status("".into());
            }
            Err(message) => app.set_logs_status(message.into()),
        }
    });
}

fn load_files(bridge: &Rc<Bridge>, path: &str) {
    let app = bridge.app.borrow().clone();
    let path = path.to_string();
    let _ = slint::spawn_local(async move {
        let query = if path.is_empty() {
            "scope=www".to_string()
        } else {
            format!("scope=www&path={}", percent_encode(&path))
        };
        match http("GET", &format!("/api/v1/files?{query}"), None).await {
            Ok(body) => {
                app.set_file_rows(model(models::to_file_rows(&body)));
                app.set_current_file_path(
                    body.get("path")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string()
                        .into(),
                );
                app.set_files_status("".into());
            }
            Err(message) => app.set_files_status(message.into()),
        }
    });
}

fn load_deployments(bridge: &Rc<Bridge>) {
    let app = bridge.app.borrow().clone();
    let _ = slint::spawn_local(async move {
        match http("GET", "/api/v1/deployments", None).await {
            Ok(body) => {
                let deployments = models::to_deployments(&body);
                let mut with_runs = Vec::with_capacity(deployments.len());
                for mut deployment in deployments {
                    if let Ok(runs) = http(
                        "GET",
                        &format!("/api/v1/deployments/{}/runs", deployment.id),
                        None,
                    )
                    .await
                    {
                        deployment.runs = model(models::to_deploy_runs(&runs));
                    }
                    with_runs.push(deployment);
                }
                app.set_deployments(model(with_runs));
                app.set_deployments_status("".into());
            }
            Err(message) => app.set_deployments_status(message.into()),
        }
    });
}

fn load_alerts(bridge: &Rc<Bridge>) {
    let app = bridge.app.borrow().clone();
    let _ = slint::spawn_local(async move {
        let channels = http("GET", "/api/v1/alerts/channels", None).await;
        let rules = http("GET", "/api/v1/alerts/rules", None).await;
        let alerts = http("GET", "/api/v1/alerts", None).await;
        if let Ok(body) = channels {
            app.set_alert_channels(model(models::to_channels(&body)));
        }
        if let Ok(body) = rules {
            app.set_alert_rules(model(models::to_rules(&body)));
        }
        if let Ok(body) = alerts {
            app.set_alerts(model(models::to_alerts(&body)));
        }
        app.set_alerts_status("".into());
    });
}

fn load_terminal(bridge: &Rc<Bridge>) {
    let app = bridge.app.borrow().clone();
    let _ = slint::spawn_local(async move {
        app.set_terminal_status("opening…".into());
        match http("POST", "/api/v1/terminal/open", Some(serde_json::json!({ "rows": 24, "cols": 80 }))).await {
            Ok(session) => {
                let id = session.get("id").and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
                let shell = session.get("shell").and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
                app.set_terminal_id(id.clone().into());
                app.set_terminal_shell(shell.into());
                app.set_terminal_status("".into());
                api::socket_open(2, &format!("/ws/terminal/{id}"));
            }
            Err(message) => app.set_terminal_status(message.into()),
        }
    });
}

fn load_users(bridge: &Rc<Bridge>) {
    let app = bridge.app.borrow().clone();
    let _ = slint::spawn_local(async move {
        match http("GET", "/api/v1/users", None).await {
            Ok(body) => {
                app.set_users(model(models::to_users(&body)));
                app.set_users_status("".into());
            }
            Err(message) => app.set_users_status(message.into()),
        }
    });
}

fn load_settings(bridge: &Rc<Bridge>) {
    let app = bridge.app.borrow().clone();
    let _ = slint::spawn_local(async move {
        let settings = http("GET", "/api/v1/settings", None).await;
        let keys = http("GET", "/api/v1/api-keys", None).await;
        let update = http("GET", "/api/v1/update", None).await;
        let certificate = http("GET", "/api/v1/panel/ssl/status", None).await;
        if let Ok(body) = settings {
            app.set_settings_json(body.to_string().into());
            let hostname = body.get("hostname").and_then(serde_json::Value::as_str).unwrap_or_default();
            let timezone = body.get("timezone").and_then(serde_json::Value::as_str).unwrap_or_default();
            let locale = body.get("locale").and_then(serde_json::Value::as_str).unwrap_or_default();
            app.set_hostname(hostname.to_string().into());
            app.set_timezone(timezone.to_string().into());
            app.set_locale(locale.to_string().into());
        }
        if let Ok(body) = keys {
            app.set_api_keys(model(models::to_api_keys(&body)));
        }
        if let Ok(body) = update {
            app.set_update_json(body.to_string().into());
        }
        if let Ok(body) = certificate {
            app.set_panel_certificate(models::to_panel_certificate(&body));
        }
        app.set_settings_status("".into());
    });
}

// ---------------------------------------------------------------------------
// Sockets
// ---------------------------------------------------------------------------

impl Bridge {
    fn on_socket_message(&self, id: u32, data: &str) {
        let value: serde_json::Value = serde_json::from_str(data).unwrap_or(serde_json::Value::Null);
        match id {
            1 => match value.get("type").and_then(serde_json::Value::as_str) {
                Some("snapshot") => {
                    let metrics = value.get("metrics").cloned().unwrap_or_default();
                    if let Some(previous) = self.prev_snapshot.borrow().clone() {
                        let (rx, tx) = models::network_rate(&previous, &metrics);
                        let mut body = metrics.clone();
                        if let serde_json::Value::Object(map) = &mut body {
                            map.insert("net_rx".into(), serde_json::json!(rx));
                            map.insert("net_tx".into(), serde_json::json!(tx));
                        }
                        self.prev_snapshot.borrow_mut().replace(metrics.clone());
                        let snapshot = models::metric_snapshot(&body);
                        if !snapshot.host.is_empty() {
                            self.app.borrow_mut().set_dash_snapshot(snapshot);
                        }
                    } else {
                        self.prev_snapshot.borrow_mut().replace(metrics.clone());
                        let snapshot = models::metric_snapshot(&metrics);
                        if !snapshot.host.is_empty() {
                            self.app.borrow_mut().set_dash_snapshot(snapshot);
                        }
                    }
                    if let Some(services) = value.get("services") {
                        self.app.borrow_mut()
                            .set_dash_services(model(models::to_services(services)));
                    }
                }
                Some("error") => {
                    self.app.borrow_mut().set_dash_status(
                        value.get("message")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("metrics stream error")
                            .to_string()
                            .into(),
                    );
                }
                _ => {}
            },
            2 => match value.get("type").and_then(serde_json::Value::as_str) {
                Some("output") => {
                    let output = value.get("data").and_then(serde_json::Value::as_str).unwrap_or_default();
                    let mut lines: Vec<crate::TerminalLine> =
                        self.app.borrow().get_terminal_lines().iter().collect();
                    for line in models::terminal_output_line(output) {
                        lines.push(line);
                    }
                    if lines.len() > 1000 {
                        lines = lines.split_at(lines.len() - 1000).1.to_vec();
                    }
                    self.app.borrow_mut().set_terminal_lines(model(lines));
                }
                Some("closed") => self.app.borrow_mut().set_terminal_status("the session ended".into()),
                Some("error") => {
                    self.app.borrow_mut()
                        .set_terminal_status(
                            value.get("message")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("terminal error")
                                .to_string()
                                .into(),
                        )
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn on_socket_close(&self, id: u32) {
        if id == 2 {
            self.app.borrow_mut().set_terminal_status("the connection closed".into());
        }
    }
}

fn percent_encode(input: &str) -> String {
    let mut out = String::new();
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

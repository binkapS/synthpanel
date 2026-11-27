//! Nginx vhost rendering and validation.

use crate::site_security::SiteSecuritySettings;
use crate::sites::{SiteExtras, SiteRecord};
use anyhow::Result;
use regex::Regex;

/// The local application upstream to proxy to, when a site is configured as
/// a Node.js or Python app site.
///
/// Sites without one keep their static/PHP vhost. This type intentionally
/// lives here so the renderer has no dependency on the process supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppProxy<'a> {
    pub app_type: &'a str,
    pub app_port: u16,
}

impl<'a> AppProxy<'a> {
    pub fn new(app_type: &'a str, app_port: u16) -> Self {
        Self { app_type, app_port }
    }

    /// Only `node` and `python` app sites get a proxy vhost.
    pub fn is_supported(&self) -> bool {
        matches!(
            self.app_type.trim().to_ascii_lowercase().as_str(),
            "node" | "nodejs" | "python" | "python3"
        )
    }
}

/// Optional vhost features. Both are off by default; the existing renderers
/// produce byte-for-byte identical output unless one is explicitly enabled.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VhostOptions {
    /// Emit `listen 443 quic reuseport;` and advertise `Alt-Svc: h3`.
    pub http3: bool,
    /// Emit PageSpeed and NGINX cache directives.
    pub pagespeed: bool,
}

impl VhostOptions {
    /// Read `SYNTHPANEL_HTTP3` / `SYNTHPANEL_PAGESPEED`.
    pub fn from_env() -> Self {
        Self {
            http3: env_flag("SYNTHPANEL_HTTP3"),
            pagespeed: env_flag("SYNTHPANEL_PAGESPEED"),
        }
    }

    /// Environment defaults, overridden by top-level `synthpanel_http3` and
    /// `synthpanel_pagespeed` markers in the site's custom directives.
    pub fn for_site(site: &SiteExtras) -> Self {
        let mut options = Self::from_env();
        if let Some(text) = site.nginx.as_deref() {
            options.apply_directives(text);
        }
        options
    }

    /// Apply `on`/`off` markers. An explicit marker wins over the env default.
    pub fn apply_directives(&mut self, text: &str) {
        let (_, (http3, pagespeed)) = extract_feature_flags(text);
        if let Some(value) = http3 {
            self.http3 = value;
        }
        if let Some(value) = pagespeed {
            self.pagespeed = value;
        }
    }
}

/// Interpret a boolean env value. Only explicit opt-ins enable a feature.
pub fn flag_value(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

fn env_flag(name: &str) -> bool {
    flag_value(std::env::var(name).ok().as_deref())
}

fn parse_on_off(value: &str, fallback: bool) -> bool {
    match value.trim().to_ascii_lowercase().as_str() {
        "on" | "1" | "true" | "yes" => true,
        "off" | "0" | "false" | "no" => false,
        _ => fallback,
    }
}

/// Remove top-level feature markers from custom directives and report the
/// site-level overrides. Lines inside blocks are left untouched.
fn extract_feature_flags(text: &str) -> (String, (Option<bool>, Option<bool>)) {
    let mut kept: Vec<&str> = Vec::new();
    let mut depth = 0i32;
    let mut http3 = None;
    let mut pagespeed = None;
    for line in text.lines() {
        if depth == 0 {
            let trimmed = line.trim();
            if let Some(value) = directive_value(trimmed, "synthpanel_http3") {
                http3 = Some(parse_on_off(value, true));
                continue;
            }
            if let Some(value) = directive_value(trimmed, "synthpanel_pagespeed") {
                pagespeed = Some(parse_on_off(value, true));
                continue;
            }
        }
        depth += brace_delta(line);
        kept.push(line);
    }
    (kept.join("\n"), (http3, pagespeed))
}

/// HTTP-context cache zone required by the optional PageSpeed/NGINX cache
/// snippet. It is deliberately not emitted into a vhost (where `proxy_cache_path`
/// would be invalid); callers may write it to a snippets directory.
pub fn render_cache_http_snippet() -> String {
    "# Optional NGINX cache zone for SYNTHPANEL_PAGESPEED=1\n\
     proxy_cache_path /var/cache/nginx/synthpanel_cache levels=1:2 keys_zone=synthpanel:10m inactive=60m max_size=1g;\n"
        .to_string()
}

/// Validate a block of custom Nginx directives before they are applied.
///
/// The linter rejects unbalanced braces and reserved directives that the
/// panel itself controls (`root`, `listen`, `server_name`, and the
/// `ssl_certificate*` lines).
pub fn lint_directives(text: &str) -> Result<()> {
    if text.trim().is_empty() {
        return Ok(());
    }
    let mut depth = 0i32;
    let mut in_string = false;
    for ch in text.chars() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth < 0 {
                    anyhow::bail!("unbalanced braces in custom directives");
                }
            }
            '"' => in_string = !in_string,
            _ if in_string => {}
            _ => {}
        }
    }
    if depth != 0 {
        anyhow::bail!("unbalanced braces in custom directives");
    }
    let reserved = ReservedDirectives::new();
    if let Some(matched) = reserved.first_match(text) {
        anyhow::bail!(
            "custom directives may not contain `{}` (reserved by the panel)",
            matched
        );
    }
    Ok(())
}

struct ReservedDirectives {
    names: Vec<&'static str>,
}

impl ReservedDirectives {
    fn new() -> Self {
        Self {
            names: vec![
                "root",
                "listen",
                "server_name",
                "ssl_certificate",
                "ssl_certificate_key",
            ],
        }
    }

    fn first_match(&self, text: &str) -> Option<&'static str> {
        self.names.iter().copied().find(|name| {
            let re = Regex::new(&format!(r"(?m)^\s*{}\b", name)).expect("regex is valid");
            re.is_match(text)
        })
    }
}

/// Render an Nginx vhost file for a site without any security overrides.
///
/// Callers that manage site security should use
/// [`render_vhost_with_security`] so the access rules are included in the
/// same render → lint → `nginx -t` → rollback apply pipeline.
pub fn render_vhost(site: &SiteExtras, panel_hostname: &str) -> String {
    render_vhost_with_security(site, panel_hostname, &SiteSecuritySettings::default())
}

/// Render an Nginx vhost including the site's security controls.
pub fn render_vhost_with_security(
    site: &SiteExtras,
    panel_hostname: &str,
    security: &SiteSecuritySettings,
) -> String {
    render_vhost_with_app(site, panel_hostname, security, None)
}

/// Render a vhost, optionally replacing the static/PHP content location with
/// a reverse proxy to a supervised Node.js/Python application.
pub fn render_vhost_with_app(
    site: &SiteExtras,
    panel_hostname: &str,
    security: &SiteSecuritySettings,
    app: Option<AppProxy<'_>>,
) -> String {
    render_vhost_with_options(
        site,
        panel_hostname,
        security,
        app,
        VhostOptions::for_site(site),
    )
}

/// Render a vhost with explicit control over the optional HTTP/3 and
/// PageSpeed/cache snippets.
pub fn render_vhost_with_options(
    site: &SiteExtras,
    panel_hostname: &str,
    security: &SiteSecuritySettings,
    app: Option<AppProxy<'_>>,
    mut options: VhostOptions,
) -> String {
    let extras = site;
    let site = &extras.site;
    let app = app.filter(AppProxy::is_supported);
    let ssl_mode = site.ssl_mode.as_str();
    // Proxy sites carry their target through the existing custom directive
    // field (see `extract_proxy_target`); the site table has no such column.
    let (directives, proxy_target) = extract_proxy_target(extras.nginx.as_deref());
    let directives = directives.map(|text| {
        let (kept, (http3, pagespeed)) = extract_feature_flags(&text);
        if let Some(value) = http3 {
            options.http3 = value;
        }
        if let Some(value) = pagespeed {
            options.pagespeed = value;
        }
        kept
    });
    let mut server_names: Vec<String> = site.all_server_names();
    server_names.sort();
    server_names.dedup();
    let server_name = server_names;

    // HTTP/3 always needs TLS, so the QUIC listener is only added to TLS
    // vhosts. With the flags off this is exactly the previous output.
    let ssl_listen = match ssl_mode {
        "none" => "listen 80;".to_string(),
        _ if options.http3 => "listen 443 ssl;\n    listen 443 quic reuseport;".to_string(),
        _ => "listen 443 ssl;".to_string(),
    };
    let http_redirect = match ssl_mode {
        "none" => String::new(),
        _ => "    # HTTP to HTTPS redirect\n    if ($scheme = http) { return 301 https://$host$request_uri; }\n".to_string(),
    };

    let ssl_block = render_ssl_block(site, ssl_mode);

    let php_block = if app.is_some() {
        // An app site is served by its process, never by PHP-FPM.
        String::new()
    } else {
        match site.template.as_str() {
            "static" | "node" | "nextjs" | "proxy" => String::new(),
            _ => {
                let php_version = site.php_version.as_deref().unwrap_or("8.3");
                format!(
                    "    location ~ \\.php$ {{\n\
                     {indent}fastcgi_pass unix:/run/php/php{ver}-sp-{id}.sock;\n\
                     {indent}include /etc/nginx/snippets/fastcgi-php.conf;\n\
                     {indent}}}\n",
                    ver = php_version,
                    id = site.id,
                    indent = "        "
                )
            }
        }
    };

    let content_location = match app {
        Some(app) => render_app_location(&app),
        None => match site.template.as_str() {
            "proxy" => render_proxy_location(proxy_target.as_deref()),
            "wordpress" => "    # WordPress permalinks\n    location / {\n        try_files $uri $uri/ /index.php?$args;\n    }\n\n"
                .to_string(),
            _ => "    # Static files\n    location / {\n        try_files $uri $uri/ =404;\n    }\n\n"
                .to_string(),
        },
    };

    let security_block = crate::site_security::render_security_block(security, &site.id);
    let acme_block = if security.has_restrictions() {
        render_acme_exception(&site.id)
    } else {
        String::new()
    };

    let directives_block = match directives.as_deref() {
        Some(text) if !text.trim().is_empty() => {
            format!("    # Custom directives\n{}", indent_lines(text, 4))
        }
        _ => String::new(),
    };

    let mut out = String::new();
    out.push_str("# Managed by SynthPanel — do not edit manually\n");
    out.push_str(&format!("# site-id={} domain={}\n\n", site.id, site.domain));
    out.push_str("server {\n");
    out.push_str(&format!("    {}\n", ssl_listen));
    out.push_str(&format!("    server_name {};\n", server_name.join(" ")));
    out.push_str(&format!(
        "    root {};
",
        site.document_root
    ));
    out.push_str("    index index.html index.php;\n\n");
    out.push_str(&http_redirect);
    out.push_str(&security_block);
    out.push_str(&ssl_block);
    if options.http3 && ssl_mode != "none" {
        out.push_str(
            "    # HTTP/3 advertisement\n\
             \x20   add_header Alt-Svc 'h3=\":443\"; ma=86400' always;\n\n",
        );
    }
    if options.pagespeed {
        out.push_str(
            "    # PageSpeed + NGINX cache (optional)\n\
             \x20   pagespeed on;\n\
             \x20   pagespeed FileCachePath /var/cache/nginx/synthpanel-pagespeed;\n\
             \x20   pagespeed EnableFilters collapse_whitespace,remove_comments;\n\
             \x20   proxy_cache synthpanel;\n\
             \x20   proxy_cache_valid 200 10m;\n\
             \x20   proxy_cache_use_stale error timeout updating;\n\
             \x20   add_header X-Cache-Status $upstream_cache_status always;\n\n",
        );
    }
    out.push_str(&php_block);
    out.push_str(&content_location);
    out.push_str(&acme_block);
    out.push_str(&directives_block);
    out.push_str("}\n");

    let _ = panel_hostname;
    out
}

/// Certificate directives, TLS hardening, HSTS and OCSP stapling for any
/// site with TLS enabled.
fn render_ssl_block(site: &SiteRecord, ssl_mode: &str) -> String {
    let (cert, key, trusted) = match ssl_mode {
        "none" => return String::new(),
        "le" => (
            format!("/etc/letsencrypt/live/{}/fullchain.pem", site.domain),
            format!("/etc/letsencrypt/live/{}/privkey.pem", site.domain),
            None,
        ),
        "custom" => {
            let paths = crate::ssl::site_cert_paths(&site.id);
            (
                paths.fullchain.display().to_string(),
                paths.key.display().to_string(),
                Some(paths.chain.display().to_string()),
            )
        }
        _ => return String::new(),
    };
    let mut out = format!(
        "    ssl_certificate {};\n\
         {indent}ssl_certificate_key {};\n",
        escape_config_value(&cert),
        escape_config_value(&key),
        indent = "    "
    );
    if let Some(trusted) = trusted {
        out.push_str(&format!(
            "    ssl_trusted_certificate {};\n",
            escape_config_value(&trusted)
        ));
    }
    out.push_str(
        "    ssl_protocols TLSv1.2 TLSv1.3;\n\
         \x20   ssl_session_cache shared:SSL:10m;\n\
         \x20   ssl_stapling on;\n\
         \x20   ssl_stapling_verify on;\n\
         \x20   resolver 1.1.1.1 8.8.8.8 valid=300s;\n\
         \x20   resolver_timeout 5s;\n\
         \x20   add_header Strict-Transport-Security \"max-age=63072000; includeSubDomains\" always;\n",
    );
    out.push('\n');
    out
}

/// The `proxy` template's location block.
///
/// The site table has no upstream column, so the target is taken from the
/// existing custom-directives field: a top-level `proxy_target <url>;` line
/// (or a plain `proxy_pass <url>;` line) is consumed and used here instead of
/// being emitted twice. Without a target the vhost stays valid and returns
/// 503 rather than emitting an incomplete `proxy_pass`.
fn render_proxy_location(target: Option<&str>) -> String {
    render_proxy_location_labeled(target, "# Reverse proxy")
}

/// The location block of a supervised app site. The upstream is always the
/// loopback port of the app process; custom directives cannot redirect it.
fn render_app_location(app: &AppProxy<'_>) -> String {
    let upstream = format!("http://127.0.0.1:{}", app.app_port);
    let label = match app.app_type.trim().to_ascii_lowercase().as_str() {
        "node" | "nodejs" => "# Node.js application (managed by SynthPanel)",
        "python" | "python3" => "# Python application (managed by SynthPanel)",
        _ => "# Application (managed by SynthPanel)",
    };
    render_proxy_location_labeled(Some(&upstream), label)
}

fn render_proxy_location_labeled(target: Option<&str>, label: &str) -> String {
    let Some(target) = target.filter(|value| safe_proxy_target(value)) else {
        return "    # Reverse proxy without a target: set `proxy_target <url>;` in the\n\
                \x20   # site's custom directives (or use a top-level `proxy_pass <url>;`).\n\
                \x20   location / {\n        return 503;\n    }\n\n"
            .to_string();
    };
    format!(
        "    {label}\n\
         {indent}location / {{\n\
         {indent}    proxy_pass {target};\n\
         {indent}    proxy_http_version 1.1;\n\
         {indent}    proxy_set_header Host $host;\n\
         {indent}    proxy_set_header X-Real-IP $remote_addr;\n\
         {indent}    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;\n\
         {indent}    proxy_set_header X-Forwarded-Proto $scheme;\n\
         {indent}    proxy_set_header X-Forwarded-Host $host;\n\
         {indent}    proxy_set_header X-Forwarded-Port $server_port;\n\
         {indent}    proxy_set_header Upgrade $http_upgrade;\n\
         {indent}    proxy_set_header Connection \"upgrade\";\n\
         {indent}    proxy_redirect off;\n\
         {indent}}}\n\n",
        indent = "    ",
        target = target
    )
}

/// Pull the proxy target out of top-level custom directives and return the
/// remaining directives text.
fn extract_proxy_target(text: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(text) = text else {
        return (None, None);
    };
    let mut target = None;
    let mut kept: Vec<&str> = Vec::new();
    let mut depth = 0i32;
    for line in text.lines() {
        if depth == 0 && target.is_none() {
            let trimmed = line.trim();
            if let Some(value) = directive_value(trimmed, "proxy_target")
                .or_else(|| directive_value(trimmed, "proxy_pass"))
            {
                target = Some(value.to_string());
                continue;
            }
        }
        depth += brace_delta(line);
        kept.push(line);
    }
    (Some(kept.join("\n")), target)
}

fn directive_value<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(name)?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let value = rest.trim().strip_suffix(';')?.trim();
    if value.is_empty() { None } else { Some(value) }
}

fn brace_delta(line: &str) -> i32 {
    let mut delta = 0;
    let mut quote = '\0';
    let mut escaped = false;
    for ch in line.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if quote != '\0' {
            if ch == quote {
                quote = '\0';
            }
            continue;
        }
        if ch == '"' || ch == '\'' {
            quote = ch;
            continue;
        }
        match ch {
            '{' => delta += 1,
            '}' => delta -= 1,
            _ => {}
        }
    }
    delta
}

fn safe_proxy_target(value: &str) -> bool {
    let value = value.trim();
    (value.starts_with("http://") || value.starts_with("https://") || value.starts_with("unix:"))
        && value
            .chars()
            .all(|c| !c.is_control() && !matches!(c, ';' | '{' | '}' | '#' | ' ' | '"' | '\''))
}

/// Keep ACME HTTP-01 reachable even when the server has access restrictions.
fn render_acme_exception(site_id: &str) -> String {
    let root = crate::site_security::acme_webroot(site_id);
    format!(
        "    # ACME HTTP-01 must remain reachable through access restrictions\n\
         {indent}location /.well-known/acme-challenge/ {{\n\
         {indent}    allow all;\n\
         {indent}    auth_basic off;\n\
         {indent}    root \"{root}\";\n\
         {indent}    try_files $uri =404;\n\
         {indent}}}\n\n",
        indent = "    ",
        root = escape_config_value(&root.display().to_string())
    )
}

fn escape_config_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn indent_lines(text: &str, spaces: usize) -> String {
    let prefix = " ".repeat(spaces);
    text.lines()
        .map(|line| {
            if line.trim().is_empty() {
                String::new()
            } else {
                format!("{prefix}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::sites::{CreateSite, Sites};

    async fn make_site_with(
        aliases: Vec<&str>,
        template: &str,
        ssl_mode: &str,
        directives: &str,
    ) -> SiteExtras {
        let mut path = std::env::temp_dir();
        path.push(format!("sp-nginx-{}.db", uuid::Uuid::new_v4()));
        let pool = db::connect(&path).await.unwrap();
        db::migrate(&pool).await.unwrap();
        let sites = Sites::new(pool);
        let site = sites
            .create(CreateSite {
                domain: "example.com".into(),
                aliases: aliases.iter().map(|s| s.to_string()).collect(),
                document_root: "/var/www/sp_test/public".into(),
                template: template.into(),
                php_version: Some("8.3".into()),
                ssl_mode: ssl_mode.into(),
                owner_id: None,
                directives: directives.into(),
            })
            .await
            .unwrap();
        sites.get_with_extras(&site.id).await.unwrap().unwrap()
    }

    async fn make_site(aliases: Vec<&str>, template: &str, ssl_mode: &str) -> SiteExtras {
        make_site_with(aliases, template, ssl_mode, "").await
    }

    #[tokio::test]
    async fn renders_static_vhost() {
        let site = make_site(vec![], "static", "none").await;
        let text = render_vhost(&site, "panel.local");
        assert!(text.contains("server_name example.com;"));
        assert!(text.contains("root /var/www/sp_test/public;"));
        assert!(text.contains("try_files $uri $uri/ =404;"));
        assert!(!text.contains("fastcgi_pass"));
        assert!(!text.contains("ssl_certificate"));
    }

    #[tokio::test]
    async fn renders_php_vhost_with_le_ssl() {
        let site = make_site(vec!["www"], "php", "le").await;
        let text = render_vhost(&site, "panel.local");
        assert!(text.contains("listen 443 ssl;"));
        assert!(text.contains("server_name example.com www;"));
        assert!(text.contains("fastcgi_pass unix:/run/php/php8.3-sp-"));
        assert!(text.contains("/etc/letsencrypt/live/example.com/"));
        assert!(text.contains("Strict-Transport-Security"));
    }

    #[tokio::test]
    async fn node_template_has_no_fastcgi() {
        let site = make_site(vec![], "node", "none").await;
        let text = render_vhost(&site, "panel.local");
        assert!(!text.contains("fastcgi_pass"));
    }

    #[tokio::test]
    async fn app_site_proxies_to_the_supervised_loopback_port() {
        let site = make_site(vec![], "node", "none").await;
        let text = render_vhost_with_app(
            &site,
            "panel.local",
            &SiteSecuritySettings::default(),
            Some(AppProxy::new("node", 3100)),
        );
        assert!(text.contains("# Node.js application (managed by SynthPanel)"));
        assert!(text.contains("proxy_pass http://127.0.0.1:3100;"));
        assert!(text.contains("proxy_http_version 1.1;"));
        assert!(text.contains("proxy_set_header Host $host;"));
        assert!(text.contains("proxy_set_header X-Real-IP $remote_addr;"));
        assert!(text.contains("proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;"));
        assert!(text.contains("proxy_set_header X-Forwarded-Proto $scheme;"));
        assert!(text.contains("proxy_set_header Upgrade $http_upgrade;"));
        assert!(text.contains("proxy_set_header Connection \"upgrade\";"));
        assert!(!text.contains("try_files $uri $uri/ =404;"));
        assert!(!text.contains("return 503;"));
    }

    #[tokio::test]
    async fn app_override_replaces_php_and_static_content_locations() {
        let site = make_site(vec![], "php", "le").await;
        let text = render_vhost_with_app(
            &site,
            "panel.local",
            &SiteSecuritySettings::default(),
            Some(AppProxy::new("python", 8000)),
        );
        assert!(text.contains("# Python application (managed by SynthPanel)"));
        assert!(text.contains("proxy_pass http://127.0.0.1:8000;"));
        assert!(!text.contains("fastcgi_pass"));
        assert!(!text.contains("# Static files"));
        // TLS and security rendering is unaffected.
        assert!(text.contains("listen 443 ssl;"));
        assert!(text.contains("ssl_certificate "));
    }

    #[tokio::test]
    async fn sites_without_app_config_keep_their_plain_vhost() {
        let node = make_site(vec![], "node", "none").await;
        let text =
            render_vhost_with_app(&node, "panel.local", &SiteSecuritySettings::default(), None);
        assert!(text.contains("try_files $uri $uri/ =404;"));
        assert!(!text.contains("proxy_pass"));
        assert!(!text.contains("fastcgi_pass"));

        let php = make_site(vec![], "php", "none").await;
        let text = render_vhost(&php, "panel.local");
        assert!(text.contains("fastcgi_pass unix:/run/php/php8.3-sp-"));
        assert!(text.contains("try_files $uri $uri/ =404;"));
    }

    #[test]
    fn unsupported_app_types_fall_back_to_the_plain_vhost() {
        assert!(!AppProxy::new("ruby", 3000).is_supported());
        assert!(AppProxy::new("node", 3000).is_supported());
        assert!(AppProxy::new("python", 3000).is_supported());
    }

    #[tokio::test]
    async fn proxy_template_has_no_fastcgi_or_static_fallback() {
        let site = make_site(vec![], "proxy", "none").await;
        let text = render_vhost(&site, "panel.local");
        assert!(text.contains("# Reverse proxy"));
        assert!(!text.contains("fastcgi_pass"));
        assert!(!text.contains("try_files $uri $uri/ =404;"));
        // No upstream configured: the vhost must still be valid.
        assert!(text.contains("return 503;"));
        assert!(!text.contains("proxy_pass ;"));
    }

    #[tokio::test]
    async fn proxy_target_comes_from_custom_directives() {
        let site = make_site_with(
            vec![],
            "proxy",
            "none",
            "proxy_target http://127.0.0.1:3000;\nadd_header X-Test yes;",
        )
        .await;
        let text = render_vhost(&site, "panel.local");
        assert!(text.contains("proxy_pass http://127.0.0.1:3000;"));
        assert!(text.contains("proxy_set_header X-Forwarded-Proto $scheme;"));
        assert!(text.contains("proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;"));
        assert!(text.contains("proxy_set_header Upgrade $http_upgrade;"));
        assert!(text.contains("add_header X-Test yes;"));
        assert!(!text.contains("proxy_target"));
        assert!(!text.contains("return 503;"));
    }

    #[tokio::test]
    async fn proxy_pass_in_custom_directives_is_also_accepted() {
        let site =
            make_site_with(vec![], "proxy", "none", "proxy_pass https://backend:8443;").await;
        let text = render_vhost(&site, "panel.local");
        assert!(text.contains("proxy_pass https://backend:8443;"));
        assert!(!text.contains("# Custom directives"));
    }

    #[test]
    fn proxy_target_inside_a_block_is_left_alone() {
        let (kept, target) = extract_proxy_target(Some(
            "location /api {\n    proxy_pass http://api;\n}\nproxy_target http://main;\n",
        ));
        assert_eq!(target.as_deref(), Some("http://main"));
        let kept = kept.unwrap();
        assert!(kept.contains("proxy_pass http://api;"));
        assert!(!kept.contains("proxy_target"));
    }

    #[tokio::test]
    async fn security_rules_are_rendered_into_the_vhost() {
        let site = make_site(vec![], "static", "none").await;
        let security = SiteSecuritySettings {
            ip_blocks: vec!["203.0.113.10".into()],
            bot_blocks: vec!["EvilBot/1.0".into()],
            basic_auth_enabled: true,
            basic_auth_user: "admin".into(),
            cloudflare_only: true,
        };
        let text = render_vhost_with_security(&site, "panel.local", &security);
        assert!(text.contains("deny 203.0.113.10;"));
        assert!(text.contains("allow 173.245.48.0/20;"));
        assert!(text.contains("deny all;"));
        assert!(text.contains(r#"$http_user_agent ~* "EvilBot\/1\.0""#));
        assert!(text.contains(r#"auth_basic "Restricted area";"#));
        assert!(text.contains("auth_basic_user_file "));
        assert!(text.contains(&format!("htpasswd-{}", site.site.id)));
        assert!(text.contains("location /.well-known/acme-challenge/"));
        assert!(text.contains("auth_basic off;"));
    }

    #[tokio::test]
    async fn custom_certificates_render_installed_paths_and_tls_hardening() {
        let site = make_site(vec![], "static", "custom").await;
        let text = render_vhost(&site, "panel.local");
        assert!(text.contains(&format!(
            "ssl_certificate {};",
            crate::ssl::site_cert_paths(&site.site.id)
                .fullchain
                .display()
        )));
        assert!(text.contains(&format!(
            "ssl_certificate_key {};",
            crate::ssl::site_cert_paths(&site.site.id).key.display()
        )));
        assert!(text.contains("ssl_trusted_certificate "));
        assert!(text.contains("ssl_stapling on;"));
        assert!(text.contains("ssl_stapling_verify on;"));
        assert!(text.contains("Strict-Transport-Security"));
    }

    #[tokio::test]
    async fn letsencrypt_sites_get_ocsp_stapling() {
        let site = make_site(vec![], "php", "le").await;
        let text = render_vhost(&site, "panel.local");
        assert!(text.contains("ssl_stapling on;"));
        assert!(text.contains("resolver "));
        assert!(
            text.contains(
                "Strict-Transport-Security \"max-age=63072000; includeSubDomains\" always;"
            )
        );
    }

    #[test]
    fn lint_rejects_reserved_directives() {
        assert!(lint_directives("root /var/www/other;\n").is_err());
        assert!(lint_directives("listen 8080;\n").is_err());
        assert!(lint_directives("server_name evil.example;\n").is_err());
    }

    #[test]
    fn lint_rejects_unbalanced_braces() {
        assert!(lint_directives("location / {\n    index index.html;\n").is_err());
        assert!(lint_directives("location / { index index.html; }\n").is_ok());
    }

    #[test]
    fn lint_accepts_safe_directives() {
        assert!(lint_directives("").is_ok());
        assert!(
            lint_directives("location /assets/ { add_header Cache-Control \"max-age=604800\"; }\n")
                .is_ok()
        );
        // The proxy target marker is consumed by the renderer, not by nginx.
        assert!(lint_directives("proxy_target http://127.0.0.1:3000;\n").is_ok());
    }

    #[test]
    fn site_records_preserve_aliases() {
        let site = crate::sites::SiteRecord {
            id: "1".into(),
            domain: "example.com".into(),
            aliases: "[\"www\",\"mail\"]".into(),
            document_root: "/x".into(),
            template: "php".into(),
            php_version: None,
            pool_user: "sp_1".into(),
            status: "active".into(),
            ssl_mode: "none".into(),
            ssl_cert_id: None,
            owner_id: None,
            created_at: "t".into(),
            updated_at: "t".into(),
        };
        let mut names = site.all_server_names();
        names.sort();
        assert_eq!(names, vec!["example.com", "mail", "www"]);
    }

    #[test]
    fn optional_features_are_off_by_default() {
        assert_eq!(
            VhostOptions::default(),
            VhostOptions {
                http3: false,
                pagespeed: false
            }
        );
        assert!(!flag_value(None));
        assert!(!flag_value(Some("")));
        assert!(!flag_value(Some("0")));
        assert!(!flag_value(Some("maybe")));
        assert!(flag_value(Some("1")));
        assert!(flag_value(Some(" TRUE ")));
        assert!(flag_value(Some("on")));
    }

    #[tokio::test]
    async fn default_options_match_the_legacy_renderer_byte_for_byte() {
        let site = make_site(vec![], "php", "le").await;
        let legacy = render_vhost(&site, "panel.local");
        let explicit = render_vhost_with_options(
            &site,
            "panel.local",
            &SiteSecuritySettings::default(),
            None,
            VhostOptions::default(),
        );
        assert_eq!(legacy, explicit);
        assert!(!legacy.contains("quic"));
        assert!(!legacy.contains("pagespeed"));
        assert!(!legacy.contains("Alt-Svc"));
    }

    #[tokio::test]
    async fn http3_and_pagespeed_snippets_are_emitted_when_enabled() {
        let site = make_site(vec![], "php", "le").await;
        let text = render_vhost_with_options(
            &site,
            "panel.local",
            &SiteSecuritySettings::default(),
            None,
            VhostOptions {
                http3: true,
                pagespeed: true,
            },
        );
        assert!(text.contains("listen 443 ssl;\n    listen 443 quic reuseport;"));
        assert!(text.contains(r#"add_header Alt-Svc 'h3=":443"; ma=86400' always;"#));
        assert!(text.contains("pagespeed on;"));
        assert!(text.contains("proxy_cache synthpanel;"));
        assert!(text.contains("proxy_cache_use_stale error timeout updating;"));
        assert!(text.contains("add_header X-Cache-Status $upstream_cache_status always;"));
    }

    #[tokio::test]
    async fn http3_is_never_emitted_without_tls() {
        let site = make_site(vec![], "static", "none").await;
        let text = render_vhost_with_options(
            &site,
            "panel.local",
            &SiteSecuritySettings::default(),
            None,
            VhostOptions {
                http3: true,
                pagespeed: false,
            },
        );
        assert!(text.contains("listen 80;"));
        assert!(!text.contains("quic"));
        assert!(!text.contains("Alt-Svc"));
    }

    #[test]
    fn site_directives_can_enable_and_disable_features() {
        let (kept, (http3, pagespeed)) = extract_feature_flags(
            "synthpanel_http3 on;\nsynthpanel_pagespeed off;\nadd_header X-Test yes;",
        );
        assert_eq!(http3, Some(true));
        assert_eq!(pagespeed, Some(false));
        assert!(kept.contains("add_header X-Test yes;"));
        assert!(!kept.contains("synthpanel_"));

        // Markers nested in a block are ordinary directives, not panel flags.
        let (kept, (http3, pagespeed)) =
            extract_feature_flags("location / {\n    synthpanel_http3 off;\n}\n");
        assert_eq!((http3, pagespeed), (None, None));
        assert!(kept.contains("synthpanel_http3 off;"));

        let mut options = VhostOptions {
            http3: true,
            pagespeed: false,
        };
        options.apply_directives("synthpanel_http3 off;\nsynthpanel_pagespeed on;");
        assert_eq!(
            options,
            VhostOptions {
                http3: false,
                pagespeed: true
            }
        );
    }

    #[tokio::test]
    async fn site_markers_are_stripped_and_applied_to_the_vhost() {
        let site = make_site_with(
            vec![],
            "static",
            "le",
            "synthpanel_http3 on;\nsynthpanel_pagespeed on;\nadd_header X-Test yes;",
        )
        .await;
        let text = render_vhost_with_options(
            &site,
            "panel.local",
            &SiteSecuritySettings::default(),
            None,
            VhostOptions::default(),
        );
        assert!(text.contains("listen 443 quic reuseport;"));
        assert!(text.contains("pagespeed on;"));
        assert!(text.contains("add_header X-Test yes;"));
        assert!(!text.contains("synthpanel_http3"));
        assert!(!text.contains("synthpanel_pagespeed"));
    }

    #[test]
    fn cache_snippet_declares_the_http_context_zone() {
        let snippet = render_cache_http_snippet();
        assert!(snippet.contains("proxy_cache_path"));
        assert!(snippet.contains("keys_zone=synthpanel:10m"));
    }
}

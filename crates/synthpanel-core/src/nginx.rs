//! Nginx vhost rendering and validation.

use crate::sites::SiteExtras;
use anyhow::Result;
use regex::Regex;

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

/// Render an Nginx vhost file for a site.
pub fn render_vhost(site: &SiteExtras, panel_hostname: &str) -> String {
    let extras = site;
    let site = &extras.site;
    let directives_source = extras.nginx.as_deref();
    let php_version = site.php_version.as_deref().unwrap_or("8.3");
    let all_names = site.all_server_names();
    let mut server_names: Vec<String> = all_names.clone();
    server_names.sort();
    server_names.dedup();
    let server_name = server_names;
    let ssl_mode = site.ssl_mode.as_str();

    let ssl_listen = match ssl_mode {
        "none" => "listen 80;".to_string(),
        _ => "listen 443 ssl;".to_string(),
    };
    let http_redirect = match ssl_mode {
        "none" => String::new(),
        _ => "    # HTTP to HTTPS redirect\n    if ($scheme = http) { return 301 https://$host$request_uri; }\n".to_string(),
    };

    let ssl_block = match ssl_mode {
        "none" => String::new(),
        "le" => format!(
            "    ssl_certificate /etc/letsencrypt/live/{domain}/fullchain.pem;\n\
             {indent}ssl_certificate_key /etc/letsencrypt/live/{domain}/privkey.pem;\n\
             {indent}ssl_protocols TLSv1.2 TLSv1.3;\n\
             {indent}ssl_session_cache shared:SSL:10m;\n\
             {indent}add_header Strict-Transport-Security \"max-age=63072000\" always;\n",
            domain = site.domain,
            indent = "    "
        ),
        "custom" => format!(
            "    ssl_certificate /etc/nginx/synthpanel/certs/{id}/cert.pem;\n\
             {indent}ssl_certificate_key /etc/nginx/synthpanel/certs/{id}/key.pem;\n",
            id = site.id,
            indent = "    "
        ),
        _ => String::new(),
    };

    let php_block = match site.template.as_str() {
        "static" | "node" | "nextjs" => String::new(),
        _ => format!(
            "    location ~ \\.php$ {{\n\
             {indent}fastcgi_pass unix:/run/php/php{ver}-sp-{id}.sock;\n\
             {indent}include /etc/nginx/snippets/fastcgi-php.conf;\n\
             {indent}}}\n",
            ver = php_version,
            id = site.id,
            indent = "        "
        ),
    };

    let directives_block = match directives_source {
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
    out.push_str(&ssl_block);
    out.push_str(&php_block);
    out.push_str(
        "    # Static files\n    location / {\n        try_files $uri $uri/ =404;\n    }\n\n",
    );
    out.push_str(&directives_block);
    out.push_str("}\n");

    let _ = panel_hostname;
    out
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

    async fn make_site(aliases: Vec<&str>, template: &str, ssl_mode: &str) -> SiteExtras {
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
                directives: String::new(),
            })
            .await
            .unwrap();
        sites.get_with_extras(&site.id).await.unwrap().unwrap()
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
}

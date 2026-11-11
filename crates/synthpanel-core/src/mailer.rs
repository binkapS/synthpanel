//! A small SMTP client and `sendmail` transport for alert delivery.
//!
//! The panel usually runs on a host with a local MTA (Postfix/Exim), so the
//! default transport pipes the message to `sendmail`. When no local MTA is
//! available the message is delivered over SMTP, optionally upgrading the
//! connection with STARTTLS and authenticating with AUTH PLAIN or
//! AUTH LOGIN.

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::Value;
use std::io;
use std::pin::Pin;
use std::task::Poll;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, ReadBuf};
use tokio::net::TcpStream;
use tracing::info;

/// A message to deliver.
#[derive(Debug, Clone)]
pub struct Mail {
    pub from: String,
    pub to: Vec<String>,
    pub subject: String,
    pub body: String,
}

impl Mail {
    /// Render the message as an RFC 5322 document.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("From: {}\r\n", self.from));
        out.push_str(&format!("To: {}\r\n", self.to.join(", ")));
        out.push_str(&format!("Subject: {}\r\n", self.subject));
        out.push_str("MIME-Version: 1.0\r\n");
        out.push_str("Content-Type: text/plain; charset=utf-8\r\n");
        out.push_str("\r\n");
        out.push_str(&self.body);
        out.push_str("\r\n");
        out
    }
}

/// Deliver a message using the transport described by `config`.
///
/// `config` understands two shapes:
///
/// ```json
/// { "command": "/usr/sbin/sendmail", "to": ["ops@example.com"] }
/// { "smtp": { "host": "localhost", "port": 25, "from": "panel@example.com" },
///   "to": ["ops@example.com"] }
/// ```
pub async fn send(config: &Value, mail: &Mail) -> Result<()> {
    if let Some(command) = config.get("command").and_then(Value::as_str) {
        return send_via_command(command, mail).await;
    }
    let smtp = config
        .get("smtp")
        .ok_or_else(|| anyhow!("the channel has neither a command nor an smtp section"))?;
    send_via_smtp(smtp, mail).await
}

/// Pipe the message into a local MTA.
async fn send_via_command(command: &str, mail: &Mail) -> Result<()> {
    let mut child = tokio::process::Command::new(command)
        .arg("-t")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("cannot run `{command}`"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(mail.render().as_bytes())
            .await
            .with_context(|| format!("cannot write to `{command}`"))?;
    }
    let output = child
        .wait_with_output()
        .await
        .with_context(|| format!("`{command}` did not finish"))?;
    if !output.status.success() {
        bail!(
            "`{command}` exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    info!(
        command,
        recipients = mail.to.len(),
        "alert delivered via the local mta"
    );
    Ok(())
}

/// Deliver the message over SMTP.
async fn send_via_smtp(config: &Value, mail: &Mail) -> Result<()> {
    let host = config
        .get("host")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("the smtp host is missing"))?;
    let port = config.get("port").and_then(Value::as_u64).unwrap_or(25) as u16;
    let username = config.get("username").and_then(Value::as_str);
    let password = config.get("password").and_then(Value::as_str);
    let want_tls = config
        .get("starttls")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let stream = TcpStream::connect((host, port))
        .await
        .with_context(|| format!("cannot connect to {host}:{port}"))?;
    let mut session = SmtpSession::new(stream).await?;

    session.expect_multi(&["220"]).await?;
    session.command("EHLO synthpanel").await?;
    let capabilities = session.read_response().await?;

    if want_tls {
        if !capabilities.to_uppercase().contains("STARTTLS") {
            bail!("the smtp server does not offer STARTTLS");
        }
        session.command("STARTTLS").await?;
        session.expect_multi(&["220"]).await?;
        session.upgrade(host).await?;
        session.command("EHLO synthpanel").await?;
        session.read_response().await?;
    }

    if let Some(username) = username {
        let password = password.unwrap_or_default();
        session.command("AUTH PLAIN").await?;
        let challenge = session.read_response().await?;
        if challenge.starts_with("334") {
            let secret = BASE64.encode(format!("\0{username}\0{password}"));
            session.write_line(&secret).await?;
            session.expect_multi(&["235"]).await?;
        } else if challenge.starts_with("535") {
            bail!("the smtp server rejected the credentials");
        }
    }

    let from = mail.from.clone();
    session.command(&format!("MAIL FROM:<{from}>")).await?;
    session.expect_multi(&["250"]).await?;
    for recipient in &mail.to {
        session.command(&format!("RCPT TO:<{recipient}>")).await?;
        session.expect_multi(&["250", "251"]).await?;
    }
    session.command("DATA").await?;
    session.expect_multi(&["354"]).await?;
    session.write_data(&mail.render()).await?;
    session.expect_multi(&["250"]).await?;
    session.command("QUIT").await?;

    info!(
        host,
        port,
        recipients = mail.to.len(),
        "alert delivered over smtp"
    );
    Ok(())
}

/// The transport an SMTP conversation runs over.
enum Transport {
    Plain(Option<TcpStream>),
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl AsyncRead for Transport {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Transport::Plain(stream) => match stream.as_mut() {
                Some(stream) => Pin::new(stream).poll_read(cx, buf),
                None => Poll::Ready(Ok(())),
            },
            Transport::Tls(stream) => Pin::new(stream.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Transport {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Transport::Plain(stream) => match stream.as_mut() {
                Some(stream) => Pin::new(stream).poll_write(cx, buf),
                None => Poll::Ready(Ok(0)),
            },
            Transport::Tls(stream) => Pin::new(stream.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Transport::Plain(stream) => match stream.as_mut() {
                Some(stream) => Pin::new(stream).poll_flush(cx),
                None => Poll::Ready(Ok(())),
            },
            Transport::Tls(stream) => Pin::new(stream.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Transport::Plain(stream) => match stream.as_mut() {
                Some(stream) => Pin::new(stream).poll_shutdown(cx),
                None => Poll::Ready(Ok(())),
            },
            Transport::Tls(stream) => Pin::new(stream.as_mut()).poll_shutdown(cx),
        }
    }
}

/// A minimal SMTP conversation.
struct SmtpSession {
    reader: BufReader<Transport>,
}

impl SmtpSession {
    async fn new(stream: TcpStream) -> Result<Self> {
        Ok(Self {
            reader: BufReader::new(Transport::Plain(Some(stream))),
        })
    }

    async fn write_line(&mut self, line: &str) -> Result<()> {
        let writer = self.reader.get_mut();
        writer.write_all(line.as_bytes()).await?;
        writer.write_all(b"\r\n").await?;
        writer.flush().await?;
        Ok(())
    }

    /// Write a message body, dot-stuffing lines that start with a dot.
    async fn write_data(&mut self, body: &str) -> Result<()> {
        let mut payload = String::with_capacity(body.len() + 16);
        for line in body.split('\n') {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if line.starts_with('.') {
                payload.push('.');
            }
            payload.push_str(line);
            payload.push_str("\r\n");
        }
        payload.push_str(".\r\n");
        let writer = self.reader.get_mut();
        writer.write_all(payload.as_bytes()).await?;
        writer.flush().await?;
        Ok(())
    }

    /// Read a (possibly multi-line) SMTP reply and return the whole thing.
    async fn read_response(&mut self) -> Result<String> {
        let mut full = String::new();
        loop {
            let mut line = String::new();
            let read = self.reader.read_line(&mut line).await?;
            if read == 0 {
                bail!("the smtp server closed the connection");
            }
            full.push_str(&line);
            // A reply ends with "250 " (space) instead of "250-" (dash).
            let trimmed = line.trim_end();
            if trimmed.len() < 4 || trimmed.as_bytes()[3] != b'-' {
                break;
            }
        }
        Ok(full)
    }

    /// Read a reply and require one of the accepted codes.
    async fn expect_multi(&mut self, codes: &[&str]) -> Result<()> {
        let response = self.read_response().await?;
        let code = response
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_string();
        if codes.contains(&code.as_str()) {
            Ok(())
        } else {
            bail!("the smtp server replied `{code}`: {}", response.trim())
        }
    }

    async fn command(&mut self, line: &str) -> Result<()> {
        self.write_line(line).await
    }

    /// Upgrade the connection with STARTTLS.
    async fn upgrade(&mut self, host: &str) -> Result<()> {
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(rustls::RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
            })
            .with_no_client_auth();
        let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(config));
        let server_name = rustls::pki_types::ServerName::try_from(host.to_string())
            .map_err(|error| anyhow!("invalid smtp host name: {error}"))?;

        let raw = match self.reader.get_mut() {
            Transport::Plain(stream) => stream
                .take()
                .ok_or_else(|| anyhow!("the smtp connection is already upgraded"))?,
            Transport::Tls(_) => bail!("the smtp connection is already encrypted"),
        };
        let stream = connector
            .connect(server_name, raw)
            .await
            .context("the smtp TLS handshake failed")?;
        // Drop anything the reader buffered before the handshake.
        let buffered = self.reader.buffer().len();
        self.reader.consume(buffered);
        *self.reader.get_mut() = Transport::Tls(Box::new(stream));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renders_headers_and_body() {
        let mail = Mail {
            from: "panel@example.com".into(),
            to: vec!["ops@example.com".into()],
            subject: "Backup failed".into(),
            body: "the backup did not finish".into(),
        };
        let rendered = mail.render();
        assert!(rendered.starts_with("From: panel@example.com\r\n"));
        assert!(rendered.contains("Subject: Backup failed\r\n"));
        assert!(rendered.contains("\r\n\r\nthe backup did not finish"));
    }

    #[tokio::test]
    async fn requires_a_transport() {
        let error = send(
            &json!({}),
            &Mail {
                from: "a@example.com".into(),
                to: vec!["b@example.com".into()],
                subject: "s".into(),
                body: "b".into(),
            },
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("neither a command nor an smtp section"));
    }

    #[tokio::test]
    async fn reports_a_missing_host() {
        let error = send(
            &json!({ "smtp": { "port": 25 } }),
            &Mail {
                from: "a@example.com".into(),
                to: vec!["b@example.com".into()],
                subject: "s".into(),
                body: "b".into(),
            },
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("smtp host is missing"));
    }
}

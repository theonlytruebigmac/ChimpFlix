//! SMTP / email delivery.
//!
//! Wraps `lettre` with our concept of "is email configured?" — every
//! feature that wants to send mail calls [`Mailer::from_settings`], and
//! a return value of `Ok(None)` means email is intentionally disabled
//! (no SMTP host stored). Callers fall back to whatever non-email path
//! makes sense for the feature (e.g. invites still return a copy-able
//! link in that case).
//!
//! Password lookup goes through the credential vault — never stored in
//! `server_settings` directly. The vault key is hard-coded to
//! [`SMTP_PASSWORD_SECRET`] so the admin "test" endpoint and the send
//! path agree on where to look.
//!
//! Transport choices:
//!   * `starttls` (default, port 587) — plain → upgraded via STARTTLS
//!   * `tls` (implicit TLS, port 465) — TLS from the first byte
//!   * `none` — plain SMTP, no encryption. Only sensible for a local
//!     submission relay on `localhost`.

use anyhow::{Context, Result, bail};
use chimpflix_common::Vault;
use chimpflix_library::{ServerSettings, queries};
use lettre::message::{Mailbox, MultiPart, SinglePart, header};
use lettre::transport::smtp::AsyncSmtpTransport;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncTransport, Message, Tokio1Executor};
use sqlx::SqlitePool;

/// Vault key under which the SMTP password is stored. Hard-coded so the
/// settings handler and the send path agree.
pub const SMTP_PASSWORD_SECRET: &str = "smtp_password";

const DEFAULT_PORT_STARTTLS: u16 = 587;
const DEFAULT_PORT_TLS: u16 = 465;
const DEFAULT_PORT_PLAIN: u16 = 25;

#[derive(Clone)]
pub struct Mailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl Mailer {
    /// Build a Mailer from the current settings + vault. Returns `Ok(None)`
    /// when email is intentionally disabled (no SMTP host configured) so
    /// feature code can branch cleanly without treating absence as error.
    pub async fn from_settings(
        settings: &ServerSettings,
        pool: &SqlitePool,
        vault: &Vault,
    ) -> Result<Option<Self>> {
        let Some(host) = settings.email_smtp_host.as_deref().map(str::trim) else {
            return Ok(None);
        };
        if host.is_empty() {
            return Ok(None);
        }
        let from_addr = settings
            .email_from_address
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .context("email_from_address required when SMTP is configured")?;

        let security = settings
            .email_smtp_security
            .as_deref()
            .unwrap_or("starttls")
            .to_ascii_lowercase();
        let port = settings.email_smtp_port.and_then(|n| u16::try_from(n).ok());

        let mut builder = match security.as_str() {
            "tls" => AsyncSmtpTransport::<Tokio1Executor>::relay(host)?,
            "starttls" => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)?,
            "none" => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host),
            other => bail!("unknown email_smtp_security: {other}"),
        };

        let resolved_port = port.unwrap_or(match security.as_str() {
            "tls" => DEFAULT_PORT_TLS,
            "starttls" => DEFAULT_PORT_STARTTLS,
            _ => DEFAULT_PORT_PLAIN,
        });
        builder = builder.port(resolved_port);

        // Auth is optional — if no username is set, we relay anonymously
        // (useful for a sendmail-style local submission daemon). If a
        // username is set but no password is in the vault, that's an
        // error — half-configured auth would silently fail at send time.
        if let Some(username) = settings.email_smtp_username.as_deref() {
            let username = username.trim();
            if !username.is_empty() {
                let password = queries::vault_get(pool, vault, SMTP_PASSWORD_SECRET)
                    .await
                    .context("load SMTP password from vault")?
                    .context("SMTP username is set but no password in the vault")?;
                builder = builder.credentials(Credentials::new(username.to_string(), password));
            }
        }

        let transport = builder.build();
        let display_name = settings
            .email_from_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let from = build_mailbox(from_addr, display_name)?;
        Ok(Some(Self { transport, from }))
    }

    /// Round-trip the SMTP server: connect, EHLO, AUTH if configured,
    /// QUIT. No message sent. Used by the admin "Test" button to surface
    /// misconfiguration before any user-visible email is queued.
    pub async fn test_connection(&self) -> Result<()> {
        self.transport
            .test_connection()
            .await
            .context("SMTP test connection")?;
        Ok(())
    }

    pub async fn send(&self, msg: OutgoingMessage<'_>) -> Result<()> {
        let to = build_mailbox(msg.to_address, msg.to_name)
            .with_context(|| format!("parse recipient address {}", msg.to_address))?;
        let message = Message::builder()
            .from(self.from.clone())
            .to(to)
            .subject(msg.subject)
            .multipart(
                MultiPart::alternative()
                    .singlepart(
                        SinglePart::builder()
                            .header(header::ContentType::TEXT_PLAIN)
                            .body(msg.text.to_string()),
                    )
                    .singlepart(
                        SinglePart::builder()
                            .header(header::ContentType::TEXT_HTML)
                            .body(msg.html.to_string()),
                    ),
            )
            .context("build email message")?;
        self.transport
            .send(message)
            .await
            .context("send email via SMTP")?;
        Ok(())
    }
}

pub struct OutgoingMessage<'a> {
    pub to_address: &'a str,
    pub to_name: Option<&'a str>,
    pub subject: &'a str,
    pub html: &'a str,
    pub text: &'a str,
}

fn build_mailbox(address: &str, display_name: Option<&str>) -> Result<Mailbox> {
    let parsed = address
        .parse()
        .with_context(|| format!("parse email address {address}"))?;
    let mailbox = match display_name {
        Some(name) if !name.is_empty() => Mailbox::new(Some(name.to_string()), parsed),
        _ => Mailbox::new(None, parsed),
    };
    Ok(mailbox)
}

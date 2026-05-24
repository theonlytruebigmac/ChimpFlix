//! Plex OAuth — PIN-based device flow.
//!
//! Plex's OAuth is built around short-lived PINs rather than the
//! classic redirect-with-code dance, so it works fine for a self-hosted
//! app that doesn't have a stable public redirect URL. The flow:
//!
//!   1. `create_pin` — POST `/api/v2/pins?strong=true`. Returns a numeric
//!      `id` and 4-letter `code`. The `id` is the server-side handle we
//!      poll on; the `code` is what the user types on plex.tv (or just
//!      part of the URL we redirect them to).
//!   2. Send the user to
//!      `https://app.plex.tv/auth#?clientID=…&code=…&context[device][product]=ChimpFlix`.
//!      Plex shows the "Sign in" screen, then on success flips the PIN
//!      from `authToken: null` to a real token.
//!   3. `poll_pin` — GET `/api/v2/pins/{id}`. Repeat until either an
//!      `authToken` materialises or `expiresAt` slides past now.
//!   4. `fetch_user` — GET `/api/v2/user` with the resulting token to
//!      retrieve the user's Plex identity (uuid, username, email, thumb).
//!
//! Plex auth tokens are long-lived (no refresh dance) so once we have
//! one, we don't store it. We just call `fetch_user` once to grab the
//! identity, then throw the token away.
//!
//! No client_secret: Plex authenticates the app via a
//! `X-Plex-Client-Identifier` header (any opaque stable string —
//! ChimpFlix generates a UUID on first enable). Anyone with that
//! identifier could impersonate "ChimpFlix" to plex.tv, so it shouldn't
//! be advertised, but it isn't a secret on the order of an OAuth
//! client_secret either.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use chimpflix_common::USER_AGENT;
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue, USER_AGENT as UA_HEADER};
use serde::Deserialize;

const PLEX_API_BASE: &str = "https://plex.tv";
const PLEX_AUTH_APP: &str = "https://app.plex.tv/auth";
/// Marketing product name we present to the user on Plex's sign-in
/// page. Kept as a constant — branding values that change later land
/// in env, but ChimpFlix's marketed name to *Plex* doesn't need to.
pub const PLEX_PRODUCT: &str = "ChimpFlix";
pub const PLEX_DEVICE: &str = "ChimpFlix Server";

#[derive(Clone)]
pub struct PlexOAuthClient {
    http: reqwest::Client,
    base_url: String,
    /// Stable per-install UUID. Provided by the operator (we generate
    /// and persist it on first enable). Identifies *this* ChimpFlix
    /// deployment to plex.tv — any two PINs issued with the same
    /// identifier are linked together by Plex's idea of "device".
    client_identifier: String,
}

impl PlexOAuthClient {
    pub fn new(client_identifier: &str) -> Result<Self> {
        let id = client_identifier.trim();
        if id.is_empty() {
            bail!("Plex client_identifier must be non-empty");
        }
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(UA_HEADER, HeaderValue::from_static(USER_AGENT));
        // Plex looks at these on every call; pinning them here means
        // every request — pin creation, polling, /user — uses the same
        // identity. Mismatching identifiers across the dance returns
        // 401 from `/pins/{id}` even when the PIN was approved.
        headers.insert(
            "X-Plex-Product",
            HeaderValue::from_static(PLEX_PRODUCT),
        );
        headers.insert(
            "X-Plex-Client-Identifier",
            HeaderValue::from_str(id).context("Plex client_identifier has non-ASCII bytes")?,
        );
        headers.insert("X-Plex-Device", HeaderValue::from_static(PLEX_DEVICE));
        headers.insert("X-Plex-Device-Name", HeaderValue::from_static(PLEX_PRODUCT));
        headers.insert("X-Plex-Platform", HeaderValue::from_static("Web"));
        headers.insert("X-Plex-Version", HeaderValue::from_static(env!("CARGO_PKG_VERSION")));
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(15))
            .build()
            .context("build Plex http client")?;
        Ok(Self {
            http,
            base_url: PLEX_API_BASE.to_string(),
            client_identifier: id.to_string(),
        })
    }

    /// Issue a fresh PIN. `strong=true` widens the resulting auth-token
    /// scope to all Plex resources we'd ever need; the default `strong=false`
    /// only grants access to a subset of endpoints (notably *not* `/user`
    /// in some Plex tier configurations).
    pub async fn create_pin(&self) -> Result<PlexPin> {
        let url = format!("{}/api/v2/pins?strong=true", self.base_url);
        let resp = self
            .http
            .post(&url)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            bail!(
                "Plex POST /pins returned {status}: {}",
                text.chars().take(200).collect::<String>()
            );
        }
        resp.json::<PlexPin>()
            .await
            .context("parse Plex /pins response")
    }

    /// Poll one PIN. Returns `Pending` while Plex hasn't seen the user
    /// approve, `Ready(token)` once they have, `Expired` after Plex's
    /// declared `expiresAt`. We don't differentiate "user denied" here
    /// because Plex's API doesn't — a denied PIN simply expires.
    pub async fn poll_pin(&self, pin_id: i64) -> Result<PinPollResult> {
        let url = format!("{}/api/v2/pins/{pin_id}", self.base_url);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        let status = resp.status();
        if status.as_u16() == 404 {
            // Plex 404s expired PINs after a grace period — treat as
            // expired so the UI can prompt for a fresh start without
            // a scary error.
            return Ok(PinPollResult::Expired);
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            bail!(
                "Plex GET /pins/{pin_id} returned {status}: {}",
                text.chars().take(200).collect::<String>()
            );
        }
        let pin: PlexPin = resp.json().await.context("parse Plex /pins/{id} response")?;
        if let Some(token) = pin.auth_token {
            return Ok(PinPollResult::Ready(token));
        }
        // Anything past `expiresAt` we treat as expired even if Plex
        // still serves the row. Clock skew between us and Plex isn't
        // common in practice (~seconds), so we don't pad.
        if pin.expires_in <= 0 {
            return Ok(PinPollResult::Expired);
        }
        Ok(PinPollResult::Pending)
    }

    /// Resolve a Plex auth-token to the underlying user. This is the
    /// only place we ever hand a per-user token to Plex — we don't
    /// store it after this call returns.
    pub async fn fetch_user(&self, auth_token: &str) -> Result<PlexUser> {
        let url = format!("{}/api/v2/user", self.base_url);
        let resp = self
            .http
            .get(&url)
            .header("X-Plex-Token", auth_token)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            bail!(
                "Plex GET /user returned {status}: {}",
                text.chars().take(200).collect::<String>()
            );
        }
        resp.json::<PlexUser>()
            .await
            .context("parse Plex /user response")
    }

    /// Build the URL we redirect the user to (or open in a popup) so
    /// they can approve the PIN on plex.tv. The `forward_url` is where
    /// Plex sends them after — typically our own `/login` so we can
    /// finalize the poll.
    pub fn auth_url(&self, code: &str, forward_url: Option<&str>) -> String {
        // Plex's auth page parses the URL fragment, so the params go
        // after a `#?` not a `?`. The leading `context[device][product]`
        // is what shows up as the label on the approval screen
        // ("Allow ChimpFlix to access your account?").
        let mut url = format!(
            "{PLEX_AUTH_APP}#?clientID={}&code={}&context%5Bdevice%5D%5Bproduct%5D={}",
            urlencode(&self.client_identifier),
            urlencode(code),
            urlencode(PLEX_PRODUCT),
        );
        if let Some(fwd) = forward_url {
            url.push_str("&forwardUrl=");
            url.push_str(&urlencode(fwd));
        }
        url
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(c),
            other => {
                let mut buf = [0u8; 4];
                for byte in other.encode_utf8(&mut buf).as_bytes() {
                    out.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct PlexPin {
    pub id: i64,
    pub code: String,
    /// `null` until the user approves; populated with a long-lived
    /// `authToken` afterwards.
    #[serde(default, rename = "authToken")]
    pub auth_token: Option<String>,
    /// Seconds remaining before Plex declares the PIN expired. Plex
    /// returns this freshly on each poll, so we can use it as the
    /// authoritative "have we run out of time".
    #[serde(default, rename = "expiresIn")]
    pub expires_in: i64,
}

#[derive(Debug, Clone)]
pub enum PinPollResult {
    Pending,
    Ready(String),
    Expired,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlexUser {
    /// Stable numeric Plex user id. We persist this as the
    /// `external_id` on `user_auth_providers` because the `uuid` field
    /// is also stable but the numeric id is what most other Plex APIs
    /// reference. Stored as a string so the column shape works for
    /// future non-numeric providers too.
    pub id: i64,
    pub uuid: String,
    pub username: String,
    /// Plex returns the user's primary email here. Optional — Plex
    /// allows account creation via Apple/Google ID with no email
    /// visible to apps in some configurations.
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub thumb: Option<String>,
    /// `true` when this is the owner of the linked Plex Home (or a
    /// solo account that's effectively its own home).
    #[serde(default)]
    pub home: bool,
    #[serde(default)]
    pub subscription: Option<PlexSubscription>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlexSubscription {
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub plan: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_blank_identifier() {
        assert!(PlexOAuthClient::new("").is_err());
        assert!(PlexOAuthClient::new("   ").is_err());
    }

    #[test]
    fn constructs_with_uuid() {
        let client =
            PlexOAuthClient::new("3f0b54c0-1ad3-4a8a-9c52-2e0a7b1f5b3a").expect("client");
        assert!(client.auth_url("ABCD", None).contains("code=ABCD"));
    }

    #[test]
    fn auth_url_includes_forward() {
        let client = PlexOAuthClient::new("client-id").unwrap();
        let url = client.auth_url("XYZ1", Some("https://flix.example.com/login"));
        assert!(url.contains("clientID=client-id"));
        assert!(url.contains("code=XYZ1"));
        assert!(url.contains("context%5Bdevice%5D%5Bproduct%5D=ChimpFlix"));
        assert!(url.contains("forwardUrl=https%3A%2F%2Fflix.example.com%2Flogin"));
    }
}

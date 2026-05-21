//! Trakt.tv API v2 client (OAuth device flow + sync).
//!
//! Two scopes of operation:
//!   - **App-scoped** — `device_code`, `poll_device_token`,
//!     `refresh_token`. Uses only the registered client_id + secret;
//!     no per-user auth required.
//!   - **User-scoped** — `push_history`, `pull_history`,
//!     `pull_playback`, `push_rating`, `pull_ratings`. Each takes the
//!     user's access_token and forwards it as the Bearer header.
//!
//! Token refresh is the caller's job: every user-scoped call returns
//! the parsed Trakt response or an error; if a 401 comes back the
//! caller refreshes the user's token via `refresh_token()` and retries.
//! Keeping refresh logic out of the client avoids passing a SqlitePool
//! into the metadata crate.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use chimpflix_common::USER_AGENT;
use reqwest::header::{
    ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, USER_AGENT as UA_HEADER,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::warn;

const TRAKT_BASE_URL: &str = "https://api.trakt.tv";
const TRAKT_API_VERSION: &str = "2";

/// JSON blob the operator pastes into the `trakt` vault slot.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TraktCreds {
    pub client_id: String,
    pub client_secret: String,
}

impl TraktCreds {
    pub fn parse(raw: &str) -> Result<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            bail!("Trakt credential value is empty");
        }
        let creds: TraktCreds = serde_json::from_str(trimmed)
            .context("Trakt credentials must be JSON with client_id/client_secret")?;
        if creds.client_id.trim().is_empty() || creds.client_secret.trim().is_empty() {
            bail!("Trakt credentials must include client_id and client_secret");
        }
        Ok(creds)
    }
}

#[derive(Clone)]
pub struct TraktClient {
    http: reqwest::Client,
    base_url: String,
    client_id: String,
    client_secret: String,
}

impl TraktClient {
    pub fn from_creds(creds: TraktCreds) -> Result<Self> {
        Self::new(&creds.client_id, &creds.client_secret)
    }

    pub fn new(client_id: &str, client_secret: &str) -> Result<Self> {
        let client_id = client_id.trim();
        let client_secret = client_secret.trim();
        if client_id.is_empty() || client_secret.is_empty() {
            bail!("Trakt client_id and client_secret must both be non-empty");
        }
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(UA_HEADER, HeaderValue::from_static(USER_AGENT));
        headers.insert("trakt-api-version", HeaderValue::from_static(TRAKT_API_VERSION));
        let api_key_value = HeaderValue::from_str(client_id)
            .context("Trakt client_id has non-ASCII characters")?;
        headers.insert("trakt-api-key", api_key_value);
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(15))
            .build()
            .context("build Trakt http client")?;
        Ok(Self {
            http,
            base_url: TRAKT_BASE_URL.to_string(),
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
        })
    }

    /// Validate the credentials by hitting the cheapest unauthenticated
    /// endpoint — `/oauth/device/code` returns immediately and tells us
    /// whether the client_id is recognised. We don't actually consume
    /// the resulting code (it expires unused).
    pub async fn validate(&self) -> Result<()> {
        let _ = self.device_code().await?;
        Ok(())
    }

    pub async fn device_code(&self) -> Result<DeviceCodeResponse> {
        let url = format!("{}/oauth/device/code", self.base_url);
        let body = json!({ "client_id": self.client_id });
        let resp = self.http.post(&url).json(&body).send().await
            .with_context(|| format!("POST {url}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            bail!(
                "Trakt /oauth/device/code returned {status}: {}",
                text.chars().take(200).collect::<String>()
            );
        }
        resp.json::<DeviceCodeResponse>()
            .await
            .context("parse Trakt device-code response")
    }

    /// Poll the device-token endpoint. Trakt returns a real token once
    /// the user has approved; until then it returns 400 (pending), 404
    /// (expired or wrong code), or 409 (already used). The caller is
    /// expected to drive the loop with the cadence Trakt suggested in
    /// `device_code`'s `interval` field.
    pub async fn poll_device_token(&self, device_code: &str) -> Result<DevicePollResult> {
        let url = format!("{}/oauth/device/token", self.base_url);
        let body = json!({
            "code": device_code,
            "client_id": self.client_id,
            "client_secret": self.client_secret,
        });
        let resp = self.http.post(&url).json(&body).send().await
            .with_context(|| format!("POST {url}"))?;
        let status = resp.status();
        if status.is_success() {
            let pair: TokenPair = resp.json().await.context("parse Trakt token response")?;
            return Ok(DevicePollResult::Ready(pair));
        }
        match status.as_u16() {
            400 => Ok(DevicePollResult::Pending),
            404 => Ok(DevicePollResult::Expired),
            409 => Ok(DevicePollResult::AlreadyApproved),
            410 => Ok(DevicePollResult::Expired),
            418 => Ok(DevicePollResult::Denied),
            429 => Ok(DevicePollResult::SlowDown),
            _ => {
                let text = resp.text().await.unwrap_or_default();
                bail!(
                    "Trakt /oauth/device/token returned {status}: {}",
                    text.chars().take(200).collect::<String>()
                );
            }
        }
    }

    pub async fn refresh_token(&self, refresh_token: &str) -> Result<TokenPair> {
        let url = format!("{}/oauth/token", self.base_url);
        let body = json!({
            "refresh_token": refresh_token,
            "client_id": self.client_id,
            "client_secret": self.client_secret,
            "grant_type": "refresh_token",
            "redirect_uri": "urn:ietf:wg:oauth:2.0:oob",
        });
        let resp = self.http.post(&url).json(&body).send().await
            .with_context(|| format!("POST {url}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            bail!(
                "Trakt /oauth/token (refresh) returned {status}: {}",
                text.chars().take(200).collect::<String>()
            );
        }
        resp.json::<TokenPair>().await.context("parse Trakt refresh response")
    }

    pub async fn push_history(
        &self,
        access_token: &str,
        entries: &[HistoryPush],
    ) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let mut movies = Vec::new();
        let mut episodes = Vec::new();
        for e in entries {
            match e {
                HistoryPush::Movie { tmdb_id, watched_at } => movies.push(json!({
                    "watched_at": watched_at,
                    "ids": { "tmdb": tmdb_id },
                })),
                HistoryPush::Episode { tmdb_show_id, season, episode, watched_at } => {
                    episodes.push(json!({
                        "watched_at": watched_at,
                        "show": { "ids": { "tmdb": tmdb_show_id } },
                        "seasons": [{
                            "number": season,
                            "episodes": [{ "number": episode }],
                        }],
                    }));
                }
            }
        }
        let body = json!({ "movies": movies, "shows": episodes });
        self.user_post("/sync/history", access_token, &body).await?;
        Ok(())
    }

    pub async fn pull_history(
        &self,
        access_token: &str,
        start_at_iso: Option<&str>,
    ) -> Result<Vec<HistoryEntry>> {
        let mut url = format!("{}/sync/history?limit=200", self.base_url);
        if let Some(s) = start_at_iso {
            url.push_str("&start_at=");
            url.push_str(&urlencode(s));
        }
        let resp = self
            .http
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {access_token}"))
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            return Err(api_error("GET /sync/history", resp).await);
        }
        resp.json::<Vec<HistoryEntry>>()
            .await
            .context("parse Trakt history")
    }

    pub async fn pull_playback(
        &self,
        access_token: &str,
    ) -> Result<Vec<PlaybackEntry>> {
        let url = format!("{}/sync/playback?limit=200", self.base_url);
        let resp = self
            .http
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {access_token}"))
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            return Err(api_error("GET /sync/playback", resp).await);
        }
        resp.json::<Vec<PlaybackEntry>>()
            .await
            .context("parse Trakt playback")
    }

    pub async fn push_rating(
        &self,
        access_token: &str,
        entry: RatingPush,
    ) -> Result<()> {
        let (movies, episodes) = match entry {
            RatingPush::Movie { tmdb_id, rating, rated_at } => (
                vec![json!({
                    "rated_at": rated_at,
                    "rating": rating,
                    "ids": { "tmdb": tmdb_id },
                })],
                vec![],
            ),
            RatingPush::Episode { tmdb_show_id, season, episode, rating, rated_at } => (
                vec![],
                vec![json!({
                    "rated_at": rated_at,
                    "show": { "ids": { "tmdb": tmdb_show_id } },
                    "seasons": [{
                        "number": season,
                        "episodes": [{
                            "number": episode,
                            "rating": rating,
                            "rated_at": rated_at,
                        }],
                    }],
                })],
            ),
        };
        let body = json!({ "movies": movies, "shows": episodes });
        self.user_post("/sync/ratings", access_token, &body).await?;
        Ok(())
    }

    pub async fn remove_rating(
        &self,
        access_token: &str,
        entry: RatingPush,
    ) -> Result<()> {
        let (movies, episodes) = match entry {
            RatingPush::Movie { tmdb_id, .. } => (
                vec![json!({ "ids": { "tmdb": tmdb_id } })],
                vec![],
            ),
            RatingPush::Episode { tmdb_show_id, season, episode, .. } => (
                vec![],
                vec![json!({
                    "show": { "ids": { "tmdb": tmdb_show_id } },
                    "seasons": [{
                        "number": season,
                        "episodes": [{ "number": episode }],
                    }],
                })],
            ),
        };
        let body = json!({ "movies": movies, "shows": episodes });
        self.user_post("/sync/ratings/remove", access_token, &body).await?;
        Ok(())
    }

    pub async fn pull_ratings(
        &self,
        access_token: &str,
    ) -> Result<Vec<RatingEntry>> {
        let url = format!("{}/sync/ratings", self.base_url);
        let resp = self
            .http
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {access_token}"))
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            return Err(api_error("GET /sync/ratings", resp).await);
        }
        resp.json::<Vec<RatingEntry>>()
            .await
            .context("parse Trakt ratings")
    }

    async fn user_post(
        &self,
        path: &str,
        access_token: &str,
        body: &serde_json::Value,
    ) -> Result<()> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .post(&url)
            .header(AUTHORIZATION, format!("Bearer {access_token}"))
            .json(body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        if !resp.status().is_success() {
            return Err(api_error(&format!("POST {path}"), resp).await);
        }
        Ok(())
    }
}

async fn api_error(label: &str, resp: reqwest::Response) -> anyhow::Error {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    warn!(%status, label, body = %text.chars().take(200).collect::<String>(), "Trakt API error");
    anyhow::anyhow!(
        "Trakt {label} returned {status}: {}",
        text.chars().take(200).collect::<String>()
    )
}

fn urlencode(s: &str) -> String {
    // Tiny inline encoder for the timestamp form Trakt expects
    // (ISO-8601 with `:` and `-`). We don't pull in `url` crate just
    // for this.
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(c),
            ':' => out.push_str("%3A"),
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
// Public projections
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub expires_in: i64,
    pub interval: i64,
}

#[derive(Debug, Clone)]
pub enum DevicePollResult {
    Pending,
    Ready(TokenPair),
    Expired,
    Denied,
    AlreadyApproved,
    SlowDown,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
    pub scope: Option<String>,
    pub created_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub enum HistoryPush {
    Movie {
        tmdb_id: i64,
        watched_at: String, // ISO-8601
    },
    Episode {
        tmdb_show_id: i64,
        season: i32,
        episode: i32,
        watched_at: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct HistoryEntry {
    pub id: i64,
    pub watched_at: String,
    pub action: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub movie: Option<TraktMovie>,
    #[serde(default)]
    pub episode: Option<TraktEpisode>,
    #[serde(default)]
    pub show: Option<TraktShow>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TraktMovie {
    pub title: String,
    pub year: Option<i32>,
    pub ids: TraktIds,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TraktShow {
    pub title: String,
    pub year: Option<i32>,
    pub ids: TraktIds,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TraktEpisode {
    pub season: i32,
    pub number: i32,
    pub title: Option<String>,
    pub ids: TraktIds,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct TraktIds {
    pub trakt: Option<i64>,
    pub tmdb: Option<i64>,
    pub imdb: Option<String>,
    pub tvdb: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaybackEntry {
    pub id: i64,
    pub progress: f64, // 0.0..100.0
    pub paused_at: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub movie: Option<TraktMovie>,
    #[serde(default)]
    pub episode: Option<TraktEpisode>,
    #[serde(default)]
    pub show: Option<TraktShow>,
}

#[derive(Debug, Clone)]
pub enum RatingPush {
    Movie {
        tmdb_id: i64,
        rating: i32,
        rated_at: String,
    },
    Episode {
        tmdb_show_id: i64,
        season: i32,
        episode: i32,
        rating: i32,
        rated_at: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct RatingEntry {
    pub rated_at: String,
    pub rating: i32,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub movie: Option<TraktMovie>,
    #[serde(default)]
    pub episode: Option<TraktEpisode>,
    #[serde(default)]
    pub show: Option<TraktShow>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_creds() {
        assert!(TraktClient::new("", "x").is_err());
        assert!(TraktClient::new("x", "").is_err());
        assert!(TraktClient::new("   ", "x").is_err());
    }

    #[test]
    fn constructs_with_valid_creds() {
        assert!(TraktClient::new("client", "secret").is_ok());
    }

    #[test]
    fn urlencode_keeps_alphanum_escapes_colon() {
        assert_eq!(urlencode("2024-01-19T12:34:56Z"), "2024-01-19T12%3A34%3A56Z");
    }
}

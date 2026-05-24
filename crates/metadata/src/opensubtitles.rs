//! OpenSubtitles.com REST API v1 client.
//!
//! Two-step credentials: an `Api-Key` header issued by registering an
//! app, plus a username+password used to mint a Bearer token (required
//! by the `/download` endpoint). The credential vault stores the whole
//! triple as one JSON blob under the `opensubtitles` slot so the admin
//! UI stays a single text input — see [`OpenSubtitlesCreds`].
//!
//! Search is unauthenticated; downloads need the Bearer. We log in lazily
//! on the first download and cache the token for the process lifetime.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use chimpflix_common::USER_AGENT;
use reqwest::header::{
    ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, USER_AGENT as UA_HEADER,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::warn;

const OS_BASE_URL: &str = "https://api.opensubtitles.com/api/v1";
const OS_API_KEY_HEADER: &str = "Api-Key";

/// Credentials triple packed into the vault slot's value field.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenSubtitlesCreds {
    pub api_key: String,
    pub username: String,
    pub password: String,
}

impl OpenSubtitlesCreds {
    pub fn parse(raw: &str) -> Result<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            bail!("OpenSubtitles credential value is empty");
        }
        let creds: OpenSubtitlesCreds = serde_json::from_str(trimmed)
            .context("OpenSubtitles credentials must be JSON with api_key/username/password")?;
        if creds.api_key.trim().is_empty()
            || creds.username.trim().is_empty()
            || creds.password.trim().is_empty()
        {
            bail!("OpenSubtitles credentials must include api_key, username, and password");
        }
        Ok(creds)
    }
}

#[derive(Clone)]
pub struct OpenSubtitlesClient {
    http: reqwest::Client,
    base_url: String,
    creds: OpenSubtitlesCreds,
    /// Bearer token cached after the first successful login. Cleared on
    /// 401 so the next download retries the login.
    token: Arc<Mutex<Option<String>>>,
}

impl OpenSubtitlesClient {
    pub fn new(creds: OpenSubtitlesCreds) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(UA_HEADER, HeaderValue::from_static(USER_AGENT));
        let api_key_value = HeaderValue::from_str(creds.api_key.trim())
            .context("OpenSubtitles api_key has non-ASCII characters")?;
        headers.insert(OS_API_KEY_HEADER, api_key_value);
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .context("build OpenSubtitles http client")?;
        Ok(Self {
            http,
            base_url: OS_BASE_URL.to_string(),
            creds,
            token: Arc::new(Mutex::new(None)),
        })
    }

    /// Confirm the api_key is valid and the username/password work for
    /// login. Used by the admin credential vault "test" button.
    pub async fn validate(&self) -> Result<()> {
        {
            let mut guard = self.token.lock().await;
            *guard = None;
        }
        self.token().await?;
        Ok(())
    }

    pub async fn search_for_movie(&self, params: SearchParams<'_>) -> Result<Vec<SubtitleHit>> {
        self.search_inner(params, None, None).await
    }

    pub async fn search_for_episode(
        &self,
        params: SearchParams<'_>,
        season: i32,
        episode: i32,
    ) -> Result<Vec<SubtitleHit>> {
        self.search_inner(params, Some(season), Some(episode)).await
    }

    async fn search_inner(
        &self,
        params: SearchParams<'_>,
        season: Option<i32>,
        episode: Option<i32>,
    ) -> Result<Vec<SubtitleHit>> {
        let mut query: Vec<(&str, String)> = Vec::new();
        if let Some(t) = params.tmdb_id {
            query.push(("tmdb_id", t.to_string()));
        }
        if let Some(i) = params.imdb_id {
            // OpenSubtitles wants the bare number (e.g. 0944947), not "tt..."
            let bare = i.trim_start_matches("tt");
            query.push(("imdb_id", bare.to_string()));
        }
        if !params.languages.is_empty() {
            query.push(("languages", params.languages.join(",")));
        }
        if let Some(s) = season {
            query.push(("season_number", s.to_string()));
        }
        if let Some(e) = episode {
            query.push(("episode_number", e.to_string()));
        }
        // Sort by download count so the most-used subtitle wins.
        query.push(("order_by", "download_count".to_string()));

        let url = format!("{}/subtitles", self.base_url);
        let resp = self
            .http
            .get(&url)
            .query(&query)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!(
                %status, %url,
                body = %body.chars().take(200).collect::<String>(),
                "OpenSubtitles search error"
            );
            bail!("OpenSubtitles search returned {status}");
        }
        let raw: SearchResponse = resp.json().await.context("parse OpenSubtitles JSON")?;
        Ok(raw
            .data
            .into_iter()
            .filter_map(|d| {
                let attrs = d.attributes?;
                let file = attrs.files.into_iter().next()?;
                Some(SubtitleHit {
                    file_id: file.file_id,
                    file_name: file.file_name.unwrap_or_default(),
                    language: attrs.language.unwrap_or_default(),
                    download_count: attrs.download_count.unwrap_or(0),
                    hearing_impaired: attrs.hearing_impaired.unwrap_or(false),
                    forced: attrs.foreign_parts_only.unwrap_or(false),
                    release: attrs.release,
                })
            })
            .collect())
    }

    /// Two-stage download: ask /download for a one-time link, then HTTP
    /// GET it. The link expires quickly so we don't bother caching.
    pub async fn download(&self, file_id: i64) -> Result<Vec<u8>> {
        let token = self.token().await?;
        let url = format!("{}/download", self.base_url);
        let body = serde_json::json!({ "file_id": file_id });
        let resp = self
            .http
            .post(&url)
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .json(&body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        let status = resp.status();
        if status.as_u16() == 401 {
            // Stale token; drop and retry once.
            let mut guard = self.token.lock().await;
            *guard = None;
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!(
                "OpenSubtitles /download returned {status}: {}",
                body.chars().take(200).collect::<String>()
            );
        }
        let link: DownloadResponse = resp.json().await.context("parse download response")?;
        let dl = self
            .http
            .get(&link.link)
            .send()
            .await
            .with_context(|| format!("GET {}", link.link))?;
        // Validate Content-Length up front when the server advertises
        // one — the OpenSubtitles download link is a one-time URL
        // signed by them, but a DNS hijack or compromised mirror
        // could return arbitrary payload. Cap at 10 MB; a real
        // subtitle (even verbose ASS for a full film) is well under
        // 2 MB. Anything bigger is suspicious.
        //
        // WEEK 1 #11 in `docs/PUBLIC_RELEASE_HARDENING.md` upgraded
        // this from a buffer-then-check (which still let the whole
        // body land in memory before deciding) to a streaming read
        // that bails as soon as the accumulator exceeds the cap. A
        // hostile upstream cranking out gigabytes at line rate is
        // now bounded by the cap, not by RAM.
        const MAX_SUBTITLE_BYTES: u64 = 10 * 1024 * 1024;
        if let Some(len) = dl.content_length() {
            if len > MAX_SUBTITLE_BYTES {
                bail!("OpenSubtitles download oversized: {len} bytes > {MAX_SUBTITLE_BYTES} max");
            }
        }
        use futures::StreamExt as _;
        let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
        let mut stream = dl.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("read subtitle chunk")?;
            buf.extend_from_slice(&chunk);
            if buf.len() as u64 > MAX_SUBTITLE_BYTES {
                bail!(
                    "OpenSubtitles download exceeded cap mid-stream ({} bytes > {} max)",
                    buf.len(),
                    MAX_SUBTITLE_BYTES,
                );
            }
        }
        let bytes = bytes::Bytes::from(buf);
        // Sniff the first few bytes against the formats we accept.
        // Picture-based / unknown payloads are rejected here rather
        // than getting stored and then surprising the player.
        if !looks_like_text_subtitle(&bytes) {
            bail!("OpenSubtitles download doesn't look like a text subtitle (SRT/ASS/SSA/VTT)");
        }
        Ok(bytes.to_vec())
    }

    async fn token(&self) -> Result<String> {
        // Hold the mutex across `login()` so concurrent callers queue
        // up behind us instead of each racing to POST `/login`. Same
        // hazard and fix as `TvdbClient::token` — the previous
        // "check, drop, login, re-take" pattern let two parallel
        // scans each trigger a fresh login on cold start, wasting
        // credentials and risking a 429 from OpenSubtitles.
        let mut guard = self.token.lock().await;
        if let Some(t) = guard.as_ref() {
            return Ok(t.clone());
        }
        let token = self.login().await?;
        *guard = Some(token.clone());
        Ok(token)
    }

    async fn login(&self) -> Result<String> {
        let url = format!("{}/login", self.base_url);
        let body = serde_json::json!({
            "username": self.creds.username,
            "password": self.creds.password,
        });
        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!(
                "OpenSubtitles login returned {status}: {}",
                body.chars().take(200).collect::<String>()
            );
        }
        let parsed: LoginResponse = resp.json().await.context("parse login response")?;
        parsed
            .token
            .ok_or_else(|| anyhow!("OpenSubtitles login returned no token"))
    }
}

/// Best-effort heuristic that the payload is a text subtitle in one
/// of the formats we accept (SRT, ASS/SSA, WebVTT). We don't fully
/// parse — just check the first ~256 bytes for a recognizable opener.
/// The defence here is against a download link being hijacked to
/// return arbitrary binary (executable, exploit payload, multi-MB
/// junk), not against malformed-but-real subtitles.
fn looks_like_text_subtitle(bytes: &[u8]) -> bool {
    // Trim BOM + leading whitespace before sniffing.
    let mut head = bytes;
    if head.starts_with(&[0xEF, 0xBB, 0xBF]) {
        head = &head[3..];
    }
    while head
        .first()
        .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
    {
        head = &head[1..];
    }
    let head = &head[..head.len().min(256)];
    let as_str = match std::str::from_utf8(head) {
        Ok(s) => s,
        // Not valid UTF-8 — could still be Latin-1 SRT. Accept ONLY
        // if every byte looks textual: ASCII printable / common
        // whitespace, or the Latin-1 supplement range (0xa0-0xff).
        // The C1 control range (0x80-0x9f) and NUL/DEL are rejected
        // — they're what binary headers (PNG \x89, etc.) hit.
        Err(_) => {
            return head
                .iter()
                .all(|&b| matches!(b, 0x20..=0x7e | b'\t' | b'\n' | b'\r' | 0xa0..=0xff));
        }
    };
    // WebVTT: must start with the magic header.
    if as_str.starts_with("WEBVTT") {
        return true;
    }
    // ASS / SSA: section headers.
    if as_str.starts_with("[Script Info]")
        || as_str.starts_with("[V4 Styles]")
        || as_str.starts_with("[V4+ Styles]")
        || as_str.starts_with("[Events]")
    {
        return true;
    }
    // SRT: first event index, e.g. "1\n00:00:01,000 --> ...".
    // Accept any leading digit followed within ~30 chars by an arrow.
    if as_str.bytes().next().is_some_and(|b| b.is_ascii_digit()) && as_str.contains("-->") {
        return true;
    }
    false
}

#[cfg(test)]
mod sniff_tests {
    use super::looks_like_text_subtitle;

    #[test]
    fn accepts_webvtt() {
        assert!(looks_like_text_subtitle(
            b"WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nHi"
        ));
    }
    #[test]
    fn accepts_ass() {
        assert!(looks_like_text_subtitle(b"[Script Info]\nTitle: x\n"));
    }
    #[test]
    fn accepts_srt() {
        assert!(looks_like_text_subtitle(
            b"1\n00:00:01,000 --> 00:00:02,000\nHi\n"
        ));
    }
    #[test]
    fn accepts_srt_with_bom() {
        assert!(looks_like_text_subtitle(
            b"\xef\xbb\xbf1\n00:00:01,000 --> 00:00:02,000\nHi\n"
        ));
    }
    #[test]
    fn rejects_png() {
        assert!(!looks_like_text_subtitle(b"\x89PNG\r\n\x1a\n"));
    }
    #[test]
    fn rejects_zip() {
        assert!(!looks_like_text_subtitle(b"PK\x03\x04"));
    }
}

// ---------------------------------------------------------------------------
// Public projections
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SearchParams<'a> {
    pub tmdb_id: Option<i64>,
    pub imdb_id: Option<&'a str>,
    pub languages: &'a [String],
}

#[derive(Debug, Clone)]
pub struct SubtitleHit {
    pub file_id: i64,
    pub file_name: String,
    pub language: String,
    pub download_count: i64,
    pub hearing_impaired: bool,
    pub forced: bool,
    pub release: Option<String>,
}

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    data: Vec<RawDatum>,
}

#[derive(Debug, Deserialize)]
struct RawDatum {
    #[serde(default)]
    attributes: Option<RawAttributes>,
}

#[derive(Debug, Deserialize)]
struct RawAttributes {
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    download_count: Option<i64>,
    #[serde(default)]
    hearing_impaired: Option<bool>,
    #[serde(default)]
    foreign_parts_only: Option<bool>,
    #[serde(default)]
    release: Option<String>,
    #[serde(default)]
    files: Vec<RawFile>,
}

#[derive(Debug, Deserialize)]
struct RawFile {
    file_id: i64,
    #[serde(default)]
    file_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LoginResponse {
    #[serde(default)]
    token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DownloadResponse {
    link: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_creds_round_trip() {
        let json = r#"{"api_key":"k","username":"u","password":"p"}"#;
        let c = OpenSubtitlesCreds::parse(json).unwrap();
        assert_eq!(c.api_key, "k");
        assert_eq!(c.username, "u");
        assert_eq!(c.password, "p");
    }

    #[test]
    fn parse_creds_rejects_empty_fields() {
        let bad = r#"{"api_key":"k","username":"","password":"p"}"#;
        assert!(OpenSubtitlesCreds::parse(bad).is_err());
    }

    #[test]
    fn parse_creds_rejects_garbage() {
        assert!(OpenSubtitlesCreds::parse("not json").is_err());
    }

    #[test]
    fn client_constructs_with_valid_creds() {
        let c = OpenSubtitlesCreds {
            api_key: "abc".into(),
            username: "u".into(),
            password: "p".into(),
        };
        assert!(OpenSubtitlesClient::new(c).is_ok());
    }
}

//! MyAnimeList API v2 client — anime ranking only (for now).
//!
//! The public `GET /v2/anime/ranking` endpoint authenticates with just an
//! app **Client ID** sent as the `X-MAL-CLIENT-ID` header — no per-user
//! OAuth. That's all the per-library anime "Top 10" rail needs. (User
//! anime-list import is a separate, OAuth-gated feature; not built here.)
//!
//! MAL doesn't publish a hard rate limit; community consensus is to keep
//! it gentle. Callers wrap calls in the `mal` circuit breaker, and 429s
//! surface in the error message so `error_class::classify` routes them to
//! the long rate-limit backoff (and trips the breaker).

use anyhow::{Context, Result};
use chimpflix_common::USER_AGENT;
use reqwest::header::{ACCEPT, HeaderMap, HeaderName, HeaderValue, USER_AGENT as UA_HEADER};
use serde::Deserialize;

const MAL_BASE: &str = "https://api.myanimelist.net/v2";

/// One entry from the anime ranking. `mal_id` is MAL's primary key; the
/// caller resolves it to local cross-ids (tvdb/tmdb/anilist) through the
/// anime-id map. `poster_url` is an absolute MAL CDN URL (unused by the
/// rail, which renders matched local items' own art — kept for parity /
/// future fallback display).
#[derive(Debug, Clone)]
pub struct MalRankingEntry {
    pub mal_id: i64,
    pub rank: i64,
    pub title: String,
    pub poster_url: Option<String>,
}

#[derive(Clone)]
pub struct MalClient {
    http: reqwest::Client,
    client_id: String,
}

impl MalClient {
    /// Build a client that adds `X-MAL-CLIENT-ID` to every request. An
    /// empty client id is rejected — callers build this only when the
    /// vault holds a non-empty id.
    pub fn new(client_id: &str) -> Result<Self> {
        let client_id = client_id.trim().to_string();
        if client_id.is_empty() {
            anyhow::bail!("MyAnimeList client id is empty");
        }
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(UA_HEADER, HeaderValue::from_static(USER_AGENT));
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .context("build MyAnimeList http client")?;
        Ok(Self { http, client_id })
    }

    fn client_id_header(&self) -> Result<(HeaderName, HeaderValue)> {
        let name = HeaderName::from_static("x-mal-client-id");
        let val = HeaderValue::from_str(&self.client_id)
            .context("MyAnimeList client id is not a valid header value")?;
        Ok((name, val))
    }

    /// Top-ranked anime. `ranking_type` ∈ all | airing | upcoming | tv |
    /// ova | movie | special | bypopularity | favorite (we use "all" =
    /// top by score). `limit` is clamped to MAL's 1..=100 page size.
    pub async fn top_anime(&self, ranking_type: &str, limit: u32) -> Result<Vec<MalRankingEntry>> {
        let limit = limit.clamp(1, 100);
        let (hname, hval) = self.client_id_header()?;
        let resp = self
            .http
            .get(format!("{MAL_BASE}/anime/ranking"))
            .header(hname, hval)
            .query(&[
                ("ranking_type", ranking_type),
                ("limit", &limit.to_string()),
                ("fields", "id,title,main_picture,mean,rank"),
            ])
            .send()
            .await
            .context("MyAnimeList ranking request")?;
        let status = resp.status();
        if !status.is_success() {
            // Surface the numeric status so error_class::classify can see
            // an "http 429" and route it to the rate-limit backoff / trip
            // the circuit breaker. 401/403 → auth class (bad client id).
            let body = resp.text().await.unwrap_or_default();
            let snippet: String = body.chars().take(200).collect();
            anyhow::bail!("MyAnimeList ranking: http {} {snippet}", status.as_u16());
        }
        let parsed: RankingResponse = resp
            .json()
            .await
            .context("MyAnimeList ranking: decode body")?;
        Ok(parsed
            .data
            .into_iter()
            .enumerate()
            .map(|(i, row)| MalRankingEntry {
                mal_id: row.node.id,
                // MAL's `ranking.rank` is the global rank; for a paged
                // ranking_type=all it matches the row order. Use the row
                // index (1-based) as the displayed rank so it's always a
                // clean 1..N regardless of which ranking_type was used.
                rank: row
                    .ranking
                    .and_then(|r| r.rank)
                    .unwrap_or((i as i64) + 1),
                title: row.node.title,
                poster_url: row.node.main_picture.and_then(|p| p.large.or(p.medium)),
            })
            .collect())
    }

    /// Cheap call for the admin credential "test" button — pulls a
    /// single-entry ranking to confirm the client id is accepted.
    pub async fn validate(&self) -> Result<()> {
        self.top_anime("all", 1).await.map(|_| ())
    }
}

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RankingResponse {
    #[serde(default)]
    data: Vec<RankingRow>,
}

#[derive(Debug, Deserialize)]
struct RankingRow {
    node: RankingNode,
    #[serde(default)]
    ranking: Option<RankingMeta>,
}

#[derive(Debug, Deserialize)]
struct RankingNode {
    id: i64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    main_picture: Option<MainPicture>,
}

#[derive(Debug, Deserialize)]
struct MainPicture {
    #[serde(default)]
    medium: Option<String>,
    #[serde(default)]
    large: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RankingMeta {
    #[serde(default)]
    rank: Option<i64>,
}

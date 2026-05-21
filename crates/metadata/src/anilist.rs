//! AniList GraphQL client.
//!
//! Used as the *primary* metadata source for anime libraries — AniList
//! has the canonical episode counts, romaji/english/native title triples,
//! and AniDB cross-references that TMDB and TVDB consistently get wrong
//! for anime. TMDB and TVDB still run after AniList to fill any remaining
//! nulls (release dates outside Japan, certain backdrops).
//!
//! No API key is required for read traffic; unauthenticated calls get
//! 30 requests/minute. Providing an OAuth token lifts the limit to 90.
//! We support the token form so the credential vault has somewhere to put
//! it, but build a working client either way.

use std::sync::Arc;

use anyhow::{Context, Result};
use chimpflix_common::USER_AGENT;
use reqwest::header::{
    ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, USER_AGENT as UA_HEADER,
};
use serde::Deserialize;
use serde_json::json;
use tracing::warn;

const ANILIST_URL: &str = "https://graphql.anilist.co";

#[derive(Clone)]
pub struct AniListClient {
    http: reqwest::Client,
    url: String,
    /// Cached bearer token. Kept in an Arc so cloning the client doesn't
    /// fan out to per-clone tokens — there's just one credential per
    /// install.
    token: Arc<Option<String>>,
}

impl AniListClient {
    /// Build an unauthenticated client (30 req/min).
    pub fn unauthenticated() -> Result<Self> {
        Self::build(None)
    }

    /// Build a client that adds `Authorization: Bearer <token>` to every
    /// request (90 req/min).
    pub fn with_token(token: &str) -> Result<Self> {
        Self::build(Some(token.trim().to_string()))
    }

    fn build(token: Option<String>) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(UA_HEADER, HeaderValue::from_static(USER_AGENT));
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .context("build AniList http client")?;
        Ok(Self {
            http,
            url: ANILIST_URL.to_string(),
            token: Arc::new(token.filter(|t| !t.is_empty())),
        })
    }

    /// Hit a trivial query (Viewer or 1-result Media search) to confirm
    /// the client is wired up and any token is accepted. Used by the
    /// admin credential vault "test" button.
    pub async fn validate(&self) -> Result<()> {
        // The minimal query that exercises both the GraphQL pipeline and
        // (if a token is set) the auth header. SiteStatistics returns a
        // tiny payload and works without authentication.
        let resp: GraphQlResponse<SiteStatsData> =
            self.post_graphql(SITE_STATS_QUERY, &json!({})).await?;
        resp.data
            .context("AniList responded but the data was missing")?;
        Ok(())
    }

    /// Single best-match search. `type: ANIME` and the optional year
    /// scoping correspond to AniList's most-popular sort.
    pub async fn lookup_show(&self, query: &str, year: Option<i32>) -> Result<Option<AniListShow>> {
        let vars = match year {
            Some(y) => json!({
                "search": query,
                "type": "ANIME",
                "startDate_greater": format!("{}0101", y),
                "startDate_lesser": format!("{}1231", y),
            }),
            None => json!({
                "search": query,
                "type": "ANIME",
            }),
        };
        let resp: GraphQlResponse<MediaWrapper> = self
            .post_graphql(MEDIA_QUERY, &vars)
            .await
            .with_context(|| format!("AniList search {query:?}"))?;
        let Some(data) = resp.data else {
            return Ok(None);
        };
        Ok(data.media.map(AniListShow::from_raw))
    }

    pub async fn fetch_show(&self, anilist_id: i64) -> Result<AniListShow> {
        let resp: GraphQlResponse<MediaWrapper> = self
            .post_graphql(MEDIA_QUERY, &json!({ "id": anilist_id }))
            .await
            .with_context(|| format!("AniList fetch id={anilist_id}"))?;
        let data = resp
            .data
            .with_context(|| format!("AniList returned no data for id={anilist_id}"))?;
        data.media
            .map(AniListShow::from_raw)
            .with_context(|| format!("AniList id={anilist_id} not found"))
    }

    async fn post_graphql<T: for<'de> Deserialize<'de>>(
        &self,
        query: &str,
        variables: &serde_json::Value,
    ) -> Result<GraphQlResponse<T>> {
        // AniList allows 30 req/min unauthenticated, 90/min with a
        // token. On a large anime library scan the worker chews
        // through that bucket fast and a 429 used to just bubble up
        // as a hard failure — half the library would silently miss
        // enrichment. Handle 429 by reading `Retry-After` and waiting
        // it out once before giving up. The retry budget is one
        // attempt: if AniList tells us 60s+, we honor it and try
        // again; if it lies or the second attempt also 429s, the
        // caller will log a warn and move on.
        for attempt in 0..2 {
            let mut req = self
                .http
                .post(&self.url)
                .json(&json!({ "query": query, "variables": variables }));
            if let Some(token) = self.token.as_ref() {
                req = req.header(AUTHORIZATION, format!("Bearer {token}"));
            }
            let resp = req
                .send()
                .await
                .with_context(|| format!("POST {}", self.url))?;
            let status = resp.status();
            if status.as_u16() == 429 && attempt == 0 {
                let wait_s = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(60)
                    .min(120);
                warn!(
                    wait_s,
                    "AniList rate-limited (429); sleeping then retrying once"
                );
                tokio::time::sleep(std::time::Duration::from_secs(wait_s)).await;
                continue;
            }
            return Self::parse_anilist_response(resp, status).await;
        }
        anyhow::bail!("AniList POST kept 429-ing after rate-limit wait")
    }

    async fn parse_anilist_response<T: for<'de> Deserialize<'de>>(
        resp: reqwest::Response,
        status: reqwest::StatusCode,
    ) -> Result<GraphQlResponse<T>> {
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!(
                %status,
                body = %body.chars().take(200).collect::<String>(),
                "AniList HTTP error"
            );
            anyhow::bail!("AniList POST returned {status}");
        }
        let parsed: GraphQlResponse<T> = resp.json().await.context("parse AniList JSON")?;
        if let Some(errs) = &parsed.errors {
            if !errs.is_empty() {
                let msg = errs
                    .iter()
                    .map(|e| e.message.as_str())
                    .collect::<Vec<_>>()
                    .join("; ");
                anyhow::bail!("AniList returned errors: {msg}");
            }
        }
        Ok(parsed)
    }
}

// ---------------------------------------------------------------------------
// Public projection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AniListShow {
    pub anilist_id: i64,
    pub mal_id: Option<i64>,
    /// Title in the order: english if present, then romaji, then native.
    /// Mirrors what most anime catalogue UIs do.
    pub title: String,
    /// Native (usually Japanese) title for the "alternative title" UI row.
    pub original_title: Option<String>,
    pub romaji_title: Option<String>,
    pub summary: Option<String>,
    pub year: Option<i32>,
    pub episode_count: Option<i32>,
    pub episode_duration_minutes: Option<i32>,
    pub format: Option<String>,
    pub status: Option<String>,
    pub genres: Vec<String>,
    pub studios: Vec<String>,
    pub poster_url: Option<String>,
    pub backdrop_url: Option<String>,
    pub average_score_percent: Option<i32>,
}

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct GraphQlResponse<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Debug, Deserialize)]
struct GraphQlError {
    #[serde(default)]
    message: String,
}

#[derive(Debug, Deserialize)]
struct MediaWrapper {
    #[serde(rename = "Media")]
    media: Option<RawMedia>,
}

#[derive(Debug, Deserialize)]
struct SiteStatsData {
    #[serde(rename = "SiteStatistics", default)]
    _site_statistics: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct RawMedia {
    id: i64,
    #[serde(rename = "idMal", default)]
    id_mal: Option<i64>,
    #[serde(default)]
    title: Option<RawTitle>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default, rename = "startDate")]
    start_date: Option<RawDate>,
    #[serde(default)]
    episodes: Option<i32>,
    #[serde(default)]
    duration: Option<i32>,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    genres: Vec<String>,
    #[serde(default, rename = "averageScore")]
    average_score: Option<i32>,
    #[serde(default, rename = "coverImage")]
    cover_image: Option<RawCoverImage>,
    #[serde(default, rename = "bannerImage")]
    banner_image: Option<String>,
    #[serde(default)]
    studios: Option<RawStudios>,
}

#[derive(Debug, Deserialize)]
struct RawTitle {
    #[serde(default)]
    romaji: Option<String>,
    #[serde(default)]
    english: Option<String>,
    #[serde(default)]
    native: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawDate {
    #[serde(default)]
    year: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct RawCoverImage {
    #[serde(default, rename = "extraLarge")]
    extra_large: Option<String>,
    #[serde(default)]
    large: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawStudios {
    #[serde(default)]
    nodes: Vec<RawStudio>,
}

#[derive(Debug, Deserialize)]
struct RawStudio {
    #[serde(default)]
    name: Option<String>,
}

impl AniListShow {
    fn from_raw(r: RawMedia) -> Self {
        let title_struct = r.title.unwrap_or(RawTitle {
            romaji: None,
            english: None,
            native: None,
        });
        let romaji_title = title_struct.romaji.clone().filter(|s| !s.is_empty());
        let english = title_struct.english.clone().filter(|s| !s.is_empty());
        let native = title_struct.native.clone().filter(|s| !s.is_empty());
        let title = english
            .clone()
            .or_else(|| romaji_title.clone())
            .or_else(|| native.clone())
            .unwrap_or_else(|| format!("AniList #{}", r.id));
        let original_title = native.filter(|n| Some(n) != english.as_ref());
        let summary = r
            .description
            .map(|s| strip_html(&s))
            .filter(|s| !s.is_empty());

        let cover = r.cover_image.unwrap_or(RawCoverImage {
            extra_large: None,
            large: None,
        });
        let poster_url = cover.extra_large.or(cover.large);

        let studios = r
            .studios
            .map(|s| s.nodes.into_iter().filter_map(|n| n.name).collect())
            .unwrap_or_default();

        Self {
            anilist_id: r.id,
            mal_id: r.id_mal,
            title,
            original_title,
            romaji_title,
            summary,
            year: r.start_date.and_then(|d| d.year),
            episode_count: r.episodes,
            episode_duration_minutes: r.duration,
            format: r.format,
            status: r.status,
            genres: r.genres,
            studios,
            poster_url,
            backdrop_url: r.banner_image,
            average_score_percent: r.average_score,
        }
    }
}

/// AniList descriptions are HTML (`<br>`, `<i>`, occasionally entities).
/// Same approach as the TVMaze stripper — tag-free output, whitespace
/// collapsed, no HTML-parser dependency. We treat every tag as an
/// implicit single-space separator (so `Foo<br>Bar` becomes `Foo Bar`),
/// then collapse the resulting whitespace runs and swallow any space
/// that lands directly before punctuation (the common artifact of
/// stripping closing tags like `</i>.`).
fn strip_html(s: &str) -> String {
    let mut tagless = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                tagless.push(' ');
            }
            _ if !in_tag => tagless.push(c),
            _ => {}
        }
    }

    let mut out = String::with_capacity(tagless.len());
    let mut pending_space = false;
    for c in tagless.chars() {
        if c.is_whitespace() {
            pending_space = true;
            continue;
        }
        let is_punct = matches!(c, '.' | ',' | '!' | '?' | ';' | ':');
        if pending_space && !out.is_empty() && !is_punct {
            out.push(' ');
        }
        out.push(c);
        pending_space = false;
    }
    out.trim().to_string()
}

const MEDIA_QUERY: &str = r#"
query ($id: Int, $search: String, $type: MediaType, $startDate_greater: FuzzyDateInt, $startDate_lesser: FuzzyDateInt) {
  Media(id: $id, search: $search, type: $type, startDate_greater: $startDate_greater, startDate_lesser: $startDate_lesser, sort: POPULARITY_DESC) {
    id
    idMal
    title { romaji english native }
    description(asHtml: false)
    startDate { year }
    episodes
    duration
    format
    status
    genres
    averageScore
    coverImage { extraLarge large }
    bannerImage
    studios(isMain: true) { nodes { name } }
  }
}
"#;

const SITE_STATS_QUERY: &str = r#"
query {
  SiteStatistics {
    anime { nodes { count } }
  }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse_show(raw: serde_json::Value) -> AniListShow {
        AniListShow::from_raw(serde_json::from_value(raw).unwrap())
    }

    #[test]
    fn picks_english_then_romaji_then_native() {
        let s = parse_show(json!({
            "id": 1,
            "title": { "romaji": "Sousou no Frieren", "english": "Frieren", "native": "葬送のフリーレン" }
        }));
        assert_eq!(s.title, "Frieren");
        assert_eq!(s.romaji_title.as_deref(), Some("Sousou no Frieren"));
        assert_eq!(s.original_title.as_deref(), Some("葬送のフリーレン"));
    }

    #[test]
    fn falls_through_when_english_absent() {
        let s = parse_show(json!({
            "id": 2,
            "title": { "romaji": "Bocchi the Rock!", "english": null, "native": "ぼっち・ざ・ろっく！" }
        }));
        assert_eq!(s.title, "Bocchi the Rock!");
        assert_eq!(s.original_title.as_deref(), Some("ぼっち・ざ・ろっく！"));
    }

    #[test]
    fn original_title_hidden_when_same_as_english() {
        let s = parse_show(json!({
            "id": 3,
            "title": { "romaji": "x", "english": "Same", "native": "Same" }
        }));
        // We hide original_title when it duplicates english to avoid a
        // redundant "alternative title" row in the UI.
        assert_eq!(s.original_title, None);
    }

    #[test]
    fn description_html_is_stripped() {
        let s = parse_show(json!({
            "id": 4,
            "title": { "romaji": "x" },
            "description": "Line one.<br><br>Line <i>two</i>.",
        }));
        assert_eq!(s.summary.as_deref(), Some("Line one. Line two."));
    }

    #[test]
    fn year_from_start_date_fuzzy() {
        let s = parse_show(json!({
            "id": 5,
            "title": { "romaji": "x" },
            "startDate": { "year": 2023 }
        }));
        assert_eq!(s.year, Some(2023));
    }

    #[test]
    fn poster_prefers_extra_large() {
        let s = parse_show(json!({
            "id": 6,
            "title": { "romaji": "x" },
            "coverImage": { "extraLarge": "xl.jpg", "large": "l.jpg" }
        }));
        assert_eq!(s.poster_url.as_deref(), Some("xl.jpg"));
    }

    #[test]
    fn unauth_client_builds() {
        assert!(AniListClient::unauthenticated().is_ok());
    }
}

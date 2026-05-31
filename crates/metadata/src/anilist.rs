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

    /// Per-episode metadata for a given AniList show id.
    ///
    /// AniList's GraphQL schema has no native per-episode title list. The
    /// only per-episode data the API exposes is `streamingEpisodes`,
    /// which AniList populates from the legal-stream listings it knows
    /// about (Crunchyroll, HiDive, etc.). For shows that aren't streamed
    /// where AniList tracks streams — older or niche anime, currently-
    /// airing seasons before their first official sub drops, anything
    /// region-blocked from AniList's scrapers — `streamingEpisodes`
    /// is empty and this returns `Ok(vec![])`. Callers should treat
    /// empty as "AniList has no episode data for this id," not as an
    /// error.
    ///
    /// The returned vector is **sorted by episode number ascending**;
    /// AniList returns streamingEpisodes in upload order (most-recent
    /// first) and entries without a parseable episode number are
    /// dropped from the output. See `parse_streaming_episode_title` for
    /// the parsing heuristic.
    pub async fn fetch_episodes(&self, anilist_id: i64) -> Result<Vec<AniListEpisode>> {
        let resp: GraphQlResponse<EpisodesWrapper> = self
            .post_graphql(EPISODES_QUERY, &json!({ "id": anilist_id }))
            .await
            .with_context(|| format!("AniList fetch_episodes id={anilist_id}"))?;
        let Some(data) = resp.data else {
            return Ok(Vec::new());
        };
        let Some(media) = data.media else {
            return Ok(Vec::new());
        };
        let mut out: Vec<AniListEpisode> = media
            .streaming_episodes
            .into_iter()
            .filter_map(|raw| {
                let parsed = parse_streaming_episode_title(raw.title.as_deref()?)?;
                Some(AniListEpisode {
                    episode_number: parsed.episode_number,
                    title: parsed.title,
                    thumbnail_url: raw.thumbnail,
                })
            })
            .collect();
        out.sort_by_key(|e| e.episode_number);
        // Dedup on episode_number — sometimes AniList lists the same
        // episode under multiple streaming sites (one entry per site).
        // Keep the first (which is the lowest-indexed after sort).
        out.dedup_by_key(|e| e.episode_number);
        Ok(out)
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
        // enrichment.
        //
        // Three things going on in this retry loop:
        //
        // 1. **Retry budget** — up to MAX_ATTEMPTS attempts on 429.
        //    The first version of this code only retried once; in
        //    practice the rate-limit window is often longer than
        //    one retry's wait, so we'd give up after two 429s and
        //    the scanner cascade fired.
        //
        // 2. **Retry-After floor** — AniList sometimes returns
        //    `Retry-After: 0` (or the header is absent and we
        //    default to a smaller wait). Honoring 0 means an
        //    immediate retry that just hits the rate limit again.
        //    We floor at MIN_RETRY_AFTER_S so even a zero header
        //    sleeps long enough for the limit window to advance.
        //
        // 3. **Exponential backoff on retry** — each subsequent
        //    attempt waits at least 2× the previous floor. Caps at
        //    MAX_RETRY_AFTER_S so a misbehaving server can't park
        //    us indefinitely.
        const MAX_ATTEMPTS: usize = 3;
        const MIN_RETRY_AFTER_S: u64 = 5;
        const MAX_RETRY_AFTER_S: u64 = 120;
        let mut backoff_floor = MIN_RETRY_AFTER_S;
        for attempt in 0..MAX_ATTEMPTS {
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
            if status.as_u16() == 429 && attempt + 1 < MAX_ATTEMPTS {
                let header_wait = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0);
                let wait_s = header_wait.max(backoff_floor).min(MAX_RETRY_AFTER_S);
                warn!(
                    wait_s,
                    header_wait,
                    attempt = attempt + 1,
                    "AniList rate-limited (429); sleeping then retrying"
                );
                tokio::time::sleep(std::time::Duration::from_secs(wait_s)).await;
                // Double the floor for the next attempt so a server
                // that keeps returning 0 still backs off geometrically.
                backoff_floor = (backoff_floor * 2).min(MAX_RETRY_AFTER_S);
                continue;
            }
            return Self::parse_anilist_response(resp, status).await;
        }
        anyhow::bail!("AniList POST kept 429-ing after {MAX_ATTEMPTS} attempts")
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
        let parsed: GraphQlResponse<T> =
            crate::http::bounded_json(resp, crate::http::DEFAULT_METADATA_BYTES, "AniList POST")
                .await
                .context("parse AniList JSON")?;
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

/// Per-episode projection returned by [`AniListClient::fetch_episodes`].
///
/// AniList doesn't expose a structured "episode title" field; this
/// struct is the result of parsing the free-form `streamingEpisodes`
/// title strings into something usable. When AniList only published an
/// episode number with no human-readable title (e.g. "Episode 1" with
/// no separator), `title` is the episode-number string itself — callers
/// should compare against the parsed `episode_number` and decide
/// whether the title is informative enough to surface.
#[derive(Debug, Clone)]
pub struct AniListEpisode {
    pub episode_number: i32,
    pub title: String,
    pub thumbnail_url: Option<String>,
}

impl AniListEpisode {
    /// True when the parsed title is more than just the episode number.
    /// Useful for callers that want to skip the enrichment when the
    /// extracted "title" wouldn't improve over filename-derived text.
    pub fn has_descriptive_title(&self) -> bool {
        let lower = self.title.to_ascii_lowercase();
        let trimmed = lower.trim();
        let stripped = trimmed
            .trim_start_matches("episode")
            .trim_start_matches("ep")
            .trim_start_matches('.')
            .trim_start_matches(' ')
            .trim_start_matches(|c: char| c.is_ascii_digit() || c == '0');
        !stripped.trim().is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct AniListShow {
    pub anilist_id: i64,
    pub mal_id: Option<i64>,
    /// Title in the order: english if present, then romaji, then native.
    /// Mirrors what most anime catalogue UIs do.
    pub title: String,
    /// English title as published by AniList, or `None` when the
    /// `english` field is empty. Exposed separately so language-aware
    /// callers can refuse to surface a Japanese fallback when the
    /// operator's `metadata_language` is en-*.
    pub english_title: Option<String>,
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
struct EpisodesWrapper {
    #[serde(rename = "Media")]
    media: Option<RawMediaWithEpisodes>,
}

#[derive(Debug, Deserialize)]
struct RawMediaWithEpisodes {
    #[serde(default, rename = "streamingEpisodes")]
    streaming_episodes: Vec<RawStreamingEpisode>,
}

#[derive(Debug, Deserialize)]
struct RawStreamingEpisode {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    thumbnail: Option<String>,
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
            english_title: english,
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

struct ParsedStreamingEpisode {
    episode_number: i32,
    title: String,
}

/// Parse a `streamingEpisodes.title` string into `(episode_number, title)`.
///
/// AniList's streaming-episode titles come from the upstream listings
/// they aggregate, so the format varies: typical examples include
/// `"Episode 1 - The Adventurers"`, `"Episode 1"`, `"S1 E1 - The Title"`,
/// `"1 - The Title"`. Returns `None` when no integer episode number can
/// be extracted (we'd have nothing to key the local episode row by).
///
/// When the title has a separator (` - ` or ` | `) the substring after
/// it is the descriptive title. When there's no separator (just
/// `"Episode 1"`), `title` is set to the episode-number string itself
/// and `AniListEpisode::has_descriptive_title()` will return false.
fn parse_streaming_episode_title(s: &str) -> Option<ParsedStreamingEpisode> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Find the first separator if any. AniList listings use either
    // ` - ` (most common) or ` | ` (occasionally).
    let (prefix, title_after) = if let Some(idx) = trimmed.find(" - ") {
        (&trimmed[..idx], Some(trimmed[idx + 3..].trim().to_string()))
    } else if let Some(idx) = trimmed.find(" | ") {
        (&trimmed[..idx], Some(trimmed[idx + 3..].trim().to_string()))
    } else {
        (trimmed, None)
    };

    // Extract the episode number from the prefix. We look for any run
    // of digits; the first one we encounter wins.
    //
    // Common prefix shapes:
    //   "Episode 1"
    //   "Episode 01"
    //   "EP 1"
    //   "E1"
    //   "S1 E1"   ← season-and-episode: take the second number
    //   "1"       ← bare number
    let mut digits = Vec::new();
    let mut current = String::new();
    for c in prefix.chars() {
        if c.is_ascii_digit() {
            current.push(c);
        } else if !current.is_empty() {
            digits.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        digits.push(current);
    }
    // S<n> E<m> shape → episode is the second number; otherwise the
    // first (and usually only) number.
    let raw = if prefix.to_ascii_lowercase().contains('s') && digits.len() >= 2 {
        &digits[1]
    } else {
        digits.first()?
    };
    let episode_number = raw.parse::<i32>().ok()?;
    if episode_number <= 0 {
        return None;
    }
    let title = title_after.unwrap_or_else(|| trimmed.to_string());
    Some(ParsedStreamingEpisode {
        episode_number,
        title,
    })
}

const EPISODES_QUERY: &str = r#"
query ($id: Int) {
  Media(id: $id, type: ANIME) {
    streamingEpisodes {
      title
      thumbnail
    }
  }
}
"#;

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

    #[test]
    fn parses_episode_with_dash_separator() {
        let p = parse_streaming_episode_title("Episode 1 - The Adventurers").unwrap();
        assert_eq!(p.episode_number, 1);
        assert_eq!(p.title, "The Adventurers");
    }

    #[test]
    fn parses_episode_with_pipe_separator() {
        let p = parse_streaming_episode_title("Episode 4 | The Land Where Souls Rest").unwrap();
        assert_eq!(p.episode_number, 4);
        assert_eq!(p.title, "The Land Where Souls Rest");
    }

    #[test]
    fn parses_zero_padded_episode_number() {
        let p = parse_streaming_episode_title("Episode 01 - First Steps").unwrap();
        assert_eq!(p.episode_number, 1);
        assert_eq!(p.title, "First Steps");
    }

    #[test]
    fn parses_season_episode_shape() {
        let p = parse_streaming_episode_title("S1 E12 - The Conclusion").unwrap();
        assert_eq!(p.episode_number, 12);
        assert_eq!(p.title, "The Conclusion");
    }

    #[test]
    fn no_separator_keeps_episode_number_as_title() {
        let p = parse_streaming_episode_title("Episode 7").unwrap();
        assert_eq!(p.episode_number, 7);
        assert_eq!(p.title, "Episode 7");
        // Caller should detect the lack of descriptive content:
        let ep = AniListEpisode {
            episode_number: p.episode_number,
            title: p.title.clone(),
            thumbnail_url: None,
        };
        assert!(!ep.has_descriptive_title());
    }

    #[test]
    fn bare_number_with_title() {
        let p = parse_streaming_episode_title("1 - Pilot").unwrap();
        assert_eq!(p.episode_number, 1);
        assert_eq!(p.title, "Pilot");
    }

    #[test]
    fn refuses_when_no_episode_number() {
        assert!(parse_streaming_episode_title("Preview").is_none());
        assert!(parse_streaming_episode_title("").is_none());
        assert!(parse_streaming_episode_title("   ").is_none());
    }

    #[test]
    fn refuses_zero_or_negative_episode_numbers() {
        // AniList occasionally lists "Episode 0" for previews; treat as
        // unparseable to keep main-feed enrichment uncluttered.
        assert!(parse_streaming_episode_title("Episode 0 - Preview").is_none());
    }

    #[test]
    fn descriptive_check_handles_episode_n_only() {
        let ep = AniListEpisode {
            episode_number: 3,
            title: "Episode 3".into(),
            thumbnail_url: None,
        };
        assert!(!ep.has_descriptive_title());
        let ep2 = AniListEpisode {
            episode_number: 3,
            title: "ep03".into(),
            thumbnail_url: None,
        };
        assert!(!ep2.has_descriptive_title());
        let ep3 = AniListEpisode {
            episode_number: 3,
            title: "The Reveal".into(),
            thumbnail_url: None,
        };
        assert!(ep3.has_descriptive_title());
    }
}

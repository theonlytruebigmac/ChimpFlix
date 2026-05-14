//! Free TVMaze v1 client. No API key, no account — TVMaze is a public
//! catalogue with rate limits (20 req/sec, lenient on bursts) that we
//! respect by only calling it during enrichment.
//!
//! Used as a TV-only **fallback** provider after TMDB. Two modes:
//!   1. TMDB returned no match → TVMaze tries to identify the show.
//!   2. TMDB matched but left fields blank → TVMaze fills the nulls,
//!      honoring the "fill nulls only" merge policy you picked.
//!
//! Why TVMaze specifically: free, no key, TV-first (better episode air
//! dates and network/status info than TMDB's TV catalogue), exposes
//! imdb/tvdb/tvrage cross-references for free, and supports HTML-tagged
//! plot summaries that we strip on ingest.

use anyhow::{Context, Result};
use chimpflix_common::USER_AGENT;
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue, USER_AGENT as UA_HEADER};
use serde::Deserialize;
use tracing::{debug, warn};

const TVMAZE_BASE_URL: &str = "https://api.tvmaze.com";

#[derive(Clone)]
pub struct TvMazeClient {
    http: reqwest::Client,
    base_url: String,
}

impl TvMazeClient {
    pub fn new() -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(UA_HEADER, HeaderValue::from_static(USER_AGENT));
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .context("build TVMaze http client")?;
        Ok(Self {
            http,
            base_url: TVMAZE_BASE_URL.to_string(),
        })
    }

    /// Single-best-match search. TVMaze's /singlesearch returns the top
    /// scoring hit directly, which is what we want for an automatic
    /// fallback — Fix Match doesn't currently surface TVMaze candidates.
    pub async fn lookup_show(&self, query: &str) -> Result<Option<TvMazeShow>> {
        let path = "/singlesearch/shows";
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .get(&url)
            .query(&[("q", query), ("embed", "externals")])
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        let status = resp.status();
        if status.as_u16() == 404 {
            debug!(query, "no TVMaze match");
            return Ok(None);
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!(%status, %url, body = %body.chars().take(200).collect::<String>(), "TVMaze error");
            anyhow::bail!("TVMaze {url} returned {status}");
        }
        let raw: RawShow = resp
            .json()
            .await
            .with_context(|| format!("parse TVMaze JSON from {url}"))?;
        Ok(Some(raw.into()))
    }
}

#[derive(Debug, Clone)]
pub struct TvMazeShow {
    pub tvmaze_id: i64,
    pub imdb_id: Option<String>,
    pub tvdb_id: Option<i64>,
    pub title: String,
    pub summary: Option<String>,
    pub year: Option<i32>,
    pub genres: Vec<String>,
    pub status: Option<String>,
    pub network: Option<String>,
    pub poster_url: Option<String>,
    pub backdrop_url: Option<String>,
}

// ---------------------------------------------------------------------------
// Wire types (only the fields we use)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RawShow {
    id: i64,
    name: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    premiered: Option<String>,
    #[serde(default)]
    genres: Vec<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    network: Option<RawNetwork>,
    #[serde(default, rename = "webChannel")]
    web_channel: Option<RawNetwork>,
    #[serde(default)]
    image: Option<RawImage>,
    #[serde(default)]
    externals: Option<RawExternals>,
    #[serde(default, rename = "_embedded")]
    embedded: Option<RawEmbedded>,
}

#[derive(Debug, Deserialize)]
struct RawNetwork {
    name: String,
}

#[derive(Debug, Deserialize)]
struct RawImage {
    #[serde(default)]
    medium: Option<String>,
    #[serde(default)]
    original: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawExternals {
    #[serde(default)]
    imdb: Option<String>,
    #[serde(default)]
    thetvdb: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct RawEmbedded {
    #[serde(default)]
    externals: Option<RawExternals>,
}

impl From<RawShow> for TvMazeShow {
    fn from(r: RawShow) -> Self {
        // `externals` can be at the top level (when ?embed=externals) or
        // nested under _embedded depending on TVMaze response shape; check
        // both.
        let externals = r.externals.or(r.embedded.and_then(|e| e.externals));
        Self {
            tvmaze_id: r.id,
            imdb_id: externals.as_ref().and_then(|x| x.imdb.clone()),
            tvdb_id: externals.and_then(|x| x.thetvdb),
            title: r.name,
            summary: r.summary.map(|s| strip_html(&s)).filter(|s| !s.is_empty()),
            year: r.premiered.as_deref().and_then(parse_year),
            genres: r.genres,
            status: r.status,
            network: r.network.or(r.web_channel).map(|n| n.name),
            poster_url: r
                .image
                .as_ref()
                .and_then(|i| i.original.clone().or_else(|| i.medium.clone())),
            backdrop_url: r.image.and_then(|i| i.original),
        }
    }
}

/// TVMaze summaries arrive as HTML (e.g. `<p>Plot…</p>`). Strip tags so
/// the text fits our plain-text summary field. Conservative — handles the
/// `<p>`, `<b>`, `<i>`, `<br>` cases TVMaze actually uses without pulling
/// in an HTML parser.
fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    // Collapse runs of whitespace (which the stripped tags often produce).
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_year(s: &str) -> Option<i32> {
    let year_str: String = s.chars().take(4).collect();
    year_str.parse().ok()
}

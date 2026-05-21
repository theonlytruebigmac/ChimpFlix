//! TheTVDB v4 client.
//!
//! Used as a backfill provider after TMDB — fills nulls (overview, year,
//! IMDb cross-ref, network, original title) for both shows and movies
//! without ever overwriting. Handles both kinds, unlike [`crate::tvmaze`]
//! which is shows-only.
//!
//! Auth flow: a single POST `/login` with `{apikey, pin?}` returns a JWT
//! valid ~30 days. We cache it in-process and refresh on the first 401.

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use chimpflix_common::USER_AGENT;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT as UA_HEADER};
use serde::Deserialize;
use tokio::sync::Mutex;
use tracing::{debug, warn};

const TVDB_BASE_URL: &str = "https://api4.thetvdb.com/v4";

#[derive(Clone)]
pub struct TvdbClient {
    http: reqwest::Client,
    base_url: String,
    apikey: String,
    pin: Option<String>,
    /// Cached bearer token. `None` means "not yet fetched" or "invalidated
    /// after a 401". Wrapped so clones of TvdbClient share the same token.
    token: Arc<Mutex<Option<String>>>,
}

impl TvdbClient {
    /// Build a client from a TVDB v4 API key. `pin` is the optional
    /// supporter PIN; pass `None` for free-tier keys.
    pub fn new(apikey: &str, pin: Option<&str>) -> Result<Self> {
        if apikey.trim().is_empty() {
            bail!("TVDB API key must not be empty");
        }
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(UA_HEADER, HeaderValue::from_static(USER_AGENT));
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .context("build TVDB http client")?;
        Ok(Self {
            http,
            base_url: TVDB_BASE_URL.to_string(),
            apikey: apikey.trim().to_string(),
            pin: pin.map(|p| p.trim().to_string()).filter(|p| !p.is_empty()),
            token: Arc::new(Mutex::new(None)),
        })
    }

    /// Hit the auth endpoint and confirm the key is accepted. Used by the
    /// admin credential vault "test" button.
    pub async fn validate(&self) -> Result<()> {
        // Force a fresh login so a stale cached token from a previous good
        // key doesn't mask an invalid value.
        {
            let mut guard = self.token.lock().await;
            *guard = None;
        }
        self.token().await?;
        Ok(())
    }

    pub async fn lookup_show(&self, query: &str, year: Option<i32>) -> Result<Option<TvdbShow>> {
        let hits: Vec<SearchHit> = self
            .search(query, "series", year)
            .await
            .with_context(|| format!("TVDB search series {query:?}"))?;
        let Some(hit) = hits.into_iter().next() else {
            debug!(query, year, "no TVDB series match");
            return Ok(None);
        };
        let id: i64 = hit.tvdb_id.parse().with_context(|| {
            format!("TVDB search returned non-numeric series id {:?}", hit.tvdb_id)
        })?;
        self.fetch_show(id).await.map(Some)
    }

    pub async fn lookup_movie(&self, query: &str, year: Option<i32>) -> Result<Option<TvdbMovie>> {
        let hits: Vec<SearchHit> = self
            .search(query, "movie", year)
            .await
            .with_context(|| format!("TVDB search movie {query:?}"))?;
        let Some(hit) = hits.into_iter().next() else {
            debug!(query, year, "no TVDB movie match");
            return Ok(None);
        };
        let id: i64 = hit.tvdb_id.parse().with_context(|| {
            format!("TVDB search returned non-numeric movie id {:?}", hit.tvdb_id)
        })?;
        self.fetch_movie(id).await.map(Some)
    }

    pub async fn fetch_show(&self, tvdb_id: i64) -> Result<TvdbShow> {
        let raw: Envelope<RawSeriesExtended> = self
            .get(&format!("/series/{tvdb_id}/extended"))
            .await?;
        Ok(TvdbShow::from_raw(raw.data))
    }

    pub async fn fetch_movie(&self, tvdb_id: i64) -> Result<TvdbMovie> {
        let raw: Envelope<RawMovieExtended> = self
            .get(&format!("/movies/{tvdb_id}/extended"))
            .await?;
        Ok(TvdbMovie::from_raw(raw.data))
    }

    async fn search(
        &self,
        query: &str,
        kind: &str,
        year: Option<i32>,
    ) -> Result<Vec<SearchHit>> {
        let mut params: Vec<(&str, String)> = vec![
            ("query", query.to_string()),
            ("type", kind.to_string()),
            ("limit", "10".to_string()),
        ];
        if let Some(y) = year {
            params.push(("year", y.to_string()));
        }
        let env: Envelope<Vec<SearchHit>> = self.get_with_query("/search", &params).await?;
        Ok(env.data)
    }

    /// Plain GET against a path; retries once after fetching a fresh token
    /// if the cached one was rejected.
    async fn get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T> {
        self.get_with_query(path, &[]).await
    }

    async fn get_with_query<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        params: &[(&str, String)],
    ) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        for attempt in 0..2 {
            let token = self.token().await?;
            let mut req = self
                .http
                .get(&url)
                .header(AUTHORIZATION, format!("Bearer {token}"));
            if !params.is_empty() {
                req = req.query(params);
            }
            let resp = req
                .send()
                .await
                .with_context(|| format!("GET {url}"))?;
            let status = resp.status();
            if status.as_u16() == 401 && attempt == 0 {
                // Stale or rejected token. Drop it and retry once.
                let mut guard = self.token.lock().await;
                *guard = None;
                continue;
            }
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                warn!(
                    %status, %url,
                    body = %body.chars().take(200).collect::<String>(),
                    "TVDB error"
                );
                bail!("TVDB {url} returned {status}");
            }
            return resp
                .json::<T>()
                .await
                .with_context(|| format!("parse TVDB JSON from {url}"));
        }
        bail!("TVDB {url} kept returning 401 after token refresh")
    }

    async fn token(&self) -> Result<String> {
        // Hold the lock across `login()` so concurrent callers queue
        // up behind us instead of each racing to POST /login. The
        // previous "check, drop, login, re-take" pattern let two
        // parallel scans each trigger a fresh login on cold-start,
        // wasting credentials and risking 429s from TVDB. Holding
        // the lock costs the second caller a single round-trip's
        // worth of wait time — acceptable since they'd have waited
        // anyway behind their own login.
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
        let mut body = serde_json::Map::new();
        body.insert("apikey".into(), serde_json::Value::String(self.apikey.clone()));
        if let Some(pin) = &self.pin {
            body.insert("pin".into(), serde_json::Value::String(pin.clone()));
        }
        let resp = self
            .http
            .post(&url)
            .json(&serde_json::Value::Object(body))
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!(
                "TVDB login returned {status}: {}",
                body.chars().take(200).collect::<String>()
            );
        }
        let env: Envelope<LoginData> = resp
            .json()
            .await
            .context("parse TVDB login response")?;
        Ok(env.data.token)
    }
}

// ---------------------------------------------------------------------------
// Public projections
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TvdbShow {
    pub tvdb_id: i64,
    pub imdb_id: Option<String>,
    pub title: String,
    pub original_title: Option<String>,
    pub summary: Option<String>,
    pub year: Option<i32>,
    pub status: Option<String>,
    pub network: Option<String>,
    pub poster_url: Option<String>,
    pub backdrop_url: Option<String>,
    pub genres: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TvdbMovie {
    pub tvdb_id: i64,
    pub imdb_id: Option<String>,
    pub title: String,
    pub original_title: Option<String>,
    pub summary: Option<String>,
    pub year: Option<i32>,
    pub runtime_minutes: Option<i32>,
    pub poster_url: Option<String>,
    pub backdrop_url: Option<String>,
    pub genres: Vec<String>,
}

// ---------------------------------------------------------------------------
// Wire types (only fields we use)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    data: T,
}

#[derive(Debug, Deserialize)]
struct LoginData {
    token: String,
}

#[derive(Debug, Deserialize)]
struct SearchHit {
    #[serde(rename = "tvdb_id")]
    tvdb_id: String,
}

#[derive(Debug, Deserialize)]
struct RawSeriesExtended {
    id: i64,
    name: String,
    #[serde(default, rename = "originalLanguage")]
    _original_language: Option<String>,
    #[serde(default, rename = "originalNetwork")]
    original_network: Option<RawNamed>,
    #[serde(default)]
    overview: Option<String>,
    #[serde(default, rename = "firstAired")]
    first_aired: Option<String>,
    #[serde(default)]
    status: Option<RawStatus>,
    #[serde(default)]
    image: Option<String>,
    #[serde(default, rename = "remoteIds")]
    remote_ids: Vec<RemoteId>,
    #[serde(default)]
    aliases: Vec<RawAlias>,
    #[serde(default)]
    genres: Vec<RawNamed>,
    #[serde(default)]
    artworks: Vec<RawArtwork>,
}

#[derive(Debug, Deserialize)]
struct RawMovieExtended {
    id: i64,
    name: String,
    #[serde(default)]
    overview: Option<String>,
    #[serde(default)]
    year: Option<String>,
    #[serde(default)]
    runtime: Option<i32>,
    #[serde(default)]
    image: Option<String>,
    #[serde(default, rename = "remoteIds")]
    remote_ids: Vec<RemoteId>,
    #[serde(default)]
    aliases: Vec<RawAlias>,
    #[serde(default)]
    genres: Vec<RawNamed>,
    #[serde(default)]
    artworks: Vec<RawArtwork>,
}

#[derive(Debug, Deserialize)]
struct RawNamed {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawStatus {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RemoteId {
    #[serde(default)]
    id: Option<String>,
    /// 2 = IMDb in TVDB's remote-id taxonomy.
    #[serde(default, rename = "sourceName")]
    source_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawAlias {
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawArtwork {
    /// 3 = series banner, 2 = series poster, 1 = series fanart per TVDB.
    /// We pick by `type` and prefer the highest-resolution entry.
    #[serde(default, rename = "type")]
    kind: Option<i32>,
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    score: Option<i64>,
}

impl TvdbShow {
    fn from_raw(r: RawSeriesExtended) -> Self {
        Self {
            tvdb_id: r.id,
            imdb_id: imdb_from_remote_ids(&r.remote_ids),
            original_title: pick_original_alias(&r.aliases, &r.name),
            title: r.name,
            summary: r.overview.filter(|s| !s.is_empty()),
            year: r.first_aired.as_deref().and_then(parse_year),
            status: r.status.and_then(|s| s.name),
            network: r.original_network.and_then(|n| n.name),
            poster_url: pick_artwork(&r.artworks, 2).or(r.image),
            backdrop_url: pick_artwork(&r.artworks, 3).or(pick_artwork(&r.artworks, 1)),
            genres: r.genres.into_iter().filter_map(|g| g.name).collect(),
        }
    }
}

impl TvdbMovie {
    fn from_raw(r: RawMovieExtended) -> Self {
        Self {
            tvdb_id: r.id,
            imdb_id: imdb_from_remote_ids(&r.remote_ids),
            original_title: pick_original_alias(&r.aliases, &r.name),
            title: r.name,
            summary: r.overview.filter(|s| !s.is_empty()),
            year: r.year.as_deref().and_then(parse_year),
            runtime_minutes: r.runtime,
            poster_url: pick_artwork(&r.artworks, 14).or(r.image.clone()),
            backdrop_url: pick_artwork(&r.artworks, 15).or(r.image),
            genres: r.genres.into_iter().filter_map(|g| g.name).collect(),
        }
    }
}

fn imdb_from_remote_ids(ids: &[RemoteId]) -> Option<String> {
    ids.iter()
        .find(|r| r.source_name.as_deref() == Some("IMDB"))
        .and_then(|r| r.id.clone())
        .filter(|s| s.starts_with("tt"))
}

/// TVDB exposes alternate titles as `aliases`. If the localized `name` we
/// took as the title differs from a non-English alias, we use the alias as
/// `original_title` so the UI can show both. Returns `None` if no useful
/// alias exists or it just duplicates the title.
fn pick_original_alias(aliases: &[RawAlias], title: &str) -> Option<String> {
    // Prefer the first non-English alias whose text differs from the title.
    aliases
        .iter()
        .find(|a| {
            a.language.as_deref().is_some_and(|l| l != "eng")
                && a.name.as_deref().is_some_and(|n| n != title && !n.is_empty())
        })
        .and_then(|a| a.name.clone())
}

fn pick_artwork(artworks: &[RawArtwork], kind: i32) -> Option<String> {
    artworks
        .iter()
        .filter(|a| a.kind == Some(kind) && a.image.is_some())
        .max_by_key(|a| a.score.unwrap_or(0))
        .and_then(|a| a.image.clone())
}

fn parse_year(s: &str) -> Option<i32> {
    s.chars().take(4).collect::<String>().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_rejects_empty_key() {
        assert!(TvdbClient::new("", None).is_err());
        assert!(TvdbClient::new("   ", None).is_err());
    }

    #[test]
    fn imdb_extracted_from_remote_ids() {
        let ids = vec![
            RemoteId {
                id: Some("12345".into()),
                source_name: Some("TheMovieDB".into()),
            },
            RemoteId {
                id: Some("tt0944947".into()),
                source_name: Some("IMDB".into()),
            },
        ];
        assert_eq!(imdb_from_remote_ids(&ids), Some("tt0944947".into()));
    }

    #[test]
    fn alias_picker_skips_english_and_duplicates() {
        let aliases = vec![
            RawAlias {
                language: Some("eng".into()),
                name: Some("Game of Thrones".into()),
            },
            RawAlias {
                language: Some("jpn".into()),
                name: Some("Game of Thrones".into()),
            },
            RawAlias {
                language: Some("jpn".into()),
                name: Some("ゲーム・オブ・スローンズ".into()),
            },
        ];
        assert_eq!(
            pick_original_alias(&aliases, "Game of Thrones"),
            Some("ゲーム・オブ・スローンズ".into())
        );
    }

    #[test]
    fn artwork_picker_takes_highest_score_of_kind() {
        let arts = vec![
            RawArtwork {
                kind: Some(2),
                image: Some("low.jpg".into()),
                score: Some(10),
            },
            RawArtwork {
                kind: Some(2),
                image: Some("high.jpg".into()),
                score: Some(99),
            },
            RawArtwork {
                kind: Some(3),
                image: Some("other.jpg".into()),
                score: Some(999),
            },
        ];
        assert_eq!(pick_artwork(&arts, 2), Some("high.jpg".into()));
    }

    #[test]
    fn year_parser_takes_first_four_digits() {
        assert_eq!(parse_year("2011-04-17"), Some(2011));
        assert_eq!(parse_year("1999"), Some(1999));
        assert_eq!(parse_year("garbage"), None);
    }
}

//! Minimal TMDB v3 client using v4 bearer token authentication.
//!
//! We hit the API directly with `reqwest` — no SDK crate. The surface is
//! tiny: lookup a movie/show by name (+ optional year) and fetch
//! season details for shows.

use anyhow::{Context, Result};
use chimpflix_common::USER_AGENT;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT as UA_HEADER};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

const TMDB_BASE_URL: &str = "https://api.themoviedb.org/3";
const TMDB_IMAGE_BASE: &str = "https://image.tmdb.org/t/p";

#[derive(Clone)]
pub struct TmdbClient {
    http: reqwest::Client,
    base_url: String,
}

impl TmdbClient {
    /// Build a TmdbClient from the `TMDB_READ_TOKEN` environment variable.
    /// Returns `Ok(None)` if the variable is unset/empty so callers can
    /// skip metadata enrichment gracefully.
    pub fn from_env() -> Result<Option<Self>> {
        match std::env::var("TMDB_READ_TOKEN") {
            Ok(token) if !token.trim().is_empty() => Ok(Some(Self::new(token.trim())?)),
            _ => Ok(None),
        }
    }

    pub fn new(token: &str) -> Result<Self> {
        let mut headers = HeaderMap::new();
        let auth = format!("Bearer {token}");
        let mut auth_value =
            HeaderValue::from_str(&auth).context("TMDB_READ_TOKEN is not valid ASCII")?;
        auth_value.set_sensitive(true);
        headers.insert(AUTHORIZATION, auth_value);
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(UA_HEADER, HeaderValue::from_static(USER_AGENT));

        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .context("build TMDB http client")?;

        Ok(Self {
            http,
            base_url: TMDB_BASE_URL.to_string(),
        })
    }

    pub async fn lookup_movie(&self, query: &str, year: Option<i32>) -> Result<Option<TmdbMovie>> {
        let mut params: Vec<(&str, String)> = vec![
            ("query", query.to_string()),
            ("language", "en-US".to_string()),
        ];
        if let Some(y) = year {
            params.push(("year", y.to_string()));
        }

        let raw: SearchPage<RawMovieHit> = self.get("/search/movie", &params).await?;
        let Some(hit) = raw.results.into_iter().next() else {
            debug!(query, year, "no TMDB movie match");
            return Ok(None);
        };

        let detail: RawMovieDetail = self
            .get(
                &format!("/movie/{}", hit.id),
                &[
                    ("language", "en-US".to_string()),
                    ("append_to_response", "external_ids".to_string()),
                ],
            )
            .await?;
        Ok(Some(TmdbMovie::from_raw(detail)))
    }

    pub async fn lookup_show(&self, query: &str, year: Option<i32>) -> Result<Option<TmdbShow>> {
        let mut params: Vec<(&str, String)> = vec![
            ("query", query.to_string()),
            ("language", "en-US".to_string()),
        ];
        if let Some(y) = year {
            params.push(("first_air_date_year", y.to_string()));
        }

        let raw: SearchPage<RawShowHit> = self.get("/search/tv", &params).await?;
        let Some(hit) = raw.results.into_iter().next() else {
            debug!(query, year, "no TMDB show match");
            return Ok(None);
        };

        let detail: RawShowDetail = self
            .get(
                &format!("/tv/{}", hit.id),
                &[
                    ("language", "en-US".to_string()),
                    ("append_to_response", "external_ids".to_string()),
                ],
            )
            .await?;
        Ok(Some(TmdbShow::from_raw(detail)))
    }

    pub async fn fetch_season(&self, show_id: i64, season_number: i32) -> Result<TmdbSeason> {
        let raw: RawSeason = self
            .get(
                &format!("/tv/{show_id}/season/{season_number}"),
                &[("language", "en-US".to_string())],
            )
            .await?;
        Ok(TmdbSeason {
            season_number: raw.season_number,
            episodes: raw
                .episodes
                .into_iter()
                .map(|e| TmdbEpisode {
                    episode_number: e.episode_number,
                    title: e.name,
                    summary: nonempty(e.overview),
                    runtime_min: e.runtime,
                    still_path: nonempty(e.still_path),
                    air_date: nonempty(e.air_date),
                })
                .collect(),
        })
    }

    async fn get<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        params: &[(&str, String)],
    ) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .get(&url)
            .query(params)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!(%status, %url, body = %body.chars().take(200).collect::<String>(), "TMDB error");
            anyhow::bail!("TMDB {url} returned {status}");
        }
        resp.json::<T>()
            .await
            .with_context(|| format!("parse TMDB JSON from {url}"))
    }
}

/// Build a TMDB image URL. Common sizes: `w92`, `w154`, `w185`, `w300`,
/// `w500`, `w780`, `original`.
pub fn tmdb_image_url(path: &str, size: &str) -> String {
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    format!("{TMDB_IMAGE_BASE}/{size}{path}")
}

// ---------------------------------------------------------------------------
// Public domain types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct TmdbMovie {
    pub tmdb_id: i64,
    pub imdb_id: Option<String>,
    pub title: String,
    pub original_title: Option<String>,
    pub summary: Option<String>,
    pub tagline: Option<String>,
    pub year: Option<i32>,
    pub runtime_min: Option<i32>,
    pub rating_audience: Option<f64>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub genres: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TmdbShow {
    pub tmdb_id: i64,
    pub imdb_id: Option<String>,
    pub title: String,
    pub original_title: Option<String>,
    pub summary: Option<String>,
    pub year: Option<i32>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub genres: Vec<String>,
    pub number_of_seasons: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TmdbSeason {
    pub season_number: i32,
    pub episodes: Vec<TmdbEpisode>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TmdbEpisode {
    pub episode_number: i32,
    pub title: String,
    pub summary: Option<String>,
    pub runtime_min: Option<i32>,
    pub still_path: Option<String>,
    pub air_date: Option<String>,
}

// ---------------------------------------------------------------------------
// Wire types (only the fields we use)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct SearchPage<T> {
    #[serde(default = "Vec::new")]
    results: Vec<T>,
}

#[derive(Debug, Deserialize)]
struct RawMovieHit {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct RawShowHit {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct RawMovieDetail {
    id: i64,
    title: String,
    original_title: Option<String>,
    overview: Option<String>,
    tagline: Option<String>,
    release_date: Option<String>,
    runtime: Option<i32>,
    vote_average: Option<f64>,
    poster_path: Option<String>,
    backdrop_path: Option<String>,
    imdb_id: Option<String>,
    #[serde(default)]
    genres: Vec<RawGenre>,
    external_ids: Option<RawExternalIds>,
}

#[derive(Debug, Deserialize)]
struct RawShowDetail {
    id: i64,
    name: String,
    original_name: Option<String>,
    overview: Option<String>,
    first_air_date: Option<String>,
    poster_path: Option<String>,
    backdrop_path: Option<String>,
    number_of_seasons: Option<i32>,
    #[serde(default)]
    genres: Vec<RawGenre>,
    external_ids: Option<RawExternalIds>,
}

#[derive(Debug, Deserialize)]
struct RawSeason {
    season_number: i32,
    #[serde(default)]
    episodes: Vec<RawEpisode>,
}

#[derive(Debug, Deserialize)]
struct RawEpisode {
    episode_number: i32,
    name: String,
    overview: Option<String>,
    runtime: Option<i32>,
    still_path: Option<String>,
    air_date: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawGenre {
    name: String,
}

#[derive(Debug, Deserialize)]
struct RawExternalIds {
    imdb_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

impl TmdbMovie {
    fn from_raw(raw: RawMovieDetail) -> Self {
        let year = raw.release_date.as_deref().and_then(parse_year);
        let imdb_id = raw
            .imdb_id
            .clone()
            .or_else(|| raw.external_ids.and_then(|e| e.imdb_id));
        Self {
            tmdb_id: raw.id,
            imdb_id: nonempty(imdb_id),
            title: raw.title,
            original_title: nonempty(raw.original_title),
            summary: nonempty(raw.overview),
            tagline: nonempty(raw.tagline),
            year,
            runtime_min: raw.runtime,
            rating_audience: raw.vote_average,
            poster_path: nonempty(raw.poster_path),
            backdrop_path: nonempty(raw.backdrop_path),
            genres: raw.genres.into_iter().map(|g| g.name).collect(),
        }
    }
}

impl TmdbShow {
    fn from_raw(raw: RawShowDetail) -> Self {
        let year = raw.first_air_date.as_deref().and_then(parse_year);
        let imdb_id = raw.external_ids.and_then(|e| e.imdb_id);
        Self {
            tmdb_id: raw.id,
            imdb_id: nonempty(imdb_id),
            title: raw.name,
            original_title: nonempty(raw.original_name),
            summary: nonempty(raw.overview),
            year,
            poster_path: nonempty(raw.poster_path),
            backdrop_path: nonempty(raw.backdrop_path),
            number_of_seasons: raw.number_of_seasons,
            genres: raw.genres.into_iter().map(|g| g.name).collect(),
        }
    }
}

fn parse_year(s: &str) -> Option<i32> {
    let year_str: String = s.chars().take(4).collect();
    year_str.parse().ok()
}

fn nonempty(s: Option<String>) -> Option<String> {
    s.filter(|v| !v.is_empty())
}

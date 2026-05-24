//! OMDb (Open Movie Database) client — pulls IMDb scores, Rotten
//! Tomatoes critic score, Metacritic, and MPAA rating for a movie
//! or show identified by its IMDb id.
//!
//! Free tier: 1,000 requests/day with no per-second cap. Paid tiers
//! lift the daily ceiling. The handler that consumes this client
//! (`fetch_external_ratings`) implements per-item backoff on 429 so
//! a brief quota breach doesn't poison the entire sweep.
//!
//! Auth: API key as a query parameter. The key lives in the
//! credential vault under the `omdb` slot — set via `chimpflix
//! creds set omdb <key>` or the admin credentials page.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use chimpflix_common::USER_AGENT;
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue, USER_AGENT as UA_HEADER};
use serde::Deserialize;
use tracing::warn;

const OMDB_BASE_URL: &str = "https://www.omdbapi.com/";

#[derive(Clone)]
pub struct OmdbClient {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
    /// Latching circuit breaker. OMDb's free tier returns `401
    /// Unauthorized` once you cross the daily request cap; the body is
    /// indistinguishable from a bad-key 401. Without a breaker the
    /// rest of a library scan keeps blasting the same dead key for
    /// every show × every episode and floods the log with thousands
    /// of `omdb http 401` lines. Once any call observes a 401 this
    /// flag is set and every subsequent call short-circuits to
    /// `Ok(None)` — chain dispatch treats it as a clean "OMDb has
    /// nothing for this title" and moves on to the next agent
    /// without further HTTP. The flag clears on the next client
    /// construction (admin saves a new key → state.set_omdb gets a
    /// fresh `OmdbClient`).
    auth_failed: Arc<AtomicBool>,
}

/// Normalized output. We strip OMDb's text formatting ("8.4/10",
/// "92%", "60") into typed scalars so the player UI doesn't have
/// to parse strings.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct OmdbRatings {
    /// IMDb out of 10 (e.g. 8.4).
    pub imdb_rating: Option<f32>,
    /// Number of IMDb votes when reported.
    pub imdb_votes: Option<i64>,
    /// Rotten Tomatoes critic score out of 100 (e.g. 92).
    pub rotten_tomatoes: Option<u8>,
    /// Metacritic Metascore out of 100.
    pub metacritic: Option<u8>,
    /// MPAA / TV rating ("PG-13", "R", "TV-MA", …). OMDb returns "N/A"
    /// when missing — we normalize to None in that case.
    pub mpaa: Option<String>,
}

/// Full OMDb title payload normalized into the fields the chain
/// dispatch cares about. Used for movies, shows, and specific
/// episodes — OMDb returns the same shape for all three.
#[derive(Debug, Clone, Default)]
pub struct OmdbTitle {
    pub imdb_id: Option<String>,
    pub title: Option<String>,
    pub year: Option<i32>,
    pub summary: Option<String>,
    pub runtime_minutes: Option<i32>,
    pub genres: Vec<String>,
    pub mpaa: Option<String>,
    pub poster_url: Option<String>,
    pub metascore: Option<i32>,
    pub director: Option<String>,
    pub writer: Option<String>,
    /// Top-billed actors as a comma-joined string ("Tom Hanks, Robin
    /// Wright, Gary Sinise"). Split + dedupe in the apply layer when
    /// populating `item_credits`.
    pub actors: Option<String>,
}

impl OmdbClient {
    pub fn new(api_key: impl Into<String>) -> Result<Self> {
        let api_key = api_key.into();
        anyhow::ensure!(!api_key.trim().is_empty(), "omdb api key is empty");
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(UA_HEADER, HeaderValue::from_static(USER_AGENT));
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .context("build OMDb http client")?;
        Ok(Self {
            http,
            api_key,
            base_url: OMDB_BASE_URL.to_string(),
            auth_failed: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Returns true once the circuit breaker has tripped (any prior
    /// call saw a 401 Unauthorized). When true every public method
    /// returns `Ok(None)` without touching the network.
    pub fn is_disabled(&self) -> bool {
        self.auth_failed.load(Ordering::Relaxed)
    }

    /// Trip the breaker. Idempotent. Logs once per process when it
    /// flips from open → closed so operators see the explanation
    /// instead of a wall of 401 lines.
    fn trip_breaker(&self) {
        if self
            .auth_failed
            .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            warn!(
                "OMDb returned 401 Unauthorized — circuit breaker tripped; suppressing further OMDb requests until the client is rebuilt (admin → credentials → OMDb)"
            );
        }
    }

    /// Lookup a movie by title (with optional year disambiguation).
    /// Returns the full OMDb payload mapped into the shared `OmdbTitle`
    /// shape so the chain dispatch can treat it uniformly with the
    /// other agents.
    pub async fn lookup_movie(
        &self,
        title: &str,
        year: Option<i32>,
    ) -> Result<Option<OmdbTitle>> {
        if self.is_disabled() {
            return Ok(None);
        }
        self.lookup(title, year, Some("movie")).await
    }

    /// Lookup a TV series by title.
    pub async fn lookup_show(&self, title: &str, year: Option<i32>) -> Result<Option<OmdbTitle>> {
        if self.is_disabled() {
            return Ok(None);
        }
        self.lookup(title, year, Some("series")).await
    }

    /// Fetch a specific episode of a show by its IMDb id and season /
    /// episode numbers. Returns the same shape as `lookup_*` — fields
    /// are populated to what OMDb knows about that episode (title,
    /// summary, runtime, poster).
    pub async fn fetch_episode(
        &self,
        imdb_id: &str,
        season: i32,
        episode: i32,
    ) -> Result<Option<OmdbTitle>> {
        if self.is_disabled() {
            return Ok(None);
        }
        let resp = self
            .http
            .get(&self.base_url)
            .query(&[
                ("apikey", self.api_key.as_str()),
                ("i", imdb_id),
                ("Season", &season.to_string()),
                ("Episode", &episode.to_string()),
            ])
            .send()
            .await
            .context("omdb http send")?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.trip_breaker();
            return Ok(None);
        }
        if !resp.status().is_success() {
            anyhow::bail!("omdb http {}", resp.status());
        }
        let raw: RawResponse = crate::http::bounded_json(
            resp,
            crate::http::DEFAULT_METADATA_BYTES,
            "OMDb GET",
        )
        .await
        .context("omdb body decode")?;
        if raw.Response.as_deref() == Some("False") {
            return Ok(None);
        }
        Ok(Some(raw.into_title()))
    }

    async fn lookup(
        &self,
        title: &str,
        year: Option<i32>,
        omdb_type: Option<&str>,
    ) -> Result<Option<OmdbTitle>> {
        let year_str = year.map(|y| y.to_string());
        let mut params: Vec<(&str, &str)> = vec![
            ("apikey", self.api_key.as_str()),
            ("t", title),
            ("plot", "full"),
        ];
        if let Some(y) = year_str.as_deref() {
            params.push(("y", y));
        }
        if let Some(t) = omdb_type {
            params.push(("type", t));
        }
        let resp = self
            .http
            .get(&self.base_url)
            .query(&params)
            .send()
            .await
            .context("omdb http send")?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.trip_breaker();
            return Ok(None);
        }
        if !resp.status().is_success() {
            anyhow::bail!("omdb http {}", resp.status());
        }
        let raw: RawResponse = crate::http::bounded_json(
            resp,
            crate::http::DEFAULT_METADATA_BYTES,
            "OMDb GET",
        )
        .await
        .context("omdb body decode")?;
        if raw.Response.as_deref() == Some("False") {
            // Title not in OMDb — treat as a clean miss so the chain
            // moves to the next agent.
            return Ok(None);
        }
        Ok(Some(raw.into_title()))
    }

    /// Fetch ratings for the given IMDb id (e.g. "tt0816692").
    /// Returns `Ok(None)` when OMDb explicitly reports "not found";
    /// every other failure surfaces as an error so the worker pool's
    /// backoff curve can decide whether to retry.
    pub async fn fetch_ratings(&self, imdb_id: &str) -> Result<Option<OmdbRatings>> {
        if self.is_disabled() {
            return Ok(None);
        }
        let resp = self
            .http
            .get(&self.base_url)
            .query(&[
                ("apikey", self.api_key.as_str()),
                ("i", imdb_id),
                ("tomatoes", "true"),
            ])
            .send()
            .await
            .context("omdb http send")?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.trip_breaker();
            return Ok(None);
        }
        if !resp.status().is_success() {
            anyhow::bail!(
                "omdb http {}: {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            );
        }
        let raw: RawResponse = crate::http::bounded_json(
            resp,
            crate::http::DEFAULT_METADATA_BYTES,
            "OMDb GET",
        )
        .await
        .context("omdb body decode")?;
        if raw.Response.as_deref() == Some("False") {
            // OMDb returns these two error strings for both unknown
            // IMDb ids and unindexed-yet-released titles. Treat as a
            // "not found" (Ok(None)) so the per-item handler stamps
            // the watermark and doesn't keep retrying.
            let not_found = matches!(
                raw.Error.as_deref(),
                Some("Incorrect IMDb ID.") | Some("Movie not found!"),
            );
            if not_found {
                return Ok(None);
            }
            anyhow::bail!("omdb negative response: {}", raw.Error.unwrap_or_default());
        }
        Ok(Some(raw.into_normalized()))
    }
}

#[derive(Debug, Default, Deserialize)]
#[allow(non_snake_case)]
struct RawResponse {
    Response: Option<String>,
    Error: Option<String>,
    imdbID: Option<String>,
    Title: Option<String>,
    Year: Option<String>,
    Plot: Option<String>,
    Runtime: Option<String>,
    Genre: Option<String>,
    Poster: Option<String>,
    Director: Option<String>,
    Writer: Option<String>,
    Actors: Option<String>,
    imdbRating: Option<String>,
    imdbVotes: Option<String>,
    Metascore: Option<String>,
    Rated: Option<String>,
    Ratings: Option<Vec<RawRatingEntry>>,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct RawRatingEntry {
    Source: String,
    Value: String,
}

#[allow(non_snake_case)]
impl RawResponse {
    /// Translate the OMDb response into the chain's common title shape.
    fn into_title(self) -> OmdbTitle {
        // OMDb returns runtime as "142 min". Strip the unit suffix.
        let runtime_minutes = self.Runtime.as_deref().and_then(|r| {
            r.split_whitespace()
                .next()
                .and_then(|n| n.parse::<i32>().ok())
        });
        // "Year" can be "2014" or "2010–2015" for shows. Take the
        // first 4 digits.
        let year = self.Year.as_deref().and_then(|y| {
            let digits: String = y.chars().take(4).collect();
            digits.parse().ok()
        });
        // OMDb returns "N/A" for empty optional fields. Normalize to
        // None so the apply layer's COALESCE doesn't insert literal
        // "N/A" strings.
        fn na_filter(v: Option<String>) -> Option<String> {
            v.filter(|s| !s.is_empty() && s != "N/A")
        }
        let genres = self
            .Genre
            .as_deref()
            .map(|g| {
                g.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        OmdbTitle {
            imdb_id: na_filter(self.imdbID.clone()),
            title: na_filter(self.Title),
            year,
            summary: na_filter(self.Plot),
            runtime_minutes,
            genres,
            mpaa: na_filter(self.Rated.clone()),
            poster_url: na_filter(self.Poster),
            metascore: self
                .Metascore
                .as_deref()
                .filter(|s| *s != "N/A")
                .and_then(|s| s.parse().ok()),
            director: na_filter(self.Director),
            writer: na_filter(self.Writer),
            actors: na_filter(self.Actors),
        }
    }

    fn into_normalized(self) -> OmdbRatings {
        let mut out = OmdbRatings::default();

        if let Some(r) = self.imdbRating.as_deref() {
            if r != "N/A" {
                if let Ok(v) = r.parse::<f32>() {
                    out.imdb_rating = Some(v);
                }
            }
        }
        if let Some(v) = self.imdbVotes.as_deref() {
            if v != "N/A" {
                let cleaned: String = v.chars().filter(|c| c.is_ascii_digit()).collect();
                if let Ok(n) = cleaned.parse::<i64>() {
                    out.imdb_votes = Some(n);
                }
            }
        }
        if let Some(m) = self.Metascore.as_deref() {
            if m != "N/A" {
                if let Ok(n) = m.parse::<u8>() {
                    out.metacritic = Some(n);
                }
            }
        }
        if let Some(rated) = self.Rated {
            if !rated.is_empty() && rated != "N/A" {
                out.mpaa = Some(rated);
            }
        }
        // Rotten Tomatoes lives inside `Ratings` as "Source: 'Rotten
        // Tomatoes', Value: '92%'". Strip the percent sign + parse.
        if let Some(ratings) = self.Ratings {
            for r in ratings {
                if r.Source == "Rotten Tomatoes" {
                    let cleaned = r.Value.trim_end_matches('%');
                    if let Ok(n) = cleaned.parse::<u8>() {
                        out.rotten_tomatoes = Some(n);
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(j: serde_json::Value) -> OmdbRatings {
        let raw: RawResponse = serde_json::from_value(j).unwrap();
        raw.into_normalized()
    }

    #[test]
    fn parses_imdb_rating_and_votes() {
        let r = parse(json!({
            "imdbRating": "8.4",
            "imdbVotes": "1,234,567",
        }));
        assert_eq!(r.imdb_rating, Some(8.4));
        assert_eq!(r.imdb_votes, Some(1_234_567));
    }

    #[test]
    fn rotten_tomatoes_strips_percent() {
        let r = parse(json!({
            "Ratings": [
                { "Source": "Internet Movie Database", "Value": "8.4/10" },
                { "Source": "Rotten Tomatoes", "Value": "92%" },
                { "Source": "Metacritic", "Value": "74/100" }
            ]
        }));
        assert_eq!(r.rotten_tomatoes, Some(92));
    }

    #[test]
    fn metascore_parses() {
        let r = parse(json!({ "Metascore": "74" }));
        assert_eq!(r.metacritic, Some(74));
    }

    #[test]
    fn na_values_become_none() {
        let r = parse(json!({
            "imdbRating": "N/A",
            "Metascore": "N/A",
            "Rated": "N/A",
        }));
        assert!(r.imdb_rating.is_none());
        assert!(r.metacritic.is_none());
        assert!(r.mpaa.is_none());
    }

    #[test]
    fn mpaa_propagates_when_set() {
        let r = parse(json!({ "Rated": "PG-13" }));
        assert_eq!(r.mpaa.as_deref(), Some("PG-13"));
    }

    #[test]
    fn empty_body_yields_default() {
        let r = parse(json!({}));
        assert!(r.imdb_rating.is_none());
        assert!(r.rotten_tomatoes.is_none());
        assert!(r.metacritic.is_none());
        assert!(r.mpaa.is_none());
        assert!(r.imdb_votes.is_none());
    }
}

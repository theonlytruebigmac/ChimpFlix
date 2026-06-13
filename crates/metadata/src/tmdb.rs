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

/// Returned (wrapped in `anyhow::Error`) when TMDB responds with a 2xx
/// status but a body we can't deserialize — empty body, Cloudflare
/// challenge HTML, malformed JSON, etc. Callers downcast to map this
/// to a 502 "TMDB unavailable" instead of a generic 500.
#[derive(Debug, thiserror::Error)]
#[error("TMDB upstream returned unparseable body from {url}: {snippet}")]
pub struct TmdbUpstreamError {
    pub url: String,
    pub snippet: String,
}

/// Returned when TMDB responds with HTTP 429 Too Many Requests. Carries
/// the `Retry-After` value parsed from the response header (or a
/// reasonable default when the header is absent) so the retry loop in
/// `get` can honour it.
#[derive(Debug, thiserror::Error)]
#[error("TMDB rate-limited (429) at {url}; retry after {retry_after_s}s")]
struct TmdbRateLimitError {
    url: String,
    retry_after_s: u64,
}

#[derive(Clone)]
pub struct TmdbClient {
    http: reqwest::Client,
    base_url: String,
    /// BCP-47 language tag sent on every TMDB request (overview,
    /// tagline, certain titles, image filtering). Defaults to `en-US`.
    /// Set via `metadata_language` server setting at startup —
    /// changes require a server restart since this is a process-wide
    /// singleton.
    language: String,
}

impl TmdbClient {
    /// Build a TmdbClient from the `TMDB_READ_TOKEN` environment variable.
    /// Returns `Ok(None)` if the variable is unset/empty so callers can
    /// skip metadata enrichment gracefully. Uses `en-US` for metadata —
    /// callers needing a different language should construct via
    /// [`Self::with_language`] after vault retrieval.
    pub fn from_env() -> Result<Option<Self>> {
        match std::env::var("TMDB_READ_TOKEN") {
            Ok(token) if !token.trim().is_empty() => Ok(Some(Self::new(token.trim())?)),
            _ => Ok(None),
        }
    }

    /// Construct with the default `en-US` language. Used by the admin
    /// credential-test path (where short-lived validation against any
    /// endpoint suffices) and any consumer that doesn't have the
    /// operator's preferred language available.
    pub fn new(token: &str) -> Result<Self> {
        Self::with_language(token, "en-US")
    }

    /// Construct with an operator-chosen BCP-47 language. Sent on every
    /// localised endpoint (`/movie/{id}`, `/tv/{id}`, search, season
    /// detail, etc.). Invalid tags get original-language fallbacks
    /// from TMDB silently — no error.
    pub fn with_language(token: &str, language: &str) -> Result<Self> {
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
            language: language.to_string(),
        })
    }

    /// The active language tag — exposed so callers can decide whether
    /// to rebuild the client after a settings change.
    pub fn language(&self) -> &str {
        &self.language
    }

    /// Value for the `include_image_language` query param. We want the
    /// base language code (`en` from `en-US`, `ja` from `ja-JP`) plus
    /// `null` so language-less art is always included as a fallback —
    /// many releases ship a no-language logo that we want as the
    /// secondary candidate when the localised one is missing.
    fn image_lang_filter(&self) -> String {
        let base = self.language.split('-').next().unwrap_or("en");
        format!("{base},null")
    }

    /// Hit a tiny endpoint to confirm the API key is accepted. Used by the
    /// admin credential vault "test" button. Returns the TMDB image
    /// configuration's base URL on success purely so the caller can render
    /// a friendly "connected" message.
    pub async fn validate(&self) -> Result<String> {
        let raw: serde_json::Value = self.get("/configuration", &[]).await?;
        let base = raw
            .get("images")
            .and_then(|i| i.get("secure_base_url"))
            .and_then(|v| v.as_str())
            .unwrap_or(TMDB_IMAGE_BASE)
            .to_string();
        Ok(base)
    }

    pub async fn lookup_movie(&self, query: &str, year: Option<i32>) -> Result<Option<TmdbMovie>> {
        let mut params: Vec<(&str, String)> = vec![
            ("query", query.to_string()),
            ("language", self.language.clone()),
        ];
        if let Some(y) = year {
            params.push(("year", y.to_string()));
        }

        let raw: SearchPage<RawMovieHit> = self.get("/search/movie", &params).await?;
        let Some(hit) = raw.results.into_iter().next() else {
            debug!(query, year, "no TMDB movie match");
            return Ok(None);
        };
        self.fetch_movie(hit.id).await.map(Some)
    }

    /// Fetch a movie by TMDB id — used both internally after a search and
    /// by Fix Match's "apply candidate" path. `images` is appended so a
    /// single round-trip yields the title-treatment logo art for the
    /// modal hero alongside the rest of the detail payload.
    pub async fn fetch_movie(&self, tmdb_id: i64) -> Result<TmdbMovie> {
        let detail: RawMovieDetail = self
            .get(
                &format!("/movie/{tmdb_id}"),
                &[
                    ("language", self.language.clone()),
                    ("append_to_response", "external_ids,images".to_string()),
                    // Restrict the images response to English (or the
                    // language-less art that many releases ship); avoids
                    // pulling logos in 30+ languages for a payload we
                    // only need a single pick from.
                    ("include_image_language", self.image_lang_filter()),
                ],
            )
            .await?;
        Ok(TmdbMovie::from_raw(detail))
    }

    /// Fetch only the title-treatment logo path for a movie. Used by
    /// the `refresh_logos` backfill task, where we already have
    /// everything else and just need the logo column populated.
    pub async fn fetch_movie_logo(&self, tmdb_id: i64) -> Result<Option<String>> {
        let raw: RawImagesResponse = self
            .get(
                &format!("/movie/{tmdb_id}/images"),
                &[("include_image_language", self.image_lang_filter())],
            )
            .await?;
        Ok(pick_logo(&raw.logos))
    }

    /// Weekly trending movies, ranked. TMDB returns up to 20 per page;
    /// we take the first page since the Top 10 rail only needs 10.
    pub async fn trending_movies(&self) -> Result<Vec<TmdbTrendingEntry>> {
        let raw: SearchPage<RawTrendingEntry> = self
            .get(
                "/trending/movie/week",
                &[("language", self.language.clone())],
            )
            .await?;
        Ok(raw
            .results
            .into_iter()
            .map(TmdbTrendingEntry::from_raw)
            .collect())
    }

    /// Weekly trending TV shows. Same shape as `trending_movies` — the
    /// endpoint returns mixed-shape entries but we hit the typed
    /// `/trending/tv/week` route so the result is homogeneous.
    pub async fn trending_shows(&self) -> Result<Vec<TmdbTrendingEntry>> {
        let raw: SearchPage<RawTrendingEntry> = self
            .get("/trending/tv/week", &[("language", self.language.clone())])
            .await?;
        Ok(raw
            .results
            .into_iter()
            .map(TmdbTrendingEntry::from_raw)
            .collect())
    }

    /// All-time top-rated movies, ranked. Drives the per-library "Top 10"
    /// rail (distinct from the home page's *trending* rail). `/movie/top_rated`
    /// returns the same entry shape as trending; first page (20) is plenty
    /// to overlap a Top 10 after the per-library intersection.
    pub async fn top_rated_movies(&self) -> Result<Vec<TmdbTrendingEntry>> {
        let raw: SearchPage<RawTrendingEntry> = self
            .get("/movie/top_rated", &[("language", self.language.clone())])
            .await?;
        Ok(raw
            .results
            .into_iter()
            .map(TmdbTrendingEntry::from_raw)
            .collect())
    }

    /// All-time top-rated TV shows. Mirror of `top_rated_movies`.
    pub async fn top_rated_shows(&self) -> Result<Vec<TmdbTrendingEntry>> {
        let raw: SearchPage<RawTrendingEntry> = self
            .get("/tv/top_rated", &[("language", self.language.clone())])
            .await?;
        Ok(raw
            .results
            .into_iter()
            .map(TmdbTrendingEntry::from_raw)
            .collect())
    }

    pub async fn lookup_show(&self, query: &str, year: Option<i32>) -> Result<Option<TmdbShow>> {
        let mut params: Vec<(&str, String)> = vec![
            ("query", query.to_string()),
            ("language", self.language.clone()),
        ];
        if let Some(y) = year {
            params.push(("first_air_date_year", y.to_string()));
        }

        let raw: SearchPage<RawShowHit> = self.get("/search/tv", &params).await?;
        let Some(hit) = raw.results.into_iter().next() else {
            debug!(query, year, "no TMDB show match");
            return Ok(None);
        };
        self.fetch_show(hit.id).await.map(Some)
    }

    pub async fn fetch_show(&self, tmdb_id: i64) -> Result<TmdbShow> {
        let detail: RawShowDetail = self
            .get(
                &format!("/tv/{tmdb_id}"),
                &[
                    ("language", self.language.clone()),
                    ("append_to_response", "external_ids,images".to_string()),
                    ("include_image_language", self.image_lang_filter()),
                ],
            )
            .await?;
        Ok(TmdbShow::from_raw(detail))
    }

    /// Title-treatment logo path for a TV show — twin of
    /// `fetch_movie_logo` but against `/tv/{id}/images`.
    pub async fn fetch_show_logo(&self, tmdb_id: i64) -> Result<Option<String>> {
        let raw: RawImagesResponse = self
            .get(
                &format!("/tv/{tmdb_id}/images"),
                &[("include_image_language", self.image_lang_filter())],
            )
            .await?;
        Ok(pick_logo(&raw.logos))
    }

    /// Multi-candidate search for Fix Match. Returns up to ~10 hits
    /// each with enough preview info (title, year, summary, poster) to
    /// render a Plex-style picker without a follow-up round-trip per
    /// candidate.
    pub async fn search_candidates(
        &self,
        kind: TmdbKind,
        query: &str,
        year: Option<i32>,
    ) -> Result<Vec<TmdbCandidate>> {
        let mut params: Vec<(&str, String)> = vec![
            ("query", query.to_string()),
            ("language", self.language.clone()),
        ];
        let path = match kind {
            TmdbKind::Movie => {
                if let Some(y) = year {
                    params.push(("year", y.to_string()));
                }
                "/search/movie"
            }
            TmdbKind::Show => {
                if let Some(y) = year {
                    params.push(("first_air_date_year", y.to_string()));
                }
                "/search/tv"
            }
        };
        let raw: SearchPage<RawCandidate> = self.get(path, &params).await?;
        Ok(raw
            .results
            .into_iter()
            .take(10)
            .map(|c| c.into_candidate(kind))
            .collect())
    }

    /// Cast + crew for the movie/show. Returns up to ~20 cast and the
    /// top-level crew roles (director, writer, producer) so the modal
    /// can render Plex-style "Cast & Crew" without splitting on the
    /// frontend.
    pub async fn fetch_credits(&self, kind: TmdbKind, tmdb_id: i64) -> Result<TmdbCredits> {
        let path = match kind {
            TmdbKind::Movie => format!("/movie/{tmdb_id}/credits"),
            TmdbKind::Show => format!("/tv/{tmdb_id}/credits"),
        };
        let raw: RawCredits = self
            .get(&path, &[("language", self.language.clone())])
            .await?;
        let cast: Vec<TmdbCastMember> = raw
            .cast
            .into_iter()
            .take(20)
            .map(|c| TmdbCastMember {
                tmdb_person_id: c.id,
                name: c.name,
                character: nonempty(c.character),
                profile_path: nonempty(c.profile_path),
                order: c.order.unwrap_or(0),
            })
            .collect();
        let crew: Vec<TmdbCrewMember> = raw
            .crew
            .into_iter()
            .filter(|c| {
                matches!(
                    c.job.as_str(),
                    "Director" | "Writer" | "Screenplay" | "Producer" | "Executive Producer"
                )
            })
            .map(|c| TmdbCrewMember {
                tmdb_person_id: c.id,
                name: c.name,
                job: c.job,
                department: c.department,
                profile_path: nonempty(c.profile_path),
            })
            .collect();
        Ok(TmdbCredits { cast, crew })
    }

    /// Full detail for a TMDB collection (franchise). Provides the
    /// overview field and the list of member movies. Called on first
    /// encounter of any movie that belongs to the collection.
    pub async fn fetch_collection(&self, tmdb_id: i64) -> Result<TmdbCollection> {
        let raw: RawCollectionDetail = self
            .get(
                &format!("/collection/{tmdb_id}"),
                &[("language", self.language.clone())],
            )
            .await?;
        Ok(TmdbCollection {
            tmdb_id: raw.id,
            name: raw.name,
            overview: nonempty(raw.overview),
            poster_path: nonempty(raw.poster_path),
            backdrop_path: nonempty(raw.backdrop_path),
            parts: raw
                .parts
                .into_iter()
                .map(|p| TmdbCollectionPart {
                    tmdb_id: p.id,
                    title: p.title.or(p.original_title).unwrap_or_default(),
                    year: p.release_date.as_deref().and_then(parse_year),
                })
                .collect(),
        })
    }

    /// Public reviews for the title. We pull only the first page (20
    /// reviews) since the modal's Reviews section caps display anyway.
    /// `rating` is a 1-10 scale on TMDB when the author chose to rate,
    /// or `None` when they only left a text review.
    pub async fn fetch_reviews(&self, kind: TmdbKind, tmdb_id: i64) -> Result<Vec<TmdbReview>> {
        let path = match kind {
            TmdbKind::Movie => format!("/movie/{tmdb_id}/reviews"),
            TmdbKind::Show => format!("/tv/{tmdb_id}/reviews"),
        };
        let raw: RawReviews = self
            .get(&path, &[("language", self.language.clone())])
            .await?;
        Ok(raw
            .results
            .into_iter()
            .map(|r| {
                let rating = r.author_details.as_ref().and_then(|d| d.rating);
                let avatar = r
                    .author_details
                    .as_ref()
                    .and_then(|d| d.avatar_path.clone())
                    .and_then(|p| tmdb_avatar_url(&p));
                TmdbReview {
                    source_id: r.id,
                    author: r.author,
                    author_url: nonempty(r.url),
                    avatar_url: avatar,
                    rating: rating.map(|f| f.round() as i32),
                    body: nonempty(r.content),
                    created_at: r.created_at.as_deref().and_then(parse_iso8601_ms),
                }
            })
            .collect())
    }

    /// All videos (trailers, teasers, featurettes, behind-the-scenes,
    /// clips) for the movie/show, filtered to YouTube since that's the
    /// only source we currently surface in the player.
    pub async fn fetch_videos(&self, kind: TmdbKind, tmdb_id: i64) -> Result<Vec<TmdbVideo>> {
        let path = match kind {
            TmdbKind::Movie => format!("/movie/{tmdb_id}/videos"),
            TmdbKind::Show => format!("/tv/{tmdb_id}/videos"),
        };
        let raw: RawVideos = self
            .get(&path, &[("language", self.language.clone())])
            .await?;
        Ok(raw
            .results
            .into_iter()
            .filter(|v| v.site.eq_ignore_ascii_case("YouTube"))
            .filter(|v| !v.key.trim().is_empty())
            .map(|v| TmdbVideo {
                key: v.key,
                name: v.name,
                kind: v.r#type,
                official: v.official,
                published_at: v.published_at,
            })
            .collect())
    }

    /// TMDB ids of titles similar to the given one. We pull the first page
    /// only — 20 candidates is plenty for the modal's rail. The caller is
    /// expected to intersect with the local library so we never surface
    /// titles the user doesn't have.
    pub async fn lookup_similar(&self, tmdb_id: i64, is_show: bool) -> Result<Vec<i64>> {
        let path = if is_show {
            format!("/tv/{tmdb_id}/similar")
        } else {
            format!("/movie/{tmdb_id}/similar")
        };
        let resp: SimilarResults = self
            .get(&path, &[("language", self.language.clone())])
            .await?;
        Ok(resp.results.into_iter().map(|r| r.id).collect())
    }

    /// Look up the first YouTube trailer for an item. Returns `None` if
    /// TMDB has no trailer or the entry has none on YouTube. Picks an
    /// official trailer when one exists, otherwise the first trailer-typed
    /// entry.
    pub async fn lookup_trailer(&self, tmdb_id: i64, is_show: bool) -> Result<Option<String>> {
        let path = if is_show {
            format!("/tv/{tmdb_id}/videos")
        } else {
            format!("/movie/{tmdb_id}/videos")
        };
        let resp: RawVideos = self
            .get(&path, &[("language", self.language.clone())])
            .await?;
        let mut trailers: Vec<&RawVideo> = resp
            .results
            .iter()
            .filter(|v| v.site.eq_ignore_ascii_case("YouTube"))
            .filter(|v| v.r#type.eq_ignore_ascii_case("Trailer"))
            .collect();
        // Prefer official trailers, then anything trailer-typed.
        trailers.sort_by_key(|v| if v.official { 0 } else { 1 });
        Ok(trailers.first().map(|v| v.key.clone()))
    }

    /// Poster candidates for the item. We don't restrict by language —
    /// the Plex parity here is that users want to *see* every poster TMDB
    /// has and pick one. TMDB returns posters in roughly preference order;
    /// we pass them through unchanged so the highest-vote ones land first.
    pub async fn fetch_posters(&self, kind: TmdbKind, tmdb_id: i64) -> Result<Vec<TmdbPoster>> {
        let path = match kind {
            TmdbKind::Movie => format!("/movie/{tmdb_id}/images"),
            TmdbKind::Show => format!("/tv/{tmdb_id}/images"),
        };
        // include_image_language=null gets the language-agnostic posters
        // (no embedded text) on top of the localized ones.
        let raw: RawImages = self
            .get(
                &path,
                &[("include_image_language", self.image_lang_filter())],
            )
            .await?;
        Ok(raw
            .posters
            .into_iter()
            .filter(|p| !p.file_path.trim().is_empty())
            .map(|p| TmdbPoster {
                thumb_url: tmdb_image_url(&p.file_path, "w342"),
                full_url: tmdb_image_url(&p.file_path, "original"),
                file_path: p.file_path,
                language: p.iso_639_1,
                width: p.width,
                height: p.height,
                vote_average: p.vote_average,
            })
            .collect())
    }

    pub async fn fetch_season(&self, show_id: i64, season_number: i32) -> Result<TmdbSeason> {
        let raw: RawSeason = self
            .get(
                &format!("/tv/{show_id}/season/{season_number}"),
                &[("language", self.language.clone())],
            )
            .await?;
        Ok(TmdbSeason {
            season_number: raw.season_number,
            episodes: raw
                .episodes
                .into_iter()
                .map(|e| TmdbEpisode {
                    tmdb_id: e.id,
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
        // Up to 3 attempts total:
        //   • 429 rate-limit: sleep Retry-After (or 60 s default) then
        //     retry. TMDB's rate-limit window is 1 s (40 req/10 s bucket);
        //     a short sleep recovers without hammering the API further.
        //   • Transient failures (network error, 5xx, empty 200): one
        //     short retry at 250 ms. TMDB's CDN occasionally returns empty
        //     200s under load.
        // Callers (scanner, Fix Match) can re-trigger so we don't loop
        // more than twice beyond the first attempt.
        const MAX_ATTEMPTS: usize = 3;
        // Min/max clamps mirror the AniList client so both respect the same
        // operator-visible behaviour.
        const MIN_RATE_LIMIT_WAIT_S: u64 = 5;
        const MAX_RATE_LIMIT_WAIT_S: u64 = 120;

        let mut last_err: Option<anyhow::Error> = None;
        for attempt in 0..MAX_ATTEMPTS {
            match self.get_once(&url, params).await {
                Ok(v) => return Ok(v),
                Err(e) if e.is::<TmdbRateLimitError>() && attempt + 1 < MAX_ATTEMPTS => {
                    let wait_s = e
                        .downcast_ref::<TmdbRateLimitError>()
                        .map(|r| r.retry_after_s)
                        .unwrap_or(60)
                        .max(MIN_RATE_LIMIT_WAIT_S)
                        .min(MAX_RATE_LIMIT_WAIT_S);
                    warn!(
                        %url,
                        wait_s,
                        attempt = attempt + 1,
                        "TMDB rate-limited (429); sleeping then retrying",
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(wait_s)).await;
                    last_err = Some(e);
                }
                Err(e) if is_retryable(&e) && attempt + 1 < MAX_ATTEMPTS => {
                    warn!(
                        %url,
                        error = %format!("{e:#}"),
                        "TMDB request failed, retrying once in 250ms",
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    last_err = Some(e);
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("TMDB GET {url} failed after {MAX_ATTEMPTS} attempts")))
    }

    async fn get_once<T: for<'de> Deserialize<'de>>(
        &self,
        url: &str,
        params: &[(&str, String)],
    ) -> Result<T> {
        let resp = self
            .http
            .get(url)
            .query(params)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;

        let status = resp.status();
        // Extract Retry-After before consuming the response body — the
        // header is only accessible on `resp` before `bounded_text` takes
        // ownership. Default to 60 s when the header is absent or
        // unparseable (TMDB sometimes omits it on soft-limit responses).
        let retry_after_s: u64 = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse().ok())
            .unwrap_or(60);

        // Buffer the body once — needed for both error logging and
        // (on success) JSON parsing. `resp.json()` consumes the body
        // and gives no visibility into what TMDB actually sent when
        // parsing fails, which is exactly the case we need to debug.
        //
        // `bounded_text` caps the read at 4 MiB and bails mid-stream
        // if exceeded — defends against a hostile or broken upstream
        // (see WEEK 1 #11 in `docs/PUBLIC_RELEASE_HARDENING.md`).
        let body = crate::http::bounded_text(
            resp,
            crate::http::DEFAULT_METADATA_BYTES,
            &format!("TMDB GET {url}"),
        )
        .await
        .with_context(|| format!("read TMDB body from {url}"))?;

        if status.as_u16() == 429 {
            // Surface as a typed error so the retry loop in `get` can
            // sleep for the right duration instead of treating this as a
            // hard failure.
            warn!(%url, retry_after_s, "TMDB rate-limited (429); will retry after backoff");
            return Err(anyhow::Error::new(TmdbRateLimitError {
                url: url.to_string(),
                retry_after_s,
            }));
        }
        if !status.is_success() {
            let snippet: String = body.chars().take(200).collect();
            warn!(%status, %url, body = %snippet, "TMDB error");
            anyhow::bail!("TMDB {url} returned {status}");
        }
        match serde_json::from_str::<T>(&body) {
            Ok(v) => Ok(v),
            Err(e) => {
                let snippet: String = body.chars().take(200).collect();
                warn!(
                    %url,
                    body_len = body.len(),
                    body = %snippet,
                    error = %e,
                    "TMDB returned 2xx but body is not the expected JSON shape",
                );
                Err(anyhow::Error::new(TmdbUpstreamError {
                    url: url.to_string(),
                    snippet,
                }))
            }
        }
    }
}

/// Network errors and unparseable upstream bodies are worth a short
/// retry. Non-2xx statuses other than 429 are NOT retried: 4xx (except
/// rate-limit) is the caller's fault (bad query, bad id) and repeat 5xx
/// in quick succession would amplify load when TMDB is already struggling.
/// HTTP 429 is handled separately in `get` before this predicate is tested.
fn is_retryable(err: &anyhow::Error) -> bool {
    if err.is::<TmdbUpstreamError>() {
        return true;
    }
    if let Some(re) = err.downcast_ref::<reqwest::Error>() {
        return re.is_timeout() || re.is_connect() || re.is_request();
    }
    false
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

/// Avatar URL helper. TMDB review authors with a TMDB-hosted avatar return
/// `avatar_path = "/abc.jpg"`, which we feed through the regular image
/// pipeline. Authors with a Gravatar return `avatar_path = "/https://..."` —
/// the leading slash is bogus and the URL is already absolute. Detect that
/// and return it only if the host is a known-safe origin; unknown absolute
/// URLs are dropped (return `None`) to prevent client-side tracking via
/// attacker-controlled image URLs.
fn tmdb_avatar_url(path: &str) -> Option<String> {
    let trimmed = path.trim_start_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        // Only allow absolute URLs from TMDB / Gravatar origins.
        let allowed = trimmed.starts_with("https://secure.gravatar.com/")
            || trimmed.starts_with("https://www.gravatar.com/")
            || trimmed.starts_with("https://gravatar.com/")
            || trimmed.starts_with("https://www.themoviedb.org/")
            || trimmed.starts_with("https://image.tmdb.org/");
        return if allowed {
            Some(trimmed.to_string())
        } else {
            None
        };
    }
    Some(tmdb_image_url(path, "w185"))
}

// ---------------------------------------------------------------------------
// Public domain types
// ---------------------------------------------------------------------------

/// A single entry in TMDB's trending list. Just enough info to cache it
/// for the Top 10 rail; the row's `tmdb_id` is what we'll later JOIN
/// against `items.tmdb_id` to find the in-library version.
#[derive(Debug, Clone, Serialize)]
pub struct TmdbTrendingEntry {
    pub tmdb_id: i64,
    pub title: String,
    pub poster_path: Option<String>,
}

impl TmdbTrendingEntry {
    fn from_raw(r: RawTrendingEntry) -> Self {
        Self {
            tmdb_id: r.id,
            title: r.title.or(r.name).unwrap_or_default(),
            poster_path: r.poster_path,
        }
    }
}

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
    /// Transparent title-treatment logo. TMDB-relative path; join with
    /// the image base URL on the client. None when no English logo is
    /// published or the request didn't include `images`.
    pub logo_path: Option<String>,
    pub genres: Vec<String>,
    /// TMDB collection (franchise) this movie belongs to, if any. Used to
    /// group sequels in the modal and on a dedicated /collection page.
    pub collection: Option<TmdbCollectionStub>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TmdbCollectionStub {
    pub tmdb_id: i64,
    pub name: String,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
}

/// Full collection detail: name, overview, and the parts (member movies).
/// We use this to backfill the overview which the `belongs_to_collection`
/// stub doesn't include.
#[derive(Debug, Clone, Serialize)]
pub struct TmdbCollection {
    pub tmdb_id: i64,
    pub name: String,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub parts: Vec<TmdbCollectionPart>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TmdbCollectionPart {
    pub tmdb_id: i64,
    pub title: String,
    pub year: Option<i32>,
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
    /// Title-treatment logo (transparent PNG). See TmdbMovie.logo_path.
    pub logo_path: Option<String>,
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
    /// TMDB's own episode id (distinct from `episode_number`). Used for
    /// stable cross-references; persisted in `episodes.tmdb_id`.
    pub tmdb_id: i64,
    pub episode_number: i32,
    pub title: String,
    pub summary: Option<String>,
    pub runtime_min: Option<i32>,
    pub still_path: Option<String>,
    pub air_date: Option<String>,
}

/// Whether an operation targets the /movie or /tv namespace. Used by the
/// shared search/credits/videos endpoints to avoid duplicating each method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TmdbKind {
    Movie,
    Show,
}

/// Compact preview of one TMDB hit for Fix Match's candidate picker.
#[derive(Debug, Clone, Serialize)]
pub struct TmdbCandidate {
    pub tmdb_id: i64,
    pub kind: &'static str, // "movie" | "show"
    pub title: String,
    pub year: Option<i32>,
    pub summary: Option<String>,
    pub poster_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TmdbCredits {
    pub cast: Vec<TmdbCastMember>,
    pub crew: Vec<TmdbCrewMember>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TmdbCastMember {
    pub tmdb_person_id: i64,
    pub name: String,
    pub character: Option<String>,
    pub profile_path: Option<String>,
    pub order: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct TmdbCrewMember {
    pub tmdb_person_id: i64,
    pub name: String,
    pub job: String,
    pub department: String,
    pub profile_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TmdbVideo {
    pub key: String, // YouTube video id
    pub name: String,
    pub kind: String, // "Trailer" | "Teaser" | "Featurette" | "Behind the Scenes" | "Clip"
    pub official: bool,
    pub published_at: Option<String>, // ISO 8601, e.g. "2024-03-15T12:00:00.000Z"
}

/// One poster candidate from the TMDB `/images` endpoint. `thumb_url` is
/// a w342 preview suitable for a grid; `full_url` is the `original` size
/// the server downloads when the user picks one.
#[derive(Debug, Clone, Serialize)]
pub struct TmdbPoster {
    pub file_path: String,
    pub thumb_url: String,
    pub full_url: String,
    pub language: Option<String>,
    pub width: i32,
    pub height: i32,
    pub vote_average: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TmdbReview {
    pub source_id: String,
    pub author: String,
    pub author_url: Option<String>,
    pub avatar_url: Option<String>,
    pub rating: Option<i32>,
    pub body: Option<String>,
    pub created_at: Option<i64>,
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
struct RawTrendingEntry {
    id: i64,
    /// Movies use `title`, shows use `name`; we accept either by
    /// declaring both fields optional and reading whichever is set.
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    poster_path: Option<String>,
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
    #[serde(default)]
    belongs_to_collection: Option<RawCollectionStub>,
    /// Populated only when `append_to_response=images` is set on the
    /// request. Absent for the search-result `RawMovieHit` shape.
    #[serde(default)]
    images: Option<RawImagesResponse>,
}

#[derive(Debug, Deserialize, Default)]
struct RawImagesResponse {
    #[serde(default)]
    logos: Vec<RawImage>,
}

#[derive(Debug, Deserialize, Clone)]
struct RawImage {
    file_path: String,
    /// Provider-reported aspect ratio. We bias slightly toward wider
    /// logos (title-treatment art is usually 2:1 to 4:1) when ranking.
    #[serde(default)]
    aspect_ratio: Option<f64>,
    /// Vote score from TMDB users. Higher = better-liked artwork.
    /// Combined with width to break ties.
    #[serde(default)]
    vote_average: Option<f64>,
    #[serde(default)]
    width: Option<i32>,
    /// Two-letter language code (or null for language-less art). We
    /// already filter the response via include_image_language, but
    /// keep the field for completeness.
    #[serde(default)]
    iso_639_1: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawCollectionStub {
    id: i64,
    name: String,
    #[serde(default)]
    poster_path: Option<String>,
    #[serde(default)]
    backdrop_path: Option<String>,
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
    #[serde(default)]
    images: Option<RawImagesResponse>,
}

#[derive(Debug, Deserialize)]
struct RawSeason {
    season_number: i32,
    #[serde(default)]
    episodes: Vec<RawEpisode>,
}

#[derive(Debug, Deserialize)]
struct RawEpisode {
    id: i64,
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
        let collection = raw.belongs_to_collection.map(|c| TmdbCollectionStub {
            tmdb_id: c.id,
            name: c.name,
            poster_path: nonempty(c.poster_path),
            backdrop_path: nonempty(c.backdrop_path),
        });
        let logo_path = raw.images.as_ref().and_then(|i| pick_logo(&i.logos));
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
            logo_path,
            genres: raw.genres.into_iter().map(|g| g.name).collect(),
            collection,
        }
    }
}

impl TmdbShow {
    fn from_raw(raw: RawShowDetail) -> Self {
        let year = raw.first_air_date.as_deref().and_then(parse_year);
        let imdb_id = raw.external_ids.and_then(|e| e.imdb_id);
        let logo_path = raw.images.as_ref().and_then(|i| pick_logo(&i.logos));
        Self {
            tmdb_id: raw.id,
            imdb_id: nonempty(imdb_id),
            title: raw.name,
            original_title: nonempty(raw.original_name),
            summary: nonempty(raw.overview),
            year,
            poster_path: nonempty(raw.poster_path),
            backdrop_path: nonempty(raw.backdrop_path),
            logo_path,
            number_of_seasons: raw.number_of_seasons,
            genres: raw.genres.into_iter().map(|g| g.name).collect(),
        }
    }
}

/// Pick the best logo from a list of TMDB image candidates. Prefers
/// English logos, then language-less (null), then ranks by vote
/// average × log(width) so a popular high-resolution logo wins over
/// a less-voted small one. Returns the TMDB-relative path or None
/// when the list is empty.
fn pick_logo(logos: &[RawImage]) -> Option<String> {
    if logos.is_empty() {
        return None;
    }
    let mut ranked: Vec<&RawImage> = logos.iter().collect();
    ranked.sort_by(|a, b| {
        let lang_rank = |img: &RawImage| match img.iso_639_1.as_deref() {
            Some("en") => 0,
            None | Some("") => 1,
            _ => 2,
        };
        let score = |img: &RawImage| -> f64 {
            let votes = img.vote_average.unwrap_or(0.0);
            let width_bonus = (img.width.unwrap_or(0).max(1) as f64).log10();
            // Slightly favor wider aspect ratios — title-treatment logos
            // typically run 2:1 to 5:1; very square images are usually
            // less polished single-letter marks.
            let ar_bonus = match img.aspect_ratio {
                Some(r) if (2.0..=6.0).contains(&r) => 0.5,
                _ => 0.0,
            };
            votes + width_bonus + ar_bonus
        };
        lang_rank(a).cmp(&lang_rank(b)).then_with(|| {
            score(b)
                .partial_cmp(&score(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    ranked.first().map(|i| i.file_path.clone())
}

#[derive(Debug, Deserialize)]
struct SimilarResults {
    #[serde(default)]
    results: Vec<SimilarHit>,
}

#[derive(Debug, Deserialize)]
struct SimilarHit {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct RawVideos {
    #[serde(default)]
    results: Vec<RawVideo>,
}

#[derive(Debug, Deserialize)]
struct RawImages {
    #[serde(default)]
    posters: Vec<RawPoster>,
}

#[derive(Debug, Deserialize)]
struct RawPoster {
    file_path: String,
    #[serde(default)]
    iso_639_1: Option<String>,
    #[serde(default)]
    width: i32,
    #[serde(default)]
    height: i32,
    #[serde(default)]
    vote_average: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct RawVideo {
    key: String,
    site: String,
    #[serde(rename = "type")]
    r#type: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    official: bool,
    #[serde(default)]
    published_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawCandidate {
    id: i64,
    // TMDB uses `title` for movies, `name` for shows. Both serdes default
    // to empty so the unused one for the active kind doesn't fail parsing.
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    overview: Option<String>,
    #[serde(default)]
    poster_path: Option<String>,
    #[serde(default)]
    release_date: Option<String>,
    #[serde(default)]
    first_air_date: Option<String>,
}

impl RawCandidate {
    fn into_candidate(self, kind: TmdbKind) -> TmdbCandidate {
        let (title, date) = match kind {
            TmdbKind::Movie => (self.title.unwrap_or_default(), self.release_date),
            TmdbKind::Show => (self.name.unwrap_or_default(), self.first_air_date),
        };
        TmdbCandidate {
            tmdb_id: self.id,
            kind: match kind {
                TmdbKind::Movie => "movie",
                TmdbKind::Show => "show",
            },
            title,
            year: date.as_deref().and_then(parse_year),
            summary: nonempty(self.overview),
            poster_path: nonempty(self.poster_path),
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawCredits {
    #[serde(default)]
    cast: Vec<RawCastMember>,
    #[serde(default)]
    crew: Vec<RawCrewMember>,
}

#[derive(Debug, Deserialize)]
struct RawCastMember {
    id: i64,
    name: String,
    #[serde(default)]
    character: Option<String>,
    #[serde(default)]
    profile_path: Option<String>,
    #[serde(default)]
    order: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct RawCrewMember {
    id: i64,
    name: String,
    job: String,
    department: String,
    #[serde(default)]
    profile_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawCollectionDetail {
    id: i64,
    name: String,
    #[serde(default)]
    overview: Option<String>,
    #[serde(default)]
    poster_path: Option<String>,
    #[serde(default)]
    backdrop_path: Option<String>,
    #[serde(default)]
    parts: Vec<RawCollectionPart>,
}

#[derive(Debug, Deserialize)]
struct RawCollectionPart {
    id: i64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    original_title: Option<String>,
    #[serde(default)]
    release_date: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawReviews {
    #[serde(default)]
    results: Vec<RawReview>,
}

#[derive(Debug, Deserialize)]
struct RawReview {
    id: String,
    author: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    author_details: Option<RawReviewAuthor>,
}

#[derive(Debug, Deserialize)]
struct RawReviewAuthor {
    #[serde(default)]
    avatar_path: Option<String>,
    #[serde(default)]
    rating: Option<f64>,
}

fn parse_year(s: &str) -> Option<i32> {
    let year_str: String = s.chars().take(4).collect();
    year_str.parse().ok()
}

fn nonempty(s: Option<String>) -> Option<String> {
    s.filter(|v| !v.is_empty())
}

/// Parse an ISO 8601 timestamp like `2024-03-15T12:00:00.000Z` to epoch ms.
/// Returns None on any malformed input so callers can just skip the field.
fn parse_iso8601_ms(s: &str) -> Option<i64> {
    let (date, rest) = s.split_once('T')?;
    let time = rest.split(['Z', '+', '.']).next()?;
    let mut date_parts = date.split('-');
    let y: i32 = date_parts.next()?.parse().ok()?;
    let m: u32 = date_parts.next()?.parse().ok()?;
    let d: u32 = date_parts.next()?.parse().ok()?;
    let mut time_parts = time.split(':');
    let hh: u32 = time_parts.next()?.parse().ok()?;
    let mm: u32 = time_parts.next()?.parse().ok()?;
    let ss: u32 = time_parts.next().unwrap_or("0").parse().ok()?;
    fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
        let y = if m <= 2 { y - 1 } else { y } as i64;
        let m = m as i64;
        let d = d as i64;
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146097 + doe - 719468
    }
    let days = days_from_civil(y, m, d);
    Some(days * 86_400_000 + hh as i64 * 3_600_000 + mm as i64 * 60_000 + ss as i64 * 1_000)
}

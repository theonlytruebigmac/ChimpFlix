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
    /// ISO 639-3 language code used in TVDB v4 episode-list path segment
    /// and search filter ("eng", "jpn", "spa", ...). Set via
    /// [`Self::with_language`]; defaults to "eng".
    language: String,
}

impl TvdbClient {
    /// Build a client from a TVDB v4 API key. `pin` is the optional
    /// supporter PIN; pass `None` for free-tier keys. Language defaults
    /// to "eng"; use [`Self::with_language`] to override.
    pub fn new(apikey: &str, pin: Option<&str>) -> Result<Self> {
        Self::with_language(apikey, pin, "en-US")
    }

    /// Build a client with an explicit BCP-47 metadata language. The
    /// tag is mapped to TVDB's ISO 639-3 code via [`bcp47_to_iso639_3`];
    /// unsupported tags fall back to "eng" so the client always points
    /// at a real TVDB language endpoint.
    pub fn with_language(apikey: &str, pin: Option<&str>, language_tag: &str) -> Result<Self> {
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
            language: bcp47_to_iso639_3(language_tag).to_string(),
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
            format!(
                "TVDB search returned non-numeric series id {:?}",
                hit.tvdb_id
            )
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
            format!(
                "TVDB search returned non-numeric movie id {:?}",
                hit.tvdb_id
            )
        })?;
        self.fetch_movie(id).await.map(Some)
    }

    pub async fn fetch_show(&self, tvdb_id: i64) -> Result<TvdbShow> {
        // `?meta=translations` pulls the `nameTranslations` +
        // `overviewTranslations` arrays inline so we can pick the
        // operator's preferred language instead of TVDB's primary-
        // language `name` field. Critical for Japanese-origin anime
        // where the primary `name` is the kanji title.
        let raw: Envelope<RawSeriesExtended> = self
            .get(&format!(
                "/series/{tvdb_id}/extended?meta=translations"
            ))
            .await?;
        Ok(TvdbShow::from_raw(raw.data, &self.language))
    }

    pub async fn fetch_movie(&self, tvdb_id: i64) -> Result<TvdbMovie> {
        let raw: Envelope<RawMovieExtended> = self
            .get(&format!(
                "/movies/{tvdb_id}/extended?meta=translations"
            ))
            .await?;
        Ok(TvdbMovie::from_raw(raw.data, &self.language))
    }

    /// Fetch the full episode list for a series in the "default"
    /// season-type order (i.e. how TVDB primarily groups episodes).
    /// The language path segment honors the operator's configured
    /// `metadata_language` server setting (mapped from BCP-47 to ISO
    /// 639-3 at client construction time); defaults to "eng" for
    /// instances that never set the preference.
    ///
    /// Returns episodes in the order TVDB lists them. Empty Vec when
    /// the series exists but has no episode rows (rare but seen on
    /// upcoming-only series).
    pub async fn fetch_episodes(&self, tvdb_id: i64) -> Result<Vec<TvdbEpisode>> {
        let lang = &self.language;
        let raw: Envelope<RawEpisodesPage> = self
            .get(&format!("/series/{tvdb_id}/episodes/default/{lang}"))
            .await?;
        Ok(raw
            .data
            .episodes
            .into_iter()
            .map(TvdbEpisode::from_raw)
            .collect())
    }

    /// Fetch extended data for a single episode — characters (cast +
    /// guest stars + crew), the long overview, and director credits.
    /// One extra HTTP call per episode; the agent only calls it from
    /// `fetch_episode` so the cost is bounded by how many episodes
    /// the scanner actually processes (not the full season list).
    pub async fn fetch_episode_extended(
        &self,
        episode_tvdb_id: i64,
    ) -> Result<TvdbEpisodeExtended> {
        let raw: Envelope<RawEpisodeExtended> = self
            .get(&format!("/episodes/{episode_tvdb_id}/extended"))
            .await?;
        Ok(TvdbEpisodeExtended::from_raw(raw.data))
    }

    async fn search(&self, query: &str, kind: &str, year: Option<i32>) -> Result<Vec<SearchHit>> {
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
            let resp = req.send().await.with_context(|| format!("GET {url}"))?;
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
        body.insert(
            "apikey".into(),
            serde_json::Value::String(self.apikey.clone()),
        );
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
        let env: Envelope<LoginData> = resp.json().await.context("parse TVDB login response")?;
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

#[derive(Debug, Clone)]
pub struct TvdbEpisode {
    pub tvdb_id: i64,
    pub season_number: i32,
    pub episode_number: i32,
    pub absolute_number: Option<i32>,
    pub title: String,
    pub summary: Option<String>,
    pub runtime_minutes: Option<i32>,
    /// First-aired date as TVDB returns it (`YYYY-MM-DD`). The library
    /// crate parses this into ms-epoch via `parse_air_date_to_ms`.
    pub air_date: Option<String>,
    /// Episode still image URL (already absolute — TVDB returns full
    /// URLs in v4's images, no path-resolution needed).
    pub still_url: Option<String>,
}

impl TvdbEpisode {
    fn from_raw(r: RawEpisode) -> Self {
        Self {
            tvdb_id: r.id,
            season_number: r.season_number.unwrap_or(0),
            episode_number: r.number.unwrap_or(0),
            absolute_number: r.absolute_number,
            title: r.name.unwrap_or_default(),
            summary: r.overview,
            runtime_minutes: r.runtime,
            air_date: r.aired,
            still_url: normalize_tvdb_image(r.image),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TvdbEpisodeExtended {
    pub tvdb_id: i64,
    pub characters: Vec<TvdbEpisodeCharacter>,
}

#[derive(Debug, Clone)]
pub struct TvdbEpisodeCharacter {
    /// TVDB person id (canonical person reference across appearances).
    pub person_id: i64,
    pub person_name: String,
    /// Character name — populated for actors / guest stars. None for
    /// crew (director, writer) where the "name" field is the job title.
    pub character_name: Option<String>,
    /// TVDB people-type: "Actor" | "Guest Star" | "Director" | "Writer"
    /// | "Crew" | ... Normalised at the agent boundary.
    pub people_type: String,
    pub profile_url: Option<String>,
    /// Lower number = earlier in the cast list.
    pub sort: i32,
}

impl TvdbEpisodeExtended {
    fn from_raw(r: RawEpisodeExtended) -> Self {
        let characters = r
            .characters
            .into_iter()
            .map(|c| TvdbEpisodeCharacter {
                person_id: c.person_id.unwrap_or(0),
                person_name: c.person_name.unwrap_or_default(),
                character_name: c.name,
                people_type: c.people_type.unwrap_or_default(),
                profile_url: c.image,
                sort: c.sort.unwrap_or(0),
            })
            .filter(|c| !c.person_name.is_empty())
            .collect();
        Self {
            tvdb_id: r.id,
            characters,
        }
    }
}

// ---------------------------------------------------------------------------
// Wire types (only fields we use)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    data: T,
}

#[derive(Debug, Deserialize)]
struct RawEpisodesPage {
    #[serde(default)]
    episodes: Vec<RawEpisode>,
}

#[derive(Debug, Deserialize)]
struct RawEpisode {
    id: i64,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    overview: Option<String>,
    #[serde(default, rename = "seasonNumber")]
    season_number: Option<i32>,
    #[serde(default)]
    number: Option<i32>,
    #[serde(default, rename = "absoluteNumber")]
    absolute_number: Option<i32>,
    #[serde(default)]
    runtime: Option<i32>,
    #[serde(default)]
    aired: Option<String>,
    /// TVDB v4 returns absolute image URLs (no `/banners/` prefix
    /// resolution needed).
    #[serde(default)]
    image: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawEpisodeExtended {
    id: i64,
    #[serde(default)]
    characters: Vec<RawCharacter>,
}

#[derive(Debug, Deserialize)]
struct RawCharacter {
    #[serde(default, rename = "personId")]
    person_id: Option<i64>,
    #[serde(default, rename = "personName")]
    person_name: Option<String>,
    /// TVDB's `name` on a character row is the *character name* (e.g.
    /// "Walter White") for actors, or the job title for crew. The
    /// agent normalises this at translation time.
    #[serde(default)]
    name: Option<String>,
    #[serde(default, rename = "peopleType")]
    people_type: Option<String>,
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    sort: Option<i32>,
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
    /// Populated only when the fetch URL includes `?meta=translations`.
    /// Used by [`TvdbShow::from_raw`] to surface the operator's
    /// configured language instead of the series' primary-language
    /// name (e.g. kanji for anime).
    #[serde(default)]
    translations: Option<RawTranslations>,
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
    #[serde(default)]
    translations: Option<RawTranslations>,
}

/// Inline translations block returned by TVDB v4 when the request URL
/// includes `?meta=translations`. Each entry is the localized name or
/// overview for one ISO 639-3 language tag.
#[derive(Debug, Deserialize, Default)]
struct RawTranslations {
    #[serde(default, rename = "nameTranslations")]
    name_translations: Vec<RawNameTranslation>,
    #[serde(default, rename = "overviewTranslations")]
    overview_translations: Vec<RawOverviewTranslation>,
}

#[derive(Debug, Deserialize)]
struct RawNameTranslation {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    language: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawOverviewTranslation {
    #[serde(default)]
    overview: Option<String>,
    #[serde(default)]
    language: Option<String>,
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
    fn from_raw(r: RawSeriesExtended, language: &str) -> Self {
        // Pick the translated title for the operator's configured
        // language, falling back to the primary-language `name`. This
        // is what stops kanji titles from leaking through for
        // English-locale operators with anime libraries.
        let title = pick_translated_name(&r.translations, language).unwrap_or(r.name.clone());
        let summary = pick_translated_overview(&r.translations, language)
            .or_else(|| r.overview.filter(|s| !s.is_empty()));
        Self {
            tvdb_id: r.id,
            imdb_id: imdb_from_remote_ids(&r.remote_ids),
            original_title: pick_original_alias(&r.aliases, &title),
            title,
            summary,
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
    fn from_raw(r: RawMovieExtended, language: &str) -> Self {
        let title = pick_translated_name(&r.translations, language).unwrap_or(r.name.clone());
        let summary = pick_translated_overview(&r.translations, language)
            .or_else(|| r.overview.filter(|s| !s.is_empty()));
        Self {
            tvdb_id: r.id,
            imdb_id: imdb_from_remote_ids(&r.remote_ids),
            original_title: pick_original_alias(&r.aliases, &title),
            title,
            summary,
            year: r.year.as_deref().and_then(parse_year),
            runtime_minutes: r.runtime,
            poster_url: pick_artwork(&r.artworks, 14).or(r.image.clone()),
            backdrop_url: pick_artwork(&r.artworks, 15).or(r.image),
            genres: r.genres.into_iter().filter_map(|g| g.name).collect(),
        }
    }
}

/// Pick the name translation matching `language` (ISO 639-3, e.g.
/// "eng") from a `?meta=translations` response. Returns `None` when no
/// translation block was returned or no matching entry exists.
fn pick_translated_name(translations: &Option<RawTranslations>, language: &str) -> Option<String> {
    translations.as_ref()?.name_translations.iter()
        .find(|t| t.language.as_deref() == Some(language))
        .and_then(|t| t.name.clone())
        .filter(|s| !s.is_empty())
}

fn pick_translated_overview(
    translations: &Option<RawTranslations>,
    language: &str,
) -> Option<String> {
    translations.as_ref()?.overview_translations.iter()
        .find(|t| t.language.as_deref() == Some(language))
        .and_then(|t| t.overview.clone())
        .filter(|s| !s.is_empty())
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
                && a.name
                    .as_deref()
                    .is_some_and(|n| n != title && !n.is_empty())
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

/// Normalize an image path returned by TVDB v4.
///
/// TVDB's behaviour around episode `image` fields is inconsistent in
/// practice:
///
///   * Episodes with proper stills return absolute CDN URLs
///     (`https://artworks.thetvdb.com/banners/episodes/.../...jpg`).
///   * Episodes without stills sometimes return `null`, sometimes an
///     empty string, sometimes whitespace.
///   * A small subset of older rows return a relative path starting
///     with `/banners/...` that still needs the CDN host prepended.
///
/// Without this normalization the empty-string case ended up in our
/// `images.source_url` column, which the UI rendered as `<img src="">`
/// — the browser interprets that as a self-reference to the current
/// page, returning HTML / 404 and showing a black tile (Plex-style
/// "no still" thumbnail). The user-visible symptom matched the bug
/// exactly.
fn normalize_tvdb_image(raw: Option<String>) -> Option<String> {
    let url = raw?.trim().to_string();
    if url.is_empty() {
        return None;
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        Some(url)
    } else if let Some(stripped) = url.strip_prefix('/') {
        Some(format!("https://artworks.thetvdb.com/{stripped}"))
    } else {
        Some(format!("https://artworks.thetvdb.com/{url}"))
    }
}

/// Translate a BCP-47 metadata language tag (e.g. "en-US", "ja-JP") into
/// TVDB's ISO 639-3 code (e.g. "eng", "jpn"). Only the language subtag
/// is consulted; the region is ignored. Unknown languages fall back to
/// "eng" so episode-list fetches always hit a real endpoint.
pub fn bcp47_to_iso639_3(tag: &str) -> &'static str {
    let primary = tag
        .split(|c: char| c == '-' || c == '_')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match primary.as_str() {
        "en" | "eng" => "eng",
        "ja" | "jpn" => "jpn",
        "ko" | "kor" => "kor",
        "zh" | "zho" | "chi" => "zho",
        "es" | "spa" => "spa",
        "fr" | "fra" | "fre" => "fra",
        "de" | "deu" | "ger" => "deu",
        "it" | "ita" => "ita",
        "pt" | "por" => "por",
        "ru" | "rus" => "rus",
        "nl" | "nld" | "dut" => "nld",
        "pl" | "pol" => "pol",
        "sv" | "swe" => "swe",
        "da" | "dan" => "dan",
        "fi" | "fin" => "fin",
        "no" | "nor" => "nor",
        "tr" | "tur" => "tur",
        "ar" | "ara" => "ara",
        "he" | "heb" => "heb",
        "th" | "tha" => "tha",
        "vi" | "vie" => "vie",
        "id" | "ind" => "ind",
        "hi" | "hin" => "hin",
        _ => "eng",
    }
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

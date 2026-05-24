//! Trakt.tv API v2 client (OAuth device flow + sync).
//!
//! Two scopes of operation:
//!   - **App-scoped** — `device_code`, `poll_device_token`,
//!     `refresh_token`. Uses only the registered client_id + secret;
//!     no per-user auth required.
//!   - **User-scoped** — `push_history`, `pull_history`,
//!     `pull_playback`, `push_rating`, `pull_ratings`. Each takes the
//!     user's access_token and forwards it as the Bearer header.
//!
//! Token refresh is the caller's job: every user-scoped call returns
//! the parsed Trakt response or an error; if a 401 comes back the
//! caller refreshes the user's token via `refresh_token()` and retries.
//! Keeping refresh logic out of the client avoids passing a SqlitePool
//! into the metadata crate.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use chimpflix_common::USER_AGENT;
use reqwest::header::{
    ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, USER_AGENT as UA_HEADER,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::warn;

const TRAKT_BASE_URL: &str = "https://api.trakt.tv";
const TRAKT_API_VERSION: &str = "2";

/// JSON blob the operator pastes into the `trakt` vault slot.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TraktCreds {
    pub client_id: String,
    pub client_secret: String,
}

impl TraktCreds {
    pub fn parse(raw: &str) -> Result<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            bail!("Trakt credential value is empty");
        }
        let creds: TraktCreds = serde_json::from_str(trimmed)
            .context("Trakt credentials must be JSON with client_id/client_secret")?;
        if creds.client_id.trim().is_empty() || creds.client_secret.trim().is_empty() {
            bail!("Trakt credentials must include client_id and client_secret");
        }
        Ok(creds)
    }
}

#[derive(Clone)]
pub struct TraktClient {
    http: reqwest::Client,
    base_url: String,
    client_id: String,
    client_secret: String,
}

impl TraktClient {
    pub fn from_creds(creds: TraktCreds) -> Result<Self> {
        Self::new(&creds.client_id, &creds.client_secret)
    }

    pub fn new(client_id: &str, client_secret: &str) -> Result<Self> {
        let client_id = client_id.trim();
        let client_secret = client_secret.trim();
        if client_id.is_empty() || client_secret.is_empty() {
            bail!("Trakt client_id and client_secret must both be non-empty");
        }
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(UA_HEADER, HeaderValue::from_static(USER_AGENT));
        headers.insert(
            "trakt-api-version",
            HeaderValue::from_static(TRAKT_API_VERSION),
        );
        let api_key_value =
            HeaderValue::from_str(client_id).context("Trakt client_id has non-ASCII characters")?;
        headers.insert("trakt-api-key", api_key_value);
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(15))
            .build()
            .context("build Trakt http client")?;
        Ok(Self {
            http,
            base_url: TRAKT_BASE_URL.to_string(),
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
        })
    }

    /// Validate the credentials by hitting the cheapest unauthenticated
    /// endpoint — `/oauth/device/code` returns immediately and tells us
    /// whether the client_id is recognised. We don't actually consume
    /// the resulting code (it expires unused).
    pub async fn validate(&self) -> Result<()> {
        let _ = self.device_code().await?;
        Ok(())
    }

    pub async fn device_code(&self) -> Result<DeviceCodeResponse> {
        let url = format!("{}/oauth/device/code", self.base_url);
        let body = json!({ "client_id": self.client_id });
        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            bail!(
                "Trakt /oauth/device/code returned {status}: {}",
                text.chars().take(200).collect::<String>()
            );
        }
        resp.json::<DeviceCodeResponse>()
            .await
            .context("parse Trakt device-code response")
    }

    /// Poll the device-token endpoint. Trakt returns a real token once
    /// the user has approved; until then it returns 400 (pending), 404
    /// (expired or wrong code), or 409 (already used). The caller is
    /// expected to drive the loop with the cadence Trakt suggested in
    /// `device_code`'s `interval` field.
    pub async fn poll_device_token(&self, device_code: &str) -> Result<DevicePollResult> {
        let url = format!("{}/oauth/device/token", self.base_url);
        let body = json!({
            "code": device_code,
            "client_id": self.client_id,
            "client_secret": self.client_secret,
        });
        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        let status = resp.status();
        if status.is_success() {
            let pair: TokenPair = resp.json().await.context("parse Trakt token response")?;
            return Ok(DevicePollResult::Ready(pair));
        }
        match status.as_u16() {
            400 => Ok(DevicePollResult::Pending),
            404 => Ok(DevicePollResult::Expired),
            409 => Ok(DevicePollResult::AlreadyApproved),
            410 => Ok(DevicePollResult::Expired),
            418 => Ok(DevicePollResult::Denied),
            429 => Ok(DevicePollResult::SlowDown),
            _ => {
                let text = resp.text().await.unwrap_or_default();
                bail!(
                    "Trakt /oauth/device/token returned {status}: {}",
                    text.chars().take(200).collect::<String>()
                );
            }
        }
    }

    pub async fn refresh_token(&self, refresh_token: &str) -> Result<TokenPair> {
        let url = format!("{}/oauth/token", self.base_url);
        let body = json!({
            "refresh_token": refresh_token,
            "client_id": self.client_id,
            "client_secret": self.client_secret,
            "grant_type": "refresh_token",
            "redirect_uri": "urn:ietf:wg:oauth:2.0:oob",
        });
        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            bail!(
                "Trakt /oauth/token (refresh) returned {status}: {}",
                text.chars().take(200).collect::<String>()
            );
        }
        resp.json::<TokenPair>()
            .await
            .context("parse Trakt refresh response")
    }

    pub async fn push_history(&self, access_token: &str, entries: &[HistoryPush]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let mut movies = Vec::new();
        let mut episodes = Vec::new();
        for e in entries {
            match e {
                HistoryPush::Movie { ids, watched_at } => movies.push(json!({
                    "watched_at": watched_at,
                    "ids": ids.to_json(),
                })),
                HistoryPush::Episode {
                    show_ids,
                    season,
                    episode,
                    watched_at,
                } => {
                    // Trakt's POST /sync/history wants the show object
                    // *itself* in the `shows` array — `ids` at the top
                    // level, `watched_at` down on the episode. The
                    // GET-style `{ "show": { "ids": ... } }` wrapper is
                    // a response shape; sending it on the POST lands
                    // every entry in `not_found.shows` (Trakt returns
                    // 201 but `added.episodes` is 0), which is exactly
                    // the "push fires but nothing appears on Trakt"
                    // symptom.
                    episodes.push(json!({
                        "ids": show_ids.to_json(),
                        "seasons": [{
                            "number": season,
                            "episodes": [{
                                "number": episode,
                                "watched_at": watched_at,
                            }],
                        }],
                    }));
                }
            }
        }
        let body = json!({ "movies": movies, "shows": episodes });
        self.user_post("/sync/history", access_token, &body).await?;
        Ok(())
    }

    /// Remove watched entries from a user's Trakt history. Mirror of
    /// [`push_history`] — same JSON shape, posted to `/sync/history/remove`.
    /// Used by the un-mark-watched code path so the local + Trakt
    /// states stay in lock-step (two-way sync).
    pub async fn remove_history(
        &self,
        access_token: &str,
        entries: &[HistoryPush],
    ) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let mut movies = Vec::new();
        let mut episodes = Vec::new();
        for e in entries {
            match e {
                HistoryPush::Movie { ids, .. } => movies.push(json!({
                    "ids": ids.to_json(),
                })),
                HistoryPush::Episode {
                    show_ids,
                    season,
                    episode,
                    ..
                } => {
                    episodes.push(json!({
                        "ids": show_ids.to_json(),
                        "seasons": [{
                            "number": season,
                            "episodes": [{ "number": episode }],
                        }],
                    }));
                }
            }
        }
        let body = json!({ "movies": movies, "shows": episodes });
        self.user_post("/sync/history/remove", access_token, &body)
            .await?;
        Ok(())
    }

    /// Live "now playing" scrobble. Fires the three lifecycle events
    /// (`/scrobble/start`, `/scrobble/pause`, `/scrobble/stop`) that
    /// drive Trakt's "YOU ARE WATCHING" banner. `/scrobble/stop` at
    /// progress ≥ 80% additionally writes a history entry server-side,
    /// so the explicit `/sync/history` push isn't strictly required for
    /// natural watch-through plays — only for explicit "Mark as watched"
    /// without a session.
    ///
    /// 409 responses from Trakt mean "you're already scrobbling this
    /// item" (start during a live session) or "you scrobbled too soon"
    /// (rate limit). Both are treated as success — there's nothing
    /// useful to do client-side except move on.
    pub async fn scrobble(
        &self,
        access_token: &str,
        action: ScrobbleAction,
        event: ScrobblePush,
    ) -> Result<()> {
        let body = match event {
            ScrobblePush::Movie { ids, progress } => json!({
                "movie": { "ids": ids.to_json() },
                "progress": progress.clamp(0.0, 100.0),
            }),
            ScrobblePush::Episode {
                show_ids,
                season,
                episode,
                progress,
            } => json!({
                "show": { "ids": show_ids.to_json() },
                "episode": { "season": season, "number": episode },
                "progress": progress.clamp(0.0, 100.0),
            }),
        };
        let url = format!("{}{}", self.base_url, action.path());
        let resp = self
            .http
            .post(&url)
            .header(AUTHORIZATION, format!("Bearer {access_token}"))
            .json(&body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        let status = resp.status();
        // 201/200 = ok. 409 = "already scrobbling" / "scrobbled too
        // recently"; both are fine — Trakt is just telling us the live
        // banner is already in the desired state.
        if status.is_success() || status.as_u16() == 409 {
            return Ok(());
        }
        Err(api_error(&format!("POST {}", action.path()), resp).await)
    }

    /// Add items to the user's Trakt watchlist. Trakt's watchlist
    /// shape is the same as `/sync/history` (movies + shows arrays
    /// keyed by ids at the top level) but without `watched_at`; the
    /// schema is `listRequestSchema` in the official repo, which is
    /// just `bulkMediaRequestSchema` with the `watched_at` field
    /// stripped semantically.
    pub async fn push_watchlist(
        &self,
        access_token: &str,
        entries: &[WatchlistPush],
    ) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let (movies, shows) = watchlist_buckets(entries);
        let body = json!({ "movies": movies, "shows": shows });
        self.user_post("/sync/watchlist", access_token, &body)
            .await?;
        Ok(())
    }

    /// Mirror of [`push_watchlist`] for removals — same body shape,
    /// posted to `/sync/watchlist/remove`.
    pub async fn remove_watchlist(
        &self,
        access_token: &str,
        entries: &[WatchlistPush],
    ) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let (movies, shows) = watchlist_buckets(entries);
        let body = json!({ "movies": movies, "shows": shows });
        self.user_post("/sync/watchlist/remove", access_token, &body)
            .await?;
        Ok(())
    }

    /// Pull `/users/me/stats` — lifetime watch totals across movies,
    /// shows, and episodes, plus ratings + network counts. Surfaced
    /// read-only in the Settings → Integrations page as a "you've
    /// watched X minutes" panel.
    pub async fn pull_user_stats(&self, access_token: &str) -> Result<UserStats> {
        let url = format!("{}/users/me/stats", self.base_url);
        let resp = self
            .http
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {access_token}"))
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            return Err(api_error("GET /users/me/stats", resp).await);
        }
        resp.json::<UserStats>()
            .await
            .context("parse Trakt user stats")
    }

    /// Pull personalized recommendations for `kind` ("movies" or
    /// "shows"). Trakt computes these from the user's watch + ratings
    /// history; the algorithm + freshness is server-side. Returns up
    /// to ~100 entries by default.
    pub async fn pull_recommendations(
        &self,
        access_token: &str,
        kind: RecommendationKind,
    ) -> Result<Vec<RecommendationEntry>> {
        let path = match kind {
            RecommendationKind::Movies => "/recommendations/movies",
            RecommendationKind::Shows => "/recommendations/shows",
        };
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {access_token}"))
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            return Err(api_error(&format!("GET {path}"), resp).await);
        }
        resp.json::<Vec<RecommendationEntry>>()
            .await
            .context("parse Trakt recommendations")
    }

    /// Pull the user's Trakt favorites — Trakt's "desert island"
    /// curated list, separate from the watchlist. Read-only in
    /// ChimpFlix (we don't have a local Favorites concept distinct
    /// from My List, so we just surface it as a one-way rail).
    pub async fn pull_favorites(&self, access_token: &str) -> Result<Vec<TraktListItem>> {
        let url = format!("{}/sync/favorites", self.base_url);
        let resp = self
            .http
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {access_token}"))
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            return Err(api_error("GET /sync/favorites", resp).await);
        }
        // Response envelope is the same as personal-list items + the
        // watchlist GET — flat array of `{ type, movie?, show? }`.
        // Reusing TraktListItem keeps the parse layer DRY.
        resp.json::<Vec<TraktListItem>>()
            .await
            .context("parse Trakt favorites")
    }

    /// Pull the user's personal Trakt lists (the ones THEY created
    /// — not lists they only liked). Used to surface each list as a
    /// dedicated rail on Home so a user's curated taxonomy is
    /// accessible in ChimpFlix without round-tripping to Trakt.
    pub async fn pull_my_lists(&self, access_token: &str) -> Result<Vec<TraktList>> {
        let url = format!("{}/users/me/lists", self.base_url);
        let resp = self
            .http
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {access_token}"))
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            return Err(api_error("GET /users/me/lists", resp).await);
        }
        resp.json::<Vec<TraktList>>()
            .await
            .context("parse Trakt lists")
    }

    /// Fetch items in one of the user's personal lists. `list_id` is
    /// either the numeric Trakt id or the slug. Returns one entry per
    /// item with the inline movie/show/episode/season object — we
    /// only consume movies + shows; the rest are dropped by the caller.
    pub async fn pull_my_list_items(
        &self,
        access_token: &str,
        list_id: &str,
    ) -> Result<Vec<TraktListItem>> {
        let url = format!(
            "{}/users/me/lists/{}/items",
            self.base_url,
            urlencode(list_id)
        );
        let resp = self
            .http
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {access_token}"))
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            return Err(api_error("GET /users/me/lists/{list_id}/items", resp).await);
        }
        resp.json::<Vec<TraktListItem>>()
            .await
            .context("parse Trakt list items")
    }

    /// Fetch the user's hidden-from-recommendations list. Used to
    /// filter the recommendations rail so dismissed items don't keep
    /// reappearing — and so a user who hid X on mobile sees the same
    /// suggestion behaviour everywhere. Trakt's recommendation
    /// algorithm already respects hides server-side, but the rail can
    /// surface freshly-hidden items in the brief window before the
    /// algo re-runs; the local filter belt-and-suspenders this.
    pub async fn pull_hidden_recommendations(
        &self,
        access_token: &str,
    ) -> Result<Vec<HiddenEntry>> {
        let url = format!("{}/users/hidden/recommendations?limit=200", self.base_url);
        let resp = self
            .http
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {access_token}"))
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            return Err(api_error("GET /users/hidden/recommendations", resp).await);
        }
        resp.json::<Vec<HiddenEntry>>()
            .await
            .context("parse Trakt hidden recommendations")
    }

    /// Hide a Trakt recommendation so the algorithm stops returning it.
    /// Used when the user dismisses a tile from the rail. `kind`
    /// matches [`pull_recommendations`].
    pub async fn hide_recommendation(
        &self,
        access_token: &str,
        kind: RecommendationKind,
        trakt_id: i64,
    ) -> Result<()> {
        let prefix = match kind {
            RecommendationKind::Movies => "movies",
            RecommendationKind::Shows => "shows",
        };
        let url = format!("{}/recommendations/{prefix}/{trakt_id}", self.base_url);
        let resp = self
            .http
            .delete(&url)
            .header(AUTHORIZATION, format!("Bearer {access_token}"))
            .send()
            .await
            .with_context(|| format!("DELETE {url}"))?;
        if !resp.status().is_success() {
            return Err(api_error(&format!("DELETE /recommendations/{prefix}"), resp).await);
        }
        Ok(())
    }

    /// Fetch upcoming episodes for shows the user has watched on Trakt.
    /// Three variants share the same response shape:
    ///   - `Shows`: every upcoming episode of every show the user tracks.
    ///   - `NewShows`: series premieres (E1 of a brand-new series).
    ///   - `SeasonPremieres`: S(N+1)E1 of any show the user tracks.
    ///
    /// The date window (`start_date` YYYY-MM-DD, `days` 1-31) gates all
    /// three. Trakt returns the entries ordered by `first_aired` asc.
    pub async fn pull_calendar_shows(
        &self,
        access_token: &str,
        kind: ShowCalendarKind,
        start_date: &str,
        days: u32,
    ) -> Result<Vec<CalendarEpisodeEntry>> {
        let days = days.clamp(1, 31);
        let suffix = match kind {
            ShowCalendarKind::Shows => "",
            ShowCalendarKind::NewShows => "/new",
            ShowCalendarKind::SeasonPremieres => "/premieres",
        };
        let url = format!(
            "{}/calendars/my/shows{suffix}/{start_date}/{days}",
            self.base_url
        );
        let resp = self
            .http
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {access_token}"))
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            return Err(api_error(&format!("GET /calendars/my/shows{suffix}"), resp).await);
        }
        resp.json::<Vec<CalendarEpisodeEntry>>()
            .await
            .context("parse Trakt calendar")
    }

    /// Movie release calendar — `/calendars/my/movies/{start}/{days}`.
    /// Different response shape than shows (no episode coords; each
    /// row has a `released` ISO date + movie object).
    pub async fn pull_calendar_movies(
        &self,
        access_token: &str,
        start_date: &str,
        days: u32,
    ) -> Result<Vec<CalendarMovieEntry>> {
        let days = days.clamp(1, 31);
        let url = format!("{}/calendars/my/movies/{start_date}/{days}", self.base_url);
        let resp = self
            .http
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {access_token}"))
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            return Err(api_error("GET /calendars/my/movies", resp).await);
        }
        resp.json::<Vec<CalendarMovieEntry>>()
            .await
            .context("parse Trakt movie calendar")
    }

    /// Mirror of [`push_collection`] for removals — same body shape,
    /// posted to `/sync/collection/remove`. Used by the nightly
    /// reconcile to mirror local-side deletes (a file scanner-removed
    /// since the previous push) up to Trakt.
    pub async fn remove_collection(
        &self,
        access_token: &str,
        movies: &[i64],
        episodes: &[(i64, i32, i32)],
    ) -> Result<()> {
        if movies.is_empty() && episodes.is_empty() {
            return Ok(());
        }
        let movies_json: Vec<_> = movies
            .iter()
            .map(|tmdb_id| json!({ "ids": { "tmdb": tmdb_id } }))
            .collect();
        let mut shows_map: std::collections::BTreeMap<i64, std::collections::BTreeMap<i32, Vec<i32>>> =
            std::collections::BTreeMap::new();
        for (show_tmdb, season, episode) in episodes {
            shows_map
                .entry(*show_tmdb)
                .or_default()
                .entry(*season)
                .or_default()
                .push(*episode);
        }
        let shows_json: Vec<_> = shows_map
            .into_iter()
            .map(|(show_tmdb, seasons)| {
                let seasons_json: Vec<_> = seasons
                    .into_iter()
                    .map(|(season, eps)| {
                        let eps_json: Vec<_> =
                            eps.into_iter().map(|n| json!({ "number": n })).collect();
                        json!({ "number": season, "episodes": eps_json })
                    })
                    .collect();
                json!({
                    "ids": { "tmdb": show_tmdb },
                    "seasons": seasons_json,
                })
            })
            .collect();
        let body = json!({ "movies": movies_json, "shows": shows_json });
        self.user_post("/sync/collection/remove", access_token, &body)
            .await?;
        Ok(())
    }

    /// Fetch Trakt's `/sync/last_activities` — a tree of timestamps
    /// for every category of user data (movies / shows / seasons /
    /// episodes × watched / collected / rated / watchlisted / paused
    /// / commented). The top-level `all` field is the max across all
    /// sections and acts as a single "anything changed since X?"
    /// cursor; we treat equality with the previously-seen `all` as a
    /// signal to skip the more expensive `/sync/history` /
    /// `/sync/playback` / `/sync/watchlist` pulls entirely.
    pub async fn pull_last_activities(&self, access_token: &str) -> Result<LastActivities> {
        let url = format!("{}/sync/last_activities", self.base_url);
        let resp = self
            .http
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {access_token}"))
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            return Err(api_error("GET /sync/last_activities", resp).await);
        }
        resp.json::<LastActivities>()
            .await
            .context("parse Trakt last_activities")
    }

    /// Push a bulk "I own these files" update to Trakt's collection.
    /// Trakt dedupes by ids server-side, so re-pushing on every nightly
    /// run is harmless; it just refreshes `collected_at`. Body shape
    /// matches `/sync/history` (movies + shows arrays with `ids` at
    /// top, episodes nested under `seasons[].episodes[]`).
    pub async fn push_collection(
        &self,
        access_token: &str,
        movies: &[i64],
        episodes: &[(i64, i32, i32)],
    ) -> Result<()> {
        if movies.is_empty() && episodes.is_empty() {
            return Ok(());
        }
        let movies_json: Vec<_> = movies
            .iter()
            .map(|tmdb_id| json!({ "ids": { "tmdb": tmdb_id } }))
            .collect();
        // Group episodes by show → season for the nested shape. Each
        // (show_tmdb, season, episode) tuple flattens into one show
        // entry with season → episodes nested under it; many tuples
        // sharing a show_tmdb collapse into a single entry.
        let mut shows_map: std::collections::BTreeMap<i64, std::collections::BTreeMap<i32, Vec<i32>>> =
            std::collections::BTreeMap::new();
        for (show_tmdb, season, episode) in episodes {
            shows_map
                .entry(*show_tmdb)
                .or_default()
                .entry(*season)
                .or_default()
                .push(*episode);
        }
        let shows_json: Vec<_> = shows_map
            .into_iter()
            .map(|(show_tmdb, seasons)| {
                let seasons_json: Vec<_> = seasons
                    .into_iter()
                    .map(|(season, eps)| {
                        let eps_json: Vec<_> = eps
                            .into_iter()
                            .map(|n| json!({ "number": n }))
                            .collect();
                        json!({ "number": season, "episodes": eps_json })
                    })
                    .collect();
                json!({
                    "ids": { "tmdb": show_tmdb },
                    "seasons": seasons_json,
                })
            })
            .collect();
        let body = json!({ "movies": movies_json, "shows": shows_json });
        self.user_post("/sync/collection", access_token, &body)
            .await?;
        Ok(())
    }

    /// Pull the user's full Trakt watchlist (movies + shows combined).
    /// Trakt returns a flat array with each entry tagged by `type`
    /// (`movie` | `show` | `season` | `episode`); season / episode
    /// entries are dropped by the caller since ChimpFlix's My List
    /// only stores at item level.
    pub async fn pull_watchlist(&self, access_token: &str) -> Result<Vec<WatchlistEntry>> {
        let url = format!("{}/sync/watchlist", self.base_url);
        let resp = self
            .http
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {access_token}"))
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            return Err(api_error("GET /sync/watchlist", resp).await);
        }
        resp.json::<Vec<WatchlistEntry>>()
            .await
            .context("parse Trakt watchlist")
    }

    pub async fn pull_history(
        &self,
        access_token: &str,
        start_at_iso: Option<&str>,
    ) -> Result<Vec<HistoryEntry>> {
        let mut url = format!("{}/sync/history?limit=200", self.base_url);
        if let Some(s) = start_at_iso {
            url.push_str("&start_at=");
            url.push_str(&urlencode(s));
        }
        let resp = self
            .http
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {access_token}"))
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            return Err(api_error("GET /sync/history", resp).await);
        }
        resp.json::<Vec<HistoryEntry>>()
            .await
            .context("parse Trakt history")
    }

    pub async fn pull_playback(&self, access_token: &str) -> Result<Vec<PlaybackEntry>> {
        let url = format!("{}/sync/playback?limit=200", self.base_url);
        let resp = self
            .http
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {access_token}"))
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            return Err(api_error("GET /sync/playback", resp).await);
        }
        resp.json::<Vec<PlaybackEntry>>()
            .await
            .context("parse Trakt playback")
    }

    pub async fn push_rating(&self, access_token: &str, entry: RatingPush) -> Result<()> {
        let (movies, episodes) = match entry {
            RatingPush::Movie {
                ids,
                rating,
                rated_at,
            } => (
                vec![json!({
                    "rated_at": rated_at,
                    "rating": rating,
                    "ids": ids.to_json(),
                })],
                vec![],
            ),
            RatingPush::Episode {
                show_ids,
                season,
                episode,
                rating,
                rated_at,
            } => (
                vec![],
                // Same shape correction as history/scrobble: `ids` at
                // the top level of the show entry, not nested under a
                // `"show":` wrapper (that's the response shape).
                vec![json!({
                    "rated_at": rated_at,
                    "ids": show_ids.to_json(),
                    "seasons": [{
                        "number": season,
                        "episodes": [{
                            "number": episode,
                            "rating": rating,
                            "rated_at": rated_at,
                        }],
                    }],
                })],
            ),
        };
        let body = json!({ "movies": movies, "shows": episodes });
        self.user_post("/sync/ratings", access_token, &body).await?;
        Ok(())
    }

    pub async fn remove_rating(&self, access_token: &str, entry: RatingPush) -> Result<()> {
        let (movies, episodes) = match entry {
            RatingPush::Movie { ids, .. } => {
                (vec![json!({ "ids": ids.to_json() })], vec![])
            }
            RatingPush::Episode {
                show_ids,
                season,
                episode,
                ..
            } => (
                vec![],
                vec![json!({
                    "ids": show_ids.to_json(),
                    "seasons": [{
                        "number": season,
                        "episodes": [{ "number": episode }],
                    }],
                })],
            ),
        };
        let body = json!({ "movies": movies, "shows": episodes });
        self.user_post("/sync/ratings/remove", access_token, &body)
            .await?;
        Ok(())
    }

    pub async fn pull_ratings(&self, access_token: &str) -> Result<Vec<RatingEntry>> {
        let url = format!("{}/sync/ratings", self.base_url);
        let resp = self
            .http
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {access_token}"))
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            return Err(api_error("GET /sync/ratings", resp).await);
        }
        resp.json::<Vec<RatingEntry>>()
            .await
            .context("parse Trakt ratings")
    }

    async fn user_post(
        &self,
        path: &str,
        access_token: &str,
        body: &serde_json::Value,
    ) -> Result<()> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .post(&url)
            .header(AUTHORIZATION, format!("Bearer {access_token}"))
            .json(body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        if !resp.status().is_success() {
            return Err(api_error(&format!("POST {path}"), resp).await);
        }
        Ok(())
    }
}

async fn api_error(label: &str, resp: reqwest::Response) -> anyhow::Error {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    warn!(%status, label, body = %text.chars().take(200).collect::<String>(), "Trakt API error");
    anyhow::anyhow!(
        "Trakt {label} returned {status}: {}",
        text.chars().take(200).collect::<String>()
    )
}

fn urlencode(s: &str) -> String {
    // Tiny inline encoder for the timestamp form Trakt expects
    // (ISO-8601 with `:` and `-`). We don't pull in `url` crate just
    // for this.
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(c),
            ':' => out.push_str("%3A"),
            other => {
                let mut buf = [0u8; 4];
                for byte in other.encode_utf8(&mut buf).as_bytes() {
                    out.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Public projections
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub expires_in: i64,
    pub interval: i64,
}

#[derive(Debug, Clone)]
pub enum DevicePollResult {
    Pending,
    Ready(TokenPair),
    Expired,
    Denied,
    AlreadyApproved,
    SlowDown,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
    pub scope: Option<String>,
    pub created_at: Option<i64>,
}

/// Set of external IDs ChimpFlix knows about a movie or show — at
/// least one must be present (the caller is expected to drop entries
/// where everything is `None` rather than send Trakt an unanchored
/// request). Trakt resolves by any of these; tmdb is preferred, but
/// for anime libraries matched via AniList only the tvdb fallback is
/// often the only thing populated.
#[derive(Debug, Clone, Default)]
pub struct TraktIdSet {
    pub tmdb: Option<i64>,
    pub imdb: Option<String>,
    pub tvdb: Option<i64>,
}

impl TraktIdSet {
    pub fn is_empty(&self) -> bool {
        self.tmdb.is_none() && self.imdb.is_none() && self.tvdb.is_none()
    }

    /// Render the set as a Trakt `ids` JSON object. Only includes
    /// fields that are populated.
    pub fn to_json(&self) -> serde_json::Value {
        let mut m = serde_json::Map::new();
        if let Some(t) = self.tmdb {
            m.insert("tmdb".into(), json!(t));
        }
        if let Some(ref s) = self.imdb {
            m.insert("imdb".into(), json!(s));
        }
        if let Some(t) = self.tvdb {
            m.insert("tvdb".into(), json!(t));
        }
        serde_json::Value::Object(m)
    }
}

#[derive(Debug, Clone)]
pub enum HistoryPush {
    Movie {
        ids: TraktIdSet,
        watched_at: String, // ISO-8601
    },
    Episode {
        show_ids: TraktIdSet,
        season: i32,
        episode: i32,
        watched_at: String,
    },
}

/// One of Trakt's three scrobble lifecycle events. `Start` opens the
/// live banner, `Pause` keeps the banner up but visually paused,
/// `Stop` closes it (and auto-writes history at progress ≥ 80%).
#[derive(Debug, Clone, Copy)]
pub enum ScrobbleAction {
    Start,
    Pause,
    Stop,
}

impl ScrobbleAction {
    fn path(self) -> &'static str {
        match self {
            ScrobbleAction::Start => "/scrobble/start",
            ScrobbleAction::Pause => "/scrobble/pause",
            ScrobbleAction::Stop => "/scrobble/stop",
        }
    }
}

/// Subset of `/users/me/stats` we surface in the UI. Trakt returns a
/// larger payload (network, seasons, ratings.distribution, …) but the
/// settings card only renders the watch-time totals + counts. The
/// rest is dropped on deserialize via `#[serde(default)]` per field.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UserStats {
    #[serde(default)]
    pub movies: UserStatsMovies,
    #[serde(default)]
    pub shows: UserStatsShows,
    #[serde(default)]
    pub episodes: UserStatsEpisodes,
    #[serde(default)]
    pub ratings: UserStatsRatings,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct UserStatsMovies {
    #[serde(default)]
    pub plays: u64,
    #[serde(default)]
    pub watched: u64,
    #[serde(default)]
    pub minutes: u64,
    #[serde(default)]
    pub collected: u64,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct UserStatsShows {
    #[serde(default)]
    pub watched: u64,
    #[serde(default)]
    pub collected: u64,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct UserStatsEpisodes {
    #[serde(default)]
    pub plays: u64,
    #[serde(default)]
    pub watched: u64,
    #[serde(default)]
    pub minutes: u64,
    #[serde(default)]
    pub collected: u64,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct UserStatsRatings {
    #[serde(default)]
    pub total: u64,
}

#[derive(Debug, Clone, Copy)]
pub enum RecommendationKind {
    Movies,
    Shows,
}

/// One personal Trakt list belonging to the authenticated user.
/// Captures the surface ChimpFlix renders on a rail header — name +
/// optional description + the id we'll use to fetch items.
#[derive(Debug, Clone, Deserialize)]
pub struct TraktList {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub item_count: i64,
    pub ids: TraktListIds,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TraktListIds {
    pub trakt: i64,
    pub slug: String,
}

/// One row of `/users/me/lists/{id}/items`. Trakt tags each entry by
/// `type` and embeds the relevant media object — same envelope as
/// history/watchlist responses.
#[derive(Debug, Clone, Deserialize)]
pub struct TraktListItem {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub movie: Option<TraktMovie>,
    #[serde(default)]
    pub show: Option<TraktShow>,
}

/// One row of `/users/hidden/{section}`. Trakt tags each entry with a
/// `type` plus the inline media object — same flat shape used by
/// history/watchlist responses.
#[derive(Debug, Clone, Deserialize)]
pub struct HiddenEntry {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub movie: Option<TraktMovie>,
    #[serde(default)]
    pub show: Option<TraktShow>,
}

/// One row from `/recommendations/{movies|shows}`. Trakt returns the
/// full media object inline (no `movie:` / `show:` wrapper here —
/// unlike history/calendar, the response IS the array of items).
/// We only need `ids` for local matching; other fields are dropped.
#[derive(Debug, Clone, Deserialize)]
pub struct RecommendationEntry {
    pub title: String,
    pub year: Option<i32>,
    pub ids: TraktIds,
}

/// One row of `/calendars/my/shows`. Trakt returns the show in the
/// `show` field and the upcoming episode in `episode`, plus a
/// `first_aired` ISO timestamp for the episode's air date.
#[derive(Debug, Clone, Deserialize)]
pub struct CalendarEpisodeEntry {
    pub first_aired: String,
    pub episode: TraktEpisode,
    pub show: TraktShow,
}

#[derive(Debug, Clone, Copy)]
pub enum ShowCalendarKind {
    /// Every upcoming episode for shows the user watches.
    Shows,
    /// Series premieres — Episode 1 of brand-new series.
    NewShows,
    /// Season premieres — Episode 1 of any season the user hasn't
    /// started yet for shows they already watch.
    SeasonPremieres,
}

/// One row of `/calendars/my/movies`. Movie releases use a `released`
/// date (YYYY-MM-DD only, no time) rather than `first_aired`, and
/// have no episode coords.
#[derive(Debug, Clone, Deserialize)]
pub struct CalendarMovieEntry {
    pub released: String,
    pub movie: TraktMovie,
}

/// Response shape for `/sync/last_activities`. Only the top-level
/// `all` field is captured here — the per-section timestamps are
/// available in Trakt's docs if a finer-grained cursor is ever
/// needed, but for "anything changed?" the rollup is sufficient.
#[derive(Debug, Clone, Deserialize)]
pub struct LastActivities {
    pub all: String,
}

/// One watchlist entry being pushed up. Mirrors ChimpFlix's `items.kind`
/// split (movie vs. tv show) — seasons/episodes aren't stored in My
/// List, so they don't need a representation here.
#[derive(Debug, Clone)]
pub enum WatchlistPush {
    Movie { tmdb_id: i64 },
    Show { tmdb_id: i64 },
}

fn watchlist_buckets(entries: &[WatchlistPush]) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    let mut movies = Vec::new();
    let mut shows = Vec::new();
    for e in entries {
        match e {
            WatchlistPush::Movie { tmdb_id } => movies.push(json!({
                "ids": { "tmdb": tmdb_id },
            })),
            WatchlistPush::Show { tmdb_id } => shows.push(json!({
                "ids": { "tmdb": tmdb_id },
            })),
        }
    }
    (movies, shows)
}

/// One row of the GET `/sync/watchlist` response. Trakt returns a
/// single flat array tagged by `type`; we keep all four flavours
/// optional so the deserializer handles every variant without us
/// needing a sum type per kind.
#[derive(Debug, Clone, Deserialize)]
pub struct WatchlistEntry {
    #[serde(default)]
    pub rank: Option<i64>,
    pub listed_at: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub movie: Option<TraktMovie>,
    #[serde(default)]
    pub show: Option<TraktShow>,
}

#[derive(Debug, Clone)]
pub enum ScrobblePush {
    Movie {
        ids: TraktIdSet,
        /// Percentage 0-100. Clamped in [`TraktClient::scrobble`].
        progress: f64,
    },
    Episode {
        show_ids: TraktIdSet,
        season: i32,
        episode: i32,
        progress: f64,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct HistoryEntry {
    pub id: i64,
    pub watched_at: String,
    pub action: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub movie: Option<TraktMovie>,
    #[serde(default)]
    pub episode: Option<TraktEpisode>,
    #[serde(default)]
    pub show: Option<TraktShow>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TraktMovie {
    pub title: String,
    pub year: Option<i32>,
    pub ids: TraktIds,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TraktShow {
    pub title: String,
    pub year: Option<i32>,
    pub ids: TraktIds,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TraktEpisode {
    pub season: i32,
    pub number: i32,
    pub title: Option<String>,
    pub ids: TraktIds,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct TraktIds {
    pub trakt: Option<i64>,
    pub tmdb: Option<i64>,
    pub imdb: Option<String>,
    pub tvdb: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaybackEntry {
    pub id: i64,
    pub progress: f64, // 0.0..100.0
    pub paused_at: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub movie: Option<TraktMovie>,
    #[serde(default)]
    pub episode: Option<TraktEpisode>,
    #[serde(default)]
    pub show: Option<TraktShow>,
}

#[derive(Debug, Clone)]
pub enum RatingPush {
    Movie {
        ids: TraktIdSet,
        rating: i32,
        rated_at: String,
    },
    Episode {
        show_ids: TraktIdSet,
        season: i32,
        episode: i32,
        rating: i32,
        rated_at: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct RatingEntry {
    pub rated_at: String,
    pub rating: i32,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub movie: Option<TraktMovie>,
    #[serde(default)]
    pub episode: Option<TraktEpisode>,
    #[serde(default)]
    pub show: Option<TraktShow>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_creds() {
        assert!(TraktClient::new("", "x").is_err());
        assert!(TraktClient::new("x", "").is_err());
        assert!(TraktClient::new("   ", "x").is_err());
    }

    #[test]
    fn constructs_with_valid_creds() {
        assert!(TraktClient::new("client", "secret").is_ok());
    }

    #[test]
    fn urlencode_keeps_alphanum_escapes_colon() {
        assert_eq!(
            urlencode("2024-01-19T12:34:56Z"),
            "2024-01-19T12%3A34%3A56Z"
        );
    }
}

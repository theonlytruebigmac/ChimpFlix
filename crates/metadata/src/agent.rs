//! Unified metadata-agent framework.
//!
//! Each provider (TMDB, TheTVDB, TVMaze, AniList, OMDb, ...) implements
//! the [`MetadataAgent`] trait. The scanner holds a heterogeneous
//! `Vec<Arc<dyn MetadataAgent>>` ordered by the library's chain config,
//! iterates it for each scanned file, and asks every capable agent for
//! the metadata it can supply. The first agent in the chain runs in
//! `WriteMode::Primary` (its data takes precedence); subsequent agents
//! run in `WriteMode::FillNulls` (data fills holes without clobbering
//! earlier writes).
//!
//! ## Architectural split: fetch vs apply
//!
//! Agents are SQL-agnostic. Each `fetch_*` method returns a typed
//! result that the library crate translates into UPDATE statements via
//! its own apply helpers. Keeping SQL out of the metadata crate means:
//!
//!   1. Adding a new agent never touches the database layer.
//!   2. The agent crate can ship without sqlx as a dependency.
//!   3. Tests for an agent can assert on returned values directly
//!      without booting an in-memory pool.
//!
//! ## Capability matrix
//!
//! Not every agent supports every kind of metadata. [`Capabilities`]
//! is the agent's self-declaration of what it CAN return. The scanner
//! dispatch loop reads it before calling — an agent with
//! `capabilities.episode = false` is skipped for episode lookups even
//! if the chain includes it. The admin UI renders the matrix as
//! per-agent badges plus operator-visible limitations strings.
//!
//! ## WriteMode
//!
//! [`WriteMode`] flows from the chain-position the scanner is calling
//! the agent in. Agents don't make this decision themselves; the
//! scanner does the position math.

use async_trait::async_trait;
use serde::Serialize;

// ---------------------------------------------------------------------------
// Capability matrix
// ---------------------------------------------------------------------------

/// What kinds of metadata an agent can produce. Returned by
/// [`MetadataAgent::capabilities`]. The dispatch loop uses this to
/// gate per-stage calls (no point asking TVMaze for cast credits when
/// the API doesn't expose them).
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Capabilities {
    /// Movies: title, year, runtime, summary, posters, etc.
    pub movie: bool,
    /// Shows: same as movie but for TV (no per-season info — see episode).
    pub show: bool,
    /// Episodes: title, summary, runtime, air date, still image.
    pub episode: bool,
    /// Cast + crew (actors, directors, writers) with character/role.
    pub cast: bool,
    /// Multiple artwork variants (poster/backdrop/logo/clearart).
    pub artwork: bool,
    /// External ratings (IMDb / RT / Metacritic / age rating).
    pub ratings: bool,
}

impl Capabilities {
    pub const fn nothing() -> Self {
        Self {
            movie: false,
            show: false,
            episode: false,
            cast: false,
            artwork: false,
            ratings: false,
        }
    }

    /// True if the agent contributes nothing — useful for the dispatch
    /// loop's fast-skip check.
    pub fn is_empty(self) -> bool {
        !(self.movie || self.show || self.episode || self.cast || self.artwork || self.ratings)
    }
}

// ---------------------------------------------------------------------------
// WriteMode
// ---------------------------------------------------------------------------

/// Write semantics used by metadata-agent apply functions. The first
/// enabled agent in a library's chain runs in `Primary` mode and
/// overwrites null-or-stale columns; every subsequent agent runs in
/// `FillNulls` and writes only where the column is NULL (or, for
/// `NOT NULL` columns like `episodes.title`, where the existing value
/// looks filename-derived).
///
/// The scanner determines mode from `chain.position(agent_name) == 0`.
/// Agents themselves never look at this — it's passed through to the
/// library-side apply helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
    Primary,
    FillNulls,
}

impl WriteMode {
    pub fn overwrites(self) -> bool {
        matches!(self, WriteMode::Primary)
    }
}

// ---------------------------------------------------------------------------
// Lookup hints
// ---------------------------------------------------------------------------

/// Hints for a movie lookup. Each external id is `Some` when a prior
/// agent in the chain already discovered it, which lets later agents
/// skip search and go straight to fetch-by-id.
#[derive(Debug, Clone)]
pub struct MovieLookup {
    pub item_id: i64,
    pub title: String,
    pub year: Option<i32>,
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<i64>,
    pub tvdb_id: Option<i64>,
}

/// Hints for a show lookup. Same shape as `MovieLookup` plus
/// AniList — anime libraries use AniList ids that don't map cleanly
/// onto TMDB/TVDB's tree-of-seasons model.
#[derive(Debug, Clone)]
pub struct ShowLookup {
    pub item_id: i64,
    pub title: String,
    pub year: Option<i32>,
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<i64>,
    pub tvdb_id: Option<i64>,
    pub anilist_id: Option<i64>,
    pub tvmaze_id: Option<i64>,
}

/// Hints for an episode lookup. `episode_number` is the value the
/// scanner has decided to query at this agent — when the show is in
/// absolute-numbering mode, the scanner remaps before calling
/// season-relative agents, so each agent sees a number it can use
/// natively. `absolute_number` is preserved so absolute-aware agents
/// (AniList, AniDB) can prefer it when present.
#[derive(Debug, Clone)]
pub struct EpisodeLookup {
    pub episode_id: i64,
    pub show: ShowLookup,
    pub season_number: i32,
    pub episode_number: i32,
    pub absolute_number: Option<i32>,
}

// ---------------------------------------------------------------------------
// Returned data shapes
// ---------------------------------------------------------------------------

/// Common-shape movie metadata. Each field is `Option` so agents can
/// only populate what they have; the library-side apply layer COALESCEs
/// per-column according to `WriteMode`.
#[derive(Debug, Clone, Default)]
pub struct MovieData {
    pub title: Option<String>,
    pub original_title: Option<String>,
    pub year: Option<i32>,
    pub release_date_ms: Option<i64>,
    pub summary: Option<String>,
    pub tagline: Option<String>,
    pub runtime_ms: Option<i64>,
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<i64>,
    pub tvdb_id: Option<i64>,
    /// TMDB-style audience rating (0-100). Other agents may map their
    /// own scale into this; OMDb's "metascore" already lives in that
    /// range.
    pub rating_audience: Option<i32>,
    /// Title-treatment logo URL (already absolute — no path-resolution
    /// dance left for the apply layer).
    pub logo_url: Option<String>,
    pub genres: Vec<String>,
    pub posters: Vec<ArtworkVariant>,
    pub backdrops: Vec<ArtworkVariant>,
    pub people: Vec<PersonCredit>,
    pub ratings: Vec<ExternalRating>,
    /// TMDB collection (franchise) the movie belongs to. Other agents
    /// don't have an equivalent — left `None`.
    pub tmdb_collection: Option<TmdbCollectionRef>,
    /// Promotional videos (trailers, featurettes). Only TMDB exposes
    /// these today; left empty by other agents.
    pub videos: Vec<VideoLink>,
    /// User-submitted reviews. Only TMDB exposes these today.
    pub reviews: Vec<ReviewEntry>,
}

/// Reference to a franchise / collection. Currently TMDB-only (no
/// other agent exposes franchise grouping), but kept agent-agnostic
/// in shape so a future provider that does could populate it too.
#[derive(Debug, Clone)]
pub struct TmdbCollectionRef {
    pub tmdb_id: i64,
    pub name: String,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VideoLink {
    /// Provider key (YouTube video id for TMDB).
    pub provider_key: String,
    pub name: String,
    /// "trailer" | "teaser" | "featurette" | "clip" | "behind-the-scenes".
    pub kind: String,
    pub official: bool,
    pub published_at_ms: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ReviewEntry {
    pub source_id: String,
    pub author: String,
    pub author_url: Option<String>,
    pub avatar_url: Option<String>,
    pub rating: Option<i32>,
    pub body: Option<String>,
    pub created_at_ms: Option<i64>,
}

/// Common-shape show (TV series) metadata.
#[derive(Debug, Clone, Default)]
pub struct ShowData {
    pub title: Option<String>,
    pub original_title: Option<String>,
    pub year: Option<i32>,
    pub summary: Option<String>,
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<i64>,
    pub tvdb_id: Option<i64>,
    pub anilist_id: Option<i64>,
    pub tvmaze_id: Option<i64>,
    pub genres: Vec<String>,
    pub posters: Vec<ArtworkVariant>,
    pub backdrops: Vec<ArtworkVariant>,
    pub people: Vec<PersonCredit>,
    /// Episode counts per season, used by the absolute-episode resolver
    /// to remap (S=1, ep=29) → (S=3, ep=5). Agents that don't track
    /// per-season counts (TVMaze, OMDb) leave this empty.
    pub season_episode_counts: Vec<SeasonEpisodeCount>,
    /// Promotional videos (trailers, featurettes). TMDB-populated; other
    /// agents leave empty.
    pub videos: Vec<VideoLink>,
    /// User-submitted reviews. TMDB-populated.
    pub reviews: Vec<ReviewEntry>,
}

#[derive(Debug, Clone)]
pub struct SeasonEpisodeCount {
    pub season_number: i32,
    pub episode_count: i32,
}

/// Common-shape episode metadata.
#[derive(Debug, Clone, Default)]
pub struct EpisodeData {
    pub title: Option<String>,
    pub summary: Option<String>,
    pub runtime_ms: Option<i64>,
    pub air_date_ms: Option<i64>,
    pub still_url: Option<String>,
    pub tmdb_id: Option<i64>,
    pub tvdb_id: Option<i64>,
    pub people: Vec<PersonCredit>,
}

#[derive(Debug, Clone)]
pub struct ArtworkVariant {
    /// Absolute URL to the artwork.
    pub url: String,
    /// Optional language tag ("en", "ja") so the apply layer can prefer
    /// the operator's UI language when multiple variants exist.
    pub language: Option<String>,
    /// Provider-reported quality / votes / aspect ratio metric, used to
    /// pick the best variant when an operator hasn't pinned one. Higher
    /// is better. Default 0 for sources that don't report it.
    pub score: f32,
}

#[derive(Debug, Clone)]
pub struct PersonCredit {
    /// External id from this agent's namespace (TMDB person id, TVDB
    /// person id, etc.). Used together with `source` to dedupe across
    /// chain runs.
    pub external_id: Option<String>,
    pub name: String,
    /// "actor" | "director" | "writer" | "producer" | "creator".
    pub role: String,
    /// Character name when `role = "actor"`.
    pub character: Option<String>,
    pub order: i32,
    pub profile_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExternalRating {
    /// "imdb" | "rt" | "metacritic" | "mpaa" | "tvpg".
    pub source: String,
    /// Free-form rating value as the agent returned it ("7.8/10",
    /// "85%", "PG-13"). Display layer interprets per-source.
    pub value: String,
}

// ---------------------------------------------------------------------------
// Agent trait
// ---------------------------------------------------------------------------

/// Unified contract every metadata provider implements. Default impls
/// return `Ok(None)` so an agent can opt out of any stage it doesn't
/// support without forcing boilerplate.
///
/// **Error convention:** transient upstream failures (network, 5xx,
/// rate limit) return `Err`. Not-found (`No upstream match for this
/// title`) returns `Ok(None)`. Callers log `Err` and continue to the
/// next agent; `Ok(None)` is silent.
#[async_trait]
pub trait MetadataAgent: Send + Sync {
    /// Stable identifier matching `library_agents.agent_name`.
    fn name(&self) -> &'static str;

    fn capabilities(&self) -> Capabilities;

    /// Operator-facing one-liner limitations. Surfaced in the admin
    /// UI's agent picker. Empty when there's nothing notable.
    fn limitations(&self) -> &'static [&'static str] {
        &[]
    }

    async fn fetch_movie(&self, _lookup: &MovieLookup) -> anyhow::Result<Option<MovieData>> {
        Ok(None)
    }

    async fn fetch_show(&self, _lookup: &ShowLookup) -> anyhow::Result<Option<ShowData>> {
        Ok(None)
    }

    async fn fetch_episode(&self, _lookup: &EpisodeLookup) -> anyhow::Result<Option<EpisodeData>> {
        Ok(None)
    }
}

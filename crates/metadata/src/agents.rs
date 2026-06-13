//! Concrete [`MetadataAgent`] impls for each provider.
//!
//! Each agent wraps the corresponding client and translates that
//! client's native response shape into the common
//! [`MovieData`] / [`ShowData`] / [`EpisodeData`] structs the trait
//! returns. The library-side apply layer is provider-agnostic from
//! here on; it COALESCEs columns based on `WriteMode` and that's it.
//!
//! Translation rules:
//! - URL paths from APIs that return relative paths (TMDB) are resolved
//!   to absolute URLs here. Callers shouldn't need to know about each
//!   provider's image base.
//! - Provider-specific extras (TMDB collections, AniList streaming
//!   episodes) that don't fit the common shape stay in the legacy
//!   scanner paths for now; they'll fold in as subsequent slices add
//!   the right common fields.
//!
//! Agents own their client by value. Provider clients all derive
//! Clone (the reqwest::Client they wrap is internally Arc'd), so
//! cloning into the registry is cheap and there are no lifetime
//! puzzles with the scanner's task tree.

use async_trait::async_trait;

use crate::agent::{
    ArtworkVariant, Capabilities, EpisodeData, EpisodeLookup, MetadataAgent, MovieData,
    MovieLookup, PersonCredit, ReviewEntry, ShowData, ShowLookup, TmdbCollectionRef, VideoLink,
};
use crate::anilist::AniListClient;
use crate::omdb::OmdbClient;
use crate::tmdb::{
    TmdbCastMember, TmdbClient, TmdbCredits, TmdbCrewMember, TmdbKind, TmdbReview, TmdbVideo,
    tmdb_image_url,
};
use crate::tvdb::TvdbClient;
use crate::tvmaze::TvMazeClient;

/// Translate TMDB's cast + crew payload into the common shape.
fn tmdb_credits_to_people(credits: TmdbCredits) -> Vec<PersonCredit> {
    let mut out: Vec<PersonCredit> = Vec::new();
    for (idx, m) in credits.cast.into_iter().enumerate() {
        out.push(tmdb_cast_to_person(m, idx));
    }
    for (idx, m) in credits.crew.into_iter().enumerate() {
        out.push(tmdb_crew_to_person(m, idx));
    }
    out
}

fn tmdb_cast_to_person(m: TmdbCastMember, idx: usize) -> PersonCredit {
    PersonCredit {
        external_id: Some(format!("tmdb:{}", m.tmdb_person_id)),
        name: m.name,
        role: "actor".to_string(),
        character: m.character,
        order: if m.order != 0 { m.order } else { idx as i32 },
        profile_url: m.profile_path.map(|p| tmdb_image_url(&p, "w185")),
    }
}

fn tmdb_crew_to_person(m: TmdbCrewMember, idx: usize) -> PersonCredit {
    let role = match m.job.as_str() {
        "Director" => "director".to_string(),
        "Writer" | "Screenplay" => "writer".to_string(),
        "Producer" | "Executive Producer" => "producer".to_string(),
        _ => "crew".to_string(),
    };
    PersonCredit {
        external_id: Some(format!("tmdb:{}", m.tmdb_person_id)),
        name: m.name,
        role,
        character: None,
        order: idx as i32,
        profile_url: m.profile_path.map(|p| tmdb_image_url(&p, "w185")),
    }
}

fn tmdb_video_to_link(v: TmdbVideo) -> VideoLink {
    let kind = match v.kind.as_str() {
        "Trailer" => "trailer",
        "Teaser" => "teaser",
        "Featurette" => "featurette",
        "Clip" => "clip",
        "Behind the Scenes" => "behind-the-scenes",
        _ => "other",
    }
    .to_string();
    let published_at_ms = v.published_at.as_deref().and_then(parse_iso8601_to_ms);
    VideoLink {
        provider_key: v.key,
        name: v.name,
        kind,
        official: v.official,
        published_at_ms,
    }
}

fn tmdb_review_to_entry(r: TmdbReview) -> ReviewEntry {
    ReviewEntry {
        source_id: r.source_id,
        author: r.author,
        author_url: r.author_url,
        avatar_url: r.avatar_url,
        rating: r.rating,
        body: r.body,
        created_at_ms: r.created_at,
    }
}

/// Best-effort ISO-8601 → epoch ms. Returns None for parse failures
/// so the caller can leave the column null.
fn parse_iso8601_to_ms(s: &str) -> Option<i64> {
    // Accept "2024-03-15T12:00:00.000Z" or "2024-03-15T12:00:00Z" or
    // a date-only "2024-03-15".
    chrono_lite::parse_to_ms(s)
}

/// Tiny inline date parser — avoids pulling chrono into the metadata
/// crate for this one use. Accepts ISO-8601 in the formats TMDB
/// returns (date-only or full datetime with milliseconds + Z).
mod chrono_lite {
    pub fn parse_to_ms(s: &str) -> Option<i64> {
        let bytes = s.as_bytes();
        if bytes.len() < 10 {
            return None;
        }
        let y: i64 = std::str::from_utf8(&bytes[0..4]).ok()?.parse().ok()?;
        let m: i64 = std::str::from_utf8(&bytes[5..7]).ok()?.parse().ok()?;
        let d: i64 = std::str::from_utf8(&bytes[8..10]).ok()?.parse().ok()?;
        let (hh, mm, ss) = if bytes.len() >= 19 && bytes[10] == b'T' {
            let h: i64 = std::str::from_utf8(&bytes[11..13]).ok()?.parse().ok()?;
            let mn: i64 = std::str::from_utf8(&bytes[14..16]).ok()?.parse().ok()?;
            let s: i64 = std::str::from_utf8(&bytes[17..19]).ok()?.parse().ok()?;
            (h, mn, s)
        } else {
            (0, 0, 0)
        };
        // Convert civil date to days-from-epoch using Howard Hinnant's
        // algorithm. Mirrors what `parse_air_date_to_ms` in queries.rs
        // does — duplicated here so metadata crate doesn't depend on
        // library.
        let y_adj = if m <= 2 { y - 1 } else { y };
        let era = y_adj.div_euclid(400);
        let yoe = y_adj - era * 400;
        let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146097 + doe - 719468;
        Some(days * 86_400_000 + hh * 3_600_000 + mm * 60_000 + ss * 1_000)
    }
}

/// Static capability + limitations lookup keyed by agent name. Used by
/// callers (admin UI registry, library agent picker) that need to know
/// what an agent CAN do without constructing the agent struct — which
/// would require a configured upstream client.
///
/// The agent trait's `capabilities()` / `limitations()` methods read
/// from the same source on the implementation side; this top-level
/// function is the equivalent dyn-free shortcut.
pub fn static_capabilities_for(name: &str) -> Capabilities {
    match name {
        "tmdb" => TmdbAgent::CAPABILITIES,
        "tvdb" => TvdbAgent::CAPABILITIES,
        "tvmaze" => TvMazeAgent::CAPABILITIES,
        "anilist" => AniListAgent::CAPABILITIES,
        "omdb" => OmdbAgent::CAPABILITIES,
        _ => Capabilities::nothing(),
    }
}

pub fn static_limitations_for(name: &str) -> &'static [&'static str] {
    match name {
        "tmdb" => TmdbAgent::LIMITATIONS,
        "tvdb" => TvdbAgent::LIMITATIONS,
        "tvmaze" => TvMazeAgent::LIMITATIONS,
        "anilist" => AniListAgent::LIMITATIONS,
        "omdb" => OmdbAgent::LIMITATIONS,
        _ => &[],
    }
}

// ---------------------------------------------------------------------------
// TMDB
// ---------------------------------------------------------------------------

/// TMDB agent — the broadest-coverage source. Provides movies, shows,
/// episodes, cast, and artwork.
pub struct TmdbAgent {
    client: TmdbClient,
}

impl TmdbAgent {
    pub const CAPABILITIES: Capabilities = Capabilities {
        movie: true,
        show: true,
        episode: true,
        cast: true,
        artwork: true,
        ratings: false,
    };
    pub const LIMITATIONS: &'static [&'static str] = &[
        "Episode metadata requires a TMDB show id. Splits anime seasons \
         differently from AniList — absolute-numbered files need \
         remapping before lookup.",
    ];

    pub fn new(client: TmdbClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl MetadataAgent for TmdbAgent {
    fn name(&self) -> &'static str {
        "tmdb"
    }

    fn capabilities(&self) -> Capabilities {
        Self::CAPABILITIES
    }

    fn limitations(&self) -> &'static [&'static str] {
        Self::LIMITATIONS
    }

    async fn fetch_movie(&self, lookup: &MovieLookup) -> anyhow::Result<Option<MovieData>> {
        let Some(meta) = self.client.lookup_movie(&lookup.title, lookup.year).await? else {
            return Ok(None);
        };
        let mut posters = Vec::new();
        if let Some(p) = &meta.poster_path {
            posters.push(ArtworkVariant {
                url: tmdb_image_url(p, "w500"),
                language: None,
                score: 1.0,
            });
        }
        let mut backdrops = Vec::new();
        if let Some(p) = &meta.backdrop_path {
            backdrops.push(ArtworkVariant {
                url: tmdb_image_url(p, "w1280"),
                language: None,
                score: 1.0,
            });
        }
        let logo_url = meta.logo_path.as_deref().map(|p| tmdb_image_url(p, "w500"));
        // TMDB returns vote_average=0.0 (not null) for newly-added titles with
        // zero votes. Filter those out so unrated titles stay NULL in the DB
        // rather than sorting as 0 in rating-based queries.
        let rating_audience = meta.rating_audience.filter(|&r| r > 0.0).map(|r| (r * 10.0).round() as i32);

        // Cast/crew + videos + reviews are fetched in parallel — three
        // independent endpoints, each with its own auth + URL. Failure
        // of any one degrades that field to empty without poisoning the
        // others; the caller decides whether to surface or log.
        let (credits, videos, reviews) = tokio::join!(
            self.client.fetch_credits(TmdbKind::Movie, meta.tmdb_id),
            self.client.fetch_videos(TmdbKind::Movie, meta.tmdb_id),
            self.client.fetch_reviews(TmdbKind::Movie, meta.tmdb_id),
        );
        let people = credits.map(tmdb_credits_to_people).unwrap_or_default();
        let videos = videos
            .map(|vs| vs.into_iter().map(tmdb_video_to_link).collect())
            .unwrap_or_default();
        let reviews = reviews
            .map(|rs| rs.into_iter().map(tmdb_review_to_entry).collect())
            .unwrap_or_default();

        let tmdb_collection = meta.collection.as_ref().map(|s| TmdbCollectionRef {
            tmdb_id: s.tmdb_id,
            name: s.name.clone(),
            poster_path: s.poster_path.clone(),
            backdrop_path: s.backdrop_path.clone(),
        });

        Ok(Some(MovieData {
            title: Some(meta.title.clone()),
            original_title: meta.original_title.clone(),
            year: meta.year,
            release_date_ms: None,
            summary: meta.summary.clone(),
            tagline: meta.tagline.clone(),
            runtime_ms: meta.runtime_min.map(|m| (m as i64) * 60_000),
            imdb_id: meta.imdb_id.clone(),
            tmdb_id: Some(meta.tmdb_id),
            tvdb_id: None,
            rating_audience,
            logo_url,
            genres: meta.genres.clone(),
            posters,
            backdrops,
            people,
            ratings: Vec::new(),
            tmdb_collection,
            videos,
            reviews,
        }))
    }

    async fn fetch_show(&self, lookup: &ShowLookup) -> anyhow::Result<Option<ShowData>> {
        let Some(meta) = self.client.lookup_show(&lookup.title, lookup.year).await? else {
            return Ok(None);
        };
        let mut posters = Vec::new();
        if let Some(p) = &meta.poster_path {
            posters.push(ArtworkVariant {
                url: tmdb_image_url(p, "w500"),
                language: None,
                score: 1.0,
            });
        }
        let mut backdrops = Vec::new();
        if let Some(p) = &meta.backdrop_path {
            backdrops.push(ArtworkVariant {
                url: tmdb_image_url(p, "w1280"),
                language: None,
                score: 1.0,
            });
        }

        let (credits, videos, reviews) = tokio::join!(
            self.client.fetch_credits(TmdbKind::Show, meta.tmdb_id),
            self.client.fetch_videos(TmdbKind::Show, meta.tmdb_id),
            self.client.fetch_reviews(TmdbKind::Show, meta.tmdb_id),
        );
        let people = credits.map(tmdb_credits_to_people).unwrap_or_default();
        let videos = videos
            .map(|vs| vs.into_iter().map(tmdb_video_to_link).collect())
            .unwrap_or_default();
        let reviews = reviews
            .map(|rs| rs.into_iter().map(tmdb_review_to_entry).collect())
            .unwrap_or_default();

        Ok(Some(ShowData {
            title: Some(meta.title.clone()),
            original_title: meta.original_title.clone(),
            year: meta.year,
            summary: meta.summary.clone(),
            imdb_id: meta.imdb_id.clone(),
            tmdb_id: Some(meta.tmdb_id),
            tvdb_id: None,
            anilist_id: None,
            tvmaze_id: None,
            genres: meta.genres.clone(),
            posters,
            backdrops,
            people,
            season_episode_counts: Vec::new(),
            videos,
            reviews,
        }))
    }

    async fn fetch_episode(&self, lookup: &EpisodeLookup) -> anyhow::Result<Option<EpisodeData>> {
        // TMDB needs a show id to fetch episodes. The dispatcher seeds
        // it from a prior `fetch_show` call (or persisted `tmdb_id` on
        // items). If absent, we can't address an episode.
        let Some(show_tmdb_id) = lookup.show.tmdb_id else {
            return Ok(None);
        };
        let season = match self
            .client
            .fetch_season(show_tmdb_id, lookup.season_number)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                let msg = format!("{e:#}");
                // Match "returned 404" rather than bare "404" to avoid
                // treating a non-404 error whose URL happens to contain
                // "404" (e.g. show id 4040404) as a missing-season.
                if msg.contains("returned 404") {
                    return Ok(None);
                }
                return Err(e);
            }
        };
        let Some(ep) = season
            .episodes
            .iter()
            .find(|e| e.episode_number == lookup.episode_number)
        else {
            return Ok(None);
        };
        let still_url = ep.still_path.as_deref().map(|p| tmdb_image_url(p, "w300"));
        Ok(Some(EpisodeData {
            title: Some(ep.title.clone()),
            summary: ep.summary.clone(),
            runtime_ms: ep.runtime_min.map(|m| (m as i64) * 60_000),
            air_date_ms: None,
            still_url,
            tmdb_id: Some(ep.tmdb_id),
            tvdb_id: None,
            people: Vec::new(),
        }))
    }
}

// ---------------------------------------------------------------------------
// TheTVDB
// ---------------------------------------------------------------------------

pub struct TvdbAgent {
    client: TvdbClient,
}

impl TvdbAgent {
    pub const CAPABILITIES: Capabilities = Capabilities {
        movie: true,
        show: true,
        episode: true,
        cast: true,
        artwork: true,
        ratings: false,
    };
    pub const LIMITATIONS: &'static [&'static str] = &[
        "Episode lookup re-fetches the full episode list per call \
         (no per-show cache yet). Acceptable for typical show \
         sizes; long-runner first scans may be slow.",
        "Per-episode cast costs one extra HTTP call per episode \
         (`/episodes/{id}/extended`). The TVDB free tier supports this \
         volume but it noticeably slows first scans of large libraries.",
    ];

    pub fn new(client: TvdbClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl MetadataAgent for TvdbAgent {
    fn name(&self) -> &'static str {
        "tvdb"
    }

    fn capabilities(&self) -> Capabilities {
        Self::CAPABILITIES
    }

    fn limitations(&self) -> &'static [&'static str] {
        Self::LIMITATIONS
    }

    async fn fetch_movie(&self, lookup: &MovieLookup) -> anyhow::Result<Option<MovieData>> {
        let Some(meta) = self.client.lookup_movie(&lookup.title, lookup.year).await? else {
            return Ok(None);
        };
        let mut posters = Vec::new();
        if let Some(url) = &meta.poster_url {
            posters.push(ArtworkVariant {
                url: url.clone(),
                language: None,
                score: 1.0,
            });
        }
        let mut backdrops = Vec::new();
        if let Some(url) = &meta.backdrop_url {
            backdrops.push(ArtworkVariant {
                url: url.clone(),
                language: None,
                score: 1.0,
            });
        }
        Ok(Some(MovieData {
            title: Some(meta.title.clone()),
            original_title: meta.original_title.clone(),
            year: meta.year,
            summary: meta.summary.clone(),
            runtime_ms: meta.runtime_minutes.map(|m| (m as i64) * 60_000),
            imdb_id: meta.imdb_id.clone(),
            tvdb_id: Some(meta.tvdb_id),
            genres: meta.genres.clone(),
            posters,
            backdrops,
            ..Default::default()
        }))
    }

    async fn fetch_show(&self, lookup: &ShowLookup) -> anyhow::Result<Option<ShowData>> {
        let Some(meta) = self.client.lookup_show(&lookup.title, lookup.year).await? else {
            return Ok(None);
        };
        let mut posters = Vec::new();
        if let Some(url) = &meta.poster_url {
            posters.push(ArtworkVariant {
                url: url.clone(),
                language: None,
                score: 1.0,
            });
        }
        let mut backdrops = Vec::new();
        if let Some(url) = &meta.backdrop_url {
            backdrops.push(ArtworkVariant {
                url: url.clone(),
                language: None,
                score: 1.0,
            });
        }
        Ok(Some(ShowData {
            title: Some(meta.title.clone()),
            original_title: meta.original_title.clone(),
            year: meta.year,
            summary: meta.summary.clone(),
            imdb_id: meta.imdb_id.clone(),
            tvdb_id: Some(meta.tvdb_id),
            genres: meta.genres.clone(),
            posters,
            backdrops,
            ..Default::default()
        }))
    }

    async fn fetch_episode(&self, lookup: &EpisodeLookup) -> anyhow::Result<Option<EpisodeData>> {
        let Some(show_tvdb_id) = lookup.show.tvdb_id else {
            return Ok(None);
        };
        let episodes = self.client.fetch_episodes(show_tvdb_id).await?;
        let Some(ep) = episodes.iter().find(|e| {
            e.season_number == lookup.season_number && e.episode_number == lookup.episode_number
        }) else {
            return Ok(None);
        };
        // Cast is fetched lazily via /episodes/{id}/extended — one
        // extra HTTP call per episode. Failures degrade `people` to
        // empty but don't fail the whole episode lookup.
        let people = match self.client.fetch_episode_extended(ep.tvdb_id).await {
            Ok(ext) => ext
                .characters
                .into_iter()
                .map(tvdb_char_to_person)
                .collect(),
            Err(_) => Vec::new(),
        };
        Ok(Some(EpisodeData {
            title: Some(ep.title.clone()).filter(|s| !s.is_empty()),
            summary: ep.summary.clone(),
            runtime_ms: ep.runtime_minutes.map(|m| (m as i64) * 60_000),
            air_date_ms: None,
            still_url: ep.still_url.clone(),
            tmdb_id: None,
            tvdb_id: Some(ep.tvdb_id),
            people,
        }))
    }
}

/// Translate a TVDB character row into the common `PersonCredit`.
/// TVDB's `name` field on a character row is the character name for
/// actors; the agent maps `people_type` into the trait's role taxonomy.
fn tvdb_char_to_person(c: crate::tvdb::TvdbEpisodeCharacter) -> PersonCredit {
    let role = match c.people_type.to_ascii_lowercase().as_str() {
        "actor" => "actor",
        "guest star" | "guest" => "guest",
        "director" => "director",
        "writer" => "writer",
        "producer" | "executive producer" => "producer",
        _ => "crew",
    }
    .to_string();
    PersonCredit {
        external_id: Some(format!("tvdb:{}", c.person_id)),
        name: c.person_name,
        role,
        character: c.character_name,
        order: c.sort,
        profile_url: c.profile_url,
    }
}

// ---------------------------------------------------------------------------
// TVMaze
// ---------------------------------------------------------------------------

pub struct TvMazeAgent {
    client: TvMazeClient,
}

impl TvMazeAgent {
    pub const CAPABILITIES: Capabilities = Capabilities {
        movie: false,
        show: true,
        episode: true,
        cast: false,
        artwork: false,
        ratings: false,
    };
    pub const LIMITATIONS: &'static [&'static str] = &[
        "TV shows only (no movies, no anime tracking).",
        "Stills are 250 × 140 thumbnails — fine for episode rows, \
         noticeably softer than TMDB at the same scale.",
    ];

    pub fn new(client: TvMazeClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl MetadataAgent for TvMazeAgent {
    fn name(&self) -> &'static str {
        "tvmaze"
    }

    fn capabilities(&self) -> Capabilities {
        Self::CAPABILITIES
    }

    fn limitations(&self) -> &'static [&'static str] {
        Self::LIMITATIONS
    }

    async fn fetch_show(&self, lookup: &ShowLookup) -> anyhow::Result<Option<ShowData>> {
        let Some(meta) = self.client.lookup_show(&lookup.title).await? else {
            return Ok(None);
        };
        let mut posters = Vec::new();
        if let Some(url) = &meta.poster_url {
            posters.push(ArtworkVariant {
                url: url.clone(),
                language: None,
                score: 1.0,
            });
        }
        let mut backdrops = Vec::new();
        if let Some(url) = &meta.backdrop_url {
            backdrops.push(ArtworkVariant {
                url: url.clone(),
                language: None,
                score: 1.0,
            });
        }
        Ok(Some(ShowData {
            title: Some(meta.title.clone()),
            year: meta.year,
            summary: meta.summary.clone(),
            imdb_id: meta.imdb_id.clone(),
            tvdb_id: meta.tvdb_id,
            tvmaze_id: Some(meta.tvmaze_id),
            genres: meta.genres.clone(),
            posters,
            backdrops,
            ..Default::default()
        }))
    }

    async fn fetch_episode(&self, lookup: &EpisodeLookup) -> anyhow::Result<Option<EpisodeData>> {
        let Some(show_tvmaze_id) = lookup.show.tvmaze_id else {
            return Ok(None);
        };
        let episodes = self.client.fetch_episodes(show_tvmaze_id).await?;
        let Some(ep) = episodes.iter().find(|e| {
            e.season_number == lookup.season_number && e.episode_number == lookup.episode_number
        }) else {
            return Ok(None);
        };
        Ok(Some(EpisodeData {
            title: Some(ep.title.clone()).filter(|s| !s.is_empty()),
            summary: ep.summary.clone(),
            runtime_ms: ep.runtime_minutes.map(|m| (m as i64) * 60_000),
            air_date_ms: None,
            still_url: ep.still_url.clone(),
            tmdb_id: None,
            tvdb_id: None,
            people: Vec::new(),
        }))
    }
}

// ---------------------------------------------------------------------------
// AniList
// ---------------------------------------------------------------------------

/// AniList agent — anime-only metadata source with a GraphQL backend.
///
/// Holds three per-scan caches (show / episode-list / season-id) so a
/// season of 12 episodes only triggers one `lookup_show` instead of 12
/// parallel calls. Without these caches AniList's free 30 req/min
/// limit gets hit immediately and cascades into 429 errors.
pub struct AniListAgent {
    client: AniListClient,
    show_cache: crate::anilist_cache::AniListShowCacheArc,
    episode_cache: crate::anilist_cache::AniListEpisodeListCacheArc,
    season_id_cache: crate::anilist_cache::AniListSeasonIdCacheArc,
    /// Operator-configured metadata language (BCP-47 like "en-US",
    /// "ja-JP"). Used by [`anilist_show_to_data`] to decide whether to
    /// surface the english / romaji / native title triple. Defaults to
    /// "en-US" if construction skips the with_language constructor.
    language: String,
}

impl AniListAgent {
    pub const CAPABILITIES: Capabilities = Capabilities {
        movie: false,
        show: true,
        episode: true,
        cast: false,
        artwork: true,
        ratings: false,
    };
    pub const LIMITATIONS: &'static [&'static str] = &[
        "Anime libraries only.",
        "Per-episode titles only when AniList has a streamingEpisodes \
         listing (sparse outside popular shows).",
        "No cast/crew exposed via the GraphQL endpoint we hit.",
        "On en-* metadata_language the agent silently skips shows that \
         only have native/romaji titles — TVDB/TMDB get to fill them.",
    ];

    pub fn new(
        client: AniListClient,
        show_cache: crate::anilist_cache::AniListShowCacheArc,
        episode_cache: crate::anilist_cache::AniListEpisodeListCacheArc,
        season_id_cache: crate::anilist_cache::AniListSeasonIdCacheArc,
    ) -> Self {
        Self::with_language(
            client,
            show_cache,
            episode_cache,
            season_id_cache,
            "en-US".to_string(),
        )
    }

    pub fn with_language(
        client: AniListClient,
        show_cache: crate::anilist_cache::AniListShowCacheArc,
        episode_cache: crate::anilist_cache::AniListEpisodeListCacheArc,
        season_id_cache: crate::anilist_cache::AniListSeasonIdCacheArc,
        language: String,
    ) -> Self {
        Self {
            client,
            show_cache,
            episode_cache,
            season_id_cache,
            language,
        }
    }

    /// Find the AniList id for a split-cour season. Returns the
    /// primary id for season 1 (or unspecified); for season > 1 walks
    /// `season_candidate_queries` until a distinct entry is found.
    /// `None` when AniList doesn't disambiguate the show by season —
    /// the caller should skip episode enrichment in that case rather
    /// than mis-assigning season-1 titles to season-2 files.
    async fn resolve_season_anilist_id(
        &self,
        show_id: i64,
        show_title: &str,
        season_number: i32,
        primary_anilist_id: i64,
    ) -> Option<i64> {
        use crate::anilist_cache::CachedAniListSeasonId;
        if season_number <= 1 {
            return Some(primary_anilist_id);
        }
        let key = (show_id, season_number);
        {
            let guard = self.season_id_cache.lock().await;
            if let Some(hit) = guard.get(&key) {
                return match hit {
                    CachedAniListSeasonId::Found(id) => Some(*id),
                    CachedAniListSeasonId::Missing | CachedAniListSeasonId::Errored => None,
                };
            }
        }
        let candidates = season_candidate_queries(show_title, season_number);
        for query in &candidates {
            match self.client.lookup_show(query, None).await {
                Ok(Some(meta)) => {
                    if meta.anilist_id != primary_anilist_id {
                        let id = meta.anilist_id;
                        let mut guard = self.season_id_cache.lock().await;
                        guard.entry(key).or_insert(CachedAniListSeasonId::Found(id));
                        return Some(id);
                    }
                }
                Ok(None) => {}
                Err(_) => {
                    let mut guard = self.season_id_cache.lock().await;
                    guard.entry(key).or_insert(CachedAniListSeasonId::Errored);
                    return None;
                }
            }
        }
        let mut guard = self.season_id_cache.lock().await;
        guard.entry(key).or_insert(CachedAniListSeasonId::Missing);
        None
    }
}

/// Generate ordered candidate query strings for "{title} season N".
/// Most-common AniList convention first so the loop short-circuits
/// quickly. Returns empty for season ≤ 1 — the primary id is already
/// correct.
pub fn season_candidate_queries(show_title: &str, season_number: i32) -> Vec<String> {
    if season_number <= 1 {
        return Vec::new();
    }
    vec![
        format!("{show_title} Season {season_number}"),
        format!("{show_title} {} Season", ordinal_suffix(season_number)),
        format!("{show_title} {season_number}"),
        format!("{show_title} Part {season_number}"),
    ]
}

fn ordinal_suffix(n: i32) -> String {
    let n_abs = n.unsigned_abs();
    let suffix = if (11..=13).contains(&(n_abs % 100)) {
        "th"
    } else {
        match n_abs % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        }
    };
    format!("{n}{suffix}")
}

#[async_trait]
impl MetadataAgent for AniListAgent {
    fn name(&self) -> &'static str {
        "anilist"
    }

    fn capabilities(&self) -> Capabilities {
        Self::CAPABILITIES
    }

    fn limitations(&self) -> &'static [&'static str] {
        Self::LIMITATIONS
    }

    async fn fetch_show(&self, lookup: &ShowLookup) -> anyhow::Result<Option<ShowData>> {
        use crate::anilist_cache::CachedAniListShow;
        let key = (lookup.title.clone(), lookup.year);
        {
            let guard = self.show_cache.lock().await;
            if let Some(hit) = guard.get(&key) {
                return Ok(match hit {
                    CachedAniListShow::Found(arc) => {
                        anilist_show_to_data(arc.as_ref(), &self.language)
                    }
                    CachedAniListShow::Missing | CachedAniListShow::Errored => None,
                });
            }
        }
        match self.client.lookup_show(&lookup.title, lookup.year).await {
            Ok(Some(meta)) => {
                let arc = std::sync::Arc::new(meta);
                {
                    let mut guard = self.show_cache.lock().await;
                    guard
                        .entry(key)
                        .or_insert(CachedAniListShow::Found(arc.clone()));
                }
                Ok(anilist_show_to_data(arc.as_ref(), &self.language))
            }
            Ok(None) => {
                let mut guard = self.show_cache.lock().await;
                guard.entry(key).or_insert(CachedAniListShow::Missing);
                Ok(None)
            }
            Err(e) => {
                let mut guard = self.show_cache.lock().await;
                guard.entry(key).or_insert(CachedAniListShow::Errored);
                Err(e)
            }
        }
    }

    async fn fetch_episode(&self, lookup: &EpisodeLookup) -> anyhow::Result<Option<EpisodeData>> {
        use crate::anilist_cache::CachedAniListEpisodes;
        let Some(primary_anilist_id) = lookup.show.anilist_id else {
            return Ok(None);
        };
        let Some(anilist_id) = self
            .resolve_season_anilist_id(
                lookup.show.item_id,
                &lookup.show.title,
                lookup.season_number,
                primary_anilist_id,
            )
            .await
        else {
            return Ok(None);
        };
        // Fetch (or cache-hit) the streamingEpisodes list.
        // Hold the lock across the network call so concurrent callers for the
        // same anilist_id don't each fire a redundant fetch (AniList free tier:
        // 30 req/min). tokio::sync::Mutex is async-aware so holding it across
        // an `.await` is safe and won't block the executor thread.
        let episodes = {
            let mut guard = self.episode_cache.lock().await;
            match guard.get(&anilist_id) {
                Some(CachedAniListEpisodes::Loaded(eps)) => eps.clone(),
                Some(CachedAniListEpisodes::Errored) => return Ok(None),
                None => {
                    match self.client.fetch_episodes(anilist_id).await {
                        Ok(list) => {
                            let arc = std::sync::Arc::new(list);
                            guard.insert(anilist_id, CachedAniListEpisodes::Loaded(arc.clone()));
                            arc
                        }
                        Err(e) => {
                            guard.insert(anilist_id, CachedAniListEpisodes::Errored);
                            return Err(e);
                        }
                    }
                }
            }
        };
        if episodes.is_empty() {
            return Ok(None);
        }
        let Some(ep) = episodes
            .iter()
            .find(|e| e.episode_number == lookup.episode_number)
        else {
            return Ok(None);
        };
        if !ep.has_descriptive_title() {
            // "Episode 7" titles are no better than the filename stem
            // and would mask a future better-source enrichment.
            return Ok(None);
        }
        if self.language.to_ascii_lowercase().starts_with("en") && !title_looks_english(&ep.title) {
            // Operator picked en-* but AniList's streamingEpisodes
            // listing for this episode is in Japanese (or romaji that
            // didn't survive the heuristic). Skip so TVDB/TMDB get to
            // populate the per-episode title.
            return Ok(None);
        }
        Ok(Some(EpisodeData {
            title: Some(ep.title.clone()),
            summary: None,
            runtime_ms: None,
            air_date_ms: None,
            still_url: ep.thumbnail_url.clone(),
            tmdb_id: None,
            tvdb_id: None,
            people: Vec::new(),
        }))
    }
}

/// Heuristic: does this title look like English text?
///
/// AniList's `streamingEpisodes` payload mixes English titles ("End and
/// Beginning") with romaji ("Hajimari to Owari") and occasionally raw
/// Japanese characters. We don't have per-locale fields in that payload
/// so we infer from the script: a title is considered English when at
/// least half its alphabetic characters are ASCII letters. Tolerates
/// stray Japanese punctuation, em-dashes, etc.
fn title_looks_english(title: &str) -> bool {
    let mut ascii_alpha = 0usize;
    let mut total_alpha = 0usize;
    for ch in title.chars() {
        if ch.is_alphabetic() {
            total_alpha += 1;
            if ch.is_ascii_alphabetic() {
                ascii_alpha += 1;
            }
        }
    }
    if total_alpha == 0 {
        return false;
    }
    // 50% threshold catches romaji ("Hajimari to Owari") and English
    // titles while rejecting "オーバーロード" and mixed-script titles
    // like "オーバーロード IV".
    ascii_alpha * 2 >= total_alpha
}

/// Translate an [`AniListShow`] into the trait-shaped [`ShowData`],
/// honoring the operator's `metadata_language` preference.
///
/// **Returns `None` when the agent should silently bail out** for this
/// show — currently when the operator picked an `en-*` language and
/// AniList has no English title for the show. In that case TVDB/TMDB
/// get a clean shot at writing the title without AniList overwriting
/// it with a romaji/native fallback first.
///
/// On non-en-* locales the cascade is unchanged: english → romaji →
/// native, which is what AniList itself does in its desktop UI.
fn anilist_show_to_data(meta: &crate::anilist::AniListShow, language: &str) -> Option<ShowData> {
    let title = pick_anilist_title(meta, language)?;
    let mut posters = Vec::new();
    if let Some(url) = &meta.poster_url {
        posters.push(ArtworkVariant {
            url: url.clone(),
            language: None,
            score: 1.0,
        });
    }
    let mut backdrops = Vec::new();
    if let Some(url) = &meta.backdrop_url {
        backdrops.push(ArtworkVariant {
            url: url.clone(),
            language: None,
            score: 1.0,
        });
    }
    Some(ShowData {
        title: Some(title),
        original_title: meta.original_title.clone(),
        year: meta.year,
        summary: meta.summary.clone(),
        anilist_id: Some(meta.anilist_id),
        genres: meta.genres.clone(),
        posters,
        backdrops,
        ..Default::default()
    })
}

/// Pick the AniList title to surface for a given BCP-47 language tag.
/// Returns `None` when the agent should refuse to surface a fallback —
/// today that's specifically en-* locales without an english title.
fn pick_anilist_title(meta: &crate::anilist::AniListShow, language: &str) -> Option<String> {
    let lang_lc = language.to_ascii_lowercase();
    if lang_lc.starts_with("en") {
        // English-preferring operators: only surface AniList data when
        // AniList itself has an English title. Otherwise let TVDB/TMDB
        // populate the row.
        meta.english_title.clone()
    } else if lang_lc.starts_with("ja") {
        meta.original_title
            .clone()
            .or_else(|| meta.romaji_title.clone())
            .or_else(|| meta.english_title.clone())
    } else {
        // Anything else: use AniList's normal cascade (already in
        // meta.title — english → romaji → native).
        Some(meta.title.clone())
    }
}

// ---------------------------------------------------------------------------
// OMDb
// ---------------------------------------------------------------------------

/// OMDb agent — primarily an IMDb-id-driven ratings supplement, also
/// usable as a fallback metadata source via its `?t=...&type=...` and
/// `?i=...&Season=&Episode=` endpoints. Free tier is 1k requests/day,
/// so this is best placed at the END of the chain (fill-nulls only)
/// rather than as the primary agent for big libraries.
pub struct OmdbAgent {
    client: OmdbClient,
}

impl OmdbAgent {
    pub const CAPABILITIES: Capabilities = Capabilities {
        movie: true,
        show: true,
        episode: true,
        cast: false,
        artwork: true,
        ratings: true,
    };
    pub const LIMITATIONS: &'static [&'static str] = &[
        "Free tier capped at 1,000 requests/day. A 5,000-movie cold \
         scan will hit the quota and stop — keep OMDb late in the \
         chain (fill-nulls only) for big libraries.",
        "Episode lookup requires the show's IMDb id; a prior agent \
         (TMDB / TVDB) must run first to supply it.",
        "Cast is exposed as a comma-joined string, not structured \
         person rows; not surfaced through the trait until Slice 6.",
    ];

    pub fn new(client: OmdbClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl MetadataAgent for OmdbAgent {
    fn name(&self) -> &'static str {
        "omdb"
    }

    fn capabilities(&self) -> Capabilities {
        Self::CAPABILITIES
    }

    fn limitations(&self) -> &'static [&'static str] {
        Self::LIMITATIONS
    }

    async fn fetch_movie(&self, lookup: &MovieLookup) -> anyhow::Result<Option<MovieData>> {
        let Some(meta) = self.client.lookup_movie(&lookup.title, lookup.year).await? else {
            return Ok(None);
        };
        Ok(Some(omdb_title_to_movie(meta)))
    }

    async fn fetch_show(&self, lookup: &ShowLookup) -> anyhow::Result<Option<ShowData>> {
        let Some(meta) = self.client.lookup_show(&lookup.title, lookup.year).await? else {
            return Ok(None);
        };
        Ok(Some(omdb_title_to_show(meta)))
    }

    async fn fetch_episode(&self, lookup: &EpisodeLookup) -> anyhow::Result<Option<EpisodeData>> {
        // OMDb keys episodes off the SHOW's IMDb id, not its own.
        // A prior agent in the chain must populate `imdb_id` on the
        // show; without one we can't address the episode.
        let Some(imdb_id) = lookup.show.imdb_id.as_deref() else {
            return Ok(None);
        };
        let Some(meta) = self
            .client
            .fetch_episode(imdb_id, lookup.season_number, lookup.episode_number)
            .await?
        else {
            return Ok(None);
        };
        Ok(Some(EpisodeData {
            title: meta.title,
            summary: meta.summary,
            runtime_ms: meta.runtime_minutes.map(|m| (m as i64) * 60_000),
            air_date_ms: None,
            still_url: meta.poster_url,
            tmdb_id: None,
            tvdb_id: None,
            people: Vec::new(),
        }))
    }
}

fn omdb_title_to_movie(meta: crate::omdb::OmdbTitle) -> MovieData {
    let mut posters = Vec::new();
    if let Some(url) = &meta.poster_url {
        posters.push(ArtworkVariant {
            url: url.clone(),
            language: None,
            score: 0.7, // OMDb posters tend to be lower-res than TMDB's
        });
    }
    MovieData {
        title: meta.title,
        year: meta.year,
        summary: meta.summary,
        runtime_ms: meta.runtime_minutes.map(|m| (m as i64) * 60_000),
        imdb_id: meta.imdb_id,
        rating_audience: meta.metascore,
        genres: meta.genres,
        posters,
        ..Default::default()
    }
}

fn omdb_title_to_show(meta: crate::omdb::OmdbTitle) -> ShowData {
    let mut posters = Vec::new();
    if let Some(url) = &meta.poster_url {
        posters.push(ArtworkVariant {
            url: url.clone(),
            language: None,
            score: 0.7,
        });
    }
    ShowData {
        title: meta.title,
        year: meta.year,
        summary: meta.summary,
        imdb_id: meta.imdb_id,
        genres: meta.genres,
        posters,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::{ordinal_suffix, season_candidate_queries};

    #[test]
    fn ordinal_basic_cases() {
        assert_eq!(ordinal_suffix(1), "1st");
        assert_eq!(ordinal_suffix(2), "2nd");
        assert_eq!(ordinal_suffix(3), "3rd");
        assert_eq!(ordinal_suffix(4), "4th");
        assert_eq!(ordinal_suffix(10), "10th");
    }

    #[test]
    fn ordinal_teens_use_th() {
        assert_eq!(ordinal_suffix(11), "11th");
        assert_eq!(ordinal_suffix(12), "12th");
        assert_eq!(ordinal_suffix(13), "13th");
    }

    #[test]
    fn ordinal_resumes_after_teens() {
        assert_eq!(ordinal_suffix(21), "21st");
        assert_eq!(ordinal_suffix(22), "22nd");
        assert_eq!(ordinal_suffix(23), "23rd");
    }

    #[test]
    fn season_candidates_empty_for_season_one_and_below() {
        assert!(season_candidate_queries("Frieren", 0).is_empty());
        assert!(season_candidate_queries("Frieren", 1).is_empty());
    }

    #[test]
    fn season_candidates_for_season_two_anime() {
        let qs = season_candidate_queries("Jujutsu Kaisen", 2);
        assert_eq!(
            qs,
            vec![
                "Jujutsu Kaisen Season 2".to_string(),
                "Jujutsu Kaisen 2nd Season".to_string(),
                "Jujutsu Kaisen 2".to_string(),
                "Jujutsu Kaisen Part 2".to_string(),
            ]
        );
    }
}

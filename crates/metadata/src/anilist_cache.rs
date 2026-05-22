//! Per-scan caches for AniList lookups.
//!
//! AniList's free tier is 30 requests/minute. Without these caches a
//! season of 12 episodes triggers 12 parallel `lookup_show` calls for
//! the same `(title, year)` — guaranteed to hit 429 and cascade
//! errors across the whole scan. The caches memoize per-show lookups
//! so the bulk-scan load profile becomes "one lookup per show + one
//! `streamingEpisodes` fetch per show id" regardless of episode count.
//!
//! The caches live in the metadata crate so [`crate::AniListAgent`]
//! can own them as fields, fulfilling the `MetadataAgent` trait
//! without forcing the scanner to plumb them through every call site.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::anilist::{AniListEpisode, AniListShow};

/// Per-scan cache for AniList `lookup_show` keyed by
/// `(normalized_title, year)`. Year is in the key because AniList
/// disambiguates by year and a different year could legitimately
/// resolve to a different show.
pub type AniListShowCache = Mutex<HashMap<(String, Option<i32>), CachedAniListShow>>;
pub type AniListShowCacheArc = Arc<AniListShowCache>;

#[derive(Clone)]
pub enum CachedAniListShow {
    Found(Arc<AniListShow>),
    /// AniList returned `Ok(None)` — confirmed-missing match.
    Missing,
    /// AniList errored (rate-limit / network / parse). Cached for the
    /// scan to avoid hammering the API; future scans retry from scratch.
    Errored,
}

/// Per-scan cache for AniList episode lookups, keyed by `anilist_id`.
/// Each entry holds the parsed `streamingEpisodes` list for one show.
pub type AniListEpisodeListCache = Mutex<HashMap<i64, CachedAniListEpisodes>>;
pub type AniListEpisodeListCacheArc = Arc<AniListEpisodeListCache>;

#[derive(Clone)]
pub enum CachedAniListEpisodes {
    /// Episode list for this anilist_id, possibly empty (the show
    /// exists but AniList has no streamingEpisodes — common for older
    /// shows or anything region-blocked from AniList's stream scrapers).
    Loaded(Arc<Vec<AniListEpisode>>),
    Errored,
}

/// Per-scan cache for "this show, this season number → which AniList id?"
/// resolution. Anime that's split-cour or split-season on AniList lives
/// under a distinct id per season — Jujutsu Kaisen S2 is `145064`, not
/// a second season of `113415`. Without season-aware lookup, every S2+
/// file either gets no AniList enrichment or (worse) gets episode 1 of
/// season 1's titles assigned to season 2's episode 1.
///
/// Keyed by `(show_id, season_number)` so two locally-distinct shows
/// with similar titles don't pollute each other's cache.
pub type AniListSeasonIdCache = Mutex<HashMap<(i64, i32), CachedAniListSeasonId>>;
pub type AniListSeasonIdCacheArc = Arc<AniListSeasonIdCache>;

#[derive(Clone, Copy)]
pub enum CachedAniListSeasonId {
    Found(i64),
    /// Tried every candidate and either got no match or every match
    /// returned the primary anilist_id. Cached as the "skip episode
    /// enrichment for this season" signal — falling back to the primary
    /// id would mis-assign S1 titles to S2 files.
    Missing,
    Errored,
}

pub fn new_show_cache() -> AniListShowCacheArc {
    Arc::new(Mutex::new(HashMap::new()))
}

pub fn new_episode_list_cache() -> AniListEpisodeListCacheArc {
    Arc::new(Mutex::new(HashMap::new()))
}

pub fn new_season_id_cache() -> AniListSeasonIdCacheArc {
    Arc::new(Mutex::new(HashMap::new()))
}

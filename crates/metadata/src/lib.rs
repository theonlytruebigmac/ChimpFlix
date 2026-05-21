//! Metadata agents.
//!
//! TMDB is the primary identifier and source of truth for credits, extras,
//! and reviews. Other agents (TVMaze, TVDB; AniList queued for anime) run
//! after TMDB to fill nulls without overwriting — see scanner enrichment
//! for the merge policy.
//!
//! The trait abstraction is still deferred because every provider has
//! enough unique surface (Fix Match candidate listing, /credits, /videos,
//! /reviews — only TMDB has all four) that a uniform trait would push the
//! complexity into the callers instead of removing it.

pub mod anilist;
pub mod omdb;
pub mod opensubtitles;
pub mod tmdb;
pub mod trakt;
pub mod tvdb;
pub mod tvmaze;

pub use anilist::{AniListClient, AniListShow};
pub use omdb::{OmdbClient, OmdbRatings};
pub use opensubtitles::{OpenSubtitlesClient, OpenSubtitlesCreds, SearchParams, SubtitleHit};
pub use tmdb::{
    TmdbCandidate, TmdbCastMember, TmdbClient, TmdbCollection, TmdbCollectionPart,
    TmdbCollectionStub, TmdbCredits, TmdbCrewMember, TmdbEpisode, TmdbKind, TmdbMovie, TmdbPoster,
    TmdbReview, TmdbSeason, TmdbShow, TmdbUpstreamError, TmdbVideo, tmdb_image_url,
};
pub use trakt::{
    DeviceCodeResponse, DevicePollResult, HistoryEntry, HistoryPush, PlaybackEntry, RatingEntry,
    RatingPush, TokenPair, TraktClient, TraktCreds, TraktIds,
};
pub use tvdb::{TvdbClient, TvdbMovie, TvdbShow};
pub use tvmaze::{TvMazeClient, TvMazeShow};

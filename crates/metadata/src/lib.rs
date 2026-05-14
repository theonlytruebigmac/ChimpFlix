//! Metadata agents.
//!
//! TMDB is the primary identifier and source of truth for credits, extras,
//! and reviews. Other agents (TVMaze first; Wikidata / AniList queued) run
//! after TMDB to fill nulls without overwriting — see scanner enrichment
//! for the merge policy.
//!
//! The trait abstraction is still deferred because every provider has
//! enough unique surface (Fix Match candidate listing, /credits, /videos,
//! /reviews — only TMDB has all four) that a uniform trait would push the
//! complexity into the callers instead of removing it.

pub mod tmdb;
pub mod tvmaze;

pub use tmdb::{
    TmdbCandidate, TmdbCastMember, TmdbClient, TmdbCollection, TmdbCollectionPart,
    TmdbCollectionStub, TmdbCredits, TmdbCrewMember, TmdbEpisode, TmdbKind, TmdbMovie, TmdbPoster,
    TmdbReview, TmdbSeason, TmdbShow, TmdbVideo, tmdb_image_url,
};
pub use tvmaze::{TvMazeClient, TvMazeShow};

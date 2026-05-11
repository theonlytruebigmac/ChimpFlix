//! Metadata agents.
//!
//! v0.1 ships a single TMDB agent. The trait abstraction is deliberately
//! deferred until a second agent (TVDB, AniDB, ...) actually appears —
//! see docs/ARCHITECTURE.md for the rationale.

pub mod tmdb;

pub use tmdb::{TmdbClient, TmdbEpisode, TmdbMovie, TmdbSeason, TmdbShow, tmdb_image_url};

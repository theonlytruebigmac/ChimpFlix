//! Metadata agents.
//!
//! Each provider implements the [`agent::MetadataAgent`] trait so the
//! scanner can hold a heterogeneous chain and ask every capable agent
//! for the metadata it can supply. The first agent in the chain runs
//! in `WriteMode::Primary` (data takes precedence); later agents fill
//! nulls. See `agent.rs` for the contract.
//!
//! Provider-specific clients (TmdbClient, TvdbClient, ...) remain
//! exported for one-off uses outside the chain (admin "Fix Match"
//! candidate search hits TMDB directly, for example).

pub mod agent;
pub mod agents;
pub mod anilist;
pub mod anilist_cache;
pub mod omdb;
pub mod opensubtitles;
pub mod tmdb;
pub mod trakt;
pub mod tvdb;
pub mod tvmaze;

pub use agent::{
    ArtworkVariant, Capabilities, EpisodeData, EpisodeLookup, ExternalRating, MetadataAgent,
    MovieData, MovieLookup, PersonCredit, ReviewEntry, SeasonEpisodeCount, ShowData, ShowLookup,
    TmdbCollectionRef, VideoLink, WriteMode,
};
pub use agents::{AniListAgent, OmdbAgent, TmdbAgent, TvMazeAgent, TvdbAgent};
pub use anilist_cache::{
    AniListEpisodeListCache, AniListEpisodeListCacheArc, AniListSeasonIdCache,
    AniListSeasonIdCacheArc, AniListShowCache, AniListShowCacheArc,
};

pub use anilist::{AniListClient, AniListEpisode, AniListShow};
pub use omdb::{OmdbClient, OmdbRatings, OmdbTitle};
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
pub use tvdb::{
    TvdbClient, TvdbEpisode, TvdbEpisodeCharacter, TvdbEpisodeExtended, TvdbMovie, TvdbShow,
};
pub use tvmaze::{TvMazeClient, TvMazeEpisode, TvMazeShow};

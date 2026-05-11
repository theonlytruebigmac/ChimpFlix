//! Library subsystem: filesystem scanner, SQLite schema, and library DB
//! accessors.

pub mod db;
pub mod events;
pub mod models;
pub mod parser;
pub mod queries;
pub mod scanner;

pub use db::open;
pub use events::{ScanEmitter, ScanEvent};
pub use models::*;
pub use scanner::run_scan;

pub use crate::models::{
    CreateInviteInput, Episode, EpisodeDetail, EpisodeListed, Invite, ItemDetail, ListedItem,
    LoginInput, MediaFileLocator, MediaFileSummary, MediaStreamSummary, OnDeckEntry,
    OnDeckResponse, PlayStateBatch, PlayStateForItem, PlayStateUpdate, RegisterInput,
    ScrobbleRequest, Season, SeasonDetail, SeasonSummary, SessionRow, SetupInput, User, UserRole,
    UserWithSecret,
};

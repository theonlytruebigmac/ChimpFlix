//! Library subsystem: filesystem scanner, SQLite schema, and library DB
//! accessors.

pub mod db;
pub mod events;
pub mod models;
pub mod parser;
pub mod queries;
pub mod scanner;
pub mod smart_rule;

pub use db::{open, open_with};
pub use events::{ScanEmitter, ScanEvent};
pub use models::*;
pub use scanner::run_scan;

pub use crate::models::{
    AgentInfo, AuditLogEntry, CreateInviteInput, Episode, EpisodeDetail, EpisodeListed, Invite,
    ItemDetail, LibraryAgent, ListedItem, LoginInput, MediaFileLocator, MediaFileSummary,
    MediaStreamSummary, NewAuditEntry, NewOptimizedVersion, NewScheduledTask, NewTranscoderPreset,
    NewWebhook, OnDeckEntry, OnDeckResponse, OptimizedVersion, PlayStateBatch, PlayStateForItem,
    PlayStateUpdate, RegisterInput, ScanJob, ScheduledTask, ScheduledTaskUpdate, ScrobbleRequest,
    Season, SeasonDetail, SeasonSummary, SecretMetadata, ServerSettings, ServerSettingsUpdate,
    SessionRow, SetupInput, TaskRun, TranscoderPreset, TranscoderPresetUpdate, User, UserRole,
    UserWithSecret, Webhook, WebhookDelivery, WebhookUpdate,
};
pub use crate::queries::{AccessMatrixEntry, LibraryStats, OnDeckOptions, SessionSummary, Tag};

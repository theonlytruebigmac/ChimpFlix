//! Row-oriented and DTO types for the library subsystem.

use serde::{Deserialize, Serialize};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LibraryKind {
    Movies,
    Shows,
    /// Anime libraries are show-shaped (one item = one series, with
    /// seasons + episodes underneath) but use a different parser path —
    /// fansub-style filenames + absolute episode numbering — and are
    /// expected to be enriched by an anime-aware agent (AniList) once it
    /// ships. The `item_kind()` mapping still resolves to `Show` so the
    /// existing read APIs and player code stay agnostic.
    Anime,
}

impl LibraryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Movies => "movies",
            Self::Shows => "shows",
            Self::Anime => "anime",
        }
    }

    pub fn from_db(s: &str) -> anyhow::Result<Self> {
        match s {
            "movies" => Ok(Self::Movies),
            "shows" => Ok(Self::Shows),
            "anime" => Ok(Self::Anime),
            other => anyhow::bail!("unknown library kind: {other}"),
        }
    }

    pub fn item_kind(&self) -> ItemKind {
        match self {
            Self::Movies => ItemKind::Movie,
            Self::Shows | Self::Anime => ItemKind::Show,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    Movie,
    Show,
}

impl ItemKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Show => "show",
        }
    }

    pub fn from_db(s: &str) -> anyhow::Result<Self> {
        match s {
            "movie" => Ok(Self::Movie),
            "show" => Ok(Self::Show),
            other => anyhow::bail!("unknown item kind: {other}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScanStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Canceled,
}

impl ScanStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }
}

// ---------------------------------------------------------------------------
// Libraries
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Library {
    pub id: i64,
    pub name: String,
    pub kind: LibraryKind,
    pub paths: Vec<String>,
    pub scan_interval_s: i64,
    pub last_scan_at: Option<i64>,
    /// Episode sort order. Plex-equivalent. One of: "oldest_first" |
    /// "newest_first".
    pub episode_sort_order: String,
    /// Episode naming source. One of: "tmdb" | "original" | "absolute".
    pub episode_naming: String,
    /// ISO 3166-1 alpha-2 country code that selects which certification
    /// system is shown (e.g. "US" → MPAA, "GB" → BBFC).
    pub certification_country: String,
    /// Where this library appears. One of: "home_and_search" | "search_only"
    /// | "hidden".
    pub visibility: String,
    /// When true, the item detail modal exposes a "Delete from disk"
    /// button that hard-deletes the media file (and cascades orphan
    /// rows) immediately, no grace window. Default false so a casual
    /// operator can't blow away a library by clicking the wrong button.
    pub allow_media_deletion: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewLibrary {
    pub name: String,
    pub kind: LibraryKind,
    pub paths: Vec<String>,
    #[serde(default)]
    pub scan_interval_s: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LibraryUpdate {
    pub name: Option<String>,
    pub paths: Option<Vec<String>>,
    pub scan_interval_s: Option<i64>,
    pub episode_sort_order: Option<String>,
    pub episode_naming: Option<String>,
    pub certification_country: Option<String>,
    pub visibility: Option<String>,
    pub allow_media_deletion: Option<bool>,
}

impl Library {
    pub(crate) fn from_row(row: &SqliteRow, paths: Vec<String>) -> anyhow::Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            kind: LibraryKind::from_db(row.try_get::<&str, _>("kind")?)?,
            paths,
            scan_interval_s: row.try_get("scan_interval_s")?,
            last_scan_at: row.try_get("last_scan_at")?,
            episode_sort_order: row
                .try_get::<String, _>("episode_sort_order")
                .unwrap_or_else(|_| "oldest_first".to_string()),
            episode_naming: row
                .try_get::<String, _>("episode_naming")
                .unwrap_or_else(|_| "tmdb".to_string()),
            certification_country: row
                .try_get::<String, _>("certification_country")
                .unwrap_or_else(|_| "US".to_string()),
            visibility: row
                .try_get::<String, _>("visibility")
                .unwrap_or_else(|_| "home_and_search".to_string()),
            allow_media_deletion: row
                .try_get::<Option<i64>, _>("allow_media_deletion")
                .ok()
                .flatten()
                .unwrap_or(0)
                != 0,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

// ---------------------------------------------------------------------------
// Per-library metadata agent chain
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryAgent {
    pub agent_name: String,
    pub priority: i64,
    pub enabled: bool,
    /// JSON-encoded agent-specific config (region code, include-adult, etc.).
    pub config_json: String,
}

impl LibraryAgent {
    pub(crate) fn from_row(row: &SqliteRow) -> anyhow::Result<Self> {
        Ok(Self {
            agent_name: row.try_get("agent_name")?,
            priority: row.try_get("priority")?,
            enabled: row.try_get::<i64, _>("enabled")? != 0,
            config_json: row.try_get("config_json")?,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentInfo {
    pub name: String,
    pub display_name: String,
    /// Item kinds this agent can produce metadata for.
    pub supported_kinds: Vec<String>,
    /// Whether the agent requires extra setup (e.g. TMDB needs a token).
    pub configured: bool,
}

// ---------------------------------------------------------------------------
// Scheduled tasks
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ScheduledTask {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub cron_expr: String,
    /// Friendly schedule label. When != "custom", `cron_expr` is ignored
    /// at runtime and the scheduler computes `next_run_at` from
    /// `frequency + last_finished_at`. See the phase 29 migration for
    /// the supported values.
    pub frequency: String,
    /// When true, the computed `next_run_at` is snapped forward to the
    /// next opening of the server's maintenance window (see
    /// `ServerSettings::maintenance_window_*`). Heavy tasks like full
    /// scans default to true so they don't run during prime-time
    /// playback hours.
    pub requires_maintenance_window: bool,
    pub params_json: String,
    pub enabled: bool,
    pub last_run_at: Option<i64>,
    pub last_status: Option<String>,
    pub last_error: Option<String>,
    pub last_duration_ms: Option<i64>,
    pub next_run_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl ScheduledTask {
    pub(crate) fn from_row(row: &SqliteRow) -> anyhow::Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            kind: row.try_get("kind")?,
            name: row.try_get("name")?,
            cron_expr: row.try_get("cron_expr")?,
            frequency: row
                .try_get::<Option<String>, _>("frequency")
                .ok()
                .flatten()
                .unwrap_or_else(|| "custom".to_string()),
            requires_maintenance_window: row
                .try_get::<Option<i64>, _>("requires_maintenance_window")
                .ok()
                .flatten()
                .unwrap_or(0)
                != 0,
            params_json: row.try_get("params_json")?,
            enabled: row.try_get::<i64, _>("enabled")? != 0,
            last_run_at: row.try_get::<Option<i64>, _>("last_run_at").ok().flatten(),
            last_status: row
                .try_get::<Option<String>, _>("last_status")
                .ok()
                .flatten(),
            last_error: row.try_get::<Option<String>, _>("last_error").ok().flatten(),
            last_duration_ms: row
                .try_get::<Option<i64>, _>("last_duration_ms")
                .ok()
                .flatten(),
            next_run_at: row.try_get("next_run_at")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NewScheduledTask {
    pub kind: String,
    pub name: String,
    /// Optional. When `frequency` is `custom`, this MUST be a valid 5-/
    /// 6-/7-field cron expression and is the source of truth. Otherwise
    /// it's preserved verbatim but ignored by the scheduler.
    #[serde(default = "default_cron_expr")]
    pub cron_expr: String,
    /// One of: manual | hourly | every_3_hours | every_6_hours |
    /// every_12_hours | daily | every_3_days | weekly | monthly |
    /// on_change | custom. Defaults to `custom` for back-compat with
    /// the previous cron-only API.
    #[serde(default = "default_frequency")]
    pub frequency: String,
    #[serde(default)]
    pub requires_maintenance_window: bool,
    #[serde(default = "default_params_json")]
    pub params_json: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_params_json() -> String {
    "{}".to_string()
}
fn default_enabled() -> bool {
    true
}
fn default_frequency() -> String {
    "custom".to_string()
}
fn default_cron_expr() -> String {
    // Hourly. Only consulted when `frequency = custom`; everything else
    // ignores cron_expr at runtime, but we need a value to satisfy the
    // NOT NULL column constraint.
    "0 0 * * * *".to_string()
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ScheduledTaskUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron_expr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_maintenance_window: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

// ---------------------------------------------------------------------------
// Optimized versions (pre-transcoded media)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct OptimizedVersion {
    pub id: i64,
    pub source_file_id: i64,
    pub preset_id: i64,
    pub output_path: String,
    pub output_size_bytes: Option<i64>,
    pub duration_ms: Option<i64>,
    /// queued | running | success | failed
    pub status: String,
    pub error: Option<String>,
    pub created_at: i64,
    pub completed_at: Option<i64>,
}

impl OptimizedVersion {
    pub(crate) fn from_row(row: &SqliteRow) -> anyhow::Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            source_file_id: row.try_get("source_file_id")?,
            preset_id: row.try_get("preset_id")?,
            output_path: row.try_get("output_path")?,
            output_size_bytes: row
                .try_get::<Option<i64>, _>("output_size_bytes")
                .ok()
                .flatten(),
            duration_ms: row.try_get::<Option<i64>, _>("duration_ms").ok().flatten(),
            status: row.try_get("status")?,
            error: row.try_get::<Option<String>, _>("error").ok().flatten(),
            created_at: row.try_get("created_at")?,
            completed_at: row.try_get::<Option<i64>, _>("completed_at").ok().flatten(),
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NewOptimizedVersion {
    pub source_file_id: i64,
    pub preset_id: i64,
}

// ---------------------------------------------------------------------------
// Webhooks
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Webhook {
    pub id: i64,
    pub name: String,
    pub url: String,
    /// Decrypted signing secret. NEVER serialized — admin GETs would
    /// otherwise hand back the live secret in JSON, defeating the
    /// at-rest encryption. The presence flag + last-4 view below is
    /// what the admin UI consumes.
    #[serde(skip)]
    pub secret: Option<String>,
    /// True when a signing secret is configured. Safe to expose; only
    /// reveals "is this hook signed at all", not the value.
    #[serde(rename = "has_secret")]
    pub has_secret_serialized: bool,
    /// Last 4 chars of the secret for the admin UI's "ends-in" hint
    /// (so the operator can verify they pasted the right value into
    /// a webhook receiver). Hidden in JSON unless we actually have a
    /// secret to truncate.
    #[serde(rename = "secret_last4", skip_serializing_if = "Option::is_none")]
    pub secret_last4_serialized: Option<String>,
    /// JSON-encoded array of event names. The dispatcher filters by this
    /// before delivering to each subscriber.
    pub event_mask: String,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Webhook {
    /// Build a `Webhook` from a DB row, decrypting `secret_enc` via the
    /// vault. Falls back to the legacy plaintext `secret` column if the
    /// row hasn't been backfilled yet — the startup task in main.rs
    /// migrates legacy rows on every boot until they're all converted.
    pub(crate) fn from_row(
        row: &SqliteRow,
        vault: &chimpflix_common::Vault,
    ) -> anyhow::Result<Self> {
        let enc_bytes: Option<Vec<u8>> =
            row.try_get::<Option<Vec<u8>>, _>("secret_enc").ok().flatten();
        let secret = if let Some(value) = enc_bytes {
            let nonce: Option<Vec<u8>> =
                row.try_get::<Option<Vec<u8>>, _>("secret_nonce").ok().flatten();
            let blob = chimpflix_common::EncryptedBlob { value, nonce };
            Some(vault.decrypt_str(&blob)?)
        } else {
            row.try_get::<Option<String>, _>("secret").ok().flatten()
        };
        let has_secret = secret.is_some();
        let secret_last4 = secret.as_deref().map(|s| {
            let n = s.chars().count();
            if n <= 4 {
                "****".to_string()
            } else {
                s.chars().skip(n - 4).collect::<String>()
            }
        });
        Ok(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            url: row.try_get("url")?,
            secret,
            has_secret_serialized: has_secret,
            secret_last4_serialized: secret_last4,
            event_mask: row.try_get("event_mask")?,
            enabled: row.try_get::<i64, _>("enabled")? != 0,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NewWebhook {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub secret: Option<String>,
    pub event_mask: Vec<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct WebhookUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_some"
    )]
    pub secret: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_mask: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WebhookDelivery {
    pub id: i64,
    pub webhook_id: i64,
    pub event: String,
    pub payload_json: String,
    pub status_code: Option<i64>,
    pub response_body: Option<String>,
    pub error: Option<String>,
    pub attempts: i64,
    pub next_retry_at: Option<i64>,
    pub delivered_at: Option<i64>,
    pub created_at: i64,
}

impl WebhookDelivery {
    pub(crate) fn from_row(row: &SqliteRow) -> anyhow::Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            webhook_id: row.try_get("webhook_id")?,
            event: row.try_get("event")?,
            payload_json: row.try_get("payload_json")?,
            status_code: row.try_get::<Option<i64>, _>("status_code").ok().flatten(),
            response_body: row
                .try_get::<Option<String>, _>("response_body")
                .ok()
                .flatten(),
            error: row.try_get::<Option<String>, _>("error").ok().flatten(),
            attempts: row.try_get("attempts")?,
            next_retry_at: row.try_get::<Option<i64>, _>("next_retry_at").ok().flatten(),
            delivered_at: row.try_get::<Option<i64>, _>("delivered_at").ok().flatten(),
            created_at: row.try_get("created_at")?,
        })
    }
}

// ---------------------------------------------------------------------------
// Transcoder presets
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct TranscoderPreset {
    pub id: i64,
    pub name: String,
    /// 0 means "no cap" (passthrough bitrate).
    pub max_video_bitrate_kbps: i64,
    /// 0 means "no cap" (original resolution).
    pub max_height: i64,
    pub audio_codec: String,
    pub audio_bitrate_kbps: i64,
    pub enabled: bool,
    pub sort_order: i64,
}

impl TranscoderPreset {
    pub(crate) fn from_row(row: &SqliteRow) -> anyhow::Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            max_video_bitrate_kbps: row.try_get("max_video_bitrate_kbps")?,
            max_height: row.try_get("max_height")?,
            audio_codec: row.try_get("audio_codec")?,
            audio_bitrate_kbps: row.try_get("audio_bitrate_kbps")?,
            enabled: row.try_get::<i64, _>("enabled")? != 0,
            sort_order: row.try_get("sort_order")?,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NewTranscoderPreset {
    pub name: String,
    pub max_video_bitrate_kbps: i64,
    pub max_height: i64,
    #[serde(default = "default_audio_codec")]
    pub audio_codec: String,
    #[serde(default = "default_audio_bitrate")]
    pub audio_bitrate_kbps: i64,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub sort_order: i64,
}

fn default_audio_codec() -> String {
    "aac".to_string()
}
fn default_audio_bitrate() -> i64 {
    192
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct TranscoderPresetUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_video_bitrate_kbps: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_height: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_bitrate_kbps: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskRun {
    pub id: i64,
    pub task_id: i64,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub status: String,
    pub error: Option<String>,
    pub log: Option<String>,
}

impl TaskRun {
    pub(crate) fn from_row(row: &SqliteRow) -> anyhow::Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            task_id: row.try_get("task_id")?,
            started_at: row.try_get("started_at")?,
            finished_at: row.try_get::<Option<i64>, _>("finished_at").ok().flatten(),
            status: row.try_get("status")?,
            error: row.try_get::<Option<String>, _>("error").ok().flatten(),
            log: row.try_get::<Option<String>, _>("log").ok().flatten(),
        })
    }
}

// ---------------------------------------------------------------------------
// Scan jobs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ScanJob {
    pub id: i64,
    pub library_id: i64,
    pub status: ScanStatus,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub files_seen: i64,
    pub files_added: i64,
    pub files_updated: i64,
    pub files_removed: i64,
    pub error_message: Option<String>,
    pub created_at: i64,
}

impl ScanJob {
    pub(crate) fn from_row(row: &SqliteRow) -> anyhow::Result<Self> {
        let status_str: &str = row.try_get("status")?;
        let status = match status_str {
            "queued" => ScanStatus::Queued,
            "running" => ScanStatus::Running,
            "completed" => ScanStatus::Completed,
            "failed" => ScanStatus::Failed,
            "canceled" => ScanStatus::Canceled,
            other => anyhow::bail!("unknown scan status: {other}"),
        };
        Ok(Self {
            id: row.try_get("id")?,
            library_id: row.try_get("library_id")?,
            status,
            started_at: row.try_get("started_at")?,
            finished_at: row.try_get("finished_at")?,
            files_seen: row.try_get("files_seen")?,
            files_added: row.try_get("files_added")?,
            files_updated: row.try_get("files_updated")?,
            files_removed: row.try_get("files_removed")?,
            error_message: row.try_get("error_message")?,
            created_at: row.try_get("created_at")?,
        })
    }
}

// ---------------------------------------------------------------------------
// Items / episodes (DTOs returned over the API)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub id: i64,
    pub library_id: i64,
    pub kind: ItemKind,
    pub title: String,
    pub sort_title: String,
    pub original_title: Option<String>,
    pub year: Option<i32>,
    pub summary: Option<String>,
    pub tagline: Option<String>,
    pub duration_ms: Option<i64>,
    pub rating_audience: Option<f64>,
    pub tmdb_id: Option<i64>,
    pub imdb_id: Option<String>,
    pub tvdb_id: Option<i64>,
    pub anilist_id: Option<i64>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    /// TMDB-relative path to the transparent title-treatment logo
    /// (e.g. `/foo.png`). The modal hero renders this instead of a
    /// plain text title when populated.
    pub logo_path: Option<String>,
    pub added_at: i64,
    pub updated_at: i64,
    /// Fields the user has manually edited. The enrichment pipeline skips
    /// any field listed here when applying provider data. Empty by default.
    pub locked_fields: Vec<String>,
    /// Local id of the collection (franchise) this item belongs to.
    /// Movies only; shows leave this NULL.
    pub collection_id: Option<i64>,
}

impl Item {
    pub(crate) fn from_row(row: &SqliteRow) -> anyhow::Result<Self> {
        // Poster / backdrop come from the images table via a LEFT JOIN that
        // exposes them as `poster_path` / `backdrop_path`. They may be
        // absent if no image has been ingested for the item yet.
        Ok(Self {
            id: row.try_get("id")?,
            library_id: row.try_get("library_id")?,
            kind: ItemKind::from_db(row.try_get::<&str, _>("kind")?)?,
            title: row.try_get("title")?,
            sort_title: row.try_get("sort_title")?,
            original_title: row.try_get("original_title")?,
            year: row.try_get("year")?,
            summary: row.try_get("summary")?,
            tagline: row.try_get("tagline")?,
            duration_ms: row.try_get("duration_ms")?,
            rating_audience: row.try_get("rating_audience")?,
            tmdb_id: row.try_get("tmdb_id")?,
            imdb_id: row.try_get("imdb_id")?,
            tvdb_id: row.try_get::<Option<i64>, _>("tvdb_id").ok().flatten(),
            anilist_id: row.try_get::<Option<i64>, _>("anilist_id").ok().flatten(),
            poster_path: row
                .try_get::<Option<String>, _>("poster_path")
                .ok()
                .flatten()
                .filter(|s| !s.is_empty()),
            backdrop_path: row
                .try_get::<Option<String>, _>("backdrop_path")
                .ok()
                .flatten()
                .filter(|s| !s.is_empty()),
            logo_path: row
                .try_get::<Option<String>, _>("logo_path")
                .ok()
                .flatten()
                .filter(|s| !s.is_empty()),
            added_at: row.try_get("added_at")?,
            updated_at: row.try_get("updated_at")?,
            locked_fields: row
                .try_get::<Option<String>, _>("locked_fields")
                .ok()
                .flatten()
                .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
                .unwrap_or_default(),
            collection_id: row.try_get::<Option<i64>, _>("collection_id").ok().flatten(),
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemSort {
    #[default]
    RecentlyAdded,
    Title,
    YearDesc,
    YearAsc,
    RatingDesc,
}

impl ItemSort {
    /// Map to an `ORDER BY` clause. Always include a stable tiebreaker
    /// (`i.id`) so paginated results don't shuffle within ties.
    pub fn order_by(&self) -> &'static str {
        match self {
            Self::RecentlyAdded => "i.added_at DESC, i.id DESC",
            Self::Title => "i.sort_title COLLATE NOCASE ASC, i.id ASC",
            Self::YearDesc => "i.year IS NULL, i.year DESC, i.sort_title COLLATE NOCASE ASC",
            Self::YearAsc => "i.year IS NULL, i.year ASC, i.sort_title COLLATE NOCASE ASC",
            Self::RatingDesc => {
                "i.rating_audience IS NULL, i.rating_audience DESC, i.sort_title COLLATE NOCASE ASC"
            }
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ItemFilter {
    pub library_id: Option<i64>,
    /// Restrict results to this set of libraries (intersected with the
    /// user's library_access). Wire format: `?library_ids=1,2,3`.
    /// Used by browse surfaces like /new-popular to honor per-user
    /// visibility prefs without leaking content from libraries the user
    /// hid in their own settings.
    #[serde(default, deserialize_with = "deserialize_csv_i64s")]
    pub library_ids: Option<Vec<i64>>,
    pub kind: Option<ItemKind>,
    /// Case-insensitive exact genre name (e.g. "Action"). When set, only
    /// items tagged with this genre are returned.
    pub genre: Option<String>,
    /// Case-insensitive substring search across title / original_title /
    /// sort_title. Cheap LIKE-based for now; FTS comes later.
    pub q: Option<String>,
    #[serde(default)]
    pub sort: Option<ItemSort>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

/// Serde deserializer for `?key=1,2,3` query-string fields. Empty string
/// (`?key=`) deserializes to `Some(vec![])` so callers can distinguish
/// "field omitted" from "field given but empty"; whitespace inside the
/// list is trimmed.
pub fn deserialize_csv_i64s<'de, D>(deserializer: D) -> Result<Option<Vec<i64>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let raw = Option::<String>::deserialize(deserializer)?;
    let Some(s) = raw else {
        return Ok(None);
    };
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let mut out = Vec::new();
    for part in trimmed.split(',') {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        out.push(p.parse::<i64>().map_err(D::Error::custom)?);
    }
    Ok(Some(out))
}

#[derive(Debug, Clone, Serialize)]
pub struct ItemPage {
    pub items: Vec<ListedItem>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListedItem {
    #[serde(flatten)]
    pub item: Item,
    pub play_state: Option<PlayStateForItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlayStateForItem {
    pub position_ms: i64,
    pub duration_ms: Option<i64>,
    pub watched: bool,
    pub view_count: i64,
    pub last_played_at: i64,
}

impl PlayStateForItem {
    pub(crate) fn from_columns(row: &SqliteRow) -> anyhow::Result<Option<Self>> {
        let position: Option<i64> = row.try_get("ps_position_ms")?;
        let Some(position_ms) = position else {
            return Ok(None);
        };
        Ok(Some(Self {
            position_ms,
            duration_ms: row.try_get("ps_duration_ms")?,
            watched: row.try_get::<i64, _>("ps_watched")? != 0,
            view_count: row.try_get("ps_view_count")?,
            last_played_at: row.try_get("ps_last_played_at")?,
        }))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ItemDetail {
    #[serde(flatten)]
    pub item: Item,
    pub genres: Vec<String>,
    pub play_state: Option<PlayStateForItem>,
    /// Always serialized as an array (empty for shows). Movies have files;
    /// shows expose them per-episode via the episode detail endpoint.
    pub files: Vec<MediaFileSummary>,
    /// Always serialized as an array (empty for movies).
    pub seasons: Vec<SeasonSummary>,
    /// Cast + crew, ordered by sort_order. Empty until the enrichment
    /// pipeline ingests credits for this item.
    #[serde(default)]
    pub credits: Vec<Credit>,
    /// Trailers, featurettes, behind-the-scenes, clips, deleted scenes.
    #[serde(default)]
    pub extras: Vec<Extra>,
    /// Aggregate review stats. Individual reviews are paged via
    /// /items/:id/reviews to keep ItemDetail bounded.
    #[serde(default)]
    pub reviews: ReviewsSummary,
}

#[derive(Debug, Clone, Serialize)]
pub struct MediaFileSummary {
    pub id: i64,
    pub container: Option<String>,
    pub duration_ms: Option<i64>,
    pub bit_rate: Option<i64>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub hdr_format: Option<String>,
    pub size_bytes: i64,
    pub streams: Vec<MediaStreamSummary>,
    /// Intro / credits ranges for skip buttons. Empty when the file
    /// hasn't been analyzed yet or has no detected markers.
    pub markers: Vec<Marker>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Marker {
    pub kind: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MediaStreamSummary {
    pub index: i32,
    pub kind: String,
    pub codec: Option<String>,
    pub language: Option<String>,
    /// Embedded track title (the `title` MKV tag — things like
    /// "Netflix eng subrip", "SDH eng subrip", "Cantonese (Traditional,
    /// Hong Kong) chi subrip"). Bluray + WEB-DL releases routinely
    /// label each track this way, and using the title in the picker
    /// gives users the same disambiguation they'd see in mpv / VLC /
    /// Haruna. None when the track wasn't tagged with a title (very
    /// common on raw transcodes).
    pub title: Option<String>,
    pub channels: Option<i32>,
    pub is_default: bool,
    pub is_forced: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SeasonSummary {
    pub id: i64,
    pub season_number: i32,
    pub title: Option<String>,
    pub episode_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Season {
    pub id: i64,
    pub show_id: i64,
    pub season_number: i32,
    pub title: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SeasonDetail {
    #[serde(flatten)]
    pub season: Season,
    pub episodes: Vec<EpisodeListed>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EpisodeListed {
    #[serde(flatten)]
    pub episode: Episode,
    pub play_state: Option<PlayStateForItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Episode {
    pub id: i64,
    pub season_id: i64,
    pub show_id: i64,
    pub season_number: i32,
    pub episode_number: i32,
    pub title: String,
    pub summary: Option<String>,
    pub air_date: Option<i64>,
    pub duration_ms: Option<i64>,
    pub thumb_path: Option<String>,
    pub added_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct EpisodeDetail {
    #[serde(flatten)]
    pub episode: Episode,
    pub play_state: Option<PlayStateForItem>,
    pub files: Vec<MediaFileSummary>,
}

/// Just enough info to stream a media file from disk.
#[derive(Debug, Clone)]
pub struct MediaFileLocator {
    pub id: i64,
    pub path: String,
    pub size_bytes: i64,
    pub container: Option<String>,
}

// ---------------------------------------------------------------------------
// People, credits, extras, reviews
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Person {
    pub id: i64,
    pub name: String,
    pub tmdb_id: Option<i64>,
    pub imdb_id: Option<String>,
    pub photo_url: Option<String>,
    pub biography: Option<String>,
    pub birthday: Option<i64>,
    pub deathday: Option<i64>,
    pub place_of_birth: Option<String>,
    pub known_for_department: Option<String>,
}

/// One person's involvement in one item. `role_kind` distinguishes cast
/// (acting) from crew roles like director/writer/producer so the UI can
/// section them. `character_name` is only meaningful for cast.
#[derive(Debug, Clone, Serialize)]
pub struct Credit {
    pub id: i64,
    pub person: Person,
    pub role_kind: String,
    pub role: String,
    pub character_name: Option<String>,
    pub sort_order: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Extra {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub source: String,
    pub source_id: String,
    pub thumb_url: Option<String>,
    pub duration_ms: Option<i64>,
    pub published_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Review {
    pub id: i64,
    pub item_id: i64,
    pub source: String,
    pub author: String,
    pub author_url: Option<String>,
    pub avatar_url: Option<String>,
    pub rating: Option<i32>,
    pub body: Option<String>,
    pub created_at: i64,
}

/// Aggregate stats for the reviews section in the item modal — kept inline
/// on ItemDetail so the typical render doesn't need a second round-trip.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ReviewsSummary {
    pub count: i64,
    pub average: Option<f64>,
}

/// One entry in a credits edit. When `person_id` is set we reuse the
/// existing `people` row (handy for re-ordering or fixing a typo); when
/// it's None we insert a new person from the provided `name` + optional
/// `photo_url`. `sort_order` is authoritative — clients can renumber freely.
#[derive(Debug, Clone, Deserialize)]
pub struct CreditEditInput {
    #[serde(default)]
    pub person_id: Option<i64>,
    pub name: String,
    pub role_kind: String,
    pub role: String,
    #[serde(default)]
    pub character_name: Option<String>,
    pub sort_order: i64,
    #[serde(default)]
    pub photo_url: Option<String>,
}

/// Full replacement payload for an item's cast/crew. Saving any list (even
/// empty) locks the `credits` field so re-enrichment won't blow it away.
#[derive(Debug, Clone, Deserialize)]
pub struct CreditsEditInput {
    pub credits: Vec<CreditEditInput>,
}

// Patch input for the Edit Metadata dialog. Each `Some` field is written to
// the corresponding column AND added to `locked_fields` so re-enrichment
// won't overwrite it. Missing/None means "no change".
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ItemEdit {
    pub title: Option<String>,
    pub sort_title: Option<String>,
    pub original_title: Option<String>,
    pub summary: Option<String>,
    pub tagline: Option<String>,
    pub year: Option<i32>,
    pub rating_age: Option<String>,
    pub rating_audience: Option<f64>,
    /// Fields to unlock (drop from locked_fields). Useful for re-enabling
    /// auto-enrichment on a specific field without resetting everything.
    #[serde(default)]
    pub unlock: Vec<String>,
}


// ---------------------------------------------------------------------------
// Play state input / output
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct PlayStateUpdate {
    pub item_id: Option<i64>,
    pub episode_id: Option<i64>,
    pub position_ms: i64,
    pub duration_ms: Option<i64>,
    pub watched: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlayStateBatch {
    pub updates: Vec<PlayStateUpdate>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScrobbleRequest {
    pub item_id: Option<i64>,
    pub episode_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum OnDeckEntry {
    Movie {
        item: Item,
        play_state: PlayStateForItem,
    },
    Episode {
        episode: Episode,
        show: Item,
        play_state: PlayStateForItem,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct OnDeckResponse {
    pub items: Vec<OnDeckEntry>,
}

// ---------------------------------------------------------------------------
// Users & sessions
// ---------------------------------------------------------------------------

/// Three-tier role hierarchy:
/// - `Owner` — root account. Manages every other account (including other
///   owners) and is the only role that can promote/demote owners. There
///   must always be at least one owner; the queries layer enforces this
///   via a count check before any owner-removing mutation.
/// - `Admin` — delegated administrator. Manages users + other admins
///   (CRUD, password reset, 2FA reset, role changes within the
///   admin/user tier), but cannot touch owner accounts in any way.
/// - `User` — regular viewer. No administrative powers.
///
/// The hierarchy ordering (`Owner > Admin > User`) is encoded in
/// [`Self::tier`] — actors can only manage targets strictly below their
/// own tier, with the exception that admins can demote/delete other
/// admins (same tier) but never owners.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    Owner,
    Admin,
    User,
}

impl UserRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::User => "user",
        }
    }

    pub fn from_db(s: &str) -> anyhow::Result<Self> {
        match s {
            "owner" => Ok(Self::Owner),
            "admin" => Ok(Self::Admin),
            "user" => Ok(Self::User),
            other => anyhow::bail!("unknown user role: {other}"),
        }
    }

    /// Hierarchy ranking. Higher = more privileged.
    pub fn tier(&self) -> u8 {
        match self {
            Self::Owner => 2,
            Self::Admin => 1,
            Self::User => 0,
        }
    }

    /// True for any administrative role (`Owner` or `Admin`). Used by
    /// the `AdminAuth` extractor and most admin-surface authorization
    /// checks that don't need to distinguish between the two.
    pub fn is_admin_or_owner(&self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub role: UserRole,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub email: Option<String>,
    pub default_audio_lang: Option<String>,
    pub default_subtitle_lang: Option<String>,
    /// Per-user toggle for email-mirroring of in-app notifications.
    /// Defaults to false so misconfigured SMTP can't surprise users
    /// with mail they didn't expect.
    pub notify_via_email: bool,
    /// Most-recent successful login. `None` if the user has never
    /// logged in (e.g. just-registered).
    pub last_login_at: Option<i64>,
    pub last_login_ip: Option<String>,
    /// The login immediately before `last_login_at` — surfaced on the
    /// post-login screen as a cheap anomaly check.
    pub previous_login_at: Option<i64>,
    pub previous_login_ip: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl User {
    pub(crate) fn from_row(row: &SqliteRow) -> anyhow::Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            username: row.try_get("username")?,
            role: UserRole::from_db(row.try_get::<&str, _>("role")?)?,
            display_name: row
                .try_get::<Option<String>, _>("display_name")
                .ok()
                .flatten(),
            avatar_url: row
                .try_get::<Option<String>, _>("avatar_path")
                .ok()
                .flatten(),
            email: row.try_get::<Option<String>, _>("email").ok().flatten(),
            default_audio_lang: row
                .try_get::<Option<String>, _>("default_audio_lang")
                .ok()
                .flatten(),
            default_subtitle_lang: row
                .try_get::<Option<String>, _>("default_subtitle_lang")
                .ok()
                .flatten(),
            notify_via_email: row
                .try_get::<i64, _>("notify_via_email")
                .ok()
                .unwrap_or(0)
                != 0,
            last_login_at: row.try_get::<Option<i64>, _>("last_login_at").ok().flatten(),
            last_login_ip: row
                .try_get::<Option<String>, _>("last_login_ip")
                .ok()
                .flatten(),
            previous_login_at: row
                .try_get::<Option<i64>, _>("previous_login_at")
                .ok()
                .flatten(),
            previous_login_ip: row
                .try_get::<Option<String>, _>("previous_login_ip")
                .ok()
                .flatten(),
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

/// Named bundle of library permissions that can be applied to many users
/// at once. Effective library access for a user is the UNION of direct
/// `library_access` rows and group-derived rows.
#[derive(Debug, Clone, Serialize)]
pub struct AccessGroup {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// Number of users currently in this group. Computed at list time
    /// for the admin overview; cheap because the join is on small
    /// indexed tables.
    pub member_count: i64,
    /// Number of libraries bound to this group.
    pub library_count: i64,
}

/// Eager-loaded variant with the joined member + library lists.
/// Used by the group editor.
#[derive(Debug, Clone, Serialize)]
pub struct AccessGroupDetail {
    #[serde(flatten)]
    pub group: AccessGroup,
    pub member_ids: Vec<i64>,
    pub library_ids: Vec<i64>,
}

impl AccessGroup {
    pub(crate) fn from_row_with_counts(row: &SqliteRow) -> anyhow::Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            description: row
                .try_get::<Option<String>, _>("description")
                .ok()
                .flatten(),
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            member_count: row.try_get::<i64, _>("member_count").unwrap_or(0),
            library_count: row.try_get::<i64, _>("library_count").unwrap_or(0),
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewAccessGroup {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AccessGroupUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_some")]
    pub description: Option<Option<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Notification {
    pub id: i64,
    pub user_id: i64,
    /// Stable string discriminator — `user.registered`, `user.2fa.disabled`,
    /// etc. The frontend renders the human message from `kind` + `payload`.
    pub kind: String,
    pub payload_json: String,
    pub read_at: Option<i64>,
    pub created_at: i64,
}

impl Notification {
    pub(crate) fn from_row(row: &SqliteRow) -> anyhow::Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            user_id: row.try_get("user_id")?,
            kind: row.try_get("kind")?,
            payload_json: row.try_get("payload_json")?,
            read_at: row.try_get::<Option<i64>, _>("read_at").ok().flatten(),
            created_at: row.try_get("created_at")?,
        })
    }
}

/// Internal-only: includes the password hash. Never serialized.
#[derive(Debug, Clone)]
pub struct UserWithSecret {
    pub user: User,
    pub password_hash: String,
}

#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id: i64,
    pub user_id: i64,
    /// SHA-256 of the cookie nonce. The cookie still carries the raw
    /// 32-byte nonce; the DB only stores its hash so a stolen `chimpflix.db`
    /// can't be turned into a forged session cookie. To verify, hash the
    /// caller-supplied nonce and compare here.
    pub nonce_hash: [u8; 32],
    pub expires_at: i64,
    pub last_seen_at: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Invite {
    pub id: i64,
    /// SHA-256 hex of the issued token. We never expose this to clients —
    /// the `#[serde(skip)]` keeps it out of API responses. Plaintext code
    /// is shown once at issuance; thereafter only the hash exists.
    #[serde(skip)]
    pub code_hash: String,
    pub created_by: i64,
    pub expires_at: Option<i64>,
    pub consumed_by: Option<i64>,
    pub consumed_at: Option<i64>,
    /// Recipient email address (None = no email sent; admin shares manually).
    pub email: Option<String>,
    /// Epoch ms when the invite email was successfully sent. None when
    /// email wasn't configured or send failed (admin can still copy the
    /// link returned on create).
    pub sent_at: Option<i64>,
    pub created_at: i64,
}

impl Invite {
    pub(crate) fn from_row(row: &SqliteRow) -> anyhow::Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            code_hash: row.try_get("code_hash")?,
            created_by: row.try_get("created_by")?,
            expires_at: row.try_get::<Option<i64>, _>("expires_at").ok().flatten(),
            consumed_by: row.try_get::<Option<i64>, _>("consumed_by").ok().flatten(),
            consumed_at: row.try_get::<Option<i64>, _>("consumed_at").ok().flatten(),
            email: row.try_get::<Option<String>, _>("email").ok().flatten(),
            sent_at: row.try_get::<Option<i64>, _>("sent_at").ok().flatten(),
            created_at: row.try_get("created_at")?,
        })
    }
}

/// Hash an invite plaintext token. SHA-256 hex. Used both to write the
/// row and to look it up on redemption.
pub fn hash_invite_code(code: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(code.trim().as_bytes());
    hex::encode(digest)
}

// ---------------------------------------------------------------------------
// Auth request bodies
//
// These all hold a plaintext password. We DO NOT derive `Debug` on
// them — a stray `tracing::debug!(?input, ...)` or middleware logger
// would otherwise dump the credential into logs (this is exactly the
// shape of the famous Twitter and Heroku credential-leak incidents).
// Manual `Debug` impls below redact the password field while keeping
// the rest of the struct legible during debugging.
// ---------------------------------------------------------------------------

#[derive(Clone, Deserialize)]
pub struct SetupInput {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub display_name: Option<String>,
    /// Owner's email. Optional at setup but strongly recommended — without
    /// it the owner can't recover via the self-service password reset
    /// flow (no address to send the reset link to).
    #[serde(default)]
    pub email: Option<String>,
}

impl std::fmt::Debug for SetupInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetupInput")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("display_name", &self.display_name)
            .field("email", &self.email)
            .finish()
    }
}

#[derive(Clone, Deserialize)]
pub struct LoginInput {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for LoginInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginInput")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Deserialize)]
pub struct RegisterInput {
    pub code: String,
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub display_name: Option<String>,
}

impl std::fmt::Debug for RegisterInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisterInput")
            // `code` is the invite token — also a credential. Redact it too.
            .field("code", &"<redacted>")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("display_name", &self.display_name)
            .finish()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateInviteInput {
    #[serde(default)]
    pub expires_in_seconds: Option<i64>,
    /// Email to send the invite to. When set + SMTP is configured, the
    /// server delivers the accept link directly. Either way the plain-
    /// text code is returned in the create response so the admin can
    /// copy/share manually if email is disabled or undeliverable.
    #[serde(default)]
    pub email: Option<String>,
    /// Optional library access pre-binding. On invite acceptance these
    /// rows are inserted into `library_access` for the new user.
    #[serde(default)]
    pub library_ids: Vec<i64>,
    /// Optional access-group pre-binding. On acceptance the user is
    /// added to each named group, inheriting the group's library set.
    #[serde(default)]
    pub group_ids: Vec<i64>,
}

// ---------------------------------------------------------------------------
// Server settings (singleton row) + audit log
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ServerSettings {
    pub server_name: String,
    pub public_url: Option<String>,
    /// JSON-encoded array of allowed CORS origins.
    pub cors_origins: String,
    /// One of: "required" | "preferred" | "disabled".
    pub secure_connections: String,
    pub telemetry_opt_in: bool,
    pub transcoder_max_concurrent: i64,
    /// One of: "none" | "vaapi" | "nvenc" | "qsv" | "videotoolbox".
    pub transcoder_hw_accel: String,
    pub transcoder_quality_ceiling_kbps: Option<i64>,
    /// One of: "speed" | "balanced" | "quality". Picks the
    /// encoder-side speed-vs-quality preset (libx264 ultrafast/
    /// veryfast/medium, NVENC p1/p4/p6, AMF speed/balanced/quality,
    /// VAAPI -global_quality 28/23/18). Default `balanced` matches
    /// behavior from before the column was introduced.
    pub transcoder_encoder_preset: String,
    /// One of: "auto" | "prefer_hw" | "require_hw". Controls how
    /// aggressively the planner enforces hardware acceleration.
    /// See the phase20 migration comment for what each mode means.
    pub transcoder_hw_strictness: String,
    // ---- Background / optimize_versions (phase 30) ----------------------
    /// libx264 preset used by the `optimize_versions` task when
    /// pre-encoding optimized files. Trades CPU time for output size.
    /// Values: ultrafast | superfast | veryfast | faster | fast |
    /// medium | slow | slower. Default `veryfast` matches the
    /// previous hard-coded value.
    pub transcoder_background_preset: String,
    /// Cap on concurrent jobs the `optimize_versions` task processes
    /// per tick. Default 1 — background work shouldn't starve live
    /// transcodes on a small machine.
    pub transcoder_max_background_concurrent: i64,
    // ---- HDR tone mapping (phase 30) -----------------------------------
    /// When true (default), HDR sources are tone-mapped to SDR via
    /// zscale + tonemap. When false the filter is skipped — saves
    /// CPU but HDR sources will look flat/washed-out on SDR clients.
    pub transcoder_hdr_tonemap_enabled: bool,
    /// Tonemap algorithm string passed to the `tonemap=tonemap=`
    /// argument. One of: hable | reinhard | mobius | bt2390 | clip |
    /// linear. Default `hable` is the previously hard-coded value
    /// and a reasonable middle ground.
    pub transcoder_hdr_tonemap_algo: String,
    // ---- SMTP / email (phase 21) ---------------------------------------
    /// SMTP server hostname (e.g. "smtp.example.com"). When None, the
    /// Mailer treats email as disabled and feature code calling
    /// `send_*()` is a no-op.
    pub email_smtp_host: Option<String>,
    /// SMTP port. Conventionally 587 for STARTTLS, 465 for implicit TLS,
    /// 25 for unencrypted (don't).
    pub email_smtp_port: Option<i64>,
    /// SMTP auth username. None means anonymous (rare; only useful for
    /// local relay setups).
    pub email_smtp_username: Option<String>,
    /// One of: "starttls" (default) | "tls" | "none".
    pub email_smtp_security: Option<String>,
    /// Envelope sender address — what recipients see in their From line.
    pub email_from_address: Option<String>,
    /// Display name. "From: <name> <address>".
    pub email_from_name: Option<String>,
    /// Global TOTP policy: "disabled" | "optional" (default) | "required".
    /// When "required", any user without verified 2FA is forced through
    /// enrollment before login completes.
    pub totp_enforcement: String,
    // ---- Maintenance window (phase 29) ---------------------------------
    /// HH:MM (24-hour) in server-local time. The window during which
    /// scheduled tasks marked `requires_maintenance_window` are allowed
    /// to run. Default 02:00 → 09:00. If the end time is <= the start
    /// the window wraps midnight.
    pub maintenance_window_start: String,
    pub maintenance_window_end: String,
    // ---- Library (phase 34) ---------------------------------------------
    /// When true (default), the filesystem watcher is spawned at
    /// startup and library scans fire within seconds of a file
    /// appearing/disappearing on disk. When false, only manual or
    /// scheduled scans run. Read once at startup — toggling requires
    /// a server restart.
    pub scan_automatically: bool,
    /// When true, completing a library scan (via file_watcher) queues
    /// `detect_markers` for every newly-discovered file that lacks
    /// auto markers. Off by default — blackdetect is expensive and the
    /// scheduled `detect_markers` task already covers bulk catch-up.
    /// Matches Plex's "Detect intro/credits when media is added."
    pub detect_markers_on_add: bool,
    // ---- Playback / library (phase 31) ---------------------------------
    /// Hard cap on the Continue Watching rail. Default 40.
    pub continue_watching_max_items: i64,
    /// In-progress items last played more than this many weeks ago
    /// are filtered out of the Continue Watching rail. Default 16.
    /// Set 0 to disable the time window entirely.
    pub continue_watching_max_age_weeks: i64,
    /// When true (default), the on_deck query augments its
    /// in-progress results with S(N+1)E01 of any show the user has
    /// watched at least one episode of. Off skips that augmentation
    /// entirely. Matches Plex's "Include season premieres in
    /// Continue Watching" toggle.
    pub continue_watching_include_premieres: bool,
    /// Single threshold for "this counts as watched" (1-99). Used by
    /// the client to auto-scrobble at this position and by the
    /// on-deck query to filter items past this point. Default 90.
    pub video_played_threshold_pct: i64,
    /// Drives auto-scrobble + on-deck filter. See migration phase 46.
    /// One of `threshold_pct` / `first_credits_marker` / `earliest_of_both`.
    pub video_completion_behaviour: String,
    /// Megabytes of memory the SQLite page cache may consume per
    /// connection. Applied via `PRAGMA cache_size` at pool open.
    /// Default 64. Set 0 to leave SQLite's default in place.
    pub database_cache_size_mb: i64,
    /// Default loudness normalization on every transcode session. When
    /// true, sessions apply the ffmpeg `loudnorm` filter (using stored
    /// per-file measurements when available, else generic targets).
    /// Per-session override still possible via the player. Off by
    /// default.
    pub audio_normalize_enabled: bool,
    /// `nice -n <level>` wrapper for ffmpeg/ffprobe invocations from
    /// the scanner and scheduled tasks (previews, chapter thumbs,
    /// loudness, marker detection). 0 disables the wrapper. Range
    /// 1..=19 (standard Unix nice). Read at server startup —
    /// changes require a restart.
    pub scanner_nice_level: i64,
    /// Filename of the uploaded pre-roll video relative to
    /// `<data_dir>/preroll/`, or None when none is set. Cleared by the
    /// admin via DELETE /admin/preroll.
    pub preroll_path: Option<String>,
    /// Master switch for pre-roll playback. When ON and `preroll_path`
    /// is set, the player runs the pre-roll first; user prefs can
    /// override per-user.
    pub preroll_enabled: bool,
    /// HEVC output mode for transcode sessions:
    /// `off` | `when_client_supports` | `always`. See migration phase 43.
    pub transcoder_hevc_encoding_mode: String,
    /// GPU device override: "auto" (default), a numeric NVENC index
    /// ("0", "1"), or a VAAPI render path ("/dev/dri/renderD129").
    /// Multi-GPU pinning; single-GPU boxes leave as "auto".
    pub transcoder_gpu_device: String,
    /// Cap on concurrent software-encoder (libx264 / libx265) sessions.
    /// Independent of `transcoder_max_concurrent`, which gates the
    /// total. Default 1 — a single CPU encode already pegs N cores;
    /// queueing additional ones starves any parallel GPU session.
    pub transcoder_max_cpu_concurrent: i64,
    // ---- Network policy (phase 32) -------------------------------------
    /// Milliseconds an HLS session can go without a keepalive before
    /// the reaper kills it. Default 90_000 (was the previous hard-
    /// coded value). Lower for snappier mobile cleanup; raise on
    /// flaky-network deployments.
    pub transcoder_reaper_idle_threshold_ms: i64,
    /// Cap on concurrent transcode sessions per user when the request
    /// originates from outside `lan_networks`. 0 disables the cap.
    pub max_remote_streams_per_user: i64,
    /// Comma-separated CIDR list. Anything matching is treated as
    /// local: bypasses the remote-streams cap and is shown as a "LAN"
    /// session in the admin dashboard. Empty = no LAN inference.
    pub lan_networks: String,
    /// Comma-separated CIDR list. Requests from a matching IP skip
    /// the cookie/session check and run as the server owner. Useful
    /// for Home Assistant / LAN automation; do NOT include public
    /// CIDRs.
    pub auth_bypass_cidrs: String,
    /// Operator-set bind override. Empty (default) honors `BIND_ADDR`
    /// env (which itself defaults to `0.0.0.0:8080`). Non-empty values
    /// like `192.168.1.50:8080` pin the listener to a specific NIC at
    /// next restart.
    pub bind_interface: String,
    /// BCP-47 language tag used for TMDB metadata fetches (overview,
    /// tagline, localized titles). Defaults to `en-US`. TMDB falls
    /// back to the original language when no translation exists for
    /// the requested tag — niche anime overviews may still come back
    /// in Japanese for that reason.
    pub metadata_language: String,
    /// Days an item stays badged as "Recently Added" in the UI.
    /// 0 disables the badge entirely; 14 is the default and matches
    /// the original hardcoded window from the Card component.
    pub recently_added_days: i64,
    /// JSON-encoded escape-hatch storage for fields added by later phases
    /// without their own migration.
    pub extras_json: String,
    pub updated_at: i64,
    pub updated_by: Option<i64>,
}

impl ServerSettings {
    pub(crate) fn from_row(row: &SqliteRow) -> anyhow::Result<Self> {
        Ok(Self {
            server_name: row.try_get("server_name")?,
            public_url: row.try_get::<Option<String>, _>("public_url").ok().flatten(),
            cors_origins: row.try_get("cors_origins")?,
            secure_connections: row.try_get("secure_connections")?,
            telemetry_opt_in: row.try_get::<i64, _>("telemetry_opt_in")? != 0,
            transcoder_max_concurrent: row.try_get("transcoder_max_concurrent")?,
            transcoder_hw_accel: row.try_get("transcoder_hw_accel")?,
            transcoder_quality_ceiling_kbps: row
                .try_get::<Option<i64>, _>("transcoder_quality_ceiling_kbps")
                .ok()
                .flatten(),
            transcoder_encoder_preset: row.try_get("transcoder_encoder_preset")?,
            transcoder_hw_strictness: row.try_get("transcoder_hw_strictness")?,
            transcoder_background_preset: row
                .try_get::<Option<String>, _>("transcoder_background_preset")
                .ok()
                .flatten()
                .unwrap_or_else(|| "veryfast".to_string()),
            transcoder_max_background_concurrent: row
                .try_get::<Option<i64>, _>("transcoder_max_background_concurrent")
                .ok()
                .flatten()
                .unwrap_or(1),
            transcoder_hdr_tonemap_enabled: row
                .try_get::<Option<i64>, _>("transcoder_hdr_tonemap_enabled")
                .ok()
                .flatten()
                .unwrap_or(1)
                != 0,
            transcoder_hdr_tonemap_algo: row
                .try_get::<Option<String>, _>("transcoder_hdr_tonemap_algo")
                .ok()
                .flatten()
                .unwrap_or_else(|| "hable".to_string()),
            email_smtp_host: row
                .try_get::<Option<String>, _>("email_smtp_host")
                .ok()
                .flatten(),
            email_smtp_port: row
                .try_get::<Option<i64>, _>("email_smtp_port")
                .ok()
                .flatten(),
            email_smtp_username: row
                .try_get::<Option<String>, _>("email_smtp_username")
                .ok()
                .flatten(),
            email_smtp_security: row
                .try_get::<Option<String>, _>("email_smtp_security")
                .ok()
                .flatten(),
            email_from_address: row
                .try_get::<Option<String>, _>("email_from_address")
                .ok()
                .flatten(),
            email_from_name: row
                .try_get::<Option<String>, _>("email_from_name")
                .ok()
                .flatten(),
            totp_enforcement: row
                .try_get::<Option<String>, _>("totp_enforcement")
                .ok()
                .flatten()
                .unwrap_or_else(|| "optional".to_string()),
            maintenance_window_start: row
                .try_get::<Option<String>, _>("maintenance_window_start")
                .ok()
                .flatten()
                .unwrap_or_else(|| "02:00".to_string()),
            maintenance_window_end: row
                .try_get::<Option<String>, _>("maintenance_window_end")
                .ok()
                .flatten()
                .unwrap_or_else(|| "09:00".to_string()),
            scan_automatically: row
                .try_get::<Option<i64>, _>("scan_automatically")
                .ok()
                .flatten()
                .unwrap_or(1)
                != 0,
            detect_markers_on_add: row
                .try_get::<Option<i64>, _>("detect_markers_on_add")
                .ok()
                .flatten()
                .unwrap_or(0)
                != 0,
            continue_watching_max_items: row
                .try_get::<Option<i64>, _>("continue_watching_max_items")
                .ok()
                .flatten()
                .unwrap_or(40),
            continue_watching_max_age_weeks: row
                .try_get::<Option<i64>, _>("continue_watching_max_age_weeks")
                .ok()
                .flatten()
                .unwrap_or(16),
            continue_watching_include_premieres: row
                .try_get::<Option<i64>, _>("continue_watching_include_premieres")
                .ok()
                .flatten()
                .unwrap_or(1)
                != 0,
            video_played_threshold_pct: row
                .try_get::<Option<i64>, _>("video_played_threshold_pct")
                .ok()
                .flatten()
                .unwrap_or(90),
            video_completion_behaviour: row
                .try_get::<Option<String>, _>("video_completion_behaviour")
                .ok()
                .flatten()
                .unwrap_or_else(|| "threshold_pct".to_string()),
            database_cache_size_mb: row
                .try_get::<Option<i64>, _>("database_cache_size_mb")
                .ok()
                .flatten()
                .unwrap_or(64),
            audio_normalize_enabled: row
                .try_get::<Option<i64>, _>("audio_normalize_enabled")
                .ok()
                .flatten()
                .unwrap_or(0)
                != 0,
            scanner_nice_level: row
                .try_get::<Option<i64>, _>("scanner_nice_level")
                .ok()
                .flatten()
                .unwrap_or(0),
            preroll_path: row
                .try_get::<Option<String>, _>("preroll_path")
                .ok()
                .flatten(),
            preroll_enabled: row
                .try_get::<Option<i64>, _>("preroll_enabled")
                .ok()
                .flatten()
                .unwrap_or(0)
                != 0,
            transcoder_hevc_encoding_mode: row
                .try_get::<Option<String>, _>("transcoder_hevc_encoding_mode")
                .ok()
                .flatten()
                .unwrap_or_else(|| "off".to_string()),
            transcoder_gpu_device: row
                .try_get::<Option<String>, _>("transcoder_gpu_device")
                .ok()
                .flatten()
                .unwrap_or_else(|| "auto".to_string()),
            transcoder_max_cpu_concurrent: row
                .try_get::<Option<i64>, _>("transcoder_max_cpu_concurrent")
                .ok()
                .flatten()
                .unwrap_or(1),
            transcoder_reaper_idle_threshold_ms: row
                .try_get::<Option<i64>, _>("transcoder_reaper_idle_threshold_ms")
                .ok()
                .flatten()
                .unwrap_or(90_000),
            max_remote_streams_per_user: row
                .try_get::<Option<i64>, _>("max_remote_streams_per_user")
                .ok()
                .flatten()
                .unwrap_or(0),
            lan_networks: row
                .try_get::<Option<String>, _>("lan_networks")
                .ok()
                .flatten()
                .unwrap_or_default(),
            auth_bypass_cidrs: row
                .try_get::<Option<String>, _>("auth_bypass_cidrs")
                .ok()
                .flatten()
                .unwrap_or_default(),
            bind_interface: row
                .try_get::<Option<String>, _>("bind_interface")
                .ok()
                .flatten()
                .unwrap_or_default(),
            metadata_language: row
                .try_get::<Option<String>, _>("metadata_language")
                .ok()
                .flatten()
                .unwrap_or_else(|| "en-US".to_string()),
            recently_added_days: row
                .try_get::<Option<i64>, _>("recently_added_days")
                .ok()
                .flatten()
                .unwrap_or(14),
            extras_json: row.try_get("extras_json")?,
            updated_at: row.try_get("updated_at")?,
            updated_by: row.try_get::<Option<i64>, _>("updated_by").ok().flatten(),
        })
    }
}

/// Partial update for server_settings. Each field is `None` to leave as-is.
/// Nested `Option<Option<T>>` columns can be explicitly nulled by sending
/// `Some(None)`.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ServerSettingsUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_some"
    )]
    pub public_url: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cors_origins: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secure_connections: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry_opt_in: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcoder_max_concurrent: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcoder_hw_accel: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_some"
    )]
    pub transcoder_quality_ceiling_kbps: Option<Option<i64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcoder_encoder_preset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcoder_hw_strictness: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcoder_background_preset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcoder_max_background_concurrent: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcoder_hdr_tonemap_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcoder_hdr_tonemap_algo: Option<String>,
    // Email / SMTP — every nullable field uses double-Option so the admin
    // UI can both clear and unset values without ambiguity.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_some"
    )]
    pub email_smtp_host: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_some"
    )]
    pub email_smtp_port: Option<Option<i64>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_some"
    )]
    pub email_smtp_username: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_some"
    )]
    pub email_smtp_security: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_some"
    )]
    pub email_from_address: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_some"
    )]
    pub email_from_name: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub totp_enforcement: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintenance_window_start: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintenance_window_end: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan_automatically: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detect_markers_on_add: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continue_watching_max_items: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continue_watching_max_age_weeks: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continue_watching_include_premieres: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_played_threshold_pct: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_completion_behaviour: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_cache_size_mb: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_normalize_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scanner_nice_level: Option<i64>,
    /// Outer Option = whether to update; inner = the value (None
    /// clears the field via SET preroll_path = NULL).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preroll_path: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preroll_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcoder_hevc_encoding_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcoder_gpu_device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcoder_max_cpu_concurrent: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcoder_reaper_idle_threshold_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_remote_streams_per_user: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lan_networks: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_bypass_cidrs: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind_interface: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recently_added_days: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extras_json: Option<String>,
}

/// serde helper: differentiate "key absent" from "key: null". The default
/// derive treats both as `None`; this preserves the distinction so PATCH
/// can clear a column with explicit JSON null without resetting other
/// nullable fields.
fn deserialize_some<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Deserialize::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditLogEntry {
    pub id: i64,
    pub actor_user_id: Option<i64>,
    pub action: String,
    pub target_kind: Option<String>,
    pub target_id: Option<String>,
    pub payload_json: Option<String>,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: i64,
}

impl AuditLogEntry {
    pub(crate) fn from_row(row: &SqliteRow) -> anyhow::Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            actor_user_id: row
                .try_get::<Option<i64>, _>("actor_user_id")
                .ok()
                .flatten(),
            action: row.try_get("action")?,
            target_kind: row.try_get::<Option<String>, _>("target_kind").ok().flatten(),
            target_id: row.try_get::<Option<String>, _>("target_id").ok().flatten(),
            payload_json: row
                .try_get::<Option<String>, _>("payload_json")
                .ok()
                .flatten(),
            ip: row.try_get::<Option<String>, _>("ip").ok().flatten(),
            user_agent: row
                .try_get::<Option<String>, _>("user_agent")
                .ok()
                .flatten(),
            created_at: row.try_get("created_at")?,
        })
    }
}

/// New entry to append to `audit_log`. All fields except `action` are
/// optional and carry context for future investigation.
#[derive(Debug, Clone, Default)]
pub struct NewAuditEntry {
    pub actor_user_id: Option<i64>,
    pub action: String,
    pub target_kind: Option<String>,
    pub target_id: Option<String>,
    pub payload_json: Option<String>,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
}

// ---------------------------------------------------------------------------
// External subtitles
// ---------------------------------------------------------------------------

/// Subtitle track fetched from an external agent or uploaded by an
/// operator. Embedded subtitle streams live on `media_streams`; this
/// table is the parallel surface the player merges into a single picker.
#[derive(Debug, Clone, Serialize)]
pub struct ExternalSubtitle {
    pub id: i64,
    pub item_id: Option<i64>,
    pub episode_id: Option<i64>,
    pub language: String,
    pub source: String,
    pub source_file_id: Option<String>,
    pub file_path: String,
    pub forced: bool,
    pub sdh: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewExternalSubtitle {
    pub item_id: Option<i64>,
    pub episode_id: Option<i64>,
    pub language: String,
    pub source: String,
    pub source_file_id: Option<String>,
    pub file_path: String,
    pub forced: bool,
    pub sdh: bool,
}

// ---------------------------------------------------------------------------
// Trending cache (Top 10)
// ---------------------------------------------------------------------------

/// Single entry written into / read from the trending_cache table.
/// Source is tagged at the caller (TMDB, Trakt) so the table can hold
/// multiple providers and the query can pick a preferred one.
#[derive(Debug, Clone, Serialize)]
pub struct TrendingEntry {
    pub rank: i64,
    pub tmdb_id: i64,
    pub title: Option<String>,
    pub poster_path: Option<String>,
}

// ---------------------------------------------------------------------------
// Credential vault metadata
// ---------------------------------------------------------------------------

/// Public-safe view of a stored secret. The plaintext value is never
/// exposed to API callers; `last4` lets the UI render a `••••1234` masked
/// preview, and `set` is always `true` for rows that exist.
#[derive(Debug, Clone, Serialize)]
pub struct SecretMetadata {
    pub name: String,
    pub set: bool,
    pub last4: String,
    pub updated_at: i64,
    pub updated_by: Option<i64>,
}

/// Sort-friendly form of a title (leading article removed for
/// alphabetic browsing).
pub fn make_sort_title(title: &str) -> String {
    let lower = title.to_lowercase();
    for article in ["the ", "a ", "an "] {
        if lower.starts_with(article) {
            return title[article.len()..].to_string();
        }
    }
    title.to_string()
}

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
}

impl LibraryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Movies => "movies",
            Self::Shows => "shows",
        }
    }

    pub fn from_db(s: &str) -> anyhow::Result<Self> {
        match s {
            "movies" => Ok(Self::Movies),
            "shows" => Ok(Self::Shows),
            other => anyhow::bail!("unknown library kind: {other}"),
        }
    }

    pub fn item_kind(&self) -> ItemKind {
        match self {
            Self::Movies => ItemKind::Movie,
            Self::Shows => ItemKind::Show,
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
    pub cron_expr: String,
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

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ScheduledTaskUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron_expr: Option<String>,
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
    pub secret: Option<String>,
    /// JSON-encoded array of event names. The dispatcher filters by this
    /// before delivering to each subscriber.
    pub event_mask: String,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Webhook {
    pub(crate) fn from_row(row: &SqliteRow) -> anyhow::Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            url: row.try_get("url")?,
            secret: row.try_get::<Option<String>, _>("secret").ok().flatten(),
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
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    Owner,
    User,
}

impl UserRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::User => "user",
        }
    }

    pub fn from_db(s: &str) -> anyhow::Result<Self> {
        match s {
            "owner" => Ok(Self::Owner),
            "user" => Ok(Self::User),
            other => anyhow::bail!("unknown user role: {other}"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub role: UserRole,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub default_audio_lang: Option<String>,
    pub default_subtitle_lang: Option<String>,
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
            default_audio_lang: row
                .try_get::<Option<String>, _>("default_audio_lang")
                .ok()
                .flatten(),
            default_subtitle_lang: row
                .try_get::<Option<String>, _>("default_subtitle_lang")
                .ok()
                .flatten(),
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
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
    pub nonce: [u8; 32],
    pub expires_at: i64,
    pub last_seen_at: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Invite {
    pub id: i64,
    pub code: String,
    pub created_by: i64,
    pub expires_at: Option<i64>,
    pub consumed_by: Option<i64>,
    pub consumed_at: Option<i64>,
    pub created_at: i64,
}

impl Invite {
    pub(crate) fn from_row(row: &SqliteRow) -> anyhow::Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            code: row.try_get("code")?,
            created_by: row.try_get("created_by")?,
            expires_at: row.try_get::<Option<i64>, _>("expires_at").ok().flatten(),
            consumed_by: row.try_get::<Option<i64>, _>("consumed_by").ok().flatten(),
            consumed_at: row.try_get::<Option<i64>, _>("consumed_at").ok().flatten(),
            created_at: row.try_get("created_at")?,
        })
    }
}

// ---------------------------------------------------------------------------
// Auth request bodies
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct SetupInput {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoginInput {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegisterInput {
    pub code: String,
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateInviteInput {
    #[serde(default)]
    pub expires_in_seconds: Option<i64>,
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

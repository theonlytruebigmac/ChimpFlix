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
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
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
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ItemFilter {
    pub library_id: Option<i64>,
    pub kind: Option<ItemKind>,
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<MediaFileSummary>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub seasons: Vec<SeasonSummary>,
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

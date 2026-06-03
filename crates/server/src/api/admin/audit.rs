//! `GET /admin/audit` — paginated admin action log.
//! `GET /admin/audit/export` — CSV export of the same filtered set.

use axum::Json;
use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use chimpflix_library::queries::AuditFilter;
use chimpflix_library::{AuditLogEntry, NewAuditEntry, queries};
use serde::{Deserialize, Serialize};

use crate::api::admin::audit_log;
use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::state::AppState;

/// Hard cap on rows streamed by the CSV export so one click can't pull an
/// unbounded result. audit_log is retention-bounded so this is generous.
const EXPORT_MAX_ROWS: i64 = 50_000;

/// An audit row enriched with the actor's resolved display name. Wraps
/// the stored [`AuditLogEntry`] (flattened so existing fields keep their
/// wire shape) and adds `actor_name`, resolved from `actor_user_id` via
/// a batched lookup so the admin Audit table can show who acted instead
/// of "user #N". `None` when the actor id is null or no longer resolves.
#[derive(Debug, Serialize)]
pub struct AuditRow {
    #[serde(flatten)]
    pub entry: AuditLogEntry,
    pub actor_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListParams {
    /// Cursor: return entries with id strictly less than this value.
    /// Legacy callers (cursor-paginated). When `offset` is present
    /// it wins.
    #[serde(default)]
    pub before: Option<i64>,
    /// Page size; clamped server-side to 1..=200.
    #[serde(default)]
    pub limit: Option<i64>,
    /// 0-based row offset for the paginated admin UI. When set,
    /// the response includes `total` + `entries` for offset/limit
    /// navigation; `next_before` is still emitted so cursor
    /// consumers stay working.
    #[serde(default)]
    pub offset: Option<i64>,
    /// When set, filter to only entries authored by this user id.
    /// Drives the per-user Audit tab in the user-management drawer.
    #[serde(default)]
    pub actor_user_id: Option<i64>,
    /// Substring match on the `action` column (case-sensitive LIKE
    /// `%action%`). Empty / whitespace-only is treated as "no filter".
    #[serde(default)]
    pub action: Option<String>,
    /// Lower bound on the `created_at` epoch-ms timestamp (inclusive).
    /// The admin date picker sends a millisecond epoch for local midnight.
    #[serde(default)]
    pub from: Option<i64>,
    /// Upper bound on the `created_at` epoch-ms timestamp (inclusive).
    #[serde(default)]
    pub to: Option<i64>,
}

impl ListParams {
    /// Collapse the request params into a [`AuditFilter`]. A blank or
    /// whitespace-only `action` is dropped so an empty search box doesn't
    /// match-nothing via `%%` semantics (it'd still match everything, but
    /// dropping it lets the unfiltered fast-path engage).
    fn to_filter(&self) -> AuditFilter {
        let action = self
            .action
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        AuditFilter {
            action,
            from_ms: self.from,
            to_ms: self.to,
            actor_user_id: self.actor_user_id,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ListResponse {
    pub entries: Vec<AuditRow>,
    pub next_before: Option<i64>,
    /// Total rows in audit_log. Drives the paginated admin
    /// surface's "X–Y of Z" summary + jump-to-page. Present on
    /// every response so clients can opt in without a second call.
    pub total: i64,
}

pub async fn list(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Query(params): Query<ListParams>,
) -> Result<Json<ListResponse>, ApiError> {
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let offset = params.offset.unwrap_or(0);
    let filter = params.to_filter();
    let (entries, total) = if !filter.is_empty() {
        // Any of action / from / to / actor set → unified filtered path.
        // This also subsumes the old actor-only branch (the per-user
        // drawer still sends only `actor_user_id`).
        let entries = queries::list_audit_filtered(&state.pool, &filter, limit, offset)
            .await
            .map_err(ApiError::Internal)?;
        let total = queries::count_audit_filtered(&state.pool, &filter)
            .await
            .map_err(ApiError::Internal)?;
        (entries, total)
    } else if params.offset.is_some() {
        let entries = queries::list_audit_paged(&state.pool, limit, offset)
            .await
            .map_err(ApiError::Internal)?;
        let total = queries::count_audit(&state.pool)
            .await
            .map_err(ApiError::Internal)?;
        (entries, total)
    } else {
        let entries = queries::list_audit(&state.pool, params.before, limit)
            .await
            .map_err(ApiError::Internal)?;
        let total = queries::count_audit(&state.pool)
            .await
            .map_err(ApiError::Internal)?;
        (entries, total)
    };
    let next_before = entries.last().map(|e| e.id);

    // Resolve actor display names in one batched query (no N+1 per row).
    // Best-effort: a lookup failure leaves every `actor_name` None and
    // the client falls back to "user #N".
    let actor_ids: Vec<i64> = {
        let mut ids: Vec<i64> = entries.iter().filter_map(|e| e.actor_user_id).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    let names = queries::resolve_user_display_names(&state.pool, &actor_ids)
        .await
        .unwrap_or_default();
    let entries: Vec<AuditRow> = entries
        .into_iter()
        .map(|entry| {
            let actor_name = entry
                .actor_user_id
                .and_then(|id| names.get(&id).cloned());
            AuditRow { entry, actor_name }
        })
        .collect();

    Ok(Json(ListResponse {
        entries,
        next_before,
        total,
    }))
}

/// `GET /admin/audit/export` — stream the filtered audit set as CSV.
///
/// Accepts the same filter params as [`list`] (action / from / to /
/// actor_user_id); pagination params are ignored — the export returns the
/// whole filtered set up to [`EXPORT_MAX_ROWS`], newest-first. Actor names
/// are resolved in one batched lookup (same as the list) so the CSV is
/// readable without a join. The act of exporting is itself audited.
pub async fn export(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Query(params): Query<ListParams>,
) -> Result<Response, ApiError> {
    let filter = params.to_filter();
    let entries = queries::list_audit_for_export(&state.pool, &filter, EXPORT_MAX_ROWS)
        .await
        .map_err(ApiError::Internal)?;

    // Resolve actor display names in one batched query (mirrors `list`).
    let actor_ids: Vec<i64> = {
        let mut ids: Vec<i64> = entries.iter().filter_map(|e| e.actor_user_id).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    let names = queries::resolve_user_display_names(&state.pool, &actor_ids)
        .await
        .unwrap_or_default();

    let row_count = entries.len();
    let mut csv = String::with_capacity(64 + row_count * 96);
    csv.push_str("id,created_at,action,actor_user_id,actor_name,target_kind,target_id,ip,user_agent,payload_json\n");
    for e in &entries {
        let actor_name = e
            .actor_user_id
            .and_then(|id| names.get(&id).cloned())
            .unwrap_or_default();
        // ISO-ish UTC stamp from epoch ms for human-readable spreadsheets.
        let when = iso_utc(e.created_at);
        let mut line = String::new();
        line.push_str(&e.id.to_string());
        line.push(',');
        line.push_str(&csv_field(&when));
        line.push(',');
        line.push_str(&csv_field(&e.action));
        line.push(',');
        line.push_str(&e.actor_user_id.map(|i| i.to_string()).unwrap_or_default());
        line.push(',');
        line.push_str(&csv_field(&actor_name));
        line.push(',');
        line.push_str(&csv_field(e.target_kind.as_deref().unwrap_or("")));
        line.push(',');
        line.push_str(&csv_field(e.target_id.as_deref().unwrap_or("")));
        line.push(',');
        line.push_str(&csv_field(e.ip.as_deref().unwrap_or("")));
        line.push(',');
        line.push_str(&csv_field(e.user_agent.as_deref().unwrap_or("")));
        line.push(',');
        line.push_str(&csv_field(e.payload_json.as_deref().unwrap_or("")));
        line.push('\n');
        csv.push_str(&line);
    }

    // Record the export itself in the audit trail (best-effort).
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "audit.export".into(),
            target_kind: Some("audit_log".into()),
            target_id: None,
            payload_json: Some(format!("{{\"rows\":{row_count}}}")),
            ip: None,
            user_agent: None,
        },
    )
    .await;

    let filename = format!("chimpflix-audit-{}.csv", now_stamp());
    let response = Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/csv; charset=utf-8"),
        )
        .header(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
                .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
        )
        .body(Body::from(csv))
        .map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))?;
    Ok(response)
}

/// Escape a single CSV field per RFC 4180 (wrap in double quotes + double
/// embedded quotes when it contains a comma/quote/CR/LF), AND neutralize
/// spreadsheet formula injection. The `user_agent`/`ip` columns come
/// straight from client request headers, so a value like `=HYPERLINK(...)`
/// or `=cmd|...` would execute when an operator opens the export in
/// Excel/Sheets. Prefix any field that begins with a formula-trigger char
/// (`=`, `+`, `-`, `@`, tab, CR, LF) with a single quote so it renders as text.
fn csv_field(s: &str) -> String {
    let guarded;
    let s: &str = if s
        .chars()
        .next()
        .is_some_and(|c| matches!(c, '=' | '+' | '-' | '@' | '\t' | '\r' | '\n'))
    {
        guarded = format!("'{s}");
        &guarded
    } else {
        s
    };
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

/// Format an epoch-ms timestamp as a UTC `YYYY-MM-DD HH:MM:SS` string for
/// the CSV. Uses chrono (already a workspace dep) so spreadsheets get a
/// sortable, human-readable column instead of a raw millisecond integer.
fn iso_utc(epoch_ms: i64) -> String {
    use chrono::{TimeZone, Utc};
    match Utc.timestamp_millis_opt(epoch_ms).single() {
        Some(dt) => dt.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        None => epoch_ms.to_string(),
    }
}

/// Compact UTC timestamp for the download filename (`YYYYMMDD-HHMMSS`).
fn now_stamp() -> String {
    use chrono::Utc;
    Utc::now().format("%Y%m%d-%H%M%S").to_string()
}

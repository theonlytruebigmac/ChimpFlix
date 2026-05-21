// Typed client for the ChimpFlix Rust backend (`/api/v1/*`).
//
// Works in both contexts:
//   - Server Components / Route Handlers: talks to CHIMPFLIX_API_URL
//     (http://server:8080 in compose) and forwards the user's
//     session cookie via next/headers.
//   - Client Components: talks to the same-origin `/api/v1/*` path,
//     which Next.js rewrites to the Rust server. The browser sends
//     cookies automatically.
//
// Types mirror the Rust serde shapes in `crates/library/src/models.rs`
// and the response wrappers in `crates/server/src/api/*.rs`. Keep them
// in sync when you change either side.

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Three-tier role hierarchy. `Owner > Admin > User` for the purposes
/// of management actions — owners manage anyone (including each
/// other), admins manage users + other admins but never owners, users
/// have no admin powers. At least one owner must always exist; the
/// backend rejects mutations that would orphan the system.
export type UserRole = "owner" | "admin" | "user";

export interface User {
  id: number;
  username: string;
  role: UserRole;
  display_name: string | null;
  avatar_url: string | null;
  email: string | null;
  default_audio_lang: string | null;
  default_subtitle_lang: string | null;
  /** Whether the user opted into email mirroring of in-app notifications. */
  notify_via_email: boolean;
  /** Most-recent successful login. null on first login. */
  last_login_at: number | null;
  last_login_ip: string | null;
  /** The login immediately before `last_login_at`. Surfaced as "last
   * signed in 3h ago from X" so users can spot unexpected access. */
  previous_login_at: number | null;
  previous_login_ip: string | null;
  created_at: number;
  updated_at: number;
}

export interface UpdateMeInput {
  // Empty string clears the field; omit to leave unchanged.
  display_name?: string;
  avatar_url?: string;
  email?: string;
  default_audio_lang?: string;
  default_subtitle_lang?: string;
  notify_via_email?: boolean;
}

export interface Notification {
  id: number;
  user_id: number;
  /** Stable discriminator: "user.registered" | "user.2fa.disabled" |
   * "user.2fa.reset" | future kinds. Frontend dispatches on this. */
  kind: string;
  /** JSON-encoded payload. Shape depends on `kind`. */
  payload_json: string;
  read_at: number | null;
  created_at: number;
}

export interface NotificationsListResponse {
  notifications: Notification[];
  unread: number;
}

export interface SessionSummary {
  id: number;
  user_id: number;
  username: string;
  user_agent: string | null;
  ip: string | null;
  last_seen_at: number;
  expires_at: number;
  created_at: number;
}

export interface MySessionEntry extends SessionSummary {
  /** True for the session that authenticated the current request. */
  current: boolean;
}

export interface RequestEmailChangeResponse {
  new_email: string;
  email_sent: boolean;
}

export interface AuthResponse {
  user: User;
}

/// Tagged login outcome. Discriminate on `status` before destructuring:
///   * `authenticated` — server set a session cookie, `user` populated.
///   * `2fa_required`  — client must POST /auth/2fa/login with the
///                       returned challenge + a TOTP or recovery code.
export type LoginResponse =
  | { status: "authenticated"; user: User }
  | {
      status: "2fa_required";
      challenge: string;
      expires_in_seconds: number;
    };

export interface AuthStatusResponse {
  setup_needed: boolean;
}

export interface TotpStatusResponse {
  enrolled: boolean;
  verified: boolean;
  enforcement: TotpEnforcement;
  unused_recovery_codes: number;
}

export interface TotpEnrollResponse {
  /// Base32 of the shared secret — for manual entry into authenticator
  /// apps that can't scan a QR.
  secret: string;
  /// `otpauth://totp/...` provisioning URI. Render as QR for scanning.
  otpauth_uri: string;
  /// Pre-rendered QR as a `data:image/svg+xml;base64,…` URL. Drop into
  /// an <img> tag — server-side rendered, no client deps needed.
  qr_data_url: string;
}

export interface TotpVerifyResponse {
  /// Recovery codes shown ONCE. Server only persists SHA-256 hashes.
  recovery_codes: string[];
}

export type LibraryKind = "movies" | "shows" | "anime";

export type EpisodeSortOrder = "oldest_first" | "newest_first";
export type EpisodeNaming = "tmdb" | "original" | "absolute";
export type LibraryVisibility = "home_and_search" | "search_only" | "hidden";

export interface Library {
  id: number;
  name: string;
  kind: LibraryKind;
  paths: string[];
  scan_interval_s: number;
  last_scan_at: number | null;
  episode_sort_order: EpisodeSortOrder;
  episode_naming: EpisodeNaming;
  certification_country: string;
  visibility: LibraryVisibility;
  /** When true, the item modal exposes a Delete-from-disk button.
   *  Off by default to protect against accidental loss. */
  allow_media_deletion: boolean;
  created_at: number;
  updated_at: number;
}

export interface NewLibraryInput {
  name: string;
  kind: LibraryKind;
  paths: string[];
  scan_interval_s?: number;
}

export interface LibraryUpdateInput {
  name?: string;
  paths?: string[];
  scan_interval_s?: number;
  episode_sort_order?: EpisodeSortOrder;
  episode_naming?: EpisodeNaming;
  certification_country?: string;
  visibility?: LibraryVisibility;
  allow_media_deletion?: boolean;
}

export interface LibraryAgent {
  agent_name: string;
  priority: number;
  enabled: boolean;
  config_json: string;
}

export interface AgentInfo {
  name: string;
  display_name: string;
  supported_kinds: string[];
  configured: boolean;
}

// ─── Scheduled tasks ───────────────────────────────────────────────────────

/** Friendly schedule label. When `custom`, `cron_expr` is the
 *  source of truth. Otherwise the scheduler computes `next_run_at`
 *  from `frequency + last_finished_at`. */
export type TaskFrequency =
  | "manual"
  | "hourly"
  | "every_3_hours"
  | "every_6_hours"
  | "every_12_hours"
  | "daily"
  | "every_3_days"
  | "weekly"
  | "monthly"
  | "on_change"
  | "custom";

export interface ScheduledTask {
  id: number;
  kind: string;
  name: string;
  cron_expr: string;
  frequency: TaskFrequency;
  /** When true, computed `next_run_at` is snapped forward into the
   *  next opening of the maintenance window (see ServerSettings). */
  requires_maintenance_window: boolean;
  params_json: string;
  enabled: boolean;
  last_run_at: number | null;
  last_status: "success" | "failed" | "running" | null;
  last_error: string | null;
  last_duration_ms: number | null;
  next_run_at: number;
  created_at: number;
  updated_at: number;
}

export interface NewScheduledTaskInput {
  kind: string;
  name: string;
  cron_expr?: string;
  frequency?: TaskFrequency;
  requires_maintenance_window?: boolean;
  params_json?: string;
  enabled?: boolean;
}

export interface ScheduledTaskUpdate {
  name?: string;
  cron_expr?: string;
  frequency?: TaskFrequency;
  requires_maintenance_window?: boolean;
  params_json?: string;
  enabled?: boolean;
}

export interface TaskKindInfo {
  kind: string;
  display_name: string;
  description: string;
  params_schema: string;
  /** Pre-filled when creating a new task of this kind. */
  default_frequency: TaskFrequency;
  /** Pre-filled when creating a new task of this kind. */
  default_requires_maintenance_window: boolean;
}

export interface TaskRun {
  id: number;
  task_id: number;
  started_at: number;
  finished_at: number | null;
  status: "success" | "failed" | "running";
  error: string | null;
  log: string | null;
}

export interface TasksListResponse {
  tasks: ScheduledTask[];
  kinds: TaskKindInfo[];
}

// ─── Registry-driven overview ────────────────────────────────────────────
//
// Shapes mirror the Rust types in `crates/server/src/api/admin/tasks_overview.rs`.
// One overview fetch = one round-trip; the page renders without further
// data joins on the client.

export type TaskMode = "automatic" | "gated" | "periodic";
export type TaskScope = "per_file" | "per_item" | "global";
export type LastStatus = "ok" | "warn" | "bad";

export interface OverviewGateInfo {
  enabled: boolean;
  /// True when the kind has no admin-flippable switch (Automatic mode).
  locked: boolean;
  /// `*_enabled` setting key. Null for Automatic kinds.
  setting_key: string | null;
}

export interface OverviewScheduleInfo {
  /// `scheduled_tasks.requires_maintenance_window` — whether the
  /// next_run_at gets snapped forward into the operator's
  /// configured low-traffic window. Round-tripped so the detail
  /// page's editable card can render the current value instead of
  /// guessing a default.
  requires_maintenance_window: boolean;
  frequency: string;
  enabled: boolean;
  next_at: number;
  last_at: number | null;
  last_status: LastStatus;
}

export interface OverviewLiveInfo {
  in_flight: number;
  queued: number;
  last_success_at_ms: number | null;
}

export interface OverviewKindCard {
  name: string;
  display_name: string;
  mode: TaskMode;
  scope: TaskScope;
  gate: OverviewGateInfo;
  schedule: OverviewScheduleInfo | null;
  live: OverviewLiveInfo;
}

export interface OverviewKindSection {
  /// Machine-stable identifier — e.g. "automatic", "gated",
  /// "all". Clients filter on this; `label` is only for display.
  id: string;
  label: string;
  kinds: OverviewKindCard[];
}

export interface OverviewKindGroup {
  /// Machine-stable identifier — "media_ingest", "watch_state",
  /// "system". Clients filter on this; `name` is only for display.
  id: string;
  name: string;
  sections: OverviewKindSection[];
}

export interface TasksOverviewResponse {
  groups: OverviewKindGroup[];
}

export interface TasksSummaryResponse {
  running: number;
  queued: number;
  succeeded_24h: number;
  failed_24h: number;
  /// Epoch ms of next maintenance-window open. Null when the
  /// window is currently open.
  next_maintenance_window_ms: number | null;
}

export interface ActivityKindHealth {
  kind: string;
  display_name: string;
  queue_depth: number;
  in_flight: number;
  jobs_per_minute: number;
  p95_duration_ms: number | null;
  recent_errors: number;
  /// Registry-shipped concurrency baseline. Surfaced so the
  /// activity page's per-kind cap editor can render "default N"
  /// hints alongside an editable override.
  default_concurrency: number;
}

export interface ActivityRecentRun {
  kind: string;
  finished_at_ms: number;
  duration_ms: number;
  success: boolean;
  error_class: string | null;
}

export interface ActivityFailedJob {
  id: number;
  kind: string;
  last_error: string | null;
  error_class: string | null;
  finished_at_ms: number | null;
}

export interface TasksActivityResponse {
  per_kind: ActivityKindHealth[];
  recent_runs: ActivityRecentRun[];
  failed: ActivityFailedJob[];
}

export interface KindDetailDailyMetrics {
  day_ms: number;
  success_count: number;
  failure_count: number;
  p50_duration_ms: number | null;
  p95_duration_ms: number | null;
  targets_processed: number;
}

export interface KindDetailResponse {
  name: string;
  display_name: string;
  mode: TaskMode;
  scope: TaskScope;
  gate: OverviewGateInfo;
  schedule: OverviewScheduleInfo | null;
  live: OverviewLiveInfo;
  p95_duration_ms: number | null;
  recent_runs: ActivityRecentRun[];
  history: KindDetailDailyMetrics[];
}

/// PATCH body for the rebuilt detail page's editable Schedule card.
/// All fields are independently optional — passing just
/// `{ frequency: "daily" }` leaves enabled, window, params alone.
export interface KindScheduleUpdate {
  frequency?: TaskFrequency;
  enabled?: boolean;
  requires_maintenance_window?: boolean;
  params_json?: string;
}

// ─── Transcoder ────────────────────────────────────────────────────────────

export interface TranscoderCapabilities {
  ffmpeg_version: string | null;
  hwaccels: string[];
  h264_encoders: string[];
  hevc_encoders: string[];
  /// Per-hwaccel list of source codecs the runtime probe confirmed
  /// the GPU can decode. Empty arrays mean either (a) the hwaccel
  /// isn't present or (b) the probe failed for every codec. Used
  /// by the admin UI to show "what your GPU actually does" and by
  /// `pick_video_treatment` to decide whether to emit
  /// `-hwaccel <name>` per session.
  decoders: HwDecoderCapabilities;
  /// Enumerated GPU devices for the admin Transcoder dropdown.
  /// Empty on single-GPU boxes (the "auto" sentinel still works);
  /// populated when multiple cards are present.
  gpu_devices: GpuDevice[];
}

export interface GpuDevice {
  name: string;
  value: string;
  backend: string;
}

export interface HwDecoderCapabilities {
  cuda: string[];
  vaapi: string[];
  qsv: string[];
  videotoolbox: string[];
}

export interface TranscoderPreset {
  id: number;
  name: string;
  max_video_bitrate_kbps: number;
  max_height: number;
  audio_codec: string;
  audio_bitrate_kbps: number;
  enabled: boolean;
  sort_order: number;
}

export interface NewTranscoderPresetInput {
  name: string;
  max_video_bitrate_kbps: number;
  max_height: number;
  audio_codec?: string;
  audio_bitrate_kbps?: number;
  enabled?: boolean;
  sort_order?: number;
}

export interface TranscoderPresetUpdate {
  name?: string;
  max_video_bitrate_kbps?: number;
  max_height?: number;
  audio_codec?: string;
  audio_bitrate_kbps?: number;
  enabled?: boolean;
  sort_order?: number;
}

// ─── Network ───────────────────────────────────────────────────────────────

export interface NetworkSettings {
  public_url: string | null;
  cors_origins: string[];
  secure_connections: SecureConnectionsMode;
  /** ms a session can go without a keepalive before the reaper kills
   *  it. Default 90_000. Read at startup — changes take effect on
   *  next server restart. */
  transcoder_reaper_idle_threshold_ms: number;
  /** Cap on concurrent transcode sessions per user when the request
   *  originates outside `lan_networks`. 0 disables the cap. */
  max_remote_streams_per_user: number;
  /** Comma-separated CIDR list. Empty = no LAN inference. */
  lan_networks: string;
  /** Comma-separated CIDR list. Matching IPs skip the cookie check
   *  and run as the server owner. */
  auth_bypass_cidrs: string;
  /** Empty = honor BIND_ADDR env (default `0.0.0.0:8080`). Non-empty
   *  values like `192.168.1.50:8080` pin the listener to a specific
   *  NIC. Restart required for changes to take effect. */
  bind_interface: string;
}

export interface NetworkUpdateInput {
  public_url?: string | null;
  cors_origins?: string[];
  secure_connections?: SecureConnectionsMode;
  transcoder_reaper_idle_threshold_ms?: number;
  max_remote_streams_per_user?: number;
  lan_networks?: string;
  auth_bypass_cidrs?: string;
  bind_interface?: string;
}

export interface ReachabilityResult {
  ok: boolean;
  public_url: string | null;
  status_code: number | null;
  latency_ms: number | null;
  error: string | null;
}

// ─── Webhooks ──────────────────────────────────────────────────────────────

export interface Webhook {
  id: number;
  name: string;
  url: string;
  secret: string | null;
  event_mask: string; // JSON-encoded string[]
  enabled: boolean;
  created_at: number;
  updated_at: number;
}

export interface NewWebhookInput {
  name: string;
  url: string;
  secret?: string;
  event_mask: string[];
  enabled?: boolean;
}

export interface WebhookUpdateInput {
  name?: string;
  url?: string;
  secret?: string | null;
  event_mask?: string[];
  enabled?: boolean;
}

export interface WebhookDelivery {
  id: number;
  webhook_id: number;
  event: string;
  payload_json: string;
  status_code: number | null;
  response_body: string | null;
  error: string | null;
  attempts: number;
  next_retry_at: number | null;
  delivered_at: number | null;
  created_at: number;
}

export interface WebhooksListResponse {
  webhooks: Webhook[];
  events: string[];
}

// ─── Phase 8: Sessions / Access ────────────────────────────────────────────

export interface AdminSessionSummary {
  id: number;
  user_id: number;
  username: string;
  user_agent: string | null;
  ip: string | null;
  last_seen_at: number;
  expires_at: number;
  created_at: number;
}

export interface AccessMatrixEntry {
  user_id: number;
  username: string;
  library_id: number;
  library_name: string;
  /// Direct `library_access` row exists. This is the only field the
  /// matrix checkbox edits.
  allowed: boolean;
  /// Access-group names that ALSO grant this user this library
  /// (`access_group_libraries` × `user_access_groups`). These grants
  /// aren't editable from the matrix — they're managed under Settings
  /// → Users → Groups — but the UI surfaces them so admins can see
  /// effective access at a glance instead of mistaking group-only
  /// access for "locked out".
  via_groups: string[];
}

export interface LibraryAccessAssignment {
  library_id: number;
  user_ids: number[];
}

// ─── Optimized Versions ────────────────────────────────────────────────────

export interface OptimizedVersion {
  id: number;
  source_file_id: number;
  preset_id: number;
  output_path: string;
  output_size_bytes: number | null;
  duration_ms: number | null;
  status: "queued" | "running" | "success" | "failed";
  error: string | null;
  created_at: number;
  completed_at: number | null;
}

export interface NewOptimizedVersionInput {
  source_file_id: number;
  preset_id: number;
}

// ─── Maintenance ───────────────────────────────────────────────────────────

export interface LogLine {
  timestamp_ms: number;
  level: string;
  target: string;
  message: string;
}

export interface AlertsResponse {
  log_alerts: LogLine[];
  audit: AuditLogEntry[];
}

export type ItemKind = "movie" | "show";

export interface Item {
  id: number;
  library_id: number;
  kind: ItemKind;
  title: string;
  sort_title: string;
  original_title: string | null;
  year: number | null;
  summary: string | null;
  tagline: string | null;
  duration_ms: number | null;
  rating_audience: number | null;
  tmdb_id: number | null;
  imdb_id: string | null;
  tvdb_id: number | null;
  anilist_id: number | null;
  poster_path: string | null;
  backdrop_path: string | null;
  /// TMDB-relative path to the transparent title-treatment logo PNG.
  /// When set, the modal hero renders this image instead of a plain
  /// text title. Populated by TMDB enrichment + the refresh_logos task.
  logo_path: string | null;
  added_at: number;
  updated_at: number;
  /// Movies only; shows leave this null.
  collection_id?: number | null;
}

export interface Collection {
  id: number;
  tmdb_id: number | null;
  kind: "auto" | "manual" | "smart";
  name: string;
  sort_title: string | null;
  overview: string | null;
  description: string | null;
  poster_path: string | null;
  backdrop_path: string | null;
  created_by_user_id: number | null;
  rule_json: string | null;
  item_count: number;
}

export interface CollectionDetail extends Collection {
  items: ListedItem[];
}

export interface NewManualCollection {
  name: string;
  sort_title?: string | null;
  description?: string | null;
}

export interface NewSmartCollection {
  name: string;
  sort_title?: string | null;
  description?: string | null;
  /// Pre-serialised rule JSON. See backend `smart_rule` module for
  /// the supported field/op vocabulary.
  rule_json: string;
}

/// Double-Option mirrors the Rust shape: `undefined` = field omitted (no
/// change); `null` = explicitly set to NULL. Useful for clearing the
/// poster_path on a collection without nuking name in the same call.
export interface ManualCollectionUpdate {
  name?: string;
  sort_title?: string | null;
  description?: string | null;
  poster_path?: string | null;
  backdrop_path?: string | null;
}

export interface PlayStateForItem {
  position_ms: number;
  duration_ms: number | null;
  watched: boolean;
  view_count: number;
  last_played_at: number;
}

// `ListedItem` flattens `Item` + `play_state`, so TS sees a single object.
export type ListedItem = Item & { play_state: PlayStateForItem | null };

export interface ItemPage {
  items: ListedItem[];
  total: number;
  page: number;
  page_size: number;
}

export type ItemSort =
  | "recently_added"
  | "title"
  | "year_desc"
  | "year_asc"
  | "rating_desc";

export interface ItemFilter {
  library_id?: number;
  // Restrict to this set of libraries. Intersected with the user's access
  // server-side, so passing libraries the user can't see is a no-op rather
  // than an error. Use this to honor visibility / hidden-library prefs on
  // browse surfaces (e.g. /new-popular global rails).
  library_ids?: ReadonlyArray<number>;
  kind?: ItemKind;
  // Case-insensitive exact genre match (e.g. "Action").
  genre?: string;
  // Case-insensitive substring search across title/original_title/sort_title.
  q?: string;
  // Defaults to "recently_added" on the server when omitted.
  sort?: ItemSort;
  page?: number;
  page_size?: number;
  /** Filter by parser confidence. `true` = clean metadata,
   *  `false` = stubs the scanner created when it couldn't
   *  fingerprint the filename. Omit for all items. Drives the
   *  "Unmatched files" admin surface. */
  auto_matched?: boolean;
}

export interface MediaStreamSummary {
  index: number;
  kind: string;
  codec: string | null;
  language: string | null;
  /// Embedded track title from the source (`title` MKV tag). Bluray /
  /// WEB-DL remuxes label every track ("Netflix eng subrip", "SDH
  /// eng subrip", etc.); using this in the player picker matches
  /// what users see in VLC / mpv / Haruna and disambiguates the
  /// half-dozen English-language subtitle variants you typically
  /// get in a remux.
  title: string | null;
  channels: number | null;
  is_default: boolean;
  is_forced: boolean;
}

export interface MediaFileSummary {
  id: number;
  container: string | null;
  duration_ms: number | null;
  bit_rate: number | null;
  width: number | null;
  height: number | null;
  hdr_format: string | null;
  size_bytes: number;
  streams: MediaStreamSummary[];
  markers: Marker[];
}

export interface Marker {
  kind: "intro" | "credits" | string;
  start_ms: number;
  end_ms: number;
  label: string | null;
  /// `auto` for markers written by the detect_markers task,
  /// `manual` for ones drawn in the operator editor. The player
  /// renders auto segments more prominently on the timeline so the
  /// user can see which areas were machine-detected.
  source: "auto" | "manual" | string;
}

/// Full marker row shape including id + source. Returned by the
/// operator-side marker editor; the player path consumes the slimmer
/// [`Marker`] above (no id/source) because client code never edits
/// markers directly.
export interface MarkerRow {
  id: number;
  kind: "intro" | "credits" | "commercial" | string;
  start_ms: number;
  end_ms: number;
  label: string | null;
  /// `auto` when written by the scheduled detect_markers task,
  /// `manual` when written by the operator's editor.
  source: "auto" | "manual" | string;
}

export interface MediaFileMarkersResponse {
  media_file_id: number;
  duration_ms: number | null;
  markers: MarkerRow[];
}

export interface ManualMarkerInput {
  kind: "intro" | "credits" | "commercial";
  start_ms: number;
  end_ms: number;
  label?: string | null;
}

// ─── Background job queue (admin) ─────────────────────────────────────────

export type JobStatusFilter = "queued" | "running" | "succeeded" | "failed" | "dead";

export interface JobSummary {
  queued: number;
  running: number;
  succeeded: number;
  failed: number;
  dead: number;
}

export interface JobRow {
  id: number;
  kind: string;
  payload: string;
  status: JobStatusFilter;
  priority: number;
  attempts: number;
  max_attempts: number;
  run_after: number;
  locked_at: number | null;
  last_error: string | null;
  created_at: number;
  started_at: number | null;
  finished_at: number | null;
}

export interface JobSweepCounts {
  markers: number;
  previews: number;
  chapter_thumbs: number;
  loudness: number;
}

export interface SeasonSummary {
  id: number;
  season_number: number;
  title: string | null;
  episode_count: number;
}

export interface Person {
  id: number;
  name: string;
  tmdb_id: number | null;
  imdb_id: string | null;
  photo_url: string | null;
  biography: string | null;
  birthday: number | null;
  deathday: number | null;
  place_of_birth: string | null;
  known_for_department: string | null;
}

export interface Credit {
  id: number;
  person: Person;
  role_kind: string;
  role: string;
  character_name: string | null;
  sort_order: number;
}

export interface Extra {
  id: number;
  kind: string;
  title: string;
  source: string;
  source_id: string;
  thumb_url: string | null;
  duration_ms: number | null;
  published_at: number | null;
}

export interface ReviewsSummary {
  count: number;
  average: number | null;
}

export interface Review {
  id: number;
  item_id: number;
  source: string;            // "tmdb"
  author: string;
  author_url: string | null;
  avatar_url: string | null;
  rating: number | null;     // 1-10 or null
  body: string | null;
  created_at: number;
}

export interface ItemEditInput {
  title?: string;
  sort_title?: string;
  original_title?: string;
  summary?: string;
  tagline?: string;
  year?: number;
  rating_age?: string;
  rating_audience?: number;
  /// Fields to unlock so future enrichment can write to them again.
  unlock?: string[];
}

export interface MatchCandidate {
  tmdb_id: number;
  kind: "movie" | "show";
  title: string;
  year: number | null;
  summary: string | null;
  poster_path: string | null;
}

export type ItemDetail = Item & {
  genres: string[];
  play_state: PlayStateForItem | null;
  files: MediaFileSummary[];
  seasons: SeasonSummary[];
  credits: Credit[];
  extras: Extra[];
  reviews: ReviewsSummary;
  locked_fields: string[];
  /// Per-user watched counts for shows. Absent for movies (whose
  /// watched state lives in `play_state`).
  watch_stats?: ShowWatchStats;
};

export interface ShowWatchStats {
  total_episodes: number;
  watched_episodes: number;
}

export interface Season {
  id: number;
  show_id: number;
  season_number: number;
  title: string | null;
  summary: string | null;
}

export interface Episode {
  id: number;
  season_id: number;
  show_id: number;
  season_number: number;
  episode_number: number;
  title: string;
  summary: string | null;
  air_date: number | null;
  duration_ms: number | null;
  thumb_path: string | null;
  added_at: number;
  updated_at: number;
}

export type EpisodeListed = Episode & { play_state: PlayStateForItem | null };

export type SeasonDetail = Season & { episodes: EpisodeListed[] };

export type EpisodeDetail = Episode & {
  play_state: PlayStateForItem | null;
  files: MediaFileSummary[];
};

export type OnDeckEntry =
  | { kind: "movie"; item: Item; play_state: PlayStateForItem }
  | {
      kind: "episode";
      episode: Episode;
      show: Item;
      play_state: PlayStateForItem;
    };

export interface OnDeckResponse {
  items: OnDeckEntry[];
}

export type ScanStatus =
  | "queued"
  | "running"
  | "completed"
  | "failed"
  | "canceled";

export interface ScanJob {
  id: number;
  library_id: number;
  status: ScanStatus;
  started_at: number | null;
  finished_at: number | null;
  files_seen: number;
  files_added: number;
  files_updated: number;
  files_removed: number;
  error_message: string | null;
  created_at: number;
}

export interface ClientCapabilities {
  supported_video_codecs?: string[];
  supported_audio_codecs?: string[];
  supported_containers?: string[];
  max_bandwidth_bps?: number | null;
}

export interface CreateSessionInput {
  media_file_id: number;
  start_position_ms?: number;
  client: ClientCapabilities;
  // 0-indexed among the file's audio streams (omit = default).
  audio_index?: number;
  // 0-indexed among the file's subtitle streams. Setting this forces a
  // transcode session and burns the chosen subtitle into the video.
  subtitle_index?: number;
  // Pre-built ASS force_style string (e.g. "Fontsize=28,PrimaryColour=&H00FFFFFF&,...").
  // The transcoder appends this to ffmpeg's `subtitles=` filter so the
  // burned-in track honors the user's font/color/background prefs. Only
  // applies to text subtitles; picture subtitles (PGS/DVD) ignore it.
  subtitle_style?: string;
  // Explicit quality target (omit = let the server decide). When set
  // the session always transcodes — direct play can't change bitrate.
  quality_target?: {
    height: number;
    bitrate_bps: number;
  };
  // EBU R128 audio loudness normalization. When true, the backend
  // forces audio re-encode and applies the `loudnorm` filter so this
  // title sits at the same perceived volume as other normalized
  // titles. Backed by a per-device pref (audioNormalize).
  audio_normalize?: boolean;
  // User-tunable subtitle sync offset in milliseconds. Positive =
  // subs delayed, negative = subs advanced. The backend re-shifts
  // the cached WebVTT by this amount, so the player just reloads
  // the new sidecar — no client-side cue manipulation. Saved
  // per (user, mediaFileId) in localStorage so the same correction
  // sticks across replays. Defaults to 0 (no offset).
  subtitle_offset_ms?: number;
}

export interface SessionInfo {
  id: string;
  mode: "direct" | "transcode";
  direct_url?: string;
  hls_master_url?: string;
  media_file_id: number;
  /// Source-time (ms) the session was started at. For freshly-created
  /// sessions this echoes the request `start_position_ms`; for
  /// prewarmed sessions returned by `/stream/prewarm` the resolver
  /// pulled it from the user's saved play_state. The player uses it
  /// to decide whether a cached prewarm matches what it's about to
  /// play — a wide gap (user scrubbed) is a cache miss.
  start_position_ms: number;
  duration_ms: number | null;
  /// Transcode sessions only. The actual height the encoder settled
  /// on after applying source-based defaults and the operator's
  /// quality ceiling — `resolved_height` of 1080 with the user's
  /// pick of "Auto" lets the player render "Auto · 1080p".
  resolved_height?: number;
  resolved_video_bitrate_bps?: number;
  /// Native source height (px). Player uses this to grey out
  /// impractical quality tiers (asking for 1080p on a 720p source
  /// is pointless — the scale filter caps at source anyway).
  source_height?: number;
  /// Human label for the active encoder ("NVIDIA NVENC", "software
  /// (libx264)", etc.). Only set for transcode sessions.
  encoder?: string;
  /// "copy" / "reencode" per stream — lets the picker show a remux
  /// chip when the cheap path is taken.
  video_treatment?: "copy" | "reencode";
  audio_treatment?: "copy" | "reencode";
}

export interface CreateSessionResponse {
  session: SessionInfo;
}

/// Body for `POST /stream/prewarm`. `rating_key` is the same opaque
/// slug the modal Play link uses ("e<id>" for an episode, "<id>" for
/// a movie or show); the backend resolves it to a media_file_id +
/// resume position via the same rules `/watch/[ratingKey]` would.
export interface PrewarmInput {
  rating_key: string;
  client: ClientCapabilities;
  audio_normalize?: boolean;
}

export interface PlayStateUpdateInput {
  item_id?: number;
  episode_id?: number;
  position_ms: number;
  duration_ms?: number;
  watched?: boolean;
}

export interface ScrobbleInput {
  item_id?: number;
  episode_id?: number;
}

/// Invite shape as returned by list / create. The plaintext token is
/// never exposed here — only the hash and metadata. The plaintext is
/// available exactly once in [`CreatedInvite`] returned by the create
/// endpoint.
export interface Invite {
  id: number;
  created_by: number;
  expires_at: number | null;
  consumed_by: number | null;
  consumed_at: number | null;
  email: string | null;
  /// Epoch ms when the invite email was successfully sent. null when
  /// SMTP wasn't configured or the send failed.
  sent_at: number | null;
  created_at: number;
}

/// List entry — adds the pre-bound library + group IDs to the bare Invite.
export interface InviteListEntry extends Invite {
  library_ids: number[];
  group_ids: number[];
}

/// Returned only at issuance. Capture and share `code` / `accept_url`
/// immediately — the server never reveals them again.
export interface CreatedInvite {
  invite: Invite;
  code: string;
  accept_url: string | null;
  email_sent: boolean;
  library_ids: number[];
  group_ids: number[];
}

export interface CreateInviteInput {
  expires_in_seconds?: number;
  email?: string;
  library_ids?: number[];
  group_ids?: number[];
}

// ── Access groups (Phase 27) ──────────────────────────────────────

/// Named bulk-assignment of library access. A user's effective set is
/// the UNION of direct library_access rows and the libraries of every
/// group they belong to.
export interface AccessGroup {
  id: number;
  name: string;
  description: string | null;
  created_at: number;
  updated_at: number;
  member_count: number;
  library_count: number;
}

export interface AccessGroupDetail extends AccessGroup {
  member_ids: number[];
  library_ids: number[];
}

export interface NewAccessGroupInput {
  name: string;
  description?: string;
}

export interface AccessGroupUpdateInput {
  name?: string;
  /** null clears the field; omit to leave unchanged. */
  description?: string | null;
}

export interface HealthResponse {
  status: "ok";
  version: string;
  uptime_s: number;
}

export interface LibraryCounts {
  libraries: number;
  movies: number;
  shows: number;
  episodes: number;
}

export interface ServerInfoResponse {
  version: string;
  library_counts: LibraryCounts;
  tmdb_enabled: boolean;
}

// ─── Admin: server settings & audit log ────────────────────────────────────

export type SecureConnectionsMode = "required" | "preferred" | "disabled";
export type TranscoderHwAccel =
  | "auto"
  | "none"
  | "vaapi"
  | "nvenc"
  | "qsv"
  | "videotoolbox"
  | "amf";

export interface ServerSettings {
  server_name: string;
  public_url: string | null;
  /** JSON-encoded array of allowed CORS origins. */
  cors_origins: string;
  secure_connections: SecureConnectionsMode;
  telemetry_opt_in: boolean;
  /** True once the first-run onboarding wizard has been completed
   *  or explicitly skipped. Drives the post-login redirect to
   *  `/onboarding` for owners on a fresh install. */
  setup_completed: boolean;
  transcoder_max_concurrent: number;
  transcoder_hw_accel: TranscoderHwAccel;
  transcoder_quality_ceiling_kbps: number | null;
  transcoder_encoder_preset: TranscoderEncoderPreset;
  transcoder_hw_strictness: TranscoderHwStrictness;
  // ── Transcoder extras (Phase 30) ──
  /** libx264 preset used by the optimize_versions task. */
  transcoder_background_preset: TranscoderBackgroundPreset;
  /** Cap on background optimize-versions concurrency per scheduler tick. */
  transcoder_max_background_concurrent: number;
  /** Worker count for the durable job queue (markers/sprites/thumbs/loudness). */
  job_workers: number;
  /** JSON object mapping job_kind → concurrency cap. Empty `{}` =
   *  registry defaults for every kind. Hot-reloadable. */
  job_kind_concurrency: string;
  /** When true (default), HDR sources are tone-mapped to SDR. */
  transcoder_hdr_tonemap_enabled: boolean;
  /** Algorithm passed to ffmpeg's tonemap filter. */
  transcoder_hdr_tonemap_algo: TonemapAlgorithm;
  /** HEVC output mode. `off` (default): always H.264.
   *  `when_client_supports`: HEVC for clients that report HEVC decode,
   *  H.264 otherwise. `always`: force HEVC every session (breaks
   *  playback on Firefox / older Chrome — only safe on Safari-only
   *  deployments). */
  transcoder_hevc_encoding_mode: HevcMode;
  /** GPU device override: "auto", a numeric NVENC index, or a
   *  /dev/dri/renderD<N> VAAPI path. */
  transcoder_gpu_device: string;
  /** Separate cap for software (libx264/libx265) sessions on top of
   *  `transcoder_max_concurrent`. Default 1 — keeps CPU encodes from
   *  starving parallel GPU sessions. */
  transcoder_max_cpu_concurrent: number;
  // ── Email / SMTP (Phase 21) ──
  email_smtp_host: string | null;
  email_smtp_port: number | null;
  email_smtp_username: string | null;
  email_smtp_security: SmtpSecurity | null;
  email_from_address: string | null;
  email_from_name: string | null;
  // ── 2FA (Phase 24) ──
  totp_enforcement: TotpEnforcement;
  // ── Maintenance window (Phase 29) ──
  /** HH:MM in server-local time. Start of the window during which
   *  background tasks marked `requires_maintenance_window` are allowed
   *  to run. Default 02:00. */
  maintenance_window_start: string;
  /** HH:MM in server-local time. End of the maintenance window.
   *  Default 09:00. If `<= start`, the window wraps midnight. */
  maintenance_window_end: string;
  // ── Library (Phase 34) ──
  /** When true (default), filesystem watcher fires library scans
   *  on file change. Read once at startup; toggling requires a
   *  server restart to take effect. */
  scan_automatically: boolean;
  /** Server-wide default for ffmpeg loudnorm. When ON, every transcode
   *  session gets the filter applied (uses stored per-file
   *  measurements when available, else generic targets). Per-session
   *  override possible via the player audio menu. */
  audio_normalize_enabled: boolean;
  /** `nice -n N` wrapper around ffmpeg in background contexts
   *  (scheduled tasks, scanner probes). 0 disables. Restart required. */
  scanner_nice_level: number;
  /** Filename of the operator-uploaded pre-roll relative to
   *  `<data_dir>/preroll/`. Null when none configured. */
  preroll_path: string | null;
  /** Master switch for pre-roll playback. */
  preroll_enabled: boolean;
  /** Output level for the pre-roll, 0..=100. Defaults to 100. */
  preroll_volume: number;
  // ── Playback / library (Phase 31) ──
  /** Hard cap on the Continue Watching rail. Default 40. */
  continue_watching_max_items: number;
  /** In-progress items last played more than this many weeks ago are
   *  filtered out of the rail. 0 disables the time-window filter. */
  continue_watching_max_age_weeks: number;
  /** When true (default), surfaces S(N+1)E01 of any show the user
   *  has watched as a Continue Watching tile when a new season
   *  exists and they haven't started it. Plex parity. */
  continue_watching_include_premieres: boolean;
  /** Single threshold (1-99) for "this counts as watched". Used by
   *  the client to auto-scrobble and by the on-deck query as its
   *  upper bound. Default 90. */
  video_played_threshold_pct: number;
  /** What counts as "watched": pure percentage, the auto-detected
   *  credits marker, or whichever comes first. Default
   *  `threshold_pct`. See migration phase 46. */
  video_completion_behaviour: CompletionBehaviour;
  /** Megabytes of SQLite page cache per connection. 0 = SQLite
   *  default (~2 MiB). Applied at next server restart. Default 64. */
  database_cache_size_mb: number;
  /** BCP-47 tag (e.g. `en-US`, `ja-JP`) sent to TMDB on every
   *  metadata fetch. TMDB returns text in this language when
   *  available; falls back to original language silently when no
   *  translation exists. Restart required for changes to take effect
   *  — existing items keep their text until a Refresh metadata. */
  metadata_language: string;
  /** Days a freshly-added item stays badged as "Recently Added" on
   *  Card. 0 disables the badge entirely. Default 14. Takes effect
   *  on the next config-poll — no rebuild required. */
  recently_added_days: number;
  /** JSON-encoded object used by later phases for forward-compat fields. */
  extras_json: string;
  updated_at: number;
  updated_by: number | null;
}

export type SmtpSecurity = "starttls" | "tls" | "none";

/// Global 2FA policy. "disabled" blocks new enrollments; "optional" is
/// the default; "required" forces every user to enroll before login
/// completes.
export type TotpEnforcement = "disabled" | "optional" | "required";

/** Patch shape. Every field optional — only included keys are updated. */
export interface ServerSettingsUpdate {
  server_name?: string;
  public_url?: string | null;
  cors_origins?: string;
  secure_connections?: SecureConnectionsMode;
  telemetry_opt_in?: boolean;
  setup_completed?: boolean;
  transcoder_max_concurrent?: number;
  transcoder_hw_accel?: TranscoderHwAccel;
  transcoder_quality_ceiling_kbps?: number | null;
  transcoder_encoder_preset?: TranscoderEncoderPreset;
  transcoder_hw_strictness?: TranscoderHwStrictness;
  transcoder_background_preset?: TranscoderBackgroundPreset;
  transcoder_max_background_concurrent?: number;
  job_workers?: number;
  job_kind_concurrency?: string;
  transcoder_hdr_tonemap_enabled?: boolean;
  transcoder_hdr_tonemap_algo?: TonemapAlgorithm;
  transcoder_hevc_encoding_mode?: HevcMode;
  transcoder_gpu_device?: string;
  transcoder_max_cpu_concurrent?: number;
  email_smtp_host?: string | null;
  email_smtp_port?: number | null;
  email_smtp_username?: string | null;
  email_smtp_security?: SmtpSecurity | null;
  email_from_address?: string | null;
  email_from_name?: string | null;
  totp_enforcement?: TotpEnforcement;
  maintenance_window_start?: string;
  maintenance_window_end?: string;
  scan_automatically?: boolean;
  audio_normalize_enabled?: boolean;
  scanner_nice_level?: number;
  preroll_enabled?: boolean;
  preroll_volume?: number;
  continue_watching_max_items?: number;
  continue_watching_max_age_weeks?: number;
  continue_watching_include_premieres?: boolean;
  video_played_threshold_pct?: number;
  video_completion_behaviour?: CompletionBehaviour;
  database_cache_size_mb?: number;
  metadata_language?: string;
  recently_added_days?: number;
  extras_json?: string;
}

export interface EmailStatusResponse {
  /** Required fields (host + from address) all present. */
  configured: boolean;
  /** Password is set in the vault. The value itself is never returned. */
  has_password: boolean;
  smtp_host: string | null;
  smtp_port: number | null;
  smtp_username: string | null;
  smtp_security: SmtpSecurity | null;
  from_address: string | null;
  from_name: string | null;
}

export interface EmailTestResponse {
  ok: boolean;
  /** Human-readable diagnostic — "SMTP handshake succeeded" or
   * "test email sent to alice@example.com" etc. */
  message: string;
}

/** Operator's speed-vs-quality dial for the encoder. */
export type TranscoderEncoderPreset = "speed" | "balanced" | "quality";

/** How strictly to enforce hardware acceleration:
 *  - `auto`: HW where possible, SW fallback per stage (current default).
 *  - `prefer_hw`: same effective behavior as auto but warns when a
 *    session falls back to software.
 *  - `require_hw`: refuse the session if any stage can't run on HW.
 *    Player gets a 409 with the specific reason.
 */
export type TranscoderHwStrictness = "auto" | "prefer_hw" | "require_hw";

export type HevcMode = "off" | "when_client_supports" | "always";

export type CompletionBehaviour =
  | "threshold_pct"
  | "first_credits_marker"
  | "earliest_of_both";

/** libx264 preset values accepted by the optimize_versions task. */
export type TranscoderBackgroundPreset =
  | "ultrafast"
  | "superfast"
  | "veryfast"
  | "faster"
  | "fast"
  | "medium"
  | "slow"
  | "slower";

/** Algorithms accepted by ffmpeg's `tonemap=tonemap=<algo>` filter. */
export type TonemapAlgorithm =
  | "hable"
  | "reinhard"
  | "mobius"
  | "bt2390"
  | "clip"
  | "linear";

export interface AuditLogEntry {
  id: number;
  actor_user_id: number | null;
  action: string;
  target_kind: string | null;
  target_id: string | null;
  payload_json: string | null;
  ip: string | null;
  user_agent: string | null;
  created_at: number;
}

export interface AuditListResponse {
  entries: AuditLogEntry[];
  next_before: number | null;
  /** Total rows in audit_log — drives the paginated admin UI's
   *  "X–Y of Z" footer and jump-to-page buttons. */
  total: number;
}

// ─── External subtitles (Phase 12a — OpenSubtitles agent) ─────────────────

export interface ExternalSubtitle {
  id: number;
  item_id: number | null;
  episode_id: number | null;
  language: string;
  source: string;
  source_file_id: string | null;
  file_path: string;
  forced: boolean;
  sdh: boolean;
  created_at: number;
}

export const externalSubtitles = {
  forItem: (itemId: number) =>
    apiFetch<{ subtitles: ExternalSubtitle[] }>(
      `/items/${itemId}/external-subtitles`,
    ),
  forEpisode: (episodeId: number) =>
    apiFetch<{ subtitles: ExternalSubtitle[] }>(
      `/episodes/${episodeId}/external-subtitles`,
    ),
  fileUrl: (id: number) => `/api/v1/external-subtitles/${id}/file`,
};

// ─── Scrub-preview sprites (Phase 12b) ─────────────────────────────────────

export interface PreviewManifest {
  sprite_url: string;
  interval_ms: number;
  tile_width: number;
  tile_height: number;
  tile_cols: number;
  tile_count: number;
}

export const previews = {
  manifest: (mediaFileId: number) =>
    apiFetch<PreviewManifest>(`/media-files/${mediaFileId}/preview/manifest`),
};

// ─── Chapter thumbnails (Phase 38) ─────────────────────────────────────────

export interface ChapterEntry {
  index: number;
  start_ms: number;
  end_ms: number;
  title: string | null;
  /// Server-relative URL; null when the generate_chapter_thumbs task
  /// hasn't run for this file yet (or extraction failed).
  thumb_url: string | null;
}

export interface ChaptersResponse {
  chapters: ChapterEntry[];
  thumbs_ready: boolean;
}

export const chapters = {
  list: (mediaFileId: number) =>
    apiFetch<ChaptersResponse>(`/media-files/${mediaFileId}/chapters`),
};

// ─── Pre-roll video (Phase 42) ─────────────────────────────────────────────

export interface PrerollStatus {
  enabled: boolean;
  configured: boolean;
  url: string | null;
  size_bytes: number | null;
  /** Output level for the pre-roll, 0..=100. Applied as `video.volume`
   *  in the gate so the player doesn't need a second settings fetch. */
  volume: number;
}

// ─── Bulk item operations (Phase 43) ───────────────────────────────────────

export interface BulkReport {
  ok: number;
  failed: number;
  errors: { item_id: number; error: string }[];
}

export const bulkItems = {
  refreshMetadata: (item_ids: number[]) =>
    apiFetch<BulkReport>("/admin/items/bulk/refresh-metadata", {
      method: "POST",
      body: { item_ids },
    }),
  addTag: (item_ids: number[], tag_name: string) =>
    apiFetch<BulkReport>("/admin/items/bulk/add-tag", {
      method: "POST",
      body: { item_ids, tag_name },
    }),
  removeTag: (item_ids: number[], tag_name: string) =>
    apiFetch<BulkReport>("/admin/items/bulk/remove-tag", {
      method: "POST",
      body: { item_ids, tag_name },
    }),
  detectMarkers: (item_ids: number[]) =>
    apiFetch<BulkReport>("/admin/items/bulk/detect-markers", {
      method: "POST",
      body: { item_ids },
    }),
};

export const preroll = {
  status: () => apiFetch<PrerollStatus>("/admin/preroll"),
  clear: () =>
    apiFetch<void>("/admin/preroll", { method: "DELETE" }),
  upload: async (file: File) => {
    const fd = new FormData();
    fd.append("file", file);
    // Mirror the CSRF + cookie handling that `apiFetch` does for
    // every other mutating call. We bypass `apiFetch` here because
    // it forces a JSON body, but the double-submit CSRF middleware
    // still requires the `X-CSRF-Token` header to match the
    // `cf_csrf` cookie — without it the server returns
    // `csrf_token_missing` and the upload 403s. Don't set
    // Content-Type explicitly so the browser injects the
    // multipart boundary.
    const headers: Record<string, string> = {};
    if (typeof document !== "undefined") {
      const csrf = csrfFromCookieString(document.cookie);
      if (csrf) headers["X-CSRF-Token"] = csrf;
    }
    const res = await fetch(`/api/v1/admin/preroll`, {
      method: "POST",
      body: fd,
      credentials: "include",
      headers,
    });
    if (!res.ok) {
      const text = await res.text().catch(() => "");
      throw new Error(text || `pre-roll upload failed: ${res.status}`);
    }
    return (await res.json()) as PrerollStatus;
  },
};

// ─── Trakt sync + ratings (Phase 15) ───────────────────────────────────────

export interface TraktLinkStart {
  user_code: string;
  verification_url: string;
  expires_in: number;
  interval: number;
}

export type TraktLinkPoll =
  | { status: "pending" }
  | { status: "ready" }
  | { status: "expired" }
  | { status: "denied" }
  | { status: "slow_down" };

export interface TraktStatus {
  linked: boolean;
  linked_at: number | null;
  last_synced_at: number | null;
  scope: string | null;
  app_configured: boolean;
}

export interface TraktSyncNowResult {
  movies_marked: number;
  episodes_marked: number;
  playback_applied: number;
}

export const trakt = {
  status: () => apiFetch<TraktStatus>("/trakt/status"),
  linkStart: () =>
    apiFetch<TraktLinkStart>("/trakt/link/start", { method: "POST" }),
  linkPoll: () => apiFetch<TraktLinkPoll>("/trakt/link/poll", { method: "POST" }),
  unlink: () =>
    apiFetch<{ removed: boolean }>("/trakt/unlink", { method: "POST" }),
  syncNow: () =>
    apiFetch<TraktSyncNowResult>("/trakt/sync-now", { method: "POST" }),
};

export interface RatingResponse {
  rating: number | null;
}

export const ratings = {
  getItem: (itemId: number) =>
    apiFetch<RatingResponse>(`/items/${itemId}/rating`),
  putItem: (itemId: number, rating: number) =>
    apiFetch<RatingResponse>(`/items/${itemId}/rating`, {
      method: "PUT",
      body: { rating },
    }),
  deleteItem: (itemId: number) =>
    apiFetch<RatingResponse>(`/items/${itemId}/rating`, { method: "DELETE" }),
  getEpisode: (episodeId: number) =>
    apiFetch<RatingResponse>(`/episodes/${episodeId}/rating`),
  putEpisode: (episodeId: number, rating: number) =>
    apiFetch<RatingResponse>(`/episodes/${episodeId}/rating`, {
      method: "PUT",
      body: { rating },
    }),
  deleteEpisode: (episodeId: number) =>
    apiFetch<RatingResponse>(`/episodes/${episodeId}/rating`, {
      method: "DELETE",
    }),
};

// ─── Tags (Phase 14) ───────────────────────────────────────────────────────

export interface Tag {
  id: number;
  name: string;
}

export const tags = {
  list: () => apiFetch<{ tags: Tag[] }>("/tags"),
  forItem: (itemId: number) =>
    apiFetch<{ tags: Tag[] }>(`/items/${itemId}/tags`),
  add: (itemId: number, name: string) =>
    apiFetch<Tag>(`/items/${itemId}/tags`, {
      method: "POST",
      body: { name },
    }),
  remove: (itemId: number, tagId: number) =>
    apiFetch<void>(`/items/${itemId}/tags/${tagId}`, { method: "DELETE" }),
};

// ─── Admin: library health report ──────────────────────────────────────────

export interface MissingFileRow {
  id: number;
  path: string;
  item_title: string | null;
  episode_title: string | null;
}

export interface LibraryNoPathRow {
  id: number;
  name: string;
}

export interface LibraryHealthResponse {
  items_without_files: number;
  items_without_metadata: number;
  items_without_poster: number;
  items_without_backdrop: number;
  orphan_episodes: number;
  orphan_media_files: number;
  missing_files: MissingFileRow[];
  libraries_without_paths: LibraryNoPathRow[];
}

/// Category keys for the `library-health/items` drill-in. Matches
/// the server-side allowlist verbatim — anything else returns 400.
export type LibraryHealthCategory =
  | "no_files"
  | "no_metadata"
  | "no_poster"
  | "no_backdrop"
  | "orphan_episodes"
  | "orphan_media_files";

export interface LibraryHealthItemRow {
  id: number;
  kind: "item" | "episode" | "media_file";
  title: string;
  subtitle: string | null;
  library_name: string | null;
  /// items.id to pass to `?modal=<id>` for click-through. None when
  /// the row IS an orphan with no parent item to anchor.
  item_id_for_modal: number | null;
  /// Full filesystem path — only populated for `media_file` rows.
  path: string | null;
}

export interface LibraryHealthItemsResponse {
  category: LibraryHealthCategory;
  total: number;
  rows: LibraryHealthItemRow[];
}

// ─── Admin: credential vault ───────────────────────────────────────────────

export interface SecretMetadata {
  name: string;
  set: boolean;
  /// Last four characters of the stored value, or "????" if the vault
  /// can't decrypt the row (e.g. master key changed underneath).
  last4: string;
  updated_at: number;
  updated_by: number | null;
}

export interface SecretSlotView {
  name: string;
  display_name: string;
  description: string;
  /// System-managed slots (currently session_hmac) reject PUT/DELETE.
  managed: boolean;
  stored: SecretMetadata | null;
}

export interface SecretsListResponse {
  encrypted_at_rest: boolean;
  slots: SecretSlotView[];
}

export interface SecretTestResponse {
  ok: boolean;
  detail: string;
}

export interface DashboardServerStatus {
  version: string;
  started_at_ms: number;
  uptime_s: number;
  now_ms: number;
}

export interface DashboardLibraryStats {
  library_id: number;
  name: string;
  kind: string;
  item_count: number;
  file_count: number;
  total_bytes: number;
}

export interface DashboardSession {
  id: string;
  user_id: number;
  media_file_id: number;
  start_position_ms: number;
  duration_ms: number | null;
  created_at: number;
  last_seen_at: number;
  /// Human label for the encoder ffmpeg is running for this session,
  /// e.g. "NVIDIA NVENC" or "software (libx264)". Comes from the
  /// transcoder so the operator can confirm hardware acceleration is
  /// actually taking effect.
  encoder: string;
  /// Whether ffmpeg is re-encoding the video stream or just copying
  /// source packets. `copy` is the fast path (audio-swap only on a
  /// client-compatible source); `reencode` is everything else.
  video_treatment: "copy" | "reencode";
  /// Same for the audio stream — `copy` when the source codec is
  /// already in the client's supported list (usually AAC).
  audio_treatment: "copy" | "reencode";
  /// Source / target heights in pixels. Lets the dashboard render
  /// "4K → 1080p" so the operator can see at a glance what each
  /// session is actually doing.
  source_height: number | null;
  target_height: number;
  /// Encoder bitrate in bps. Matches the BANDWIDTH advertised on
  /// the HLS master playlist.
  target_video_bitrate_bps: number;
  /// Operator's speed-vs-quality preset for the session ("speed",
  /// "balanced", "quality"). The transcoder maps this to the
  /// encoder's own preset vocabulary (libx264 ultrafast/veryfast/
  /// medium, NVENC p1/p4/p6, etc.).
  encoder_preset: string;
}

export interface DashboardScanJob {
  id: number;
  library_id: number;
  status: ScanStatus;
  started_at: number | null;
  finished_at: number | null;
  files_seen: number;
  files_added: number;
  files_updated: number;
  files_removed: number;
  error_message: string | null;
  created_at: number;
}

export interface DashboardDisk {
  path: string;
  label: string;
  total_bytes: number;
  used_bytes: number;
}

export interface DashboardResponse {
  server: DashboardServerStatus;
  library_stats: DashboardLibraryStats[];
  active_transcodes: DashboardSession[];
  recent_scans: DashboardScanJob[];
  disks: DashboardDisk[];
}

// ---------------------------------------------------------------------------
// Fetch core
// ---------------------------------------------------------------------------

export class ChimpFlixApiError extends Error {
  constructor(
    readonly status: number,
    readonly body: string,
  ) {
    super(`chimpflix api ${status}: ${body || "(empty body)"}`);
    this.name = "ChimpFlixApiError";
  }

  /// Returns the user-facing `.error.message` from the JSON body when
  /// present, otherwise null. Callers that want a clean modal banner
  /// should prefer this over `.message` (which is the raw debug form
  /// used for unrecognised error shapes).
  get friendlyMessage(): string | null {
    if (!this.body) return null;
    try {
      const parsed = JSON.parse(this.body) as unknown;
      if (
        parsed &&
        typeof parsed === "object" &&
        "error" in parsed &&
        parsed.error &&
        typeof parsed.error === "object" &&
        "message" in parsed.error &&
        typeof (parsed.error as { message: unknown }).message === "string"
      ) {
        return (parsed.error as { message: string }).message;
      }
    } catch {
      // Body wasn't JSON — fall through to null.
    }
    return null;
  }
}

/// Pull the friendliest message out of any unknown error — prefers the
/// server-supplied `.error.message`, falls back to `.message`, then
/// `String(e)`. Used by every catch path that surfaces an error to the
/// UI without needing to know whether it's an API error or something
/// else.
export function friendlyErrorMessage(e: unknown): string {
  if (e instanceof ChimpFlixApiError) {
    return e.friendlyMessage ?? e.message;
  }
  if (e instanceof Error) return e.message;
  return String(e);
}

type QueryValue =
  | string
  | number
  | boolean
  | ReadonlyArray<string | number>
  | undefined
  | null;

interface FetchOptions {
  method?: "GET" | "POST" | "PUT" | "PATCH" | "DELETE";
  body?: unknown;
  query?: Record<string, QueryValue>;
  cache?: RequestCache;
  // Disable cookie forwarding for endpoints that don't require auth.
  noAuth?: boolean;
}

const isServer = () => typeof window === "undefined";

function buildUrl(path: string, query?: FetchOptions["query"]): string {
  const base = isServer()
    ? (process.env.CHIMPFLIX_API_URL ?? "http://server:8080") + "/api/v1"
    : "/api/v1";
  const url = new URL(
    base.replace(/\/+$/, "") + (path.startsWith("/") ? path : `/${path}`),
    isServer() ? undefined : window.location.origin,
  );
  if (query) {
    for (const [k, v] of Object.entries(query)) {
      if (v === undefined || v === null) continue;
      if (Array.isArray(v)) {
        // Server-side ItemFilter::library_ids and other csv-shaped filters
        // expect comma-separated values, not repeated `?k=…&k=…` params.
        url.searchParams.set(k, v.join(","));
      } else {
        url.searchParams.set(k, String(v));
      }
    }
  }
  return url.toString();
}

async function serverCookieHeader(): Promise<string> {
  const { cookies } = await import("next/headers");
  const jar = await cookies();
  return jar
    .getAll()
    .map((c) => `${c.name}=${c.value}`)
    .join("; ");
}

/// Pluck the CSRF companion cookie value from any Cookie-header-shaped
/// string. Used by both the SSR path (cookies served from Next's cookie
/// jar) and the client path (parsing document.cookie). Cookie name is
/// `__Host-cf_csrf` over HTTPS, `cf_csrf` on plain HTTP — try both.
function csrfFromCookieString(raw: string): string | null {
  for (const chunk of raw.split(";")) {
    const trimmed = chunk.trim();
    if (trimmed.startsWith("__Host-cf_csrf=")) {
      return trimmed.slice("__Host-cf_csrf=".length);
    }
    if (trimmed.startsWith("cf_csrf=")) {
      return trimmed.slice("cf_csrf=".length);
    }
  }
  return null;
}

/// Client-side CSRF read from `document.cookie`. Returns null on the
/// server (the SSR path uses csrfFromCookieString directly against the
/// Next-supplied cookie header). Exported so callers that bypass
/// `apiFetch` (e.g. the player's teardown path, which uses raw fetch
/// + keepalive for reliability during unload) can still attach the
/// token; without it the server's CSRF middleware rejects with 403.
export function readCsrfToken(): string | null {
  if (typeof document === "undefined") return null;
  return csrfFromCookieString(document.cookie);
}

/// Normalize a user-supplied "next" / return-to path to one that we
/// know is safe to `router.push(...)`. Returns the path verbatim only
/// when it is unambiguously local: starts with a single `/`, is not
/// protocol-relative (`//host/...`), is not a backslash-trick
/// (`/\evil.com`), and doesn't carry a colon in the first segment
/// that could be parsed as a scheme (`/x:/y` is fine but `:javascript`
/// is not — we reject anything before the first `/` containing `:`).
/// Anything else returns the fallback.
///
/// Used by post-login redirects and any other code path that takes a
/// "where should we send the user next" query parameter. Without
/// validation, `?next=https://attacker.com` flows into `router.push`
/// and Next.js happily navigates off-origin.
export function safeLocalPath(
  candidate: string | null | undefined,
  fallback = "/",
): string {
  if (!candidate) return fallback;
  // Must start with exactly one '/' followed by something that isn't
  // another '/' (which would make it protocol-relative).
  if (!candidate.startsWith("/")) return fallback;
  if (candidate.startsWith("//")) return fallback;
  // Backslash trick: some browsers normalize `\` to `/`, so
  // `/\evil.com` becomes `//evil.com`.
  if (candidate.startsWith("/\\")) return fallback;
  // Reject control characters that could break the URL boundary.
  if (/[\x00-\x1f]/.test(candidate)) return fallback;
  return candidate;
}

async function apiFetch<T>(path: string, opts: FetchOptions = {}): Promise<T> {
  const url = buildUrl(path, opts.query);
  const headers: Record<string, string> = {};
  if (opts.body !== undefined) headers["Content-Type"] = "application/json";
  let serverCookies: string | null = null;
  if (isServer() && !opts.noAuth) {
    const cookieHeader = await serverCookieHeader();
    if (cookieHeader) {
      headers["Cookie"] = cookieHeader;
      serverCookies = cookieHeader;
    }
  }
  // CSRF double-submit token: read the non-HttpOnly `cf_csrf`
  // (or `__Host-cf_csrf` over HTTPS) cookie and echo its value in
  // X-CSRF-Token on every state-changing request. The server rejects
  // mutating requests where cookie != header. GETs / HEADs are exempt.
  const method = (opts.method ?? "GET").toUpperCase();
  const isMutating = method !== "GET" && method !== "HEAD" && method !== "OPTIONS";
  if (isMutating) {
    // SSR path: pull the token from the forwarded cookie header we
    // just built. Client path: parse document.cookie.
    const csrf = serverCookies
      ? csrfFromCookieString(serverCookies)
      : readCsrfToken();
    if (csrf) headers["X-CSRF-Token"] = csrf;
  }
  const res = await fetch(url, {
    method: opts.method ?? "GET",
    headers,
    body: opts.body !== undefined ? JSON.stringify(opts.body) : undefined,
    cache: opts.cache ?? "no-store",
    credentials: isServer() ? undefined : "include",
  });
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new ChimpFlixApiError(res.status, text);
  }
  // 204 (No Content) and 205 (Reset Content) have no body by spec.
  // Also bail when the server explicitly says Content-Length: 0,
  // since reading json() on an empty body throws "unexpected end of
  // data" and surfaces to the user as a misleading "JSON parse error".
  if (
    res.status === 204 ||
    res.status === 205 ||
    res.headers.get("content-length") === "0"
  ) {
    return undefined as T;
  }
  // 202 (Accepted) may or may not carry a body — some async-dispatch
  // endpoints fire-and-forget (no body), others return a queued count
  // or job descriptor. Read it as text first; if empty, treat as
  // body-less; otherwise parse JSON. Previously we unconditionally
  // discarded the body and returned undefined, which broke any
  // caller that destructured the response (e.g. detect-markers'
  // `{ queued }`).
  if (res.status === 202) {
    const text = await res.text();
    if (!text) return undefined as T;
    try {
      return JSON.parse(text) as T;
    } catch {
      return undefined as T;
    }
  }
  return (await res.json()) as T;
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

export const auth = {
  status: () =>
    apiFetch<AuthStatusResponse>("/auth/status", { noAuth: true }),
  setup: (input: {
    username: string;
    password: string;
    display_name?: string;
    email?: string;
  }) =>
    apiFetch<AuthResponse>("/auth/setup", {
      method: "POST",
      body: input,
      noAuth: true,
    }),
  login: (input: { username: string; password: string }) =>
    apiFetch<LoginResponse>("/auth/login", {
      method: "POST",
      body: input,
      noAuth: true,
    }),
  /// Second step of login when the first returned `2fa_required`.
  /// Pass either `code` (6-digit TOTP) or `recovery_code` (one of the
  /// 10 single-use hex codes issued at enrollment).
  loginTwoFactor: (input: {
    challenge: string;
    code?: string;
    recovery_code?: string;
  }) =>
    apiFetch<AuthResponse>("/auth/2fa/login", {
      method: "POST",
      body: input,
      noAuth: true,
    }),
  logout: () => apiFetch<void>("/auth/logout", { method: "POST" }),
  /// Wipe every OTHER session for this user — keeps the current one
  /// alive. Returns the count of sessions that were terminated.
  revokeOtherSessions: () =>
    apiFetch<{ revoked: number }>("/auth/sessions/revoke-others", {
      method: "POST",
    }),
  /// Live sessions for the current user, newest activity first. The
  /// `current` flag marks the one that authenticated the request.
  listMySessions: () =>
    apiFetch<{ sessions: MySessionEntry[] }>("/auth/me/sessions"),
  /// Revoke a single session. Use the current session to log yourself
  /// out as a side effect, or another session to drop just that device.
  revokeMySession: (sessionId: number) =>
    apiFetch<void>(`/auth/me/sessions/${sessionId}`, { method: "DELETE" }),
  /// Self-service password change. Requires the current password as a
  /// re-auth. On success, every OTHER session is revoked.
  changePassword: (input: { current_password: string; new_password: string }) =>
    apiFetch<{ sessions_revoked: number }>("/auth/me/password", {
      method: "POST",
      body: input,
    }),
  /// Request a verification email to the NEW address. The change isn't
  /// applied until the link in that email is followed.
  requestEmailChange: (input: { new_email: string; password: string }) =>
    apiFetch<RequestEmailChangeResponse>("/auth/me/email/request-change", {
      method: "POST",
      body: input,
    }),
  /// Apply the email change. Token comes from the verification email.
  confirmEmailChange: (token: string) =>
    apiFetch<{ email: string }>("/auth/me/email/confirm", {
      method: "POST",
      body: { token },
    }),
  me: () => apiFetch<AuthResponse>("/auth/me"),
  updateMe: (input: UpdateMeInput) =>
    apiFetch<AuthResponse>("/auth/me", { method: "PATCH", body: input }),
  /// Request a password-reset email. Always succeeds (204) regardless
  /// of whether the email matches a registered user — anti-enumeration.
  requestPasswordReset: (email: string) =>
    apiFetch<void>("/auth/password-reset/request", {
      method: "POST",
      body: { email },
      noAuth: true,
    }),
  /// Redeem the token + set a new password. Returns the count of other
  /// sessions that were revoked (so the UI can surface "we signed out
  /// N other devices").
  confirmPasswordReset: (input: { token: string; new_password: string }) =>
    apiFetch<{ sessions_revoked: number }>("/auth/password-reset/confirm", {
      method: "POST",
      body: input,
      noAuth: true,
    }),
  register: (input: {
    code: string;
    username: string;
    password: string;
    display_name?: string;
  }) => apiFetch<AuthResponse>("/auth/register", { method: "POST", body: input }),
  listInvites: () =>
    apiFetch<{ invites: InviteListEntry[] }>("/auth/invites"),
  createInvite: (input: CreateInviteInput) =>
    apiFetch<CreatedInvite>("/auth/invites", { method: "POST", body: input }),
  revokeInvite: (id: number) =>
    apiFetch<void>(`/auth/invites/${id}`, { method: "DELETE" }),
  listUsers: () => apiFetch<{ users: User[] }>("/auth/users"),
  deleteUser: (id: number) =>
    apiFetch<void>(`/auth/users/${id}`, { method: "DELETE" }),
  // ── 2FA / TOTP (Phase 24) ──────────────────────────────────────
  twoFactor: {
    status: () => apiFetch<TotpStatusResponse>("/auth/2fa/status"),
    /// Start enrollment. Returns the otpauth URI + base32 secret to
    /// surface in the UI. The TOTP secret isn't active until the user
    /// proves possession by calling verify() with a code.
    enroll: (password: string) =>
      apiFetch<TotpEnrollResponse>("/auth/2fa/enroll", {
        method: "POST",
        body: { password },
      }),
    /// Finish enrollment. On success returns 10 single-use recovery
    /// codes — shown exactly once; server only stores hashes.
    verify: (code: string) =>
      apiFetch<TotpVerifyResponse>("/auth/2fa/verify", {
        method: "POST",
        body: { code },
      }),
    disable: (password: string) =>
      apiFetch<void>("/auth/2fa/disable", {
        method: "POST",
        body: { password },
      }),
    /// Regenerate the recovery-code set. Invalidates any unused old
    /// codes. Returns the fresh set shown once.
    regenerateRecoveryCodes: (password: string) =>
      apiFetch<TotpVerifyResponse>("/auth/2fa/recovery-codes/regenerate", {
        method: "POST",
        body: { password },
      }),
  },
  // ── Notifications (Phase 25) ────────────────────────────────────
  notifications: {
    list: (limit = 50) =>
      apiFetch<NotificationsListResponse>("/notifications", {
        query: { limit },
      }),
    unreadCount: () =>
      apiFetch<{ unread: number }>("/notifications/unread-count"),
    markRead: (id: number) =>
      apiFetch<void>(`/notifications/${id}/read`, { method: "POST" }),
    markAllRead: () =>
      apiFetch<{ marked: number }>("/notifications/read-all", {
        method: "POST",
      }),
  },
  setUserRole: (id: number, role: UserRole) =>
    apiFetch<{ user: User }>(`/auth/users/${id}`, {
      method: "PATCH",
      body: { role },
    }),
};

export const myList = {
  list: () => apiFetch<{ items: ListedItem[] }>("/my-list"),
  add: (itemId: number) =>
    apiFetch<void>(`/my-list/${itemId}`, { method: "POST" }),
  remove: (itemId: number) =>
    apiFetch<void>(`/my-list/${itemId}`, { method: "DELETE" }),
};

export const prefs = {
  hiddenLibraries: () =>
    apiFetch<{ library_ids: number[] }>("/prefs/hidden-libraries"),
  setHiddenLibraries: (library_ids: number[]) =>
    apiFetch<void>("/prefs/hidden-libraries", {
      method: "PUT",
      body: { library_ids },
    }),
};

// ---------------------------------------------------------------------------
// Libraries
// ---------------------------------------------------------------------------

export const libraries = {
  list: () => apiFetch<{ libraries: Library[] }>("/libraries"),
  // Server returns the Library directly, NOT `{ library: ... }`.
  // The previous wrapped types let `const { library } = await
  // create(...)` silently destructure `undefined` from the real
  // (unwrapped) response, which then crashed any downstream
  // `library.id` access. Types now mirror the backend's
  // `Json<Library>` return.
  create: (input: NewLibraryInput) =>
    apiFetch<Library>("/libraries", { method: "POST", body: input }),
  get: (id: number) => apiFetch<Library>(`/libraries/${id}`),
  update: (id: number, input: LibraryUpdateInput) =>
    apiFetch<Library>(`/libraries/${id}`, {
      method: "PATCH",
      body: input,
    }),
  delete: (id: number) =>
    apiFetch<void>(`/libraries/${id}`, { method: "DELETE" }),
  triggerScan: (id: number) =>
    apiFetch<{ scan: ScanJob }>(`/libraries/${id}/scan`, { method: "POST" }),
  listScans: (id: number) =>
    apiFetch<{ scans: ScanJob[] }>(`/libraries/${id}/scans`),
  getAccess: (id: number) =>
    apiFetch<{ user_ids: number[] }>(`/libraries/${id}/access`),
  setAccess: (id: number, user_ids: number[]) =>
    apiFetch<void>(`/libraries/${id}/access`, {
      method: "PUT",
      body: { user_ids },
    }),
  detectMarkers: (id: number) =>
    apiFetch<{ queued: number }>(`/libraries/${id}/detect-markers`, {
      method: "POST",
    }),
  /// Run a synchronous verify pass for one library — stat every
  /// media_file and soft-delete the ones whose disk path no longer
  /// exists. Cheap to call ad-hoc; the same scan also runs weekly
  /// across every library via the `verify_libraries` scheduler task.
  verify: (id: number) =>
    apiFetch<LibraryVerifyResult>(`/libraries/${id}/verify`, {
      method: "POST",
    }),
  stats: (id: number) =>
    apiFetch<LibraryStatsResult>(`/libraries/${id}/stats`),
  refreshMetadata: (id: number) =>
    apiFetch<{ queued: number }>(`/libraries/${id}/refresh-metadata`, {
      method: "POST",
    }),
  generatePreviews: (id: number) =>
    apiFetch<{ queued: number }>(`/libraries/${id}/generate-previews`, {
      method: "POST",
    }),
  /// Hard-delete soft-deleted files past the grace window AND
  /// cascade-sweep orphaned episodes/seasons/items. The default
  /// `grace_days` matches the daily scheduler task (7); set to 0
  /// to immediately reap everything currently marked removed.
  purge: (id: number, grace_days?: number) =>
    apiFetch<LibraryPurgeResult>(`/libraries/${id}/purge`, {
      method: "POST",
      query:
        grace_days !== undefined
          ? { grace_days: String(grace_days) }
          : undefined,
    }),
};

/// Response from `libraries.verify(id)`. See the server-side
/// `VerifyResponse` for what each field means; `orphan_count` is
/// the total soft-deleted rows for the library after the run, used
/// to badge the library card.
export interface LibraryVerifyResult {
  library_id: number;
  files_checked: number;
  files_missing: number;
  newly_marked_removed: number;
  still_missing: number;
  returned_files: number;
  orphan_count: number;
}

export interface LibraryPurgeResult {
  files_purged: number;
  episodes_purged: number;
  seasons_purged: number;
  items_purged: number;
}

/// Per-library stats for the admin card.
export interface LibraryStatsResult {
  library_id: number;
  items: number;
  episodes: number;
  files: number;
  total_bytes: number;
  orphan_files: number;
  last_scanned_at: number | null;
}

// ---------------------------------------------------------------------------
// Items / seasons / episodes
// ---------------------------------------------------------------------------

export const items = {
  list: (filter: ItemFilter = {}) =>
    apiFetch<ItemPage>("/items", {
      query: filter as Record<string, QueryValue>,
    }),
  /// Global trending intersected with the local library. Empty until
  /// the operator runs (or schedules) the `refresh_trending` task.
  /// Pass `library_ids` to further restrict (e.g. honoring the user's
  /// visibility prefs on /new-popular).
  trending: (
    kind: "movie" | "show",
    limit = 10,
    library_ids?: ReadonlyArray<number>,
  ) =>
    apiFetch<{ items: Array<ListedItem & { rank: number }> }>("/items/trending", {
      query: { kind, limit, library_ids },
    }),
  get: (id: number) => apiFetch<ItemDetail>(`/items/${id}`),
  trailer: (id: number) =>
    apiFetch<{ video_id: string | null }>(`/items/${id}/trailer`),
  similar: (id: number) =>
    apiFetch<{ items: ListedItem[] }>(`/items/${id}/similar`),
  detectMarkers: (id: number) =>
    apiFetch<{ queued: number }>(`/items/${id}/detect-markers`, {
      method: "POST",
    }),
  // ─── Per-media-file marker editor (owner-only) ─────────────────────────
  /// Read every marker (auto + manual) on a media file. The full row
  /// shape (with id + source) is used by the operator editor; the
  /// player path consumes a slimmer Marker via the item detail.
  listMarkers: (mediaFileId: number) =>
    apiFetch<MediaFileMarkersResponse>(`/media-files/${mediaFileId}/markers`),
  /// Replace every manual marker on a media file with `markers`. Auto
  /// rows are preserved — they're regenerated by the detection task.
  /// The server-side PUT validates kind / width / duration before the
  /// transaction lands.
  replaceManualMarkers: (
    mediaFileId: number,
    markers: ManualMarkerInput[],
  ) =>
    apiFetch<MediaFileMarkersResponse>(`/media-files/${mediaFileId}/markers`, {
      method: "PUT",
      body: { markers },
    }),
  // ─── Edit & Fix Match (owner-only on the server) ───────────────────────
  patch: (id: number, edit: ItemEditInput) =>
    apiFetch<ItemDetail>(`/items/${id}`, { method: "PATCH", body: edit }),
  /// Hard-delete every media file for this item (and its episodes if
  /// it's a show). Gated by the owning library's
  /// `allow_media_deletion` flag plus owner role. Returns a small
  /// summary of what was removed so the UI can decide whether to
  /// navigate away (item itself was purged) or just close the modal.
  deleteMedia: (id: number) =>
    apiFetch<DeleteMediaResponse>(`/items/${id}/media`, { method: "DELETE" }),
  refresh: (id: number) =>
    apiFetch<ItemDetail>(`/items/${id}/refresh`, { method: "POST" }),
  matchSearch: (id: number, q: string, year?: number) =>
    apiFetch<{ candidates: MatchCandidate[] }>(`/items/${id}/match-search`, {
      query: { q, year },
    }),
  matchApply: (id: number, tmdb_id: number) =>
    apiFetch<ItemDetail>(`/items/${id}/match-apply`, {
      method: "POST",
      body: { tmdb_id },
    }),
  /// Owner-only: merge this item INTO target_id. All media files (or
  /// per-episode files, for shows) get re-pointed onto the target,
  /// then the source item row is deleted. Returns the merge report
  /// plus the target's refreshed detail.
  mergeInto: (id: number, target_id: number) =>
    apiFetch<{
      report: {
        moved_files: number;
        created_seasons: number;
        created_episodes: number;
      };
      target: ItemDetail;
    }>(`/items/${id}/merge-into`, {
      method: "POST",
      body: { target_id },
    }),
  // ─── Reviews (read-only: top reviews from the metadata provider) ───────
  listReviews: (id: number) =>
    apiFetch<{ reviews: Review[] }>(`/items/${id}/reviews`),
  // ─── Cast & Crew (owner-only) ──────────────────────────────────────────
  patchCredits: (id: number, credits: CreditEditInput[]) =>
    apiFetch<ItemDetail>(`/items/${id}/credits`, {
      method: "PATCH",
      body: { credits },
    }),
  // ─── TMDB poster picker (owner-only) ───────────────────────────────────
  tmdbPosters: (id: number) =>
    apiFetch<{ posters: TmdbPoster[] }>(`/items/${id}/tmdb-posters`),
  applyTmdbPoster: (id: number, path: string) =>
    apiFetch<ItemDetail>(`/items/${id}/poster/from-tmdb`, {
      method: "POST",
      body: { path },
    }),
};

export interface CreditEditInput {
  person_id?: number;
  name: string;
  role_kind: "cast" | "director" | "writer" | "producer" | "crew";
  role: string;
  character_name?: string;
  sort_order: number;
  photo_url?: string;
}

export interface TmdbPoster {
  file_path: string;
  thumb_url: string;
  full_url: string;
  language: string | null;
  width: number;
  height: number;
  vote_average: number | null;
}

export const seasons = {
  get: (id: number) => apiFetch<SeasonDetail>(`/seasons/${id}`),
};

export const episodes = {
  get: (id: number) => apiFetch<EpisodeDetail>(`/episodes/${id}`),
  /// Hard-delete this episode's media file. If it was the last
  /// episode of its season, the season is purged too; if last
  /// season, the show is purged. Owner-gated + library-gated on the
  /// server.
  deleteMedia: (id: number) =>
    apiFetch<DeleteMediaResponse>(`/episodes/${id}/media`, {
      method: "DELETE",
    }),
};

export interface DeleteMediaResponse {
  files_deleted: number;
  episodes_purged: number;
  seasons_purged: number;
  items_purged: number;
  /** Source-file and preview-sprite paths the server is unlinking
   *  in the background. Surfaced so the operator UI can confirm
   *  exactly what got removed. */
  paths: string[];
}

// ---------------------------------------------------------------------------
// Stream / play-state
// ---------------------------------------------------------------------------

export const stream = {
  createSession: (input: CreateSessionInput) =>
    apiFetch<CreateSessionResponse>("/stream/sessions", {
      method: "POST",
      body: input,
    }),
  /// Spin up a play session before the user has clicked Play.
  /// Same response shape as `createSession` — the player can adopt
  /// the returned session by id if it matches what would otherwise
  /// be created on mount. See [`/lib/prewarm`].
  prewarmSession: (input: PrewarmInput) =>
    apiFetch<CreateSessionResponse>("/stream/prewarm", {
      method: "POST",
      body: input,
    }),
  deleteSession: (id: string) =>
    apiFetch<void>(`/stream/sessions/${encodeURIComponent(id)}`, {
      method: "DELETE",
    }),
  /// Pause the backing ffmpeg encoder (SIGSTOP on the server). Player
  /// calls this on the HTML5 pause event so a long pause doesn't burn
  /// CPU/GPU writing segments nobody is going to watch. Fire-and-
  /// forget — the response is 204 and the client doesn't block on it.
  pauseSession: (id: string) =>
    apiFetch<void>(`/stream/sessions/${encodeURIComponent(id)}/pause`, {
      method: "POST",
    }),
  /// Resume a paused session (SIGCONT). Paired with `pauseSession`.
  resumeSession: (id: string) =>
    apiFetch<void>(`/stream/sessions/${encodeURIComponent(id)}/resume`, {
      method: "POST",
    }),
  // Direct & HLS playback URLs are paths, not API calls — the player
  // consumes them as <video src>. Use these helpers to build them.
  directUrl: (mediaFileId: number) => `/api/v1/stream/${mediaFileId}/direct`,
  hlsMasterUrl: (sessionId: string) =>
    `/api/v1/stream/sessions/${encodeURIComponent(sessionId)}/master.m3u8`,
};

export interface PlayStateConfig {
  /** Threshold (1–99) at which the player auto-scrobbles a session. */
  played_threshold_pct: number;
  /** Drives the auto-scrobble decision alongside `played_threshold_pct`. */
  completion_behaviour: CompletionBehaviour;
  /** Days an item stays badged as "Recently Added" on Card. 0 = badge
   *  disabled entirely. Read by `useRecentlyAddedDays()` in Card.tsx. */
  recently_added_days: number;
}

export const playState = {
  update: (input: PlayStateUpdateInput) =>
    apiFetch<void>("/play-state", {
      method: "POST",
      body: { updates: [input] },
    }),
  scrobble: (input: ScrobbleInput) =>
    apiFetch<void>("/play-state/scrobble", { method: "POST", body: input }),
  /// Fine-grained playback event (pause / resume) for the admin
  /// Stats engagement metrics. Fire-and-forget on the server side —
  /// callers should swallow any error since the player must not be
  /// blocked by a stats DB write.
  event: (
    input: {
      kind: "pause" | "resume";
      item_id?: number;
      episode_id?: number;
      position_ms?: number;
    },
  ) =>
    apiFetch<void>("/play-state/event", { method: "POST", body: input }),
  setWatched: (
    input: {
      item_id?: number;
      episode_id?: number;
      /// When set, marks every episode of this show id watched
      /// (or unwatched). Server runs the bulk upsert atomically and
      /// fans out Trakt history pushes per affected episode.
      show_id?: number;
      watched: boolean;
    },
  ) =>
    apiFetch<void>("/play-state/watched", { method: "POST", body: input }),
  onDeck: () => apiFetch<OnDeckResponse>("/play-state/on-deck"),
  history: (limit?: number) =>
    apiFetch<{ items: ListedItem[] }>("/play-state/history", {
      query: limit ? { limit } : undefined,
    }),
  config: () => apiFetch<PlayStateConfig>("/play-state/config"),
};

export const collections = {
  // `include_auto` defaults to false on the server — TMDB-discovered
  // franchise rows are excluded unless the caller (typically the admin
  // panel) explicitly opts in. Keeps the home rail clean on a fresh
  // server that hasn't been manually curated.
  list: (opts: { include_auto?: boolean } = {}) =>
    apiFetch<{ collections: Collection[] }>(
      opts.include_auto ? "/collections?include_auto=true" : "/collections",
    ),
  get: (id: number) => apiFetch<CollectionDetail>(`/collections/${id}`),
  // Admin-only mutations (server enforces OwnerAuth). Auto collections
  // (tmdb-discovered franchises) reject mutations with a 400 validation
  // error — only kind = "manual" rows accept these calls.
  create: (input: NewManualCollection) =>
    apiFetch<{ id: number }>("/admin/collections", {
      method: "POST",
      body: input,
    }),
  update: (id: number, patch: ManualCollectionUpdate) =>
    apiFetch<void>(`/admin/collections/${id}`, {
      method: "PATCH",
      body: patch,
    }),
  delete: (id: number) =>
    apiFetch<void>(`/admin/collections/${id}`, { method: "DELETE" }),
  addItems: (id: number, item_ids: number[]) =>
    apiFetch<{ inserted: number }>(`/admin/collections/${id}/items`, {
      method: "POST",
      body: { item_ids },
    }),
  removeItem: (id: number, item_id: number) =>
    apiFetch<void>(`/admin/collections/${id}/items/${item_id}`, {
      method: "DELETE",
    }),
  reorder: (id: number, item_ids: number[]) =>
    apiFetch<void>(`/admin/collections/${id}/items`, {
      method: "PUT",
      body: { item_ids },
    }),
  createSmart: (input: NewSmartCollection) =>
    apiFetch<{ id: number }>("/admin/smart-collections", {
      method: "POST",
      body: input,
    }),
  updateSmartRule: (id: number, rule_json: string) =>
    apiFetch<void>(`/admin/smart-collections/${id}/rule`, {
      method: "PUT",
      body: { rule_json },
    }),
  /// Multipart upload — mirrors items.uploadPoster pattern. File is
  /// stored under `<data_dir>/collection_posters/{id}.<ext>` and the
  /// collection's `poster_path` is set to a versioned `/api/v1/collections/
  /// {id}/poster/blob?v=<ts>` URL so browser caches refetch on overwrite.
  uploadPoster: async (id: number, file: File) => {
    const fd = new FormData();
    fd.append("file", file);
    const res = await fetch(`/api/v1/admin/collections/${id}/poster`, {
      method: "POST",
      body: fd,
      credentials: "include",
    });
    if (!res.ok) {
      const text = await res.text().catch(() => "");
      throw new Error(text || `upload failed: ${res.status}`);
    }
  },
  uploadBackdrop: async (id: number, file: File) => {
    const fd = new FormData();
    fd.append("file", file);
    const res = await fetch(`/api/v1/admin/collections/${id}/backdrop`, {
      method: "POST",
      body: fd,
      credentials: "include",
    });
    if (!res.ok) {
      const text = await res.text().catch(() => "");
      throw new Error(text || `upload failed: ${res.status}`);
    }
  },
};

// ---------------------------------------------------------------------------
// Health / server info
// ---------------------------------------------------------------------------

export const server = {
  health: () => apiFetch<HealthResponse>("/health", { noAuth: true }),
  info: () => apiFetch<ServerInfoResponse>("/server-info"),
};

// ─── Admin Stats (Tautulli-lite) ───────────────────────────────────────────

export interface StatsOverview {
  total_plays: number;
  completions: number;
  direct_plays: number;
  transcoded_plays: number;
  unique_users: number;
}

export interface StatsActivityRow {
  id: number;
  occurred_at: number;
  user_id: number;
  username: string;
  event_type: "start" | "progress" | "pause" | "resume" | "complete" | "stop";
  decision: "direct" | "transcode" | null;
  video_codec: string | null;
  audio_codec: string | null;
  container: string | null;
  ip: string | null;
  item_id: number | null;
  episode_id: number | null;
  title: string | null;
}

export interface StatsTopUserRow {
  user_id: number;
  username: string;
  display_name: string | null;
  play_count: number;
  completions: number;
  last_seen_at: number | null;
}

export interface StatsTopItemRow {
  item_id: number | null;
  title: string;
  kind: string;
  play_count: number;
  last_played_at: number | null;
  year: number | null;
}

export interface StatsDailyBucket {
  /// `YYYY-MM-DD` (UTC) — gap-free across the requested window.
  day: string;
  starts: number;
  completions: number;
}

export interface StatsHourBucket {
  /// 0..=23 — server local-time hour.
  hour: number;
  starts: number;
}

export interface StatsPlatformBucket {
  /// Coarse name bucketed server-side from the user_agent string
  /// (Firefox, Chrome, Android, iOS, LG TV, Roku, …).
  platform: string;
  starts: number;
}

/// Live snapshot from the in-memory TranscodeManager — no DB hop.
/// Mirrors `chimpflix_transcoder::SessionSnapshot` (only the fields
/// the Stats UI needs).
export interface NowPlayingSession {
  id: string;
  user_id: number;
  media_file_id: number;
  start_position_ms: number;
  duration_ms: number | null;
  created_at: number;
  last_seen_at: number;
  encoder: string;
  video_treatment: "copy" | "reencode";
  audio_treatment: "copy" | "reencode";
  source_height: number | null;
  target_height: number;
  target_video_bitrate_bps: number;
  encoder_preset: string;
  /// Cumulative bytes served over HTTP since session start (segment +
  /// playlist GETs). Flushed to `playback_events.bytes_sent` on close.
  bytes_served: number;
}

export interface StatsLibraryBucket {
  library_id: number;
  name: string;
  kind: string;
  starts: number;
}

// ─── Admin (owner-only) ────────────────────────────────────────────────────

export const admin = {
  dashboard: () => apiFetch<DashboardResponse>("/admin/dashboard"),
  /// Background job queue. Powers the Admin → Maintenance → Job
  /// queue page. See `crates/server/src/api/admin/jobs.rs` for the
  /// backend.
  jobs: {
    summary: () => apiFetch<JobSummary>("/admin/jobs/summary"),
    list: (
      opts: {
        kind?: string;
        status?: JobStatusFilter;
        limit?: number;
        offset?: number;
      } = {},
    ) => {
      const qs = new URLSearchParams();
      if (opts.kind) qs.set("kind", opts.kind);
      if (opts.status) qs.set("status", opts.status);
      if (opts.limit != null) qs.set("limit", String(opts.limit));
      if (opts.offset != null) qs.set("offset", String(opts.offset));
      const suffix = qs.toString() ? `?${qs}` : "";
      return apiFetch<{ jobs: JobRow[]; total: number }>(
        `/admin/jobs${suffix}`,
      );
    },
    requeue: (jobId: number) =>
      apiFetch<{ requeued: boolean }>(`/admin/jobs/${jobId}/requeue`, {
        method: "POST",
      }),
    /// Sweep every existing file lacking any pipeline artifact and
    /// enqueue the corresponding jobs. Idempotent (each per-file
    /// enqueue is deduped); safe to re-run while jobs are in flight.
    processAllPending: () =>
      apiFetch<JobSweepCounts>("/admin/jobs/process-all-pending", {
        method: "POST",
      }),
    /// Delete all currently-queued rows. Running jobs are NOT
    /// killed — they finish their current file and then stop.
    /// Optionally scope to one kind via `kind` arg.
    wipeQueued: (kind?: string) =>
      apiFetch<{ removed: number }>(
        `/admin/jobs/queued${kind ? `?kind=${encodeURIComponent(kind)}` : ""}`,
        { method: "DELETE" },
      ),
    /// Delete every `dead` row. Useful after a renamed/removed kind
    /// leaves orphan rows that no handler will ever pick up.
    clearDead: () =>
      apiFetch<{ removed: number }>("/admin/jobs/dead", { method: "DELETE" }),
  },
  stats: {
    /// Last-N-day counters + the live now-playing count for the hero
    /// tiles. `days` defaults to 30, clamped to [1, 365] server-side.
    overview: (days?: number) =>
      apiFetch<{ days: number; overview: StatsOverview; now_playing_count: number }>(
        days ? `/admin/stats/overview?days=${days}` : "/admin/stats/overview",
      ),
    /// Newest-first event feed. Pass `before` (the smallest id from
    /// the previous page) to paginate older entries. `user_id` scopes
    /// to a single user — the per-user drill-in uses this.
    activity: (opts: { limit?: number; before?: number; user_id?: number } = {}) => {
      const qs = new URLSearchParams();
      if (opts.limit != null) qs.set("limit", String(opts.limit));
      if (opts.before != null) qs.set("before", String(opts.before));
      if (opts.user_id != null) qs.set("user_id", String(opts.user_id));
      const suffix = qs.toString() ? `?${qs}` : "";
      return apiFetch<{ events: StatsActivityRow[] }>(`/admin/stats/activity${suffix}`);
    },
    /// Daily series for the activity chart — gap-free across the
    /// requested window so a quiet day shows up as a zero bar
    /// rather than missing.
    playsPerDay: (days?: number) =>
      apiFetch<{ days: number; buckets: StatsDailyBucket[] }>(
        days ? `/admin/stats/plays-per-day?days=${days}` : "/admin/stats/plays-per-day",
      ),
    /// Hour-of-day distribution (24 buckets, server local time).
    /// Tautulli's most-loved chart — "when does my household watch?"
    playsPerHour: (days?: number) =>
      apiFetch<{ days: number; buckets: StatsHourBucket[] }>(
        days ? `/admin/stats/plays-per-hour?days=${days}` : "/admin/stats/plays-per-hour",
      ),
    /// Top libraries by play count. Movies aggregate via
    /// items.library_id directly; episodes roll up through
    /// episodes → seasons → items.library_id (parent show).
    topLibraries: (opts: { days?: number; limit?: number } = {}) => {
      const qs = new URLSearchParams();
      if (opts.days != null) qs.set("days", String(opts.days));
      if (opts.limit != null) qs.set("limit", String(opts.limit));
      const suffix = qs.toString() ? `?${qs}` : "";
      return apiFetch<{ days: number; libraries: StatsLibraryBucket[] }>(
        `/admin/stats/top-libraries${suffix}`,
      );
    },
    /// Coarse platform breakdown (Firefox / Chrome / iOS / Roku / …)
    /// bucketed server-side from the user_agent string.
    topPlatforms: (opts: { days?: number; limit?: number } = {}) => {
      const qs = new URLSearchParams();
      if (opts.days != null) qs.set("days", String(opts.days));
      if (opts.limit != null) qs.set("limit", String(opts.limit));
      const suffix = qs.toString() ? `?${qs}` : "";
      return apiFetch<{ days: number; platforms: StatsPlatformBucket[] }>(
        `/admin/stats/top-platforms${suffix}`,
      );
    },
    topUsers: (opts: { days?: number; limit?: number } = {}) => {
      const qs = new URLSearchParams();
      if (opts.days != null) qs.set("days", String(opts.days));
      if (opts.limit != null) qs.set("limit", String(opts.limit));
      const suffix = qs.toString() ? `?${qs}` : "";
      return apiFetch<{ days: number; users: StatsTopUserRow[] }>(
        `/admin/stats/top-users${suffix}`,
      );
    },
    topItems: (opts: { days?: number; limit?: number } = {}) => {
      const qs = new URLSearchParams();
      if (opts.days != null) qs.set("days", String(opts.days));
      if (opts.limit != null) qs.set("limit", String(opts.limit));
      const suffix = qs.toString() ? `?${qs}` : "";
      return apiFetch<{ days: number; items: StatsTopItemRow[] }>(
        `/admin/stats/top-items${suffix}`,
      );
    },
    nowPlaying: () =>
      apiFetch<{ sessions: NowPlayingSession[] }>("/admin/stats/now-playing"),
  },
  settings: {
    get: () => apiFetch<{ settings: ServerSettings }>("/admin/settings"),
    patch: (patch: ServerSettingsUpdate) =>
      apiFetch<{ settings: ServerSettings }>("/admin/settings", {
        method: "PATCH",
        body: patch,
      }),
  },
  email: {
    status: () => apiFetch<EmailStatusResponse>("/admin/settings/email"),
    setPassword: (password: string) =>
      apiFetch<void>("/admin/settings/email/password", {
        method: "PUT",
        body: { password },
      }),
    clearPassword: () =>
      apiFetch<void>("/admin/settings/email/password", { method: "DELETE" }),
    test: (sendTo?: string) =>
      apiFetch<EmailTestResponse>("/admin/settings/email/test", {
        method: "POST",
        body: sendTo ? { send_to: sendTo } : {},
      }),
  },
  audit: {
    list: (
      params: { before?: number; limit?: number; offset?: number } = {},
    ) =>
      apiFetch<AuditListResponse>("/admin/audit", {
        query: {
          before: params.before,
          limit: params.limit,
          offset: params.offset,
        },
      }),
  },
  stopSession: (id: string) =>
    apiFetch<void>(`/stream/sessions/${encodeURIComponent(id)}`, {
      method: "DELETE",
    }),
  /// Admin: wipe another user's TOTP enrollment + recovery codes. Used
  /// when a user has lost their authenticator device. Login proceeds
  /// as password-only until the user re-enrolls.
  resetUserTwoFactor: (userId: number) =>
    apiFetch<void>(`/admin/users/${userId}/2fa/reset`, { method: "POST" }),
  /// Admin: send a password-reset email to another user. Token is
  /// generated server-side and never exposed to the admin — same
  /// single-use guarantee as the self-service /auth/password-reset
  /// path. `ok=false` means the email wasn't delivered (no address on
  /// file, SMTP not configured, or SMTP send error); the response
  /// `message` is safe to show as a toast.
  sendUserPasswordReset: (userId: number) =>
    apiFetch<{ ok: boolean; message: string }>(
      `/admin/users/${userId}/password-reset`,
      { method: "POST" },
    ),
  /// Named access groups — bulk-assignment of library permissions.
  /// User-side resolution unions direct library_access with group-derived.
  accessGroups: {
    list: () =>
      apiFetch<{ groups: AccessGroup[] }>("/admin/access-groups"),
    get: (id: number) =>
      apiFetch<AccessGroupDetail>(`/admin/access-groups/${id}`),
    create: (input: NewAccessGroupInput) =>
      apiFetch<AccessGroup>("/admin/access-groups", {
        method: "POST",
        body: input,
      }),
    update: (id: number, patch: AccessGroupUpdateInput) =>
      apiFetch<AccessGroup>(`/admin/access-groups/${id}`, {
        method: "PATCH",
        body: patch,
      }),
    delete: (id: number) =>
      apiFetch<void>(`/admin/access-groups/${id}`, { method: "DELETE" }),
    setLibraries: (id: number, libraryIds: number[]) =>
      apiFetch<void>(`/admin/access-groups/${id}/libraries`, {
        method: "PUT",
        body: { library_ids: libraryIds },
      }),
    setMembers: (id: number, userIds: number[]) =>
      apiFetch<void>(`/admin/access-groups/${id}/members`, {
        method: "PUT",
        body: { user_ids: userIds },
      }),
    getUserGroups: (userId: number) =>
      apiFetch<{ group_ids: number[] }>(
        `/admin/users/${userId}/access-groups`,
      ),
    setUserGroups: (userId: number, groupIds: number[]) =>
      apiFetch<void>(`/admin/users/${userId}/access-groups`, {
        method: "PUT",
        body: { group_ids: groupIds },
      }),
  },
  secrets: {
    list: () => apiFetch<SecretsListResponse>("/admin/secrets"),
    set: (name: string, value: string) =>
      apiFetch<SecretSlotView>(`/admin/secrets/${encodeURIComponent(name)}`, {
        method: "PUT",
        body: { value },
      }),
    clear: (name: string) =>
      apiFetch<SecretSlotView>(`/admin/secrets/${encodeURIComponent(name)}`, {
        method: "DELETE",
      }),
    test: (name: string, value?: string) =>
      apiFetch<SecretTestResponse>(
        `/admin/secrets/${encodeURIComponent(name)}/test`,
        { method: "POST", body: value ? { value } : {} },
      ),
  },
  libraryHealth: () =>
    apiFetch<LibraryHealthResponse>("/admin/library-health"),
  /// Drill-in for a single Library Health counter — returns the
  /// actual rows behind the count so admins can act on them. See
  /// `LibraryHealthCategory` for valid keys.
  libraryHealthItems: (
    category: LibraryHealthCategory,
    opts: { limit?: number; offset?: number } = {},
  ) => {
    const qs = new URLSearchParams({ category });
    if (opts.limit != null) qs.set("limit", String(opts.limit));
    if (opts.offset != null) qs.set("offset", String(opts.offset));
    return apiFetch<LibraryHealthItemsResponse>(
      `/admin/library-health/items?${qs}`,
    );
  },
  agents: {
    list: () => apiFetch<{ agents: AgentInfo[] }>("/admin/agents"),
    getForLibrary: (libraryId: number) =>
      apiFetch<{ agents: LibraryAgent[] }>(
        `/admin/libraries/${libraryId}/agents`,
      ),
    setForLibrary: (libraryId: number, agents: LibraryAgent[]) =>
      apiFetch<{ agents: LibraryAgent[] }>(
        `/admin/libraries/${libraryId}/agents`,
        { method: "PUT", body: { agents } },
      ),
  },
  logs: (params: { level?: string; limit?: number } = {}) =>
    apiFetch<{ lines: LogLine[] }>("/admin/logs", {
      query: params as Record<string, string | number | undefined>,
    }),
  alerts: (params: { limit?: number } = {}) =>
    apiFetch<AlertsResponse>("/admin/alerts", {
      query: params as Record<string, string | number | undefined>,
    }),
  optimized: {
    list: () =>
      apiFetch<{ versions: OptimizedVersion[] }>("/admin/optimized"),
    enqueue: (input: NewOptimizedVersionInput) =>
      apiFetch<OptimizedVersion>("/admin/optimized", {
        method: "POST",
        body: input,
      }),
    delete: (id: number) =>
      apiFetch<void>(`/admin/optimized/${id}`, { method: "DELETE" }),
  },
  sessions: {
    list: () =>
      apiFetch<{ sessions: AdminSessionSummary[] }>("/admin/sessions"),
    revoke: (id: number) =>
      apiFetch<void>(`/admin/sessions/${id}`, { method: "DELETE" }),
    listForUser: (userId: number) =>
      apiFetch<{ sessions: AdminSessionSummary[] }>(
        `/admin/users/${userId}/sessions`,
      ),
    revokeAllForUser: (userId: number) =>
      apiFetch<{ revoked: number }>(`/admin/users/${userId}/sessions`, {
        method: "DELETE",
      }),
  },
  access: {
    get: () =>
      apiFetch<{ entries: AccessMatrixEntry[] }>("/admin/access"),
    put: (libraries: LibraryAccessAssignment[]) =>
      apiFetch<{ entries: AccessMatrixEntry[] }>("/admin/access", {
        method: "PUT",
        body: { libraries },
      }),
  },
  webhooks: {
    list: () => apiFetch<WebhooksListResponse>("/admin/webhooks"),
    create: (input: NewWebhookInput) =>
      apiFetch<{ webhook: Webhook }>("/admin/webhooks", {
        method: "POST",
        body: input,
      }),
    update: (id: number, patch: WebhookUpdateInput) =>
      apiFetch<{ webhook: Webhook }>(`/admin/webhooks/${id}`, {
        method: "PATCH",
        body: patch,
      }),
    delete: (id: number) =>
      apiFetch<void>(`/admin/webhooks/${id}`, { method: "DELETE" }),
    test: (id: number) =>
      apiFetch<void>(`/admin/webhooks/${id}/test`, { method: "POST" }),
    listDeliveries: (id: number, limit?: number) =>
      apiFetch<{ deliveries: WebhookDelivery[] }>(
        `/admin/webhooks/${id}/deliveries`,
        { query: limit ? { limit } : undefined },
      ),
  },
  network: {
    get: () => apiFetch<NetworkSettings>("/admin/network"),
    patch: (patch: NetworkUpdateInput) =>
      apiFetch<NetworkSettings>("/admin/network", {
        method: "PATCH",
        body: patch,
      }),
    testReachability: () =>
      apiFetch<ReachabilityResult>("/admin/network/test-reachability", {
        method: "POST",
      }),
  },
  transcoder: {
    capabilities: () =>
      apiFetch<{ capabilities: TranscoderCapabilities; cache_root: string }>(
        "/admin/transcoder/capabilities",
      ),
    listPresets: () =>
      apiFetch<{ presets: TranscoderPreset[] }>("/admin/transcoder/presets"),
    createPreset: (input: NewTranscoderPresetInput) =>
      apiFetch<{ preset: TranscoderPreset }>("/admin/transcoder/presets", {
        method: "POST",
        body: input,
      }),
    updatePreset: (id: number, patch: TranscoderPresetUpdate) =>
      apiFetch<{ preset: TranscoderPreset }>(
        `/admin/transcoder/presets/${id}`,
        { method: "PATCH", body: patch },
      ),
    deletePreset: (id: number) =>
      apiFetch<void>(`/admin/transcoder/presets/${id}`, { method: "DELETE" }),
  },
  tasks: {
    list: () => apiFetch<TasksListResponse>("/admin/tasks"),
    create: (input: NewScheduledTaskInput) =>
      apiFetch<{ task: ScheduledTask }>("/admin/tasks", {
        method: "POST",
        body: input,
      }),
    update: (id: number, patch: ScheduledTaskUpdate) =>
      apiFetch<{ task: ScheduledTask }>(`/admin/tasks/${id}`, {
        method: "PATCH",
        body: patch,
      }),
    delete: (id: number) =>
      apiFetch<void>(`/admin/tasks/${id}`, { method: "DELETE" }),
    runNow: (id: number) =>
      apiFetch<void>(`/admin/tasks/${id}/run`, { method: "POST" }),
    listRuns: (id: number, limit?: number) =>
      apiFetch<{ runs: TaskRun[] }>(`/admin/tasks/${id}/runs`, {
        query: limit ? { limit } : undefined,
      }),
    /// Registry-driven views backing the rebuilt tasks UI. Sibling to
    /// the row-based CRUD above (which still drives the legacy
    /// advanced editor at /admin/tasks).
    overview: () =>
      apiFetch<TasksOverviewResponse>("/admin/tasks/overview"),
    summary: () =>
      apiFetch<TasksSummaryResponse>("/admin/tasks/summary"),
    activity: () =>
      apiFetch<TasksActivityResponse>("/admin/tasks/activity"),
    /// Detail payload for one kind — schedule + gate + live counters
    /// + ring buffer of recent runs + 30-day history rollup. Powers
    /// the per-task drill-in page.
    detail: (kind: string) =>
      apiFetch<KindDetailResponse>(
        `/admin/tasks/kind/${encodeURIComponent(kind)}`,
      ),
    /// Toggle the gate setting for one task kind. Returns 204 on
    /// success; client refetches `overview` to reflect the new state.
    setGate: (kind: string, enabled: boolean) =>
      apiFetch<void>(`/admin/tasks/kind/${encodeURIComponent(kind)}/gate`, {
        method: "PATCH",
        body: { enabled },
      }),
    /// Edit the `scheduled_tasks` row backing a kind: frequency,
    /// enabled flag, maintenance-window snap, params. Returns the
    /// refreshed detail payload so the client can reuse it as the
    /// new render state without a separate refetch.
    updateKindSchedule: (kind: string, patch: KindScheduleUpdate) =>
      apiFetch<KindDetailResponse>(
        `/admin/tasks/kind/${encodeURIComponent(kind)}`,
        { method: "PATCH", body: patch },
      ),
    /// Dispatch the kind once via the scheduler. Returns 202; the
    /// caller is expected to poll the detail endpoint for live
    /// counters/status updates.
    runKindNow: (kind: string) =>
      apiFetch<void>(`/admin/tasks/kind/${encodeURIComponent(kind)}/run`, {
        method: "POST",
      }),
  },
  /// One-click instance-wide maintenance actions. Paired with the
  /// scheduled tasks under `admin.tasks` — those run on a cadence;
  /// these are the operator's on-demand path.
  maintenance: {
    verifyAll: () =>
      apiFetch<VerifyAllResult>("/admin/maintenance/verify-all", {
        method: "POST",
      }),
    purgeAll: (grace_days?: number) =>
      apiFetch<MaintenancePurgeResult>("/admin/maintenance/purge-all", {
        method: "POST",
        query:
          grace_days !== undefined
            ? { grace_days: String(grace_days) }
            : undefined,
      }),
    vacuumDatabase: () =>
      apiFetch<VacuumResult>("/admin/maintenance/vacuum", { method: "POST" }),
    clearTranscodeCache: () =>
      apiFetch<ClearTranscodeCacheResult>(
        "/admin/maintenance/clear-transcode-cache",
        { method: "POST" },
      ),
  },
};

export interface VerifyAllResult {
  libraries_checked: number;
  files_checked: number;
  files_missing: number;
  newly_marked_removed: number;
  returned_files: number;
}

export interface MaintenancePurgeResult {
  files_purged: number;
  episodes_purged: number;
  seasons_purged: number;
  items_purged: number;
}

export interface VacuumResult {
  bytes_reclaimed: number;
  before_bytes: number;
  after_bytes: number;
  duration_ms: number;
}

export interface ClearTranscodeCacheResult {
  sessions_removed: number;
  bytes_freed: number;
}

/// Trigger a SQLite snapshot and stream it as a download. The backend
/// creates a VACUUM INTO snapshot then streams the file back; we wrap it
/// in a Blob and synthesize a click on a hidden <a> to surface the
/// browser's native save dialog.
export async function downloadBackup(): Promise<void> {
  const res = await fetch("/api/v1/admin/backup", {
    method: "POST",
    credentials: "include",
  });
  if (!res.ok) {
    throw new Error(`backup failed: ${res.status}`);
  }
  // Pull the filename from Content-Disposition; fall back to a stamp.
  const disposition = res.headers.get("Content-Disposition") ?? "";
  const match = disposition.match(/filename="([^"]+)"/);
  const filename = match?.[1] ?? `chimpflix-${Date.now()}.db`;
  const blob = await res.blob();
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

// ─── Backup management (auto snapshots) ─────────────────────────────────────

export interface BackupEntry {
  filename: string;
  size_bytes: number;
  /** Last-modified time in milliseconds since epoch. */
  modified_ms: number;
}

export interface ListBackupsResponse {
  backups: BackupEntry[];
  /** True when a pending-restore is staged and waiting for the
   *  next server restart to apply. */
  pending_restore: boolean;
  total_bytes: number;
}

export interface StageRestoreResponse {
  staged: string;
  message: string;
}

export const backups = {
  list: () => apiFetch<ListBackupsResponse>("/admin/backups"),

  /** Trigger the browser download for a specific persisted snapshot. */
  download: async (filename: string): Promise<void> => {
    const res = await fetch(
      `/api/v1/admin/backups/${encodeURIComponent(filename)}/download`,
      { credentials: "include" },
    );
    if (!res.ok) {
      throw new Error(`download failed: ${res.status}`);
    }
    const blob = await res.blob();
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
  },

  delete: (filename: string) =>
    apiFetch<void>(`/admin/backups/${encodeURIComponent(filename)}`, {
      method: "DELETE",
    }),

  /** Stage a backup as the next-boot database. Server returns plain-
   *  language instructions; the actual restore happens on restart. */
  stageRestore: (filename: string) =>
    apiFetch<StageRestoreResponse>(
      `/admin/backups/${encodeURIComponent(filename)}/stage-restore`,
      { method: "POST" },
    ),

  cancelRestore: () =>
    apiFetch<void>("/admin/backups/cancel-restore", { method: "POST" }),
};

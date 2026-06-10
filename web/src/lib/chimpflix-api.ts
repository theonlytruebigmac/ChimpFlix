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
  /** Per-user subtitle styling. null = use the client-default. Drives
   * both browser `::cue` rendering and ASS burn-in. See
   * `web/src/lib/subtitle-style.ts` for the canonical model + helpers. */
  subtitle_font_size_px: number | null;
  subtitle_text_color: string | null;
  subtitle_background_color: string | null;
  subtitle_font_family: string | null;
  subtitle_edge: string | null;
  subtitle_bottom_inset_pct: number | null;
  /** Whether the user opted into email mirroring of in-app notifications. */
  notify_via_email: boolean;
  /** Per-kind notification preferences as a JSON object keyed by kind
   * discriminator (e.g. `{"job.failed":{"enabled":false}}`). Empty object
   * = all defaults. Security kinds (`user.2fa.*`) always notify. */
  notification_prefs_json: string;
  /** Personal Discord webhook URL. null = not configured. When set,
   * notifications that reach the user are mirrored to this webhook
   * (treated like the email channel for per-kind prefs + quiet hours). */
  discord_webhook_url: string | null;
  /** IANA timezone name (e.g. `America/New_York`). Defaults to `UTC`.
   * Used by the notifier to interpret the quiet-hours window in the
   * user's local wall-clock time. */
  timezone: string;
  /** Per-user home-page rail layout overlay: a JSON array of
   * `{"rail_id":"<id>","enabled":<bool>}` entries. SPARSE/overlay
   * semantics — any rail absent from the array keeps its default position +
   * enabled state, so `"[]"` (the default) = stock home. The array order is
   * the user's desired rail order. Rail ids match the backend
   * `HOME_RAIL_CATALOGUE` (see `web/src/app/page.tsx` for the id↔rail map). */
  home_rails_json: string;
  /** DEPRECATED / unused by the UI: was meant to hide fully-watched titles
   * from Continue Watching, but on-deck already excludes finished titles so it
   * was a no-op. The DB column remains; the settings UI no longer reads or
   * writes it. Default false. */
  hide_watched_cw: boolean;
  /** When true, hide titles with an explicit mature rating from home + browse
   * (enforced server-side at the items query layer). FAIL-OPEN: unrated /
   * NULL-rated titles are still shown, so it's a no-op until `rating_age` is
   * populated. Default false. */
  kids_safe: boolean;
  /** Most-recent successful login. null on first login. */
  last_login_at: number | null;
  last_login_ip: string | null;
  /** The login immediately before `last_login_at`. Surfaced as "last
   * signed in 3h ago from X" so users can spot unexpected access. */
  previous_login_at: number | null;
  previous_login_ip: string | null;
  /** When true the account is disabled — the login gate rejects it
   * (after the password check). Owners are never lockable. */
  locked: boolean;
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
  // Subtitle styling. Strings: empty string clears, omit to leave
  // unchanged. Numerics: present sets, omit to leave unchanged
  // (clearing back to null isn't exposed — the UI always picks a
  // concrete value from a closed palette).
  subtitle_font_size_px?: number;
  subtitle_text_color?: string;
  subtitle_background_color?: string;
  subtitle_font_family?: string;
  subtitle_edge?: string;
  subtitle_bottom_inset_pct?: number;
  notify_via_email?: boolean;
  /** JSON object of per-kind notification prefs; validated server-side. */
  notification_prefs_json?: string;
  /** Personal Discord webhook URL. Empty string clears it; a non-empty
   * value must be a Discord webhook URL (validated server-side); omit to
   * leave unchanged. */
  discord_webhook_url?: string;
  /** IANA timezone name (e.g. `America/New_York`). Empty string resets to
   * `UTC`; a non-empty value must be a valid IANA name (validated
   * server-side); omit to leave unchanged. */
  timezone?: string;
  /** Home-page rail layout overlay: a JSON array of
   * `{"rail_id":"<id>","enabled":<bool>}` entries. Empty string resets to
   * `"[]"` (stock home); a non-empty value must be a JSON array of known-rail
   * entries (validated server-side); omit to leave unchanged. */
  home_rails_json?: string;
  /** DEPRECATED / unused by the UI (no-op against on-deck). Still accepted by
   * the server for backward compat; the settings UI no longer sends it.
   * Present sets the boolean; omit to leave unchanged. */
  hide_watched_cw?: boolean;
  /** Hide explicitly mature-rated titles from home + browse (fail-open: unrated
   * titles still show). Present sets the boolean; omit to leave unchanged. */
  kids_safe?: boolean;
}

/** One home-rail in the customization catalogue: the stable wire id stored
 * in `User.home_rails_json` plus the human label shown in the Home &
 * visibility settings. */
export interface HomeRailCatalogueEntry {
  rail_id: string;
  label: string;
}

/** One entry in a `home_rails_json` overlay patch sent to PATCH /auth/me.
 * `rail_id` must be a member of {@link HOME_RAIL_CATALOGUE}; `enabled`
 * toggles whether the rail renders. The array order is the user's desired
 * top-to-bottom rail order. */
export interface HomeRailPref {
  rail_id: string;
  enabled: boolean;
}

/** The stable catalogue of home-page rails the customization UI can toggle +
 * reorder, in their default top-to-bottom order.
 *
 * MUST stay in sync with the backend `HOME_RAIL_CATALOGUE`
 * (crates/library/src/models.rs) — that constant is the source of truth the
 * /auth/me prefs API validates `home_rails_json` against — AND with
 * `HOME_RAIL_ORDER` in web/src/app/page.tsx, which maps each id to the
 * rendered rail node. `User.home_rails_json` is a SPARSE overlay over this
 * list: any rail absent from a user's overlay keeps its default position +
 * enabled state, so an empty overlay = the stock home page. */
export const HOME_RAIL_CATALOGUE: readonly HomeRailCatalogueEntry[] = [
  { rail_id: "continue_watching", label: "Continue Watching" },
  { rail_id: "recently_added", label: "Recently Added" },
  { rail_id: "coming_soon", label: "Coming Soon" },
  { rail_id: "season_premieres", label: "New Seasons" },
  { rail_id: "calendar", label: "Coming Up · Calendar" },
  { rail_id: "upcoming_movies", label: "Upcoming Movies" },
  { rail_id: "trakt_recs_movies", label: "Recommended for You · Movies" },
  { rail_id: "trakt_recs_shows", label: "Recommended for You · Shows" },
  { rail_id: "trakt_favorites", label: "Your Trakt Favorites" },
  { rail_id: "trakt_lists", label: "Your Trakt Lists" },
  { rail_id: "top10_movies", label: "Top 10 Movies This Week" },
  { rail_id: "top10_shows", label: "Top 10 Shows This Week" },
  { rail_id: "collections", label: "Collections" },
  { rail_id: "library_sections", label: "New in Your Libraries" },
  { rail_id: "movie_genres", label: "Movie Genre Rails" },
  { rail_id: "show_genres", label: "Show Genre Rails" },
] as const;

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
  total: number;
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
export type PrimaryMetadataAgent = "tmdb" | "tvdb";

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
  /** Which metadata agent runs first (Primary mode) for this library.
   *  The remaining agents in `library_agents` run after it as FillNulls
   *  fallbacks. New movie/show libraries default to "tmdb"; anime
   *  libraries default to "tvdb". */
  primary_metadata_agent: PrimaryMetadataAgent;
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
  primary_metadata_agent?: PrimaryMetadataAgent;
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
  /** Whether this agent participates in the scan-time metadata chain.
   *  Non-chain agents (OMDb / Trakt / OpenSubtitles) are listed for
   *  credential-status visibility only — adding them to the priority
   *  picker would silently no-op. */
  participates_in_chain: boolean;
  /** Per-stage capability flags sourced from `MetadataAgent::capabilities()`.
   *  The UI renders one badge per `true` field so operators see at a
   *  glance what the agent can contribute. Agents that don't go through
   *  the chain (opensubtitles, trakt) return all false. */
  capabilities: AgentCapabilities;
  /** Operator-facing limitations — surfaced in an info tooltip next to
   *  the badges. Empty when there's nothing notable. */
  limitations: string[];
}

export interface AgentCapabilities {
  movie: boolean;
  show: boolean;
  episode: boolean;
  cast: boolean;
  artwork: boolean;
  ratings: boolean;
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
  /// Rough wall-clock ETA to drain the queue, in seconds. Computed
  /// server-side as `queue × p95 / effective_concurrency`. `null`
  /// when the queue is empty or there's not enough run history yet
  /// to produce a p95.
  eta_seconds_remaining: number | null;
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

/// One currently-running job — drives the per-row entry in the
/// header ActivityIndicator popover ("Detecting markers: WIND
/// BREAKER" style). `title` is server-resolved from the payload's
/// `item_id` or `file_id` so the client never touches per-handler
/// payload shapes.
export interface ActivityRunningJob {
  id: number;
  kind: string;
  display_name: string;
  title: string | null;
  /// "S02E04"-style suffix when the job targets an episode; null for
  /// movie / non-episodic targets.
  episode_code: string | null;
  started_at_ms: number | null;
}

export interface TasksActivityResponse {
  per_kind: ActivityKindHealth[];
  running_jobs: ActivityRunningJob[];
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
  /** Diagnostic of what the server *actually* trusts and sees as the
   *  request peer. Drives the "your proxy config is broken" banner
   *  on the admin home (see PUBLIC_RELEASE_HARDENING.md WEEK 1 #8). */
  proxy_diagnostic: ProxyDiagnostic;
  /** The most recent persisted reachability check, or absent/undefined
   *  when none has ever run. Lets the Network page show a standing
   *  "Reachable · checked Xm ago" banner across reloads. */
  last_reachability?: LastReachability;
}

/** Persisted snapshot of the last reachability check (mirrors the
 *  server's `LastReachability`). `checked_at` is epoch ms — the client
 *  renders the relative "checked Xm ago" label from it. */
export interface LastReachability {
  ok: boolean;
  public_url: string | null;
  status_code: number | null;
  latency_ms: number | null;
  error: string | null;
  checked_at: number;
}

export interface ProxyDiagnostic {
  /** TRUSTED_PROXIES parsed at boot, as CIDR strings. Empty = no
   *  proxy headers are honoured at all. */
  trusted_proxies: string[];
  /** Immediate TCP peer for the request that loaded this page. */
  peer_ip: string | null;
  /** True when peer is in an RFC1918 / RFC4193 / loopback / link-local
   *  range — almost always a reverse proxy or Docker bridge. */
  peer_is_private: boolean;
  /** True when the peer looks private AND TRUSTED_PROXIES doesn't
   *  cover it. The UI banner fires off this flag. */
  looks_misconfigured: boolean;
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

export interface WebhookLastDelivery {
  status_code: number | null;
  delivered: boolean;
  created_at: number; // ms epoch
}

export interface Webhook {
  id: number;
  name: string;
  url: string;
  secret: string | null;
  event_mask: string; // JSON-encoded string[]
  enabled: boolean;
  created_at: number;
  updated_at: number;
  /** Summary of the most recent delivery; omitted if never delivered. */
  last_delivery?: WebhookLastDelivery;
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

/// Tri-state library access level (phase 107).
///   - "none" — no grant: library + items HIDDEN.
///   - "view" — can browse/see metadata, but CANNOT play.
///   - "full" — can browse AND play (the prior binary "allowed").
export type AccessLevel = "none" | "view" | "full";

export interface AccessMatrixEntry {
  user_id: number;
  username: string;
  library_id: number;
  library_name: string;
  /// A direct `library_access` row exists (any level). Kept for back-compat;
  /// `level` carries the actual direct grant level.
  allowed: boolean;
  /// Level of the DIRECT grant ("none" when no direct row exists). This is
  /// what the matrix's per-cell selector edits. A user may still have
  /// group-derived access — see `via_groups`.
  level: AccessLevel;
  /// Access-group names that ALSO grant this user this library
  /// (`access_group_libraries` × `user_access_groups`). These grants
  /// aren't editable from the matrix — they're managed under Settings
  /// → Users → Groups — but the UI surfaces them so admins can see
  /// effective access at a glance instead of mistaking group-only
  /// access for "locked out".
  via_groups: string[];
}

/// One user's desired level for a library, sent by the matrix.
export interface UserAccessGrant {
  user_id: number;
  level: AccessLevel;
}

export interface LibraryAccessAssignment {
  library_id: number;
  /// Legacy binary shape (every listed user → "full"). Send `grants` for
  /// tri-state.
  user_ids?: number[];
  /// Tri-state per-user levels. Takes precedence over `user_ids`.
  grants?: UserAccessGrant[];
}

// ─── Optimized Versions ────────────────────────────────────────────────────

export interface OptimizedVersion {
  id: number;
  source_file_id: number;
  preset_id: number;
  output_path: string;
  output_size_bytes: number | null;
  duration_ms: number | null;
  status: "queued" | "running" | "success" | "failed" | "cancelled";
  /**
   * Re-encode progress in tenths of a percent (0..=1000), or null while
   * queued / before the worker has stamped a measurement. The admin UI
   * renders a determinate bar when present and an indeterminate
   * "running" bar when null on a running row.
   */
  progress_permille: number | null;
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
  /**
   * Content/age rating (e.g. "PG-13", "TV-MA"); free-text, may be null.
   * Optional so looser item shapes (ApiItem, partial rows) stay assignable;
   * the full `GET /items/:id` response always includes it.
   */
  rating_age?: string | null;
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
  /// Resume point — where the player seeks to on "Resume". Moves
  /// backward when the user seeks back and quits (correct for resume).
  position_ms: number;
  /// Furthest point ever reached. Monotonic — never shrinks on a backward
  /// seek. Drives the progress bar + "X min left" so skipping around
  /// doesn't make a finished episode look un-watched.
  max_position_ms: number;
  duration_ms: number | null;
  watched: boolean;
  view_count: number;
  last_played_at: number;
}

// `ListedItem` flattens `Item` + `play_state`, so TS sees a single object.
// `best_quality_height` + `best_hdr_format` come from list-query subqueries
// over media_files (max height + best HDR variant across the item's files).
// Both are optional — some narrower endpoints (e.g. trakt-derived rails)
// don't populate them, in which case the UI just hides the quality chip.
export type ListedItem = Item & {
  play_state: PlayStateForItem | null;
  best_quality_height?: number | null;
  best_hdr_format?: string | null;
};

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
  | "rating_desc"
  | "duration_desc"
  | "duration_asc"
  | "last_played"
  | "random"
  | "size_desc"
  | "size_asc";

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
  /** Only items the current user has never started. */
  unwatched_only?: boolean;
  /** Only items the current user is in the middle of. */
  in_progress_only?: boolean;
  /** Only items the current user has finished. */
  watched_only?: boolean;
  /** Inclusive lower bound on item year. Decade chips translate to
   *  `{year_min: 2020, year_max: 2029}` etc. */
  year_min?: number;
  year_max?: number;
  /** Seed for the random sort. Stable pagination requires the same
   *  seed across page requests in a session. */
  random_seed?: number;
  /** Resolution buckets: any of `sd` / `720` / `1080` / `4k`. An item
   *  matches when ≥1 of its current files lands in any selected
   *  bucket. CSV-encoded on the wire. */
  resolutions?: ReadonlyArray<string>;
  /** HDR formats: any of `sdr` / `hdr10` / `hlg`. SDR = absence of
   *  an hdr_format tag — that's what the scanner stores today. */
  hdr?: ReadonlyArray<string>;
  /** Video codecs (ffprobe names): any of `hevc` / `h264` / `av1` /
   *  `vp9` / `mpeg4` / `mpeg2video` / `other`. `other` matches any
   *  codec outside the known list. */
  codecs?: ReadonlyArray<string>;
  /** Existence/count probe. When true the server returns `total` only
   *  (zero `items`) AND bypasses the per-user kids_safe filter, so the
   *  home page can ask "is the server scanned at all?" without a
   *  kids_safe profile on an unrated library seeing a false empty-home
   *  screen. Returns no titles, so it can't be used to bypass kids_safe
   *  to view content. */
  count_only?: boolean;
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
  /// Per-stage timing JSON (e.g. `{"markers_ms":182000,
  /// "loudness_ms":67000}`) when the handler reports a breakdown.
  /// Null for legacy rows + kinds that don't emit timings.
  stage_timings_json: string | null;
}

/// Live progress entry for an in-flight job. Returned in the
/// `/admin/jobs` response keyed by job id; missing entries mean the
/// job isn't currently executing.
export interface JobProgress {
  stage: string;
  percent: number | null;
  updated_at_ms: number;
}

export interface JobSweepCounts {
  markers: number;
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

/// Returned by `peopleApi.get(id)` — the full Person row plus every
/// item in the user's accessible libraries that credits them. Empty
/// items array means "person exists in the catalog but you can't see
/// any of their titles" (e.g. they only appear on hidden libraries).
export interface PersonDetail extends Person {
  items: ListedItem[];
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
  /// True when the episode has at least one downloaded (non-removed)
  /// media file and is therefore playable. False marks a PLACEHOLDER
  /// episode the metadata agent materialized for an in-progress / future
  /// season (correct finale flag + calendar coverage) that has no file
  /// behind it. Optional + defaulted-true on the client so responses that
  /// predate the backend field never hide a real, downloaded episode.
  has_file?: boolean;
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

/// One locally-known episode whose air date falls inside the calendar
/// window — the LOCAL-data complement to the Trakt-driven coming-soon rail.
/// Field names are camelCase (the backend `UpcomingEpisode` is
/// `#[serde(rename_all = "camelCase")]`). `airDate` is epoch milliseconds,
/// date-granular (midnight UTC). The frontend groups by `airDate`.
export interface CalendarEpisode {
  episodeId: number;
  showId: number;
  showTitle: string;
  seasonNumber: number;
  episodeNumber: number;
  episodeTitle: string | null;
  airDate: number;
  durationMs: number | null;
  /// Highest episode number in this episode's season (for finale flagging).
  maxEpisodeNumber: number;
  isFinale: boolean;
  isPremiere: boolean;
  /// True when the episode already has a downloaded (live) file; false for a
  /// placeholder the metadata agent materialized for its air date but that
  /// isn't downloaded yet. Drives the "Available" vs "Not yet" affordance —
  /// an episode airing "today" may not have a file until later in the day.
  hasFile: boolean;
  /// Episode still, if scanned.
  stillPath: string | null;
  /// Parent show poster / backdrop.
  posterPath: string | null;
  backdropPath: string | null;
}

export interface CalendarResponse {
  episodes: CalendarEpisode[];
}

export type ScanStatus =
  | "queued"
  | "running"
  | "succeeded"
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
  // `bitrate_bps` is optional: the player ships a pure "Resolution"
  // picker and lets the backend ladder fill in the bitrate. Use the
  // separate `bitrate_cap_bps` below for a user-controlled cap.
  quality_target?: {
    height: number;
    bitrate_bps?: number;
  };
  // User-controlled video bitrate cap in bits per second. Independent
  // of resolution — lets a viewer pick "1080p but cap at 3 Mbps for
  // my mobile plan." Applied after any resolution-driven default and
  // before the operator's server-wide ceiling. Omit / 0 = no cap.
  bitrate_cap_bps?: number;
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

/// One library bound to a group, with the level the group grants (phase 107).
export interface GroupLibraryGrant {
  library_id: number;
  level: AccessLevel;
}

export interface AccessGroupDetail extends AccessGroup {
  member_ids: number[];
  /// Bare library ids the group grants (any level). `library_grants` carries
  /// the per-library level.
  library_ids: number[];
  library_grants: GroupLibraryGrant[];
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
  /** Master switch for open self-registration. When true (default),
   *  `/auth/register` provisions an account even with no invite code.
   *  When false the server is invite-only — codeless self-signup is
   *  rejected, but invite-bearing registration still works. */
  allow_signups: boolean;
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
  /** Worker count for the durable job queue (markers/loudness/subtitles/ratings). */
  job_workers: number;
  /** JSON object mapping job_kind → concurrency cap. Empty `{}` =
   *  registry defaults for every kind. Hot-reloadable. */
  job_kind_concurrency: string;
  /** When true (default), HDR sources are tone-mapped to SDR. */
  transcoder_hdr_tonemap_enabled: boolean;
  /** Algorithm passed to ffmpeg's tonemap filter. */
  transcoder_hdr_tonemap_algo: TonemapAlgorithm;
  /** When true, text subtitles (SRT / ASS / SSA / mov_text) are burned
   *  into the video via the `subtitles=` filter. Default false — text
   *  subs take the WebVTT-sidecar overlay path instead. Picture subs
   *  (PGS / VobSub / DVB) always burn regardless. */
  transcoder_burn_ass_subtitles: boolean;
  /** When true, EBU R 128 volume leveling uses precise per-file
   *  measurements (loudnorm two-pass / `linear=true`) instead of the
   *  single-pass estimate. Default false. Only takes effect when
   *  normalization is engaged. */
  transcoder_two_pass_loudnorm: boolean;
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
  /** Swap the inotify backend for `notify::PollWatcher`. Required for
   *  NFS/SMB mounts and bind-mounted-into-container setups where
   *  inotify events don't propagate. Off by default. Restart required. */
  file_watcher_use_polling: boolean;
  /** Seconds between polling-watcher rescans. Only consulted when
   *  `file_watcher_use_polling` is true. Default 30. */
  file_watcher_poll_interval_secs: number;
  /** Plex-style periodic full rescan of every library, independent of the
   *  filesystem watcher. Default on. Takes effect immediately. */
  periodic_scan_enabled: boolean;
  /** Periodic-scan interval as a scheduler frequency token (one of
   *  SCAN_INTERVALS): every_15_minutes | every_30_minutes | hourly |
   *  every_2_hours | every_6_hours | every_12_hours | daily. Default hourly. */
  periodic_scan_frequency: string;
  /** When on, a completed scan immediately hard-deletes that library's
   *  soft-removed files instead of waiting for the 7-day grace window.
   *  Default off. */
  empty_trash_after_scan: boolean;
  /** Server-wide default for ffmpeg loudnorm. When ON, every transcode
   *  session gets the filter applied (uses stored per-file
   *  measurements when available, else generic targets). Per-session
   *  override possible via the player audio menu. */
  audio_normalize_enabled: boolean;
  /** Server-wide default subtitle sync offset in ms, added to the
   *  client-supplied per-file offset before the WebVTT cue shift.
   *  Lets the operator paper over a library-wide source drift so the
   *  player stepper stays a relative tweak. 0 = no global shift.
   *  Positive delays subtitles, negative advances them. Clamped
   *  ±30_000 ms server-side. */
  subtitle_default_offset_ms: number;
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

/** Envelope returned by GET/PATCH `/admin/settings`. `version` and
 *  `data_dir` are read-only facts surfaced for display — PATCH ignores
 *  them and they have no field on `ServerSettingsUpdate`. */
export interface SettingsResponse {
  settings: ServerSettings;
  /** Server build version (the server crate's package version). */
  version: string;
  /** On-disk DATA_DIR path the server is running against. */
  data_dir: string;
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
  allow_signups?: boolean;
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
  transcoder_burn_ass_subtitles?: boolean;
  transcoder_two_pass_loudnorm?: boolean;
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
  file_watcher_use_polling?: boolean;
  file_watcher_poll_interval_secs?: number;
  periodic_scan_enabled?: boolean;
  periodic_scan_frequency?: string;
  empty_trash_after_scan?: boolean;
  audio_normalize_enabled?: boolean;
  subtitle_default_offset_ms?: number;
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
  /** Cap on auto-backup snapshots kept under
   *  `<data_dir>/backups/auto/`. Default 14, clamped 0..=365 (0
   *  disables pruning). See PUBLIC_RELEASE_HARDENING.md BLOCK #4. */
  backup_retention_count?: number;
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
  /// Resolved display name for `actor_user_id` (display_name ??
  /// username), stitched server-side via a batched lookup on the
  /// `/admin/audit` list endpoint. Null when the actor id is null or no
  /// longer resolves — fall back to "user #{actor_user_id}". Absent on
  /// other surfaces that return raw `AuditLogEntry` (e.g. maintenance
  /// dashboard), so it's optional on the shared type.
  actor_name?: string | null;
}

export interface AuditListResponse {
  entries: AuditLogEntry[];
  next_before: number | null;
  /** Total rows in audit_log — drives the paginated admin UI's
   *  "X–Y of Z" footer and jump-to-page buttons. */
  total: number;
}

/** Query params accepted by both `audit.list` and `audit.exportCsv`. */
export interface AuditListParams {
  before?: number;
  limit?: number;
  offset?: number;
  /** Filter to entries authored by this user id. Drives the Audit tab in
   *  the user-management drawer. */
  actor_user_id?: number;
  /** Substring match on the `action` column. */
  action?: string;
  /** Lower bound on the entry timestamp, epoch ms (inclusive). */
  from?: number;
  /** Upper bound on the entry timestamp, epoch ms (inclusive). */
  to?: number;
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

/** Whole-library bulk operation selector. `mark_watched` /
 *  `mark_unwatched` affect only the acting operator's play-state;
 *  `rescan` queues a library scan; `delete` destroys all content. */
export type LibraryBulkOp =
  | "mark_watched"
  | "mark_unwatched"
  | "rescan"
  | "delete";

export interface LibraryBulkResponse {
  library_id: number;
  op: LibraryBulkOp;
  /** Items/episodes whose play-state changed (mark watched/unwatched). */
  affected?: number;
  /** Top-level items deleted (delete op; children cascade). */
  deleted_items?: number;
  /** Queued scan job id (rescan op). */
  scan_job_id?: number;
  /** One-line human summary suitable for a banner. */
  message: string;
}

export const bulkLibrary = {
  markWatched: (library_id: number) =>
    apiFetch<LibraryBulkResponse>("/admin/libraries/bulk", {
      method: "POST",
      body: { library_id, op: "mark_watched" satisfies LibraryBulkOp },
    }),
  markUnwatched: (library_id: number) =>
    apiFetch<LibraryBulkResponse>("/admin/libraries/bulk", {
      method: "POST",
      body: { library_id, op: "mark_unwatched" satisfies LibraryBulkOp },
    }),
  rescan: (library_id: number) =>
    apiFetch<LibraryBulkResponse>("/admin/libraries/bulk", {
      method: "POST",
      body: { library_id, op: "rescan" satisfies LibraryBulkOp },
    }),
  /** DESTRUCTIVE. The backend rejects unless `confirm_library_id`
   *  echoes `library_id` AND `confirm_name` exactly matches the
   *  library's name. */
  deleteContent: (
    library_id: number,
    confirm_library_id: number,
    confirm_name: string,
  ) =>
    apiFetch<LibraryBulkResponse>("/admin/libraries/bulk", {
      method: "POST",
      body: {
        library_id,
        op: "delete" satisfies LibraryBulkOp,
        confirm_library_id,
        confirm_name,
      },
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
  /** True when the access token is within ~10 days of expiring AND
   *  no sync has refreshed it lately. Drives the "your Trakt link
   *  is about to silently expire" warning in Settings. See
   *  PUBLIC_RELEASE_HARDENING.md MONTH 1. */
  expiring_soon: boolean;
  /** True when the access token has already expired. The next sync
   *  attempts a refresh; if that fails, the user must re-link. */
  expired: boolean;
}

export interface TraktSyncNowResult {
  /** True when a fresh sync was enqueued; false when an equivalent
   *  pull/push pair was already queued or running (the button was
   *  pressed again before the prior run finished). Either way the work
   *  runs in the background — a real sync pushes the full local history
   *  and pulls history + playback + watchlist, which can take over a
   *  minute, so it no longer blocks the request. Watched status and
   *  watchlist update as the jobs complete. */
  queued: boolean;
}

export interface TraktUserStats {
  movies: {
    plays: number;
    watched: number;
    minutes: number;
    collected: number;
  };
  shows: { watched: number; collected: number };
  episodes: {
    plays: number;
    watched: number;
    minutes: number;
    collected: number;
  };
  ratings: { total: number };
}

export interface TraktUpcomingEpisode {
  show_item_id: number | null;
  show_title: string;
  season: number;
  episode: number;
  episode_title: string | null;
  first_aired: string;
  show_tmdb_id: number | null;
}

export interface TraktUpcomingResponse {
  items: TraktUpcomingEpisode[];
}

export interface TraktUpcomingMovie {
  movie_item_id: number | null;
  title: string;
  year: number | null;
  released: string;
  tmdb_id: number | null;
}

export interface TraktUpcomingMoviesResponse {
  items: TraktUpcomingMovie[];
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
  /** Upcoming episodes for shows the user has watched on Trakt.
   *  `days` clamps to 1..=31 server-side; default 14. `variant`:
   *    - "shows" (default): every upcoming episode
   *    - "premieres": season premieres only
   *    - "new": series premieres (brand-new shows)
   *  Returns an empty list when the user hasn't linked Trakt. */
  calendarShows: (
    days = 14,
    variant: "shows" | "premieres" | "new" = "shows",
  ) =>
    apiFetch<TraktUpcomingResponse>(
      `/trakt/calendars/shows?days=${days}&variant=${variant}`,
    ),
  /** Upcoming movie releases for movies the user tracks on Trakt
   *  (watchlist + collection). `days` defaults to 30 (movies dribble
   *  out more sparsely than weekly episodes). */
  calendarMovies: (days = 30) =>
    apiFetch<TraktUpcomingMoviesResponse>(
      `/trakt/calendars/movies?days=${days}`,
    ),
  /** Lifetime watch totals from `/users/me/stats`. Returns `null`
   *  when the user hasn't linked Trakt. */
  stats: () => apiFetch<TraktUserStats | null>("/trakt/stats"),
  /** Personalized Trakt recommendations intersected with the user's
   *  accessible local library. Pass `"movie"` or `"show"`. Returns an
   *  empty list when the user hasn't linked Trakt. */
  recommendations: (kind: "movie" | "show") =>
    apiFetch<{ items: ListedItem[] }>(
      `/trakt/recommendations?kind=${kind}`,
    ),
  /** The user's personal Trakt lists, each hydrated with the subset
   *  of items in their accessible local library. Lists with no local
   *  intersection are omitted server-side. */
  lists: () => apiFetch<TraktListsResponse>("/trakt/lists"),
  /** The user's Trakt favorites (their curated subset of the
   *  watchlist), intersected with the local library. Read-only:
   *  there's no local Favorites concept to write to. */
  favorites: () => apiFetch<{ items: ListedItem[] }>("/trakt/favorites"),
};

export interface TraktListView {
  id: number;
  slug: string;
  name: string;
  description: string | null;
  items: ListedItem[];
}

export interface TraktListsResponse {
  lists: TraktListView[];
}

// ─── Plex OAuth (PIN flow) ────────────────────────────────────────────────

export type PlexStartInput =
  | { intent: "login" }
  | { intent: "signup"; invite_code: string }
  | { intent: "link" };

export interface PlexStartResponse {
  pin_handle: string;
  auth_url: string;
  user_code: string;
  expires_in: number;
}

/// Poll outcomes. `Ready` is only emitted for the `link` intent —
/// login / signup return the classic `{ user }` payload (with Set-Cookie
/// session) directly, same shape as the password-login response, so
/// the post-login bootstrap path is shared.
export type PlexPollResult =
  | { status: "pending" }
  | { status: "expired" }
  | { status: "unknown_handle" }
  | { status: "not_linked"; plex_username: string }
  | { status: "linked" }
  // login + signup responses come back as the classic auth payload
  // (no `status` field), so we surface it as a separate variant the
  // caller hits via `if ("user" in result)`.
  | AuthResponse;

export interface PlexLinkSummary {
  provider: string;
  external_username: string | null;
  external_email: string | null;
  linked_at: number;
  last_login_at: number | null;
}

export const plex = {
  start: (input: PlexStartInput) =>
    apiFetch<PlexStartResponse>("/auth/plex/start", {
      method: "POST",
      body: input,
    }),
  poll: (pin_handle: string) =>
    apiFetch<PlexPollResult>("/auth/plex/poll", {
      method: "POST",
      body: { pin_handle },
    }),
  listLinks: () => apiFetch<PlexLinkSummary[]>("/auth/plex/link"),
  unlink: () =>
    apiFetch<{ removed: boolean }>("/auth/plex/link", { method: "DELETE" }),
};

export interface RatingResponse {
  rating: number | null;
}

export interface AllRatingsResponse {
  items: Record<string, number>;
  episodes: Record<string, number>;
}

export const ratings = {
  /// Bulk fetch — every rating the current user has set, in one call.
  /// Used by `lib/likes.ts` so the home page renders Like state on
  /// every card without firing N parallel `/items/:id/rating` GETs.
  listMine: () => apiFetch<AllRatingsResponse>("/ratings"),
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
  items: LibraryHealthItemRow[];
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
  /// State of the ffmpeg child for this session. `healthy` is the
  /// normal case; `exited` means the heartbeat probe or the stderr-
  /// drain task saw the process gone — the row should render a
  /// warning pill in the admin dashboard.
  transcode_health?: TranscodeHealth;
  /// Subtitle sidecar extraction state, when the session has a
  /// sidecar. Absent on burn-in / no-subtitle sessions.
  subtitle_health?: SubtitleHealth | null;
  /// Resolved display name for `user_id` (display_name ?? username),
  /// stitched server-side via a batched lookup. Null when the user no
  /// longer resolves — fall back to "User #{user_id}".
  username?: string | null;
  /// Resolved human title for `media_file_id` — a movie title, or the
  /// show name for an episode (with `subtitle` carrying "S1E4 — …").
  /// Null when the file no longer resolves — fall back to "#{media_file_id}".
  title?: string | null;
  /// Episode descriptor ("S1E4 — Episode title") for TV sessions; null
  /// for movie sessions.
  subtitle?: string | null;
}

/// Mirrors `chimpflix_transcoder::TranscodeHealth`.
export type TranscodeHealth =
  | { kind: "healthy" }
  | { kind: "exited"; detail: string; at_ms: number };

/// Mirrors `chimpflix_transcoder::SubtitleHealth`.
export type SubtitleHealth =
  | { kind: "pending" }
  | { kind: "ready" }
  | { kind: "failed"; reason: string };

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
  /** Global count of movie items across every library. Drives the
   *  "N · M movies · K eps" Library hero tile. */
  movie_count: number;
  /** Global count of episodes across every library. */
  episode_count: number;
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
  // Reject anything before the first '/' (after the leading slash) that
  // contains a colon — e.g. '/javascript:alert(1)' or '/:scheme/path'.
  const nextSlash = candidate.indexOf("/", 1);
  const firstSegment = nextSlash === -1 ? candidate.slice(1) : candidate.slice(1, nextSlash);
  if (firstSegment.includes(":")) return fallback;
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
    apiFetch<{ revoked: number }>("/auth/me/sessions/revoke-others", {
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
  }) => apiFetch<AuthResponse>("/auth/register", { method: "POST", body: input, noAuth: true }),
  listInvites: () =>
    apiFetch<{ invites: InviteListEntry[] }>("/admin/invites"),
  createInvite: (input: CreateInviteInput) =>
    apiFetch<CreatedInvite>("/admin/invites", { method: "POST", body: input }),
  revokeInvite: (id: number) =>
    apiFetch<void>(`/admin/invites/${id}`, { method: "DELETE" }),
  listUsers: () => apiFetch<{ users: User[] }>("/admin/users"),
  deleteUser: (id: number) =>
    apiFetch<void>(`/admin/users/${id}`, { method: "DELETE" }),
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
    clearAll: () =>
      apiFetch<{ cleared: number }>("/notifications/clear", {
        method: "POST",
      }),
  },
  setUserRole: (id: number, role: UserRole) =>
    apiFetch<{ user: User }>(`/admin/users/${id}`, {
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
    apiFetch<{ library_ids: number[] }>("/auth/me/hidden-libraries"),
  setHiddenLibraries: (library_ids: number[]) =>
    apiFetch<void>("/auth/me/hidden-libraries", {
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
  /** Sum of every (non-removed) media file's duration in ms. */
  total_runtime_ms: number;
  /** Items that have at least one poster image. */
  items_with_poster: number;
  /** Items lacking every external id (tmdb / tvdb / imdb). */
  items_missing_ids: number;
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
  /// Per-library, type-aware Top 10 (Movies/Shows → TMDB top-rated,
  /// Anime → MyAnimeList ranking, blended with local top-watched). The
  /// source is decided server-side from the library's kind.
  libraryTop: (libraryId: number, limit = 10) =>
    apiFetch<{ items: Array<ListedItem & { rank: number }> }>(
      `/libraries/${libraryId}/top`,
      { query: { limit } },
    ),
  /// Locally-known upcoming/recent episodes whose air date falls in the
  /// requested window — the LOCAL-data complement to the Trakt coming-soon
  /// rail. Honors the same per-library visibility + kids-safe rules as
  /// browse. Pass `days` (window ahead of now) and optionally
  /// `lookback_days` (days of already-aired context before today — the rail
  /// uses 0, the /calendar page uses 1 to lead with "Yesterday"), or an
  /// explicit `from`/`to` epoch-ms window. Pass `library_ids` to honor the
  /// user's visibility prefs.
  calendar: (
    opts: {
      days?: number;
      lookback_days?: number;
      from?: number;
      to?: number;
      limit?: number;
      library_ids?: ReadonlyArray<number>;
    } = {},
  ) =>
    apiFetch<CalendarResponse>("/calendar", {
      query: opts as Record<string, QueryValue>,
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
  /// Owner-only: enqueue an OpenSubtitles fetch for this title. For a
  /// show the background job fans out to every downloaded episode.
  /// `configured: false` means OpenSubtitles credentials aren't set;
  /// `language` echoes the resolved target (owner default → server
  /// metadata language → "en"). Fetched tracks appear in the player
  /// automatically once the job runs.
  fetchSubtitles: (id: number) =>
    apiFetch<{ queued: number; configured: boolean; language: string }>(
      `/items/${id}/fetch-subtitles`,
      { method: "POST" },
    ),
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
  listReviews: (id: number, opts?: { limit?: number; offset?: number }) => {
    const params = new URLSearchParams();
    if (opts?.limit !== undefined) params.set("limit", String(opts.limit));
    if (opts?.offset !== undefined) params.set("offset", String(opts.offset));
    const qs = params.toString();
    return apiFetch<{ reviews: Review[]; total: number }>(
      `/items/${id}/reviews${qs ? `?${qs}` : ""}`,
    );
  },
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
  /// Owner-only: fetch a single episode's subtitle from OpenSubtitles
  /// inline. `added` is 1 on a fresh download, 0 on a miss / already
  /// present; `configured: false` means credentials aren't set.
  fetchSubtitles: (id: number) =>
    apiFetch<{ added: number; configured: boolean; language: string }>(
      `/episodes/${id}/fetch-subtitles`,
      { method: "POST" },
    ),
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
  /** Source-file paths the server is unlinking in the background.
   *  Surfaced so the operator UI can confirm exactly what got
   *  removed. */
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

export interface CastSignResponse {
  /// Opaque HMAC token. Append as `?ct=<token>` to manifest +
  /// segment URLs the Cast receiver fetches; the server's
  /// `StreamAuthUser` extractor accepts it in lieu of the cookie.
  token: string;
  /// Absolute expiry (epoch ms). The player refreshes the token if
  /// it's within ~5 minutes of expiring while a cast session is
  /// still active.
  expires_at_ms: number;
}

/// Cast / AirPlay support. AirPlay needs no server-side handshake
/// (Safari reuses the local video element's cookies when bridging to
/// the AirPlay target), so this module only carries the Cast side.
export const cast = {
  sign: () => apiFetch<CastSignResponse>("/cast/sign", { method: "POST" }),
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
  history: (opts: { limit?: number; page?: number } = {}) =>
    apiFetch<{ items: ListedItem[]; total: number }>("/play-state/history", {
      // Use != null so that 0 is forwarded (not silently dropped by truthiness).
      query: {
        ...(opts.limit != null ? { limit: opts.limit } : {}),
        ...(opts.page != null ? { page: opts.page } : {}),
      },
    }),
  config: () => apiFetch<PlayStateConfig>("/play-state/config"),
};

export const people = {
  /// Fetch a person's full profile + the items in the user's
  /// accessible libraries that credit them. 404s when the person id
  /// doesn't exist in the catalog.
  get: (id: number) => apiFetch<PersonDetail>(`/people/${id}`),
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
    // Bypasses apiFetch (multipart, not JSON), so the double-submit CSRF
    // header must be attached by hand or the server 403s. Mirror
    // preroll.upload.
    const headers: Record<string, string> = {};
    const csrf = readCsrfToken();
    if (csrf) headers["X-CSRF-Token"] = csrf;
    const res = await fetch(`/api/v1/admin/collections/${id}/poster`, {
      method: "POST",
      body: fd,
      credentials: "include",
      headers,
    });
    if (!res.ok) {
      const text = await res.text().catch(() => "");
      throw new Error(text || `upload failed: ${res.status}`);
    }
  },
  uploadBackdrop: async (id: number, file: File) => {
    const fd = new FormData();
    fd.append("file", file);
    // See uploadPoster — multipart bypass of apiFetch needs the CSRF
    // header attached manually.
    const headers: Record<string, string> = {};
    const csrf = readCsrfToken();
    if (csrf) headers["X-CSRF-Token"] = csrf;
    const res = await fetch(`/api/v1/admin/collections/${id}/backdrop`, {
      method: "POST",
      body: fd,
      credentials: "include",
      headers,
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
  /// Total watch time over the window, in milliseconds. Aggregated as
  /// the sum of started-stream durations (see StatsOverview::watched_ms
  /// on the backend). Rendered as minutes / "≈ N hours".
  watched_ms: number;
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
  /// This user's watch time over the window, in milliseconds (sum of
  /// started-stream durations). Surfaced as an hours figure on the row.
  watched_ms: number;
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
  transcode_health?: TranscodeHealth;
  subtitle_health?: SubtitleHealth | null;
  /// Resolved display name for `user_id` (display_name ?? username).
  /// Null when the user no longer resolves — fall back to "User #{user_id}".
  username?: string | null;
  /// Resolved human title for `media_file_id` — movie title or show
  /// name. Null when unresolved — fall back to "#{media_file_id}".
  title?: string | null;
  /// Episode descriptor ("S1E4 — Episode title") for TV; null for movies.
  subtitle?: string | null;
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
      return apiFetch<{
        jobs: JobRow[];
        total: number;
        progress: Record<string, JobProgress>;
      }>(`/admin/jobs${suffix}`);
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
      return apiFetch<{ events: StatsActivityRow[] }>(`/admin/stats/recent-plays${suffix}`);
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
    get: () => apiFetch<SettingsResponse>("/admin/settings"),
    patch: (patch: ServerSettingsUpdate) =>
      apiFetch<SettingsResponse>("/admin/settings", {
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
    list: (params: AuditListParams = {}) =>
      apiFetch<AuditListResponse>("/admin/audit", {
        query: {
          before: params.before,
          limit: params.limit,
          offset: params.offset,
          actor_user_id: params.actor_user_id,
          action: params.action,
          from: params.from,
          to: params.to,
        },
      }),
    /** Download the filtered audit set as CSV. Same filters as `list`
     *  (pagination ignored server-side); the backend streams a
     *  `text/csv` attachment, which we wrap in a Blob + synthesized
     *  click so the browser shows its native save dialog. */
    exportCsv: async (
      params: Pick<
        AuditListParams,
        "actor_user_id" | "action" | "from" | "to"
      > = {},
    ): Promise<void> => {
      const qs = new URLSearchParams();
      if (params.action) qs.set("action", params.action);
      if (params.from != null) qs.set("from", String(params.from));
      if (params.to != null) qs.set("to", String(params.to));
      if (params.actor_user_id != null)
        qs.set("actor_user_id", String(params.actor_user_id));
      const suffix = qs.toString() ? `?${qs}` : "";
      const res = await fetch(`/api/v1/admin/audit/export${suffix}`, {
        credentials: "include",
      });
      if (!res.ok) {
        const text = await res.text().catch(() => "");
        throw new Error(text || `audit export failed: ${res.status}`);
      }
      const disposition = res.headers.get("Content-Disposition") ?? "";
      const match = disposition.match(/filename="([^"]+)"/);
      const filename = match?.[1] ?? `chimpflix-audit-${Date.now()}.csv`;
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
  /// Admin: lock (disable) a user account. A locked account fails the
  /// login gate after the password check. Owners can't be locked, and
  /// you can't lock your own account. Returns the updated user row.
  lockUser: (userId: number) =>
    apiFetch<{ user: User }>(`/admin/users/${userId}/lock`, {
      method: "POST",
    }),
  /// Admin: unlock (re-enable) a previously locked account.
  unlockUser: (userId: number) =>
    apiFetch<{ user: User }>(`/admin/users/${userId}/unlock`, {
      method: "POST",
    }),
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
    /// Replace the group's bound libraries. Pass `grants` for tri-state
    /// per-library levels ("view"/"full"); pass a bare `number[]` to bind
    /// every library at "full" (legacy binary shape).
    setLibraries: (id: number, grants: number[] | GroupLibraryGrant[]) => {
      const body =
        grants.length > 0 && typeof grants[0] === "object"
          ? { grants: grants as GroupLibraryGrant[] }
          : { library_ids: grants as number[] };
      return apiFetch<void>(`/admin/access-groups/${id}/libraries`, {
        method: "PUT",
        body,
      });
    },
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
  plex: {
    /// Rotate the per-install Plex client identifier so future PIN
    /// flows announce a fresh identity to Plex. Does NOT invalidate
    /// existing per-user Plex links — those are stored separately
    /// under each user's auth-provider row.
    rotateIdentifier: () =>
      apiFetch<{ rotated: boolean }>("/admin/plex/rotate-identifier", {
        method: "POST",
      }),
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
  versions: {
    list: () =>
      apiFetch<{ versions: OptimizedVersion[] }>("/admin/versions"),
    enqueue: (input: NewOptimizedVersionInput) =>
      apiFetch<OptimizedVersion>("/admin/versions", {
        method: "POST",
        body: input,
      }),
    delete: (id: number) =>
      apiFetch<void>(`/admin/versions/${id}`, { method: "DELETE" }),
    /**
     * Cancel a queued or running optimized version. Queued rows are
     * flipped straight to `cancelled`; running rows additionally have
     * their ffmpeg child killed and partial output removed by the
     * worker. Returns the row's reconciled state.
     */
    cancel: (id: number) =>
      apiFetch<OptimizedVersion>(`/admin/versions/${id}/cancel`, {
        method: "POST",
      }),
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
      apiFetch<{ entries: AccessMatrixEntry[] }>("/admin/access-matrix"),
    put: (libraries: LibraryAccessAssignment[]) =>
      apiFetch<{ entries: AccessMatrixEntry[] }>("/admin/access-matrix", {
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
    listDeliveries: (id: number, limit?: number, offset?: number) => {
      const query: Record<string, string | number | boolean> = {};
      if (limit !== undefined) query.limit = limit;
      if (offset !== undefined) query.offset = offset;
      return apiFetch<{ deliveries: WebhookDelivery[]; total: number }>(
        `/admin/webhooks/${id}/deliveries`,
        { query: Object.keys(query).length > 0 ? query : undefined },
      );
    },
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
    /** Re-run ffmpeg hardware detection without a server restart (after a
     *  driver/GPU change). Returns the fresh capabilities, which are also
     *  swapped into the live encoder-selection path server-side. */
    reprobeCapabilities: () =>
      apiFetch<{ capabilities: TranscoderCapabilities; cache_root: string }>(
        "/admin/transcoder/capabilities/reprobe",
        { method: "POST" },
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
    /// Row-based list of scheduled tasks. Kept only for the Admin
    /// Home dashboard's "Up next" / "Recently run" cards; per-kind
    /// editing now lives in `updateKindSchedule` below.
    list: () => apiFetch<TasksListResponse>("/admin/tasks"),
    /// Registry-driven views backing the rebuilt tasks UI.
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
  // Raw fetch (binary stream, not JSON) bypasses apiFetch, so attach the
  // double-submit CSRF header by hand or the POST 403s. Mirror
  // preroll.upload / collections.uploadPoster.
  const headers: Record<string, string> = {};
  const csrf = readCsrfToken();
  if (csrf) headers["X-CSRF-Token"] = csrf;
  const res = await fetch("/api/v1/admin/backups", {
    method: "POST",
    credentials: "include",
    headers,
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
  /** True when this server has at least one row encrypted at rest.
   *  Drives the "back up your vault key alongside the snapshot" banner
   *  on the admin backups page (see PUBLIC_RELEASE_HARDENING.md
   *  BLOCK #1). */
  vault_key_required: boolean;
  /** Cap on retained auto-snapshots from `backup_retention_count`.
   *  The UI uses this to surface "N of M retained" and warn before
   *  the daily prune kicks in. 0 means pruning is disabled (see
   *  PUBLIC_RELEASE_HARDENING.md BLOCK #4). */
  retention_count: number;
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

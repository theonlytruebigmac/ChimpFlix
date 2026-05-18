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

export type UserRole = "owner" | "user";

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

export interface ScheduledTask {
  id: number;
  kind: string;
  name: string;
  cron_expr: string;
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
  cron_expr: string;
  params_json?: string;
  enabled?: boolean;
}

export interface ScheduledTaskUpdate {
  name?: string;
  cron_expr?: string;
  params_json?: string;
  enabled?: boolean;
}

export interface TaskKindInfo {
  kind: string;
  display_name: string;
  description: string;
  params_schema: string;
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
}

export interface NetworkUpdateInput {
  public_url?: string | null;
  cors_origins?: string[];
  secure_connections?: SecureConnectionsMode;
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
  allowed: boolean;
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

export interface PrivacyResponse {
  telemetry_opt_in: boolean;
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
  tmdb_id: number;
  name: string;
  overview: string | null;
  poster_path: string | null;
  backdrop_path: string | null;
  item_count: number;
}

export interface CollectionDetail extends Collection {
  items: ListedItem[];
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
};

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
  transcoder_max_concurrent: number;
  transcoder_hw_accel: TranscoderHwAccel;
  transcoder_quality_ceiling_kbps: number | null;
  transcoder_encoder_preset: TranscoderEncoderPreset;
  transcoder_hw_strictness: TranscoderHwStrictness;
  // ── Email / SMTP (Phase 21) ──
  email_smtp_host: string | null;
  email_smtp_port: number | null;
  email_smtp_username: string | null;
  email_smtp_security: SmtpSecurity | null;
  email_from_address: string | null;
  email_from_name: string | null;
  // ── 2FA (Phase 24) ──
  totp_enforcement: TotpEnforcement;
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
  transcoder_max_concurrent?: number;
  transcoder_hw_accel?: TranscoderHwAccel;
  transcoder_quality_ceiling_kbps?: number | null;
  transcoder_encoder_preset?: TranscoderEncoderPreset;
  transcoder_hw_strictness?: TranscoderHwStrictness;
  email_smtp_host?: string | null;
  email_smtp_port?: number | null;
  email_smtp_username?: string | null;
  email_smtp_security?: SmtpSecurity | null;
  email_from_address?: string | null;
  email_from_name?: string | null;
  totp_enforcement?: TotpEnforcement;
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

async function apiFetch<T>(path: string, opts: FetchOptions = {}): Promise<T> {
  const url = buildUrl(path, opts.query);
  const headers: Record<string, string> = {};
  if (opts.body !== undefined) headers["Content-Type"] = "application/json";
  if (isServer() && !opts.noAuth) {
    const cookieHeader = await serverCookieHeader();
    if (cookieHeader) headers["Cookie"] = cookieHeader;
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
  if (res.status === 204) return undefined as T;
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
  create: (input: NewLibraryInput) =>
    apiFetch<{ library: Library }>("/libraries", { method: "POST", body: input }),
  get: (id: number) => apiFetch<{ library: Library }>(`/libraries/${id}`),
  update: (id: number, input: LibraryUpdateInput) =>
    apiFetch<{ library: Library }>(`/libraries/${id}`, {
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
  // ─── Edit & Fix Match (owner-only on the server) ───────────────────────
  patch: (id: number, edit: ItemEditInput) =>
    apiFetch<ItemDetail>(`/items/${id}`, { method: "PATCH", body: edit }),
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
};

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

export const playState = {
  update: (input: PlayStateUpdateInput) =>
    apiFetch<void>("/play-state", {
      method: "POST",
      body: { updates: [input] },
    }),
  scrobble: (input: ScrobbleInput) =>
    apiFetch<void>("/play-state/scrobble", { method: "POST", body: input }),
  setWatched: (
    input: { item_id?: number; episode_id?: number; watched: boolean },
  ) =>
    apiFetch<void>("/play-state/watched", { method: "POST", body: input }),
  onDeck: () => apiFetch<OnDeckResponse>("/play-state/on-deck"),
  history: (limit?: number) =>
    apiFetch<{ items: ListedItem[] }>("/play-state/history", {
      query: limit ? { limit } : undefined,
    }),
};

export const collections = {
  list: () =>
    apiFetch<{ collections: Collection[] }>("/collections"),
  get: (id: number) => apiFetch<CollectionDetail>(`/collections/${id}`),
};

// ---------------------------------------------------------------------------
// Health / server info
// ---------------------------------------------------------------------------

export const server = {
  health: () => apiFetch<HealthResponse>("/health", { noAuth: true }),
  info: () => apiFetch<ServerInfoResponse>("/server-info"),
};

// ─── Admin (owner-only) ────────────────────────────────────────────────────

export const admin = {
  dashboard: () => apiFetch<DashboardResponse>("/admin/dashboard"),
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
    list: (params: { before?: number; limit?: number } = {}) =>
      apiFetch<AuditListResponse>("/admin/audit", {
        query: {
          before: params.before,
          limit: params.limit,
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
  privacy: {
    get: () => apiFetch<PrivacyResponse>("/admin/privacy"),
    patch: (telemetry_opt_in: boolean) =>
      apiFetch<PrivacyResponse>("/admin/privacy", {
        method: "PATCH",
        body: { telemetry_opt_in },
      }),
  },
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
      apiFetch<{ capabilities: TranscoderCapabilities }>(
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

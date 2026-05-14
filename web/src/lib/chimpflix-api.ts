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
  default_audio_lang: string | null;
  default_subtitle_lang: string | null;
  created_at: number;
  updated_at: number;
}

export interface UpdateMeInput {
  // Empty string clears the field; omit to leave unchanged.
  display_name?: string;
  avatar_url?: string;
  default_audio_lang?: string;
  default_subtitle_lang?: string;
}

export interface AuthResponse {
  user: User;
}

export interface AuthStatusResponse {
  setup_needed: boolean;
}

export type LibraryKind = "movies" | "shows";

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
  poster_path: string | null;
  backdrop_path: string | null;
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
}

export interface SessionInfo {
  id: string;
  mode: "direct" | "transcode";
  direct_url?: string;
  hls_master_url?: string;
  media_file_id: number;
  duration_ms: number | null;
}

export interface CreateSessionResponse {
  session: SessionInfo;
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

export interface Invite {
  id: number;
  code: string;
  created_by: number;
  expires_at: number | null;
  consumed_by: number | null;
  consumed_at: number | null;
  created_at: number;
}

export interface CreateInviteInput {
  expires_in_seconds?: number;
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
  | "none"
  | "vaapi"
  | "nvenc"
  | "qsv"
  | "videotoolbox";

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
  /** JSON-encoded object used by later phases for forward-compat fields. */
  extras_json: string;
  updated_at: number;
  updated_by: number | null;
}

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
  extras_json?: string;
}

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

interface FetchOptions {
  method?: "GET" | "POST" | "PUT" | "PATCH" | "DELETE";
  body?: unknown;
  query?: Record<string, string | number | boolean | undefined>;
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
      if (v !== undefined && v !== null) url.searchParams.set(k, String(v));
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
  setup: (input: { username: string; password: string; display_name?: string }) =>
    apiFetch<AuthResponse>("/auth/setup", {
      method: "POST",
      body: input,
      noAuth: true,
    }),
  login: (input: { username: string; password: string }) =>
    apiFetch<AuthResponse>("/auth/login", {
      method: "POST",
      body: input,
      noAuth: true,
    }),
  logout: () => apiFetch<void>("/auth/logout", { method: "POST" }),
  me: () => apiFetch<AuthResponse>("/auth/me"),
  updateMe: (input: UpdateMeInput) =>
    apiFetch<AuthResponse>("/auth/me", { method: "PATCH", body: input }),
  register: (input: {
    code: string;
    username: string;
    password: string;
    display_name?: string;
  }) => apiFetch<AuthResponse>("/auth/register", { method: "POST", body: input }),
  listInvites: () => apiFetch<{ invites: Invite[] }>("/auth/invites"),
  createInvite: (input: CreateInviteInput) =>
    apiFetch<{ invite: Invite }>("/auth/invites", { method: "POST", body: input }),
  revokeInvite: (code: string) =>
    apiFetch<void>(`/auth/invites/${encodeURIComponent(code)}`, {
      method: "DELETE",
    }),
  listUsers: () => apiFetch<{ users: User[] }>("/auth/users"),
  deleteUser: (id: number) =>
    apiFetch<void>(`/auth/users/${id}`, { method: "DELETE" }),
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
};

// ---------------------------------------------------------------------------
// Items / seasons / episodes
// ---------------------------------------------------------------------------

export const items = {
  list: (filter: ItemFilter = {}) =>
    apiFetch<ItemPage>("/items", { query: filter as Record<string, string | number | undefined> }),
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
  deleteSession: (id: string) =>
    apiFetch<void>(`/stream/sessions/${encodeURIComponent(id)}`, {
      method: "DELETE",
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
};

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

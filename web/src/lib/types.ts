// TypeScript types mirroring the ChimpFlix backend API.
// See crates/library/src/models.rs for the source of truth.

export type UserRole = "owner" | "user";

export interface User {
  id: number;
  username: string;
  role: UserRole;
  display_name: string | null;
  created_at: number;
  updated_at: number;
}

export interface ApiErrorBody {
  error: { code: string; message: string };
}

export interface AuthMeResponse {
  user: User;
}

export interface AuthStatus {
  setup_needed: boolean;
}

export interface AuthSetupRequest {
  username: string;
  password: string;
  display_name?: string;
}

export interface LoginRequest {
  username: string;
  password: string;
}

export interface RegisterRequest {
  code: string;
  username: string;
  password: string;
  display_name?: string;
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

export interface InvitesListResponse {
  invites: Invite[];
}

export interface PlayState {
  position_ms: number;
  duration_ms: number | null;
  watched: boolean;
  view_count: number;
  last_played_at: number;
}

export interface Library {
  id: number;
  name: string;
  kind: "movies" | "shows";
  paths: string[];
  scan_interval_s: number;
  last_scan_at: number | null;
  created_at: number;
  updated_at: number;
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
  play_state?: PlayState | null;
}

export interface ItemPage {
  items: Item[];
  total: number;
  page: number;
  page_size: number;
}

export interface ItemFilter {
  library_id?: number;
  kind?: ItemKind;
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
}

export interface SeasonSummary {
  id: number;
  season_number: number;
  title: string | null;
  episode_count: number;
}

export interface ItemDetail extends Item {
  genres: string[];
  files?: MediaFileSummary[];
  seasons?: SeasonSummary[];
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
  play_state?: PlayState | null;
}

export interface EpisodeDetail extends Episode {
  files: MediaFileSummary[];
}

export interface SeasonDetail {
  id: number;
  show_id: number;
  season_number: number;
  title: string | null;
  summary: string | null;
  episodes: Episode[];
}

export interface PlayStateUpdate {
  item_id?: number;
  episode_id?: number;
  position_ms: number;
  duration_ms?: number;
  watched?: boolean;
}

export interface OnDeckEntry {
  kind: "movie" | "episode";
  item?: Item;
  episode?: Episode;
  show?: Item;
  play_state: PlayState;
}

export interface OnDeckResponse {
  items: OnDeckEntry[];
}

export interface ClientCapabilities {
  supported_video_codecs: string[];
  supported_audio_codecs: string[];
  supported_containers: string[];
  max_bandwidth_bps?: number;
}

export interface CreateSessionRequest {
  media_file_id: number;
  start_position_ms?: number;
  client: ClientCapabilities;
}

export interface SessionInfo {
  id: string;
  mode: "direct" | "transcode";
  direct_url?: string;
  hls_master_url?: string;
  media_file_id: number;
  duration_ms?: number;
}

export interface CreateSessionResponse {
  session: SessionInfo;
}

export interface ScanJob {
  id: number;
  library_id: number;
  status: "queued" | "running" | "completed" | "failed" | "canceled";
  started_at: number | null;
  finished_at: number | null;
  files_seen: number;
  files_added: number;
  files_updated: number;
  files_removed: number;
  error_message: string | null;
  created_at: number;
}

export interface ServerInfo {
  version: string;
  library_counts: {
    libraries: number;
    movies: number;
    shows: number;
    episodes: number;
  };
  tmdb_enabled: boolean;
}

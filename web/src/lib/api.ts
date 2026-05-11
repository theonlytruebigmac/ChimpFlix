// Thin, typed client for the ChimpFlix backend.
//
// All paths are relative to `/api/v1` — the Next.js rewrite in
// next.config.ts proxies that prefix to the Rust server, so the browser
// only ever sees a single origin.

import type {
  ApiErrorBody,
  AuthMeResponse,
  AuthSetupRequest,
  AuthStatus,
  CreateSessionRequest,
  CreateSessionResponse,
  EpisodeDetail,
  Invite,
  InvitesListResponse,
  Item,
  ItemDetail,
  ItemFilter,
  ItemPage,
  Library,
  LoginRequest,
  OnDeckResponse,
  PlayStateUpdate,
  RegisterRequest,
  ScanJob,
  SeasonDetail,
  ServerInfo,
} from "./types";

const API_BASE = "/api/v1";

export class ApiClientError extends Error {
  status: number;
  code?: string;
  constructor(message: string, status: number, code?: string) {
    super(message);
    this.name = "ApiClientError";
    this.status = status;
    this.code = code;
  }
  get isUnauthorized(): boolean {
    return this.status === 401;
  }
  get isForbidden(): boolean {
    return this.status === 403;
  }
  get isNotFound(): boolean {
    return this.status === 404;
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    credentials: "include",
    ...init,
    headers: {
      "Content-Type": "application/json",
      ...(init?.headers ?? {}),
    },
  });
  if (!res.ok) {
    let code: string | undefined;
    let message = res.statusText || `HTTP ${res.status}`;
    try {
      const body = (await res.json()) as ApiErrorBody;
      code = body?.error?.code;
      message = body?.error?.message ?? message;
    } catch {
      // Body wasn't JSON; leave defaults.
    }
    throw new ApiClientError(message, res.status, code);
  }
  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

function qs(params: Record<string, string | number | undefined>): string {
  const entries: [string, string][] = [];
  for (const [k, v] of Object.entries(params)) {
    if (v === undefined || v === "") continue;
    entries.push([k, String(v)]);
  }
  return entries.length ? `?${new URLSearchParams(entries).toString()}` : "";
}

export const chimpflix = {
  auth: {
    status: () => request<AuthStatus>("/auth/status"),
    setup: (body: AuthSetupRequest) =>
      request<AuthMeResponse>("/auth/setup", {
        method: "POST",
        body: JSON.stringify(body),
      }),
    login: (body: LoginRequest) =>
      request<AuthMeResponse>("/auth/login", {
        method: "POST",
        body: JSON.stringify(body),
      }),
    register: (body: RegisterRequest) =>
      request<AuthMeResponse>("/auth/register", {
        method: "POST",
        body: JSON.stringify(body),
      }),
    logout: () => request<void>("/auth/logout", { method: "POST" }),
    me: () => request<AuthMeResponse>("/auth/me"),
    invites: {
      list: () => request<InvitesListResponse>("/auth/invites"),
      create: (expiresInSeconds?: number) =>
        request<{ invite: Invite }>("/auth/invites", {
          method: "POST",
          body: JSON.stringify({ expires_in_seconds: expiresInSeconds }),
        }),
      revoke: (code: string) =>
        request<void>(`/auth/invites/${encodeURIComponent(code)}`, {
          method: "DELETE",
        }),
    },
  },
  serverInfo: () => request<ServerInfo>("/server-info"),
  libraries: {
    list: () => request<{ libraries: Library[] }>("/libraries"),
    triggerScan: (id: number) =>
      request<ScanJob>(`/libraries/${id}/scan`, { method: "POST" }),
  },
  items: {
    list: (filter: ItemFilter = {}) =>
      request<ItemPage>(
        `/items${qs(filter as Record<string, string | number | undefined>)}`,
      ),
    get: (id: number) => request<ItemDetail>(`/items/${id}`),
  },
  seasons: {
    get: (id: number) => request<SeasonDetail>(`/seasons/${id}`),
  },
  episodes: {
    get: (id: number) => request<EpisodeDetail>(`/episodes/${id}`),
  },
  playState: {
    update: (updates: PlayStateUpdate[]) =>
      request<void>("/play-state", {
        method: "POST",
        body: JSON.stringify({ updates }),
      }),
    scrobble: (req: { item_id?: number; episode_id?: number }) =>
      request<void>("/play-state/scrobble", {
        method: "POST",
        body: JSON.stringify(req),
      }),
    onDeck: () => request<OnDeckResponse>("/play-state/on-deck"),
  },
  stream: {
    createSession: (req: CreateSessionRequest) =>
      request<CreateSessionResponse>("/stream/sessions", {
        method: "POST",
        body: JSON.stringify(req),
      }),
    deleteSession: (id: string) =>
      request<void>(`/stream/sessions/${encodeURIComponent(id)}`, {
        method: "DELETE",
      }),
    /** Direct play URL for a `<video src>` (browser sends cookies). */
    directUrl: (fileId: number) => `${API_BASE}/stream/${fileId}/direct`,
  },
};

export type Chimpflix = typeof chimpflix;
export type { Item, ItemDetail };

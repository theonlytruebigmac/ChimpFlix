"use client";

import { useEffect, useState } from "react";
import {
  auth as authApi,
  ChimpFlixApiError,
  type MySessionEntry,
} from "@/lib/chimpflix-api";
import { ConfirmDialog } from "./ConfirmDialog";

export function SettingsSessionsClient() {
  const [sessions, setSessions] = useState<MySessionEntry[] | null>(null);
  const [busy, setBusy] = useState<number | "all" | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [askRevokeOne, setAskRevokeOne] = useState<{ id: number; label: string } | null>(null);
  const [askRevokeOthers, setAskRevokeOthers] = useState(false);

  useEffect(() => {
    void refresh();
  }, []);

  async function refresh() {
    try {
      const { sessions } = await authApi.listMySessions();
      setSessions(sessions);
    } catch (e) {
      setError(parseError(e));
      setSessions([]);
    }
  }

  function revokeOne(id: number, label: string) {
    setAskRevokeOne({ id, label });
  }

  async function confirmRevokeOne() {
    if (!askRevokeOne) return;
    const { id, label } = askRevokeOne;
    setAskRevokeOne(null);
    setBusy(id);
    setError(null);
    setMessage(null);
    try {
      await authApi.revokeMySession(id);
      setMessage(`Signed out ${label}.`);
      await refresh();
    } catch (e) {
      setError(parseError(e));
    } finally {
      setBusy(null);
    }
  }

  function revokeOthers() {
    setAskRevokeOthers(true);
  }

  async function confirmRevokeOthers() {
    setAskRevokeOthers(false);
    setBusy("all");
    setError(null);
    setMessage(null);
    try {
      const { revoked } = await authApi.revokeOtherSessions();
      setMessage(
        revoked === 0
          ? "No other active sessions."
          : `Signed out ${revoked} other ${revoked === 1 ? "session" : "sessions"}.`,
      );
      await refresh();
    } catch (e) {
      setError(parseError(e));
    } finally {
      setBusy(null);
    }
  }

  if (sessions === null) {
    return <p className="text-xs text-white/55">Loading sessions…</p>;
  }

  const others = sessions.filter((s) => !s.current).length;

  return (
    <div className="space-y-3">
      <p className="text-xs text-white/55">
        Sessions stay valid for 30 days. Revoke any you don&apos;t recognise.
      </p>

      <ul className="divide-y divide-white/5 overflow-hidden rounded-md border border-white/10 bg-black/20">
        {sessions.length === 0 && (
          <li className="px-3 py-4 text-center text-xs text-white/50">
            No active sessions.
          </li>
        )}
        {sessions.map((s) => (
          <li
            key={s.id}
            className="flex items-start justify-between gap-3 px-3 py-2.5 text-xs"
          >
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <span className="font-medium text-white/90">
                  {summarizeUserAgent(s.user_agent)}
                </span>
                {s.current && (
                  <span className="rounded-full border border-emerald-500/40 bg-emerald-500/10 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-emerald-200">
                    This device
                  </span>
                )}
              </div>
              <div className="mt-0.5 text-white/50">
                {s.ip ?? "unknown IP"} · last seen {formatAge(s.last_seen_at)}
              </div>
              <div className="text-[10px] uppercase tracking-wider text-white/35">
                Created {new Date(s.created_at).toLocaleDateString()} · expires{" "}
                {new Date(s.expires_at).toLocaleDateString()}
              </div>
            </div>
            <button
              type="button"
              onClick={() =>
                revokeOne(
                  s.id,
                  s.current ? "this device" : summarizeUserAgent(s.user_agent),
                )
              }
              disabled={busy === s.id}
              className="rounded border border-white/15 px-2.5 py-1 text-[11px] font-medium text-white/80 hover:bg-white/5 disabled:opacity-50"
            >
              {busy === s.id ? "…" : "Sign out"}
            </button>
          </li>
        ))}
      </ul>

      {others > 0 && (
        <button
          type="button"
          onClick={revokeOthers}
          disabled={busy === "all"}
          className="rounded border border-white/15 px-3 py-2 text-xs font-medium text-white/80 hover:bg-white/5 disabled:opacity-50"
        >
          {busy === "all" ? "Signing out…" : `Sign out of all ${others} other ${others === 1 ? "device" : "devices"}`}
        </button>
      )}

      {error && (
        <div className="rounded border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {error}
        </div>
      )}
      {message && <p className="text-xs text-white/70">{message}</p>}
      {askRevokeOne && (
        <ConfirmDialog
          title={`Sign out ${askRevokeOne.label}?`}
          body="That device will need to log in again. Anything currently playing on it stops at the next heartbeat."
          confirmLabel="Sign out"
          destructive
          busy={busy === askRevokeOne.id}
          onConfirm={() => void confirmRevokeOne()}
          onCancel={() => setAskRevokeOne(null)}
        />
      )}
      {askRevokeOthers && (
        <ConfirmDialog
          title="Sign out of all other devices?"
          body="You'll stay signed in here; every other session needs to log in again."
          confirmLabel="Sign out others"
          destructive
          busy={busy === "all"}
          onConfirm={() => void confirmRevokeOthers()}
          onCancel={() => setAskRevokeOthers(false)}
        />
      )}
    </div>
  );
}

function summarizeUserAgent(ua: string | null): string {
  if (!ua) return "Unknown device";
  // Light-touch UA sniff — just enough to render a useful label
  // without pulling in a UA-parser dep.
  const u = ua.toLowerCase();
  let browser = "Browser";
  if (u.includes("firefox")) browser = "Firefox";
  else if (u.includes("edg/")) browser = "Edge";
  else if (u.includes("opr/") || u.includes("opera")) browser = "Opera";
  else if (u.includes("chrome")) browser = "Chrome";
  else if (u.includes("safari")) browser = "Safari";
  let os = "";
  if (u.includes("windows")) os = "Windows";
  else if (u.includes("mac os") || u.includes("macintosh")) os = "macOS";
  else if (u.includes("android")) os = "Android";
  else if (u.includes("iphone") || u.includes("ipad")) os = "iOS";
  else if (u.includes("linux")) os = "Linux";
  return os ? `${browser} on ${os}` : browser;
}

function formatAge(ms: number): string {
  const diffMin = Math.max(0, Math.floor((Date.now() - ms) / 60_000));
  if (diffMin < 1) return "just now";
  if (diffMin < 60) return `${diffMin}m ago`;
  if (diffMin < 60 * 24) return `${Math.floor(diffMin / 60)}h ago`;
  return `${Math.floor(diffMin / (60 * 24))}d ago`;
}

function parseError(e: unknown): string {
  if (e instanceof ChimpFlixApiError) {
    try {
      const parsed = JSON.parse(e.body) as { error?: { message?: string } };
      if (parsed.error?.message) return parsed.error.message;
    } catch {
      /* fall through */
    }
    return `HTTP ${e.status}`;
  }
  return "Network error";
}

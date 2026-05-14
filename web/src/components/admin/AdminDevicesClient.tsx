"use client";

import { useState } from "react";
import {
  admin as adminApi,
  type AdminSessionSummary,
} from "@/lib/chimpflix-api";

export function AdminDevicesClient({ initial }: { initial: AdminSessionSummary[] }) {
  const [sessions, setSessions] = useState(initial);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function refresh() {
    try {
      const r = await adminApi.sessions.list();
      setSessions(r.sessions);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  async function revoke(id: number) {
    setBusy(true);
    setError(null);
    try {
      await adminApi.sessions.revoke(id);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-4">
      {error && (
        <div className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {error}
        </div>
      )}

      {sessions.length === 0 ? (
        <div className="rounded-lg border border-dashed border-white/15 bg-white/2 p-8 text-center text-sm text-white/50">
          No active sessions.
        </div>
      ) : (
        <div className="overflow-hidden rounded-lg border border-white/10">
          <table className="w-full text-sm">
            <thead className="bg-white/5 text-left text-xs uppercase tracking-wider text-white/40">
              <tr>
                <th className="px-4 py-2">User</th>
                <th className="px-4 py-2">Device</th>
                <th className="px-4 py-2">IP</th>
                <th className="px-4 py-2">Last seen</th>
                <th className="px-4 py-2">Expires</th>
                <th className="px-4 py-2" />
              </tr>
            </thead>
            <tbody>
              {sessions.map((s) => (
                <tr key={s.id} className="border-t border-white/5">
                  <td className="whitespace-nowrap px-4 py-2 font-medium">
                    @{s.username}
                  </td>
                  <td className="px-4 py-2 text-xs text-white/60">
                    <div className="line-clamp-2 max-w-md" title={s.user_agent ?? ""}>
                      {summarizeUserAgent(s.user_agent)}
                    </div>
                  </td>
                  <td className="whitespace-nowrap px-4 py-2 font-mono text-xs text-white/60">
                    {s.ip ?? "—"}
                  </td>
                  <td className="whitespace-nowrap px-4 py-2 text-white/60">
                    {new Date(s.last_seen_at).toLocaleString()}
                  </td>
                  <td className="whitespace-nowrap px-4 py-2 text-white/60">
                    {new Date(s.expires_at).toLocaleDateString()}
                  </td>
                  <td className="whitespace-nowrap px-4 py-2 text-right">
                    <button
                      disabled={busy}
                      onClick={() => revoke(s.id)}
                      className="rounded border border-white/15 px-2 py-1 text-xs text-white/70 hover:border-red-500/50 hover:text-red-300"
                    >
                      Revoke
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

// Compress the UA to "Browser on Platform" if we can recognize the family,
// otherwise show the raw string (truncated by the line-clamp).
function summarizeUserAgent(ua: string | null): string {
  if (!ua) return "—";
  const browser = /Edg\/(\d+)/.exec(ua)
    ? "Edge"
    : /Chrome\/(\d+)/.exec(ua)
      ? "Chrome"
      : /Firefox\/(\d+)/.exec(ua)
        ? "Firefox"
        : /Safari\/(\d+)/.exec(ua)
          ? "Safari"
          : null;
  const os = /Windows/.exec(ua)
    ? "Windows"
    : /Mac OS X/.exec(ua)
      ? "macOS"
      : /Linux/.exec(ua)
        ? "Linux"
        : /iPhone|iPad/.exec(ua)
          ? "iOS"
          : /Android/.exec(ua)
            ? "Android"
            : null;
  if (browser && os) return `${browser} on ${os}`;
  if (browser) return browser;
  return ua;
}

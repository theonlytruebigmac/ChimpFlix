"use client";

import { useState } from "react";
import {
  admin as adminApi,
  type AdminSessionSummary,
} from "@/lib/chimpflix-api";
import { Pill } from "./ui";

export function AdminDevicesClient({ initial }: { initial: AdminSessionSummary[] }) {
  const [sessions, setSessions] = useState(initial);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Wall-clock at the time of the most recent fetch. Captured
  // alongside the session list so the "expiring soon" pill can be
  // computed during render without calling Date.now() there (which
  // trips react-hooks/purity). Zero on first render before initial
  // is timestamped — the pill just doesn't highlight, which is fine.
  const [nowMs, setNowMs] = useState(0);

  async function refresh() {
    try {
      const r = await adminApi.sessions.list();
      setSessions(r.sessions);
      // Date.now() inside an async handler is fine but
      // react-hooks/purity is conservative when the function is
      // defined at component-scope — disable here, this only runs
      // after the user clicks Revoke.
      // eslint-disable-next-line react-hooks/purity
      setNowMs(Date.now());
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

  // If every session IP is a private / loopback address, the reverse
  // proxy in front of the server isn't being trusted — meaning
  // X-Forwarded-For is ignored and we're recording the Docker bridge
  // peer IP instead of the real client. Flag visibly so the operator
  // knows it's a config issue, not a missing feature.
  const allPrivate =
    sessions.length > 0 && sessions.every((s) => isPrivateIp(s.ip));

  return (
    <div className="space-y-4">
      {error && (
        <div className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {error}
        </div>
      )}

      {allPrivate && (
        <div className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-200">
          <strong>All session IPs look private.</strong> Your reverse proxy is
          probably terminating the connection, leaving us with the LAN /
          Docker-bridge peer IP. Set the{" "}
          <code className="font-mono text-amber-100">TRUSTED_PROXIES</code> env
          var to a comma-separated CIDR list of your proxies (e.g.{" "}
          <code className="font-mono text-amber-100">172.16.0.0/12</code> for
          Docker, plus your Traefik / Cloudflare ranges) so we honour{" "}
          <code className="font-mono text-amber-100">X-Forwarded-For</code> and
          record the real client IP.
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
              {sessions.map((s) => {
                const expiring =
                  nowMs > 0 && s.expires_at - nowMs < 7 * 86_400_000;
                return (
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
                    <td className="whitespace-nowrap px-4 py-2">
                      {expiring ? (
                        <Pill tone="warn">
                          {new Date(s.expires_at).toLocaleDateString()}
                        </Pill>
                      ) : (
                        <span className="text-xs text-white/60">
                          {new Date(s.expires_at).toLocaleDateString()}
                        </span>
                      )}
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
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

// Return true for loopback / private IPv4 / unique-local IPv6. The
// banner above keys off this — if every session shows a private IP,
// the reverse proxy's `X-Forwarded-For` is being ignored.
function isPrivateIp(ip: string | null): boolean {
  if (!ip) return true;
  // IPv6 loopback + link-local + unique-local.
  if (ip === "::1") return true;
  if (ip.startsWith("fe80:") || ip.startsWith("fc") || ip.startsWith("fd")) {
    return true;
  }
  // Map ::ffff:1.2.3.4 → 1.2.3.4 so the v4 logic below handles it.
  const v4 = ip.startsWith("::ffff:") ? ip.slice(7) : ip;
  if (!/^\d{1,3}(\.\d{1,3}){3}$/.test(v4)) return false;
  const [a, b] = v4.split(".").map(Number);
  if (a === 10) return true;
  if (a === 127) return true;
  if (a === 172 && b >= 16 && b <= 31) return true;
  if (a === 192 && b === 168) return true;
  if (a === 169 && b === 254) return true;
  return false;
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

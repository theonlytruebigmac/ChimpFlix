"use client";

import { useState } from "react";
import {
  admin as adminApi,
  type AdminSessionSummary,
} from "@/lib/chimpflix-api";
import { ConfirmDialog } from "../ConfirmDialog";
import { formatDateTime } from "@/lib/format";

export function AdminDevicesClient({ initial }: { initial: AdminSessionSummary[] }) {
  const [sessions, setSessions] = useState(initial);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [askRevoke, setAskRevoke] = useState<AdminSessionSummary | null>(null);
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
    <div>
      <div className="cf-flex cf-between" style={{ marginBottom: 14 }}>
        <div className="cf-muted" style={{ fontSize: 13 }}>
          Every active session across all users.
        </div>
      </div>

      {error && (
        <div role="alert" aria-live="assertive" className="cf-banner cf-err">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 8v4M12 16v.5" />
          </svg>
          <div>{error}</div>
        </div>
      )}

      {allPrivate && (
        <div className="cf-banner cf-warn">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M12 3l9 16H3z" />
            <path d="M12 9v4M12 16v.5" />
          </svg>
          <div>
            <b>All session IPs look private.</b> Your reverse proxy is probably
            terminating the connection, leaving us with the LAN / Docker-bridge
            peer IP. Set the{" "}
            <span className="cf-mono">TRUSTED_PROXIES</span> env var to a
            comma-separated CIDR list of your proxies (e.g.{" "}
            <span className="cf-mono">172.16.0.0/12</span> for Docker, plus your
            Traefik / Cloudflare ranges) so we honour{" "}
            <span className="cf-mono">X-Forwarded-For</span> and record the real
            client IP.
          </div>
        </div>
      )}

      {sessions.length === 0 ? (
        <div className="cf-card" style={{ marginBottom: 0 }}>
          <div
            className="cf-card-body cf-pad cf-center cf-muted"
            style={{ fontSize: 13 }}
          >
            No active sessions.
          </div>
        </div>
      ) : (
        <div className="cf-card" style={{ marginBottom: 0, overflowX: "auto" }}>
          <table className="cf-table">
            <thead>
              <tr>
                <th>User</th>
                <th>Device</th>
                <th>IP</th>
                <th>Last seen</th>
                <th>Expires</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {sessions.map((s) => {
                const expiring =
                  nowMs > 0 && s.expires_at - nowMs < 7 * 86_400_000;
                return (
                  <tr key={s.id}>
                    <td style={{ whiteSpace: "nowrap", fontWeight: 600 }}>
                      @{s.username}
                    </td>
                    <td className="cf-muted">
                      <div
                        title={s.user_agent ?? ""}
                        style={{
                          maxWidth: 320,
                          display: "-webkit-box",
                          WebkitLineClamp: 2,
                          WebkitBoxOrient: "vertical",
                          overflow: "hidden",
                        }}
                      >
                        {summarizeUserAgent(s.user_agent)}
                      </div>
                    </td>
                    <td className="cf-mono">{s.ip ?? "—"}</td>
                    <td className="cf-muted" style={{ whiteSpace: "nowrap" }}>
                      {formatDateTime(s.last_seen_at)}
                    </td>
                    <td style={{ whiteSpace: "nowrap" }}>
                      {expiring ? (
                        <span className="cf-pill cf-warn">
                          {formatDateTime(s.expires_at)}
                        </span>
                      ) : (
                        <span className="cf-muted" style={{ fontSize: 12 }}>
                          {formatDateTime(s.expires_at)}
                        </span>
                      )}
                    </td>
                    <td className="cf-num">
                      <button
                        type="button"
                        disabled={busy}
                        onClick={() => setAskRevoke(s)}
                        className="cf-btn cf-ghost cf-tiny"
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

      {askRevoke && (
        <ConfirmDialog
          title={`Revoke session for @${askRevoke.username}?`}
          body={
            <>
              The session on{" "}
              <strong>{summarizeUserAgent(askRevoke.user_agent)}</strong> will
              be signed out immediately. The user will need to sign in again
              from that device.
            </>
          }
          confirmLabel="Revoke session"
          destructive
          busy={busy}
          onConfirm={async () => {
            const target = askRevoke;
            await revoke(target.id);
            setAskRevoke(null);
          }}
          onCancel={() => setAskRevoke(null)}
        />
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

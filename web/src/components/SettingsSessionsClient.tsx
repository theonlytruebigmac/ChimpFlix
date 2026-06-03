"use client";

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import {
  auth as authApi,
  ChimpFlixApiError,
  type MySessionEntry,
} from "@/lib/chimpflix-api";
import { ConfirmDialog } from "./ConfirmDialog";

export function SettingsSessionsClient() {
  const router = useRouter();
  const [sessions, setSessions] = useState<MySessionEntry[] | null>(null);
  const [busy, setBusy] = useState<number | "all" | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [askRevokeOne, setAskRevokeOne] = useState<{ id: number; label: string; current: boolean } | null>(null);
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

  function revokeOne(id: number, label: string, current: boolean) {
    setAskRevokeOne({ id, label, current });
  }

  async function confirmRevokeOne() {
    if (!askRevokeOne) return;
    const { id, label, current: isCurrent } = askRevokeOne;
    setAskRevokeOne(null);
    setBusy(id);
    setError(null);
    setMessage(null);
    try {
      await authApi.revokeMySession(id);
      if (isCurrent) {
        // Current session was just invalidated — redirect to login instead of
        // calling refresh() with a now-dead cookie (which would yield a 401).
        router.push("/login");
        return;
      }
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
    return <p className="cf-faint" style={{ fontSize: 13 }}>Loading sessions…</p>;
  }

  const others = sessions.filter((s) => !s.current).length;
  const countLabel =
    sessions.length === 1
      ? "One device currently signed in to your account."
      : `${sessions.length} devices currently signed in to your account.`;

  return (
    <div>
      {/* ── active sessions ──────────────────────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Active sessions</div>
            <div className="cf-sub">{countLabel}</div>
          </div>
          {others > 0 && (
            <div className="cf-head-aside">
              <button
                type="button"
                className="cf-btn cf-danger cf-sm"
                onClick={revokeOthers}
                disabled={busy === "all"}
              >
                {busy === "all" ? "Signing out…" : "Sign out everywhere else"}
              </button>
            </div>
          )}
        </div>

        {sessions.length === 0 ? (
          <div className="cf-card-body cf-pad">
            <span className="cf-faint" style={{ fontSize: 13 }}>
              No active sessions.
            </span>
          </div>
        ) : (
          <table className="cf-table">
            <thead>
              <tr>
                <th>Device</th>
                <th>IP address</th>
                <th>Last seen</th>
                <th>Expires</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {sessions.map((s) => {
                const label = summarizeUserAgent(s.user_agent);
                return (
                  <tr key={s.id}>
                    <td>
                      <div className="cf-flex cf-gap8">
                        <DeviceIcon ua={s.user_agent} />
                        {s.current && (
                          <span
                            className="cf-pill cf-accent"
                            style={{ padding: "2px 8px" }}
                          >
                            This device
                          </span>
                        )}
                        <span>{label}</span>
                      </div>
                    </td>
                    <td className="cf-mono">{s.ip ?? "unknown"}</td>
                    <td>{formatAge(s.last_seen_at)}</td>
                    <td className="cf-faint">{formatExpiresIn(s.expires_at)}</td>
                    <td className="cf-num">
                      <button
                        type="button"
                        className="cf-btn cf-ghost cf-tiny"
                        onClick={() =>
                          revokeOne(s.id, s.current ? "this device" : label, s.current)
                        }
                        disabled={busy === s.id}
                      >
                        {busy === s.id ? "…" : "Sign out"}
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>

      {/* ── how "sign out everywhere else" behaves ───────────────────── */}
      <div className="cf-banner cf-info" style={{ marginTop: 18 }}>
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <circle cx="12" cy="12" r="9" />
          <path d="M12 8v.5M12 11v5" />
        </svg>
        <div>
          Sessions stay valid for 30 days. Signing out elsewhere ends every
          other session but keeps <b>this device</b> signed in — you will not be
          logged out here.
        </div>
      </div>

      {error && (
        <div
          role="status"
          aria-live="polite"
          className="cf-banner cf-err"
          style={{ marginTop: 14 }}
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 8v4M12 16v.5" />
          </svg>
          <div>{error}</div>
        </div>
      )}
      {message && (
        <div
          role="status"
          aria-live="polite"
          className="cf-muted"
          style={{ marginTop: 14, fontSize: 13 }}
        >
          {message}
        </div>
      )}

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

/// Device-type glyph derived purely from the UA string (no backend).
/// Falls back to a generic laptop/monitor for desktop browsers.
function DeviceIcon({ ua }: { ua: string | null }) {
  const kind = deviceKind(ua);
  const common = {
    className: "cf-ico",
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: 2,
    strokeLinecap: "round" as const,
    strokeLinejoin: "round" as const,
    width: 16,
    height: 16,
    style: { flex: "none", opacity: 0.85 },
  };
  switch (kind) {
    case "phone":
      return (
        <svg {...common} aria-hidden>
          <rect x="6" y="3" width="12" height="18" rx="2" />
          <path d="M11 18h2" />
        </svg>
      );
    case "tablet":
      return (
        <svg {...common} aria-hidden>
          <rect x="4" y="3" width="16" height="18" rx="2" />
          <path d="M10 18h4" />
        </svg>
      );
    case "tv":
      return (
        <svg {...common} aria-hidden>
          <rect x="2" y="5" width="20" height="13" rx="2" />
          <path d="M8 21h8M12 18v3" />
        </svg>
      );
    default: // laptop / desktop
      return (
        <svg {...common} aria-hidden>
          <rect x="3" y="4" width="18" height="12" rx="2" />
          <path d="M8 20h8M12 16v4" />
        </svg>
      );
  }
}

type DeviceKind = "phone" | "tablet" | "tv" | "laptop";

function deviceKind(ua: string | null): DeviceKind {
  if (!ua) return "laptop";
  const u = ua.toLowerCase();
  if (
    u.includes("appletv") ||
    u.includes("tvos") ||
    u.includes("smart-tv") ||
    u.includes("smarttv") ||
    u.includes("googletv") ||
    u.includes("crkey") ||
    u.includes("roku") ||
    u.includes("web0s") ||
    u.includes("tizen") ||
    u.includes("netcast")
  ) {
    return "tv";
  }
  if (u.includes("ipad") || (u.includes("tablet") && !u.includes("mobile"))) {
    return "tablet";
  }
  if (u.includes("iphone") || u.includes("ipod") || u.includes("mobile") || u.includes("android")) {
    return "phone";
  }
  return "laptop";
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
  if (diffMin < 1) return "Just now";
  if (diffMin < 60) return `${diffMin}m ago`;
  if (diffMin < 60 * 24) return `${Math.floor(diffMin / 60)}h ago`;
  return `${Math.floor(diffMin / (60 * 24))}d ago`;
}

/// Relative future for the Expires column ("in 30 days"), matching the
/// mockup. Operates on the same absolute `expires_at` the data carries.
function formatExpiresIn(ms: number): string {
  const diffMin = Math.floor((ms - Date.now()) / 60_000);
  if (diffMin <= 0) return "expired";
  if (diffMin < 60) return `in ${diffMin}m`;
  if (diffMin < 60 * 24) {
    const h = Math.floor(diffMin / 60);
    return `in ${h}h`;
  }
  const d = Math.floor(diffMin / (60 * 24));
  return `in ${d} ${d === 1 ? "day" : "days"}`;
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

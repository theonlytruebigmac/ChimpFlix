"use client";

import { useEffect, useRef, useState } from "react";
import {
  auth as authApi,
  trakt as traktApi,
  type TraktLinkStart,
  type TraktStatus,
  type TraktSyncNowResult,
  type TraktUserStats,
} from "@/lib/chimpflix-api";
import { ConfirmDialog } from "./ConfirmDialog";
import { formatDateTime } from "@/lib/format";

/// Per-user integrations, rendered in the console design language: a
/// service-tile grid up top, then the connected-service detail (Trakt
/// today — device-code link flow, sync, unlink, and watch stats).
///
/// Discord is a real per-user integration: a personal webhook the user's
/// notifications are mirrored to. Tiles for services that aren't wired
/// per-user yet (Simkl) render as honest disabled "Coming soon" cards
/// rather than fake controls — tracked as feature-gap decisions.
export function SettingsIntegrationsClient() {
  const [status, setStatus] = useState<TraktStatus | null>(null);
  const [pending, setPending] = useState<TraktLinkStart | null>(null);
  const [busy, setBusy] = useState<
    null | "load" | "link" | "poll" | "unlink" | "sync"
  >(null);
  const [error, setError] = useState<string | null>(null);
  const [lastSync, setLastSync] = useState<TraktSyncNowResult | null>(null);
  const [stats, setStats] = useState<TraktUserStats | null>(null);
  const [askUnlink, setAskUnlink] = useState(false);
  // Discord per-user webhook. `discordUrl` is the saved value (null = not
  // configured); `discordEditing` reveals the inline input; `discordDraft`
  // holds the in-progress URL; `discordBusy` gates Save/Disconnect.
  const [discordUrl, setDiscordUrl] = useState<string | null>(null);
  const [discordEditing, setDiscordEditing] = useState(false);
  const [discordDraft, setDiscordDraft] = useState("");
  const [discordBusy, setDiscordBusy] = useState(false);
  const [discordError, setDiscordError] = useState<string | null>(null);
  const pollTimer = useRef<number | null>(null);
  // Current polling interval in ms. Stored in a ref so poll() can read and
  // update it without a stale closure (RFC 8628 §3.5 slow_down handling).
  const intervalMsRef = useRef<number>(5000);
  // Guards against concurrent overlapping poll() calls. `busy` state is
  // stale inside the setInterval closure (captured at startLink's render
  // cycle), so a ref is used for reliable in-flight deduplication.
  const pollInFlightRef = useRef(false);
  // True while this component is mounted. The poll() callback runs on
  // a Trakt-suggested interval (~5s) and lives across many awaits; if
  // the user navigates away mid-poll, the late-arriving response
  // would otherwise call setState on an unmounted component and trip
  // React warnings.
  const aliveRef = useRef(true);

  useEffect(() => {
    aliveRef.current = true;
    refresh();
    loadDiscord();
    return () => {
      aliveRef.current = false;
      if (pollTimer.current) window.clearInterval(pollTimer.current);
    };
  }, []);

  async function loadDiscord() {
    try {
      const { user } = await authApi.me();
      if (!aliveRef.current) return;
      setDiscordUrl(user.discord_webhook_url);
    } catch {
      // The Discord tile is a secondary surface — a failed /auth/me here
      // shouldn't blow up the whole integrations page (Trakt loads on its
      // own path). It just renders as "not connected" until a reload.
    }
  }

  async function saveDiscord() {
    const url = discordDraft.trim();
    if (!url) return;
    setDiscordBusy(true);
    setDiscordError(null);
    try {
      const { user } = await authApi.updateMe({ discord_webhook_url: url });
      if (!aliveRef.current) return;
      setDiscordUrl(user.discord_webhook_url);
      setDiscordEditing(false);
      setDiscordDraft("");
    } catch (e) {
      if (aliveRef.current) {
        setDiscordError(e instanceof Error ? e.message : String(e));
      }
    } finally {
      if (aliveRef.current) setDiscordBusy(false);
    }
  }

  async function disconnectDiscord() {
    setDiscordBusy(true);
    setDiscordError(null);
    try {
      // Empty string clears the webhook server-side (Some(None)).
      const { user } = await authApi.updateMe({ discord_webhook_url: "" });
      if (!aliveRef.current) return;
      setDiscordUrl(user.discord_webhook_url);
      setDiscordEditing(false);
      setDiscordDraft("");
    } catch (e) {
      if (aliveRef.current) {
        setDiscordError(e instanceof Error ? e.message : String(e));
      }
    } finally {
      if (aliveRef.current) setDiscordBusy(false);
    }
  }

  async function refresh() {
    setBusy("load");
    try {
      const s = await traktApi.status();
      setStatus(s);
      if (s.linked) {
        try {
          setStats(await traktApi.stats());
        } catch {
          // Stats are an optional surface — don't block the card on a
          // Trakt rate-limit or transient error.
        }
      } else {
        setStats(null);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function startLink() {
    setBusy("link");
    setError(null);
    try {
      const s = await traktApi.linkStart();
      setPending(s);
      const interval = Math.max(2, s.interval) * 1000;
      intervalMsRef.current = interval;
      pollTimer.current = window.setInterval(() => poll(), interval);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function poll() {
    // Use a ref rather than `busy` state: the setInterval callback captures
    // a stale closure where `busy` is always null, so the state guard never
    // fires. The ref is set synchronously before the first await.
    if (pollInFlightRef.current) return;
    pollInFlightRef.current = true;
    setBusy("poll");
    try {
      const result = await traktApi.linkPoll();
      if (!aliveRef.current) return;
      switch (result.status) {
        case "ready":
          stopPolling();
          setPending(null);
          await refresh();
          break;
        case "expired":
        case "denied":
          stopPolling();
          setPending(null);
          setError(
            result.status === "expired"
              ? "The link code expired before you approved it."
              : "You denied the link request on Trakt.",
          );
          break;
        case "pending":
          break;
        case "slow_down":
          // RFC 8628 §3.5: increase the polling interval by 5 s and restart
          // the timer at the new rate to avoid continued rate-limiting.
          stopPolling();
          intervalMsRef.current += 5000;
          pollTimer.current = window.setInterval(
            () => poll(),
            intervalMsRef.current,
          );
          break;
      }
    } catch (e) {
      if (!aliveRef.current) return;
      stopPolling();
      setPending(null);
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      pollInFlightRef.current = false;
      if (aliveRef.current) setBusy(null);
    }
  }

  function stopPolling() {
    if (pollTimer.current) {
      window.clearInterval(pollTimer.current);
      pollTimer.current = null;
    }
  }

  async function unlinkConfirmed() {
    setBusy("unlink");
    try {
      await traktApi.unlink();
      await refresh();
      setAskUnlink(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function syncNow() {
    setBusy("sync");
    setLastSync(null);
    try {
      setLastSync(await traktApi.syncNow());
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  const linked = !!status?.linked;
  const configured = !!status?.app_configured;
  const discordConnected = !!discordUrl;

  return (
    <div>
      {/* ── service grid ──────────────────────────────────────────── */}
      <div className="cf-grid cf-c3" style={{ marginBottom: 8 }}>
        {/* Trakt */}
        <div className={`cf-itile${linked ? " cf-linked" : ""}`}>
          <div className="cf-it-top">
            <div
              className="cf-it-logo"
              style={{ background: "#ed1c24", color: "#fff" }}
            >
              T
            </div>
            <div>
              <div className="cf-it-name">Trakt.tv</div>
              {linked ? (
                <span className="cf-pill cf-ok" style={{ marginTop: 5 }}>
                  <span className="cf-dot" />
                  Connected
                </span>
              ) : configured ? (
                <span className="cf-pill" style={{ marginTop: 5 }}>
                  Not connected
                </span>
              ) : (
                <span className="cf-pill cf-warn" style={{ marginTop: 5 }}>
                  Not configured
                </span>
              )}
            </div>
          </div>
          <div className="cf-it-desc">
            Two-way scrobbling, watch history, resume points, and ratings sync.
          </div>
          <div className="cf-it-foot">
            {linked ? (
              <>
                <a className="cf-btn cf-sm" href="#trakt">
                  Manage
                </a>
                {status?.last_synced_at && (
                  <span
                    className="cf-faint"
                    style={{ fontSize: 12, marginLeft: "auto" }}
                  >
                    Synced {formatDateTime(status.last_synced_at)}
                  </span>
                )}
              </>
            ) : configured ? (
              <button
                className="cf-btn cf-sm cf-primary"
                onClick={startLink}
                disabled={busy !== null || !!pending}
              >
                {busy === "link" ? "Starting…" : "Connect"}
              </button>
            ) : (
              <span className="cf-faint" style={{ fontSize: 12 }}>
                Server setup needed
              </span>
            )}
          </div>
        </div>

        {/* TMDB · TVDB — server-managed metadata providers */}
        <div className="cf-itile">
          <div className="cf-it-top">
            <div
              className="cf-it-logo"
              style={{ background: "#0d253f", color: "#01d277" }}
            >
              M
            </div>
            <div>
              <div className="cf-it-name">TMDB · TVDB</div>
              <span className="cf-pill cf-info" style={{ marginTop: 5 }}>
                Server-managed
              </span>
            </div>
          </div>
          <div className="cf-it-desc">
            Metadata providers. Configured once by the admin under Server →
            Credentials.
          </div>
          <div className="cf-it-foot">
            <span className="cf-faint" style={{ fontSize: 12 }}>
              No action needed
            </span>
          </div>
        </div>

        {/* Discord — per-user notification webhook */}
        <div className={`cf-itile${discordConnected ? " cf-linked" : ""}`}>
          <div className="cf-it-top">
            <div
              className="cf-it-logo"
              style={{ background: "#5865f2", color: "#fff" }}
            >
              D
            </div>
            <div>
              <div className="cf-it-name">Discord</div>
              {discordConnected ? (
                <span className="cf-pill cf-ok" style={{ marginTop: 5 }}>
                  <span className="cf-dot" />
                  Connected
                </span>
              ) : (
                <span className="cf-pill" style={{ marginTop: 5 }}>
                  Not connected
                </span>
              )}
            </div>
          </div>
          <div className="cf-it-desc">
            Pipe your notifications to a Discord webhook of your choice. They
            follow your per-kind notification prefs and quiet hours.
          </div>

          {discordEditing && !discordConnected && (
            <div className="cf-pad" style={{ paddingTop: 0 }}>
              <input
                className="cf-input"
                type="url"
                inputMode="url"
                autoFocus
                placeholder="https://discord.com/api/webhooks/…"
                value={discordDraft}
                onChange={(e) => setDiscordDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void saveDiscord();
                  if (e.key === "Escape") {
                    setDiscordEditing(false);
                    setDiscordDraft("");
                    setDiscordError(null);
                  }
                }}
                disabled={discordBusy}
              />
              {discordError && (
                <div
                  className="cf-faint"
                  style={{ marginTop: 6, fontSize: 12, color: "var(--err)" }}
                >
                  {discordError}
                </div>
              )}
            </div>
          )}

          {discordConnected && (
            <div className="cf-pad" style={{ paddingTop: 0 }}>
              <span className="cf-mono cf-faint" style={{ fontSize: 12 }}>
                {maskDiscordUrl(discordUrl!)}
              </span>
            </div>
          )}

          <div className="cf-it-foot">
            {discordConnected ? (
              <button
                className="cf-btn cf-danger cf-sm"
                onClick={() => void disconnectDiscord()}
                disabled={discordBusy}
              >
                {discordBusy ? "Disconnecting…" : "Disconnect"}
              </button>
            ) : discordEditing ? (
              <>
                <button
                  className="cf-btn cf-sm cf-primary"
                  onClick={() => void saveDiscord()}
                  disabled={discordBusy || !discordDraft.trim()}
                >
                  {discordBusy ? "Saving…" : "Save"}
                </button>
                <button
                  className="cf-btn cf-sm"
                  onClick={() => {
                    setDiscordEditing(false);
                    setDiscordDraft("");
                    setDiscordError(null);
                  }}
                  disabled={discordBusy}
                >
                  Cancel
                </button>
              </>
            ) : (
              <button
                className="cf-btn cf-sm"
                onClick={() => setDiscordEditing(true)}
              >
                Connect
              </button>
            )}
          </div>
        </div>

        {/* Simkl — alternative scrobble target (feature-gap: not wired yet) */}
        <div className="cf-itile" style={{ opacity: 0.6 }}>
          <div className="cf-it-top">
            <div
              className="cf-it-logo"
              style={{ background: "#202830", color: "#00d735" }}
            >
              S
            </div>
            <div>
              <div className="cf-it-name">Simkl</div>
              <span className="cf-pill" style={{ marginTop: 5 }}>
                Coming soon
              </span>
            </div>
          </div>
          <div className="cf-it-desc">
            An alternative scrobble target alongside Trakt.
          </div>
          <div className="cf-it-foot">
            <button className="cf-btn cf-sm" disabled>
              Connect
            </button>
          </div>
        </div>
      </div>

      {/* ── not-configured guidance (owner action) ───────────────────── */}
      {status && !configured && (
        <div className="cf-banner cf-info">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 8v.5M12 11v5" />
          </svg>
          <div>
            The server owner needs to register a Trakt OAuth app at{" "}
            <a
              href="https://trakt.tv/oauth/applications"
              target="_blank"
              rel="noreferrer"
            >
              trakt.tv/oauth/applications
            </a>{" "}
            (redirect URI <code>urn:ietf:wg:oauth:2.0:oob</code>) and paste the
            JSON into the Trakt credential slot.
          </div>
        </div>
      )}

      {/* ── pending device-code link ─────────────────────────────────── */}
      {pending && (
        <div className="cf-card">
          <div className="cf-card-body cf-pad">
            <div className="cf-muted" style={{ fontSize: 13 }}>
              Open Trakt in another tab and enter this code:
            </div>
            <div
              className="cf-mono"
              style={{
                marginTop: 8,
                fontSize: 28,
                letterSpacing: "0.4em",
                color: "#fff",
              }}
            >
              {pending.user_code}
            </div>
            <a
              href={pending.verification_url}
              target="_blank"
              rel="noreferrer"
              className="cf-pill cf-accent"
              style={{ marginTop: 10, display: "inline-flex" }}
            >
              {pending.verification_url}
            </a>
            <div className="cf-faint" style={{ marginTop: 10, fontSize: 12 }}>
              Waiting for approval… (this card updates automatically)
            </div>
          </div>
        </div>
      )}

      {/* ── connected detail ─────────────────────────────────────────── */}
      {linked && (
        <>
          <div className="cf-section-title" id="trakt">
            Trakt.tv — connected
          </div>

          {status?.expired ? (
            <div className="cf-banner cf-err">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M12 3l9 16H3z" />
                <path d="M12 10v4M12 17v.5" />
              </svg>
              <div>
                Your Trakt access token has <b>expired</b>. The next sync will try
                to refresh it; if that fails (the refresh token also expires after
                ~60 days of no use), unlink and re-link.
              </div>
            </div>
          ) : (
            status?.expiring_soon && (
              <div className="cf-banner cf-warn">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                  <path d="M12 3l9 16H3z" />
                  <path d="M12 10v4M12 17v.5" />
                </svg>
                <div>
                  Your access token <b>expires soon</b>. Running a sync refreshes it
                  automatically — or it&rsquo;ll quietly stop working.
                </div>
              </div>
            )
          )}

          <div className="cf-card">
            <div className="cf-card-head">
              <div className="cf-flex cf-gap12">
                <div
                  className="cf-it-logo"
                  style={{ background: "#ed1c24", color: "#fff" }}
                >
                  T
                </div>
                <div>
                  <div className="cf-ttl">Trakt account</div>
                  <div className="cf-sub">
                    Every play scrobbles to your history; an hourly job pulls
                    history &amp; resume points back. Ratings sync both directions.
                  </div>
                </div>
              </div>
              <div className="cf-head-aside">
                <button
                  className="cf-btn cf-sm"
                  onClick={syncNow}
                  disabled={busy !== null}
                >
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                    <path d="M4 10a8 8 0 0 1 14-4l2 2M20 14a8 8 0 0 1-14 4l-2-2" />
                    <path d="M18 4v4h-4M6 20v-4h4" />
                  </svg>
                  {busy === "sync" ? "Syncing…" : "Sync now"}
                </button>
                <button
                  className="cf-btn cf-danger cf-sm"
                  onClick={() => setAskUnlink(true)}
                  disabled={busy !== null}
                >
                  {busy === "unlink" ? "Unlinking…" : "Unlink"}
                </button>
              </div>
            </div>
            <div className="cf-card-body">
              <div className="cf-row">
                <div className="cf-row-main">
                  <div className="cf-row-label">Linked since</div>
                </div>
                <div className="cf-row-control cf-faint">
                  {status?.linked_at ? formatDateTime(status.linked_at) : "—"}
                </div>
              </div>
              <div className="cf-row">
                <div className="cf-row-main">
                  <div className="cf-row-label">Last sync</div>
                </div>
                <div className="cf-row-control">
                  {status?.last_synced_at ? (
                    <span className="cf-pill cf-ok">
                      <span className="cf-dot" />
                      {formatDateTime(status.last_synced_at)}
                    </span>
                  ) : (
                    <span className="cf-faint">Never</span>
                  )}
                </div>
              </div>
            </div>
          </div>

          {lastSync && (
            <div className="cf-banner cf-ok">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M20 6L9 17l-5-5" />
              </svg>
              <div>
                {lastSync.queued
                  ? "Sync started — running in the background. Watched status and watchlist update as it completes."
                  : "A sync is already running in the background. Watched status and watchlist update as it completes."}
              </div>
            </div>
          )}

          {stats && (
            <div className="cf-grid cf-c4">
              <StatTile
                tone="cf-tone-red"
                label="Movies"
                value={stats.movies.watched.toLocaleString()}
                meta={`${formatMinutes(stats.movies.minutes)} watched`}
                icon={<rect x="3" y="4" width="18" height="16" rx="2" />}
              />
              <StatTile
                tone="cf-tone-blue"
                label="Shows"
                value={stats.shows.watched.toLocaleString()}
                meta={`${stats.episodes.watched.toLocaleString()} episodes`}
                icon={
                  <>
                    <rect x="3" y="4" width="18" height="14" rx="2" />
                    <path d="M8 20h8" />
                  </>
                }
              />
              <StatTile
                tone="cf-tone-violet"
                label="Episodes"
                value={stats.episodes.plays.toLocaleString()}
                meta={`${formatMinutes(stats.episodes.minutes)} watched`}
                icon={<path d="M3 12h4l3 8 4-16 3 8h4" />}
              />
              <StatTile
                tone="cf-tone-amber"
                label="Ratings"
                value={stats.ratings.total.toLocaleString()}
                meta="given"
                icon={
                  <path d="M12 4l2.5 5 5.5.7-4 3.9 1 5.4-5-2.8-5 2.8 1-5.4-4-3.9 5.5-.7z" />
                }
              />
            </div>
          )}
        </>
      )}

      {error && (
        <div
          role="status"
          aria-live="polite"
          className="cf-banner cf-err"
          style={{ marginTop: 16 }}
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 8v4M12 16v.5" />
          </svg>
          <div>{error}</div>
        </div>
      )}

      {askUnlink && (
        <ConfirmDialog
          title="Unlink Trakt?"
          body="Your scrobbles and watched markers stop syncing to Trakt. Local play state is unaffected — re-link any time to resume."
          confirmLabel="Unlink"
          destructive
          busy={busy === "unlink"}
          onConfirm={() => void unlinkConfirmed()}
          onCancel={() => setAskUnlink(false)}
        />
      )}
    </div>
  );
}

/// One `.cf-stat` tile. `icon` is the inner SVG path(s); the wrapping
/// <svg> + tone styling come from the console design system.
function StatTile({
  tone,
  label,
  value,
  meta,
  icon,
}: {
  tone: string;
  label: string;
  value: string;
  meta?: string;
  icon: React.ReactNode;
}) {
  return (
    <div className={`cf-stat ${tone}`}>
      <div className="cf-stat-top">
        <span className="cf-stat-ico">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            {icon}
          </svg>
        </span>
        {label}
      </div>
      <div className="cf-stat-val">{value}</div>
      {meta && <div className="cf-stat-meta">{meta}</div>}
    </div>
  );
}

/// Mask a Discord webhook URL for display — show the host + the truncated
/// webhook id, hide the secret token entirely. A Discord webhook looks like
/// `https://discord.com/api/webhooks/<id>/<token>`; we never re-show the
/// token (the server doesn't hand it back either, but the saved value lives
/// in this component briefly after a Save).
function maskDiscordUrl(url: string): string {
  const marker = "/api/webhooks/";
  const idx = url.indexOf(marker);
  if (idx === -1) return "Connected";
  const rest = url.slice(idx + marker.length);
  const id = rest.split("/")[0] ?? "";
  const shortId = id.length > 8 ? `${id.slice(0, 8)}…` : id;
  return `…/api/webhooks/${shortId}/••••••`;
}

function formatMinutes(minutes: number): string {
  if (minutes <= 0) return "0 min";
  const days = Math.floor(minutes / (60 * 24));
  const hours = Math.floor((minutes % (60 * 24)) / 60);
  if (days > 0) {
    return `${days}d ${hours}h`;
  }
  if (hours > 0) {
    return `${hours}h ${minutes % 60}m`;
  }
  return `${minutes} min`;
}

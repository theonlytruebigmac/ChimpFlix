"use client";

import { useEffect, useRef, useState } from "react";
import {
  trakt as traktApi,
  type TraktLinkStart,
  type TraktStatus,
  type TraktSyncNowResult,
} from "@/lib/chimpflix-api";

/// Per-user integrations card. Right now the only integration here is
/// Trakt — when more land (Trakt+anything-else), wrap each in its own
/// subsection.
export function SettingsIntegrationsClient() {
  const [status, setStatus] = useState<TraktStatus | null>(null);
  const [pending, setPending] = useState<TraktLinkStart | null>(null);
  const [busy, setBusy] = useState<
    null | "load" | "link" | "poll" | "unlink" | "sync"
  >(null);
  const [error, setError] = useState<string | null>(null);
  const [lastSync, setLastSync] = useState<TraktSyncNowResult | null>(null);
  const pollTimer = useRef<number | null>(null);

  useEffect(() => {
    refresh();
    return () => {
      if (pollTimer.current) window.clearInterval(pollTimer.current);
    };
  }, []);

  async function refresh() {
    setBusy("load");
    try {
      setStatus(await traktApi.status());
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
      // Begin polling at the server-suggested interval (Trakt asks for
      // ~5s) until the user approves or the code expires.
      const interval = Math.max(2, s.interval) * 1000;
      pollTimer.current = window.setInterval(() => poll(), interval);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function poll() {
    if (busy === "poll") return;
    setBusy("poll");
    try {
      const result = await traktApi.linkPoll();
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
        case "slow_down":
          // Keep polling; nothing to do here.
          break;
      }
    } catch (e) {
      stopPolling();
      setPending(null);
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  function stopPolling() {
    if (pollTimer.current) {
      window.clearInterval(pollTimer.current);
      pollTimer.current = null;
    }
  }

  async function unlink() {
    if (!confirm("Unlink your Trakt account?")) return;
    setBusy("unlink");
    try {
      await traktApi.unlink();
      await refresh();
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

  return (
    <div className="space-y-6">
      <div className="border-y border-white/5 py-4 text-sm">
        <div className="flex items-baseline justify-between gap-3">
          <div className="flex items-baseline gap-3">
            <span className="text-white">Trakt.tv</span>
            {status?.app_configured ? (
              <span className="rounded bg-emerald-500/15 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-emerald-300">
                Configured
              </span>
            ) : (
              <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-amber-300">
                Not configured
              </span>
            )}
          </div>
        </div>
        <p className="mt-1 text-xs text-white/55">
          Two-way sync: every play scrobble and mark-watched lands in
          your Trakt history; an hourly job pulls history + resume
          points back. Ratings sync both directions.
        </p>

        {!status?.app_configured && (
          <p className="mt-2 text-xs text-amber-300">
            The server owner needs to register a Trakt OAuth app at{" "}
            <a
              href="https://trakt.tv/oauth/applications"
              target="_blank"
              rel="noreferrer"
              className="underline"
            >
              trakt.tv/oauth/applications
            </a>{" "}
            (redirect URI <code>urn:ietf:wg:oauth:2.0:oob</code>) and
            paste the JSON into the Trakt credential slot.
          </p>
        )}

        {status && status.app_configured && !status.linked && !pending && (
          <button
            onClick={startLink}
            disabled={busy !== null}
            className="mt-3 rounded bg-(--color-accent) px-3 py-1.5 text-xs font-semibold text-white transition disabled:opacity-50"
          >
            {busy === "link" ? "Starting…" : "Link Trakt account"}
          </button>
        )}

        {pending && (
          <div className="mt-3 rounded border border-(--color-accent)/40 bg-(--color-accent)/10 p-3">
            <div className="text-xs text-white/65">
              Open Trakt in another tab and enter this code:
            </div>
            <div className="mt-2 font-mono text-2xl tracking-[0.5em] text-white">
              {pending.user_code}
            </div>
            <a
              href={pending.verification_url}
              target="_blank"
              rel="noreferrer"
              className="mt-2 inline-block text-xs text-(--color-accent) underline"
            >
              {pending.verification_url}
            </a>
            <div className="mt-2 text-xs text-white/45">
              Waiting for approval… (this card updates automatically)
            </div>
          </div>
        )}

        {status?.linked && (
          <div className="mt-3 space-y-2">
            <div className="text-xs text-white/60">
              Linked since{" "}
              {status.linked_at
                ? new Date(status.linked_at).toLocaleString()
                : "—"}
              {status.last_synced_at && (
                <>
                  {" · "}last sync{" "}
                  {new Date(status.last_synced_at).toLocaleString()}
                </>
              )}
            </div>
            <div className="flex gap-2">
              <button
                onClick={syncNow}
                disabled={busy !== null}
                className="rounded bg-white/10 px-3 py-1.5 text-xs font-medium hover:bg-white/15 disabled:opacity-50"
              >
                {busy === "sync" ? "Syncing…" : "Sync now"}
              </button>
              <button
                onClick={unlink}
                disabled={busy !== null}
                className="rounded border border-red-500/40 bg-red-500/10 px-3 py-1.5 text-xs font-medium text-red-200 hover:bg-red-500/15 disabled:opacity-50"
              >
                {busy === "unlink" ? "Unlinking…" : "Unlink"}
              </button>
            </div>
            {lastSync && (
              <div className="text-xs text-emerald-300">
                Pulled {lastSync.movies_marked} movies,{" "}
                {lastSync.episodes_marked} episodes,{" "}
                {lastSync.playback_applied} resume points.
              </div>
            )}
          </div>
        )}

        {error && (
          <div className="mt-3 text-xs text-red-300">{error}</div>
        )}
      </div>
    </div>
  );
}

"use client";

import { useEffect, useState } from "react";
import { plex, type PlexLinkSummary } from "@/lib/chimpflix-api";
import { ConfirmDialog } from "./ConfirmDialog";
import { PlexSignInButton } from "./PlexSignInButton";
import { formatDate } from "@/lib/format";

/// Per-user "Linked accounts" surface. Today this is Plex-only; when we
/// add Google OAuth the same surface will list both providers. Rendered
/// as a console setting-row to match the redesign mockup.
export function SettingsLinkedAccountsClient() {
  const [links, setLinks] = useState<PlexLinkSummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [askUnlink, setAskUnlink] = useState(false);

  async function load() {
    try {
      setLinks(await plex.listLinks());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }
  useEffect(() => {
    // Fire-and-forget initial fetch. The eslint rule wants us to use
    // useEffectEvent (still experimental in this React version); the
    // load() call schedules an async setState after the await, which
    // is the intended hydration path. Same pattern is in use by
    // SettingsIntegrationsClient with no surface complaint — the
    // disable here is explicit because the typecheck sometimes flags
    // it depending on flow-sensitive analysis order.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    load();
  }, []);

  const plexLink = links?.find((l) => l.provider === "plex");

  async function unlinkConfirmed() {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      await plex.unlink();
      setNotice("Plex account unlinked.");
      await load();
      setAskUnlink(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div>
      <div className="cf-row" style={{ paddingTop: 0 }}>
        <div className="cf-row-main">
          <div className="cf-row-label">
            <span style={{ color: "#e5a00d", fontWeight: 800 }}>Plex</span>.tv
            {plexLink && (
              <span className="cf-pill cf-ok" style={{ marginLeft: 6 }}>
                <span className="cf-dot" />
                Connected
              </span>
            )}
          </div>
          {plexLink ? (
            <div className="cf-row-help">
              Linked {formatDate(plexLink.linked_at)} as{" "}
              <b>{plexLink.external_username ?? "your Plex account"}</b>
              {plexLink.external_email ? ` · ${plexLink.external_email}` : ""}
            </div>
          ) : (
            <div className="cf-row-help">
              Sign in with your Plex account instead of a password.
            </div>
          )}
        </div>
        <div className="cf-row-control">
          {plexLink ? (
            <button
              type="button"
              onClick={() => setAskUnlink(true)}
              disabled={busy}
              className="cf-btn cf-danger cf-sm"
            >
              {busy ? "Unlinking…" : "Unlink"}
            </button>
          ) : (
            <PlexSignInButton
              intent={{ intent: "link" }}
              onLinked={async () => {
                setNotice("Plex account linked.");
                await load();
              }}
              onError={(msg) => setError(msg)}
            />
          )}
        </div>
      </div>

      {notice && (
        <div className="cf-banner cf-ok" style={{ marginTop: 14, marginBottom: 0 }}>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M20 6L9 17l-5-5" />
          </svg>
          <div>{notice}</div>
        </div>
      )}
      {error && (
        <div className="cf-banner cf-err" style={{ marginTop: 14, marginBottom: 0 }}>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 8v4M12 16v.5" />
          </svg>
          <div>{error}</div>
        </div>
      )}
      {askUnlink && (
        <ConfirmDialog
          title="Unlink Plex?"
          body="You can sign in again later with your password. If you haven't set one, use 'Forgot password' first to recover access."
          confirmLabel="Unlink"
          destructive
          busy={busy}
          onConfirm={() => void unlinkConfirmed()}
          onCancel={() => setAskUnlink(false)}
        />
      )}
    </div>
  );
}

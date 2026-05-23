"use client";

import { useEffect, useState } from "react";
import { plex, type PlexLinkSummary } from "@/lib/chimpflix-api";
import { ConfirmDialog } from "./ConfirmDialog";
import { PlexSignInButton } from "./PlexSignInButton";

/// Per-user "Linked accounts" card. Today this is Plex-only; when we
/// add Google OAuth the same surface will list both providers.
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
    <div className="space-y-3 text-sm">
      <div>
        <div className="flex items-baseline justify-between gap-3">
          <div>
            <div className="text-white">Plex.tv</div>
            {plexLink ? (
              <div className="mt-0.5 text-xs text-white/55">
                Linked to{" "}
                <span className="text-white">
                  {plexLink.external_username ?? "your Plex account"}
                </span>
                {plexLink.external_email && (
                  <span className="text-white/40">
                    {" "}
                    ({plexLink.external_email})
                  </span>
                )}
                {" · "}since{" "}
                {new Date(plexLink.linked_at).toLocaleDateString()}
              </div>
            ) : (
              <div className="mt-0.5 text-xs text-white/55">
                Sign in with your Plex account instead of a password.
              </div>
            )}
          </div>
          {plexLink && (
            <button
              type="button"
              onClick={() => setAskUnlink(true)}
              disabled={busy}
              className="rounded border border-red-500/40 bg-red-500/10 px-3 py-1.5 text-xs font-medium text-red-200 hover:bg-red-500/15 disabled:opacity-50"
            >
              {busy ? "Unlinking…" : "Unlink"}
            </button>
          )}
        </div>
        {!plexLink && (
          <div className="mt-3">
            <PlexSignInButton
              intent={{ intent: "link" }}
              onLinked={async () => {
                setNotice("Plex account linked.");
                await load();
              }}
              onError={(msg) => setError(msg)}
            />
          </div>
        )}
      </div>

      {notice && (
        <div
          role="status"
          aria-live="polite"
          className="rounded border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-xs text-emerald-200"
        >
          {notice}
        </div>
      )}
      {error && (
        <div
          role="status"
          aria-live="polite"
          className="rounded border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300"
        >
          {error}
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

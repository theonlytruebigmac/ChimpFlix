"use client";

import { useEffect, useState } from "react";
import {
  auth as authApi,
  ChimpFlixApiError,
  type Invite,
} from "@/lib/chimpflix-api";

type ExpiryChoice = "1d" | "7d" | "30d" | "never";

const EXPIRY_LABELS: Record<ExpiryChoice, string> = {
  "1d": "1 day",
  "7d": "7 days",
  "30d": "30 days",
  never: "Never",
};

const EXPIRY_SECONDS: Record<ExpiryChoice, number | undefined> = {
  "1d": 86_400,
  "7d": 604_800,
  "30d": 2_592_000,
  never: undefined,
};

export function SettingsInvitesClient() {
  const [invites, setInvites] = useState<Invite[] | null>(null);
  const [expiry, setExpiry] = useState<ExpiryChoice>("7d");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [origin, setOrigin] = useState("");

  useEffect(() => {
    setOrigin(window.location.origin);
    refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function refresh() {
    try {
      const { invites } = await authApi.listInvites();
      setInvites(invites);
    } catch (e) {
      if (e instanceof ChimpFlixApiError && e.status === 403) {
        setMessage("Only the owner can manage invites.");
      } else {
        setMessage("Failed to load invites.");
      }
      setInvites([]);
    }
  }

  async function create() {
    setBusy(true);
    setMessage(null);
    try {
      await authApi.createInvite({
        expires_in_seconds: EXPIRY_SECONDS[expiry],
      });
      await refresh();
    } catch {
      setMessage("Failed to create invite.");
    } finally {
      setBusy(false);
    }
  }

  async function revoke(code: string) {
    if (!window.confirm("Revoke this invite code?")) return;
    try {
      await authApi.revokeInvite(code);
      await refresh();
    } catch {
      setMessage("Failed to revoke.");
    }
  }

  function copy(code: string) {
    const link = `${origin}/login?invite=${encodeURIComponent(code)}`;
    void navigator.clipboard.writeText(link).catch(() => {});
    setMessage("Invite link copied to clipboard.");
    window.setTimeout(() => setMessage(null), 2500);
  }

  if (invites === null) {
    return <p className="text-sm text-white/60">Loading…</p>;
  }

  const open = invites.filter((i) => !i.consumed_at);
  const used = invites.filter((i) => i.consumed_at);

  return (
    <div>
      <div className="mb-4 flex flex-wrap items-end gap-2">
        <label className="text-xs">
          <span className="mb-1 block text-white/60">Expires</span>
          <select
            value={expiry}
            onChange={(e) => setExpiry(e.target.value as ExpiryChoice)}
            className="rounded bg-white/10 px-3 py-2 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
          >
            {(Object.keys(EXPIRY_LABELS) as ExpiryChoice[]).map((k) => (
              <option key={k} value={k}>
                {EXPIRY_LABELS[k]}
              </option>
            ))}
          </select>
        </label>
        <button
          type="button"
          onClick={create}
          disabled={busy}
          className="rounded bg-(--color-accent) px-3 py-2 text-xs font-semibold text-white transition disabled:opacity-50"
        >
          {busy ? "Creating…" : "+ Create invite"}
        </button>
      </div>

      {open.length === 0 && used.length === 0 && (
        <p className="text-sm text-white/60">
          No invites yet. Create one above and share the link with a friend.
        </p>
      )}

      {open.length > 0 && (
        <div className="mb-4">
          <h3 className="mb-2 text-xs uppercase tracking-wide text-white/50">
            Active
          </h3>
          <ul className="divide-y divide-white/5">
            {open.map((inv) => (
              <li
                key={inv.code}
                className="flex items-center justify-between gap-3 py-2 text-sm"
              >
                <div className="min-w-0 flex-1">
                  <div className="truncate font-mono text-xs text-white/85">
                    {inv.code}
                  </div>
                  <div className="text-xs text-white/50">
                    {inv.expires_at
                      ? `Expires ${new Date(inv.expires_at).toLocaleString()}`
                      : "Never expires"}
                  </div>
                </div>
                <div className="flex shrink-0 gap-2">
                  <button
                    type="button"
                    onClick={() => copy(inv.code)}
                    className="rounded bg-white/10 px-3 py-1.5 text-xs font-medium hover:bg-white/15"
                  >
                    Copy link
                  </button>
                  <button
                    type="button"
                    onClick={() => revoke(inv.code)}
                    className="rounded bg-red-500/15 px-3 py-1.5 text-xs font-medium text-red-200 hover:bg-red-500/25"
                  >
                    Revoke
                  </button>
                </div>
              </li>
            ))}
          </ul>
        </div>
      )}

      {used.length > 0 && (
        <div>
          <h3 className="mb-2 text-xs uppercase tracking-wide text-white/50">
            Used
          </h3>
          <ul className="divide-y divide-white/5">
            {used.map((inv) => (
              <li
                key={inv.code}
                className="flex items-center justify-between gap-3 py-2 text-sm"
              >
                <div className="min-w-0 flex-1">
                  <div className="truncate font-mono text-xs text-white/55">
                    {inv.code}
                  </div>
                  <div className="text-xs text-white/45">
                    Used{" "}
                    {inv.consumed_at
                      ? new Date(inv.consumed_at).toLocaleString()
                      : ""}
                  </div>
                </div>
              </li>
            ))}
          </ul>
        </div>
      )}

      {message && (
        <p className="mt-3 text-xs text-white/70">{message}</p>
      )}
    </div>
  );
}

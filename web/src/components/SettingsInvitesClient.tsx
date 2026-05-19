"use client";

import { useEffect, useState } from "react";
import {
  admin as adminApi,
  auth as authApi,
  libraries as librariesApi,
  ChimpFlixApiError,
  type AccessGroup,
  type CreatedInvite,
  type InviteListEntry,
  type Library,
} from "@/lib/chimpflix-api";

type ExpiryChoice = "1d" | "3d" | "7d" | "30d" | "never";

const EXPIRY_LABELS: Record<ExpiryChoice, string> = {
  "1d": "1 day",
  "3d": "3 days",
  "7d": "7 days",
  "30d": "30 days",
  never: "Never",
};

const EXPIRY_SECONDS: Record<ExpiryChoice, number | undefined> = {
  "1d": 86_400,
  "3d": 259_200,
  "7d": 604_800,
  "30d": 2_592_000,
  never: undefined,
};

export function SettingsInvitesClient() {
  const [invites, setInvites] = useState<InviteListEntry[] | null>(null);
  const [libraries, setLibraries] = useState<Library[]>([]);
  const [groups, setGroups] = useState<AccessGroup[]>([]);
  const [expiry, setExpiry] = useState<ExpiryChoice>("3d");
  const [email, setEmail] = useState("");
  const [selectedLibIds, setSelectedLibIds] = useState<Set<number>>(new Set());
  const [selectedGroupIds, setSelectedGroupIds] = useState<Set<number>>(new Set());
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [issued, setIssued] = useState<CreatedInvite | null>(null);

  useEffect(() => {
    refresh();
    librariesApi
      .list()
      .then(({ libraries }) => setLibraries(libraries))
      .catch(() => {});
    adminApi.accessGroups
      .list()
      .then(({ groups }) => setGroups(groups))
      .catch(() => {});
  }, []);

  async function refresh() {
    try {
      const { invites } = await authApi.listInvites();
      setInvites(invites);
    } catch (e) {
      if (e instanceof ChimpFlixApiError && e.status === 403) {
        setError("Only the owner can manage invites.");
      } else {
        setError("Failed to load invites.");
      }
      setInvites([]);
    }
  }

  async function create() {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      const trimmed = email.trim();
      if (trimmed && (!trimmed.includes("@") || trimmed.length > 320)) {
        throw new Error("Email must look like local@domain");
      }
      const created = await authApi.createInvite({
        expires_in_seconds: EXPIRY_SECONDS[expiry],
        email: trimmed || undefined,
        library_ids: Array.from(selectedLibIds),
        group_ids: Array.from(selectedGroupIds),
      });
      setIssued(created);
      setEmail("");
      setSelectedLibIds(new Set());
      setSelectedGroupIds(new Set());
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to create invite.");
    } finally {
      setBusy(false);
    }
  }

  async function revoke(id: number) {
    if (!window.confirm("Revoke this invite?")) return;
    try {
      await authApi.revokeInvite(id);
      await refresh();
    } catch {
      setError("Failed to revoke.");
    }
  }

  function copyAcceptUrl() {
    if (!issued?.accept_url) return;
    void navigator.clipboard.writeText(issued.accept_url).catch(() => {});
    setMessage("Accept link copied to clipboard.");
    window.setTimeout(() => setMessage(null), 2500);
  }
  function copyCode() {
    if (!issued?.code) return;
    void navigator.clipboard.writeText(issued.code).catch(() => {});
    setMessage("Code copied to clipboard.");
    window.setTimeout(() => setMessage(null), 2500);
  }

  function toggleLib(id: number) {
    setSelectedLibIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }
  function toggleGroup(id: number) {
    setSelectedGroupIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  if (invites === null) {
    return <p className="text-sm text-white/60">Loading…</p>;
  }

  const open = invites.filter((i) => !i.consumed_at);
  const used = invites.filter((i) => i.consumed_at);
  const libraryName = (id: number) =>
    libraries.find((l) => l.id === id)?.name ?? `Library #${id}`;
  const groupName = (id: number) =>
    groups.find((g) => g.id === id)?.name ?? `Group #${id}`;

  return (
    <div>
      {/* Issuance banner — shown once, then dismissable. */}
      {issued && <IssuedBanner issued={issued} onCopyUrl={copyAcceptUrl} onCopyCode={copyCode} onDismiss={() => setIssued(null)} />}

      <div className="mb-4 space-y-3 rounded-md border border-white/10 bg-white/3 p-4">
        <h3 className="text-xs font-semibold uppercase tracking-wider text-white/50">
          Create invite
        </h3>
        <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
          <label className="text-xs">
            <span className="mb-1 block text-white/60">Recipient email (optional)</span>
            <input
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              placeholder="alice@example.com"
              autoComplete="off"
              spellCheck={false}
              maxLength={320}
              className="w-full rounded bg-white/10 px-3 py-2 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
            />
            <span className="mt-1 block text-[11px] text-white/40">
              When set + SMTP is configured, the link is emailed automatically.
            </span>
          </label>
          <label className="text-xs">
            <span className="mb-1 block text-white/60">Expires</span>
            <select
              value={expiry}
              onChange={(e) => setExpiry(e.target.value as ExpiryChoice)}
              className="w-full rounded bg-white/10 px-3 py-2 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
            >
              {(Object.keys(EXPIRY_LABELS) as ExpiryChoice[]).map((k) => (
                <option key={k} value={k}>
                  {EXPIRY_LABELS[k]}
                </option>
              ))}
            </select>
          </label>
        </div>

        <div>
          <span className="mb-1 block text-xs text-white/60">
            Add to access groups (optional)
          </span>
          {groups.length === 0 ? (
            <p className="text-xs text-white/40">
              No groups yet. Create one under Users → Groups.
            </p>
          ) : (
            <div className="flex flex-wrap gap-2">
              {groups.map((g) => {
                const active = selectedGroupIds.has(g.id);
                return (
                  <button
                    key={g.id}
                    type="button"
                    onClick={() => toggleGroup(g.id)}
                    title={g.description ?? undefined}
                    className={
                      "rounded-full border px-3 py-1 text-xs transition-colors " +
                      (active
                        ? "border-(--color-accent) bg-accent/20 text-white"
                        : "border-white/15 text-white/70 hover:border-white/30")
                    }
                  >
                    {g.name}{" "}
                    <span className="text-white/40">({g.library_count})</span>
                  </button>
                );
              })}
            </div>
          )}
          <span className="mt-1 block text-[11px] text-white/40">
            Group membership grants every library bound to the group.
          </span>
        </div>

        <div>
          <span className="mb-1 block text-xs text-white/60">
            Pre-grant direct library access (optional)
          </span>
          {libraries.length === 0 ? (
            <p className="text-xs text-white/40">
              No libraries to bind. Add libraries first.
            </p>
          ) : (
            <div className="flex flex-wrap gap-2">
              {libraries.map((l) => {
                const active = selectedLibIds.has(l.id);
                return (
                  <button
                    key={l.id}
                    type="button"
                    onClick={() => toggleLib(l.id)}
                    className={
                      "rounded-full border px-3 py-1 text-xs transition-colors " +
                      (active
                        ? "border-(--color-accent) bg-accent/20 text-white"
                        : "border-white/15 text-white/70 hover:border-white/30")
                    }
                  >
                    {l.name}
                  </button>
                );
              })}
            </div>
          )}
          <span className="mt-1 block text-[11px] text-white/40">
            Direct grants add to anything the user inherits via groups.
            You can also grant access after acceptance from Users → Access.
          </span>
        </div>

        <div>
          <button
            type="button"
            onClick={create}
            disabled={busy}
            className="rounded bg-(--color-accent) px-4 py-2.5 text-sm font-semibold text-white sm:py-2 sm:text-xs transition disabled:opacity-50"
          >
            {busy ? "Creating…" : "Create invite"}
          </button>
        </div>
      </div>

      {error && (
        <div className="mb-3 rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {error}
        </div>
      )}
      {message && <p className="mb-3 text-xs text-white/70">{message}</p>}

      {open.length === 0 && used.length === 0 && (
        <p className="text-sm text-white/60">
          No invites yet. Create one above.
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
                key={inv.id}
                className="flex items-start justify-between gap-3 py-3 text-sm"
              >
                <div className="min-w-0 flex-1 space-y-0.5">
                  <div className="text-white/90">
                    {inv.email ?? <span className="text-white/50">No recipient</span>}
                  </div>
                  <div className="text-xs text-white/50">
                    {inv.expires_at
                      ? `Expires ${new Date(inv.expires_at).toLocaleString()}`
                      : "Never expires"}
                    {" · "}
                    {inv.sent_at
                      ? `Emailed ${new Date(inv.sent_at).toLocaleDateString()}`
                      : inv.email
                        ? "Email pending / failed"
                        : "Link only"}
                  </div>
                  {(inv.library_ids.length > 0 || inv.group_ids.length > 0) && (
                    <div className="flex flex-wrap gap-1 pt-1">
                      {inv.group_ids.map((id) => (
                        <span
                          key={`g-${id}`}
                          className="rounded-full border border-accent/30 bg-accent/10 px-2 py-0.5 text-[10px] text-white/75"
                        >
                          group: {groupName(id)}
                        </span>
                      ))}
                      {inv.library_ids.map((id) => (
                        <span
                          key={`l-${id}`}
                          className="rounded-full border border-white/10 px-2 py-0.5 text-[10px] text-white/60"
                        >
                          {libraryName(id)}
                        </span>
                      ))}
                    </div>
                  )}
                </div>
                <div className="flex shrink-0 gap-2">
                  <button
                    type="button"
                    onClick={() => revoke(inv.id)}
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
                key={inv.id}
                className="flex items-center justify-between gap-3 py-2 text-sm"
              >
                <div className="min-w-0 flex-1">
                  <div className="text-white/70">
                    {inv.email ?? <span className="text-white/40">No recipient</span>}
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
    </div>
  );
}

function IssuedBanner({
  issued,
  onCopyUrl,
  onCopyCode,
  onDismiss,
}: {
  issued: CreatedInvite;
  onCopyUrl: () => void;
  onCopyCode: () => void;
  onDismiss: () => void;
}) {
  return (
    <div className="mb-4 rounded-md border border-emerald-500/30 bg-emerald-500/10 p-4 text-xs text-emerald-100">
      <div className="mb-2 flex items-center justify-between">
        <strong className="text-sm">Invite created</strong>
        <button
          type="button"
          onClick={onDismiss}
          className="text-emerald-200/70 hover:text-emerald-100"
        >
          Dismiss
        </button>
      </div>
      <p className="mb-3 text-emerald-200/80">
        {issued.email_sent
          ? `Emailed to ${issued.invite.email}. The link below is your one-time backup — it will not be shown again.`
          : `Share this link with the recipient — it will not be shown again. ${
              issued.invite.email
                ? "(Email delivery failed; copy and send manually.)"
                : "(No email configured.)"
            }`}
      </p>
      {issued.accept_url ? (
        <div className="space-y-2">
          <code className="block break-all rounded bg-black/30 p-2 font-mono text-[11px] text-emerald-100">
            {issued.accept_url}
          </code>
          <button
            type="button"
            onClick={onCopyUrl}
            className="rounded bg-emerald-500/30 px-3 py-1.5 text-[11px] font-semibold text-white hover:bg-emerald-500/40"
          >
            Copy link
          </button>
        </div>
      ) : (
        <div className="space-y-2">
          <p className="text-emerald-200/70">
            Set Server → Network → Public URL to get a clickable accept link.
            Until then, share this code:
          </p>
          <code className="block break-all rounded bg-black/30 p-2 font-mono text-[11px] text-emerald-100">
            {issued.code}
          </code>
          <button
            type="button"
            onClick={onCopyCode}
            className="rounded bg-emerald-500/30 px-3 py-1.5 text-[11px] font-semibold text-white hover:bg-emerald-500/40"
          >
            Copy code
          </button>
        </div>
      )}
    </div>
  );
}

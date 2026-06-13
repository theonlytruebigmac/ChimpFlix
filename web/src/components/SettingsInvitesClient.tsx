"use client";

import { useEffect, useRef, useState } from "react";
import { ConfirmDialog } from "./ConfirmDialog";
import { LoadingPlaceholder } from "./ui/LoadingPlaceholder";
import { formatDate, formatDateTime } from "@/lib/format";
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
  const [askRevokeId, setAskRevokeId] = useState<number | null>(null);
  const [revokeBusy, setRevokeBusy] = useState(false);

  // Tracks the "Copied to clipboard" toast timer so an unmount mid-
  // toast doesn't fire `setMessage` against a torn-down component.
  const messageTimerRef = useRef<number | null>(null);
  useEffect(() => {
    return () => {
      if (messageTimerRef.current !== null) {
        window.clearTimeout(messageTimerRef.current);
        messageTimerRef.current = null;
      }
    };
  }, []);
  function scheduleMessageClear() {
    if (messageTimerRef.current !== null) {
      window.clearTimeout(messageTimerRef.current);
    }
    messageTimerRef.current = window.setTimeout(() => {
      messageTimerRef.current = null;
      setMessage(null);
    }, 2500);
  }

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

  function revoke(id: number) {
    setAskRevokeId(id);
  }

  async function confirmRevoke() {
    if (askRevokeId == null) return;
    setRevokeBusy(true);
    try {
      await authApi.revokeInvite(askRevokeId);
      await refresh();
      setAskRevokeId(null);
    } catch {
      // Close the dialog so the error banner is not hidden behind the overlay.
      setAskRevokeId(null);
      setError("Failed to revoke.");
    } finally {
      setRevokeBusy(false);
    }
  }

  function copyAcceptUrl() {
    if (!issued?.accept_url) return;
    void navigator.clipboard.writeText(issued.accept_url).catch(() => {});
    setMessage("Accept link copied to clipboard.");
    scheduleMessageClear();
  }
  function copyCode() {
    if (!issued?.code) return;
    void navigator.clipboard.writeText(issued.code).catch(() => {});
    setMessage("Code copied to clipboard.");
    scheduleMessageClear();
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
    return <LoadingPlaceholder />;
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
      {issued && (
        <IssuedBanner
          issued={issued}
          onCopyUrl={copyAcceptUrl}
          onCopyCode={copyCode}
          onDismiss={() => setIssued(null)}
        />
      )}

      {/* ── invite someone ─────────────────────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Invite someone</div>
            <div className="cf-sub">
              Sends an email link, or generates a one-time signup URL.
            </div>
          </div>
        </div>
        <div className="cf-card-body cf-pad">
          <div className="cf-grid cf-c2">
            <div className="cf-field" style={{ marginBottom: 0 }}>
              <label className="cf-field-label">Recipient email (optional)</label>
              <input
                type="email"
                className="cf-input"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                placeholder="alice@example.com"
                autoComplete="off"
                spellCheck={false}
                maxLength={320}
              />
              <span className="cf-faint" style={{ display: "block", marginTop: 6, fontSize: 11.5 }}>
                When set + SMTP is configured, the link is emailed automatically.
              </span>
            </div>
            <div className="cf-field" style={{ marginBottom: 0 }}>
              <label className="cf-field-label">Expires</label>
              <select
                className="cf-select"
                value={expiry}
                onChange={(e) => setExpiry(e.target.value as ExpiryChoice)}
              >
                {(Object.keys(EXPIRY_LABELS) as ExpiryChoice[]).map((k) => (
                  <option key={k} value={k}>
                    {EXPIRY_LABELS[k]}
                  </option>
                ))}
              </select>
            </div>
          </div>

          <div className="cf-field" style={{ marginTop: 16, marginBottom: 0 }}>
            <label className="cf-field-label">Add to access groups (optional)</label>
            {groups.length === 0 ? (
              <p className="cf-faint" style={{ fontSize: 12 }}>
                No groups yet. Create one under the Access groups tab.
              </p>
            ) : (
              <div className="cf-flex cf-wrap cf-gap8">
                {groups.map((g) => {
                  const active = selectedGroupIds.has(g.id);
                  return (
                    <button
                      key={g.id}
                      type="button"
                      onClick={() => toggleGroup(g.id)}
                      title={g.description ?? undefined}
                      className={`cf-pill${active ? " cf-accent" : ""}`}
                      style={{ cursor: "pointer", padding: "5px 12px" }}
                    >
                      {g.name}{" "}
                      <span className="cf-faint">({g.library_count})</span>
                    </button>
                  );
                })}
              </div>
            )}
            <span className="cf-faint" style={{ display: "block", marginTop: 6, fontSize: 11.5 }}>
              Group membership grants every library bound to the group.
            </span>
          </div>

          <div className="cf-field" style={{ marginTop: 16, marginBottom: 0 }}>
            <label className="cf-field-label">Pre-grant direct library access (optional)</label>
            {libraries.length === 0 ? (
              <p className="cf-faint" style={{ fontSize: 12 }}>
                No libraries to bind. Add libraries first.
              </p>
            ) : (
              <div className="cf-flex cf-wrap cf-gap8">
                {libraries.map((l) => {
                  const active = selectedLibIds.has(l.id);
                  return (
                    <button
                      key={l.id}
                      type="button"
                      onClick={() => toggleLib(l.id)}
                      className={`cf-pill${active ? " cf-accent" : ""}`}
                      style={{ cursor: "pointer", padding: "5px 12px" }}
                    >
                      {l.name}
                    </button>
                  );
                })}
              </div>
            )}
            <span className="cf-faint" style={{ display: "block", marginTop: 6, fontSize: 11.5 }}>
              Direct grants add to anything the user inherits via groups. You can
              also grant access after acceptance from the Access matrix tab.
            </span>
          </div>

          <div style={{ marginTop: 16 }}>
            <button
              type="button"
              className="cf-btn cf-primary"
              onClick={create}
              disabled={busy}
            >
              {busy ? "Creating…" : "Send invite"}
            </button>
          </div>
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
      {message && (
        <div role="status" aria-live="polite" className="cf-muted" style={{ marginBottom: 16, fontSize: 13 }}>
          {message}
        </div>
      )}

      {open.length === 0 && used.length === 0 && (
        <p className="cf-muted" style={{ fontSize: 13 }}>
          No invites yet. Create one above.
        </p>
      )}

      {open.length > 0 && (
        <div className="cf-card" style={{ marginBottom: used.length > 0 ? 18 : 0 }}>
          <div className="cf-card-head">
            <div>
              <div className="cf-ttl">Pending</div>
            </div>
          </div>
          <table className="cf-table">
            <tbody>
              {open.map((inv) => (
                <tr key={inv.id}>
                  <td>
                    <div className="cf-mono">
                      {inv.email ?? (
                        <span className="cf-faint">No recipient</span>
                      )}
                    </div>
                    <div className="cf-faint" style={{ fontSize: 11.5, marginTop: 2 }}>
                      {inv.sent_at
                        ? `Emailed ${formatDate(inv.sent_at)}`
                        : inv.email
                          ? "Email pending / failed"
                          : "Link only"}
                    </div>
                    {(inv.library_ids.length > 0 || inv.group_ids.length > 0) && (
                      <div
                        className="cf-flex cf-wrap"
                        style={{ gap: 4, marginTop: 6 }}
                      >
                        {inv.group_ids.map((id) => (
                          <span
                            key={`g-${id}`}
                            className="cf-pill cf-accent"
                            style={{ padding: "1px 7px", fontSize: 10 }}
                          >
                            group: {groupName(id)}
                          </span>
                        ))}
                        {inv.library_ids.map((id) => (
                          <span
                            key={`l-${id}`}
                            className="cf-pill"
                            style={{ padding: "1px 7px", fontSize: 10 }}
                          >
                            {libraryName(id)}
                          </span>
                        ))}
                      </div>
                    )}
                  </td>
                  <td className="cf-faint" style={{ whiteSpace: "nowrap" }}>
                    {inv.expires_at
                      ? `expires ${formatDateTime(inv.expires_at)}`
                      : "never expires"}
                  </td>
                  <td className="cf-num">
                    <button
                      type="button"
                      className="cf-btn cf-ghost cf-tiny"
                      onClick={() => revoke(inv.id)}
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

      {used.length > 0 && (
        <div className="cf-card" style={{ marginBottom: 0 }}>
          <div className="cf-card-head">
            <div>
              <div className="cf-ttl">Used</div>
            </div>
          </div>
          <table className="cf-table">
            <tbody>
              {used.map((inv) => (
                <tr key={inv.id}>
                  <td>
                    <div className="cf-mono cf-muted">
                      {inv.email ?? (
                        <span className="cf-faint">No recipient</span>
                      )}
                    </div>
                  </td>
                  <td className="cf-faint" style={{ whiteSpace: "nowrap" }}>
                    Used {inv.consumed_at ? formatDateTime(inv.consumed_at) : ""}
                  </td>
                  <td className="cf-num"></td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {askRevokeId != null && (
        <ConfirmDialog
          title="Revoke this invite?"
          body="The invite link / code will stop working. Anyone who already accepted keeps their account."
          confirmLabel="Revoke"
          destructive
          busy={revokeBusy}
          onConfirm={() => void confirmRevoke()}
          onCancel={() => setAskRevokeId(null)}
        />
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
    <div className="cf-banner cf-ok" style={{ flexDirection: "column", alignItems: "stretch" }}>
      <div className="cf-flex cf-between">
        <b>Invite created</b>
        <button type="button" className="cf-btn cf-ghost cf-tiny" onClick={onDismiss}>
          Dismiss
        </button>
      </div>
      <p style={{ margin: "8px 0", fontSize: 12.5 }}>
        {issued.email_sent
          ? `Emailed to ${issued.invite.email}. The link below is your one-time backup — it will not be shown again.`
          : `Share this link with the recipient — it will not be shown again. ${
              issued.invite.email
                ? "(Email delivery failed; copy and send manually.)"
                : "(No email configured.)"
            }`}
      </p>
      {issued.accept_url ? (
        <div className="cf-stack" style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          <code
            className="cf-mono"
            style={{
              display: "block",
              wordBreak: "break-all",
              borderRadius: 6,
              background: "rgba(0,0,0,.3)",
              padding: 8,
              fontSize: 11,
            }}
          >
            {issued.accept_url}
          </code>
          <div>
            <button type="button" className="cf-btn cf-sm" onClick={onCopyUrl}>
              Copy link
            </button>
          </div>
        </div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          <p style={{ margin: 0, fontSize: 12.5 }}>
            Set Server → Network → Public URL to get a clickable accept link.
            Until then, share this code:
          </p>
          <code
            className="cf-mono"
            style={{
              display: "block",
              wordBreak: "break-all",
              borderRadius: 6,
              background: "rgba(0,0,0,.3)",
              padding: 8,
              fontSize: 11,
            }}
          >
            {issued.code}
          </code>
          <div>
            <button type="button" className="cf-btn cf-sm" onClick={onCopyCode}>
              Copy code
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

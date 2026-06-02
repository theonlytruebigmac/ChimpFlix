"use client";

import { useEffect, useMemo, useState } from "react";
import {
  admin as adminApi,
  auth as authApi,
  ChimpFlixApiError,
  type AccessLevel,
  type AccessMatrixEntry,
  type AdminSessionSummary,
  type AuditLogEntry,
  type User,
  type UserRole,
} from "@/lib/chimpflix-api";
import { ConfirmDialog } from "../ConfirmDialog";
import { LoadingPlaceholder } from "../ui/LoadingPlaceholder";
import { formatDate, formatDateTime } from "@/lib/format";

interface Props {
  currentUserId: number;
  currentUserRole: UserRole;
}

type FilterId = "all" | "owners" | "admins" | "online" | "locked";

const ROLE_PILL: Record<UserRole, string> = {
  owner: "cf-pill cf-accent",
  admin: "cf-pill cf-info",
  user: "cf-pill",
};
const ROLE_LABEL: Record<UserRole, string> = {
  owner: "Owner",
  admin: "Admin",
  user: "User",
};

function tier(role: UserRole): number {
  switch (role) {
    case "owner":
      return 2;
    case "admin":
      return 1;
    case "user":
      return 0;
  }
}

/// One Users master-detail: the user list on the left, a sticky tabbed
/// drawer (Profile / Access / Devices / Audit) on the right. Folds in
/// what used to live at /users/devices and /users/access per user.
export function AdminUsersUnifiedClient({
  currentUserId,
  currentUserRole,
}: Props) {
  const [users, setUsers] = useState<User[] | null>(null);
  const [sessions, setSessions] = useState<AdminSessionSummary[]>([]);
  const [matrix, setMatrix] = useState<AccessMatrixEntry[]>([]);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [filter, setFilter] = useState<FilterId>("all");
  const [query, setQuery] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  // Ticks whenever the drawer needs to refresh derived data (after
  // role/reset/delete/revoke). Cheap: re-fetches the three lists.
  const [refreshTick, setRefreshTick] = useState(0);
  // Wall-clock at fetch time, used by `formatRelative` to display
  // "last seen 3h ago" in the table. Calling Date.now() during
  // render trips react-hooks/purity; capture it alongside the
  // fetched data so we re-derive only when the data refreshes.
  const [nowMs, setNowMs] = useState(0);

  useEffect(() => {
    let cancelled = false;
    async function loadAll() {
      try {
        const [u, s, m] = await Promise.all([
          authApi.listUsers(),
          adminApi.sessions.list().catch(() => ({ sessions: [] as AdminSessionSummary[] })),
          adminApi.access
            .get()
            .catch(() => ({ entries: [] as AccessMatrixEntry[] })),
        ]);
        if (cancelled) return;
        setUsers(u.users);
        setSessions(s.sessions);
        setMatrix(m.entries);
        setNowMs(Date.now());
      } catch (e) {
        if (cancelled) return;
        if (e instanceof ChimpFlixApiError && e.status === 403) {
          setMessage("Only owners and admins can manage users.");
        } else {
          setMessage("Failed to load users.");
        }
        setUsers([]);
      }
    }
    loadAll();
    return () => {
      cancelled = true;
    };
  }, [refreshTick]);

  // Sessions per user for the online/streaming indicator + Devices
  // tab. Computed once per fetch so the table's Status column doesn't
  // do a linear scan per row.
  const sessionsByUser = useMemo(() => {
    const map = new Map<number, AdminSessionSummary[]>();
    for (const s of sessions) {
      const list = map.get(s.user_id) ?? [];
      list.push(s);
      map.set(s.user_id, list);
    }
    return map;
  }, [sessions]);

  const accessByUser = useMemo(() => {
    const map = new Map<number, AccessMatrixEntry[]>();
    for (const e of matrix) {
      const list = map.get(e.user_id) ?? [];
      list.push(e);
      map.set(e.user_id, list);
    }
    return map;
  }, [matrix]);

  const counts = useMemo(() => {
    const all = users?.length ?? 0;
    const owners = users?.filter((u) => u.role === "owner").length ?? 0;
    const admins = users?.filter((u) => u.role === "admin").length ?? 0;
    const online =
      users?.filter((u) => (sessionsByUser.get(u.id)?.length ?? 0) > 0)
        .length ?? 0;
    const locked = users?.filter((u) => u.locked).length ?? 0;
    return { all, owners, admins, online, locked };
  }, [users, sessionsByUser]);

  const filtered = useMemo(() => {
    if (!users) return [];
    let list = users;
    if (filter === "owners") list = list.filter((u) => u.role === "owner");
    else if (filter === "admins") list = list.filter((u) => u.role === "admin");
    else if (filter === "online") {
      list = list.filter((u) => (sessionsByUser.get(u.id)?.length ?? 0) > 0);
    } else if (filter === "locked") list = list.filter((u) => u.locked);
    if (query.trim()) {
      const q = query.trim().toLowerCase();
      list = list.filter(
        (u) =>
          u.username.toLowerCase().includes(q) ||
          (u.display_name ?? "").toLowerCase().includes(q) ||
          (u.email ?? "").toLowerCase().includes(q),
      );
    }
    return list;
  }, [users, filter, query, sessionsByUser]);

  const selected = useMemo(
    () => users?.find((u) => u.id === selectedId) ?? null,
    [users, selectedId],
  );

  function bumpRefresh() {
    setRefreshTick((t) => t + 1);
  }

  if (users === null) {
    return <LoadingPlaceholder />;
  }
  if (users.length === 0) {
    return (
      <p className="cf-muted" style={{ fontSize: 13 }}>
        {message ?? "No users yet."}
      </p>
    );
  }

  const chips: Array<{ id: FilterId; label: string }> = [
    { id: "all", label: "All" },
    { id: "owners", label: "Owners" },
    { id: "admins", label: "Admins" },
    { id: "online", label: "Online" },
    { id: "locked", label: "Locked" },
  ];

  // Only render the 1fr + 440px master-detail grid when a row is
  // selected; otherwise let the list span the full width.
  return (
    <div>
      <div
        className="cf-flex cf-between cf-wrap cf-gap12"
        style={{ marginBottom: 14 }}
      >
        <div className="cf-seg cf-accent" role="tablist" aria-label="Filter users">
          {chips.map((c) => (
            <button
              key={c.id}
              type="button"
              role="tab"
              aria-selected={filter === c.id}
              className={filter === c.id ? "cf-on" : ""}
              onClick={() => setFilter(c.id)}
            >
              {c.label}
              {counts[c.id] > 0 && (
                <span style={{ marginLeft: 6, opacity: 0.6 }}>
                  {counts[c.id]}
                </span>
              )}
            </button>
          ))}
        </div>
        <input
          className="cf-input cf-w-auto"
          style={{ minWidth: 220 }}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search name or email…"
        />
      </div>

      <div className={selected ? "cf-md" : undefined}>
        <div className="cf-md-list">
          {filtered.map((u) => (
            <UserRow
              key={u.id}
              user={u}
              selected={selectedId === u.id}
              currentUserId={currentUserId}
              liveSessions={sessionsByUser.get(u.id) ?? []}
              onSelect={() => setSelectedId(u.id)}
            />
          ))}
          {filtered.length === 0 && (
            <div
              className="cf-card"
              style={{ marginBottom: 0 }}
            >
              <div className="cf-card-body cf-pad cf-center cf-muted" style={{ fontSize: 13 }}>
                No users match this filter.
              </div>
            </div>
          )}
        </div>

        {selected && (
          <UserDrawer
            key={selected.id}
            user={selected}
            currentUserId={currentUserId}
            currentUserRole={currentUserRole}
            sessions={sessionsByUser.get(selected.id) ?? []}
            access={accessByUser.get(selected.id) ?? []}
            fullMatrix={matrix}
            nowMs={nowMs}
            onClose={() => setSelectedId(null)}
            onChanged={bumpRefresh}
            onDeleted={() => {
              setSelectedId(null);
              bumpRefresh();
            }}
            onMessage={setMessage}
          />
        )}
      </div>
      {message && (
        <p className="cf-muted" style={{ marginTop: 12, fontSize: 12.5 }}>
          {message}
        </p>
      )}
    </div>
  );
}

// ─── Table row ──────────────────────────────────────────────────────

function UserRow({
  user,
  selected,
  currentUserId,
  liveSessions,
  onSelect,
}: {
  user: User;
  selected: boolean;
  currentUserId: number;
  liveSessions: AdminSessionSummary[];
  onSelect: () => void;
}) {
  const online = liveSessions.length > 0;
  const name = user.display_name ?? user.username;
  return (
    <button
      type="button"
      onClick={onSelect}
      className={`cf-md-item${selected ? " cf-active" : ""}`}
    >
      <Avatar name={name} />
      <div className="min-w-0">
        <div className="cf-md-name">
          {name}
          {user.id === currentUserId && (
            <span className="cf-faint" style={{ marginLeft: 6, fontWeight: 400 }}>
              (you)
            </span>
          )}
        </div>
        <div className="cf-md-sub">{user.email ?? `@${user.username}`}</div>
      </div>
      <div className="cf-md-trail">
        <span className={ROLE_PILL[user.role]} style={{ padding: "2px 8px" }}>
          {ROLE_LABEL[user.role]}
        </span>
        {user.locked && (
          <span className="cf-pill cf-err" style={{ padding: "1px 7px" }}>
            Locked
          </span>
        )}
        {online ? (
          <span className="cf-pill cf-ok" style={{ padding: "1px 7px" }}>
            <span className="cf-dot" />
            Online
          </span>
        ) : (
          <span className="cf-pill" style={{ padding: "1px 7px" }}>
            <span className="cf-dot" style={{ background: "var(--ghost)" }} />
            Offline
          </span>
        )}
      </div>
    </button>
  );
}

// ─── Avatar ─────────────────────────────────────────────────────────

/// Five-tone console avatar keyed by name so the same user always gets
/// the same color across the list and drawer header.
function Avatar({
  name,
  size,
}: {
  name: string;
  size?: number;
}) {
  const initials = name
    .split(/\s+/)
    .map((s) => s[0]?.toUpperCase() ?? "")
    .join("")
    .slice(0, 2);
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h + name.charCodeAt(i)) % 5;
  const tone = `cf-a${h + 1}`;
  const style = size
    ? { width: size, height: size, fontSize: Math.round(size * 0.38) }
    : undefined;
  return (
    <span className={`cf-avatar ${tone}`} style={style}>
      {initials || "?"}
    </span>
  );
}

// ─── Drawer ─────────────────────────────────────────────────────────

const DRAWER_TABS = [
  { id: "profile", label: "Profile" },
  { id: "access", label: "Access" },
  { id: "devices", label: "Devices" },
  { id: "audit", label: "Audit" },
] as const;

function UserDrawer({
  user,
  currentUserId,
  currentUserRole,
  sessions,
  access,
  fullMatrix,
  nowMs,
  onClose,
  onChanged,
  onDeleted,
  onMessage,
}: {
  user: User;
  currentUserId: number;
  currentUserRole: UserRole;
  sessions: AdminSessionSummary[];
  access: AccessMatrixEntry[];
  fullMatrix: AccessMatrixEntry[];
  nowMs: number;
  onClose: () => void;
  onChanged: () => void;
  onDeleted: () => void;
  onMessage: (msg: string | null) => void;
}) {
  const [tab, setTab] = useState<string>("profile");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Confirmation gating — boolean per action since each fires against
  // the single `user` this drawer is rendered for.
  const [askSendReset, setAskSendReset] = useState(false);
  const [askResetTwoFactor, setAskResetTwoFactor] = useState(false);
  const [askKickSessions, setAskKickSessions] = useState(false);
  const [askDeleteUser, setAskDeleteUser] = useState(false);
  const sessionCount = sessions.length;
  const online = sessionCount > 0;
  const canActOnTarget =
    user.id !== currentUserId && tier(currentUserRole) >= tier(user.role);
  const name = user.display_name ?? user.username;

  async function withBusy<T>(label: string, fn: () => Promise<T>) {
    setBusy(label);
    setError(null);
    onMessage(null);
    try {
      const r = await fn();
      return r;
    } catch (e) {
      setError(friendlyError(e));
      throw e;
    } finally {
      setBusy(null);
    }
  }

  async function setRole(role: UserRole) {
    try {
      await withBusy("role", () => authApi.setUserRole(user.id, role));
      onChanged();
    } catch {
      /* error surfaced */
    }
  }
  async function setLocked(locked: boolean) {
    try {
      await withBusy("lock", () =>
        locked ? adminApi.lockUser(user.id) : adminApi.unlockUser(user.id),
      );
      onChanged();
    } catch {
      /* error surfaced */
    }
  }
  function sendPasswordReset() {
    setAskSendReset(true);
  }
  async function confirmSendPasswordReset() {
    setAskSendReset(false);
    try {
      const r = await withBusy("reset", () =>
        adminApi.sendUserPasswordReset(user.id),
      );
      onMessage(r.message);
    } catch {
      /* error surfaced */
    }
  }
  function resetTwoFactor() {
    setAskResetTwoFactor(true);
  }
  async function confirmResetTwoFactor() {
    setAskResetTwoFactor(false);
    try {
      await withBusy("2fa", () => adminApi.resetUserTwoFactor(user.id));
      onMessage("2FA reset.");
    } catch {
      /* error surfaced */
    }
  }
  function kickSessions() {
    if (sessionCount === 0) return;
    setAskKickSessions(true);
  }
  async function confirmKickSessions() {
    setAskKickSessions(false);
    try {
      await withBusy("kick", async () => {
        for (const s of sessions) {
          await adminApi.sessions.revoke(s.id);
        }
      });
      onChanged();
    } catch {
      /* error surfaced */
    }
  }
  async function revokeSession(id: number) {
    try {
      await withBusy(`session-${id}`, () => adminApi.sessions.revoke(id));
      onChanged();
    } catch {
      /* error surfaced */
    }
  }
  function deleteUser() {
    setAskDeleteUser(true);
  }
  async function confirmDeleteUser() {
    setAskDeleteUser(false);
    try {
      await withBusy("delete", () => authApi.deleteUser(user.id));
      onMessage(`Removed "${name}".`);
      onDeleted();
    } catch {
      /* error surfaced */
    }
  }

  const accessCount = access.filter((e) => e.allowed).length;

  return (
    <div className="cf-drawer">
      <div className="cf-drawer-head">
        <Avatar name={name} size={42} />
        <div className="min-w-0" style={{ flex: 1 }}>
          <div
            style={{ fontSize: 16, fontWeight: 700 }}
            className="truncate"
          >
            {name}
          </div>
          <div className="cf-sub">
            {(user.email ?? `@${user.username}`)} ·{" "}
            {online ? "Online now" : "Offline"}
          </div>
        </div>
        <button
          type="button"
          onClick={onClose}
          aria-label="Close"
          className="cf-btn cf-ghost cf-tiny"
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <line x1="18" y1="6" x2="6" y2="18" />
            <line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      </div>

      <div className="cf-drawer-body">
        <div className="cf-mtabs" role="tablist">
          {DRAWER_TABS.map((t) => {
            const count =
              t.id === "devices" && sessionCount > 0
                ? sessionCount
                : t.id === "access" && accessCount > 0
                  ? accessCount
                  : undefined;
            return (
              <button
                key={t.id}
                type="button"
                role="tab"
                aria-selected={tab === t.id}
                className={`cf-mtab${tab === t.id ? " cf-on" : ""}`}
                onClick={() => setTab(t.id)}
              >
                {t.label}
                {count !== undefined && (
                  <span style={{ marginLeft: 5, opacity: 0.6 }}>{count}</span>
                )}
              </button>
            );
          })}
        </div>

        {error && (
          <div
            role="alert"
            aria-live="assertive"
            className="cf-banner cf-err"
          >
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <circle cx="12" cy="12" r="9" />
              <path d="M12 8v4M12 16v.5" />
            </svg>
            <div>{error}</div>
          </div>
        )}

        {tab === "profile" && (
          <ProfileTab
            user={user}
            sessionCount={sessionCount}
            canActOnTarget={canActOnTarget}
            isSelf={user.id === currentUserId}
            currentUserRole={currentUserRole}
            busy={busy}
            onSetRole={setRole}
            onSetLocked={setLocked}
            onSendReset={sendPasswordReset}
            onReset2FA={resetTwoFactor}
            onKickSessions={kickSessions}
          />
        )}

        {tab === "access" && (
          <AccessTab
            userId={user.id}
            isOwner={user.role === "owner"}
            entries={access}
            fullMatrix={fullMatrix}
            onChanged={onChanged}
            onError={setError}
          />
        )}

        {tab === "devices" && (
          <DevicesTab
            sessions={sessions}
            busy={busy}
            nowMs={nowMs}
            onRevoke={revokeSession}
          />
        )}

        {tab === "audit" && (
          <AuditTab userId={user.id} username={user.username} />
        )}
      </div>

      <div className="cf-drawer-foot">
        {canActOnTarget && (
          <button
            type="button"
            onClick={deleteUser}
            disabled={busy === "delete"}
            className="cf-btn cf-danger cf-sm"
            style={{ marginLeft: "auto" }}
          >
            {busy === "delete" ? "Deleting…" : "Delete user"}
          </button>
        )}
      </div>

      {askSendReset && (
        <ConfirmDialog
          title={`Send password reset to "${name}"?`}
          body="A reset link is emailed to the address on file. The link expires in 1 hour and can only be used once."
          confirmLabel="Send"
          busy={busy === "reset"}
          onConfirm={() => void confirmSendPasswordReset()}
          onCancel={() => setAskSendReset(false)}
        />
      )}
      {askResetTwoFactor && (
        <ConfirmDialog
          title={`Reset 2FA for "${name}"?`}
          body="They'll log in with just their password until they re-enroll. Existing recovery codes are invalidated."
          confirmLabel="Reset 2FA"
          destructive
          busy={busy === "2fa"}
          onConfirm={() => void confirmResetTwoFactor()}
          onCancel={() => setAskResetTwoFactor(false)}
        />
      )}
      {askKickSessions && (
        <ConfirmDialog
          title={`Revoke all ${sessionCount} active session${sessionCount === 1 ? "" : "s"}?`}
          body={`"${name}" will be signed out of every device. Anything currently playing stops at the next heartbeat.`}
          confirmLabel="Revoke sessions"
          destructive
          busy={busy === "kick"}
          onConfirm={() => void confirmKickSessions()}
          onCancel={() => setAskKickSessions(false)}
        />
      )}
      {askDeleteUser && (
        <ConfirmDialog
          title={`Delete user "${name}"?`}
          body="Their sessions, watch state, and personal lists will be removed. Library files on disk are untouched."
          confirmLabel="Delete user"
          destructive
          busy={busy === "delete"}
          onConfirm={() => void confirmDeleteUser()}
          onCancel={() => setAskDeleteUser(false)}
        />
      )}
    </div>
  );
}

// ─── Drawer tabs ────────────────────────────────────────────────────

function ProfileTab({
  user,
  sessionCount,
  canActOnTarget,
  isSelf,
  currentUserRole,
  busy,
  onSetRole,
  onSetLocked,
  onSendReset,
  onReset2FA,
  onKickSessions,
}: {
  user: User;
  sessionCount: number;
  canActOnTarget: boolean;
  isSelf: boolean;
  currentUserRole: UserRole;
  busy: string | null;
  onSetRole: (role: UserRole) => void;
  onSetLocked: (locked: boolean) => void;
  onSendReset: () => void;
  onReset2FA: () => void;
  onKickSessions: () => void;
}) {
  const lastLogin = user.last_login_at
    ? `${formatDateTime(user.last_login_at)}${user.last_login_ip ? ` · ${user.last_login_ip}` : ""}`
    : "never";

  return (
    <>
      {/* Role — editable in-place when the actor outranks the target. */}
      <div className="cf-field">
        <label className="cf-field-label">Role</label>
        {canActOnTarget ? (
          <select
            className="cf-select"
            value={user.role}
            disabled={busy === "role"}
            onChange={(e) => {
              const next = e.target.value as UserRole;
              if (next !== user.role) onSetRole(next);
            }}
          >
            <option value="owner">Owner</option>
            <option value="admin">Admin</option>
            <option value="user">User</option>
          </select>
        ) : (
          <div className="cf-muted" style={{ fontSize: 13 }}>
            {user.role === "owner"
              ? "Owner"
              : user.role === "admin"
                ? "Admin"
                : "User"}
          </div>
        )}
      </div>

      <div className="cf-grid cf-c2">
        <div className="cf-field" style={{ marginBottom: 0 }}>
          <label className="cf-field-label">Created</label>
          <div style={{ fontSize: 13 }}>{formatDate(user.created_at)}</div>
        </div>
        <div className="cf-field" style={{ marginBottom: 0 }}>
          <label className="cf-field-label">Last login</label>
          <div style={{ fontSize: 13 }}>{lastLogin}</div>
        </div>
      </div>

      <div className="cf-row" style={{ padding: "13px 0" }}>
        <div className="cf-row-main">
          <div className="cf-row-label" style={{ fontSize: 13 }}>
            Active sessions
          </div>
          <div className="cf-row-help">
            {sessionCount === 0
              ? "No devices currently signed in."
              : `${sessionCount} device${sessionCount === 1 ? "" : "s"} signed in — see the Devices tab.`}
          </div>
        </div>
        <div className="cf-row-control">
          <span className="cf-num-badge">{sessionCount}</span>
        </div>
      </div>

      <div className="cf-row" style={{ padding: "13px 0" }}>
        <div className="cf-row-main">
          <div className="cf-row-label" style={{ fontSize: 13 }}>
            Email mirror
          </div>
          <div className="cf-row-help">
            Mirrors in-app notifications to {user.email ?? "the account email"}.
          </div>
        </div>
        <div className="cf-row-control">
          <span className={`cf-pill${user.notify_via_email ? " cf-ok" : ""}`}>
            {user.notify_via_email ? "On" : "Off"}
          </span>
        </div>
      </div>

      {canActOnTarget && (
        <>
          {/* Lock account — owners are never lockable, and you can't
              lock yourself, so the switch is hidden in those cases. */}
          {user.role !== "owner" && !isSelf && (
            <div className="cf-row" style={{ padding: "13px 0" }}>
              <div className="cf-row-main">
                <div className="cf-row-label" style={{ fontSize: 13 }}>
                  Lock account
                </div>
                <div className="cf-row-help">
                  {user.locked
                    ? "This account is disabled — sign-in is blocked until you unlock it."
                    : "Disable sign-in for this account without deleting it."}
                </div>
              </div>
              <div className="cf-row-control">
                <button
                  type="button"
                  role="switch"
                  aria-checked={user.locked}
                  aria-label="Lock account"
                  disabled={busy === "lock"}
                  className={"cf-switch" + (user.locked ? " cf-on" : "")}
                  onClick={() => onSetLocked(!user.locked)}
                />
              </div>
            </div>
          )}

          <div className="cf-row" style={{ padding: "13px 0" }}>
            <div className="cf-row-main">
              <div className="cf-row-label" style={{ fontSize: 13 }}>
                Reset password
              </div>
              <div className="cf-row-help">Emails a one-time reset link.</div>
            </div>
            <div className="cf-row-control">
              <button
                type="button"
                className="cf-btn cf-sm"
                onClick={onSendReset}
                disabled={busy === "reset"}
              >
                {busy === "reset" ? "…" : "Send link"}
              </button>
            </div>
          </div>

          <div className="cf-row" style={{ padding: "13px 0" }}>
            <div className="cf-row-main">
              <div className="cf-row-label" style={{ fontSize: 13 }}>
                Reset two-factor
              </div>
              <div className="cf-row-help">
                Clears their authenticator + recovery codes.
              </div>
            </div>
            <div className="cf-row-control">
              <button
                type="button"
                className="cf-btn cf-sm"
                onClick={onReset2FA}
                disabled={busy === "2fa"}
              >
                {busy === "2fa" ? "…" : "Reset 2FA"}
              </button>
            </div>
          </div>

          <div className="cf-row" style={{ padding: "13px 0" }}>
            <div className="cf-row-main">
              <div className="cf-row-label" style={{ fontSize: 13 }}>
                Sign out everywhere
              </div>
              <div className="cf-row-help">
                Revokes every active session for this user.
              </div>
            </div>
            <div className="cf-row-control">
              <button
                type="button"
                className="cf-btn cf-sm"
                onClick={onKickSessions}
                disabled={busy === "kick" || sessionCount === 0}
              >
                {busy === "kick" ? "…" : "Kick sessions"}
              </button>
            </div>
          </div>

          <p className="cf-faint" style={{ marginTop: 10, fontSize: 11.5 }}>
            Role hierarchy:{" "}
            <span className="cf-mono">
              {currentUserRole === "owner"
                ? "owner can manage anyone"
                : "admin can manage user + admin (not owners)"}
            </span>
          </p>
        </>
      )}
    </>
  );
}

// Per-library tri-state access control for one user (phase 107). Each
// library gets a None / View / Full segmented control that edits the
// DIRECT `library_access` grant level; group-derived access (`via_groups`)
// is shown as informational pills and isn't editable here (it lives under
// Users → Groups). Persists the whole set in one bulk PUT to the same
// /admin/access-matrix endpoint the matrix uses, scoped to this user.
const ACCESS_LEVELS: { id: AccessLevel; label: string }[] = [
  { id: "none", label: "None" },
  { id: "view", label: "View" },
  { id: "full", label: "Full" },
];

function AccessTab({
  userId,
  isOwner,
  entries,
  fullMatrix,
  onChanged,
  onError,
}: {
  userId: number;
  isOwner: boolean;
  entries: AccessMatrixEntry[];
  // The whole user × library matrix. The save path is a per-library
  // full-replace on the backend, so we must resend EVERY user's current
  // direct level for each touched library — otherwise other users' grants
  // would be wiped. We pull those siblings out of the full matrix.
  fullMatrix: AccessMatrixEntry[];
  onChanged: () => void;
  onError: (msg: string | null) => void;
}) {
  // Libraries this user could be granted, with their current direct level.
  // Sorted by name so the list is stable across refreshes.
  const libs = useMemo(
    () =>
      [...entries]
        .sort((a, b) => a.library_name.localeCompare(b.library_name))
        .map((e) => ({
          library_id: e.library_id,
          library_name: e.library_name,
          level: e.level,
          via_groups: e.via_groups,
        })),
    [entries],
  );

  // Local edit state: library_id → desired direct level. Seeded from the
  // server's direct-grant levels.
  const [levels, setLevels] = useState<Map<number, AccessLevel>>(
    () => new Map(libs.map((l) => [l.library_id, l.level])),
  );
  const [busy, setBusy] = useState(false);

  // Re-seed when the user (and thus `entries`) changes. The drawer is keyed
  // by user id so this remounts per user, but a post-save refresh swaps the
  // `entries` prop in place — resync so the baseline tracks the saved state.
  useEffect(() => {
    setLevels(new Map(libs.map((l) => [l.library_id, l.level])));
  }, [libs]);

  if (isOwner) {
    return (
      <p className="cf-muted" style={{ fontSize: 12.5 }}>
        Owners always have <b>Full</b> access to every library — there's
        nothing to configure here.
      </p>
    );
  }

  if (libs.length === 0) {
    return (
      <p className="cf-muted" style={{ fontSize: 12.5 }}>
        No libraries exist yet. Create one under{" "}
        <a href="/settings/admin/libraries">Libraries</a> to grant access.
      </p>
    );
  }

  const dirty = libs.some(
    (l) => (levels.get(l.library_id) ?? "none") !== l.level,
  );

  function setLevel(libraryId: number, level: AccessLevel) {
    setLevels((prev) => {
      const next = new Map(prev);
      next.set(libraryId, level);
      return next;
    });
  }

  async function save() {
    setBusy(true);
    onError(null);
    try {
      // The /admin/access-matrix PUT is a per-library FULL replace of every
      // direct grant. So for each library this user's level changed, we
      // resend the COMPLETE direct-grant set: every OTHER user's current
      // direct level (from the full matrix) plus this user's new level.
      // Owners are force-kept at full server-side regardless. Only touched
      // libraries are included so we don't needlessly rewrite the rest.
      const changed = libs.filter(
        (l) => (levels.get(l.library_id) ?? "none") !== l.level,
      );
      const payload = changed.map((l) => {
        const others = fullMatrix
          .filter(
            (e) =>
              e.library_id === l.library_id &&
              e.user_id !== userId &&
              e.allowed,
          )
          .map((e) => ({ user_id: e.user_id, level: e.level }));
        return {
          library_id: l.library_id,
          grants: [
            ...others,
            { user_id: userId, level: levels.get(l.library_id) ?? "none" },
          ],
        };
      });
      await adminApi.access.put(payload);
      onChanged();
    } catch (e) {
      onError(friendlyError(e));
    } finally {
      setBusy(false);
    }
  }

  function discard() {
    setLevels(new Map(libs.map((l) => [l.library_id, l.level])));
  }

  return (
    <>
      <p className="cf-muted" style={{ marginTop: 0, fontSize: 12.5 }}>
        Per-library access for this user. <b>None</b> hides the library,{" "}
        <b>View</b> lets them browse it without playing, <b>Full</b> allows
        browse + playback.
      </p>
      <AccessLegend />
      {libs.map((l) => {
        const cur = levels.get(l.library_id) ?? "none";
        return (
          <div className="cf-row" key={l.library_id} style={{ padding: "11px 0" }}>
            <div className="cf-row-main">
              <div className="cf-row-label" style={{ fontSize: 13 }}>
                {l.library_name}
              </div>
              {l.via_groups.length > 0 && (
                <div
                  className="cf-flex cf-wrap cf-gap8"
                  style={{ marginTop: 4 }}
                >
                  {l.via_groups.map((g) => (
                    <span
                      key={g}
                      className="cf-pill cf-info"
                      style={{ padding: "1px 7px", fontSize: 10.5 }}
                    >
                      via {g}
                    </span>
                  ))}
                </div>
              )}
            </div>
            <div className="cf-row-control">
              <div
                className="cf-seg"
                role="group"
                aria-label={`Access level for ${l.library_name}`}
              >
                {ACCESS_LEVELS.map((opt) => (
                  <button
                    key={opt.id}
                    type="button"
                    aria-pressed={cur === opt.id}
                    className={cur === opt.id ? "cf-on" : ""}
                    onClick={() => setLevel(l.library_id, opt.id)}
                  >
                    {opt.label}
                  </button>
                ))}
              </div>
            </div>
          </div>
        );
      })}
      {dirty && (
        <div className="cf-flex cf-gap8" style={{ marginTop: 14 }}>
          <button
            type="button"
            className="cf-btn cf-ghost cf-sm"
            onClick={discard}
            disabled={busy}
          >
            Discard
          </button>
          <button
            type="button"
            className="cf-btn cf-primary cf-sm"
            onClick={save}
            disabled={busy}
          >
            {busy ? "Saving…" : "Save access"}
          </button>
        </div>
      )}
    </>
  );
}

// Three-tier legend shared by the drawer Access tab. Canonical cf-pill
// tones (match the access matrix + groups): None = muted, View = amber
// (cf-warn), Full = green (cf-ok). The blue "via {group}" pills above are
// deliberately a different tone — they mark group-derived access, not a
// level the admin sets here.
function AccessLegend() {
  return (
    <div
      className="cf-flex cf-wrap cf-gap8"
      style={{ margin: "2px 0 12px", alignItems: "center" }}
    >
      <span className="cf-pill" style={{ padding: "1px 8px", fontSize: 10.5 }}>
        None · hidden
      </span>
      <span
        className="cf-pill cf-warn"
        style={{ padding: "1px 8px", fontSize: 10.5 }}
      >
        View · browse only
      </span>
      <span
        className="cf-pill cf-ok"
        style={{ padding: "1px 8px", fontSize: 10.5 }}
      >
        Full · browse + play
      </span>
    </div>
  );
}

function DevicesTab({
  sessions,
  busy,
  nowMs,
  onRevoke,
}: {
  sessions: AdminSessionSummary[];
  busy: string | null;
  nowMs: number;
  onRevoke: (id: number) => void;
}) {
  if (sessions.length === 0) {
    return (
      <p className="cf-muted" style={{ fontSize: 12.5 }}>
        No active sessions for this user.
      </p>
    );
  }
  return (
    <div className="cf-md-list" style={{ gap: 6 }}>
      {sessions.map((s) => {
        const last =
          nowMs > 0
            ? formatRelative(nowMs - s.last_seen_at)
            : formatDateTime(s.last_seen_at);
        return (
          <div
            key={s.id}
            className="cf-flex cf-between"
            style={{
              padding: "11px 13px",
              border: "1px solid var(--line)",
              borderRadius: "var(--r)",
            }}
          >
            <div className="min-w-0">
              <div style={{ fontWeight: 600, fontSize: 13 }} className="truncate">
                {summarizeUserAgent(s.user_agent)}
              </div>
              <div className="cf-faint cf-mono" style={{ fontSize: 11 }}>
                {s.ip ?? "—"} · {last}
              </div>
            </div>
            <button
              type="button"
              onClick={() => onRevoke(s.id)}
              disabled={busy === `session-${s.id}`}
              className="cf-btn cf-ghost cf-tiny"
            >
              {busy === `session-${s.id}` ? "…" : "Revoke"}
            </button>
          </div>
        );
      })}
    </div>
  );
}

function AuditTab({
  userId,
  username,
}: {
  userId: number;
  username: string;
}) {
  const [entries, setEntries] = useState<AuditLogEntry[] | null>(null);
  const [total, setTotal] = useState(0);
  const [pageSize, setPageSize] = useState(20);
  const [error, setError] = useState<string | null>(null);
  // Wall-clock captured at fetch time, used for relative "5m ago"
  // labels. Stays 0 until after mount (set alongside the fetched
  // data) so SSR/client output matches — same convention as the
  // parent table's `nowMs`. Rows fall back to absolute time at 0.
  const [nowMs, setNowMs] = useState(0);

  useEffect(() => {
    let cancelled = false;
    // Intentional: reset to loading state when the user/page filter
    // changes so the skeleton reads correctly between fetches. The
    // alternative (showing the prior user's entries during the refetch)
    // would be confusing in the per-user audit drawer.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setEntries(null);
    setError(null);
    adminApi.audit
      .list({ actor_user_id: userId, limit: pageSize, offset: 0 })
      .then((res) => {
        if (cancelled) return;
        setEntries(res.entries);
        setTotal(res.total);
        setNowMs(Date.now());
      })
      .catch((e) => {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
        setEntries([]);
      });
    return () => {
      cancelled = true;
    };
  }, [userId, pageSize]);

  if (entries === null) {
    return <LoadingPlaceholder variant="inline" />;
  }
  if (error) {
    return (
      <p style={{ fontSize: 12.5, color: "var(--err)" }}>
        Couldn&rsquo;t load audit entries: {error}
      </p>
    );
  }
  if (entries.length === 0) {
    return (
      <p className="cf-muted" style={{ fontSize: 12.5 }}>
        No admin actions recorded for{" "}
        <span className="cf-mono">@{username}</span> yet.
      </p>
    );
  }

  return (
    <div>
      <div className="cf-faint" style={{ fontSize: 11, marginBottom: 6 }}>
        {total === 1 ? "1 action" : `${total.toLocaleString()} actions`}
        {entries.length < total && ` — showing latest ${entries.length}`}
      </div>
      <div style={{ borderTop: "1px solid var(--line-faint)" }}>
        {entries.map((e) => (
          <AuditRow key={e.id} entry={e} nowMs={nowMs} />
        ))}
      </div>
      {entries.length < total && (
        <button
          type="button"
          onClick={() => setPageSize((s) => s + 20)}
          className="cf-btn cf-sm"
          style={{ marginTop: 8, width: "100%", justifyContent: "center" }}
        >
          Load more
        </button>
      )}
      <p className="cf-faint" style={{ paddingTop: 8, fontSize: 11 }}>
        Full server-wide log at{" "}
        <a href="/settings/admin/logs?tab=audit">
          Maintenance → Logs → Audit trail
        </a>
        .
      </p>
    </div>
  );
}

function AuditRow({ entry, nowMs }: { entry: AuditLogEntry; nowMs: number }) {
  const when = new Date(entry.created_at);
  const hasTarget = entry.target_kind != null || entry.target_id != null;
  const target = hasTarget
    ? `${entry.target_kind ?? "—"}${
        entry.target_id != null ? ` #${entry.target_id}` : ""
      }`
    : null;
  // Relative time once the post-mount clock is available; until then
  // (and on the server) fall back to a compact absolute timestamp so
  // SSR and the first client render agree.
  const relTime =
    nowMs > 0
      ? formatRelative(nowMs - when.getTime())
      : when.toLocaleString(undefined, {
          month: "short",
          day: "numeric",
          hour: "2-digit",
          minute: "2-digit",
        });
  const absTime = when.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: 3,
        padding: "10px 0",
        borderBottom: "1px solid var(--line-faint)",
      }}
    >
      {/* Line 1: action (truncates) + target tag — never collides with time */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          minWidth: 0,
        }}
      >
        <span
          className="cf-mono"
          title={entry.action}
          style={{
            fontSize: 13,
            fontWeight: 500,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
            minWidth: 0,
            flex: 1,
          }}
        >
          {entry.action}
        </span>
        {target && (
          <span className="cf-tag cf-mono" title={target}>
            {target}
          </span>
        )}
      </div>
      {/* Line 2: faint meta — relative time · IP (IP omitted when absent) */}
      <div className="cf-faint" style={{ fontSize: 11 }}>
        <time dateTime={when.toISOString()} title={absTime}>
          {relTime}
        </time>
        {entry.ip && (
          <>
            {" · "}
            <span className="cf-mono">{entry.ip}</span>
          </>
        )}
      </div>
    </div>
  );
}

// ─── Helpers ────────────────────────────────────────────────────────

/// Map a failed admin API call to user-facing text. Prefers the
/// server's structured error message when present (validation surface,
/// e.g. "you can't demote the last owner"); otherwise picks a generic
/// synonym keyed off the HTTP status class so operators stop seeing
/// raw "HTTP 500" strings.
function friendlyError(e: unknown): string {
  if (e instanceof ChimpFlixApiError) {
    try {
      const parsed = JSON.parse(e.body) as { error?: { message?: string } };
      if (parsed.error?.message) return parsed.error.message;
    } catch {
      /* fall through */
    }
    if (e.status === 401 || e.status === 403)
      return "You don't have permission to do that.";
    if (e.status === 404) return "Not found.";
    if (e.status === 409) return "Conflicting state — try refreshing.";
    if (e.status >= 500) return "Server error. Try again in a moment.";
    return "Couldn't save. Try again.";
  }
  return "Network error. Check your connection and try again.";
}

function formatRelative(diffMs: number): string {
  if (diffMs < 60_000) return "now";
  if (diffMs < 3_600_000) return `${Math.floor(diffMs / 60_000)}m ago`;
  if (diffMs < 86_400_000) return `${Math.floor(diffMs / 3_600_000)}h ago`;
  return `${Math.floor(diffMs / 86_400_000)}d ago`;
}

function summarizeUserAgent(ua: string | null): string {
  if (!ua) return "Unknown device";
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
  if (browser && os) return `${browser} · ${os}`;
  if (browser) return browser;
  return ua.slice(0, 80);
}

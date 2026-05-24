"use client";

import { useEffect, useMemo, useState } from "react";
import {
  admin as adminApi,
  auth as authApi,
  ChimpFlixApiError,
  type AccessMatrixEntry,
  type AdminSessionSummary,
  type AuditLogEntry,
  type User,
  type UserRole,
} from "@/lib/chimpflix-api";
import {
  Drawer,
  DrawerBody,
  DrawerHeader,
  DrawerKV,
  DrawerSection,
  DrawerTabs,
  FilterChip,
  Pill,
  type DrawerTab,
  type PillTone,
} from "./ui";
import { ConfirmDialog } from "../ConfirmDialog";
import { LoadingPlaceholder } from "../ui/LoadingPlaceholder";

interface Props {
  currentUserId: number;
  currentUserRole: UserRole;
}

type FilterId = "all" | "owners" | "admins" | "online" | "locked";

const ROLE_TONE: Record<UserRole, PillTone> = {
  owner: "bad",
  admin: "info",
  user: "muted",
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

/// Mock 3 — one Users page that holds the table + a sticky tabbed
/// drawer (Profile / Access / Devices / Audit). Replaces the previous
/// SettingsUsersClient long-row layout and folds in what used to live
/// at /users/devices and /users/access for individual users.
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
    // "Locked" is a placeholder until we surface lockout status —
    // counted as 0 today but kept so the filter chip is visible and
    // can light up once the backend exposes the field.
    const locked = 0;
    return { all, owners, admins, online, locked };
  }, [users, sessionsByUser]);

  const filtered = useMemo(() => {
    if (!users) return [];
    let list = users;
    if (filter === "owners") list = list.filter((u) => u.role === "owner");
    else if (filter === "admins") list = list.filter((u) => u.role === "admin");
    else if (filter === "online") {
      list = list.filter((u) => (sessionsByUser.get(u.id)?.length ?? 0) > 0);
    }
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
      <p className="text-sm text-white/60">{message ?? "No users yet."}</p>
    );
  }

  // Only reserve the drawer slot when something is actually selected
  // — otherwise the table column gets squeezed (clipping the Status
  // column at typical viewport widths) and the filter chips wrap to a
  // second row. Drawer width 380px so the table has at least ~520px
  // even on 1280-wide laptops after the page chrome eats its share.
  const gridClass = selected
    ? "grid grid-cols-1 gap-4 lg:grid-cols-[1fr_380px]"
    : "grid grid-cols-1 gap-4";

  return (
    <div className={gridClass}>
      <div className="min-w-0">
        <FilterBar
          filter={filter}
          counts={counts}
          query={query}
          onFilter={setFilter}
          onQuery={setQuery}
        />
        <div className="overflow-hidden rounded-b-lg border border-t-0 border-white/10 bg-white/2">
          <table className="w-full table-fixed text-sm">
            <thead className="bg-white/4 text-left text-[11.5px] uppercase tracking-wider text-white/45">
              <tr>
                <th className="px-3 py-2 font-semibold">User</th>
                <th className="w-24 px-3 py-2 font-semibold whitespace-nowrap">Role</th>
                <th className="w-28 px-3 py-2 font-semibold whitespace-nowrap">Status</th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((u) => (
                <UserRow
                  key={u.id}
                  user={u}
                  selected={selectedId === u.id}
                  currentUserId={currentUserId}
                  liveSessions={sessionsByUser.get(u.id) ?? []}
                  accessCount={accessByUser.get(u.id)?.filter((e) => e.allowed).length ?? 0}
                  nowMs={nowMs}
                  onSelect={() => setSelectedId(u.id)}
                />
              ))}
            </tbody>
          </table>
          {filtered.length === 0 && (
            <div className="px-4 py-8 text-center text-sm text-white/45">
              No users match this filter.
            </div>
          )}
        </div>
        {message && <p className="mt-3 text-xs text-white/70">{message}</p>}
      </div>

      {selected && (
        <UserDrawer
          key={selected.id}
          user={selected}
          currentUserId={currentUserId}
          currentUserRole={currentUserRole}
          sessions={sessionsByUser.get(selected.id) ?? []}
          access={accessByUser.get(selected.id) ?? []}
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
  );
}

// ─── Filter bar + table row ─────────────────────────────────────────

function FilterBar({
  filter,
  counts,
  query,
  onFilter,
  onQuery,
}: {
  filter: FilterId;
  counts: Record<FilterId, number>;
  query: string;
  onFilter: (f: FilterId) => void;
  onQuery: (q: string) => void;
}) {
  const chips: Array<{ id: FilterId; label: string }> = [
    { id: "all", label: "All" },
    { id: "owners", label: "Owners" },
    { id: "admins", label: "Admins" },
    { id: "online", label: "Online" },
    { id: "locked", label: "Locked" },
  ];
  return (
    <div className="flex flex-wrap items-center gap-2 rounded-t-lg border border-white/10 bg-white/2 p-3">
      <input
        value={query}
        onChange={(e) => onQuery(e.target.value)}
        placeholder="Search by name or email…"
        className="h-8 max-w-70 flex-1 rounded-md border border-white/10 bg-black/30 px-3 text-[13px] outline-none focus:border-white/30"
      />
      {chips.map((c) => (
        <FilterChip
          key={c.id}
          active={filter === c.id}
          count={counts[c.id]}
          onClick={() => onFilter(c.id)}
        >
          {c.label}
        </FilterChip>
      ))}
    </div>
  );
}

function UserRow({
  user,
  selected,
  currentUserId,
  liveSessions,
  accessCount,
  nowMs,
  onSelect,
}: {
  user: User;
  selected: boolean;
  currentUserId: number;
  liveSessions: AdminSessionSummary[];
  accessCount: number;
  nowMs: number;
  onSelect: () => void;
}) {
  const online = liveSessions.length > 0;
  const lastSeen = online
    ? "now"
    : user.last_login_at && nowMs > 0
      ? formatRelative(nowMs - user.last_login_at)
      : user.last_login_at
        ? new Date(user.last_login_at).toLocaleDateString()
        : "never";
  const subtitleBits: string[] = [];
  if (user.email) subtitleBits.push(user.email);
  subtitleBits.push(
    `${accessCount} ${accessCount === 1 ? "library" : "libraries"}`,
  );
  subtitleBits.push(`last ${lastSeen}`);
  return (
    <tr
      onClick={onSelect}
      className={`cursor-pointer border-t border-white/6 ${selected ? "bg-accent/6" : "hover:bg-white/2.5"}`}
    >
      <td className="px-3 py-3">
        <div className="flex items-center gap-3">
          <Avatar name={user.display_name ?? user.username} />
          <div className="min-w-0">
            <div className="truncate text-[13.5px] font-medium">
              {user.display_name ?? user.username}
              {user.id === currentUserId && (
                <span className="ml-1.5 text-[11px] text-white/40">(you)</span>
              )}
            </div>
            <div className="truncate text-[11.5px] text-white/55">
              @{user.username} · {subtitleBits.join(" · ")}
            </div>
          </div>
        </div>
      </td>
      <td className="whitespace-nowrap px-3 py-3">
        <Pill tone={ROLE_TONE[user.role]}>{ROLE_LABEL[user.role]}</Pill>
      </td>
      <td className="whitespace-nowrap px-3 py-3">
        {online ? (
          <Pill tone="ok" dot>
            online
          </Pill>
        ) : (
          <Pill tone="muted" dot>
            offline
          </Pill>
        )}
      </td>
    </tr>
  );
}

function Avatar({ name }: { name: string }) {
  const initials = name
    .split(/\s+/)
    .map((s) => s[0]?.toUpperCase() ?? "")
    .join("")
    .slice(0, 2);
  // Six-tone palette keyed by name so the same user always gets the
  // same color across the table and drawer header. Hash is the sum of
  // char codes mod 6 — fine for visual variety, not security.
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h + name.charCodeAt(i)) % 6;
  const palette = [
    "from-indigo-500 to-fuchsia-500",
    "from-amber-500 to-red-500",
    "from-emerald-500 to-cyan-500",
    "from-pink-500 to-rose-500",
    "from-violet-500 to-pink-500",
    "from-cyan-500 to-indigo-500",
  ];
  return (
    <span
      className={`grid h-8 w-8 shrink-0 place-items-center rounded-full bg-linear-to-br ${palette[h]} text-[11px] font-bold text-white`}
    >
      {initials || "?"}
    </span>
  );
}

// ─── Drawer ─────────────────────────────────────────────────────────

const DRAWER_TABS: DrawerTab[] = [
  { id: "profile", label: "Profile" },
  { id: "access", label: "Access" },
  { id: "devices", label: "Devices" },
  { id: "audit", label: "Audit" },
];

function UserDrawer({
  user,
  currentUserId,
  currentUserRole,
  sessions,
  access,
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
  onClose: () => void;
  onChanged: () => void;
  onDeleted: () => void;
  onMessage: (msg: string | null) => void;
}) {
  const [tab, setTab] = useState("profile");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Confirmation gating — boolean per action since each fires against
  // the single `user` this drawer is rendered for.
  const [askSendReset, setAskSendReset] = useState(false);
  const [askResetTwoFactor, setAskResetTwoFactor] = useState(false);
  const [askKickSessions, setAskKickSessions] = useState(false);
  const [askDeleteUser, setAskDeleteUser] = useState(false);
  const sessionCount = sessions.length;
  const canActOnTarget =
    user.id !== currentUserId && tier(currentUserRole) >= tier(user.role);

  // Drawer tabs with live counts so the Devices tab shows "Devices 3"
  // when the user has 3 active sessions, etc.
  const tabsWithCounts: DrawerTab[] = DRAWER_TABS.map((t) => {
    if (t.id === "devices" && sessionCount > 0) {
      return { ...t, count: sessionCount };
    }
    if (t.id === "access" && access.length > 0) {
      return { ...t, count: access.filter((e) => e.allowed).length };
    }
    return t;
  });

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
      onMessage(`Removed "${user.display_name ?? user.username}".`);
      onDeleted();
    } catch {
      /* error surfaced */
    }
  }

  return (
    <Drawer>
      <DrawerHeader onClose={onClose}>
        <div className="mb-2 flex items-center gap-3">
          <Avatar name={user.display_name ?? user.username} />
          <div className="min-w-0">
            <div className="truncate text-base font-bold tracking-tight">
              {user.display_name ?? user.username}
            </div>
            <div className="truncate text-[11.5px] text-white/55">
              {user.email ?? `@${user.username}`}
            </div>
          </div>
        </div>
        <div className="flex flex-wrap gap-1.5">
          <Pill tone={ROLE_TONE[user.role]}>{ROLE_LABEL[user.role]}</Pill>
          {sessionCount > 0 ? (
            <Pill tone="ok" dot>
              online
            </Pill>
          ) : (
            <Pill tone="muted" dot>
              offline
            </Pill>
          )}
          {user.id === currentUserId && (
            <Pill tone="muted">it&apos;s you</Pill>
          )}
        </div>
      </DrawerHeader>

      <DrawerTabs tabs={tabsWithCounts} activeId={tab} onSelect={setTab} />

      <DrawerBody>
        {error && (
          <div className="mb-3 rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
            {error}
          </div>
        )}

        {tab === "profile" && (
          <ProfileTab
            user={user}
            sessionCount={sessionCount}
            canActOnTarget={canActOnTarget}
            currentUserRole={currentUserRole}
            busy={busy}
            onSetRole={setRole}
            onSendReset={sendPasswordReset}
            onReset2FA={resetTwoFactor}
            onKickSessions={kickSessions}
            onDelete={deleteUser}
          />
        )}

        {tab === "access" && <AccessTab entries={access} />}

        {tab === "devices" && (
          <DevicesTab
            sessions={sessions}
            busy={busy}
            onRevoke={revokeSession}
          />
        )}

        {tab === "audit" && (
          <AuditTab userId={user.id} username={user.username} />
        )}
      </DrawerBody>
      {askSendReset && (
        <ConfirmDialog
          title={`Send password reset to "${user.display_name ?? user.username}"?`}
          body="A reset link is emailed to the address on file. The link expires in 1 hour and can only be used once."
          confirmLabel="Send"
          busy={busy === "reset"}
          onConfirm={() => void confirmSendPasswordReset()}
          onCancel={() => setAskSendReset(false)}
        />
      )}
      {askResetTwoFactor && (
        <ConfirmDialog
          title={`Reset 2FA for "${user.display_name ?? user.username}"?`}
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
          body={`"${user.display_name ?? user.username}" will be signed out of every device. Anything currently playing stops at the next heartbeat.`}
          confirmLabel="Revoke sessions"
          destructive
          busy={busy === "kick"}
          onConfirm={() => void confirmKickSessions()}
          onCancel={() => setAskKickSessions(false)}
        />
      )}
      {askDeleteUser && (
        <ConfirmDialog
          title={`Delete user "${user.display_name ?? user.username}"?`}
          body="Their sessions, watch state, and personal lists will be removed. Library files on disk are untouched."
          confirmLabel="Delete user"
          destructive
          busy={busy === "delete"}
          onConfirm={() => void confirmDeleteUser()}
          onCancel={() => setAskDeleteUser(false)}
        />
      )}
    </Drawer>
  );
}

// ─── Drawer tabs ────────────────────────────────────────────────────

function ProfileTab({
  user,
  sessionCount,
  canActOnTarget,
  currentUserRole,
  busy,
  onSetRole,
  onSendReset,
  onReset2FA,
  onKickSessions,
  onDelete,
}: {
  user: User;
  sessionCount: number;
  canActOnTarget: boolean;
  currentUserRole: UserRole;
  busy: string | null;
  onSetRole: (role: UserRole) => void;
  onSendReset: () => void;
  onReset2FA: () => void;
  onKickSessions: () => void;
  onDelete: () => void;
}) {
  const lastLogin = user.last_login_at
    ? `${new Date(user.last_login_at).toLocaleString()}${user.last_login_ip ? ` · ${user.last_login_ip}` : ""}`
    : "never";
  const previousLogin = user.previous_login_at
    ? `${new Date(user.previous_login_at).toLocaleString()}${user.previous_login_ip ? ` · ${user.previous_login_ip}` : ""}`
    : "—";

  return (
    <>
      <DrawerKV
        rows={[
          { label: "Display name", value: user.display_name ?? "—" },
          { label: "Email", value: user.email ?? "—" },
          {
            label: "Joined",
            value: new Date(user.created_at).toLocaleDateString(),
          },
          { label: "Last login", value: lastLogin },
          { label: "Previous login", value: previousLogin },
          {
            label: "Active sessions",
            value:
              sessionCount === 0
                ? "0"
                : `${sessionCount} (see Devices tab)`,
          },
          {
            label: "Email-mirror",
            value: user.notify_via_email ? "on" : "off",
          },
        ]}
      />

      {canActOnTarget && (
        <DrawerSection title="Quick actions">
          <div className="grid grid-cols-2 gap-2">
            <DrawerAction
              label="Send reset"
              busy={busy === "reset"}
              onClick={onSendReset}
            />
            <DrawerAction
              label="Reset 2FA"
              busy={busy === "2fa"}
              onClick={onReset2FA}
            />
            <DrawerAction
              label="Kick sessions"
              busy={busy === "kick"}
              disabled={sessionCount === 0}
              onClick={onKickSessions}
            />
            <DrawerAction
              label="Change role…"
              busy={busy === "role"}
              onClick={() => {
                // No menu primitive yet — surface via window.prompt
                // until we build a dropdown. Backend rejects invalid
                // transitions, so a typo just shows an error.
                const next = window.prompt(
                  `New role for "${user.display_name ?? user.username}" (owner / admin / user):`,
                  user.role,
                );
                if (next && (next === "owner" || next === "admin" || next === "user")) {
                  if (next !== user.role) onSetRole(next);
                }
              }}
            />
          </div>
          <p className="mt-2 text-[11px] text-white/45">
            Role hierarchy:{" "}
            <code className="font-mono">
              {currentUserRole === "owner"
                ? "owner can manage anyone"
                : "admin can manage user + admin (not owners)"}
            </code>
          </p>
        </DrawerSection>
      )}

      {canActOnTarget && (
        <DrawerSection title="Danger zone">
          <button
            type="button"
            onClick={onDelete}
            disabled={busy === "delete"}
            className="w-full rounded-md border border-red-500/30 bg-red-500/10 px-3 py-1.5 text-[12px] font-medium text-red-300 hover:bg-red-500/20 disabled:opacity-50"
          >
            {busy === "delete" ? "Deleting…" : "Delete user…"}
          </button>
        </DrawerSection>
      )}
    </>
  );
}

function AccessTab({ entries }: { entries: AccessMatrixEntry[] }) {
  const allowed = entries.filter((e) => e.allowed || e.via_groups.length > 0);
  if (allowed.length === 0) {
    return (
      <p className="text-[12.5px] text-white/55">
        No library access. Grant some by editing the access matrix at{" "}
        <a
          href="/settings/admin/users/access"
          className="text-white/80 underline hover:text-white"
        >
          Users → Access
        </a>
        .
      </p>
    );
  }
  return (
    <>
      <p className="mb-3 text-[11.5px] text-white/55">
        Edit grants under{" "}
        <a
          href="/settings/admin/users/access"
          className="text-white/80 underline hover:text-white"
        >
          Users → Access
        </a>
        . This view is read-only.
      </p>
      <div className="rounded-md border border-white/10 bg-white/2">
        {allowed.map((e, i) => {
          const direct = e.allowed;
          const groups = e.via_groups;
          return (
            <div
              key={e.library_id}
              className={`flex items-center justify-between gap-2 px-3 py-2 text-[12.5px] ${i === 0 ? "" : "border-t border-white/6"}`}
            >
              <span className="min-w-0 truncate">{e.library_name}</span>
              <div className="flex flex-wrap items-center gap-1.5">
                {direct && <Pill>direct</Pill>}
                {groups.map((g) => (
                  <Pill key={g} tone="ok">
                    via {g}
                  </Pill>
                ))}
              </div>
            </div>
          );
        })}
      </div>
    </>
  );
}

function DevicesTab({
  sessions,
  busy,
  onRevoke,
}: {
  sessions: AdminSessionSummary[];
  busy: string | null;
  onRevoke: (id: number) => void;
}) {
  if (sessions.length === 0) {
    return (
      <p className="text-[12.5px] text-white/55">
        No active sessions for this user.
      </p>
    );
  }
  return (
    <div className="space-y-2">
      {sessions.map((s) => (
        <div
          key={s.id}
          className="rounded-md border border-white/10 bg-white/2 p-3"
        >
          <div className="flex items-start justify-between gap-2">
            <div className="min-w-0">
              <div className="truncate text-[12.5px] font-medium">
                {summarizeUserAgent(s.user_agent)}
              </div>
              <div className="mt-0.5 text-[11px] text-white/45">
                {s.ip ?? "—"} · last seen{" "}
                {new Date(s.last_seen_at).toLocaleString()}
              </div>
              <div className="text-[11px] text-white/40">
                expires {new Date(s.expires_at).toLocaleDateString()}
              </div>
            </div>
            <button
              type="button"
              onClick={() => onRevoke(s.id)}
              disabled={busy === `session-${s.id}`}
              className="shrink-0 rounded border border-white/15 px-2 py-1 text-[11px] text-white/75 hover:border-red-500/50 hover:text-red-300 disabled:opacity-50"
            >
              Revoke
            </button>
          </div>
        </div>
      ))}
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
      <p className="text-[12.5px] text-red-300">
        Couldn&rsquo;t load audit entries: {error}
      </p>
    );
  }
  if (entries.length === 0) {
    return (
      <p className="text-[12.5px] text-white/55">
        No admin actions recorded for{" "}
        <code className="font-mono text-white/75">@{username}</code> yet.
      </p>
    );
  }

  return (
    <div className="space-y-2">
      <div className="text-[11px] text-white/45">
        {total === 1 ? "1 action" : `${total.toLocaleString()} actions`}
        {entries.length < total && ` — showing latest ${entries.length}`}
      </div>
      <ol className="space-y-2">
        {entries.map((e) => (
          <AuditRow key={e.id} entry={e} />
        ))}
      </ol>
      {entries.length < total && (
        <button
          type="button"
          onClick={() => setPageSize((s) => s + 20)}
          className="mt-2 w-full rounded border border-white/10 bg-white/2 px-2 py-1.5 text-[11.5px] text-white/65 hover:border-white/20 hover:text-white"
        >
          Load more
        </button>
      )}
      <p className="pt-1 text-[11px] text-white/40">
        Full server-wide log at{" "}
        <a
          href="/settings/admin/maintenance/logs/audit"
          className="text-white/55 underline hover:text-white"
        >
          Maintenance → Logs → Audit trail
        </a>
        .
      </p>
    </div>
  );
}

function AuditRow({ entry }: { entry: AuditLogEntry }) {
  const when = new Date(entry.created_at);
  return (
    <li className="rounded-md border border-white/10 bg-white/2 p-2.5">
      <div className="flex items-baseline justify-between gap-2">
        <code className="truncate font-mono text-[11.5px] text-white/85">
          {entry.action}
        </code>
        <time
          className="shrink-0 text-[10.5px] text-white/45"
          dateTime={when.toISOString()}
          title={when.toLocaleString()}
        >
          {when.toLocaleString(undefined, {
            month: "short",
            day: "numeric",
            hour: "2-digit",
            minute: "2-digit",
          })}
        </time>
      </div>
      {(entry.target_kind || entry.target_id) && (
        <div className="mt-0.5 text-[11px] text-white/55">
          {entry.target_kind ?? "—"}
          {entry.target_id != null && (
            <span className="text-white/35"> #{entry.target_id}</span>
          )}
        </div>
      )}
      {entry.ip && (
        <div className="mt-0.5 font-mono text-[10.5px] text-white/40">
          {entry.ip}
        </div>
      )}
    </li>
  );
}

function DrawerAction({
  label,
  busy,
  disabled,
  onClick,
}: {
  label: string;
  busy?: boolean;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={busy || disabled}
      className="rounded-md border border-white/15 bg-white/4 px-3 py-1.5 text-center text-[12px] font-medium text-white/85 hover:bg-white/8 disabled:cursor-not-allowed disabled:opacity-50"
    >
      {busy ? "…" : label}
    </button>
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
  if (diffMs < 60_000) return "just now";
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
  if (browser && os) return `${browser} on ${os}`;
  if (browser) return browser;
  return ua.slice(0, 80);
}

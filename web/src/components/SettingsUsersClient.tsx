"use client";

import { useEffect, useState } from "react";
import {
  admin as adminApi,
  auth as authApi,
  ChimpFlixApiError,
  type User,
  type UserRole,
} from "@/lib/chimpflix-api";

interface Props {
  currentUserId: number;
  /// The viewer's role. Drives per-row button visibility — admins
  /// see the "Promote to admin / Demote to user" pair but not the
  /// "Promote to owner" button (owner-only). Buttons against owner
  /// targets are hidden entirely for admin viewers.
  currentUserRole: UserRole;
}

// Hierarchy tier mirror of the backend's `UserRole::tier`. Used to
// decide whether the viewer is allowed to act on a target (must
// hold a tier >= target's tier).
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

function roleLabel(role: UserRole): string {
  switch (role) {
    case "owner":
      return "Owner";
    case "admin":
      return "Admin";
    case "user":
      return "User";
  }
}

function roleColor(role: UserRole): string {
  switch (role) {
    case "owner":
      return "text-accent";
    case "admin":
      return "text-amber-300";
    case "user":
      return "text-white/55";
  }
}

export function SettingsUsersClient({
  currentUserId,
  currentUserRole,
}: Props) {
  const [users, setUsers] = useState<User[] | null>(null);
  const [busy, setBusy] = useState<number | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    refresh();
  }, []);

  async function refresh() {
    try {
      const { users } = await authApi.listUsers();
      setUsers(users);
    } catch (e) {
      if (e instanceof ChimpFlixApiError && e.status === 403) {
        setMessage("Only owners and admins can manage users.");
      } else {
        setMessage("Failed to load users.");
      }
      setUsers([]);
    }
  }

  async function setRole(id: number, role: UserRole) {
    setBusy(id);
    setMessage(null);
    try {
      await authApi.setUserRole(id, role);
      await refresh();
    } catch (e) {
      if (e instanceof ChimpFlixApiError) {
        try {
          const parsed = JSON.parse(e.body) as { error?: { message?: string } };
          setMessage(`Failed: ${parsed.error?.message ?? `HTTP ${e.status}`}`);
        } catch {
          setMessage(`Failed: HTTP ${e.status}`);
        }
      } else {
        setMessage("Failed: network error");
      }
    } finally {
      setBusy(null);
    }
  }

  async function resetTwoFactor(id: number, label: string) {
    if (
      !window.confirm(
        `Reset 2FA for "${label}"? They'll be able to log in with just their password until they re-enroll.`,
      )
    ) {
      return;
    }
    setBusy(id);
    setMessage(null);
    try {
      await adminApi.resetUserTwoFactor(id);
      setMessage(`2FA reset for "${label}".`);
    } catch (e) {
      if (e instanceof ChimpFlixApiError) {
        if (e.status === 404) {
          setMessage(`"${label}" had no 2FA enrolled.`);
        } else {
          try {
            const parsed = JSON.parse(e.body) as {
              error?: { message?: string };
            };
            setMessage(`Failed: ${parsed.error?.message ?? `HTTP ${e.status}`}`);
          } catch {
            setMessage(`Failed: HTTP ${e.status}`);
          }
        }
      } else {
        setMessage("Failed: network error");
      }
    } finally {
      setBusy(null);
    }
  }

  async function sendPasswordReset(id: number, label: string) {
    if (
      !window.confirm(
        `Send a password reset email to "${label}"? The link expires in 1 hour and can only be used once.`,
      )
    ) {
      return;
    }
    setBusy(id);
    setMessage(null);
    try {
      // Returns `{ ok, message }` — `ok=false` is a valid response when
      // the user has no email on file or SMTP isn't configured. We
      // surface whichever message the server returned, no try/catch
      // distinction needed for the soft-fail cases.
      const r = await adminApi.sendUserPasswordReset(id);
      setMessage(r.message);
    } catch (e) {
      if (e instanceof ChimpFlixApiError) {
        try {
          const parsed = JSON.parse(e.body) as { error?: { message?: string } };
          setMessage(`Failed: ${parsed.error?.message ?? `HTTP ${e.status}`}`);
        } catch {
          setMessage(`Failed: HTTP ${e.status}`);
        }
      } else {
        setMessage("Failed: network error");
      }
    } finally {
      setBusy(null);
    }
  }

  async function remove(id: number, label: string) {
    if (
      !window.confirm(
        `Delete user "${label}"? Their sessions and watch state will be removed.`,
      )
    ) {
      return;
    }
    setBusy(id);
    setMessage(null);
    try {
      await authApi.deleteUser(id);
      setMessage(`Removed "${label}".`);
      await refresh();
    } catch (e) {
      if (e instanceof ChimpFlixApiError) {
        try {
          const parsed = JSON.parse(e.body) as { error?: { message?: string } };
          setMessage(`Failed: ${parsed.error?.message ?? `HTTP ${e.status}`}`);
        } catch {
          setMessage(`Failed: HTTP ${e.status}`);
        }
      } else {
        setMessage("Failed: network error");
      }
    } finally {
      setBusy(null);
    }
  }

  if (users === null) return <p className="text-sm text-white/60">Loading…</p>;

  if (users.length === 0) {
    return (
      <p className="text-sm text-white/60">
        {message ?? "No users yet."}
      </p>
    );
  }

  return (
    <div>
      <ul className="divide-y divide-white/5">
        {users.map((u) => (
          <li
            key={u.id}
            className="flex items-center justify-between gap-4 py-3 text-sm"
          >
            <div>
              <div className="font-medium">
                {u.display_name ?? u.username}
              </div>
              <div className="text-xs text-white/55">
                @{u.username} ·{" "}
                <span className={roleColor(u.role)}>{roleLabel(u.role)}</span>
                {u.id === currentUserId && (
                  <span className="ml-1 text-white/40">(you)</span>
                )}
              </div>
              <div className="mt-0.5 text-xs text-white/40">
                Joined {new Date(u.created_at).toLocaleDateString()}
              </div>
            </div>
            {u.id !== currentUserId && tier(currentUserRole) >= tier(u.role) && (
              <div className="flex flex-wrap justify-end gap-2">
                {/* Role mutation buttons. Admins see promote-to-admin /
                    demote-to-user; only owners see promote/demote
                    against the owner tier. The backend rejects any
                    request that violates the hierarchy, so even if a
                    user crafts a request directly they get a 403. */}
                {u.role === "user" && (
                  <button
                    type="button"
                    onClick={() => setRole(u.id, "admin")}
                    disabled={busy === u.id}
                    title="Grant this user delegated admin powers (manage users + other admins, but not owners)."
                    className="rounded bg-white/10 px-3 py-1.5 text-xs font-medium transition hover:bg-white/15 disabled:opacity-50"
                  >
                    Promote to admin
                  </button>
                )}
                {u.role === "admin" && (
                  <button
                    type="button"
                    onClick={() => setRole(u.id, "user")}
                    disabled={busy === u.id}
                    title="Revoke admin powers — they remain a regular viewer."
                    className="rounded bg-white/10 px-3 py-1.5 text-xs font-medium transition hover:bg-white/15 disabled:opacity-50"
                  >
                    Demote to user
                  </button>
                )}
                {currentUserRole === "owner" && u.role !== "owner" && (
                  <button
                    type="button"
                    onClick={() => setRole(u.id, "owner")}
                    disabled={busy === u.id}
                    title="Grant full ownership — they'll be able to do everything you can, including managing other owners."
                    className="rounded bg-white/10 px-3 py-1.5 text-xs font-medium transition hover:bg-white/15 disabled:opacity-50"
                  >
                    Promote to owner
                  </button>
                )}
                {currentUserRole === "owner" && u.role === "owner" && (
                  <button
                    type="button"
                    onClick={() => setRole(u.id, "admin")}
                    disabled={busy === u.id}
                    title="Demote this owner to admin. Rejected if this is the last owner."
                    className="rounded bg-white/10 px-3 py-1.5 text-xs font-medium transition hover:bg-white/15 disabled:opacity-50"
                  >
                    Demote to admin
                  </button>
                )}
                <button
                  type="button"
                  onClick={() =>
                    sendPasswordReset(u.id, u.display_name ?? u.username)
                  }
                  disabled={busy === u.id}
                  title="Email this user a single-use link to choose a new password. Token expires in 1 hour."
                  className="rounded bg-white/10 px-3 py-1.5 text-xs font-medium transition hover:bg-white/15 disabled:opacity-50"
                >
                  Send password reset
                </button>
                <button
                  type="button"
                  onClick={() =>
                    resetTwoFactor(u.id, u.display_name ?? u.username)
                  }
                  disabled={busy === u.id}
                  title="Wipe this user's TOTP enrollment + recovery codes. Use when they've lost their authenticator device."
                  className="rounded bg-amber-500/15 px-3 py-1.5 text-xs font-medium text-amber-200 transition hover:bg-amber-500/25 disabled:opacity-50"
                >
                  Reset 2FA
                </button>
                {/* Delete is allowed against same-or-lower tier targets;
                    owner-tier targets only by another owner. The
                    backend's last-owner guard prevents deleting the
                    final owner regardless. */}
                {(currentUserRole === "owner" || u.role !== "owner") && (
                  <button
                    type="button"
                    onClick={() =>
                      remove(u.id, u.display_name ?? u.username)
                    }
                    disabled={busy === u.id}
                    className="rounded bg-red-500/15 px-3 py-1.5 text-xs font-medium text-red-200 transition hover:bg-red-500/25 disabled:opacity-50"
                  >
                    {busy === u.id ? "…" : "Delete"}
                  </button>
                )}
              </div>
            )}
          </li>
        ))}
      </ul>
      {message && <p className="mt-3 text-xs text-white/70">{message}</p>}
    </div>
  );
}

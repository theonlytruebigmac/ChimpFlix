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
}

export function SettingsUsersClient({ currentUserId }: Props) {
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
        setMessage("Only the owner can manage users.");
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
                <span
                  className={
                    u.role === "owner"
                      ? "text-(--color-accent)"
                      : "text-white/55"
                  }
                >
                  {u.role === "owner" ? "Owner" : "User"}
                </span>
                {u.id === currentUserId && (
                  <span className="ml-1 text-white/40">(you)</span>
                )}
              </div>
              <div className="mt-0.5 text-xs text-white/40">
                Joined {new Date(u.created_at).toLocaleDateString()}
              </div>
            </div>
            {u.id !== currentUserId && (
              <div className="flex flex-wrap justify-end gap-2">
                <button
                  type="button"
                  onClick={() =>
                    setRole(u.id, u.role === "owner" ? "user" : "owner")
                  }
                  disabled={busy === u.id}
                  className="rounded bg-white/10 px-3 py-1.5 text-xs font-medium transition hover:bg-white/15 disabled:opacity-50"
                >
                  {u.role === "owner" ? "Demote to user" : "Promote to owner"}
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
                {u.role !== "owner" && (
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

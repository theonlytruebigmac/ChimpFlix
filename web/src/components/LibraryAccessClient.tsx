"use client";

import { useEffect, useState } from "react";
import {
  auth as authApi,
  libraries as librariesApi,
  type User,
} from "@/lib/chimpflix-api";

interface Props {
  libraryId: number;
  libraryName: string;
  onClose: () => void;
}

export function LibraryAccessClient({ libraryId, libraryName, onClose }: Props) {
  const [allUsers, setAllUsers] = useState<User[] | null>(null);
  const [selected, setSelected] = useState<Set<number> | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    Promise.all([authApi.listUsers(), librariesApi.getAccess(libraryId)])
      .then(([{ users }, access]) => {
        if (cancelled) return;
        setAllUsers(users);
        setSelected(new Set(access.user_ids));
      })
      .catch(() => {
        if (cancelled) return;
        setAllUsers([]);
        setSelected(new Set());
        setMessage("Failed to load access list.");
      });
    return () => {
      cancelled = true;
    };
  }, [libraryId]);

  async function toggle(id: number) {
    if (!selected) return;
    const next = new Set(selected);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setSelected(next);
    setBusy(true);
    setMessage(null);
    try {
      await librariesApi.setAccess(libraryId, [...next]);
    } catch {
      setMessage("Failed to save.");
      setSelected(selected);
    } finally {
      setBusy(false);
    }
  }

  if (!allUsers || !selected) {
    return (
      <div className="mt-3 rounded-md border border-white/10 bg-white/2 p-4 text-sm text-white/55">
        Loading…
      </div>
    );
  }

  return (
    <div className="mt-3 rounded-md border border-white/10 bg-white/2 p-4">
      <div className="mb-3 flex items-center justify-between">
        <h3 className="text-sm font-semibold">
          Access to &ldquo;{libraryName}&rdquo;
        </h3>
        <button
          type="button"
          onClick={onClose}
          className="text-xs text-white/55 hover:text-white"
        >
          Close
        </button>
      </div>
      <p className="mb-3 text-xs text-white/55">
        Pick which non-owner users can browse this library. Owners always
        have access.
      </p>
      <ul className="space-y-1">
        {allUsers.map((u) => {
          const ownerLocked = u.role === "owner";
          const isChecked = selected.has(u.id) || ownerLocked;
          return (
            <li key={u.id}>
              <label className="flex cursor-pointer items-center gap-3 rounded p-2 text-sm hover:bg-white/5">
                <input
                  type="checkbox"
                  checked={isChecked}
                  disabled={busy || ownerLocked}
                  onChange={() => toggle(u.id)}
                  className="h-4 w-4 accent-(--color-accent) disabled:opacity-60"
                />
                <span className={ownerLocked ? "text-white" : "text-white"}>
                  {u.display_name ?? u.username}
                </span>
                <span className="text-xs text-white/45">
                  @{u.username}
                  {ownerLocked && " · owner (always)"}
                </span>
              </label>
            </li>
          );
        })}
      </ul>
      {message && <p className="mt-2 text-xs text-red-300">{message}</p>}
    </div>
  );
}

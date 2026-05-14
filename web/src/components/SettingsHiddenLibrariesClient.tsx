"use client";

import { useEffect, useState } from "react";
import {
  prefs as prefsApi,
  type Library,
} from "@/lib/chimpflix-api";

interface Props {
  libraries: Library[];
}

export function SettingsHiddenLibrariesClient({ libraries }: Props) {
  const [hidden, setHidden] = useState<Set<number> | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    prefsApi
      .hiddenLibraries()
      .then((res) => {
        if (!cancelled) setHidden(new Set(res.library_ids));
      })
      .catch(() => {
        if (!cancelled) setHidden(new Set());
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function toggle(id: number) {
    if (!hidden) return;
    const next = new Set(hidden);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setHidden(next);
    setBusy(true);
    setMessage(null);
    try {
      await prefsApi.setHiddenLibraries([...next]);
    } catch {
      setMessage("Failed to save. Try again.");
      setHidden(hidden);
    } finally {
      setBusy(false);
    }
  }

  if (hidden === null) {
    return <p className="text-sm text-white/60">Loading…</p>;
  }

  if (libraries.length === 0) {
    return (
      <p className="text-sm text-white/60">
        Add a library first to choose which ones to hide.
      </p>
    );
  }

  return (
    <div>
      <p className="mb-3 text-xs text-white/55">
        Hidden libraries are excluded from your home page and browse rails.
      </p>
      <ul className="space-y-2">
        {libraries.map((lib) => {
          const isHidden = hidden.has(lib.id);
          return (
            <li key={lib.id}>
              <label className="flex cursor-pointer items-center gap-3 rounded p-2 text-sm hover:bg-white/5">
                <input
                  type="checkbox"
                  checked={isHidden}
                  onChange={() => toggle(lib.id)}
                  disabled={busy}
                  className="h-4 w-4 accent-(--color-accent)"
                />
                <span className={isHidden ? "text-white/60" : "text-white"}>
                  {lib.name}
                </span>
                <span className="text-xs text-white/45">
                  ({lib.kind === "movies" ? "Movies" : "Shows"})
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

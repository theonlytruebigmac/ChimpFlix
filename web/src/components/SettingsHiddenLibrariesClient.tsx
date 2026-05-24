"use client";

import { useEffect, useRef, useState } from "react";
import {
  prefs as prefsApi,
  type Library,
} from "@/lib/chimpflix-api";
import { LoadingPlaceholder } from "./ui/LoadingPlaceholder";
import { SettingsFeedback } from "./ui/SettingsFeedback";

interface Props {
  libraries: Library[];
}

export function SettingsHiddenLibrariesClient({ libraries }: Props) {
  const [hidden, setHidden] = useState<Set<number> | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Auto-clear the "Saved" pill after 2.5s so it doesn't linger on the
  // page indefinitely after the last toggle. Errors stay until the
  // next toggle so the user can read them.
  const okTimerRef = useRef<number | null>(null);
  useEffect(() => {
    return () => {
      if (okTimerRef.current !== null) {
        window.clearTimeout(okTimerRef.current);
        okTimerRef.current = null;
      }
    };
  }, []);

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
    setError(null);
    try {
      await prefsApi.setHiddenLibraries([...next]);
      setMessage("Saved");
      if (okTimerRef.current !== null) window.clearTimeout(okTimerRef.current);
      okTimerRef.current = window.setTimeout(() => {
        okTimerRef.current = null;
        setMessage(null);
      }, 2500);
    } catch {
      setError("Couldn't save. Try again.");
      setHidden(hidden);
      setMessage(null);
    } finally {
      setBusy(false);
    }
  }

  if (hidden === null) {
    return <LoadingPlaceholder />;
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
      <p className="mb-3 text-xs text-white/50">
        Hidden libraries are excluded from your home page and browse rails.
      </p>
      <ul className="divide-y divide-white/5 border-y border-white/5">
        {libraries.map((lib) => {
          const isHidden = hidden.has(lib.id);
          return (
            <li key={lib.id}>
              <label className="flex cursor-pointer items-center justify-between gap-3 py-3 text-sm transition-colors hover:bg-white/2">
                <span className="flex items-baseline gap-2">
                  <span className={isHidden ? "text-white/55" : "text-white"}>
                    {lib.name}
                  </span>
                  <span className="text-xs text-white/40">
                    {lib.kind === "movies" ? "Movies" : "Shows"}
                  </span>
                </span>
                <span className="relative inline-flex h-6 w-11 shrink-0 items-center">
                  <input
                    type="checkbox"
                    checked={isHidden}
                    onChange={() => toggle(lib.id)}
                    disabled={busy}
                    className="peer sr-only"
                  />
                  <span className="absolute inset-0 rounded-full bg-white/15 transition-colors peer-checked:bg-(--color-accent)" />
                  <span className="absolute left-0.5 inline-block h-5 w-5 transform rounded-full bg-white shadow transition-transform peer-checked:translate-x-5" />
                </span>
              </label>
            </li>
          );
        })}
      </ul>
      <div className="mt-3">
        <SettingsFeedback message={message} error={error} />
      </div>
    </div>
  );
}

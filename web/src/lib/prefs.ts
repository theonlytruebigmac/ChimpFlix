"use client";

// Tiny localStorage-backed prefs hook for *per-device* state — things
// like player volume, muted state, playback speed, and the trailer mute
// toggle that don't make sense to sync across devices. Account-wide
// preferences (default audio language, default subtitle language, avatar,
// display name) live on the user record and go through /api/v1/auth/me.
//
// `updatePrefs` is sync; UI components can call it inside event handlers
// and re-read via the hook on the next render.

import { useCallback, useEffect, useState } from "react";

export interface Prefs {
  trailerMuted: boolean;
  volume: number;
  muted: boolean;
  playbackRate: number;
  autoplayNext: boolean;
}

const STORAGE_KEY = "cf_prefs_v1";
const DEFAULT: Prefs = {
  trailerMuted: true,
  volume: 1,
  muted: false,
  playbackRate: 1,
  autoplayNext: true,
};
const CHANGE_EVENT = "cf_prefs_change";

function read(): Prefs {
  if (typeof window === "undefined") return DEFAULT;
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT;
    return { ...DEFAULT, ...(JSON.parse(raw) as Partial<Prefs>) };
  } catch {
    return DEFAULT;
  }
}

function write(p: Prefs): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(p));
    window.dispatchEvent(new CustomEvent(CHANGE_EVENT));
  } catch {
    // ignore
  }
}

export function usePrefs(): [Prefs, (patch: Partial<Prefs>) => void] {
  const [prefs, setPrefs] = useState<Prefs>(DEFAULT);

  useEffect(() => {
    setPrefs(read());
    function onChange() {
      setPrefs(read());
    }
    window.addEventListener(CHANGE_EVENT, onChange);
    window.addEventListener("storage", onChange);
    return () => {
      window.removeEventListener(CHANGE_EVENT, onChange);
      window.removeEventListener("storage", onChange);
    };
  }, []);

  const update = useCallback((patch: Partial<Prefs>) => {
    const next = { ...read(), ...patch };
    setPrefs(next);
    write(next);
  }, []);

  return [prefs, update];
}

export function getPrefs(): Prefs {
  return read();
}

export function updatePrefs(patch: Partial<Prefs>): void {
  const next = { ...read(), ...patch };
  write(next);
}

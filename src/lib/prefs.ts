"use client";

import { useCallback, useEffect, useState } from "react";

export type Prefs = {
  volume: number; // 0..1
  muted: boolean;
  playbackRate: number; // 0.25..4 in practice
  autoplayNext: boolean;
  trailerMuted: boolean;
};

const KEY = "cf_prefs";
const EVENT = "app:prefs:changed";

export const DEFAULT_PREFS: Prefs = {
  volume: 1,
  muted: false,
  playbackRate: 1,
  autoplayNext: true,
  trailerMuted: true,
};

function clampVolume(v: unknown): number {
  if (typeof v !== "number" || !Number.isFinite(v)) return 1;
  return Math.max(0, Math.min(1, v));
}

function clampRate(r: unknown): number {
  if (typeof r !== "number" || !Number.isFinite(r)) return 1;
  return Math.max(0.25, Math.min(4, r));
}

function read(): Prefs {
  if (typeof window === "undefined") return DEFAULT_PREFS;
  try {
    const raw = window.localStorage.getItem(KEY);
    if (!raw) return DEFAULT_PREFS;
    const parsed = JSON.parse(raw) as Partial<Prefs> | null;
    if (!parsed || typeof parsed !== "object") return DEFAULT_PREFS;
    return {
      volume: clampVolume(parsed.volume),
      muted: typeof parsed.muted === "boolean" ? parsed.muted : false,
      playbackRate: clampRate(parsed.playbackRate),
      autoplayNext:
        typeof parsed.autoplayNext === "boolean" ? parsed.autoplayNext : true,
      trailerMuted:
        typeof parsed.trailerMuted === "boolean" ? parsed.trailerMuted : true,
    };
  } catch {
    return DEFAULT_PREFS;
  }
}

function write(prefs: Prefs): void {
  if (typeof window === "undefined") return;
  window.localStorage.setItem(KEY, JSON.stringify(prefs));
  window.dispatchEvent(new Event(EVENT));
}

export function getPrefs(): Prefs {
  return read();
}

export function updatePrefs(updates: Partial<Prefs>): void {
  write({ ...read(), ...updates });
}

/**
 * Reactive prefs. Subscribes to in-tab `app:prefs:changed` and the native
 * `storage` event so multiple components stay in sync within one tab and
 * across tabs respectively.
 */
export function usePrefs(): [Prefs, (updates: Partial<Prefs>) => void] {
  const [prefs, setPrefsState] = useState<Prefs>(DEFAULT_PREFS);

  useEffect(() => {
    setPrefsState(read());
    function update() {
      setPrefsState(read());
    }
    window.addEventListener(EVENT, update);
    window.addEventListener("storage", update);
    return () => {
      window.removeEventListener(EVENT, update);
      window.removeEventListener("storage", update);
    };
  }, []);

  const update = useCallback((updates: Partial<Prefs>) => {
    updatePrefs(updates);
  }, []);

  return [prefs, update];
}

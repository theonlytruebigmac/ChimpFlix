"use client";

// Tiny localStorage-backed prefs hook for *per-device* state — things
// like player volume, muted state, playback speed, and the trailer mute
// toggle that don't make sense to sync across devices. Account-wide
// preferences (default audio language, default subtitle language, avatar,
// display name, subtitle styling) live on the user record and go through
// /api/v1/auth/me. For subtitle styling specifically, see
// `web/src/lib/subtitle-style.ts`.
//
// `updatePrefs` is sync; UI components can call it inside event handlers
// and re-read via the hook on the next render.

import { useCallback, useSyncExternalStore } from "react";

export interface Prefs {
  trailerMuted: boolean;
  volume: number;
  muted: boolean;
  playbackRate: number;
  autoplayNext: boolean;
  /// EBU R128 loudness normalization on the transcoded audio. When
  /// on, the player asks the backend to apply ffmpeg's `loudnorm`
  /// filter (-16 LUFS / -1.5 dB peak). Off by default because (a)
  /// some users prefer untouched dynamic range, and (b) enabling it
  /// disables the audio-copy fast path (loudnorm requires re-encode).
  audioNormalize: boolean;
  /// When the player enters an intro marker, automatically seek
  /// past the end. Mirrors Netflix's "Skip Intro" auto behavior so
  /// the user doesn't have to click the skip button each episode.
  /// Credits markers still require manual confirmation because they
  /// often contain mid/post-credits scenes the user wants to see.
  autoSkipIntro: boolean;
}

const STORAGE_KEY = "cf_prefs_v1";
const DEFAULT: Prefs = {
  trailerMuted: true,
  volume: 1,
  muted: false,
  playbackRate: 1,
  autoplayNext: true,
  audioNormalize: false,
  autoSkipIntro: false,
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

// useSyncExternalStore-compatible subscribe — listens to both our
// in-tab change event and cross-tab storage events so prefs updates
// in one tab propagate to every other open tab in real time.
function subscribePrefs(callback: () => void): () => void {
  if (typeof window === "undefined") return () => {};
  window.addEventListener(CHANGE_EVENT, callback);
  window.addEventListener("storage", callback);
  return () => {
    window.removeEventListener(CHANGE_EVENT, callback);
    window.removeEventListener("storage", callback);
  };
}

// Cache the snapshot so React's useSyncExternalStore identity check
// is stable until a real change happens; otherwise read() would return
// a fresh object every call and trigger infinite re-renders.
let snapshotCache: Prefs = DEFAULT;
let snapshotKey = "";
function getPrefsSnapshot(): Prefs {
  if (typeof window === "undefined") return DEFAULT;
  const raw = window.localStorage.getItem(STORAGE_KEY) ?? "";
  if (raw === snapshotKey) return snapshotCache;
  snapshotKey = raw;
  snapshotCache = read();
  return snapshotCache;
}
function getServerSnapshot(): Prefs {
  return DEFAULT;
}

export function usePrefs(): [Prefs, (patch: Partial<Prefs>) => void] {
  const prefs = useSyncExternalStore(
    subscribePrefs,
    getPrefsSnapshot,
    getServerSnapshot,
  );

  const update = useCallback((patch: Partial<Prefs>) => {
    const next = { ...read(), ...patch };
    write(next);
    // write() already dispatches CHANGE_EVENT which re-triggers
    // useSyncExternalStore's subscriber → fresh snapshot pulled.
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

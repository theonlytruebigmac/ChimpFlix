"use client";

// Tiny localStorage-backed prefs hook for *per-device* state — things
// like player volume, muted state, playback speed, and the trailer mute
// toggle that don't make sense to sync across devices. Account-wide
// preferences (default audio language, default subtitle language, avatar,
// display name) live on the user record and go through /api/v1/auth/me.
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
  /// Subtitle styling — applies only to external sidecar subtitles
  /// rendered via HTML5 `<track>`. Embedded subs are burned into the
  /// transcode and can't be restyled.
  subtitleFontScale: number;          // multiplier on the browser default
  subtitleColor: string;              // CSS color
  subtitleBackground: string;         // CSS color (incl. alpha)
  subtitlePosition: "bottom" | "top"; // anchor edge
  /// Font family for subtitles. `"default"` lets libass / the browser
  /// pick the system default — same behavior as before this preference
  /// existed. Other values are generic families (sans / serif / mono)
  /// that map both to a CSS `font-family` for external `<track>`
  /// rendering and to an ASS `Fontname=` for burned-in transcodes.
  /// Kept generic on purpose: shipping specific font names would
  /// require the docker image to bundle those fonts and fontconfig
  /// to resolve them; the generic families already cover the
  /// legibility-vs-aesthetic axis users actually want to choose along.
  subtitleFontFamily: "default" | "sans" | "serif" | "mono";
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
  subtitleFontScale: 1,
  subtitleColor: "#FFFFFF",
  subtitleBackground: "rgba(0, 0, 0, 0.6)",
  subtitlePosition: "bottom",
  subtitleFontFamily: "default",
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

/// Convert subtitle prefs into an ffmpeg `force_style` string suitable
/// for appending to a `subtitles=` filter:
///
///   force_style='Fontsize=28,PrimaryColour=&H00FFFFFF&,...'
///
/// ASS color format is `&HAABBGGRR&` (alpha first, BGR not RGB, alpha
/// inverted from CSS where 0xFF means OPAQUE in ASS but TRANSPARENT in
/// CSS). Returns null when the user hasn't deviated from defaults (no
/// point bloating every transcode session with redundant style args).
export function prefsToAssStyle(p: Prefs): string | null {
  const fontSize = Math.round(28 * Math.max(0.5, Math.min(3, p.subtitleFontScale)));
  const primary = cssColorToAss(p.subtitleColor) ?? "&H00FFFFFF&";
  const back = cssColorToAss(p.subtitleBackground) ?? "&H66000000&";
  // Alignment: ASS uses numpad-style positions. 2 = bottom-centre,
  // 8 = top-centre.
  const align = p.subtitlePosition === "top" ? 8 : 2;
  // BorderStyle=3 paints an opaque box behind the text using BackColour.
  // BorderStyle=1 gives an outline+shadow instead. The box reads better
  // against busy footage, which is the whole point of a user-set bg.
  const parts = [
    `Fontsize=${fontSize}`,
    `PrimaryColour=${primary}`,
    `BackColour=${back}`,
    `BorderStyle=3`,
    `Outline=1`,
    `Shadow=0`,
    `Alignment=${align}`,
  ];
  // Only emit Fontname when the user picked a non-default. libass
  // resolves these via fontconfig at burn-in time — Debian-based
  // images bundle `fonts-dejavu` and `fonts-liberation`, which both
  // expose the generic family names below.
  const fontname = assFontnameFor(p.subtitleFontFamily);
  if (fontname) parts.push(`Fontname=${fontname}`);
  return parts.join(",");
}

/// Resolve the user's subtitle font preference to an ASS `Fontname`
/// value, or `null` for the default (omit the directive entirely so
/// libass uses its built-in fallback). The names below are libass /
/// fontconfig generics — they resolve to whatever the system has
/// available rather than a specific TTF.
function assFontnameFor(
  fam: Prefs["subtitleFontFamily"],
): string | null {
  switch (fam) {
    case "sans":
      return "Sans";
    case "serif":
      return "Serif";
    case "mono":
      return "Monospace";
    case "default":
    default:
      return null;
  }
}

/// CSS `font-family` value for the user's subtitle font preference.
/// Used by the ::cue stylesheet for external sidecar subtitles —
/// embedded burned-in subs go through ASS instead (see
/// [`prefsToAssStyle`]). `null` means "let the browser pick its
/// caption default" (no rule emitted).
export function cssFontFamilyForSubtitlePref(
  fam: Prefs["subtitleFontFamily"],
): string | null {
  switch (fam) {
    case "sans":
      return "system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif";
    case "serif":
      return "Georgia, 'Times New Roman', serif";
    case "mono":
      return "ui-monospace, SFMono-Regular, Menlo, monospace";
    case "default":
    default:
      return null;
  }
}

/// Parse a CSS color string to ASS `&HAABBGGRR&`. Returns null on
/// anything we can't recognise — caller falls back to its default.
/// Handles `#RRGGBB`, `#RRGGBBAA`, `#RGB`, and `rgb()`/`rgba()`. Named
/// colors not supported (rarely used in our prefs UI).
function cssColorToAss(css: string): string | null {
  const trimmed = css.trim();
  let r = 0, g = 0, b = 0, a = 1;
  if (trimmed.startsWith("#")) {
    const hex = trimmed.slice(1);
    if (hex.length === 3) {
      r = parseInt(hex[0] + hex[0], 16);
      g = parseInt(hex[1] + hex[1], 16);
      b = parseInt(hex[2] + hex[2], 16);
    } else if (hex.length === 6) {
      r = parseInt(hex.slice(0, 2), 16);
      g = parseInt(hex.slice(2, 4), 16);
      b = parseInt(hex.slice(4, 6), 16);
    } else if (hex.length === 8) {
      r = parseInt(hex.slice(0, 2), 16);
      g = parseInt(hex.slice(2, 4), 16);
      b = parseInt(hex.slice(4, 6), 16);
      a = parseInt(hex.slice(6, 8), 16) / 255;
    } else {
      return null;
    }
  } else {
    const m = trimmed.match(
      /^rgba?\(\s*(\d+(?:\.\d+)?)\s*,\s*(\d+(?:\.\d+)?)\s*,\s*(\d+(?:\.\d+)?)\s*(?:,\s*(\d+(?:\.\d+)?))?\s*\)$/,
    );
    if (!m) return null;
    r = Math.round(parseFloat(m[1]));
    g = Math.round(parseFloat(m[2]));
    b = Math.round(parseFloat(m[3]));
    if (m[4] !== undefined) a = parseFloat(m[4]);
  }
  if ([r, g, b].some((v) => !Number.isFinite(v) || v < 0 || v > 255)) {
    return null;
  }
  // ASS alpha: 0x00 = fully opaque, 0xFF = fully transparent (inverse
  // of CSS). Clamp to [0, 1] first.
  const cssAlpha = Math.max(0, Math.min(1, a));
  const assAlpha = Math.round((1 - cssAlpha) * 255);
  const hex = (n: number) => n.toString(16).toUpperCase().padStart(2, "0");
  return `&H${hex(assAlpha)}${hex(b)}${hex(g)}${hex(r)}&`;
}

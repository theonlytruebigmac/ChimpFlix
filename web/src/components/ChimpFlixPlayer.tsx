"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type ButtonHTMLAttributes,
} from "react";
import type Hls from "hls.js";
import {
  ChimpFlixApiError,
  auth as authApi,
  stream as streamApi,
  playState as playStateApi,
  seasons as seasonsApi,
  readCsrfToken,
} from "@/lib/chimpflix-api";
import { plexImage } from "@/lib/image";
import { detectClientCapabilities, isSafari } from "@/lib/client-caps";
import { getPrefs, updatePrefs, usePrefs } from "@/lib/prefs";
import {
  DEFAULT_SUBTITLE_STYLE,
  cssFontFamilyForSubtitleStyle,
  subtitleStyleToAss,
  type SubtitleFontFamily,
  type SubtitleStyle,
} from "@/lib/subtitle-style";
import { consumePrewarm } from "@/lib/prewarm";
import { useFocusTrap } from "@/lib/use-focus-trap";

export interface QualityChoice {
  label: string;
  /// `null` = let the server decide (auto, derived from source).
  /// Any other value forces transcode to scale to this height. The
  /// per-rung bitrate is no longer baked in here — the backend's
  /// ladder fills in a default for the chosen height, and the
  /// independent [`BitrateCapChoice`] applies a user-controlled
  /// upper bound regardless of resolution.
  height: number | null;
}

/// Fixed quality ladder, ordered high → low. "Auto" sits at the top so
/// picker scrolling matches user expectations.
const QUALITY_OPTIONS: QualityChoice[] = [
  { label: "Auto", height: null },
  { label: "1080p", height: 1080 },
  { label: "720p", height: 720 },
  { label: "480p", height: 480 },
  { label: "240p", height: 240 },
];

/// User-controlled video bitrate cap. Independent of resolution so a
/// viewer can pick "1080p but cap at 3 Mbps for my mobile plan" without
/// downsampling to 720p just to save bandwidth. `null` = no cap (use
/// the resolution's ladder default).
export interface BitrateCapChoice {
  label: string;
  bps: number | null;
}

const BITRATE_CAP_OPTIONS: BitrateCapChoice[] = [
  { label: "No cap", bps: null },
  { label: "10 Mbps", bps: 10_000_000 },
  { label: "5 Mbps", bps: 5_000_000 },
  { label: "3 Mbps", bps: 3_000_000 },
  { label: "2 Mbps", bps: 2_000_000 },
  { label: "1 Mbps", bps: 1_000_000 },
  { label: "500 Kbps", bps: 500_000 },
  { label: "250 Kbps", bps: 250_000 },
];

// Subtitle styling model + helpers live in @/lib/subtitle-style.ts as
// the canonical single source of truth, server-synced per account via
// users.subtitle_*. The presets below are UI-only — they don't change
// the model, they're just the picker palette.
const FONT_SIZE_PRESETS: { label: string; px: number }[] = [
  { label: "S", px: 18 },
  { label: "M", px: 24 },
  { label: "L", px: 32 },
  { label: "XL", px: 42 },
];
const TEXT_COLOR_PRESETS: { label: string; value: string }[] = [
  { label: "White", value: "#ffffff" },
  { label: "Yellow", value: "#ffe066" },
  { label: "Cyan", value: "#7dd3fc" },
  { label: "Green", value: "#a3e635" },
];
const BG_PRESETS: { label: string; value: string }[] = [
  { label: "None", value: "rgba(0,0,0,0)" },
  { label: "Light", value: "rgba(0,0,0,0.35)" },
  { label: "Medium", value: "rgba(0,0,0,0.55)" },
  { label: "Solid", value: "rgba(0,0,0,0.85)" },
];

const OFFSET_STORAGE_PREFIX = "chimpflix:subtitle:offset:";

export interface VersionChoice {
  media_file_id: number;
  /// Pre-formatted label for the picker, e.g. "4K HDR · HEVC" or
  /// "1080p". The watch page builds this from MediaFileSummary so the
  /// player stays string-pure.
  label: string;
  /// Audio tracks for this specific file. Indices are 0-based among the
  /// file's audio streams. Each version may have different audio (a
  /// 4K release commonly bundles more language dubs than the 1080p).
  audioTracks: StreamChoice[];
  /// Embedded subtitle tracks for this specific file, plus the same
  /// external subs that apply to every version (their URLs aren't file
  /// scoped). Lets the picker show the right rows after a switch.
  subtitleTracks: StreamChoice[];
}

export interface StreamChoice {
  // 0-indexed among that kind's streams in the file. Pass straight to the
  // server's audio_index / subtitle_index.
  idx: number;
  label: string;
  language?: string | null;
  /// Raw codec name from ffprobe (lowercase). Used by the watch-page
  /// auto-picker to skip picture-based subtitles (PGS, DVD, VobSub)
  /// that need a heavyweight overlay path the user almost never
  /// wants by default — the user can still select them manually
  /// from the picker if they're the only option.
  codec?: string | null;
  /// When set, this is an external subtitle (`external_subtitles` row).
  /// The player renders it via an HTML5 `<track>` instead of asking the
  /// transcoder to burn it in — works for direct-play and HLS without
  /// the subtitle-burn fallback. Transcode-burn for external subs is
  /// queued as a follow-up.
  externalUrl?: string;
}

export interface PlayerMarker {
  kind: "intro" | "credits" | string;
  start_ms: number;
  end_ms: number;
  /// `auto` (detect_markers task) or `manual` (operator editor).
  /// Drives a more prominent timeline tint for auto markers so the
  /// user can see detected segments at a glance vs hand-curated
  /// ones. Optional so older API consumers (and tests) don't break.
  source?: "auto" | "manual" | string;
}

export type EpisodeSibling = {
  ratingKey: string;
  title: string;
  thumb?: string;
  summary?: string;
  duration?: number;
  viewOffset?: number;
  index?: number;
  parentTitle?: string;
};

interface Props {
  title: string;
  subtitle?: string;
  mediaFileId: number;
  // Best-known duration in milliseconds. Comes from the file's metadata
  // (ffprobe) — authoritative across the whole title, unlike `video.duration`
  // which only reflects what HLS has surfaced so far. Used for the time
  // display and progress bar.
  durationMs?: number;
  startPositionMs?: number;
  itemId?: number;
  episodeId?: number;
  backHref: string;
  nextHref?: string;
  nextLabel?: string;
  nextThumb?: string;
  audioTracks?: StreamChoice[];
  subtitleTracks?: StreamChoice[];
  audioIndex?: number;
  subtitleIndex?: number;
  /// When set, seed the player with an external subtitle as the initial
  /// selection (overrides `subtitleIndex`). Used so a saved-language
  /// preference can target an OpenSubtitles-fetched track on first
  /// load, not just the embedded streams.
  externalSubtitleUrl?: string;
  markers?: PlayerMarker[];
  seasonEpisodes?: EpisodeSibling[];
  /// Rating-key of the currently-playing episode. Identifies which row
  /// in the in-player popup gets the Netflix-style "expanded" treatment
  /// (thumbnail + synopsis); every other row renders as a compact item.
  currentRatingKey?: string;
  /// DB id of the current season. The picker pane uses it to put a
  /// checkmark next to the right entry.
  currentSeasonId?: number;
  /// Show id. The popup uses it to lazy-load episodes for a different
  /// season when the viewer changes seasons mid-playback.
  showId?: number;
  /// Show title — shown as the heading in the season-picker pane.
  showTitle?: string;
  /// All seasons in the show, ordered as the API returns them. Powers
  /// the season picker (back-arrow on the episodes pane).
  seasons?: { id: number; season_number: number; title: string | null }[];
  /// When the same title has multiple media files (4K + 1080p, etc.)
  /// the player exposes a Version picker. Initial `mediaFileId` is
  /// treated as the active one; switching versions just swaps the id
  /// the session is built against, preserving playback position.
  versions?: VersionChoice[];
  /// Operator-configured threshold (1–99) at which we auto-scrobble
  /// the session as watched. Comes from `/play-state/config` so the
  /// player stays in sync with the server's source of truth. Default
  /// 90 matches the historical baked-in value.
  playedThresholdPct?: number;
  /// One of `threshold_pct` / `first_credits_marker` /
  /// `earliest_of_both`. Drives the scrobble decision alongside
  /// `playedThresholdPct`. Default `threshold_pct` when omitted.
  completionBehaviour?: string;
  /// Initial subtitle styling, server-sourced from the user record.
  /// The player owns local state from here on and PATCHes /auth/me
  /// when the viewer changes a value in the gear menu. Omitted →
  /// fall back to `DEFAULT_SUBTITLE_STYLE` (e.g. preview surfaces
  /// without an authenticated user).
  initialSubtitleStyle?: SubtitleStyle;
}

/// iOS Safari (and standalone iPhone PWAs) doesn't implement
/// `Element.requestFullscreen`; you have to call
/// `webkitEnterFullscreen()` on the HTMLVideoElement itself. That
/// method shows iOS's native video player chrome (not ours), but
/// at least the video covers the screen. Better than nothing —
/// without this fallback iPhone users have no way to leave the
/// pillarboxed inline player view.
function tryWebkitVideoFullscreen(video: HTMLVideoElement | null): void {
  if (!video) return;
  const v = video as HTMLVideoElement & {
    webkitEnterFullscreen?: () => void;
  };
  if (typeof v.webkitEnterFullscreen === "function") {
    try {
      v.webkitEnterFullscreen();
    } catch {
      // Best-effort; nothing else we can do here.
    }
  }
}

const PLAY_STATE_INTERVAL_MS = 10_000;
/// Fallback threshold when the prop isn't passed (e.g. an older route
/// still wraps the player). Matches the value this used to be baked at.
const DEFAULT_SCROBBLE_THRESHOLD = 0.9;
const COUNTDOWN_WINDOW_SECONDS = 10;
const SPEED_OPTIONS = [0.5, 0.75, 1, 1.25, 1.5, 2] as const;

function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "0:00";
  const total = Math.floor(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h > 0) {
    return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
  }
  return `${m}:${String(s).padStart(2, "0")}`;
}

function activeMarker(
  currentMs: number,
  markers?: PlayerMarker[],
): PlayerMarker | null {
  if (!markers) return null;
  for (const m of markers) {
    if (currentMs >= m.start_ms && currentMs <= m.end_ms) return m;
  }
  return null;
}

function markerLabel(m: PlayerMarker): string {
  if (m.kind === "credits") return "Skip Credits";
  if (m.kind === "commercial") return "Skip Ad";
  return "Skip Intro";
}

/// Keyboard nav for a `role="menu"` container that holds
/// `role="menuitemradio"` buttons. ArrowDown / ArrowUp cycle through
/// items (wrapping at the ends); Home / End jump to first / last.
/// Matches the WAI-ARIA menu pattern so keyboard users can drive
/// the audio / subtitle / quality pickers without Tab-bouncing
/// through every disabled row.
function handleMenuArrowKeys(e: React.KeyboardEvent<HTMLDivElement>) {
  if (
    e.key !== "ArrowDown"
    && e.key !== "ArrowUp"
    && e.key !== "Home"
    && e.key !== "End"
  ) {
    return;
  }
  const items = Array.from(
    e.currentTarget.querySelectorAll<HTMLButtonElement>(
      '[role="menuitemradio"]',
    ),
  ).filter((el) => !el.disabled);
  if (items.length === 0) return;
  e.preventDefault();
  const active = document.activeElement;
  const idx = items.findIndex((el) => el === active);
  let next = 0;
  if (e.key === "ArrowDown") {
    next = idx < 0 ? 0 : (idx + 1) % items.length;
  } else if (e.key === "ArrowUp") {
    next = idx <= 0 ? items.length - 1 : idx - 1;
  } else if (e.key === "End") {
    next = items.length - 1;
  }
  items[next]?.focus();
}

/// Subtitle codec names ffprobe emits for picture-based formats. These
/// can't be soft-rendered — the transcoder has to overlay-burn them
/// into the video, which means the user's subtitle-appearance panel
/// (font, color, background) is ignored and changing the selection
/// rerolls the session. Surface that in the picker so the user
/// understands why their styling didn't take. Keep this list in sync
/// with the watch-page auto-picker's exclusion set.
const PICTURE_SUBTITLE_CODECS = new Set([
  "hdmv_pgs_subtitle",
  "pgs",
  "dvd_subtitle",
  "dvdsub",
  "dvb_subtitle",
  "vobsub",
  "xsub",
]);

function isPictureSubtitle(codec: string | null | undefined): boolean {
  return codec ? PICTURE_SUBTITLE_CODECS.has(codec.toLowerCase()) : false;
}

/// Brief top-anchored pill shown when the player mounts mid-file (i.e.
/// the user is resuming). Plex-style: communicates the resume position
/// and offers a one-click "Start over". Fades in on mount, auto-hides
/// after 6 s, dismissable via the close button.
function ResumePill({
  resumeSec,
  onStartOver,
  onDismiss,
}: {
  resumeSec: number;
  onStartOver: () => void;
  onDismiss: () => void;
}) {
  const [appear, setAppear] = useState(false);
  useEffect(() => {
    const id = requestAnimationFrame(() => setAppear(true));
    return () => cancelAnimationFrame(id);
  }, []);
  return (
    <div
      className={`pointer-events-auto absolute inset-x-0 top-[max(4.5rem,calc(env(safe-area-inset-top)+3.5rem))] z-30 flex justify-center px-4 transition-all duration-200 ${
        appear ? "opacity-100 translate-y-0" : "opacity-0 -translate-y-2"
      }`}
    >
      <div className="flex items-center gap-3 rounded-full border border-white/15 bg-black/75 px-4 py-2 text-sm text-white/90 shadow-2xl backdrop-blur-md">
        <span className="text-white/70">
          Resumed from{" "}
          <span className="font-medium text-white tabular-nums">
            {formatTime(resumeSec)}
          </span>
        </span>
        <button
          type="button"
          onClick={onStartOver}
          className="rounded-full border border-white/30 px-3 py-0.5 text-xs font-semibold text-white transition-colors hover:border-white hover:bg-white hover:text-black"
        >
          Start over
        </button>
        <button
          type="button"
          onClick={onDismiss}
          aria-label="Dismiss"
          title="Dismiss"
          className="text-white/55 transition-colors hover:text-white"
        >
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2.5"
            strokeLinecap="round"
            aria-hidden
          >
            <line x1="6" y1="6" x2="18" y2="18" />
            <line x1="18" y1="6" x2="6" y2="18" />
          </svg>
        </button>
      </div>
    </div>
  );
}

/// Skip-marker pill with a live countdown so the operator sees how
/// long they have to act. The countdown updates from a local rAF tick
/// rather than re-rendering the whole player every 250ms — keeps the
/// price of this affordance tiny.
function SkipMarkerButton({
  marker,
  currentMs,
  onSkip,
  onDismiss,
}: {
  marker: PlayerMarker;
  currentMs: number;
  onSkip: (m: PlayerMarker) => void;
  onDismiss: () => void;
}) {
  const remaining = Math.max(0, Math.ceil((marker.end_ms - currentMs) / 1000));
  // Fade-in once on mount so the pill animates in rather than popping.
  // `appear` flips true after the first paint — Tailwind's transition
  // then animates the opacity + translate from the initial values.
  const [appear, setAppear] = useState(false);
  useEffect(() => {
    // Use rAF so the initial paint sees `appear=false`, and the next
    // frame sees `appear=true` — the transition lerps between them.
    const id = requestAnimationFrame(() => setAppear(true));
    return () => cancelAnimationFrame(id);
  }, []);
  return (
    <div
      className={`pointer-events-auto absolute bottom-28 right-3 z-30 flex items-stretch shadow-2xl transition-all duration-200 sm:bottom-32 sm:right-8 ${
        appear ? "opacity-100 translate-y-0" : "opacity-0 translate-y-2"
      }`}
    >
      <button
        type="button"
        onClick={() => onSkip(marker)}
        className="inline-flex items-center gap-2 rounded-l-md border border-r-0 border-white/30 bg-white/95 px-4 py-2 text-sm font-semibold text-black transition-colors hover:bg-white sm:px-6 sm:py-2.5"
      >
        <span>{markerLabel(marker)}</span>
        {remaining > 0 && (
          <span
            aria-hidden
            className="tabular-nums text-[0.78em] text-black/55"
          >
            {remaining}s
          </span>
        )}
      </button>
      <button
        type="button"
        onClick={onDismiss}
        aria-label="Dismiss"
        title="Dismiss"
        className="inline-flex items-center justify-center rounded-r-md border border-white/30 bg-white/95 px-2.5 text-black/55 transition-colors hover:bg-white hover:text-black"
      >
        <svg
          width="12"
          height="12"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2.5"
          strokeLinecap="round"
          aria-hidden
        >
          <line x1="6" y1="6" x2="18" y2="18" />
          <line x1="18" y1="6" x2="6" y2="18" />
        </svg>
      </button>
    </div>
  );
}

export function ChimpFlixPlayer({
  title,
  subtitle,
  mediaFileId,
  durationMs,
  startPositionMs = 0,
  itemId,
  episodeId,
  backHref,
  nextHref,
  nextLabel,
  nextThumb,
  audioTracks,
  subtitleTracks,
  audioIndex,
  subtitleIndex,
  externalSubtitleUrl,
  markers,
  seasonEpisodes,
  currentRatingKey,
  currentSeasonId,
  showId,
  showTitle,
  seasons,
  versions,
  playedThresholdPct,
  completionBehaviour,
  initialSubtitleStyle = DEFAULT_SUBTITLE_STYLE,
}: Props) {
  // Normalize the configured threshold to a fraction (0-1) once. Clamp
  // to a sane band to defend against the API returning garbage.
  const scrobbleThreshold =
    playedThresholdPct == null
      ? DEFAULT_SCROBBLE_THRESHOLD
      : Math.min(0.99, Math.max(0.5, playedThresholdPct / 100));
  const router = useRouter();
  const [prefs] = usePrefs();
  const containerRef = useRef<HTMLDivElement>(null);
  const videoRef = useRef<HTMLVideoElement>(null);
  const hideTimerRef = useRef<number | null>(null);
  const hlsRef = useRef<Hls | null>(null);
  // Whether the device's primary input is a touch screen. Computed
  // once via matchMedia because per-event `pointerType` is unreliable
  // in the Android Chrome PWA wrapper — pointerup sometimes fires with
  // pointerType="mouse" on a real touch tap, which made the click
  // handler take the desktop branch (togglePlay = pause) instead of
  // the touch branch (show controls). Device-based detection sidesteps
  // that quirk entirely. `useState` (not useMemo) so SSR + hydration
  // agree on `false` initially and the effect bumps it to true post-
  // hydration if appropriate — keeps us out of hydration-mismatch land.
  const [isTouchDevice, setIsTouchDevice] = useState(false);
  useEffect(() => {
    if (typeof window === "undefined") return;
    const matches =
      window.matchMedia?.("(hover: none) and (pointer: coarse)").matches ??
      false;
    // eslint-disable-next-line react-hooks/set-state-in-effect
    if (matches) setIsTouchDevice(true);
  }, []);
  // Late-bound ref to seekBy so callbacks declared before seekBy (Media
  // Session, etc.) can route through the source-time-aware seek path
  // without TDZ errors from a direct reference.
  const seekByRef = useRef<((delta: number) => void) | null>(null);
  // True while the user is mid-scrub (pointer down on the progress bar).
  // The stall watchdog and auto-skip-intro effects check this to avoid
  // yanking currentTime in the middle of a user-initiated seek.
  const scrubbingRef = useRef(false);
  const scrobbledRef = useRef(false);
  /// Backend session id for the currently-mounted HLS stream. The
  /// session-creation effect sets this; the pause/resume + scrub
  /// pre-warm hooks read it. `null` for a direct-play session (no
  /// transcoder session to pause/resume).
  const activeSessionIdRef = useRef<string | null>(null);
  // Captured the resume position so a track switch mid-playback comes back
  // to roughly where the user was, not the original startPositionMs.
  // Always source-time (file timeline), not HLS media-time.
  const liveTimeMsRef = useRef<number>(startPositionMs);
  // The source-time at which the current session's HLS stream begins.
  // 0 for direct play (file timeline == video.currentTime). For transcode,
  // ffmpeg fast-seeks to start_position_ms and HLS.js then renders that
  // as media-time 0 — so source-time = video.currentTime + sessionStartMs.
  // All public reads/writes of position go through this offset.
  const sessionStartMsRef = useRef<number>(0);
  // Bumped when the user seeks before the current session's start. The
  // session useEffect lists it as a dep so the bump tears the session
  // down and creates a new one rooted at `liveTimeMsRef.current`.
  const [resumeEpoch, setResumeEpoch] = useState(0);
  // Debounce handle for `triggerSessionRestart`. A user spamming the
  // scrubber can fire 5-10 restart-eligible seekTo calls in under a
  // second; each one used to spawn a DELETE + POST pair against the
  // backend, and the old session's in-flight segment fetches would
  // race the new session's manifest. With this timer we coalesce
  // adjacent restarts so only the final target survives — the user
  // still lands where they expect, but ffmpeg only spins up once.
  const restartDebounceRef = useRef<number | null>(null);
  // The user's pending seek target (source-time, ms) while a session
  // restart is being debounced or in-flight. SEPARATE from
  // `liveTimeMsRef` because that ref is overwritten by every
  // onTimeUpdate while the existing session is still playing — if
  // we used liveTimeMsRef as the restart target, a fast user seeking
  // backward would have their target clobbered by the playhead's
  // forward progress during the debounce window. The useEffect that
  // mints new sessions reads this ref FIRST and clears it after
  // consumption; if null, falls back to liveTimeMsRef (mount-time
  // resume path).
  const pendingRestartTargetMsRef = useRef<number | null>(null);
  // Snapshot of the last audio/subtitle/quality combo that produced a
  // working session. If session creation fails after the user changed
  // one of those axes, the catch path reverts to this snapshot so the
  // player isn't stuck on a broken selection with no recovery — the
  // state update triggers another session-create with the known-good
  // combo. Reset on file/episode change so we don't try to restore
  // indices that don't exist on the new file.
  const lastGoodTracksRef = useRef<{
    audio: number | undefined;
    subtitle: number | null | undefined;
    quality: QualityChoice;
    bitrateCap: BitrateCapChoice;
  } | null>(null);

  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  /// Set true when HLS.js is mid-recovery from a fatal error
  /// (network blip, MSE decode hiccup). A subtle overlay communicates
  /// the state to the user without the alarming "playback failed"
  /// chrome the error path uses. Cleared after the recovery attempt
  /// settles (success → playback resumes; failure → `error` is set).
  const [reconnecting, setReconnecting] = useState(false);
  const [playing, setPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(startPositionMs / 1000);
  // End of the contiguous buffered range that contains currentTime,
  // expressed in source-time seconds (i.e. file timeline, not HLS
  // media-time). Drives the lighter "buffer ahead" overlay on the
  // seekbar so users get the Netflix-style visualization of how
  // much is already loaded past the playhead.
  const [bufferedEnd, setBufferedEnd] = useState(0);
  // Server-provided duration is the source of truth for the time display.
  // `video.duration` only reflects what HLS has parsed so far, which on a
  // live transcoder is wildly under-counted (e.g. 15 minutes for a 2-hour
  // movie). Fall back to that only if the server didn't give us one.
  const [videoDuration, setVideoDuration] = useState(
    durationMs ? durationMs / 1000 : 0,
  );
  const [muted, setMuted] = useState(false);
  const [volume, setVolume] = useState(1);
  const [isFullscreen, setIsFullscreen] = useState(false);
  const [showControls, setShowControls] = useState(true);
  /// Touch-only: brief overlay rendered on the side that was just
  /// double-tapped, fading out after ~600ms. `null` = none. The number
  /// is the delta in seconds (negative for back, positive for forward)
  /// so the overlay can render "-10s" / "+10s".
  const [seekFlash, setSeekFlash] = useState<{
    side: "left" | "right";
    delta: number;
    nonce: number;
  } | null>(null);
  const [autoplayBlocked, setAutoplayBlocked] = useState(false);
  // Local selection state. `undefined` = transcoder default. For subtitles
  // we use `null` to mean "explicitly off" (no subtitle_index sent).
  const [audioSel, setAudioSel] = useState<number | undefined>(audioIndex);
  const [subtitleSel, setSubtitleSel] = useState<number | null | undefined>(
    subtitleIndex,
  );
  /// Active media file id. Mutated by the Version picker; the session
  /// useEffect rebuilds the HLS pipeline against the new id while
  /// `liveTimeMsRef` preserves playback position across the swap.
  const [activeMediaFileId, setActiveMediaFileId] = useState(mediaFileId);
  /// Selected quality tier. Auto (default) lets the server pick; any
  /// non-auto choice forces transcode at the chosen resolution and
  /// bitrate. The session useEffect lists this as a dep so picking a
  /// new tier re-rolls ffmpeg.
  const [qualitySel, setQualitySel] = useState<QualityChoice>(
    QUALITY_OPTIONS[0],
  );
  /// User-controlled bitrate cap, independent of [`qualitySel`]. Sits
  /// next to the resolution picker in the Quality popover. "No cap"
  /// (the default) means the backend uses the ladder default for
  /// whatever resolution we end up at.
  const [bitrateCapSel, setBitrateCapSel] = useState<BitrateCapChoice>(
    BITRATE_CAP_OPTIONS[0],
  );
  /// Snapshot of what the server actually decided to run for the
  /// current session. Surfaced in the Quality picker so a user with
  /// "Auto" selected can see the resolved tier (e.g. "Auto · 1080p"),
  /// the encoder in play, and so impractical tiers (above source
  /// height) can be greyed out. Reset on every session re-roll.
  const [sessionStatus, setSessionStatus] = useState<{
    height: number | null;
    sourceHeight: number | null;
    encoder: string | null;
    videoTreatment: "copy" | "reencode" | null;
    audioTreatment: "copy" | "reencode" | null;
  } | null>(null);
  /// User-tunable subtitle sync offset in milliseconds. Positive =
  /// subs delayed, negative = subs advanced. Persisted per (user,
  /// mediaFileId) so the same correction sticks on replays of the
  /// same source. Defaults to 0 (cache-extracted WebVTT is already
  /// shifted by the seek offset on the server side).
  ///
  /// Changes here flow into the session POST as
  /// `subtitle_offset_ms`, which triggers a session restart (server
  /// re-shifts the cached WebVTT — ~50 ms because the source is
  /// already in the per-file cache). No HLS.js cue-time fiddling
  /// needed.
  const [subtitleOffsetMs, setSubtitleOffsetMs] = useState<number>(0);
  /// Subtitle styling state. Seeded from the user record (server-side
  /// source of truth, phase 89). The gear menu mutates this locally
  /// for immediate visual feedback and fires a PATCH /auth/me so the
  /// new value follows the user across devices. Optimistic — a 4xx
  /// is logged but doesn't roll back the local state, which keeps the
  /// in-session UX snappy.
  const [subtitleStyle, setSubtitleStyleLocal] =
    useState<SubtitleStyle>(initialSubtitleStyle);
  const setSubtitleStyle = useCallback((next: SubtitleStyle) => {
    setSubtitleStyleLocal(next);
    authApi
      .updateMe({
        subtitle_font_size_px: next.fontSizePx,
        subtitle_text_color: next.textColor,
        subtitle_background_color: next.backgroundColor,
        subtitle_font_family: next.fontFamily,
        subtitle_edge: next.edge,
        subtitle_bottom_inset_pct: next.bottomInsetPct,
      })
      .catch(() => {
        // Best-effort persistence; viewer keeps the look in-session
        // even if the write failed (e.g. transient offline).
      });
  }, []);
  // External-subtitle selection lives alongside subtitleSel. When set,
  // the embedded `subtitle_index` is forced off and the video gets a
  // sibling `<track>` element. Only one path can be active at a time.
  // We hold url + language together so the `<track>` element gets the
  // right `srcLang` (browsers expose it via track.language and use it
  // to honor the user's accept-language preferences).
  const [externalSub, setExternalSub] = useState<{
    url: string;
    language: string | null;
  } | null>(
    externalSubtitleUrl
      ? { url: externalSubtitleUrl, language: null }
      : null,
  );
  const externalSubUrl = externalSub?.url ?? null;

  // One-shot cleanup of the pre-phase-89 device-local appearance key.
  // Subtitle styling is server-synced now; the orphan key would just
  // confuse anyone inspecting DevTools. Safe to noop after the first
  // mount that observes it — localStorage.removeItem on a missing
  // key is a cheap no-op so we don't bother gating.
  useEffect(() => {
    if (typeof window === "undefined") return;
    try {
      window.localStorage.removeItem("chimpflix:subtitle:appearance");
    } catch {
      // ignore
    }
  }, []);

  // Hydrate per-file subtitle offset from localStorage on file change.
  // Offset stays device-local (each title has its own sync drift; what
  // a viewer dialled in for one file shouldn't follow them to another).
  // This is "sync to external state" so the setState-in-effect pattern
  // is correct — the alternative would be a useSyncExternalStore
  // wrapper for negligible benefit.
  useEffect(() => {
    if (typeof window === "undefined") return;
    /* eslint-disable react-hooks/set-state-in-effect */
    try {
      const offsetRaw = window.localStorage.getItem(
        OFFSET_STORAGE_PREFIX + activeMediaFileId,
      );
      if (offsetRaw !== null) {
        const n = Number.parseInt(offsetRaw, 10);
        if (Number.isFinite(n)) setSubtitleOffsetMs(n);
      } else {
        // No saved value for this file — make sure we don't keep
        // showing the previous file's offset after a version
        // switch.
        setSubtitleOffsetMs(0);
      }
    } catch {
      // Corrupt localStorage or quota issue; fall back to defaults.
    }
    /* eslint-enable react-hooks/set-state-in-effect */
  }, [activeMediaFileId]);

  useEffect(() => {
    if (typeof window === "undefined") return;
    try {
      window.localStorage.setItem(
        OFFSET_STORAGE_PREFIX + activeMediaFileId,
        String(subtitleOffsetMs),
      );
    } catch {
      // Quota exceeded or private-browsing mode; ignore.
    }
  }, [activeMediaFileId, subtitleOffsetMs]);

  // Move WebVTT cues vertically so they (1) honor the user's
  // bottom-inset preference, (2) auto-shift up while the player
  // controls overlay is visible (otherwise controls cover the
  // bottom-most line), AND (3) account for letterboxing so the
  // user's "5% from bottom" means 5% above the visible video,
  // not 5% above the player element. Native WebVTT positioning
  // is element-relative — without the letterbox math, anything
  // under ~10% from bottom on a 16:9 video in a 21:9 container
  // lands in the bottom black bar.
  //
  // We operate on TextTrack.cues directly (`cue.line` +
  // `snapToLines=false`) because that's the only standardized way
  // to reposition WebVTT — `::cue` CSS can style the text but not
  // place the cue box.
  useEffect(() => {
    const v = videoRef.current;
    if (!v) return;
    const applyAll = () => {
      // Compute letterbox height as a percentage of the video
      // ELEMENT. `videoWidth/Height` is the source resolution
      // and zero until `loadedmetadata`; bail out cleanly until
      // we know the aspect.
      const elementW = v.clientWidth;
      const elementH = v.clientHeight;
      const vidW = v.videoWidth;
      const vidH = v.videoHeight;
      let letterboxBottomPct = 0;
      if (vidW > 0 && vidH > 0 && elementW > 0 && elementH > 0) {
        const videoAspect = vidW / vidH;
        const elementAspect = elementW / elementH;
        if (videoAspect > elementAspect) {
          // Black bars top + bottom. Render height shrinks; the
          // bottom bar is (element - rendered) / 2.
          const renderedH = elementW / videoAspect;
          const barTotal = elementH - renderedH;
          letterboxBottomPct = ((barTotal / 2) / elementH) * 100;
        }
        // If videoAspect < elementAspect we have pillarbox (left
        // + right bars), which doesn't affect vertical
        // positioning, so we leave letterboxBottomPct at 0.
      }
      const baseBottomPct = Math.max(0, Math.min(60, subtitleStyle.bottomInsetPct));
      const controlsBumpPct = showControls ? 16 : 0;
      const effectiveBottomPct = Math.min(
        baseBottomPct + letterboxBottomPct + controlsBumpPct,
        80,
      );
      const lineFromTop = 100 - effectiveBottomPct;

      for (let i = 0; i < v.textTracks.length; i++) {
        const track = v.textTracks[i];
        const cues = track.cues;
        if (!cues) continue;
        for (let j = 0; j < cues.length; j++) {
          const cue = cues[j] as VTTCue;
          cue.snapToLines = false;
          cue.line = lineFromTop;
          cue.lineAlign = "end";
        }
      }
    };
    applyAll();

    // Re-apply on every condition that changes the layout: new
    // cues loading (addtrack/cuechange), video metadata
    // arriving (loadedmetadata supplies videoWidth/Height), the
    // player element resizing (fullscreen toggle, window
    // resize, sidebar collapse).
    const onAddTrack = () => applyAll();
    const onCueChange = () => applyAll();
    const onMeta = () => applyAll();
    v.textTracks.addEventListener("addtrack", onAddTrack);
    for (let i = 0; i < v.textTracks.length; i++) {
      v.textTracks[i].addEventListener("cuechange", onCueChange);
    }
    v.addEventListener("loadedmetadata", onMeta);
    v.addEventListener("resize", onMeta);
    const ro = new ResizeObserver(() => applyAll());
    ro.observe(v);
    const fsHandler = () => applyAll();
    document.addEventListener("fullscreenchange", fsHandler);
    return () => {
      v.textTracks.removeEventListener("addtrack", onAddTrack);
      for (let i = 0; i < v.textTracks.length; i++) {
        v.textTracks[i].removeEventListener("cuechange", onCueChange);
      }
      v.removeEventListener("loadedmetadata", onMeta);
      v.removeEventListener("resize", onMeta);
      ro.disconnect();
      document.removeEventListener("fullscreenchange", fsHandler);
    };
  }, [showControls, subtitleStyle.bottomInsetPct]);

  // Inject a global `::cue` stylesheet so the user's appearance
  // prefs apply to BOTH the HLS.js-managed WebVTT sidecar and any
  // external `<track>` we render. Updated reactively on change —
  // no session restart needed since the WebVTT cues themselves
  // aren't styled, only the browser's renderer is.
  //
  // The `::cue` pseudo-element is the standard hook for styling
  // WebVTT captions; CSS variables in here let `background-color`
  // and `color` be controlled live without re-mounting the
  // stylesheet.
  useEffect(() => {
    if (typeof document === "undefined") return;
    const STYLE_ID = "chimpflix-subtitle-style";
    let el = document.getElementById(STYLE_ID) as HTMLStyleElement | null;
    if (!el) {
      el = document.createElement("style");
      el.id = STYLE_ID;
      document.head.appendChild(el);
    }
    const edgeRule = (() => {
      switch (subtitleStyle.edge) {
        case "outline":
          // Multi-shadow trick to fake an outline that survives
          // browsers that don't support text-stroke on ::cue.
          return "text-shadow: -1.5px -1.5px 0 #000, 1.5px -1.5px 0 #000, -1.5px 1.5px 0 #000, 1.5px 1.5px 0 #000 !important;";
        case "shadow":
          return "text-shadow: 2px 2px 4px rgba(0,0,0,0.85) !important;";
        case "none":
        default:
          return "text-shadow: none !important;";
      }
    })();
    // `!important` everywhere because the browsers' UA stylesheet
    // for WebVTT cues uses high-specificity selectors and would
    // otherwise win the cascade against bare `::cue`. The two-
    // selector form (`::cue` and `video::cue`) is also a
    // workaround for older Chrome where the un-prefixed selector
    // didn't always apply to programmatically-created tracks
    // (HLS.js's case). Both `background` shorthand and
    // `background-color` are emitted because Chrome historically
    // only honored the shorthand on ::cue.
    //
    // We *intentionally* don't emit the functional `::cue(c)`,
    // `::cue(v)`, `::cue(i)`, `::cue(b)` variants: Firefox's CSS
    // parser flags them as "Unknown pseudo-class or pseudo-element
    // 'cue'" and drops the entire ruleset, while every other
    // browser accepts them. Inheritance from the bare `::cue`
    // already cascades into the child elements (<c>, <v>, <i>,
    // <b>) so dropping them costs nothing and silences four
    // console warnings per page load in Firefox.
    const font = `-apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif`;
    const cssBlock = `
      background: ${subtitleStyle.backgroundColor} !important;
      background-color: ${subtitleStyle.backgroundColor} !important;
      color: ${subtitleStyle.textColor} !important;
      font-size: ${subtitleStyle.fontSizePx}px !important;
      font-family: ${font} !important;
      line-height: 1.25 !important;
      ${edgeRule}
    `;
    el.textContent = `
      ::cue { ${cssBlock} }
      video::cue { ${cssBlock} }
    `;
    return () => {
      // Don't remove on unmount — other player instances or tab
      // re-entries should keep the style. The element is keyed by
      // id so re-creating is a no-op.
    };
  }, [subtitleStyle]);

  // <track> elements default to `disabled` until JS flips their mode —
  // even with the `default` attribute, autoplay-policy quirks across
  // Chrome/Firefox can leave them hidden. Force `showing` whenever an
  // external sub becomes the active selection, and `disabled` when
  // it's cleared so the off case behaves predictably.
  useEffect(() => {
    const v = videoRef.current;
    if (!v) return;
    const tracks = v.textTracks;
    for (let i = 0; i < tracks.length; i++) {
      tracks[i].mode = externalSubUrl ? "showing" : "disabled";
    }
  }, [externalSubUrl]);

  // Inject scoped ::cue styling for the active video element so external
  // subtitles honor the user's font/color/background prefs. ::cue can
  // only be styled via a stylesheet (not inline), so we mount a <style>
  // node and rewrite its rules whenever the prefs change. The
  // `cf-cue-{id}` class scopes the rules to this one video so multiple
  // open tabs / PiP windows don't fight each other.
  // React's useId guarantees a stable, render-deterministic identifier
  // per component instance — replaces a Math.random() in useRef that
  // violated component purity and could collide across tabs. The
  // class is derived directly (no ref indirection) so it's safe to
  // read during render — fixes the prior `Cannot access refs during
  // render` lint.
  const cueScopeId = useId();
  const cueClass = `cf-cue-${cueScopeId.replace(/:/g, "")}`;
  useEffect(() => {
    const id = cueClass;
    let styleEl = document.getElementById(id) as HTMLStyleElement | null;
    if (!styleEl) {
      styleEl = document.createElement("style");
      styleEl.id = id;
      document.head.appendChild(styleEl);
    }
    const fontSize = Math.max(8, Math.min(128, subtitleStyle.fontSizePx));
    const bg = subtitleStyle.backgroundColor;
    const color = subtitleStyle.textColor;
    const family = cssFontFamilyForSubtitleStyle(subtitleStyle.fontFamily);
    // Edge → CSS text-shadow. Outline = multi-direction tight shadow
    // (poor man's stroke; CSS doesn't have a real text-stroke that
    // composes well with native cue rendering). Shadow = single
    // bottom-right drop. None = no shadow.
    const edgeRule =
      subtitleStyle.edge === "outline"
        ? "text-shadow: -1px -1px 0 #000, 1px -1px 0 #000, -1px 1px 0 #000, 1px 1px 0 #000;"
        : subtitleStyle.edge === "shadow"
          ? "text-shadow: 2px 2px 3px rgba(0,0,0,0.85);"
          : "text-shadow: none;";
    styleEl.textContent = `
      .${id}::cue {
        font-size: ${fontSize}px;
        color: ${color};
        background: ${bg};
        ${family ? `font-family: ${family};` : ""}
        ${edgeRule}
      }
    `;
    return () => {
      // Don't remove on unmount — the same id will be reused on re-mount
      // and removing/recreating causes a flash of unstyled cues. The
      // style node is cheap; let it persist for the page lifetime.
    };
  }, [
    cueClass,
    subtitleStyle.backgroundColor,
    subtitleStyle.textColor,
    subtitleStyle.fontSizePx,
    subtitleStyle.fontFamily,
    subtitleStyle.edge,
  ]);
  const [tracksOpen, setTracksOpen] = useState(false);
  const [playbackRate, setPlaybackRate] = useState(1);
  const [speedOpen, setSpeedOpen] = useState(false);
  const [autoNextCancelled, setAutoNextCancelled] = useState(false);
  const [episodesOpen, setEpisodesOpen] = useState(false);
  const [pipActive, setPipActive] = useState(false);
  const [showRemaining, setShowRemaining] = useState(true);
  /// "Stats for nerds" overlay — surfaces decoded resolution, the
  /// active HLS level, buffer ahead, dropped frames, and the resolved
  /// session info in one place. Toggle with `s` or via the controls
  /// button. Off by default; the panel only samples while visible so
  /// it carries no cost when closed.
  const [statsOpen, setStatsOpen] = useState(false);
  const [hotkeysOpen, setHotkeysOpen] = useState(false);
  // Resume-pill state. When the player mounts at >30s into the file the
  // user is resuming, so we surface a brief "Resumed from 1:23:45 ·
  // Start over" affordance instead of silently dropping them mid-scene
  // (Plex's pattern). Auto-hides after 6s; manual dismiss via the X.
  const [resumePillVisible, setResumePillVisible] = useState(
    startPositionMs > 30_000,
  );
  // Derived: the marker (if any) that contains the current playback time.
  const activeMarkerOverlay = activeMarker(currentTime * 1000, markers);
  // Track the intro markers we've already auto-skipped this session so
  // a user who manually seeks back into the intro isn't yanked
  // forward again. Keyed by the intro's start_ms which is stable
  // across renders.
  const skippedIntrosRef = useRef<Set<number>>(new Set());
  // The user's "no thanks" on the skip pill stores the rejected
  // marker's identity here. When the active marker changes (we leave
  // the region or the next ep loads a fresh one) the dismissal clears
  // and the pill is eligible to render again. Key includes kind so an
  // intro at the same start_ms as a credits marker isn't suppressed
  // by the wrong dismissal.
  const activeMarkerKey = activeMarkerOverlay
    ? `${activeMarkerOverlay.kind}:${activeMarkerOverlay.start_ms}`
    : null;
  const [dismissedMarkerKey, setDismissedMarkerKey] = useState<string | null>(
    null,
  );
  useEffect(() => {
    if (dismissedMarkerKey !== null && dismissedMarkerKey !== activeMarkerKey) {
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setDismissedMarkerKey(null);
    }
  }, [activeMarkerKey, dismissedMarkerKey]);

  // Tracks shown in the picker follow the active version. Versions can
  // differ in stream layout (4K with 3 audio dubs, 1080p with 1), so
  // we look up by id rather than reusing the initial mount's tracks.
  const activeVersion = versions?.find(
    (v) => v.media_file_id === activeMediaFileId,
  );
  // Memoised so downstream useCallback deps don't churn every render.
  // The fallback chain produces a new array literal each time without
  // useMemo, which makes the `?? []` reference unstable and forces
  // any consumer's deps to invalidate on every render.
  const activeAudioTracks = useMemo(
    () => activeVersion?.audioTracks ?? audioTracks ?? [],
    [activeVersion, audioTracks],
  );
  const activeSubtitleTracks = useMemo(
    () => activeVersion?.subtitleTracks ?? subtitleTracks ?? [],
    [activeVersion, subtitleTracks],
  );

  /// Swap the file the session is built against. Resets embedded
  /// audio/subtitle selections — indices are 0-based within the FILE's
  /// streams, so the same number means a different track on a version
  /// with a different stream layout. External subs survive because
  /// their URL is item-scoped, not file-scoped.
  const selectVersion = useCallback(
    (id: number) => {
      if (id === activeMediaFileId) return;
      setAudioSel(undefined);
      setSubtitleSel(undefined);
      setActiveMediaFileId(id);
    },
    [activeMediaFileId],
  );

  const attemptPlay = useCallback(async () => {
    const v = videoRef.current;
    if (!v) return;
    try {
      await v.play();
      setAutoplayBlocked(false);
    } catch (err) {
      if (err instanceof DOMException && err.name === "NotAllowedError") {
        setAutoplayBlocked(true);
      }
    }
  }, []);

  // ── Session setup ────────────────────────────────────────────────────────
  // Asks the Rust backend for a play session, wires up HTML5 <video> for
  // direct play or hls.js for transcode, and tears the session down on
  // unmount. Re-runs when audio/subtitle selection changes (a fresh manifest
  // is required because the transcoder burns subtitles into the video).
  useEffect(() => {
    let cancelled = false;
    let sessionId: string | null = null;
    // Direct mode has no server-side session, so [stream.rs:delete_session]
    // can't resolve the file from a session id alone. We hold the
    // media_file_id locally and include it in the close beacon so the
    // server can fire Trakt scrobble Stop. Null for transcode (the
    // snapshot covers it) and for "no session created yet".
    let directMediaFileId: number | null = null;
    let cleanup: () => void = () => {};
    let keepaliveTimer: number | null = null;

    // Prefer the seek target set by triggerSessionRestart — that's the
    // user's actual intent for THIS re-run. If absent (mount-time
    // resume, or a re-run triggered by an audio/subtitle change rather
    // than a seek), fall back to liveTimeMsRef (current playback
    // position) or the saved startPositionMs. Consume the target
    // immediately so the next re-run that's NOT a seek doesn't accidentally
    // re-use a stale value.
    const pendingTarget = pendingRestartTargetMsRef.current;
    pendingRestartTargetMsRef.current = null;
    const resumeMs =
      pendingTarget !== null
        ? pendingTarget
        : liveTimeMsRef.current > 1000
          ? liveTimeMsRef.current
          : startPositionMs;
    // Keep liveTimeMsRef in sync so a subsequent restart without a
    // seek (e.g. audio track change) doesn't lose the user's current
    // position. Without this, a track switch immediately after a
    // backward seek would skip back to the OLD playback position
    // because pendingTarget would be null and liveTimeMs would still
    // hold the pre-seek value.
    liveTimeMsRef.current = resumeMs;

    async function start() {
      const video = videoRef.current;
      if (!video) return;
      setLoading(true);
      setError(null);

      let resp: Awaited<ReturnType<typeof streamApi.createSession>>;
      try {
        // Only attach a subtitle_style when we're actually burning in.
        // The transcoder uses it as the `force_style=` argument on
        // ffmpeg's `subtitles=` filter — for direct play or external
        // <track> rendering it does nothing.
        const burnedSubtitleStyle =
          subtitleSel !== null && subtitleSel !== undefined
            ? (subtitleStyleToAss(subtitleStyle) ?? undefined)
            : undefined;
        const livePrefs = getPrefs();
        // Detected per-browser support — widens direct-play to HEVC on
        // Safari, AC3 on macOS, VP9 on Chrome/Firefox, etc. Cached for
        // the page lifetime so each session re-roll doesn't redo the
        // canPlayType probes.
        const clientCaps = detectClientCapabilities();
        // Try to adopt a hover-time pre-warmed session. We only
        // qualify on the user-hasn't-touched-anything path because
        // the prewarm was created with default audio/subtitle/quality
        // — any user selection would mismatch what ffmpeg is already
        // encoding. The match contract (mediaFileId + position within
        // tolerance) is enforced inside `consumePrewarm`.
        const noCustomSelection =
          audioSel === undefined &&
          (subtitleSel === undefined || subtitleSel === null) &&
          qualitySel.height === null &&
          bitrateCapSel.bps === null;
        const prewarmed = noCustomSelection
          ? consumePrewarm(activeMediaFileId, resumeMs)
          : null;
        if (prewarmed) {
          resp = { session: prewarmed };
        } else {
          resp = await streamApi.createSession({
            media_file_id: activeMediaFileId,
            start_position_ms: resumeMs,
            audio_index: audioSel,
            subtitle_index: subtitleSel === null ? undefined : subtitleSel,
            subtitle_style: burnedSubtitleStyle,
            quality_target:
              qualitySel.height !== null
                ? { height: qualitySel.height }
                : undefined,
            bitrate_cap_bps: bitrateCapSel.bps ?? undefined,
            // Only send when on — omitting matches Rust's `#[serde(default)]`
            // and keeps the request payload small on the (default) off case.
            audio_normalize: livePrefs.audioNormalize ? true : undefined,
            subtitle_offset_ms:
              subtitleOffsetMs !== 0 ? subtitleOffsetMs : undefined,
            client: {
              supported_video_codecs: clientCaps.video,
              supported_audio_codecs: clientCaps.audio,
              supported_containers: clientCaps.containers,
            },
          });
        }
      } catch (e) {
        if (cancelled) return;
        if (e instanceof ChimpFlixApiError && e.status === 401) {
          router.push(
            "/login?next=" + encodeURIComponent(window.location.pathname),
          );
          return;
        }
        // If we had a working session and the user just changed an
        // audio/subtitle/quality axis, revert to the last known-good
        // combo so they aren't stuck staring at the error overlay.
        // The state update re-runs this effect with the previous
        // selection, which we know works.
        const lastGood = lastGoodTracksRef.current;
        if (
          lastGood &&
          (lastGood.audio !== audioSel ||
            lastGood.subtitle !== subtitleSel ||
            lastGood.quality.height !== qualitySel.height ||
            lastGood.bitrateCap.bps !== bitrateCapSel.bps)
        ) {
          console.warn(
            "[player] track/quality switch failed; reverting to last working selection",
          );
          setAudioSel(lastGood.audio);
          setSubtitleSel(lastGood.subtitle);
          setQualitySel(lastGood.quality);
          setBitrateCapSel(lastGood.bitrateCap);
          return;
        }
        setError("Could not start playback");
        return;
      }

      sessionId = resp.session.id !== "direct" ? resp.session.id : null;
      activeSessionIdRef.current = sessionId;
      // For direct play, remember the file id so teardown can include
      // it in the close beacon — without it the server can't fire
      // Trakt scrobble Stop (no session snapshot to resolve from).
      directMediaFileId =
        resp.session.id === "direct" ? activeMediaFileId : null;

      // If the user navigated away or switched versions/tracks during
      // the round-trip, fire DELETE inline so the orphan transcoder
      // doesn't keep encoding. The cleanup closure already ran before
      // we got here, so it can't see this sessionId. Must include the
      // CSRF token — the server's middleware 403s un-tokened DELETEs
      // and the same orphan-session bug the documented teardown path
      // fixes would otherwise repeat here.
      if (cancelled) {
        if (sessionId) {
          const id = sessionId;
          const csrf = readCsrfToken();
          const headers: Record<string, string> = {};
          if (csrf) headers["X-CSRF-Token"] = csrf;
          fetch(`/api/v1/stream/sessions/${encodeURIComponent(id)}`, {
            method: "DELETE",
            keepalive: true,
            credentials: "include",
            headers,
          }).catch(() => {});
        }
        return;
      }

      // Keepalive: while the player is mounted, ping the master
      // playlist every 60s so the server's idle reaper doesn't kill
      // the session out from under a paused user. HLS.js stops
      // polling once its buffer is full, so a 5-minute pause would
      // otherwise leave us with a dead session and 404 segments on
      // resume. 60s is comfortably under the 5-minute reaper floor
      // and the request is cheap (master.m3u8 is synthesised, not
      // disk-read). Skipped for direct play, which has no session.
      if (sessionId) {
        const id = sessionId;
        keepaliveTimer = window.setInterval(() => {
          fetch(
            `/api/v1/stream/sessions/${encodeURIComponent(id)}/master.m3u8`,
            { credentials: "include" },
          ).catch(() => {
            // Network blip — ignore. HLS auto-recovery handles the
            // user-visible side; we just need to keep trying.
          });
        }, 60_000);
      }
      if (resp.session.duration_ms) {
        setVideoDuration(resp.session.duration_ms / 1000);
      }

      // Capture what the server actually decided to run so the picker
      // can show "Auto · 1080p · NVENC". Direct-play sessions leave
      // everything null (the player isn't transcoding so there's
      // nothing interesting to report).
      setSessionStatus({
        height: resp.session.resolved_height ?? null,
        sourceHeight: resp.session.source_height ?? null,
        encoder: resp.session.encoder ?? null,
        videoTreatment: resp.session.video_treatment ?? null,
        audioTreatment: resp.session.audio_treatment ?? null,
      });

      // Snapshot the combo that just worked. If a subsequent track or
      // quality change fails, the catch branch reverts to this.
      lastGoodTracksRef.current = {
        audio: audioSel,
        subtitle: subtitleSel,
        quality: qualitySel,
        bitrateCap: bitrateCapSel,
      };

      // Transcode sessions have ffmpeg fast-seek to its
      // start_position_ms and HLS.js renders that as media-time 0.
      // Record the offset so source-time reads can add it back.
      // Direct play has no shift.
      //
      // Critical: use the *response's* start_position_ms, not the
      // request's `resumeMs`. When the server adopts an existing
      // compatible session (find_compatible), its start_position_ms
      // is where the encoder originally began — NOT where the user
      // asked to resume. Using `resumeMs` here would misalign HLS-time
      // and source-time, causing every seek to land at the wrong place.
      //
      // …with one important caveat: some HLS configurations (Safari's
      // native player, or HLS.js when the encoder writes a non-zero
      // PROGRAM-DATE-TIME) surface `video.currentTime` already in
      // source-time. If we then add `sessionStartMs` on top, every
      // position read doubles and the user reports "arrow-key jumped
      // super far". `applyResume` measures the actual currentTime
      // after metadata loads and zeroes the offset if it detects
      // source-time mode — this single check protects both reads
      // (`onTimeUpdate`, `report()`) and writes (`seekBy`/`seekTo`).
      sessionStartMsRef.current =
        resp.session.mode === "transcode"
          ? resp.session.start_position_ms
          : 0;

      function applyResume() {
        if (!video) return;
        if (
          resp.session.mode === "transcode" &&
          Number.isFinite(video.currentTime)
        ) {
          const observedSec = video.currentTime;
          const expectedSrcSec = resumeMs / 1000;
          // If the player surfaces source-time already (observed ≈
          // expected, both > 0), drop the offset to 0 so subsequent
          // reads treat currentTime as source-time directly. The 5s
          // tolerance covers fast-seek landing on a keyframe near
          // but not exactly at resumeMs.
          if (
            expectedSrcSec > 1 &&
            Math.abs(observedSec - expectedSrcSec) < 5
          ) {
            sessionStartMsRef.current = 0;
          }
        }
        // For direct play, video.currentTime is source-time, so seek to
        // resumeMs. For transcode where currentTime is HLS-time, the
        // ffmpeg seek already landed us at resumeMs — no extra seek.
        const offsetSec = (resumeMs - sessionStartMsRef.current) / 1000;
        if (offsetSec > 1) {
          video.currentTime = offsetSec;
        }
      }

      if (resp.session.mode === "direct" && resp.session.direct_url) {
        video.src = resp.session.direct_url;
        const onLoaded = () => {
          applyResume();
          attemptPlay();
        };
        video.addEventListener("loadedmetadata", onLoaded, { once: true });
        cleanup = () => video.removeEventListener("loadedmetadata", onLoaded);
        return;
      }

      if (resp.session.mode === "transcode" && resp.session.hls_master_url) {
        const url = resp.session.hls_master_url;
        // Gate native HLS playback on actual Safari, NOT on
        // `canPlayType("application/vnd.apple.mpegurl")`. Android
        // Chrome (and especially Chrome PWAs on Android) advertises
        // HLS support via canPlayType but its demuxer fails with
        // `PipelineStatus::DEMUXER_ERROR_COULD_NOT_PARSE` the moment
        // the manifest hits the pipeline — symptom is a black <video>
        // with controls visible and no useful console output unless
        // the new <video> error listener catches it. Only iOS and
        // macOS Safari actually have working native HLS; every other
        // browser must go through hls.js even if it claims otherwise.
        if (isSafari() && video.canPlayType("application/vnd.apple.mpegurl")) {
          video.src = url;
          const onLoaded = () => {
            applyResume();
            attemptPlay();
          };
          video.addEventListener("loadedmetadata", onLoaded, { once: true });
          cleanup = () =>
            video.removeEventListener("loadedmetadata", onLoaded);
          return;
        }
        // Tracked in the outer scope so the session-cleanup function
        // can clear it. Without that, a session teardown that lands
        // BEFORE `start()` (the inner warmup-finish handler) fires
        // leaves a 4-second timeout in flight referencing a destroyed
        // hls instance; the timer eventually wakes up and tries to
        // call attemptPlay() on a stale closure — visible in profilers
        // as a slow accumulation of detached HTMLVideoElement references.
        let warmupSafetyTimer: number | null = null;
        try {
          const HlsModule = (await import("hls.js")).default;
          if (cancelled) return;
          if (HlsModule.isSupported()) {
            // Mobile browsers have tighter memory + worker constraints
            // than desktop. iOS Safari especially will kill a tab whose
            // MSE buffer crosses ~150 MB; Android Chrome throttles
            // workers in fullscreen. So we halve the buffers and turn
            // off the worker on touch devices — playback gets slightly
            // less stutter cushion, but we stop hitting the OOM /
            // worker-killed paths that manifest as a freeze after 15-30s.
            const isMobile =
              typeof window !== "undefined" &&
              typeof navigator !== "undefined" &&
              (window.matchMedia?.("(hover: none) and (pointer: coarse)").matches ||
                /android|iphone|ipad|ipod/i.test(navigator.userAgent));
            const hls = new HlsModule({
              enableWorker: !isMobile,
              // WebVTT subtitle sidecar support. When the master
              // playlist carries an `#EXT-X-MEDIA:TYPE=SUBTITLES`
              // group (server-side: text subs go out as sidecar
              // instead of being burned into video), HLS.js loads
              // the WebVTT and creates a `TextTrack` on the
              // <video> element. `renderTextTracksNatively=true`
              // lets the browser render the overlay itself —
              // standard captions, no canvas tricks. Without
              // this opt-in HLS.js leaves the track in `disabled`
              // mode and the user sees no subs even though the
              // sidecar is loaded. `subtitleDisplay` lives on the
              // hls instance (not the config), so it's set on the
              // hls object after construction below.
              enableWebVTT: true,
              renderTextTracksNatively: true,
              // Keep 30 s of played-out segments in the back buffer so
              // small back-seeks (re-watching a line of dialog) don't
              // need to refetch. Larger back buffer trades RAM for
              // smoother scrubbing. Trimmed on mobile.
              backBufferLength: isMobile ? 10 : 30,
              // Forward buffer targets. ffmpeg writes a 6 s segment
              // every ~1 s on a healthy box (NVDEC + NVENC pipeline),
              // so we can comfortably target a 60 s ahead buffer on
              // desktop. Mobile gets a tighter 30s window to avoid
              // tripping the browser's memory-pressure killer.
              maxBufferLength: isMobile ? 30 : 60,
              maxMaxBufferLength: isMobile ? 60 : 120,
              // Hard cap on buffer size in MB. 1080p ~5 Mbps × 60 s ≈
              // 38 MB; desktop gets 200 MB of headroom, mobile gets
              // 120 MB which is still under iOS Safari's per-tab MSE
              // budget (~150 MB before the OS kills the page). 60 MB
              // was the previous value but tripped the ceiling on
              // higher-bitrate streams (8-10 Mbps × 30 s ≈ 30-40 MB
              // with two segments in flight), forcing HLS.js to pause
              // appending and starving playback even though the network
              // had bandwidth available.
              maxBufferSize: isMobile ? 120 * 1000 * 1000 : 200 * 1000 * 1000,
              // First-manifest timeouts have to cover the server's
              // wait for ffmpeg to write `index.m3u8` (now 30s after
              // the bump to handle slow boxes). Variant + fragment
              // loads stay tighter so mid-playback hiccups surface
              // quickly.
              manifestLoadingTimeOut: 35000,
              manifestLoadingMaxRetry: 4,
              manifestLoadingRetryDelay: 500,
              levelLoadingTimeOut: 15000,
              levelLoadingMaxRetry: 4,
              fragLoadingTimeOut: 20000,
              fragLoadingMaxRetry: 6,
              // Initial bandwidth estimate before HLS.js observes real
              // throughput. 5 Mbps was too optimistic on mobile —
              // jittery 4G/5G picks a high quality, segments arrive
              // late, the stall watchdog kicks in and the user sees
              // a jitter or skip on the first few seconds. 2.5 Mbps
              // is conservative; the ABR model converges within ~3
              // segments so the only cost is a brief lower-quality
              // start. Desktop keeps the higher estimate.
              abrEwmaDefaultEstimate: isMobile ? 2_500_000 : 5_000_000,
            });
            hlsRef.current = hls;
            hls.loadSource(url);
            hls.attachMedia(video);
            hls.on(HlsModule.Events.MANIFEST_PARSED, () => {
              applyResume();
              // Belt-and-suspenders for the WebVTT sidecar: even
              // though the master playlist marks the subtitle
              // group DEFAULT=YES, some HLS.js versions leave it
              // disabled until something explicitly opts in.
              // Setting `subtitleTrack` to the first available
              // track index here matches what every other player
              // does ("user picked this sub, show it").
              if (hls.subtitleTracks && hls.subtitleTracks.length > 0) {
                hls.subtitleTrack = 0;
                hls.subtitleDisplay = true;
              }
              // Pre-roll warmup: wait until the player has 15s of
              // forward buffer (~2.5 segments) before calling .play().
              //
              // The earlier 6s threshold was meant to cover one
              // segment of headroom, but in practice produced a
              // visible mid-second-1 stutter on fresh sessions. Root
              // cause is the ffmpeg manifest cadence: the playlist is
              // EVENT-type / `hls_list_size 0`, so it grows as new
              // segments arrive — but HLS.js only re-polls the manifest
              // every `targetduration` (= 6s) seconds. The sequence we
              // were hitting:
              //
              //   wall=0  load playlist v0; lists [seg0]
              //   wall=1  seg0 download done → warmup gate fires
              //   wall=1  .play() called; playback consumes seg0
              //   wall=6  playlist re-polled, learns about seg1
              //   wall=6+ seg1 download starts
              //   wall=7  playback reaches end of seg0; seg1 not yet
              //           in MSE buffer → STALL → browser shows the
              //           native loading spinner
              //
              // Bumping the warmup to 15s ensures that by the time
              // .play() fires, the manifest has been polled at least
              // once or twice and segments 0+1 (and often 2) are
              // already in the MSE buffer. The encoder + manifest
              // pipeline then keeps pace with real-time playback for
              // the rest of the session.
              //
              // User preference (2026-05-21): "I'd rather show the
              // loading screen / circle longer if it hid whatever
              // causes that stutter." Bias toward longer warmup over
              // visible buffer underruns.
              //
              // Safety timeout bumped in lockstep so a genuinely
              // slow encoder still falls through to .play() within a
              // reasonable window rather than spinning forever.
              //
              // We also gate on `video.readyState >= HAVE_FUTURE_DATA`
              // — the browser's own "ready to play through" signal —
              // because `buffered.end` can lag the decoder for a few
              // hundred ms after `FRAG_BUFFERED` fires.
              const WARMUP_TARGET_SEC = 15;
              const WARMUP_TIMEOUT_MS = 15000;
              let started = false;
              const start = () => {
                if (started) return;
                started = true;
                hls.off(HlsModule.Events.FRAG_BUFFERED, onFrag);
                if (warmupSafetyTimer !== null) {
                  window.clearTimeout(warmupSafetyTimer);
                  warmupSafetyTimer = null;
                }
                attemptPlay();
              };
              // Find the buffered range that contains `currentTime`
              // and return its forward extent. `buffered.end(0)` would
              // be wrong after a resume: if the user resumes at
              // 1:23:45 and HLS has buffered [1:23:40, 1:23:55], that
              // range is index 0 only if no other range exists.
              // After a seek-during-warmup it could be index 1.
              const aheadFromCurrent = (): number => {
                const buf = video.buffered;
                const t = video.currentTime;
                for (let i = 0; i < buf.length; i++) {
                  // Tiny epsilon to absorb floating-point rounding —
                  // MSE timestamps occasionally land .001 past the
                  // exact play head.
                  if (t >= buf.start(i) - 0.05 && t <= buf.end(i) + 0.05) {
                    return buf.end(i) - t;
                  }
                }
                return 0;
              };
              const onFrag = () => {
                if (video.buffered.length === 0) return;
                if (
                  aheadFromCurrent() >= WARMUP_TARGET_SEC &&
                  video.readyState >= 3 /* HAVE_FUTURE_DATA */
                ) {
                  start();
                }
              };
              hls.on(HlsModule.Events.FRAG_BUFFERED, onFrag);
              warmupSafetyTimer = window.setTimeout(start, WARMUP_TIMEOUT_MS);
            });
            // Fatal-error recovery: HLS.js docs recommend trying
            // `recoverMediaError()` for media errors and `startLoad()`
            // for network errors before declaring the session dead.
            // recoverMediaError can be called up to twice; on the
            // second pass it ALSO swaps audio codec to handle a
            // particularly stubborn class of MSE errors. We track
            // recovery attempts per session so a genuinely broken
            // stream still surfaces an error eventually instead of
            // looping forever.
            let mediaRecoveryAttempts = 0;
            const MAX_MEDIA_RECOVERY = 2;
            hls.on(HlsModule.Events.ERROR, (_e, data) => {
              if (!data.fatal) return;
              setReconnecting(true);
              switch (data.type) {
                case HlsModule.ErrorTypes.NETWORK_ERROR:
                  // Most network errors clear on retry — segments may
                  // be slow because ffmpeg is still encoding ahead of
                  // playback. `startLoad()` resumes loading at the
                  // current position with the existing buffer intact.
                  hls.startLoad();
                  // Clear the reconnecting overlay after a beat; if
                  // the error returns we'll flip it back on.
                  window.setTimeout(() => setReconnecting(false), 1500);
                  return;
                case HlsModule.ErrorTypes.MEDIA_ERROR:
                  if (mediaRecoveryAttempts < MAX_MEDIA_RECOVERY) {
                    mediaRecoveryAttempts += 1;
                    if (mediaRecoveryAttempts === MAX_MEDIA_RECOVERY) {
                      // Second attempt: swap codec first. The HLS.js
                      // FAQ specifically recommends this sequence for
                      // tough MSE decode errors.
                      hls.swapAudioCodec();
                    }
                    hls.recoverMediaError();
                    window.setTimeout(() => setReconnecting(false), 1500);
                    return;
                  }
                  break;
                default:
                  // Mux / other errors aren't recoverable in the
                  // generic sense — fall through to the error overlay.
                  break;
              }
              setReconnecting(false);
              setError(friendlyHlsError(data));
            });
            cleanup = () => {
              hlsRef.current = null;
              if (warmupSafetyTimer !== null) {
                window.clearTimeout(warmupSafetyTimer);
                warmupSafetyTimer = null;
              }
              hls.destroy();
              // Firefox holds onto the MediaSource and SourceBuffers
              // after hls.destroy() unless we also clear the video
              // element's src and force a reload — without this,
              // every session restart leaks one MediaSource (with its
              // buffered segments) into memory. Symptoms: gradual
              // memory growth + Firefox-specific playback slowdown
              // after a dozen+ seeks/track-changes in a single page
              // session. Chrome/Safari release the MediaSource on
              // destroy() alone, but the extra pair of calls is
              // harmless there.
              try {
                video.removeAttribute("src");
                video.load();
              } catch (e) {
                // The expected error is `InvalidStateError` on an
                // element whose MediaSource was already detached;
                // that's the no-op case we deliberately allow.
                // Anything else (OOM, unusual DOMException) is worth
                // surfacing — bare `catch {}` masked real bugs during
                // development, so we now warn loudly and only swallow
                // the specific expected case.
                if (
                  !(e instanceof DOMException) ||
                  e.name !== "InvalidStateError"
                ) {
                  console.warn(
                    "ChimpFlixPlayer: unexpected error during MSE cleanup",
                    e,
                  );
                }
              }
            };
            return;
          }
        } catch (e) {
          setError(e instanceof Error ? e.message : String(e));
          return;
        }
        setError("HLS not supported in this browser");
        return;
      }

      setError("Server returned an unplayable session");
    }

    // Tear down the transcode session. Two paths:
    //
    //   * In-SPA cleanup (resumeEpoch++ for a session restart, audio
    //     track change, etc.): use fetch+keepalive with an explicit
    //     X-CSRF-Token header. Without the token the server's CSRF
    //     middleware rejects with 403 and the session lingers — which
    //     was the cause of the "unable to jump back" bug, because
    //     find_compatible would then adopt the still-alive session on
    //     the next POST /sessions.
    //   * True page unload (pagehide non-persisted): sendBeacon is the
    //     only reliable transport, and the `/close` endpoint is
    //     explicitly exempted from the double-submit token check on
    //     the server side (Origin + session-cookie + per-handler
    //     ownership check still apply).
    function teardownSession(opts?: { unload?: boolean }) {
      // Transcode → use the real session id. Direct → use literal
      // "direct" and pass the file id so [stream.rs:delete_session]
      // can fire Trakt scrobble Stop (without it the server can't
      // resolve the owning item/episode and the live "YOU ARE
      // WATCHING" banner stays up until Trakt times it out).
      const idForClose = sessionId ?? (directMediaFileId !== null ? "direct" : null);
      if (!idForClose) return;
      const directQuery =
        idForClose === "direct" && directMediaFileId !== null
          ? `?media_file_id=${directMediaFileId}`
          : "";
      const closeUrl = `/api/v1/stream/sessions/${encodeURIComponent(idForClose)}/close${directQuery}`;
      const deleteUrl = `/api/v1/stream/sessions/${encodeURIComponent(idForClose)}${directQuery}`;
      try {
        if (
          opts?.unload &&
          typeof navigator !== "undefined" &&
          typeof navigator.sendBeacon === "function"
        ) {
          // sendBeacon is fire-and-forget POST; the browser guarantees
          // delivery even if the page is unloading. Empty body — the
          // session id is encoded in the URL. Server-side exemption
          // covers the missing CSRF token.
          const beaconOk = navigator.sendBeacon(closeUrl, new Blob());
          if (beaconOk) return;
        }
        // In-SPA path: keepalive fetch with the CSRF token. DELETE keeps
        // the existing semantics; the server's middleware accepts the
        // token via X-CSRF-Token mirroring the cf_csrf cookie value.
        const csrf = readCsrfToken();
        const headers: Record<string, string> = {};
        if (csrf) headers["X-CSRF-Token"] = csrf;
        fetch(deleteUrl, {
          method: "DELETE",
          keepalive: true,
          credentials: "include",
          headers,
        }).catch(() => {});
      } catch {
        // Fetch / sendBeacon can throw synchronously during unload on
        // some browsers — we tried, the 90s server-side reaper will
        // mop up either way.
      }
    }
    // pagehide is the canonical "the page is going away" signal. We use
    // `event.persisted` to distinguish bfcache (page may come back, don't
    // tear down) from real unload (close, hard navigation, system kill).
    // On mobile this catches force-close of the PWA: Chrome fires pagehide
    // with persisted=false when the OS unloads the page. App-switch (where
    // the user comes back) only fires visibilitychange, not pagehide, so
    // we don't falsely kill sessions for backgrounding. The server's
    // 90s reaper picks up sessions where pagehide didn't fire at all
    // (sudden process kill, network blip during keepalive).
    const onPageHide = (e: PageTransitionEvent) => {
      if (e.persisted) return;
      teardownSession({ unload: true });
    };
    window.addEventListener("pagehide", onPageHide);

    // Page Lifecycle freeze/resume — Chrome (incl. PWAs) can freeze
    // backgrounded tabs to save memory after 5+ minutes idle. While
    // frozen, all JS pauses, including our 60s keepalive interval.
    // On resume we fire one keepalive immediately so the next 60s
    // tick doesn't race the server's idle-reaper threshold.
    //
    // No-op on browsers that don't fire these events — the keepalive
    // interval continues working normally for active tabs.
    const onResume = () => {
      if (!sessionId) return;
      fetch(
        `/api/v1/stream/sessions/${encodeURIComponent(sessionId)}/master.m3u8`,
        { credentials: "include" },
      ).catch(() => {
        // Network blip on resume — the next interval tick will retry.
      });
    };
    document.addEventListener("resume", onResume);

    // iOS Safari (incl. PWA on iOS) doesn't fire the Page Lifecycle
    // `resume` event. Instead, returning from the bfcache fires
    // `pageshow` with `persisted: true`. Same recovery as the Chrome
    // path — one immediate keepalive to repair `last_seen` before the
    // 60s interval ticks. Without this, an iPhone user who backgrounds
    // the PWA for >5 minutes comes back to a 404 manifest because the
    // server reaper has already culled the session.
    //
    // We also fire on the *non*-persisted pageshow (fresh load) so the
    // path is one-and-the-same; the cost is a single extra HTTP roundtrip
    // at page load that would have been redundant with `start()` anyway.
    const onPageShow = (e: PageTransitionEvent) => {
      if (!e.persisted) return;
      if (!sessionId) return;
      fetch(
        `/api/v1/stream/sessions/${encodeURIComponent(sessionId)}/master.m3u8`,
        { credentials: "include" },
      ).catch(() => {});
    };
    window.addEventListener("pageshow", onPageShow);

    start();

    return () => {
      cancelled = true;
      cleanup();
      window.removeEventListener("pagehide", onPageHide);
      window.removeEventListener("pageshow", onPageShow);
      document.removeEventListener("resume", onResume);
      if (keepaliveTimer !== null) {
        window.clearInterval(keepaliveTimer);
      }
      if (sessionId) {
        // Use the same keepalive path: even React unmounts can coincide
        // with the page going away (Back button after watching).
        teardownSession();
      }
      activeSessionIdRef.current = null;
    };
    // Re-running on audio/subtitle changes tears down the existing session
    // and asks for a new one with the chosen tracks. `startPositionMs` is
    // intentionally captured once via liveTimeMsRef so a deps change here
    // doesn't restart playback from the original resume point.
    // `resumeEpoch` is bumped by seekTo() when the user seeks before the
    // current session's start — the HLS stream doesn't include those
    // segments, so we need a fresh ffmpeg rooted at the new position.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    activeMediaFileId,
    audioSel,
    subtitleSel,
    subtitleOffsetMs,
    qualitySel,
    bitrateCapSel,
    resumeEpoch,
  ]);

  // ── Video state subscriptions ────────────────────────────────────────────
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;

    const onLoadedMetadata = () => {
      // Only adopt video.duration if the server didn't give us one. Server
      // metadata is authoritative for HLS where video.duration grows over
      // time as segments arrive. For transcode, video.duration is also
      // HLS media-time (0 to total-sessionStart), so add the offset back
      // to land in source-time.
      if (!durationMs && Number.isFinite(video.duration)) {
        setVideoDuration(video.duration + sessionStartMsRef.current / 1000);
      }
    };
    const onTimeUpdate = () => {
      // For transcode, video.currentTime is HLS media-time (0-based at
      // the session start). Add the session's source-time offset so
      // downstream consumers see file-timeline values.
      const srcTimeSec = video.currentTime + sessionStartMsRef.current / 1000;
      setCurrentTime(srcTimeSec);
      liveTimeMsRef.current = Math.floor(srcTimeSec * 1000);
    };
    const onProgress = () => {
      // Track the trailing edge of whatever buffered range contains
      // currentTime. We don't sum the whole buffered set because
      // gaps (rare with HLS, but possible after a backward seek)
      // would mislead the UI into showing "buffered ahead" through
      // empty regions. The range containing currentTime is the only
      // contiguous run the user can play through without rebuffer.
      const now = video.currentTime;
      let end = now;
      for (let i = 0; i < video.buffered.length; i++) {
        if (video.buffered.start(i) <= now && video.buffered.end(i) >= now) {
          end = video.buffered.end(i);
          break;
        }
      }
      setBufferedEnd(end + sessionStartMsRef.current / 1000);
    };
    const onPlay = () => {
      setPlaying(true);
      setAutoplayBlocked(false);
    };
    const onPause = () => setPlaying(false);
    const onWaiting = () => setLoading(true);
    const onCanPlay = () => {
      setLoading(false);
      if (video.paused && !autoplayBlocked) {
        attemptPlay();
      }
    };
    const onPlaying = () => {
      setLoading(false);
      setPlaying(true);
      setAutoplayBlocked(false);
    };
    const onVolumeChange = () => {
      setMuted(video.muted);
      setVolume(video.volume);
    };
    // Surface HTMLMediaElement errors directly. HLS.js's MEDIA_ERROR
    // path catches most decode failures, but not all of them — Android
    // Chrome (especially in standalone PWA mode) can put MediaCodec into
    // a "no decoded output" state that hls.js never sees because the
    // segment buffer keeps receiving valid appends. Without this
    // listener the player just sits black with the controls visible,
    // which is exactly the failure mode bug reports keep describing.
    // We log the MediaError code + the network/ready states so the
    // remote-debug DevTools console has something to grep for, and we
    // pop the error overlay with a human-readable label.
    const onMediaError = () => {
      const err = video.error;
      const code = err?.code ?? 0;
      const message = err?.message ?? "";
      const label = ((): string => {
        switch (code) {
          case 1:
            return "playback aborted (MEDIA_ERR_ABORTED)";
          case 2:
            return "network error fetching media (MEDIA_ERR_NETWORK)";
          case 3:
            return "media decode failed (MEDIA_ERR_DECODE) — codec/level rejected by browser";
          case 4:
            return "media not supported (MEDIA_ERR_SRC_NOT_SUPPORTED)";
          default:
            return `unknown <video> error (code=${code})`;
        }
      })();
      console.error("[chimpflix] <video> error", {
        code,
        message,
        networkState: video.networkState,
        readyState: video.readyState,
        currentSrc: video.currentSrc,
      });
      setError(message ? `${label}: ${message}` : label);
      setReconnecting(false);
    };

    video.addEventListener("loadedmetadata", onLoadedMetadata);
    video.addEventListener("timeupdate", onTimeUpdate);
    video.addEventListener("timeupdate", onProgress);
    video.addEventListener("progress", onProgress);
    video.addEventListener("play", onPlay);
    video.addEventListener("pause", onPause);
    video.addEventListener("waiting", onWaiting);
    video.addEventListener("canplay", onCanPlay);
    video.addEventListener("playing", onPlaying);
    video.addEventListener("volumechange", onVolumeChange);
    video.addEventListener("error", onMediaError);

    return () => {
      video.removeEventListener("loadedmetadata", onLoadedMetadata);
      video.removeEventListener("timeupdate", onTimeUpdate);
      video.removeEventListener("timeupdate", onProgress);
      video.removeEventListener("progress", onProgress);
      video.removeEventListener("play", onPlay);
      video.removeEventListener("pause", onPause);
      video.removeEventListener("waiting", onWaiting);
      video.removeEventListener("canplay", onCanPlay);
      video.removeEventListener("playing", onPlaying);
      video.removeEventListener("volumechange", onVolumeChange);
      video.removeEventListener("error", onMediaError);
    };
  }, [attemptPlay, autoplayBlocked, durationMs]);

  // Stall-recovery watchdog. Two paths to detect a stall:
  //   1. `waiting` event — the browser tells us directly that playback
  //      has paused because the source buffer is empty. Fast and exact.
  //      We arm a short timer on `waiting` and disarm on `playing`/`pause`.
  //   2. Polling fallback — for the silent decoder wedge case where no
  //      event fires but currentTime stops moving. Slower (6s).
  // The kick is the same in both cases: ask HLS to resume loading, nudge
  // currentTime to wake the decoder, and re-issue play(). If kicks pile
  // up in a minute, the stream itself is the problem — surface the
  // error overlay so the user can refresh.
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    const WAITING_TIMEOUT_MS = 4000;
    const POLL_STALL_MS = 6000;
    const KICK_WINDOW_MS = 60_000;
    const MAX_KICKS = 3;
    // A "clean run" of this many uninterrupted seconds after a kick
    // means the recovery worked — clear the kicks budget so a later,
    // unrelated stall doesn't inherit those strikes and trip the
    // give-up overlay prematurely.
    const KICK_RESET_AFTER_CLEAN_SEC = 15;
    let lastAdvanceAt = Date.now();
    let lastTime = video.currentTime;
    let waitingTimer: number | null = null;
    let cleanSinceKickAt: number | null = null;
    const kicks: number[] = [];
    const tryKick = (reason: "waiting" | "poll") => {
      if (video.paused || video.ended) return;
      // Don't kick during a user-initiated scrub — the +0.001 nudge
      // would compete with the drag and visibly skip the playhead.
      if (scrubbingRef.current || video.seeking) return;
      const now = Date.now();
      while (kicks.length && kicks[0] < now - KICK_WINDOW_MS) kicks.shift();
      if (kicks.length >= MAX_KICKS) {
        setError("Playback stalled and could not recover. Try refreshing.");
        return;
      }
      kicks.push(now);
      cleanSinceKickAt = now;
      lastAdvanceAt = now;
      lastTime = video.currentTime;
      const hls = hlsRef.current;
      if (hls) {
        try {
          hls.startLoad();
        } catch {}
      }
      try {
        // Tiny forward nudge wakes a wedged decoder. A no-op assignment
        // is sometimes ignored; +0.001 forces a real seek that pulls
        // the next available sample from the source buffer. Capped at
        // duration so we don't try to seek past the end.
        const target = video.currentTime + 0.001;
        if (
          !Number.isFinite(video.duration) ||
          target < video.duration - 0.1
        ) {
          video.currentTime = target;
        }
      } catch {}
      void video.play().catch(() => {});
      // Re-arm the waiting timer if the kick was triggered by it —
      // a kick that didn't help should trigger another after the
      // same threshold.
      if (reason === "waiting" && waitingTimer === null) {
        waitingTimer = window.setTimeout(
          () => tryKick("waiting"),
          WAITING_TIMEOUT_MS,
        );
      }
    };
    const onWaiting = () => {
      if (waitingTimer !== null) return;
      waitingTimer = window.setTimeout(
        () => tryKick("waiting"),
        WAITING_TIMEOUT_MS,
      );
    };
    const cancelWaiting = () => {
      if (waitingTimer !== null) {
        window.clearTimeout(waitingTimer);
        waitingTimer = null;
      }
    };
    video.addEventListener("waiting", onWaiting);
    video.addEventListener("playing", cancelWaiting);
    video.addEventListener("pause", cancelWaiting);
    video.addEventListener("seeking", cancelWaiting);
    const onTick = () => {
      if (video.paused || video.ended || video.seeking) {
        lastAdvanceAt = Date.now();
        lastTime = video.currentTime;
        return;
      }
      if (video.currentTime > lastTime + 0.05) {
        const now = Date.now();
        lastAdvanceAt = now;
        lastTime = video.currentTime;
        // Forgive past kicks once playback has been advancing
        // smoothly for a while — otherwise three unrelated stalls
        // spaced 20s apart all count against the 60s budget and
        // surface the unrecoverable-error overlay even though each
        // one recovered on its own.
        if (
          cleanSinceKickAt !== null &&
          now - cleanSinceKickAt > KICK_RESET_AFTER_CLEAN_SEC * 1000
        ) {
          kicks.length = 0;
          cleanSinceKickAt = null;
        }
        return;
      }
      if (Date.now() - lastAdvanceAt < POLL_STALL_MS) return;
      tryKick("poll");
    };
    const interval = window.setInterval(onTick, 2000);
    return () => {
      window.clearInterval(interval);
      cancelWaiting();
      video.removeEventListener("waiting", onWaiting);
      video.removeEventListener("playing", cancelWaiting);
      video.removeEventListener("pause", cancelWaiting);
      video.removeEventListener("seeking", cancelWaiting);
    };
  }, []);

  // Apply persisted prefs (volume, muted, playback rate) on mount.
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    const saved = getPrefs();
    video.volume = saved.volume;
    video.muted = saved.muted;
    video.playbackRate = saved.playbackRate;
    setPlaybackRate(saved.playbackRate);
  }, []);

  // Fullscreen tracking. iOS Safari (and standalone PWAs) doesn't
  // dispatch `fullscreenchange` because it doesn't implement the
  // standard Element.requestFullscreen API — it uses the older
  // `webkitEnterFullscreen` on HTMLVideoElement, which fires
  // `webkitbeginfullscreen` / `webkitendfullscreen` ON THE VIDEO
  // element. Without those listeners, the controls' fullscreen icon
  // would never flip to "exit" on iPhone, leaving keyboard / a11y
  // users with no clear out-of-fullscreen signal.
  useEffect(() => {
    const onChange = () =>
      setIsFullscreen(Boolean(document.fullscreenElement));
    document.addEventListener("fullscreenchange", onChange);
    const video = videoRef.current;
    const onWebkitBegin = () => setIsFullscreen(true);
    const onWebkitEnd = () => setIsFullscreen(false);
    if (video) {
      video.addEventListener("webkitbeginfullscreen", onWebkitBegin);
      video.addEventListener("webkitendfullscreen", onWebkitEnd);
    }
    return () => {
      document.removeEventListener("fullscreenchange", onChange);
      if (video) {
        video.removeEventListener("webkitbeginfullscreen", onWebkitBegin);
        video.removeEventListener("webkitendfullscreen", onWebkitEnd);
      }
    };
  }, []);

  // PiP tracking.
  //
  // Browsers (Chrome on Android in particular) like to pause the video
  // when the PiP window is dismissed. From the viewer's perspective
  // that's wrong: they pressed "close PiP", not "pause". Capture the
  // pre-enter playing state and restore it on leave so closing the PiP
  // window is a no-op for playback.
  useEffect(() => {
    const v = videoRef.current;
    if (!v) return;
    let wasPlayingBeforePip = false;
    const onEnter = () => {
      wasPlayingBeforePip = !v.paused;
      setPipActive(true);
    };
    const onLeave = () => {
      setPipActive(false);
      if (wasPlayingBeforePip && v.paused) {
        v.play().catch(() => {
          // Autoplay policy can refuse — best-effort, user can hit
          // play again. Don't surface this as a user-visible error.
        });
      }
    };
    v.addEventListener("enterpictureinpicture", onEnter);
    v.addEventListener("leavepictureinpicture", onLeave);
    return () => {
      v.removeEventListener("enterpictureinpicture", onEnter);
      v.removeEventListener("leavepictureinpicture", onLeave);
    };
  }, []);

  // Screen Wake Lock. While the user is actively watching (playing,
  // page visible) we hold a screen wake lock so the phone doesn't dim
  // and turn off mid-episode. The lock auto-releases on visibility
  // change to hidden (browser policy) so we re-acquire on the visible
  // transition. Gracefully no-op on browsers without the API.
  useEffect(() => {
    if (typeof navigator === "undefined") return;
    const wl = (navigator as Navigator & {
      wakeLock?: {
        request: (type: "screen") => Promise<{
          released: boolean;
          release: () => Promise<void>;
        }>;
      };
    }).wakeLock;
    if (!wl) return;
    let sentinel: { release: () => Promise<void> } | null = null;
    let cancelled = false;
    const acquire = async () => {
      if (cancelled || !playing) return;
      if (document.visibilityState !== "visible") return;
      try {
        const s = await wl.request("screen");
        if (cancelled) {
          await s.release().catch(() => {});
          return;
        }
        sentinel = s;
      } catch {
        // NotAllowed (page hidden, low battery saver, etc.) — skip.
      }
    };
    const release = async () => {
      const s = sentinel;
      sentinel = null;
      if (s) await s.release().catch(() => {});
    };
    const onVisibilityChange = () => {
      if (document.visibilityState === "visible") {
        void acquire();
      }
    };
    if (playing) void acquire();
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => {
      cancelled = true;
      void release();
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  }, [playing]);

  // Media Session API. Wires up lock-screen and notification-shade
  // playback controls on Android (and iOS Safari). Without this the
  // OS shows generic "Chrome is playing audio" controls; with it we
  // get title and play/pause/seek buttons. Seeks route through
  // `seekByRef` so transcode sessions (HLS media-time != source-time)
  // get the same source-time-aware seek path as the on-screen buttons —
  // calling video.currentTime directly here would jump to the wrong
  // position once `-ss N` offset is in play.
  useEffect(() => {
    if (typeof navigator === "undefined") return;
    if (!("mediaSession" in navigator)) return;
    const ms = navigator.mediaSession;
    const v = videoRef.current;
    if (!v) return;
    ms.metadata = new MediaMetadata({
      title,
      artist: subtitle ?? undefined,
    });
    const onPlay = () => {
      void v.play().catch(() => {});
    };
    const onPause = () => v.pause();
    const onSeekBackward = (details: MediaSessionActionDetails) => {
      const offset = details.seekOffset ?? 10;
      seekByRef.current?.(-offset);
    };
    const onSeekForward = (details: MediaSessionActionDetails) => {
      const offset = details.seekOffset ?? 10;
      seekByRef.current?.(offset);
    };
    try {
      ms.setActionHandler("play", onPlay);
      ms.setActionHandler("pause", onPause);
      ms.setActionHandler("seekbackward", onSeekBackward);
      ms.setActionHandler("seekforward", onSeekForward);
    } catch {
      // Some browsers don't support all action types.
    }
    return () => {
      try {
        ms.setActionHandler("play", null);
        ms.setActionHandler("pause", null);
        ms.setActionHandler("seekbackward", null);
        ms.setActionHandler("seekforward", null);
      } catch {
        // ignore
      }
    };
  }, [title, subtitle]);

  // Periodic play-state updates + scrobble at threshold.
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;

    // Server validates that exactly one of item_id / episode_id is set, so
    // pick the more specific one when both are passed in.
    const target = episodeId
      ? { episode_id: episodeId }
      : itemId
        ? { item_id: itemId }
        : null;

    function report() {
      if (!video) return;
      if (!target) return;
      // Persist source-time, not HLS media-time. video.duration is HLS
      // duration (truncated for transcode sessions) and isn't usable —
      // prefer the server's full-file `videoDuration`.
      const positionMs = Math.floor(
        video.currentTime * 1000 + sessionStartMsRef.current,
      );
      const knownDurationMs =
        videoDuration > 0 ? Math.floor(videoDuration * 1000) : undefined;
      // Position updates only while the user is actively watching.
      // Paused/ended state has no new position to report (the pause /
      // ended event handler captured the final position already) and
      // we don't want to keep nudging `last_played_at` while the
      // tile is sitting in the background.
      if (!video.paused && !video.ended) {
        playStateApi
          .update({
            ...target,
            position_ms: positionMs,
            duration_ms: knownDurationMs,
          })
          .catch(() => {});
      }
      // Threshold scrobble has to run regardless of play state. The
      // common "binge to natural end → next episode auto-plays" path
      // ends with `video.ended = true` *before* the next 10s tick
      // ever fires — bailing here means the scrobble at 90% never
      // gets sent, and the operator has to manually mark each
      // finished episode. (This was the symptom users hit: "I
      // watched the whole thing and it didn't mark watched.")
      if (!scrobbledRef.current && knownDurationMs) {
        // Threshold scrobble: position past the configured percentage.
        const pastThreshold =
          positionMs / knownDurationMs >= scrobbleThreshold;
        // Credits-marker scrobble: any auto-detected `credits` marker
        // whose start_ms we've passed. Used when the operator picks
        // `first_credits_marker` or `earliest_of_both`. The first
        // such marker (markers are sorted by start_ms upstream) wins.
        const behaviour = completionBehaviour ?? "threshold_pct";
        const wantMarker =
          behaviour === "first_credits_marker" ||
          behaviour === "earliest_of_both";
        const firstCredits = wantMarker
          ? markers?.find((m) => m.kind === "credits") ?? null
          : null;
        const pastCreditsMarker =
          firstCredits != null && positionMs >= firstCredits.start_ms;
        // `first_credits_marker` falls back to the threshold when the
        // file has no marker — otherwise long files without detected
        // markers would never scrobble.
        const shouldScrobble = (() => {
          switch (behaviour) {
            case "first_credits_marker":
              return firstCredits ? pastCreditsMarker : pastThreshold;
            case "earliest_of_both":
              return pastCreditsMarker || pastThreshold;
            default:
              return pastThreshold;
          }
        })();
        if (shouldScrobble) {
          scrobbledRef.current = true;
          playStateApi.scrobble(target).catch(() => {});
        }
      }
    }

    const interval = window.setInterval(report, PLAY_STATE_INTERVAL_MS);
    const onPause = () => {
      // Just persist position. We used to also SIGSTOP ffmpeg here to
      // save GPU during long pauses, but the mobile-PWA pause/play
      // event pair is unreliable (Chrome PWA fires `pause` on visibility
      // hints + various lifecycle moments without a matching `play`),
      // which left ffmpeg permanently SIGSTOP'd and the player wedged.
      // NVENC is cheap enough that letting the encoder run ahead is
      // strictly better than the leak risk.
      report();
    };
    const onEnded = () => report();
    // Seeking is the one input where a 10 s poll can drop the user's
    // position on reload — they scrub to 1:30:00, the polling tick
    // hasn't fired yet, they close the tab. Without this listener the
    // resume next time lands wherever the last interval landed (could
    // be 10 s back, could be the original startPositionMs). The
    // `seeked` event fires after every seek lands; the report write
    // is cheap so we don't bother debouncing.
    const onSeeked = () => report();
    video.addEventListener("pause", onPause);
    video.addEventListener("ended", onEnded);
    video.addEventListener("seeked", onSeeked);
    return () => {
      window.clearInterval(interval);
      video.removeEventListener("pause", onPause);
      video.removeEventListener("ended", onEnded);
      video.removeEventListener("seeked", onSeeked);
      report();
    };
  }, [itemId, episodeId, videoDuration, scrobbleThreshold, markers, completionBehaviour]);

  // Stats: emit `pause` / `resume` events for the admin Stats engagement
  // metrics. Pause is debounced 3s so seek-driven micro-pauses don't
  // flood the events table — only intentional "I stepped away" pauses
  // count. Resume only fires when a preceding pause was actually
  // emitted, so a transient pause→play (seek, autoplay handoff) is a
  // no-op end-to-end. Fire-and-forget; the stats DB can never disrupt
  // playback.
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    if (itemId == null && episodeId == null) return;
    const target: { item_id?: number; episode_id?: number } = episodeId
      ? { episode_id: episodeId }
      : { item_id: itemId };

    let pendingPause: number | null = null;
    let pauseEmitted = false;

    const positionMs = () =>
      Math.floor(video.currentTime * 1000 + sessionStartMsRef.current);

    const cancelPending = () => {
      if (pendingPause != null) {
        window.clearTimeout(pendingPause);
        pendingPause = null;
      }
    };

    const onPause = () => {
      cancelPending();
      // 3s debounce — quick pauses from seek/autoplay handoff don't
      // count. The browser fires `pause` ahead of `seeking` for
      // user-initiated seeks, so debouncing here also covers that
      // path without a separate seek listener.
      pendingPause = window.setTimeout(() => {
        pendingPause = null;
        if (!video.paused) return;
        playStateApi
          .event({ kind: "pause", position_ms: positionMs(), ...target })
          .catch(() => {});
        pauseEmitted = true;
      }, 3_000);
    };

    const onPlay = () => {
      cancelPending();
      if (!pauseEmitted) return;
      pauseEmitted = false;
      playStateApi
        .event({ kind: "resume", position_ms: positionMs(), ...target })
        .catch(() => {});
    };

    video.addEventListener("pause", onPause);
    video.addEventListener("play", onPlay);
    return () => {
      cancelPending();
      video.removeEventListener("pause", onPause);
      video.removeEventListener("play", onPlay);
    };
  }, [itemId, episodeId]);

  // Auto-advance to next episode.
  useEffect(() => {
    const video = videoRef.current;
    if (!video || !nextHref) return;
    function onEnded() {
      if (!autoNextCancelled && prefs.autoplayNext && nextHref) {
        router.push(nextHref);
      }
    }
    video.addEventListener("ended", onEnded);
    return () => video.removeEventListener("ended", onEnded);
  }, [nextHref, router, autoNextCancelled, prefs.autoplayNext]);

  // Reset session-bound refs when the file or episode changes. Without
  // this, the second episode in a binge session never scrobbles
  // (scrobbledRef stays true after the first ep's scrobble) and any
  // intro whose start_ms collides with one we've already auto-skipped
  // this mount goes un-skipped (very plausible on shows that put the
  // intro at the same offset each episode). lastGoodTracksRef also
  // resets because audio/subtitle indices are file-scoped — reverting
  // to an old file's index on the new file would land on a different
  // language or stream.
  useEffect(() => {
    scrobbledRef.current = false;
    skippedIntrosRef.current = new Set<number>();
    lastGoodTracksRef.current = null;
  }, [activeMediaFileId, episodeId]);

  // Idle-hide controls. Always shows + (re-)arms the auto-hide timer.
  // Idempotent: calling it multiple times in a row from cascading event
  // handlers (pointerdown → click) just keeps the controls visible and
  // pushes the auto-hide deadline out — no toggle race possible.
  const resetHide = useCallback(() => {
    setShowControls(true);
    if (hideTimerRef.current) window.clearTimeout(hideTimerRef.current);
    hideTimerRef.current = window.setTimeout(() => {
      const v = videoRef.current;
      if (v && !v.paused) setShowControls(false);
    }, 3000);
  }, []);

  // Clear the auto-hide timer on unmount. resetHide() schedules a
  // window timeout that holds setShowControls + videoRef in scope; a
  // tab close / route transition mid-window would otherwise leak the
  // closure until the browser GCs it.
  useEffect(() => {
    return () => {
      if (hideTimerRef.current !== null) {
        window.clearTimeout(hideTimerRef.current);
        hideTimerRef.current = null;
      }
    };
  }, []);

  // Auto-hide the resume pill after 6 seconds. Long enough for the user
  // to register the message and act on it, short enough to clear the
  // chrome before the opening scene matters.
  useEffect(() => {
    if (!resumePillVisible) return;
    const t = window.setTimeout(() => setResumePillVisible(false), 6000);
    return () => window.clearTimeout(t);
  }, [resumePillVisible]);

  // Imperative controls.
  const togglePlay = useCallback(() => {
    const v = videoRef.current;
    if (!v) return;
    if (v.paused) v.play().catch(() => {});
    else v.pause();
  }, []);

  // Schedule a session restart at the given source-time. 250ms debounce
  // coalesces a burst of scrubs into one DELETE + POST cycle; without
  // it, fast-scrubbing the bar would queue several teardown/start pairs
  // back-to-back and the old session's in-flight segment fetches would
  // race the new manifest. The latest target wins (each call clobbers
  // the previous pending one). Cleared on unmount.
  //
  // Critical: writes to `pendingRestartTargetMsRef`, NOT `liveTimeMsRef`.
  // The latter is updated on every onTimeUpdate while the still-playing
  // session continues forward; if we wrote the target there, the
  // playhead's forward progress would overwrite our backward target
  // during the 250ms debounce window — the symptom users reported as
  // "unable to jump back on the timeline". The new session's
  // start_position is read from `pendingRestartTargetMsRef` first.
  const triggerSessionRestart = useCallback((sourceTimeSec: number) => {
    pendingRestartTargetMsRef.current = Math.max(
      0,
      Math.floor(sourceTimeSec * 1000),
    );
    if (restartDebounceRef.current !== null) {
      window.clearTimeout(restartDebounceRef.current);
    }
    restartDebounceRef.current = window.setTimeout(() => {
      restartDebounceRef.current = null;
      setResumeEpoch((e) => e + 1);
    }, 250);
  }, []);
  useEffect(() => {
    return () => {
      if (restartDebounceRef.current !== null) {
        window.clearTimeout(restartDebounceRef.current);
        restartDebounceRef.current = null;
      }
    };
  }, []);

  // All seek/seekBy/seekTo arguments are in SOURCE-time (file timeline).
  // Convert to HLS media-time by subtracting the session start; if the
  // target lands before the session start we roll the session at the
  // new position via `resumeEpoch`.
  //
  // Defensive: anything not finite (NaN/Infinity) bails out early.
  // `video.currentTime` can return NaN before metadata loads, and a
  // single NaN propagating into a `setCurrentTime`/`seekTo` chain
  // would land the player at an undefined position.
  const seekTo = useCallback((time: number) => {
    const v = videoRef.current;
    if (!v) return;
    if (!Number.isFinite(time) || time < 0) return;
    const sessionStartSec = sessionStartMsRef.current / 1000;
    const offsetSec = time - sessionStartSec;
    if (offsetSec < 0) {
      // Backward seek before the session's encode start point: tear
      // down + restart at the new position. ffmpeg fast-seeks (`-ss
      // BEFORE -i`) so this is near-instant — the player shows the
      // loading spinner for ~1 s while the new session warms up.
      triggerSessionRestart(time);
      return;
    }

    // Forward seek past the encoded range. Without a restart, the
    // browser sits with `readyState=HAVE_METADATA` waiting for HLS.js
    // to produce a segment for `offsetSec` — but ffmpeg encodes
    // linearly forward at realtime, so the segment would only land
    // after `(offsetSec − encoded_so_far)` seconds of wall-clock
    // wait. For anything more than a few seconds out that's a hang;
    // restart at the new position so ffmpeg fast-seeks there.
    //
    // We approximate "encoded so far" using `v.buffered`'s rightmost
    // edge — HLS.js mirrors ffmpeg's manifest into the SourceBuffer,
    // so the right edge of the buffer is the same as the rightmost
    // segment the encoder has finished. Anything more than 10 s past
    // that gets a session restart; smaller forward seeks let the
    // browser handle it natively (no flicker).
    let bufferedEndSec = 0;
    for (let i = 0; i < v.buffered.length; i++) {
      bufferedEndSec = Math.max(bufferedEndSec, v.buffered.end(i));
    }
    if (offsetSec - bufferedEndSec > 10) {
      triggerSessionRestart(time);
      return;
    }

    // Clamp against the HLS-side duration when known. While the stream
    // is still being encoded `v.duration` can be 0 or Infinity — in
    // that case skip the upper clamp so a forward seek into not-yet-
    // available territory still triggers a buffer fetch instead of
    // snapping to 0.
    const hlsMax = Number.isFinite(v.duration) && v.duration > 0
      ? v.duration
      : Number.POSITIVE_INFINITY;
    v.currentTime = Math.max(0, Math.min(hlsMax, offsetSec));
  }, [triggerSessionRestart]);

  const seekBy = useCallback(
    (delta: number) => {
      const v = videoRef.current;
      if (!v) return;
      const cur = v.currentTime;
      if (!Number.isFinite(cur)) return;
      const srcTimeSec = cur + sessionStartMsRef.current / 1000;
      seekTo(srcTimeSec + delta);
    },
    [seekTo],
  );
  // Keep the late-bound ref pointed at the latest seekBy so MediaSession
  // and any other early-declared callback can call through it.
  useEffect(() => {
    seekByRef.current = seekBy;
  }, [seekBy]);

  /// Double-tap-to-seek for touch devices. Tap the left third → -10s,
  /// the right third → +10s. Single tap on the video still toggles
  /// play/pause; we suppress that toggle on the SECOND tap of a
  /// double-tap so the user doesn't get two play state flips. Mouse
  /// clicks bypass this entirely and go straight to togglePlay (the
  /// existing onClick handler) — Chrome fires both pointerup and click
  /// on a mouse, so we identify mice via `pointerType !== "touch"`.
  const lastTapRef = useRef<{ at: number; x: number; w: number } | null>(null);
  const seekFlashIdRef = useRef(0);
  const suppressNextClickRef = useRef(false);
  // Holds the 150ms timer that auto-clears `suppressNextClickRef`. We
  // track it so a player unmount within the suppression window can
  // cancel the pending clear — otherwise the closure outlives the
  // component and runs against a stale ref. Tiny leak in practice
  // (150ms × small closure), but inconsistent with how the other
  // player timers are managed.
  const suppressClearTimerRef = useRef<number | null>(null);
  useEffect(() => {
    return () => {
      if (suppressClearTimerRef.current !== null) {
        window.clearTimeout(suppressClearTimerRef.current);
        suppressClearTimerRef.current = null;
      }
    };
  }, []);
  // Double-tap seek is wired to the *container* (not the video element).
  // When the controls are shown, the top + bottom gradient bars cover
  // the corners where edge taps land, and a video-only listener would
  // miss the second tap. Container-level capture catches both, and we
  // filter on `target.closest("button, a")` so taps that hit a real
  // control button still go straight to that button's handler instead
  // of triggering a seek.
  const onContainerPointerUp = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      // Double-tap-to-seek is a touch gesture only. Mouse + pen skip
      // straight to togglePlay via the click handler. `pointerType` on
      // the React PointerEvent is usually accurate on Android Chrome
      // PWA; if it isn't, the `isTouchDevice` fallback below treats the
      // primary-input-is-touch case as touch too.
      const isTouchEvent =
        e.pointerType === "touch" ||
        (isTouchDevice && e.pointerType !== "mouse");
      if (!isTouchEvent) return;
      // Skip if the tap landed on / inside an actual control —
      // buttons, links, menu items. Those have their own handlers and
      // shouldn't get hijacked by the seek gesture (e.g. a quick
      // double-tap on the Play button would otherwise seek backwards).
      const target = e.target as Element | null;
      if (target?.closest("button, a, [role='menuitem'], [role='menuitemradio']")) {
        return;
      }
      const rect = e.currentTarget.getBoundingClientRect();
      const x = e.clientX - rect.left;
      const w = rect.width;
      const now = Date.now();
      const prev = lastTapRef.current;
      // Same horizontal third + within 280ms = double-tap.
      const sameSide = (a: number, b: number) =>
        (a < w / 3 && b < w / 3) || (a > (2 * w) / 3 && b > (2 * w) / 3);
      if (prev && now - prev.at < 280 && sameSide(prev.x, x)) {
        e.preventDefault();
        // Swallow the click that the browser would otherwise emit for
        // the second tap — stops it from also triggering togglePlay.
        // `stopPropagation` doesn't suppress the click; the synthetic
        // click fires from the touch sequence regardless. So we set a
        // ref that the video's onClick checks and ignores once. The
        // timeout is the fallback if the synthetic click never fires
        // (touch cancelled, finger dragged off target). 150ms is long
        // enough to cover the synthetic-click delay (~50ms typically)
        // but short enough that the next deliberate user tap isn't
        // accidentally swallowed — 500ms used to be the value and
        // overlapped real follow-up taps, making touch feel dead.
        suppressNextClickRef.current = true;
        if (suppressClearTimerRef.current !== null) {
          window.clearTimeout(suppressClearTimerRef.current);
        }
        suppressClearTimerRef.current = window.setTimeout(() => {
          suppressClearTimerRef.current = null;
          suppressNextClickRef.current = false;
        }, 150);
        const delta = x < w / 2 ? -10 : 10;
        seekBy(delta);
        seekFlashIdRef.current += 1;
        setSeekFlash({
          side: delta < 0 ? "left" : "right",
          delta,
          nonce: seekFlashIdRef.current,
        });
        // Auto-clear the flash after the animation finishes so a
        // re-tap of the same side fires a fresh animation rather than
        // continuing the previous one.
        const myNonce = seekFlashIdRef.current;
        window.setTimeout(() => {
          setSeekFlash((cur) => (cur?.nonce === myNonce ? null : cur));
        }, 650);
        lastTapRef.current = null;
        return;
      }
      lastTapRef.current = { at: now, x, w };
    },
    [seekBy, isTouchDevice],
  );
  const onVideoClick = useCallback(() => {
    if (suppressNextClickRef.current) {
      suppressNextClickRef.current = false;
      return;
    }
    // Mobile vs desktop tap semantics:
    //  - Touch: a tap reliably reveals the controls (and re-arms the
    //    auto-hide timer). Never toggle on tap — that races with the
    //    container's pointerdown→resetHide which set show=true a moment
    //    earlier, leaving the user staring at a black screen because
    //    the toggle flipped it right back to false. Users hide controls
    //    by waiting, not by tapping again.
    //  - Mouse / desktop: a click toggles play/pause, the classic
    //    desktop pattern that doesn't trip the "menu won't show" trap
    //    because hover already keeps controls visible.
    // Detection uses the matchMedia-derived `isTouchDevice`, not a
    // per-event pointerType ref, because Android PWA Chrome will
    // sometimes lose the pointer-type signal between pointerup and the
    // synthetic click.
    if (isTouchDevice) {
      resetHide();
      return;
    }
    togglePlay();
  }, [isTouchDevice, resetHide, togglePlay]);

  // Scrub-time pre-warm. Called by ProgressBar after the user holds
  // a drag position for ~350 ms. Fires a `createSession` at the
  // candidate position so ffmpeg starts encoding there before the
  // user releases. On release the regular `seekTo` flow runs; the
  // backend's `find_compatible` lookup adopts the prewarmed session
  // if the release lands within tolerance.
  //
  // Two short-circuits keep this cheap:
  //   * If the target is already inside the current session's
  //     buffered range, no prewarm needed — the native seek covers
  //     it instantly.
  //   * If the user is on Auto and a specific quality is in flight
  //     (qualitySel not Auto), we use the explicit tier so the
  //     prewarm parameters match what the player will request.
  const prewarmAtPosition = useCallback(
    (sourceTimeSec: number) => {
      const v = videoRef.current;
      if (!v) return;
      if (!Number.isFinite(sourceTimeSec) || sourceTimeSec < 0) return;
      const sessionStartSec = sessionStartMsRef.current / 1000;
      const targetHlsSec = sourceTimeSec - sessionStartSec;
      // Already buffered? Skip — release seek will be instant.
      let bufferedEndSec = 0;
      for (let i = 0; i < v.buffered.length; i++) {
        bufferedEndSec = Math.max(bufferedEndSec, v.buffered.end(i));
      }
      if (
        targetHlsSec >= 0 &&
        targetHlsSec <= bufferedEndSec &&
        targetHlsSec >= 0
      ) {
        return;
      }
      const livePrefs = getPrefs();
      const clientCaps = detectClientCapabilities();
      // Fire-and-forget. The response (a new session id) doesn't
      // need to flow back to the player — `find_compatible` on the
      // backend will discover it when the player's eventual seek
      // POSTs at the same position.
      streamApi
        .createSession({
          media_file_id: activeMediaFileId,
          start_position_ms: Math.floor(sourceTimeSec * 1000),
          audio_index: audioSel,
          subtitle_index: subtitleSel === null ? undefined : subtitleSel,
          quality_target:
            qualitySel.height !== null
              ? { height: qualitySel.height }
              : undefined,
          bitrate_cap_bps: bitrateCapSel.bps ?? undefined,
          audio_normalize: livePrefs.audioNormalize ? true : undefined,
          subtitle_offset_ms:
            subtitleOffsetMs !== 0 ? subtitleOffsetMs : undefined,
          client: {
            supported_video_codecs: clientCaps.video,
            supported_audio_codecs: clientCaps.audio,
            supported_containers: clientCaps.containers,
          },
        })
        .catch(() => {
          // Best-effort; failure just means the seek-on-release
          // takes the normal cold-start path.
        });
    },
    [
      activeMediaFileId,
      audioSel,
      subtitleSel,
      qualitySel,
      bitrateCapSel,
      subtitleOffsetMs,
    ],
  );

  // Auto-skip intros. Fires once per intro per session: when the
  // active marker changes to an "intro", we seek to its end and
  // remember the start_ms so a user who manually scrubs back doesn't
  // get yanked forward again. Credits markers are intentionally NOT
  // auto-skipped — many shows put post-credits scenes in there.
  // Suppressed during scrubbing so a user dragging through an intro
  // region doesn't get the playhead yanked to the credits mid-drag.
  useEffect(() => {
    if (!prefs.autoSkipIntro) return;
    if (!activeMarkerOverlay || activeMarkerOverlay.kind !== "intro") return;
    if (scrubbingRef.current) return;
    if (skippedIntrosRef.current.has(activeMarkerOverlay.start_ms)) return;
    skippedIntrosRef.current.add(activeMarkerOverlay.start_ms);
    seekTo(activeMarkerOverlay.end_ms / 1000);
  }, [activeMarkerOverlay, prefs.autoSkipIntro, seekTo]);

  const toggleMute = useCallback(() => {
    const v = videoRef.current;
    if (!v) return;
    v.muted = !v.muted;
    updatePrefs({ muted: v.muted });
  }, []);

  const toggleFullscreen = useCallback(() => {
    // Standard path first — works on every desktop browser + Android
    // Chrome and the Chrome PWA on every platform we care about.
    if (document.fullscreenElement) {
      document.exitFullscreen().catch(() => {});
      return;
    }
    if (containerRef.current?.requestFullscreen) {
      containerRef.current
        .requestFullscreen()
        .catch(() => {
          // Spec-compliant call rejected; try the iOS Safari
          // video-element path below as a fallback.
          tryWebkitVideoFullscreen(videoRef.current);
        });
      return;
    }
    // iOS Safari (including standalone PWAs) doesn't implement
    // Element.requestFullscreen for arbitrary elements — only
    // HTMLVideoElement.webkitEnterFullscreen on the video itself.
    // Falls back to that here so iPhone users can actually go
    // fullscreen from the player UI.
    tryWebkitVideoFullscreen(videoRef.current);
  }, []);

  const togglePip = useCallback(() => {
    const v = videoRef.current;
    if (!v) return;
    // PiP can be force-disabled per-element by extensions / a11y tools.
    // Clear defensively before the request so the button works even if
    // something else flipped the flag mid-session.
    if (v.disablePictureInPicture) {
      v.disablePictureInPicture = false;
    }
    // Stay synchronous until requestPictureInPicture() is called so
    // the user gesture grant is still in scope. Firefox + Safari
    // reject PiP requests made *after* an await in the same handler
    // ("Document is not focused" / "must be handling a user gesture"),
    // and Chromium has been tightening this too. Use .then/.catch
    // for follow-up work instead of async/await.
    if (document.pictureInPictureElement) {
      document
        .exitPictureInPicture()
        .catch((err) =>
          console.warn("[player] exit picture-in-picture failed", err),
        );
      return;
    }
    if (!document.pictureInPictureEnabled) {
      console.warn(
        "[player] picture-in-picture not supported in this browser",
      );
      return;
    }
    if (typeof v.requestPictureInPicture !== "function") {
      console.warn(
        "[player] video element has no requestPictureInPicture",
      );
      return;
    }
    v.requestPictureInPicture().catch((err) => {
      console.warn("[player] picture-in-picture request failed", err);
    });
  }, []);

  // Audio/subtitle selection causes a fresh session (the transcoder burns
  // subtitles in, so there's no in-stream switch).
  //
  // We also persist the picked language as the user's default so the
  // next title auto-selects the same audio / subtitle language without
  // them re-picking. Fire-and-forget — a failed save is fine, the
  // in-session selection still applies; only the cross-title memory
  // misses. Skip the save when the track has no language tag (e.g.
  // "Commentary" with `und`) because saving "und" would clobber the
  // user's English default with garbage.
  const selectAudio = useCallback(
    (idx: number) => {
      setAudioSel(idx);
      const picked = activeAudioTracks.find((t) => t.idx === idx);
      const lang = picked?.language;
      if (lang && lang !== "und") {
        authApi.updateMe({ default_audio_lang: lang }).catch(() => {});
      }
    },
    [activeAudioTracks],
  );

  const selectSubtitle = useCallback((track: StreamChoice | null) => {
    if (track === null) {
      // Explicitly off — clear both surfaces but DON'T clobber the
      // saved language preference. The user might be turning subs off
      // for this scene only; the auto-picker can still kick in next
      // title. Clearing the default lives in Settings → Profile.
      setSubtitleSel(null);
      setExternalSub(null);
      return;
    }
    if (track.externalUrl) {
      // External: skip the burn-in session reload, attach a <track>.
      setSubtitleSel(null);
      setExternalSub({
        url: track.externalUrl,
        language: track.language ?? null,
      });
    } else {
      setExternalSub(null);
      setSubtitleSel(track.idx);
    }
    const lang = track.language;
    if (lang && lang !== "und") {
      authApi.updateMe({ default_subtitle_lang: lang }).catch(() => {});
    }
  }, []);

  const toggleSubtitles = useCallback(() => {
    if (
      (subtitleSel === null || subtitleSel === undefined) &&
      externalSubUrl === null
    ) {
      const first = activeSubtitleTracks[0];
      if (first) selectSubtitle(first);
    } else {
      selectSubtitle(null);
    }
  }, [subtitleSel, externalSubUrl, activeSubtitleTracks, selectSubtitle]);

  const setVolumeValue = useCallback((value: number) => {
    const v = videoRef.current;
    if (!v) return;
    const clamped = Math.max(0, Math.min(1, value));
    v.volume = clamped;
    const wasMuted = v.muted;
    if (clamped > 0 && wasMuted) v.muted = false;
    updatePrefs({
      volume: clamped,
      muted: clamped === 0 ? wasMuted : false,
    });
  }, []);

  const setSpeed = useCallback((rate: number) => {
    const v = videoRef.current;
    if (!v) return;
    v.playbackRate = rate;
    setPlaybackRate(rate);
    updatePrefs({ playbackRate: rate });
  }, []);

  // Keyboard shortcuts.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const target = e.target as HTMLElement | null;
      if (
        target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.isContentEditable)
      ) {
        return;
      }
      switch (e.key) {
        case " ":
        case "k":
        case "K":
          e.preventDefault();
          togglePlay();
          resetHide();
          break;
        case "ArrowLeft":
        case "j":
        case "J":
          seekBy(-10);
          resetHide();
          break;
        case "ArrowRight":
        case "l":
        case "L":
          seekBy(10);
          resetHide();
          break;
        case "ArrowUp": {
          e.preventDefault();
          const v = videoRef.current;
          if (v) setVolumeValue(Math.min(1, (v.muted ? 0 : v.volume) + 0.05));
          resetHide();
          break;
        }
        case "ArrowDown": {
          e.preventDefault();
          const v = videoRef.current;
          if (v) setVolumeValue(Math.max(0, (v.muted ? 0 : v.volume) - 0.05));
          resetHide();
          break;
        }
        case "Home": {
          e.preventDefault();
          seekTo(0);
          resetHide();
          break;
        }
        case "End": {
          e.preventDefault();
          if (videoDuration > 0) seekTo(videoDuration - 1);
          resetHide();
          break;
        }
        case "f":
        case "F":
          toggleFullscreen();
          resetHide();
          break;
        case "m":
        case "M":
          toggleMute();
          resetHide();
          break;
        case "c":
        case "C":
          toggleSubtitles();
          resetHide();
          break;
        case "p":
        case "P":
          togglePip();
          resetHide();
          break;
        case "s":
        case "S":
          setStatsOpen((v) => !v);
          resetHide();
          break;
        case "?":
          // Shift+/ on US layouts, ? on most others. Toggle the
          // shortcut overlay so first-time users can discover the
          // hotkeys without RTFM.
          e.preventDefault();
          setHotkeysOpen((v) => !v);
          resetHide();
          break;
        case "n":
        case "N":
          if (nextHref) {
            e.preventDefault();
            router.push(nextHref);
          }
          break;
        case "0":
        case "1":
        case "2":
        case "3":
        case "4":
        case "5":
        case "6":
        case "7":
        case "8":
        case "9": {
          // YouTube-style jump-to-percent: 0 = start, 5 = midpoint,
          // 9 = 90%. Useful for power-skipping through known content.
          if (videoDuration > 0) {
            e.preventDefault();
            seekTo(videoDuration * (Number.parseInt(e.key, 10) / 10));
            resetHide();
          }
          break;
        }
        case ">":
        case ".":
          if (e.shiftKey || e.key === ">") {
            const cur = videoRef.current?.playbackRate ?? 1;
            const next = SPEED_OPTIONS.find((o) => o > cur);
            if (next !== undefined) setSpeed(next);
            resetHide();
          }
          break;
        case "<":
        case ",":
          if (e.shiftKey || e.key === "<") {
            const cur = videoRef.current?.playbackRate ?? 1;
            const prev = [...SPEED_OPTIONS].reverse().find((o) => o < cur);
            if (prev !== undefined) setSpeed(prev);
            resetHide();
          }
          break;
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [
    togglePlay,
    seekBy,
    seekTo,
    toggleFullscreen,
    toggleMute,
    toggleSubtitles,
    togglePip,
    setVolumeValue,
    setSpeed,
    resetHide,
    videoDuration,
    nextHref,
    router,
  ]);

  return (
    <div
      ref={containerRef}
      onMouseMove={resetHide}
      onPointerDown={resetHide}
      onPointerUp={onContainerPointerUp}
      className={`fixed inset-0 z-50 select-none bg-black ${
        showControls ? "" : "cursor-none"
      }`}
    >
      <video
        ref={videoRef}
        playsInline
        autoPlay
        onClick={onVideoClick}
        crossOrigin="anonymous"
        className={`h-full w-full bg-black ${cueClass}`}
      >
        {externalSub && (
          <track
            kind="subtitles"
            src={externalSub.url}
            srcLang={externalSub.language ?? "und"}
            default
            onError={() => {
              // External subtitle 404 / network blip would otherwise
              // leave the picker showing the row as "active" while no
              // cues render — a silent failure. Clear the selection
              // so the UI reflects the real state and the user can
              // pick another row.
              console.warn(
                "[player] external subtitle failed to load",
                externalSub.url,
              );
              selectSubtitle(null);
            }}
          />
        )}
      </video>

      {error && <ErrorOverlay message={error} />}
      {loading && !error && !autoplayBlocked && <LoadingSpinner />}
      {autoplayBlocked && !error && <BigPlayButton onClick={attemptPlay} />}
      {reconnecting && !error && <ReconnectingOverlay />}

      {statsOpen && (
        <StatsOverlay
          videoRef={videoRef}
          hlsRef={hlsRef}
          sessionStatus={sessionStatus}
          targetHeight={sessionStatus?.height ?? null}
          onClose={() => setStatsOpen(false)}
        />
      )}

      {hotkeysOpen && (
        <HotkeysOverlay onClose={() => setHotkeysOpen(false)} />
      )}

      {/* Double-tap-seek flash overlay (touch only). The `key` forces a
          remount on each new seek so the CSS animation restarts even
          when the side is unchanged. */}
      {seekFlash && (
        <div
          key={seekFlash.nonce}
          aria-hidden
          className={`zf-seek-flash pointer-events-none absolute top-1/2 z-30 flex h-32 w-32 -translate-y-1/2 items-center justify-center rounded-full bg-white/15 backdrop-blur-sm ${
            seekFlash.side === "left" ? "left-4 sm:left-12" : "right-4 sm:right-12"
          }`}
        >
          <div className="flex flex-col items-center text-white">
            {seekFlash.side === "left" ? (
              <Rewind10Icon />
            ) : (
              <Forward10Icon />
            )}
            <span className="mt-1 text-xs font-semibold">
              {seekFlash.delta > 0 ? "+10s" : "-10s"}
            </span>
          </div>
        </div>
      )}

      {activeMarkerOverlay && dismissedMarkerKey !== activeMarkerKey && (
        <SkipMarkerButton
          marker={activeMarkerOverlay}
          currentMs={currentTime * 1000}
          onSkip={(m) => {
            seekTo(m.end_ms / 1000);
          }}
          onDismiss={() => setDismissedMarkerKey(activeMarkerKey)}
        />
      )}

      {resumePillVisible && (
        <ResumePill
          resumeSec={startPositionMs / 1000}
          onStartOver={() => {
            seekTo(0);
            setResumePillVisible(false);
          }}
          onDismiss={() => setResumePillVisible(false)}
        />
      )}

      {nextHref &&
        nextLabel &&
        !autoNextCancelled &&
        prefs.autoplayNext &&
        videoDuration > 0 &&
        videoDuration - currentTime <= COUNTDOWN_WINDOW_SECONDS &&
        videoDuration - currentTime > 0 && (
          <NextEpisodeCountdown
            secondsLeft={Math.max(0, Math.ceil(videoDuration - currentTime))}
            href={nextHref}
            label={nextLabel}
            thumb={nextThumb}
            onCancel={() => setAutoNextCancelled(true)}
          />
        )}

      <div
        // When the controls are hidden, the wrapper covers the entire
        // video surface (absolute inset-0). `inert` masks the subtree
        // but in some browsers (notably Android Chrome PWA) it also
        // swallows the tap that would otherwise bubble to the container's
        // onPointerDown→resetHide, so the user can't reveal controls
        // again — touch appears dead. `pointer-events-none` on the
        // wrapper instead lets the tap fall through to the container,
        // and since no descendant inside this wrapper sets an explicit
        // `pointer-events: auto`, the `none` correctly propagates and
        // hidden buttons can't capture taps.
        aria-hidden={!showControls}
        className={`absolute inset-0 transition-opacity duration-200 ${
          showControls ? "opacity-100" : "pointer-events-none opacity-0"
        }`}
      >
        {/* Top bar — back link + title (title hides on desktop because
            the bottom row already shows it there). */}
        <div className="absolute inset-x-0 top-0 bg-linear-to-b from-black/80 to-transparent">
          <div className="flex items-center gap-3 pl-[max(0.75rem,env(safe-area-inset-left))] pr-[max(0.75rem,env(safe-area-inset-right))] pt-[max(0.75rem,env(safe-area-inset-top))] pb-3 sm:gap-6 sm:pl-[max(2rem,env(safe-area-inset-left))] sm:pr-[max(2rem,env(safe-area-inset-right))] sm:pt-[max(1.25rem,env(safe-area-inset-top))] sm:pb-5">
            <Link
              href={backHref}
              aria-label="Back"
              className="flex shrink-0 items-center gap-2 rounded-full p-2 -m-2 text-white/85 transition-colors hover:text-white"
            >
              <BackIcon />
              <span className="hidden text-sm font-medium sm:inline">Back</span>
            </Link>
            {/* Title in the top bar on mobile only — desktop keeps it
                centered in the bottom row where it's always paired with
                the controls. */}
            <div className="min-w-0 flex-1 sm:hidden">
              <div className="truncate text-sm font-semibold leading-tight">
                {title}
              </div>
              {subtitle && (
                <div className="mt-0.5 truncate text-xs text-white/70">
                  {subtitle}
                </div>
              )}
            </div>
          </div>
        </div>

        {/* Bottom controls. Tighter padding on phones so the button row
            doesn't get crowded; mobile chrome already has thumb-reach
            margins via the bottom-bar gradient. Safe-area-inset offsets
            push controls clear of iOS home indicator + landscape notch. */}
        <div className="absolute inset-x-0 bottom-0 bg-linear-to-t from-black/85 to-transparent pl-[max(0.75rem,env(safe-area-inset-left))] pr-[max(0.75rem,env(safe-area-inset-right))] pb-[max(0.75rem,env(safe-area-inset-bottom))] pt-12 sm:pl-[max(2rem,env(safe-area-inset-left))] sm:pr-[max(2rem,env(safe-area-inset-right))] sm:pb-[max(1.5rem,env(safe-area-inset-bottom))] sm:pt-16">
          <div className="flex items-center gap-2 sm:gap-3">
            {/*
              Current position, left of the bar. Always visible so the
              user can see exactly where they are without having to
              hover the bar or do mental arithmetic against the
              remaining-time counter. Tabular-nums keeps the digits
              from jittering as the second ticks.
            */}
            <span className="shrink-0 text-sm tabular-nums text-white/85">
              {formatTime(currentTime)}
            </span>
            <div className="grow">
              <ProgressBar
                currentTime={currentTime}
                duration={videoDuration}
                bufferedEnd={bufferedEnd}
                onSeek={seekTo}
                onSeekHint={prewarmAtPosition}
                onScrubChange={(s) => {
                  scrubbingRef.current = s;
                }}
                markers={markers}
              />
            </div>
            <button
              type="button"
              onClick={() => setShowRemaining((s) => !s)}
              aria-label={
                showRemaining ? "Show total duration" : "Show time remaining"
              }
              className="shrink-0 text-sm tabular-nums text-white/85 transition-colors hover:text-white"
            >
              {showRemaining
                ? `-${formatTime(Math.max(0, videoDuration - currentTime))}`
                : formatTime(videoDuration)}
            </button>
          </div>

          <div className="mt-2 flex items-center gap-2 sm:gap-4">
            <div className="flex shrink-0 items-center gap-1 sm:gap-5">
              <IconButton
                onClick={togglePlay}
                aria-label={playing ? "Pause" : "Play"}
              >
                {playing ? <PauseIcon /> : <PlayIcon />}
              </IconButton>
              <IconButton
                onClick={() => seekBy(-10)}
                aria-label="Skip back 10 seconds"
              >
                <Rewind10Icon />
              </IconButton>
              <IconButton
                onClick={() => seekBy(10)}
                aria-label="Skip forward 10 seconds"
              >
                <Forward10Icon />
              </IconButton>
              <VolumeControl
                muted={muted}
                volume={volume}
                onToggleMute={toggleMute}
                onVolumeChange={setVolumeValue}
              />
            </div>
            {/* Hide the title strip on phones — the bottom row needs
                every pixel for controls. Title is in the top bar
                already on mobile via the back-link area. */}
            <div className="hidden min-w-0 grow text-center sm:block">
              <div className="truncate text-sm font-semibold leading-tight">
                {title}
              </div>
              {subtitle && (
                <div className="mt-0.5 truncate text-xs text-white/70">
                  {subtitle}
                </div>
              )}
            </div>
            <div className="ml-auto flex shrink-0 items-center gap-1 sm:ml-0 sm:gap-5">
              {nextHref && (
                <Link
                  href={nextHref}
                  aria-label={nextLabel ?? "Next episode"}
                  title={nextLabel ?? "Next episode"}
                  className="flex h-10 items-center justify-center text-white/90 transition-colors hover:text-white"
                >
                  <NextEpisodeIcon />
                </Link>
              )}
              {seasonEpisodes && seasonEpisodes.length > 1 && (
                <EpisodesControl
                  open={episodesOpen}
                  episodes={seasonEpisodes}
                  currentRatingKey={currentRatingKey}
                  currentSeasonId={currentSeasonId}
                  showId={showId}
                  showTitle={showTitle}
                  seasons={seasons}
                  onToggle={() => setEpisodesOpen((o) => !o)}
                  onClose={() => setEpisodesOpen(false)}
                />
              )}
              <TracksControl
                audioTracks={activeAudioTracks}
                subtitleTracks={activeSubtitleTracks}
                versions={versions ?? []}
                activeMediaFileId={activeMediaFileId}
                onVersionSelect={selectVersion}
                audioSel={audioSel}
                subtitleSel={subtitleSel}
                externalSubUrl={externalSubUrl}
                qualityOptions={QUALITY_OPTIONS}
                qualitySel={qualitySel}
                onQualitySelect={setQualitySel}
                bitrateCapOptions={BITRATE_CAP_OPTIONS}
                bitrateCapSel={bitrateCapSel}
                onBitrateCapSelect={setBitrateCapSel}
                sessionStatus={sessionStatus}
                open={tracksOpen}
                onToggle={() => setTracksOpen((o) => !o)}
                onClose={() => setTracksOpen(false)}
                onAudioSelect={selectAudio}
                onSubtitleSelect={selectSubtitle}
              />
              <SpeedControl
                rate={playbackRate}
                open={speedOpen}
                onToggle={() => setSpeedOpen((o) => !o)}
                onClose={() => setSpeedOpen(false)}
                onSelect={setSpeed}
              />
              <IconButton
                onClick={togglePip}
                aria-label={
                  pipActive ? "Exit picture-in-picture" : "Picture-in-picture"
                }
                aria-pressed={pipActive}
              >
                <PipIcon />
              </IconButton>
              <SubtitleSettingsControl
                offsetMs={subtitleOffsetMs}
                onOffsetChange={setSubtitleOffsetMs}
                style={subtitleStyle}
                onStyleChange={setSubtitleStyle}
                hasActiveSubtitle={
                  externalSubUrl !== null || typeof subtitleSel === "number"
                }
              />
              <IconButton
                onClick={toggleFullscreen}
                aria-label={isFullscreen ? "Exit fullscreen" : "Fullscreen"}
              >
                {isFullscreen ? <FullscreenExitIcon /> : <FullscreenIcon />}
              </IconButton>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

// ── UI subcomponents ────────────────────────────────────────────────────────

function IconButton({
  children,
  className = "",
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type="button"
      {...props}
      className={`flex h-11 w-11 items-center justify-center text-white/90 transition-colors hover:text-white ${className}`}
    >
      {children}
    </button>
  );
}

function BigPlayButton({ onClick }: { onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label="Play"
      className="absolute inset-0 z-20 flex cursor-pointer items-center justify-center bg-black/30 transition-colors hover:bg-black/50"
    >
      <div className="flex h-24 w-24 items-center justify-center rounded-full bg-white text-black shadow-2xl transition-transform hover:scale-105">
        <svg
          width="44"
          height="44"
          viewBox="0 0 24 24"
          fill="currentColor"
          aria-hidden
        >
          <path d="M7 4l13 8-13 8V4z" />
        </svg>
      </div>
    </button>
  );
}

function TracksControl({
  audioTracks,
  subtitleTracks,
  versions,
  activeMediaFileId,
  onVersionSelect,
  audioSel,
  subtitleSel,
  externalSubUrl,
  qualityOptions,
  qualitySel,
  onQualitySelect,
  bitrateCapOptions,
  bitrateCapSel,
  onBitrateCapSelect,
  sessionStatus,
  open,
  onToggle,
  onClose,
  onAudioSelect,
  onSubtitleSelect,
}: {
  audioTracks: StreamChoice[];
  subtitleTracks: StreamChoice[];
  versions: VersionChoice[];
  activeMediaFileId: number;
  onVersionSelect: (mediaFileId: number) => void;
  audioSel?: number;
  subtitleSel?: number | null;
  externalSubUrl: string | null;
  qualityOptions: QualityChoice[];
  qualitySel: QualityChoice;
  onQualitySelect: (q: QualityChoice) => void;
  bitrateCapOptions: BitrateCapChoice[];
  bitrateCapSel: BitrateCapChoice;
  onBitrateCapSelect: (q: BitrateCapChoice) => void;
  sessionStatus: {
    height: number | null;
    sourceHeight: number | null;
    encoder: string | null;
    videoTreatment: "copy" | "reencode" | null;
    audioTreatment: "copy" | "reencode" | null;
  } | null;
  open: boolean;
  onToggle: () => void;
  onClose: () => void;
  onAudioSelect: (idx: number) => void;
  onSubtitleSelect: (track: StreamChoice | null) => void;
}) {
  const wrapRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    function onDocClick(e: MouseEvent) {
      if (!wrapRef.current?.contains(e.target as Node)) onClose();
    }
    function onEsc(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    document.addEventListener("mousedown", onDocClick);
    document.addEventListener("keydown", onEsc);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
      document.removeEventListener("keydown", onEsc);
    };
  }, [open, onClose]);

  // Count visible columns to size the popover. Quality + Audio + Subs
  // are always present; Version only when there's more than one file.
  const hasVersion = versions.length > 1;
  const columnCount = (hasVersion ? 1 : 0) + 3;
  const widthClass =
    columnCount === 4 ? "w-3xl grid-cols-4" : "w-2xl grid-cols-3";

  return (
    <div ref={wrapRef} className="relative">
      <IconButton
        onClick={onToggle}
        aria-label="Audio, subtitles, and quality"
        aria-haspopup="menu"
        aria-expanded={open}
      >
        <CaptionsIcon />
      </IconButton>
      {open && (
        <div
          role="menu"
          onKeyDown={handleMenuArrowKeys}
          // Bottom-sheet on phones: full-width minus 0.5rem margin,
          // pinned to the bottom of the viewport, scrollable if the
          // contents overflow. Reverts to the anchored popover at sm+
          // so the existing desktop layout is untouched.
          className={`fixed inset-x-2 bottom-2 z-50 grid max-h-[75vh] overflow-y-auto rounded-md border border-white/10 bg-black/95 shadow-2xl backdrop-blur-sm sm:absolute sm:inset-x-auto sm:bottom-full sm:right-0 sm:mb-3 sm:max-h-none sm:overflow-hidden ${widthClass}`}
        >
          {hasVersion && (
            <VersionColumn
              versions={versions}
              activeMediaFileId={activeMediaFileId}
              onSelect={onVersionSelect}
            />
          )}
          <QualityColumn
            options={qualityOptions}
            active={qualitySel}
            onSelect={onQualitySelect}
            bitrateCapOptions={bitrateCapOptions}
            bitrateCapActive={bitrateCapSel}
            onBitrateCapSelect={onBitrateCapSelect}
            sessionStatus={sessionStatus}
          />
          <StreamColumn
            label="Audio"
            tracks={audioTracks}
            isActive={(t) => audioSel === t.idx}
            offSelected={false}
            onSelect={(t) => {
              if (t !== null) onAudioSelect(t.idx);
            }}
            offOption={false}
          />
          <StreamColumn
            label="Subtitles"
            tracks={subtitleTracks}
            isActive={(t) =>
              t.externalUrl
                ? externalSubUrl === t.externalUrl
                : externalSubUrl === null && subtitleSel === t.idx
            }
            offSelected={
              (subtitleSel === null || subtitleSel === undefined) &&
              externalSubUrl === null
            }
            onSelect={(t) => onSubtitleSelect(t)}
            offOption={true}
            badgeFor={(t) => (isPictureSubtitle(t.codec) ? "Burn-in" : null)}
          />
        </div>
      )}
    </div>
  );
}

function QualityColumn({
  options,
  active,
  onSelect,
  bitrateCapOptions,
  bitrateCapActive,
  onBitrateCapSelect,
  sessionStatus,
}: {
  options: QualityChoice[];
  active: QualityChoice;
  onSelect: (q: QualityChoice) => void;
  bitrateCapOptions: BitrateCapChoice[];
  bitrateCapActive: BitrateCapChoice;
  onBitrateCapSelect: (b: BitrateCapChoice) => void;
  sessionStatus: {
    height: number | null;
    sourceHeight: number | null;
    encoder: string | null;
    videoTreatment: "copy" | "reencode" | null;
    audioTreatment: "copy" | "reencode" | null;
  } | null;
}) {
  // Build the "Auto · 1080p" annotation. Only meaningful when the
  // user has Auto selected and the server actually transcoded —
  // direct-play sessions report no resolved tier and don't need
  // disambiguation.
  const isAuto = active.height === null;
  const resolvedHeight = sessionStatus?.height;
  const autoSubLabel = isAuto && resolvedHeight ? `${resolvedHeight}p` : null;
  const isRemux =
    sessionStatus?.videoTreatment === "copy" &&
    sessionStatus?.audioTreatment === "copy";
  // Grey out tiers that exceed source resolution — the scale filter
  // already caps at source ("scale=-2:'min(target,ih)'"), so a 1080p
  // pick on a 720p source produces 720p output at 1080p's bitrate
  // budget. Pointless; better to hide the choice than to lie about
  // what'll happen.
  const sourceHeight = sessionStatus?.sourceHeight ?? null;
  const isImpractical = (q: QualityChoice) =>
    sourceHeight !== null && q.height !== null && q.height > sourceHeight;
  return (
    <div className="border-r border-white/10">
      <div className="border-b border-white/10 px-4 py-3 text-[0.7rem] font-semibold uppercase tracking-wider text-white/60">
        Resolution
      </div>
      <ul className="max-h-48 overflow-y-auto py-2">
        {options.map((q) => (
          <TrackRow
            key={q.label}
            label={
              q.label === "Auto" && autoSubLabel
                ? `Auto · ${autoSubLabel}`
                : q.label
            }
            active={active.height === q.height}
            disabled={isImpractical(q)}
            onClick={() => {
              if (isImpractical(q)) return;
              onSelect(q);
            }}
          />
        ))}
      </ul>
      {/* Independent bitrate cap. Stacked in the same column so we
          don't have to widen the popover for a fifth column — most
          users never touch this, but mobile-data viewers really
          want it. */}
      <div className="border-t border-b border-white/10 px-4 py-3 text-[0.7rem] font-semibold uppercase tracking-wider text-white/60">
        Max bitrate
      </div>
      <ul className="max-h-48 overflow-y-auto py-2">
        {bitrateCapOptions.map((b) => (
          <TrackRow
            key={b.label}
            label={b.label}
            active={bitrateCapActive.bps === b.bps}
            onClick={() => onBitrateCapSelect(b)}
          />
        ))}
      </ul>
      {sessionStatus?.encoder && (
        <div className="border-t border-white/10 px-4 py-2 text-[0.65rem] uppercase tracking-wider text-white/45">
          {isRemux ? "Remux · " : ""}
          {sessionStatus.encoder}
        </div>
      )}
    </div>
  );
}

function VersionColumn({
  versions,
  activeMediaFileId,
  onSelect,
}: {
  versions: VersionChoice[];
  activeMediaFileId: number;
  onSelect: (mediaFileId: number) => void;
}) {
  return (
    <div className="border-r border-white/10">
      <div className="border-b border-white/10 px-4 py-3 text-[0.7rem] font-semibold uppercase tracking-wider text-white/60">
        Version
      </div>
      <ul className="max-h-72 overflow-y-auto py-2">
        {versions.map((v) => (
          <TrackRow
            key={v.media_file_id}
            label={v.label}
            active={activeMediaFileId === v.media_file_id}
            onClick={() => onSelect(v.media_file_id)}
          />
        ))}
      </ul>
    </div>
  );
}

/// Dedicated player-controls button that opens a small popover
/// with subtitle sync offset + appearance controls. Replaces the
/// old stats-overlay button slot — stats are still toggleable
/// via the `S` keybind, freeing this slot for something that
/// gets touched more often (especially sync, which varies per
/// title and is the #1 reason a power user wants captions
/// settings).
function SubtitleSettingsControl({
  offsetMs,
  onOffsetChange,
  style,
  onStyleChange,
  hasActiveSubtitle,
}: {
  offsetMs: number;
  onOffsetChange: (ms: number) => void;
  style: SubtitleStyle;
  onStyleChange: (a: SubtitleStyle) => void;
  /// Whether a subtitle is currently selected. Drives the offset
  /// stepper's enabled state — offset is a no-op without an
  /// active sub.
  hasActiveSubtitle: boolean;
}) {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    function onDocClick(e: MouseEvent) {
      if (!wrapRef.current?.contains(e.target as Node)) setOpen(false);
    }
    function onEsc(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    document.addEventListener("mousedown", onDocClick);
    document.addEventListener("keydown", onEsc);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
      document.removeEventListener("keydown", onEsc);
    };
  }, [open]);
  const bump = (deltaMs: number) => {
    const next = Math.max(-30_000, Math.min(30_000, offsetMs + deltaMs));
    onOffsetChange(next);
  };
  const offsetLabel = (() => {
    if (offsetMs === 0) return "0 s";
    const s = (offsetMs / 1000).toFixed(offsetMs % 1000 === 0 ? 1 : 2);
    return `${offsetMs > 0 ? "+" : ""}${s} s`;
  })();
  return (
    <div ref={wrapRef} className="relative">
      <IconButton
        onClick={() => setOpen((o) => !o)}
        aria-label="Subtitle sync and appearance"
        aria-haspopup="menu"
        aria-expanded={open}
        title="Subtitle settings"
      >
        <SubtitleSettingsIcon />
      </IconButton>
      {open && (
        <div
          role="menu"
          onKeyDown={handleMenuArrowKeys}
          // Landscape phones are short — 75vh used to leave the menu
          // taller than the viewport, anchored at the bottom, with the
          // top (sync offset section) clipped off-screen. Anchor with
          // top+bottom so the menu fills the available height and uses
          // an inner scroll container; bumping max-h alone doesn't help
          // in landscape where 85vh ≈ 300px.
          className="fixed inset-x-2 top-2 bottom-2 z-50 flex flex-col overflow-hidden rounded-md border border-white/10 bg-black/95 shadow-2xl backdrop-blur-sm sm:absolute sm:inset-x-auto sm:bottom-full sm:top-auto sm:right-0 sm:mb-3 sm:max-h-[80vh] sm:w-96"
        >
          <div className="shrink-0 border-b border-white/10 px-4 py-3 text-[0.7rem] font-semibold uppercase tracking-wider text-white/60">
            Subtitle settings
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain">
          <div className="px-4 py-3">
            <div className="mb-1 flex items-center justify-between text-[0.65rem] font-semibold uppercase tracking-wider text-white/55">
              <span>Sync offset</span>
              <span className="tabular-nums text-white/80">{offsetLabel}</span>
            </div>
            <div className="flex items-center justify-between gap-1">
              <OffsetStep label="−1s" onClick={() => bump(-1000)} disabled={!hasActiveSubtitle} />
              <OffsetStep label="−.5" onClick={() => bump(-500)} disabled={!hasActiveSubtitle} />
              <OffsetStep label="−.1" onClick={() => bump(-100)} disabled={!hasActiveSubtitle} />
              <OffsetStep
                label="0"
                onClick={() => onOffsetChange(0)}
                disabled={!hasActiveSubtitle || offsetMs === 0}
                primary
              />
              <OffsetStep label="+.1" onClick={() => bump(100)} disabled={!hasActiveSubtitle} />
              <OffsetStep label="+.5" onClick={() => bump(500)} disabled={!hasActiveSubtitle} />
              <OffsetStep label="+1s" onClick={() => bump(1000)} disabled={!hasActiveSubtitle} />
            </div>
            <p className="mt-1 text-[0.6rem] leading-snug text-white/40">
              {hasActiveSubtitle
                ? "Negative = subs earlier · positive = subs later"
                : "Pick a subtitle to enable sync offset"}
            </p>
          </div>
          <div className="border-t border-white/10 px-4 py-3">
            <div className="mb-2 text-[0.65rem] font-semibold uppercase tracking-wider text-white/55">
              Appearance
            </div>
            <SubtitleStylePanel value={style} onChange={onStyleChange} />
          </div>
          </div>
        </div>
      )}
    </div>
  );
}

function OffsetStep({
  label,
  onClick,
  disabled,
  primary = false,
}: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  primary?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={`flex-1 rounded px-1.5 py-1 text-center text-[0.7rem] tabular-nums transition-colors ${
        primary
          ? "bg-white/15 text-white hover:bg-white/25"
          : "bg-white/5 text-white/85 hover:bg-white/15"
      } disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-white/5`}
    >
      {label}
    </button>
  );
}

export function SubtitleStylePanel({
  value,
  onChange,
}: {
  value: SubtitleStyle;
  onChange: (next: SubtitleStyle) => void;
}) {
  const patch = (p: Partial<SubtitleStyle>) => onChange({ ...value, ...p });
  return (
    <div className="mt-3 space-y-3 text-[0.7rem]">
      <div>
        <div className="mb-1 text-white/55">Size</div>
        <div className="flex gap-1">
          {FONT_SIZE_PRESETS.map((opt) => (
            <ApprBtn
              key={opt.label}
              label={opt.label}
              active={value.fontSizePx === opt.px}
              onClick={() => patch({ fontSizePx: opt.px })}
            />
          ))}
        </div>
      </div>
      <div>
        <div className="mb-1 text-white/55">Color</div>
        <div className="flex gap-1">
          {TEXT_COLOR_PRESETS.map((opt) => (
            <button
              key={opt.value}
              type="button"
              onClick={() => patch({ textColor: opt.value })}
              aria-label={opt.label}
              title={opt.label}
              className={`h-6 w-6 rounded border-2 transition-transform hover:scale-110 ${
                value.textColor === opt.value
                  ? "border-white"
                  : "border-white/20"
              }`}
              style={{ backgroundColor: opt.value }}
            />
          ))}
        </div>
      </div>
      <div>
        <div className="mb-1 text-white/55">Background</div>
        <div className="flex gap-1">
          {BG_PRESETS.map((opt) => (
            <ApprBtn
              key={opt.label}
              label={opt.label}
              active={value.backgroundColor === opt.value}
              onClick={() => patch({ backgroundColor: opt.value })}
            />
          ))}
        </div>
      </div>
      <div>
        <div className="mb-1 text-white/55">Edge</div>
        <div className="flex gap-1">
          {(["outline", "shadow", "none"] as const).map((e) => (
            <ApprBtn
              key={e}
              label={e[0].toUpperCase() + e.slice(1)}
              active={value.edge === e}
              onClick={() => patch({ edge: e })}
            />
          ))}
        </div>
      </div>
      <div>
        <div className="mb-1 text-white/55">Font</div>
        <div className="flex gap-1">
          {(
            [
              ["default", "Default"],
              ["sans", "Sans"],
              ["serif", "Serif"],
              ["mono", "Mono"],
            ] as const
          ).map(([val, lbl]) => (
            <ApprBtn
              key={val}
              label={lbl}
              active={value.fontFamily === val}
              onClick={() => patch({ fontFamily: val as SubtitleFontFamily })}
            />
          ))}
        </div>
      </div>
      <div>
        <div className="mb-1 flex items-center justify-between text-white/55">
          <span>Position</span>
          <span className="tabular-nums text-white/80">
            {value.bottomInsetPct}% from bottom
          </span>
        </div>
        <input
          type="range"
          min={0}
          max={45}
          step={1}
          value={value.bottomInsetPct}
          onChange={(e) =>
            patch({ bottomInsetPct: Number.parseInt(e.target.value, 10) })
          }
          className="w-full accent-white"
          aria-label="Subtitle vertical position"
        />
        <div className="mt-0.5 flex justify-between text-[0.55rem] uppercase tracking-wider text-white/40">
          <span>Video edge</span>
          <span>Middle</span>
        </div>
        <p className="mt-0.5 text-[0.55rem] leading-snug text-white/40">
          Measured from the bottom of the visible video — letterbox is
          accounted for automatically.
        </p>
      </div>
      <div>
        <button
          type="button"
          onClick={() => onChange(DEFAULT_SUBTITLE_STYLE)}
          className="text-[0.65rem] uppercase tracking-wider text-white/50 hover:text-white/85"
        >
          Reset to defaults
        </button>
      </div>
    </div>
  );
}

function ApprBtn({
  label,
  active,
  onClick,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`rounded px-2 py-1 text-[0.65rem] uppercase tracking-wider transition-colors ${
        active
          ? "bg-white/20 text-white"
          : "bg-white/5 text-white/70 hover:bg-white/15 hover:text-white"
      }`}
    >
      {label}
    </button>
  );
}

function StreamColumn({
  label,
  tracks,
  isActive,
  offSelected,
  onSelect,
  offOption,
  badgeFor,
}: {
  label: string;
  tracks: StreamChoice[];
  isActive: (t: StreamChoice) => boolean;
  offSelected: boolean;
  onSelect: (track: StreamChoice | null) => void;
  offOption: boolean;
  /// When set, called for each row to compute an optional per-track
  /// badge (e.g. "Burn-in" for picture-based subtitle codecs). Returning
  /// null produces no badge.
  badgeFor?: (t: StreamChoice) => string | null;
}) {
  return (
    <div className="border-r border-white/10 last:border-r-0">
      <div className="border-b border-white/10 px-4 py-3 text-[0.7rem] font-semibold uppercase tracking-wider text-white/60">
        {label}
      </div>
      <ul className="max-h-72 overflow-y-auto py-2">
        {offOption && (
          <TrackRow
            label="Off"
            active={offSelected}
            onClick={() => onSelect(null)}
          />
        )}
        {tracks.map((t, i) => (
          <TrackRow
            key={t.externalUrl ?? `embedded-${t.idx}-${i}`}
            label={t.label}
            active={isActive(t)}
            onClick={() => onSelect(t)}
            badge={badgeFor?.(t) ?? null}
          />
        ))}
        {tracks.length === 0 && !offOption && (
          <li className="px-4 py-2 text-sm text-white/50">None available</li>
        )}
      </ul>
    </div>
  );
}

function TrackRow({
  label,
  active,
  onClick,
  disabled = false,
  badge = null,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
  /// Renders the row at low opacity and blocks clicks. Used by the
  /// Quality column to indicate tiers above source resolution that
  /// won't actually produce sharper output.
  disabled?: boolean;
  /// Optional secondary tag appended after the label (e.g. "Burn-in"
  /// for picture-based subtitles). Right-aligned in amber so it reads
  /// as informational warning rather than label noise.
  badge?: string | null;
}) {
  return (
    <li>
      <button
        type="button"
        onClick={onClick}
        role="menuitemradio"
        aria-checked={active}
        aria-disabled={disabled}
        disabled={disabled}
        className={`flex w-full items-center gap-2 px-4 py-2 text-left text-sm transition-colors focus:outline-none focus-visible:bg-white/10 ${
          disabled
            ? "cursor-not-allowed text-white/30"
            : active
              ? "text-white"
              : "text-white/75 hover:text-white"
        }`}
      >
        <span
          aria-hidden
          className={`flex h-4 w-4 shrink-0 items-center justify-center text-(--color-accent) ${
            active ? "opacity-100" : "opacity-0"
          }`}
        >
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="3"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <polyline points="20 6 9 17 4 12" />
          </svg>
        </span>
        <span className="truncate">{label}</span>
        {badge && (
          <span className="ml-auto shrink-0 rounded-sm border border-amber-500/30 bg-amber-500/15 px-1 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-amber-300">
            {badge}
          </span>
        )}
      </button>
    </li>
  );
}

function NextEpisodeCountdown({
  secondsLeft,
  href,
  label,
  thumb,
  onCancel,
}: {
  secondsLeft: number;
  href: string;
  label: string;
  thumb?: string;
  onCancel: () => void;
}) {
  return (
    <div className="pointer-events-auto absolute bottom-24 right-2 left-2 z-30 max-w-sm overflow-hidden rounded-md border border-white/10 bg-black/95 shadow-2xl backdrop-blur-sm sm:bottom-28 sm:right-8 sm:left-auto sm:w-80">
      {thumb && (
        // eslint-disable-next-line @next/next/no-img-element
        <img
          src={thumb}
          alt=""
          className="aspect-video w-full object-cover"
        />
      )}
      <div className="px-4 pb-4 pt-3">
        <div className="text-xs uppercase tracking-wider text-white/60">
          Next episode in {secondsLeft}s
        </div>
        <div className="mt-1 line-clamp-2 text-base font-semibold leading-tight">
          {label}
        </div>
        <div className="mt-3 flex gap-2">
          <Link
            href={href}
            className="flex-1 rounded bg-white px-3 py-1.5 text-center text-sm font-semibold text-black transition-colors hover:bg-white/85"
          >
            Watch now
          </Link>
          <button
            type="button"
            onClick={onCancel}
            className="rounded border border-white/40 px-3 py-1.5 text-sm font-medium text-white transition-colors hover:border-white"
          >
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}

function VolumeControl({
  muted,
  volume,
  onToggleMute,
  onVolumeChange,
}: {
  muted: boolean;
  volume: number;
  onToggleMute: () => void;
  onVolumeChange: (v: number) => void;
}) {
  const [hovered, setHovered] = useState(false);
  const trackRef = useRef<HTMLDivElement>(null);
  const effectiveVolume = muted ? 0 : volume;

  function pointToVolume(clientX: number): number {
    const track = trackRef.current;
    if (!track) return effectiveVolume;
    const rect = track.getBoundingClientRect();
    return Math.max(0, Math.min(1, (clientX - rect.left) / rect.width));
  }

  function onPointerDown(e: React.PointerEvent<HTMLDivElement>) {
    e.preventDefault();
    onVolumeChange(pointToVolume(e.clientX));
    const onMove = (ev: PointerEvent) =>
      onVolumeChange(pointToVolume(ev.clientX));
    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  }

  return (
    <div
      className="flex items-center"
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      <IconButton
        onClick={onToggleMute}
        aria-label={muted ? "Unmute" : "Mute"}
      >
        {muted || volume === 0 ? <VolumeMutedIcon /> : <VolumeIcon />}
      </IconButton>
      <div
        ref={trackRef}
        onPointerDown={onPointerDown}
        role="slider"
        aria-label="Volume"
        aria-valuemin={0}
        aria-valuemax={1}
        aria-valuenow={effectiveVolume}
        // Hidden on mobile — touch users adjust volume via OS controls,
        // and a hover-reveal slider is unreachable without a pointer.
        // The mute toggle stays available for both.
        className={`relative hidden h-1 cursor-pointer overflow-hidden rounded-full bg-white/30 transition-all duration-150 sm:block ${
          hovered ? "ml-1 w-24 opacity-100" : "ml-0 w-0 opacity-0"
        }`}
      >
        <div
          className="absolute inset-y-0 left-0 bg-white"
          style={{ width: `${effectiveVolume * 100}%` }}
        />
      </div>
    </div>
  );
}

function SpeedControl({
  rate,
  open,
  onToggle,
  onClose,
  onSelect,
}: {
  rate: number;
  open: boolean;
  onToggle: () => void;
  onClose: () => void;
  onSelect: (rate: number) => void;
}) {
  const wrapRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    function onDocClick(e: MouseEvent) {
      if (!wrapRef.current?.contains(e.target as Node)) onClose();
    }
    function onEsc(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    document.addEventListener("mousedown", onDocClick);
    document.addEventListener("keydown", onEsc);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
      document.removeEventListener("keydown", onEsc);
    };
  }, [open, onClose]);

  return (
    <div ref={wrapRef} className="relative">
      <IconButton
        onClick={onToggle}
        aria-label="Playback speed"
        aria-haspopup="menu"
        aria-expanded={open}
      >
        <SpeedIcon />
      </IconButton>
      {open && (
        <div
          role="menu"
          onKeyDown={handleMenuArrowKeys}
          className="fixed inset-x-2 bottom-2 z-50 overflow-hidden rounded-md border border-white/10 bg-black/95 shadow-2xl backdrop-blur-sm sm:absolute sm:inset-x-auto sm:bottom-full sm:right-0 sm:mb-3 sm:w-32"
        >
          <ul className="py-2">
            {SPEED_OPTIONS.map((opt) => {
              const active = Math.abs(opt - rate) < 0.001;
              return (
                <li key={opt}>
                  <button
                    type="button"
                    onClick={() => {
                      onSelect(opt);
                      onClose();
                    }}
                    role="menuitemradio"
                    aria-checked={active}
                    className={`flex w-full items-center gap-2 px-4 py-1.5 text-left text-sm transition-colors focus:outline-none focus-visible:bg-white/10 ${
                      active ? "text-white" : "text-white/75 hover:text-white"
                    }`}
                  >
                    <span
                      aria-hidden
                      className={`flex h-4 w-4 shrink-0 items-center justify-center text-(--color-accent) ${
                        active ? "opacity-100" : "opacity-0"
                      }`}
                    >
                      <svg
                        width="14"
                        height="14"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        strokeWidth="3"
                        strokeLinecap="round"
                        strokeLinejoin="round"
                      >
                        <polyline points="20 6 9 17 4 12" />
                      </svg>
                    </span>
                    <span className="tabular-nums">
                      {opt === 1 ? "Normal" : `${opt}×`}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </div>
  );
}

function EpisodesControl({
  open,
  episodes,
  currentRatingKey,
  currentSeasonId,
  showId,
  showTitle,
  seasons,
  onToggle,
  onClose,
}: {
  open: boolean;
  episodes: EpisodeSibling[];
  currentRatingKey?: string;
  currentSeasonId?: number;
  showId?: number;
  showTitle?: string;
  seasons?: { id: number; season_number: number; title: string | null }[];
  onToggle: () => void;
  onClose: () => void;
}) {
  const wrapRef = useRef<HTMLDivElement>(null);
  // Local override for season-switching. `null` means "show the route's
  // current season" — the popup re-opens to that state because the
  // override is cleared whenever the popup is dismissed (see the close
  // handler in the wrapper). Storing only an override avoids the
  // cascading-render setState-in-effect that would otherwise be needed
  // to reset state when `open` flips back to true.
  const [override, setOverride] = useState<{
    seasonId: number;
    episodes: EpisodeSibling[];
    label: string | undefined;
  } | null>(null);
  const [loadingSeason, setLoadingSeason] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);

  const viewSeasonId = override?.seasonId ?? currentSeasonId;
  const viewEpisodes = override?.episodes ?? episodes;
  const viewSeasonLabel = override?.label ?? episodes[0]?.parentTitle;

  // Wrapper close handler: clear the override + collapse the picker so
  // the popup is "fresh" on the next open. Keeps the effect-free reset.
  const handleClose = useCallback(() => {
    setOverride(null);
    setPickerOpen(false);
    onClose();
  }, [onClose]);

  useEffect(() => {
    if (!open) return;
    function onDocClick(e: MouseEvent) {
      if (!wrapRef.current?.contains(e.target as Node)) handleClose();
    }
    function onEsc(e: KeyboardEvent) {
      if (e.key === "Escape") {
        if (pickerOpen) setPickerOpen(false);
        else handleClose();
      }
    }
    document.addEventListener("mousedown", onDocClick);
    document.addEventListener("keydown", onEsc);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
      document.removeEventListener("keydown", onEsc);
    };
  }, [open, handleClose, pickerOpen]);

  async function switchSeason(seasonId: number) {
    setLoadingSeason(true);
    try {
      const season = await seasonsApi.get(seasonId);
      const mapped: EpisodeSibling[] = season.episodes.map((e) => ({
        ratingKey: `e${e.id}`,
        title: e.title,
        thumb: plexImage(e.thumb_path ?? undefined, 320, 180) ?? undefined,
        summary: e.summary ?? undefined,
        duration: e.duration_ms ?? undefined,
        viewOffset: e.play_state?.position_ms,
        index: e.episode_number,
        parentTitle: `Season ${e.season_number}`,
      }));
      setOverride({
        seasonId,
        episodes: mapped,
        label: mapped[0]?.parentTitle,
      });
      setPickerOpen(false);
    } catch {
      // Best-effort — leave the existing pane in place if the season
      // fetch fails so the user isn't stuck in a half-loaded picker.
    } finally {
      setLoadingSeason(false);
    }
  }

  const canShowPicker = !!(seasons && seasons.length > 1 && showId !== undefined);

  return (
    <div ref={wrapRef} className="relative">
      <IconButton
        onClick={onToggle}
        aria-label="Episodes"
        aria-haspopup="menu"
        aria-expanded={open}
      >
        <EpisodesIcon />
      </IconButton>
      {open && (
        <div
          role="menu"
          onKeyDown={handleMenuArrowKeys}
          className="fixed inset-x-2 bottom-2 z-50 max-h-[75vh] overflow-y-auto rounded-md border border-white/10 bg-black/95 shadow-2xl backdrop-blur-sm sm:absolute sm:inset-x-auto sm:bottom-full sm:right-0 sm:mb-3 sm:max-h-none sm:w-md sm:overflow-hidden"
        >
          {pickerOpen ? (
            <SeasonPickerPane
              showTitle={showTitle ?? "Seasons"}
              seasons={seasons ?? []}
              currentSeasonId={viewSeasonId}
              loading={loadingSeason}
              onBack={() => setPickerOpen(false)}
              onSelect={switchSeason}
            />
          ) : (
            <>
              <div className="flex items-center gap-2 border-b border-white/10 px-3 py-3">
                {canShowPicker ? (
                  <button
                    type="button"
                    onClick={() => setPickerOpen(true)}
                    aria-label="Choose season"
                    className="flex h-7 w-7 shrink-0 items-center justify-center rounded text-white/80 transition-colors hover:bg-white/10 hover:text-white"
                  >
                    <svg
                      width="18"
                      height="18"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="2"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      aria-hidden
                    >
                      <line x1="19" y1="12" x2="5" y2="12" />
                      <polyline points="12 19 5 12 12 5" />
                    </svg>
                  </button>
                ) : null}
                <div className="truncate text-sm font-semibold">
                  {viewSeasonLabel ?? "Episodes"}
                </div>
              </div>
              <ul className="max-h-112 overflow-y-auto">
                {viewEpisodes.map((ep) => (
                  <EpisodeRow
                    key={ep.ratingKey}
                    episode={ep}
                    active={ep.ratingKey === currentRatingKey}
                    onClose={handleClose}
                  />
                ))}
              </ul>
            </>
          )}
        </div>
      )}
    </div>
  );
}

function SeasonPickerPane({
  showTitle,
  seasons,
  currentSeasonId,
  loading,
  onBack,
  onSelect,
}: {
  showTitle: string;
  seasons: { id: number; season_number: number; title: string | null }[];
  currentSeasonId?: number;
  loading: boolean;
  onBack: () => void;
  onSelect: (seasonId: number) => void;
}) {
  return (
    <div>
      <div className="flex items-center gap-2 border-b border-white/10 px-3 py-3">
        <button
          type="button"
          onClick={onBack}
          aria-label="Back to episodes"
          className="flex h-7 w-7 shrink-0 items-center justify-center rounded text-white/80 transition-colors hover:bg-white/10 hover:text-white"
        >
          <svg
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden
          >
            <line x1="19" y1="12" x2="5" y2="12" />
            <polyline points="12 19 5 12 12 5" />
          </svg>
        </button>
        <div className="truncate text-sm font-semibold">{showTitle}</div>
      </div>
      <ul className="max-h-112 overflow-y-auto">
        {seasons.map((s) => {
          const active = s.id === currentSeasonId;
          const label = s.title?.trim() || `Season ${s.season_number}`;
          return (
            <li key={s.id}>
              <button
                type="button"
                disabled={loading}
                onClick={() => onSelect(s.id)}
                className={`flex w-full items-center gap-3 border-b border-white/5 px-4 py-3 text-left transition-colors last:border-b-0 hover:bg-white/5 disabled:opacity-50 ${
                  active ? "bg-white/5" : ""
                }`}
              >
                <span
                  aria-hidden
                  className={`flex h-4 w-4 shrink-0 items-center justify-center text-(--color-accent) ${
                    active ? "opacity-100" : "opacity-0"
                  }`}
                >
                  <svg
                    width="14"
                    height="14"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="3"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  >
                    <polyline points="20 6 9 17 4 12" />
                  </svg>
                </span>
                <span className="text-sm font-medium">{label}</span>
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

function EpisodeRow({
  episode,
  active,
  onClose,
}: {
  episode: EpisodeSibling;
  active: boolean;
  onClose: () => void;
}) {
  const progress =
    episode.viewOffset && episode.duration
      ? Math.min(100, (episode.viewOffset / episode.duration) * 100)
      : null;

  // Netflix's pattern: the row the viewer is *on* gets the rich
  // thumbnail-plus-synopsis treatment; every other row is a compact
  // number + title strip with a small progress underline. This keeps
  // the popup short so it doesn't dominate the player chrome and
  // makes the active episode pop without needing a separate accent
  // colour.
  if (active) {
    return (
      <li>
        <Link
          href={`/watch/${episode.ratingKey}`}
          onClick={onClose}
          aria-current="true"
          className="flex gap-3 border-b border-white/5 border-l-2 border-l-(--color-accent) bg-white/5 px-4 py-3 transition-colors last:border-b-0 hover:bg-white/10"
        >
          <div className="relative aspect-video w-32 shrink-0 overflow-hidden rounded bg-black/50">
            {episode.thumb && (
              // eslint-disable-next-line @next/next/no-img-element
              <img
                src={episode.thumb}
                alt=""
                loading="lazy"
                className="h-full w-full object-cover"
              />
            )}
            {progress !== null && (
              <div className="absolute inset-x-0 bottom-0 h-0.5 bg-white/25">
                <div
                  className="h-full bg-(--color-accent)"
                  style={{ width: `${progress}%` }}
                />
              </div>
            )}
          </div>
          <div className="min-w-0 flex-1">
            <div className="flex items-baseline gap-2">
              {episode.index !== undefined && (
                <span className="text-sm font-semibold tabular-nums text-white/85">
                  {episode.index}
                </span>
              )}
              <span className="line-clamp-1 text-sm font-medium">
                {episode.title}
              </span>
            </div>
            {episode.summary && (
              <p className="mt-1 line-clamp-2 text-xs text-white/65">
                {episode.summary}
              </p>
            )}
          </div>
        </Link>
      </li>
    );
  }

  return (
    <li>
      <Link
        href={`/watch/${episode.ratingKey}`}
        onClick={onClose}
        className="flex items-center gap-3 border-b border-white/5 px-4 py-2.5 transition-colors last:border-b-0 hover:bg-white/5"
      >
        {episode.index !== undefined && (
          <span className="w-6 shrink-0 text-sm font-semibold tabular-nums text-white/70">
            {episode.index}
          </span>
        )}
        <span className="line-clamp-1 flex-1 text-sm text-white/90">
          {episode.title}
        </span>
        {progress !== null && (
          <span className="hidden h-0.5 w-12 shrink-0 overflow-hidden rounded bg-white/15 sm:block">
            <span
              className="block h-full bg-(--color-accent)"
              style={{ width: `${progress}%` }}
            />
          </span>
        )}
      </Link>
    </li>
  );
}

function HotkeysOverlay({ onClose }: { onClose: () => void }) {
  // Esc + Tab focus cycling + restore-focus-on-close handled by
  // the shared hook. The player's global keyboard handler also
  // has a `?` toggle so opening + closing with the same key
  // works without needing a separate listener here.
  const dialogRef = useRef<HTMLDivElement>(null);
  const titleId = "hotkeys-dialog-title";
  useFocusTrap(dialogRef, { onClose });
  // Two columns of {keys, action} for the hotkey reference. Keep the
  // action labels short — this overlay isn't documentation, it's a
  // glance-and-go reminder. `?` toggles itself so the user can
  // dismiss with the same key they opened it with.
  const groups: ReadonlyArray<{ title: string; items: ReadonlyArray<[string, string]> }> = [
    {
      title: "Playback",
      items: [
        ["Space / k", "Play / Pause"],
        ["←  /  →", "Seek 10s back / fwd"],
        ["j  /  l", "Seek 10s back / fwd"],
        ["Home / End", "Seek to start / end"],
        ["0 – 9", "Jump to 0 – 90%"],
        [".  /  ,", "Speed up / slow down"],
        ["n", "Next episode"],
      ],
    },
    {
      title: "Volume + display",
      items: [
        ["↑  /  ↓", "Volume up / down"],
        ["m", "Mute"],
        ["f", "Fullscreen"],
        ["p", "Picture-in-picture"],
        ["c", "Toggle subtitles"],
      ],
    },
    {
      title: "Overlays",
      items: [
        ["s", "Stats for nerds"],
        ["?", "This help"],
        ["Esc", "Close overlay"],
      ],
    },
  ];
  return (
    <div
      className="absolute inset-0 z-40 flex items-center justify-center bg-black/80 p-4"
      onClick={onClose}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="max-h-full w-full max-w-2xl overflow-y-auto rounded-lg border border-white/20 bg-neutral-950/95 p-6 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-4 flex items-baseline justify-between gap-2">
          <h2 id={titleId} className="text-lg font-semibold">
            Keyboard shortcuts
          </h2>
          <button
            type="button"
            onClick={onClose}
            className="text-xs text-white/55 hover:text-white"
          >
            Close (Esc)
          </button>
        </div>
        <div className="grid grid-cols-1 gap-6 sm:grid-cols-3">
          {groups.map((g) => (
            <div key={g.title}>
              <h3 className="mb-2 text-xs font-semibold uppercase tracking-wider text-white/45">
                {g.title}
              </h3>
              <dl className="space-y-1.5">
                {g.items.map(([keys, action]) => (
                  <div
                    key={keys}
                    className="flex items-baseline justify-between gap-3"
                  >
                    <dt className="font-mono text-xs text-white/85">{keys}</dt>
                    <dd className="text-right text-xs text-white/65">{action}</dd>
                  </div>
                ))}
              </dl>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

function LoadingSpinner() {
  return (
    <div className="pointer-events-none absolute inset-0 z-10 flex items-center justify-center bg-black/40">
      <div className="h-20 w-20 animate-spin rounded-full border-4 border-white/10 border-t-(--color-accent)" />
    </div>
  );
}

/// Subtler than the loading spinner: small pill at the top of the
/// frame that says "Reconnecting…" while HLS.js retries a fatal
/// error in the background. Avoids the alarming full-screen error
/// chrome for transient blips that resolve in a second or two.
function ReconnectingOverlay() {
  return (
    <div className="pointer-events-none absolute inset-x-0 top-20 z-10 flex justify-center">
      <div className="flex items-center gap-2 rounded-full border border-white/15 bg-black/75 px-3 py-1.5 text-xs font-medium text-white/85 shadow-lg backdrop-blur-sm">
        <span className="inline-block h-2 w-2 animate-pulse rounded-full bg-(--color-accent)" />
        Reconnecting…
      </div>
    </div>
  );
}

/// "Stats for nerds" panel: small, monospace, top-right of the frame.
/// Visible-only sampling (500ms interval, torn down on close) so it
/// adds zero cost when hidden. Mirrors the YouTube/Chrome dev-overlay
/// convention so support reports can paste a screenshot and a
/// developer can immediately see decoded resolution, the active HLS
/// level, buffer ahead, dropped-frame ratio, and the resolved
/// transcoder decisions.
function StatsOverlay({
  videoRef,
  hlsRef,
  sessionStatus,
  targetHeight,
  onClose,
}: {
  videoRef: React.RefObject<HTMLVideoElement | null>;
  hlsRef: React.RefObject<Hls | null>;
  sessionStatus: {
    height: number | null;
    sourceHeight: number | null;
    encoder: string | null;
    videoTreatment: "copy" | "reencode" | null;
    audioTreatment: "copy" | "reencode" | null;
  } | null;
  targetHeight: number | null;
  onClose: () => void;
}) {
  const [snap, setSnap] = useState<{
    decodedWidth: number;
    decodedHeight: number;
    levelLabel: string | null;
    levelBitrateKbps: number | null;
    bandwidthKbps: number | null;
    bufferAheadSec: number | null;
    droppedFrames: number;
    decodedFrames: number;
    dropRatio: number;
    playbackRate: number;
    volumePct: number;
  } | null>(null);

  useEffect(() => {
    let raf: number | null = null;
    function sample() {
      const v = videoRef.current;
      const hls = hlsRef.current;
      if (!v) {
        raf = window.setTimeout(sample, 500);
        return;
      }
      // Forward buffer: find the range containing currentTime and
      // measure to its end. If currentTime falls in a gap (rare; HLS
      // usually fills gaplessly), report 0.
      let bufferAheadSec: number | null = null;
      const now = v.currentTime;
      for (let i = 0; i < v.buffered.length; i++) {
        if (v.buffered.start(i) <= now && v.buffered.end(i) >= now) {
          bufferAheadSec = v.buffered.end(i) - now;
          break;
        }
      }

      // getVideoPlaybackQuality is the modern spec; Safari (older) only
      // exposes webkit-prefixed counters. Fall back if needed.
      type LegacyVideo = HTMLVideoElement & {
        webkitDroppedFrameCount?: number;
        webkitDecodedFrameCount?: number;
      };
      const legacy = v as LegacyVideo;
      let dropped = 0;
      let decoded = 0;
      if (typeof v.getVideoPlaybackQuality === "function") {
        const q = v.getVideoPlaybackQuality();
        dropped = q.droppedVideoFrames;
        decoded = q.totalVideoFrames;
      } else {
        dropped = legacy.webkitDroppedFrameCount ?? 0;
        decoded = legacy.webkitDecodedFrameCount ?? 0;
      }
      const dropRatio = decoded > 0 ? dropped / decoded : 0;

      let levelLabel: string | null = null;
      let levelBitrateKbps: number | null = null;
      let bandwidthKbps: number | null = null;
      if (hls) {
        const idx = hls.currentLevel;
        if (idx >= 0 && hls.levels && hls.levels[idx]) {
          const lvl = hls.levels[idx];
          const h = lvl.height ?? null;
          levelLabel = h ? `${h}p` : `level ${idx}`;
          levelBitrateKbps = lvl.bitrate ? Math.round(lvl.bitrate / 1000) : null;
        } else if (idx === -1) {
          levelLabel = "auto";
        }
        if (typeof hls.bandwidthEstimate === "number") {
          bandwidthKbps = Math.round(hls.bandwidthEstimate / 1000);
        }
      }

      setSnap({
        decodedWidth: v.videoWidth || 0,
        decodedHeight: v.videoHeight || 0,
        levelLabel,
        levelBitrateKbps,
        bandwidthKbps,
        bufferAheadSec,
        droppedFrames: dropped,
        decodedFrames: decoded,
        dropRatio,
        playbackRate: v.playbackRate,
        volumePct: v.muted ? 0 : Math.round(v.volume * 100),
      });
      raf = window.setTimeout(sample, 500);
    }
    sample();
    return () => {
      if (raf !== null) window.clearTimeout(raf);
    };
  }, [videoRef, hlsRef]);

  // Color the dropped-frame ratio so problem playback jumps out: green
  // <0.5%, amber 0.5-2%, red >2%. These thresholds are the same ones
  // Chrome's media-internals uses to call a session "unhealthy".
  const dropColor = !snap
    ? "text-white/70"
    : snap.dropRatio > 0.02
      ? "text-red-300"
      : snap.dropRatio > 0.005
        ? "text-amber-300"
        : "text-emerald-300";

  return (
    <div className="pointer-events-auto absolute right-4 top-20 z-20 w-72 rounded-md border border-white/10 bg-black/85 p-3 font-mono text-xs text-white/90 shadow-xl backdrop-blur-sm">
      <div className="mb-2 flex items-center justify-between">
        <span className="text-[10px] font-semibold uppercase tracking-wider text-white/55">
          Playback stats
        </span>
        <button
          type="button"
          onClick={onClose}
          aria-label="Close stats"
          className="rounded p-0.5 text-white/55 transition-colors hover:bg-white/10 hover:text-white"
        >
          <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden>
            <path
              d="M2 2l8 8M10 2l-8 8"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
            />
          </svg>
        </button>
      </div>

      <dl className="space-y-1">
        <StatRow label="Decoded">
          {snap && snap.decodedWidth > 0
            ? `${snap.decodedWidth}×${snap.decodedHeight}`
            : "—"}
        </StatRow>
        <StatRow label="HLS level">
          {snap?.levelLabel
            ? `${snap.levelLabel}${
                snap.levelBitrateKbps ? ` · ${snap.levelBitrateKbps} kbps` : ""
              }`
            : "—"}
        </StatRow>
        <StatRow label="Bandwidth est.">
          {snap?.bandwidthKbps != null ? `${snap.bandwidthKbps} kbps` : "—"}
        </StatRow>
        <StatRow label="Buffer ahead">
          {snap?.bufferAheadSec != null
            ? `${snap.bufferAheadSec.toFixed(1)} s`
            : "—"}
        </StatRow>
        <StatRow label="Frames" valueClassName={dropColor}>
          {snap
            ? `${snap.droppedFrames} dropped / ${snap.decodedFrames} (${(
                snap.dropRatio * 100
              ).toFixed(2)}%)`
            : "—"}
        </StatRow>
        <StatRow label="Rate / vol">
          {snap ? `${snap.playbackRate.toFixed(2)}× · ${snap.volumePct}%` : "—"}
        </StatRow>
      </dl>

      {(sessionStatus?.encoder ||
        sessionStatus?.sourceHeight ||
        targetHeight) && (
        <>
          <div className="my-2 h-px bg-white/10" />
          <dl className="space-y-1">
            {sessionStatus?.sourceHeight ? (
              <StatRow label="Source">{sessionStatus.sourceHeight}p</StatRow>
            ) : null}
            {targetHeight ? (
              <StatRow label="Target">{targetHeight}p</StatRow>
            ) : null}
            {sessionStatus?.encoder ? (
              <StatRow label="Encoder">{sessionStatus.encoder}</StatRow>
            ) : null}
            {sessionStatus?.videoTreatment ? (
              <StatRow label="Video">
                {sessionStatus.videoTreatment === "copy" ? "copy" : "re-encode"}
              </StatRow>
            ) : null}
            {sessionStatus?.audioTreatment ? (
              <StatRow label="Audio">
                {sessionStatus.audioTreatment === "copy" ? "copy" : "re-encode"}
              </StatRow>
            ) : null}
          </dl>
        </>
      )}

      <div className="mt-2 text-[10px] text-white/40">Press S to toggle.</div>
    </div>
  );
}

function StatRow({
  label,
  children,
  valueClassName = "text-white/85",
}: {
  label: string;
  children: React.ReactNode;
  valueClassName?: string;
}) {
  return (
    <div className="flex items-center justify-between gap-3">
      <dt className="text-white/50">{label}</dt>
      <dd className={`tabular-nums ${valueClassName}`}>{children}</dd>
    </div>
  );
}

function ErrorOverlay({ message }: { message: string }) {
  return (
    <div className="absolute inset-0 z-10 flex items-center justify-center bg-black/85">
      <div className="max-w-md px-6 text-center">
        <p className="mb-2 text-lg font-semibold text-(--color-accent)">
          Playback failed
        </p>
        <p className="text-sm text-white/70">{message}</p>
        <p className="mt-4 text-xs text-white/50">
          Common causes: server unreachable, transcoder busy, or the file
          can&apos;t be HLS-streamed.
        </p>
        <div className="mt-5 flex items-center justify-center gap-2">
          <button
            type="button"
            onClick={() => window.location.reload()}
            className="rounded-md bg-white px-4 py-2 text-sm font-semibold text-black transition-colors hover:bg-white/85"
          >
            Try again
          </button>
          <button
            type="button"
            onClick={() => window.history.back()}
            className="rounded-md border border-white/20 px-4 py-2 text-sm font-medium text-white/80 transition-colors hover:border-white/40 hover:text-white"
          >
            Go back
          </button>
        </div>
      </div>
    </div>
  );
}

/// Map HLS.js's structured fatal-error payload to a user-readable
/// sentence. Falls back to a generic "Something went wrong" when the
/// detail isn't one we've seen — better than leaking `networkError /
/// fragLoadError` to the overlay.
function friendlyHlsError(data: { type?: string; details?: string }): string {
  const details = data.details ?? "";
  switch (details) {
    case "manifestLoadError":
    case "manifestLoadTimeOut":
    case "manifestParsingError":
      return "Couldn't load the stream. The transcoder may still be warming up — try again.";
    case "levelLoadError":
    case "levelLoadTimeOut":
      return "Lost the stream playlist. Try again in a moment.";
    case "fragLoadError":
    case "fragLoadTimeOut":
      return "A video segment didn't load. Your connection may have dropped.";
    case "bufferStalledError":
      return "Playback stalled. Your connection may be too slow for the current quality.";
    case "internalException":
    case "bufferAppendError":
    case "bufferAppendingError":
      return "The browser couldn't decode the stream. Try lowering the quality.";
    case "audioTrackLoadError":
    case "audioTrackLoadTimeOut":
      return "Audio track failed to load. Try switching audio or refreshing.";
    case "subtitleTrackLoadError":
    case "subtitleTrackLoadTimeOut":
      return "Subtitle track failed to load. Disable subtitles or try again.";
    default:
      if (data.type === "networkError") {
        return "Network problem. Check your connection and try again.";
      }
      if (data.type === "mediaError") {
        return "The browser couldn't decode the stream. Try lowering the quality.";
      }
      return "Something went wrong with playback. Try refreshing.";
  }
}

function ProgressBar({
  currentTime,
  duration,
  bufferedEnd,
  onSeek,
  onSeekHint,
  onScrubChange,
  markers,
}: {
  currentTime: number;
  duration: number;
  /// Trailing edge of the contiguous buffered range that includes
  /// currentTime, in source-time seconds. Drives the Netflix-style
  /// lighter overlay between the playhead and the buffer edge.
  /// 0 means "no buffer info yet" — the overlay just doesn't render.
  bufferedEnd: number;
  onSeek: (t: number) => void;
  /// Fires during drag (debounced internally) at the candidate
  /// release position. The player uses this to pre-warm an ffmpeg
  /// session at the target so the actual seek-on-release is near-
  /// instant. Optional — ProgressBar works without it.
  onSeekHint?: (t: number) => void;
  /// Fires whenever the user enters / exits scrub mode. The parent
  /// uses this to suppress the stall watchdog's currentTime nudge and
  /// the auto-skip-intro effect from firing mid-drag, both of which
  /// would yank the playhead from under the user.
  onScrubChange?: (scrubbing: boolean) => void;
  /// Intro / credits / chapter regions to overlay as colored
  /// segments. Each marker spans [start_ms, end_ms] on the source
  /// timeline; we position them as a percentage of duration.
  markers?: PlayerMarker[];
}) {
  const trackRef = useRef<HTMLDivElement>(null);
  const [hovering, setHovering] = useState(false);
  const [scrubbing, setScrubbing] = useState(false);
  // Pointer-x within the track, in pixels from its left edge. `null`
  // when the mouse isn't over the track. Drives both the time tooltip
  // and the scrub-preview thumbnail rendering.
  const [hoverX, setHoverX] = useState<number | null>(null);
  // Scrub position WHILE dragging. The visible progress bar fill
  // follows this so the user sees instant feedback, but the actual
  // `onSeek` call holds off until pointerup — committing per-move
  // would session-restart on every micromove past the buffer.
  const [scrubTime, setScrubTime] = useState<number | null>(null);
  const hintTimerRef = useRef<number | null>(null);
  // Mirrored width of the progress track. Read during render to map
  // `hoverX` → time; updated by a ResizeObserver. Storing in state
  // (vs. reading trackRef.current.getBoundingClientRect() at render
  // time) is what keeps the component pure under strict mode.
  const [trackWidth, setTrackWidth] = useState(0);
  useEffect(() => {
    const node = trackRef.current;
    if (!node) return;
    setTrackWidth(node.getBoundingClientRect().width);
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (entry) setTrackWidth(entry.contentRect.width);
    });
    ro.observe(node);
    return () => ro.disconnect();
  }, []);

  const pointToTime = useCallback(
    (clientX: number): number => {
      const track = trackRef.current;
      if (!track || !duration) return 0;
      const rect = track.getBoundingClientRect();
      const ratio = Math.max(
        0,
        Math.min(1, (clientX - rect.left) / rect.width),
      );
      return ratio * duration;
    },
    [duration],
  );

  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    setScrubbing(true);
    onScrubChange?.(true);
    const initial = pointToTime(e.clientX);
    setScrubTime(initial);

    let lastT = initial;
    const onMove = (ev: PointerEvent) => {
      const t = pointToTime(ev.clientX);
      lastT = t;
      setScrubTime(t);
      // Debounced pre-warm — after 350 ms of relatively stable drag
      // position, kick off a session at that target so the eventual
      // release seek finds segments already encoding. The 350 ms
      // window is empirically the difference between "user is
      // dragging through" and "user has stopped on a target".
      if (onSeekHint) {
        if (hintTimerRef.current !== null) {
          window.clearTimeout(hintTimerRef.current);
        }
        hintTimerRef.current = window.setTimeout(() => {
          hintTimerRef.current = null;
          onSeekHint(t);
        }, 350);
      }
    };
    const onUp = () => {
      setScrubbing(false);
      onScrubChange?.(false);
      setScrubTime(null);
      if (hintTimerRef.current !== null) {
        window.clearTimeout(hintTimerRef.current);
        hintTimerRef.current = null;
      }
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      // Commit ONCE at release — `seekTo` either does a native
      // currentTime jump (if buffered) or tears down + restarts the
      // session at this position. No more multi-restart cascade
      // from intermediate drag positions.
      onSeek(lastT);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  };

  const onMouseMove = (e: React.MouseEvent<HTMLDivElement>) => {
    const track = trackRef.current;
    if (!track) return;
    setHoverX(e.clientX - track.getBoundingClientRect().left);
  };

  // While scrubbing, the fill follows the drag position even though
  // we don't commit the seek until release. This is what the user
  // expects — instant visual feedback during the drag without the
  // session churn that would come from honoring every micromove.
  const displayTime = scrubTime ?? currentTime;
  const progress = duration > 0 ? (displayTime / duration) * 100 : 0;
  const expanded = hovering || scrubbing;
  const hoverTime =
    hoverX !== null && duration > 0 && trackWidth > 0
      ? (hoverX / trackWidth) * duration
      : null;

  return (
    <div
      ref={trackRef}
      onPointerDown={onPointerDown}
      onMouseEnter={() => setHovering(true)}
      onMouseLeave={() => {
        setHovering(false);
        setHoverX(null);
      }}
      onMouseMove={onMouseMove}
      role="slider"
      aria-label="Seek"
      aria-valuemin={0}
      aria-valuemax={duration || 0}
      aria-valuenow={currentTime}
      className="group relative cursor-pointer py-2"
    >
      <div
        className={`relative w-full overflow-hidden rounded-full bg-white/30 transition-[height] duration-150 ${
          expanded ? "h-1.5" : "h-1"
        }`}
      >
        {/*
          Marker overlays sit BEHIND the progress fill so the played
          portion still reads as the accent color, but the unplayed
          portion shows tinted segments where intros / credits live.
          Color picked per kind: intro = teal (skip-affordance
          familiar from Netflix), credits = amber, anything else
          neutral. Auto-detected segments (source=auto) get a
          stronger fill + a 1px top accent stripe so the user can
          see at a glance which ranges were machine-found; manual
          ones stay subdued so they don't visually compete with the
          buffer / playhead. Pointer events disabled so they don't
          intercept scrubs.
        */}
        {markers && duration > 0 && markers.map((m, i) => {
          const startPct = Math.max(0, Math.min(100, (m.start_ms / 1000 / duration) * 100));
          const endPct = Math.max(0, Math.min(100, (m.end_ms / 1000 / duration) * 100));
          const widthPct = Math.max(0, endPct - startPct);
          if (widthPct < 0.1) return null;
          const isAuto = m.source === "auto";
          const fillCls = m.kind === "credits"
            ? isAuto ? "bg-amber-400/70" : "bg-amber-400/30"
            : m.kind === "intro"
              ? isAuto ? "bg-sky-400/70" : "bg-sky-400/30"
              : isAuto ? "bg-white/40" : "bg-white/20";
          const stripeCls = isAuto
            ? m.kind === "credits"
              ? "bg-amber-300"
              : m.kind === "intro"
                ? "bg-sky-300"
                : "bg-white"
            : null;
          return (
            <div
              key={`${m.kind}-${m.start_ms}-${i}`}
              aria-hidden
              className="pointer-events-none absolute inset-y-0"
              style={{ left: `${startPct}%`, width: `${widthPct}%` }}
            >
              <div className={`absolute inset-0 ${fillCls}`} />
              {stripeCls && (
                <div className={`absolute inset-x-0 top-0 h-px ${stripeCls}`} />
              )}
            </div>
          );
        })}
        {/*
          Netflix-style buffer-ahead overlay. Sits ABOVE the marker
          tints but BELOW the played-progress fill so the played
          portion still shows in accent color; the unplayed-but-
          buffered range gets a lighter white tint. We clamp at 100%
          so an over-counted buffer (rare; HLS.js sometimes reports
          slightly past video duration) doesn't draw past the
          track. Hidden entirely when bufferedEnd hasn't caught up
          to the playhead — avoids a flicker right after seek when
          buffered ranges haven't been recomputed.
        */}
        {duration > 0 && bufferedEnd > displayTime && (
          <div
            aria-hidden
            className="pointer-events-none absolute inset-y-0 bg-white/45"
            style={{
              left: `${progress}%`,
              width: `${Math.max(0, Math.min(100, (bufferedEnd / duration) * 100) - progress)}%`,
            }}
          />
        )}
        <div
          className="absolute inset-y-0 left-0 bg-(--color-accent)"
          style={{ width: `${progress}%` }}
        />
      </div>
      {expanded && (
        <div
          className="absolute top-1/2 h-3.5 w-3.5 -translate-x-1/2 -translate-y-1/2 rounded-full bg-(--color-accent) shadow-md"
          style={{ left: `${progress}%` }}
        />
      )}
      {/*
        When the cursor sits inside a marker region, surface its
        label as a small pill above the bar. Useful as an
        affordance — "the credits start here" is much clearer than
        "this orange tint here means something."
      */}
      {hoverX !== null && hoverTime !== null && (
        (() => {
          const hovered = activeMarker(hoverTime * 1000, markers);
          if (!hovered) return null;
          return (
            <div
              className="pointer-events-none absolute -translate-x-1/2 rounded-full border border-white/15 bg-black/85 px-2 py-0.5 text-[0.65rem] font-semibold uppercase tracking-wider text-white/85 shadow-md"
              style={{ left: hoverX, bottom: "calc(100% + 0.5rem)" }}
            >
              {hovered.kind}
            </div>
          );
        })()
      )}
      {/*
        Time bubble visible during scrub. Covers the gap on touch
        devices where there is no hover, and also makes desktop drag
        clearer (the existing fill + thumb don't show a numeric
        target). Sized large + tabular-nums so a thumb on mobile
        doesn't obscure it. Anchored to bottom of the bar with a
        wider offset so the user's finger doesn't sit on top of it.
      */}
      {scrubbing && scrubTime !== null && duration > 0 && (
        <div
          aria-hidden
          className="pointer-events-none absolute -translate-x-1/2 rounded-md border border-white/20 bg-black/90 px-2.5 py-1 text-sm font-semibold tabular-nums text-white shadow-2xl"
          style={{
            left: `${progress}%`,
            bottom: "calc(100% + 1.25rem)",
          }}
        >
          {formatTime(scrubTime)}
        </div>
      )}
    </div>
  );
}

// ── SVG icons ───────────────────────────────────────────────────────────────

function BackIcon() {
  return (
    <svg
      width="22"
      height="22"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <line x1="19" y1="12" x2="5" y2="12" />
      <polyline points="12 19 5 12 12 5" />
    </svg>
  );
}

function PlayIcon() {
  return (
    <svg
      width="28"
      height="28"
      viewBox="0 0 24 24"
      fill="currentColor"
      aria-hidden
    >
      <path d="M6 4l14 8-14 8V4z" />
    </svg>
  );
}

function PauseIcon() {
  return (
    <svg
      width="28"
      height="28"
      viewBox="0 0 24 24"
      fill="currentColor"
      aria-hidden
    >
      <rect x="6" y="4" width="4" height="16" rx="0.5" />
      <rect x="14" y="4" width="4" height="16" rx="0.5" />
    </svg>
  );
}

function Rewind10Icon() {
  return (
    <svg
      width="26"
      height="26"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <path d="M11 4 7 8l4 4" />
      <path d="M7 8h6a6 6 0 1 1-6 6" />
      <text
        x="12"
        y="17"
        textAnchor="middle"
        fontSize="6.5"
        fontWeight="700"
        fill="currentColor"
        stroke="none"
      >
        10
      </text>
    </svg>
  );
}

function Forward10Icon() {
  return (
    <svg
      width="26"
      height="26"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <path d="M13 4l4 4-4 4" />
      <path d="M17 8h-6a6 6 0 1 0 6 6" />
      <text
        x="12"
        y="17"
        textAnchor="middle"
        fontSize="6.5"
        fontWeight="700"
        fill="currentColor"
        stroke="none"
      >
        10
      </text>
    </svg>
  );
}

function VolumeIcon() {
  return (
    <svg
      width="24"
      height="24"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" fill="currentColor" />
      <path d="M15.5 8.5a4 4 0 0 1 0 7" />
      <path d="M18.5 5.5a8 8 0 0 1 0 13" />
    </svg>
  );
}

function VolumeMutedIcon() {
  return (
    <svg
      width="24"
      height="24"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" fill="currentColor" />
      <line x1="22" y1="9" x2="16" y2="15" />
      <line x1="16" y1="9" x2="22" y2="15" />
    </svg>
  );
}

function FullscreenIcon() {
  return (
    <svg
      width="22"
      height="22"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <polyline points="4 9 4 4 9 4" />
      <polyline points="20 9 20 4 15 4" />
      <polyline points="4 15 4 20 9 20" />
      <polyline points="20 15 20 20 15 20" />
    </svg>
  );
}

function FullscreenExitIcon() {
  return (
    <svg
      width="22"
      height="22"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <polyline points="9 4 9 9 4 9" />
      <polyline points="15 4 15 9 20 9" />
      <polyline points="9 20 9 15 4 15" />
      <polyline points="15 20 15 15 20 15" />
    </svg>
  );
}

function NextEpisodeIcon() {
  return (
    <svg
      width="22"
      height="22"
      viewBox="0 0 24 24"
      fill="currentColor"
      aria-hidden
    >
      <path d="M5 4l11 8-11 8V4z" />
      <rect x="17" y="4" width="2.5" height="16" rx="0.5" />
    </svg>
  );
}

function EpisodesIcon() {
  return (
    <svg
      width="22"
      height="22"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <rect x="3" y="6" width="18" height="13" rx="2" />
      <line x1="7" y1="3" x2="17" y2="3" />
      <line x1="6" y1="11" x2="11" y2="11" />
      <line x1="6" y1="14" x2="14" y2="14" />
    </svg>
  );
}

function PipIcon() {
  return (
    <svg
      width="22"
      height="22"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <rect x="2" y="4" width="20" height="14" rx="2" />
      <rect x="12" y="11" width="8" height="6" rx="1" fill="currentColor" />
    </svg>
  );
}

function CaptionsIcon() {
  return (
    <svg
      width="22"
      height="22"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <rect x="2" y="5" width="20" height="14" rx="2" />
      <line x1="6" y1="11" x2="10" y2="11" />
      <line x1="14" y1="11" x2="18" y2="11" />
      <line x1="6" y1="15" x2="11" y2="15" />
      <line x1="15" y1="15" x2="18" y2="15" />
    </svg>
  );
}

function SubtitleSettingsIcon() {
  // Captions glyph with a small gear motif at the corner — signals
  // "subtitles, settings" without needing a tooltip.
  return (
    <svg
      width="22"
      height="22"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <rect x="2" y="5" width="16" height="14" rx="2" />
      <line x1="5" y1="11" x2="9" y2="11" />
      <line x1="5" y1="15" x2="13" y2="15" />
      <circle cx="19" cy="19" r="3" fill="currentColor" stroke="none" />
      <circle cx="19" cy="19" r="1.1" fill="black" stroke="none" />
    </svg>
  );
}

function SpeedIcon() {
  return (
    <svg
      width="22"
      height="22"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <circle cx="12" cy="13" r="8" />
      <polyline points="12 9 12 13 15 15" />
      <line x1="9" y1="3" x2="15" y2="3" />
    </svg>
  );
}


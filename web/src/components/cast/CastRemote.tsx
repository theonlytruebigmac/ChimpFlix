"use client";

/// The connected-cast control surface — the "remote".
///
/// This is what ChimpFlix shows whenever a Cast session is live, in two
/// places that share this one component:
///   * embedded in the immersive `/watch` player (`variant="embedded"`),
///     replacing the local video frame; and
///   * as a full-screen overlay opened from the app-wide mini-controller
///     (`variant="page"`) so the session stays controllable while you
///     browse.
///
/// Every control here drives the RECEIVER via the shared singleton
/// `RemotePlayerController` (see `@/lib/cast`), and every value rendered
/// comes from `useRemotePlayback()`, which mirrors the receiver — so a
/// TV remote or a second phone moving the playhead is reflected here too.
/// Crucially, the Stop control does NOT depend on the live media-status
/// stream, so the user can always tear the session down even if status
/// goes stale (the failure mode that traps Jellyfin web users).

import { useCallback, useEffect, useRef, useState } from "react";
import {
  castPlayPause,
  castSeekToMediaTime,
  castSetVolume,
  castToggleMute,
  endCastSession,
  useCastTrackController,
  useRemotePlayback,
  type CastTrackOption,
} from "@/lib/cast";

function formatTime(totalSeconds: number): string {
  if (!Number.isFinite(totalSeconds) || totalSeconds < 0) totalSeconds = 0;
  const s = Math.floor(totalSeconds % 60);
  const m = Math.floor((totalSeconds / 60) % 60);
  const h = Math.floor(totalSeconds / 3600);
  const pad = (n: number) => n.toString().padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

const SKIP_SECONDS = 10;

export interface CastRemoteProps {
  /// When provided, render a collapse (chevron-down) affordance — used
  /// by the full-screen expanded controller to drop back to the mini
  /// controller. Omitted when embedded in the player.
  onCollapse?: () => void;
  /// "page" = full-screen overlay (its own safe-area padding); "embedded"
  /// = fills the player frame.
  variant?: "page" | "embedded";
}

export function CastRemote({ onCollapse, variant = "page" }: CastRemoteProps) {
  const remote = useRemotePlayback();
  const tracks = useCastTrackController();

  // Local scrub preview: while dragging we show the dragged position and
  // hold off committing to the receiver until release, so the SDK's
  // CURRENT_TIME_CHANGED events (which lag a beat) don't fight the thumb.
  const [scrub, setScrub] = useState<number | null>(null);
  const barRef = useRef<HTMLDivElement>(null);
  const draggingRef = useRef(false);

  const duration = remote.durationS > 0 ? remote.durationS : 0;
  const position = scrub ?? remote.currentTimeS;
  const fraction =
    duration > 0 ? Math.min(1, Math.max(0, position / duration)) : 0;

  const seekToFraction = useCallback(
    (f: number) => {
      if (duration <= 0) return;
      castSeekToMediaTime(Math.max(0, Math.min(duration, f * duration)));
    },
    [duration],
  );

  const fractionFromPointer = useCallback((clientX: number): number => {
    const el = barRef.current;
    if (!el) return 0;
    const rect = el.getBoundingClientRect();
    if (rect.width <= 0) return 0;
    return Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
  }, []);

  const onBarPointerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      if (duration <= 0 || !remote.canSeek) return;
      draggingRef.current = true;
      e.currentTarget.setPointerCapture(e.pointerId);
      setScrub(fractionFromPointer(e.clientX) * duration);
    },
    [duration, remote.canSeek, fractionFromPointer],
  );
  const onBarPointerMove = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      if (!draggingRef.current || duration <= 0) return;
      setScrub(fractionFromPointer(e.clientX) * duration);
    },
    [duration, fractionFromPointer],
  );
  const onBarPointerUp = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      if (!draggingRef.current) return;
      draggingRef.current = false;
      const f = fractionFromPointer(e.clientX);
      seekToFraction(f);
      // Keep the preview pinned briefly so the thumb doesn't snap back to
      // the stale receiver clock before the seek lands; clear on the next
      // tick — the incoming CURRENT_TIME_CHANGED then takes over.
      setScrub(f * duration);
      window.setTimeout(() => {
        if (!draggingRef.current) setScrub(null);
      }, 600);
    },
    [fractionFromPointer, seekToFraction, duration],
  );

  const skip = useCallback(
    (delta: number) => {
      const target = Math.max(0, remote.currentTimeS + delta);
      castSeekToMediaTime(duration > 0 ? Math.min(duration, target) : target);
    },
    [remote.currentTimeS, duration],
  );

  const title = remote.title ?? "Casting";
  const deviceLabel = remote.deviceName
    ? `Playing on ${remote.deviceName}`
    : "Connected";

  const rootPad =
    variant === "page"
      ? "px-[max(1rem,env(safe-area-inset-left))] pb-[max(1.5rem,env(safe-area-inset-bottom))] pt-[max(1.5rem,env(safe-area-inset-top))]"
      : "p-6 sm:p-10";

  return (
    <div
      className={`pointer-events-auto absolute inset-0 z-10 flex flex-col bg-[radial-gradient(circle_at_50%_30%,#16161a_0%,#0a0a0b_75%)] text-white ${rootPad}`}
    >
      {/* Top row: connected device + collapse. */}
      <div className="flex items-center gap-3">
        <CastGlyph className="h-5 w-5 shrink-0 text-(--color-accent)" />
        <span className="min-w-0 flex-1 truncate text-sm font-medium text-white/80">
          {deviceLabel}
        </span>
        {onCollapse && (
          <button
            type="button"
            onClick={onCollapse}
            aria-label="Minimize"
            title="Minimize"
            className="flex h-9 w-9 items-center justify-center rounded-full text-white/70 transition-colors hover:bg-white/10 hover:text-white"
          >
            <ChevronDownIcon />
          </button>
        )}
      </div>

      {/* Artwork + title. */}
      <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-5 py-6">
        <div className="aspect-2/3 w-40 overflow-hidden rounded-lg bg-white/5 shadow-2xl ring-1 ring-white/10 sm:w-48">
          {remote.imageUrl ? (
            // Receiver-supplied poster; plain <img> (not next/image) since
            // the URL is arbitrary and we want zero layout dependency.
            // eslint-disable-next-line @next/next/no-img-element
            <img
              src={remote.imageUrl}
              alt=""
              className="h-full w-full object-cover"
            />
          ) : (
            <div className="flex h-full w-full items-center justify-center">
              <CastGlyph className="h-12 w-12 text-white/25" />
            </div>
          )}
        </div>
        <div className="max-w-md px-4 text-center">
          <div className="truncate text-xl font-semibold sm:text-2xl">
            {title}
          </div>
          {remote.isMediaLoaded ? null : (
            <div className="mt-2 text-sm text-white/55">Loading on TV…</div>
          )}
        </div>
      </div>

      {/* Controls. */}
      <div className="mx-auto w-full max-w-xl">
        {/* Scrub bar. */}
        <div className="flex items-center gap-3">
          <span className="w-12 shrink-0 text-right text-xs tabular-nums text-white/70">
            {formatTime(position)}
          </span>
          <div
            ref={barRef}
            onPointerDown={onBarPointerDown}
            onPointerMove={onBarPointerMove}
            onPointerUp={onBarPointerUp}
            className={`group relative h-8 flex-1 ${
              remote.canSeek ? "cursor-pointer" : "cursor-default opacity-60"
            }`}
          >
            <div className="absolute inset-x-0 top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-white/20">
              <div
                className="h-full rounded-full bg-(--color-accent)"
                style={{ width: `${fraction * 100}%` }}
              />
            </div>
            <div
              className="absolute top-1/2 h-3.5 w-3.5 -translate-x-1/2 -translate-y-1/2 rounded-full bg-white shadow"
              style={{ left: `${fraction * 100}%` }}
            />
          </div>
          <span className="w-12 shrink-0 text-xs tabular-nums text-white/70">
            {duration > 0 ? formatTime(duration) : "--:--"}
          </span>
        </div>

        {/* Transport. */}
        <div className="mt-4 flex items-center justify-center gap-6">
          <button
            type="button"
            onClick={() => skip(-SKIP_SECONDS)}
            disabled={!remote.canSeek}
            aria-label="Skip back 10 seconds"
            title="Back 10s"
            className="flex h-11 w-11 items-center justify-center rounded-full text-white/85 transition-colors hover:bg-white/10 hover:text-white disabled:opacity-40"
          >
            <Replay10Icon />
          </button>
          <button
            type="button"
            onClick={castPlayPause}
            disabled={!remote.canPause}
            aria-label={remote.isPaused ? "Play" : "Pause"}
            title={remote.isPaused ? "Play" : "Pause"}
            className="flex h-16 w-16 items-center justify-center rounded-full bg-white text-black shadow-lg transition-transform hover:scale-105 disabled:opacity-40"
          >
            {remote.isPaused ? <PlayIcon /> : <PauseIcon />}
          </button>
          <button
            type="button"
            onClick={() => skip(SKIP_SECONDS)}
            disabled={!remote.canSeek}
            aria-label="Skip forward 10 seconds"
            title="Forward 10s"
            className="flex h-11 w-11 items-center justify-center rounded-full text-white/85 transition-colors hover:bg-white/10 hover:text-white disabled:opacity-40"
          >
            <Forward10Icon />
          </button>
        </div>

        {/* Volume + tracks + stop. */}
        <div className="mt-5 flex flex-wrap items-center justify-center gap-x-5 gap-y-3">
          {remote.canControlVolume && (
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={castToggleMute}
                aria-label={remote.isMuted ? "Unmute" : "Mute"}
                title={remote.isMuted ? "Unmute" : "Mute"}
                className="flex h-9 w-9 items-center justify-center rounded-full text-white/80 transition-colors hover:bg-white/10 hover:text-white"
              >
                {remote.isMuted || remote.volume === 0 ? (
                  <MuteIcon />
                ) : (
                  <VolumeIcon />
                )}
              </button>
              <input
                type="range"
                min={0}
                max={1}
                step={0.05}
                value={remote.isMuted ? 0 : remote.volume}
                onChange={(e) => castSetVolume(Number(e.target.value))}
                aria-label="Volume"
                className="h-1 w-24 cursor-pointer accent-(--color-accent)"
              />
            </div>
          )}

          {tracks && (
            <div className="flex items-center gap-2">
              <TrackMenu label="Audio" options={tracks.audio} busy={tracks.busy} />
              <TrackMenu
                label="Subtitles"
                options={tracks.subtitle}
                busy={tracks.busy}
              />
              <TrackMenu
                label="Quality"
                options={tracks.quality}
                busy={tracks.busy}
              />
            </div>
          )}
        </div>

        {/* Stop — always present, never gated on media-status. */}
        <div className="mt-6 flex justify-center">
          <button
            type="button"
            onClick={() => endCastSession(true)}
            className="inline-flex items-center gap-2 rounded-full border border-white/25 px-5 py-2 text-sm font-semibold text-white transition-colors hover:border-white hover:bg-white hover:text-black"
          >
            <StopIcon />
            Stop casting
          </button>
        </div>
      </div>
    </div>
  );
}

/// Compact track/quality picker. A button that toggles a small popover
/// of radio-style options. Hidden entirely when there are no options.
function TrackMenu({
  label,
  options,
  busy,
}: {
  label: string;
  options: CastTrackOption[];
  busy: boolean;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("pointerdown", onDoc);
    return () => document.removeEventListener("pointerdown", onDoc);
  }, [open]);
  if (options.length === 0) return null;
  const active = options.find((o) => o.active);
  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        disabled={busy}
        className="rounded-full border border-white/20 px-3 py-1.5 text-xs font-medium text-white/85 transition-colors hover:border-white/40 hover:text-white disabled:opacity-40"
      >
        {label}
        {active ? <span className="text-white/50"> · {active.label}</span> : ""}
      </button>
      {open && (
        <div className="absolute bottom-full left-1/2 z-20 mb-2 max-h-64 w-56 -translate-x-1/2 overflow-auto rounded-lg border border-white/10 bg-black/95 p-1 shadow-2xl backdrop-blur">
          {options.map((o, i) => (
            <button
              key={`${o.label}-${i}`}
              type="button"
              role="menuitemradio"
              aria-checked={o.active}
              onClick={() => {
                o.onSelect();
                setOpen(false);
              }}
              className={`flex w-full items-center gap-2 rounded-md px-3 py-2 text-left text-sm transition-colors hover:bg-white/10 ${
                o.active ? "text-(--color-accent)" : "text-white/85"
              }`}
            >
              <span className="w-3 shrink-0">{o.active ? "✓" : ""}</span>
              <span className="truncate">{o.label}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

// --- icons (kept inline so the remote has no external icon dep) ---

function CastGlyph({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden
    >
      <path d="M2 8V6a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2h-6" />
      <path d="M2 12a8 8 0 0 1 8 8" />
      <path d="M2 16a4 4 0 0 1 4 4" />
      <circle cx="3" cy="20" r="1.2" fill="currentColor" stroke="none" />
    </svg>
  );
}
function PlayIcon() {
  return (
    <svg width="26" height="26" viewBox="0 0 24 24" fill="currentColor" aria-hidden>
      <path d="M8 5v14l11-7z" />
    </svg>
  );
}
function PauseIcon() {
  return (
    <svg width="26" height="26" viewBox="0 0 24 24" fill="currentColor" aria-hidden>
      <rect x="6" y="5" width="4" height="14" rx="1" />
      <rect x="14" y="5" width="4" height="14" rx="1" />
    </svg>
  );
}
function Replay10Icon() {
  return (
    <svg width="26" height="26" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="M11 4 6 8l5 4" />
      <path d="M6 8h7a6 6 0 1 1-6 6" />
      <text x="12" y="16" fontSize="7" fill="currentColor" stroke="none" textAnchor="middle" fontWeight="700">10</text>
    </svg>
  );
}
function Forward10Icon() {
  return (
    <svg width="26" height="26" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="m13 4 5 4-5 4" />
      <path d="M18 8h-7a6 6 0 1 0 6 6" />
      <text x="12" y="16" fontSize="7" fill="currentColor" stroke="none" textAnchor="middle" fontWeight="700">10</text>
    </svg>
  );
}
function VolumeIcon() {
  return (
    <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="M11 5 6 9H2v6h4l5 4z" />
      <path d="M15.5 8.5a5 5 0 0 1 0 7" />
      <path d="M19 5a9 9 0 0 1 0 14" />
    </svg>
  );
}
function MuteIcon() {
  return (
    <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="M11 5 6 9H2v6h4l5 4z" />
      <line x1="22" y1="9" x2="16" y2="15" />
      <line x1="16" y1="9" x2="22" y2="15" />
    </svg>
  );
}
function StopIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor" aria-hidden>
      <rect x="6" y="6" width="12" height="12" rx="2" />
    </svg>
  );
}
function ChevronDownIcon() {
  return (
    <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="m6 9 6 6 6-6" />
    </svg>
  );
}

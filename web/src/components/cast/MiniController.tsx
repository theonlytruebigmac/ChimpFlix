"use client";

/// Persistent "now casting" bar shown on every browse/detail screen
/// while a Cast session is live (the player page renders its own
/// full-frame remote instead). Tapping the body expands the full
/// controller; play/pause and stop are reachable without expanding.
/// Mirrors the Plex/YouTube mini-player pattern.

import {
  castPlayPause,
  endCastSession,
  useRemotePlayback,
} from "@/lib/cast";

export interface MiniControllerProps {
  onExpand: () => void;
}

export function MiniController({ onExpand }: MiniControllerProps) {
  const remote = useRemotePlayback();

  const title = remote.title ?? "Casting";
  const device = remote.deviceName
    ? `Playing on ${remote.deviceName}`
    : "Connected";

  return (
    <div className="pointer-events-auto fixed inset-x-0 bottom-0 z-40 px-[max(0.5rem,env(safe-area-inset-left))] pb-[max(0.5rem,env(safe-area-inset-bottom))]">
      <div className="mx-auto flex max-w-3xl items-center gap-3 rounded-xl border border-white/10 bg-black/85 p-2 shadow-2xl backdrop-blur-md">
        {/* Tap target → expand. A button so it's keyboard-reachable. */}
        <button
          type="button"
          onClick={onExpand}
          aria-label="Open cast controls"
          className="flex min-w-0 flex-1 items-center gap-3 rounded-lg p-1 text-left transition-colors hover:bg-white/5"
        >
          <div className="h-12 w-12 shrink-0 overflow-hidden rounded-md bg-white/5 ring-1 ring-white/10">
            {remote.imageUrl ? (
              // eslint-disable-next-line @next/next/no-img-element
              <img
                src={remote.imageUrl}
                alt=""
                className="h-full w-full object-cover"
              />
            ) : (
              <div className="flex h-full w-full items-center justify-center">
                <CastGlyph className="h-6 w-6 text-white/30" />
              </div>
            )}
          </div>
          <div className="min-w-0 flex-1">
            <div className="truncate text-sm font-semibold text-white">
              {title}
            </div>
            <div className="flex items-center gap-1.5 truncate text-xs text-(--color-accent)">
              <CastGlyph className="h-3.5 w-3.5 shrink-0" />
              <span className="truncate">{device}</span>
            </div>
          </div>
        </button>

        <button
          type="button"
          onClick={castPlayPause}
          disabled={!remote.canPause}
          aria-label={remote.isPaused ? "Play" : "Pause"}
          title={remote.isPaused ? "Play" : "Pause"}
          className="flex h-11 w-11 shrink-0 items-center justify-center rounded-full bg-white text-black transition-transform hover:scale-105 disabled:opacity-40"
        >
          {remote.isPaused ? <PlayIcon /> : <PauseIcon />}
        </button>

        <button
          type="button"
          onClick={() => endCastSession(true)}
          aria-label="Stop casting"
          title="Stop casting"
          className="flex h-11 w-11 shrink-0 items-center justify-center rounded-full text-white/75 transition-colors hover:bg-white/10 hover:text-white"
        >
          <StopIcon />
        </button>
      </div>
    </div>
  );
}

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
    <svg width="22" height="22" viewBox="0 0 24 24" fill="currentColor" aria-hidden>
      <path d="M8 5v14l11-7z" />
    </svg>
  );
}
function PauseIcon() {
  return (
    <svg width="22" height="22" viewBox="0 0 24 24" fill="currentColor" aria-hidden>
      <rect x="6" y="5" width="4" height="14" rx="1" />
      <rect x="14" y="5" width="4" height="14" rx="1" />
    </svg>
  );
}
function StopIcon() {
  return (
    <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor" aria-hidden>
      <rect x="6" y="6" width="12" height="12" rx="2" />
    </svg>
  );
}

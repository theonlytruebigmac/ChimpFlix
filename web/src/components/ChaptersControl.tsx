"use client";

import { useEffect, useState } from "react";
import {
  chapters as chaptersApi,
  type ChapterEntry,
} from "@/lib/chimpflix-api";

interface Props {
  mediaFileId: number;
  /// Source-time seek callback (seconds). The player's existing
  /// `seekTo` accepts source-time and handles the HLS offset.
  onSeekTo: (seconds: number) => void;
}

/// Chapter menu button + popover for the player toolbar. Renders
/// nothing when the source has no chapter metadata, so the button
/// only appears on titles where it's useful.
///
/// Thumbnails come from the `/media-files/{id}/chapters/{i}/thumb`
/// endpoint when the `generate_chapter_thumbs` scheduled task has
/// run for the file. When `thumbs_ready` is false (task hasn't run
/// or extraction failed), we render a chapter strip without
/// thumbnails — title-only is still useful navigation.
export function ChaptersControl({ mediaFileId, onSeekTo }: Props) {
  const [list, setList] = useState<ChapterEntry[]>([]);
  const [thumbsReady, setThumbsReady] = useState(false);
  const [open, setOpen] = useState(false);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    let cancelled = false;
    chaptersApi
      .list(mediaFileId)
      .then((r) => {
        if (cancelled) return;
        setList(r.chapters);
        setThumbsReady(r.thumbs_ready);
      })
      .catch(() => {
        // 404 or backend error — silently treat as no chapters.
      })
      .finally(() => {
        if (!cancelled) setLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, [mediaFileId]);

  if (!loaded || list.length === 0) return null;

  function pickChapter(c: ChapterEntry) {
    onSeekTo(c.start_ms / 1000);
    setOpen(false);
  }

  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-label="Chapters"
        aria-expanded={open}
        className="flex h-11 w-11 items-center justify-center rounded-full text-white/90 transition-colors hover:bg-white/10"
      >
        {/* Numbered-list glyph — small enough to read at the toolbar
            scale, doesn't compete with the rest of the control row */}
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden>
          <line x1="9" y1="6" x2="20" y2="6" />
          <line x1="9" y1="12" x2="20" y2="12" />
          <line x1="9" y1="18" x2="20" y2="18" />
          <circle cx="4" cy="6" r="1" fill="currentColor" />
          <circle cx="4" cy="12" r="1" fill="currentColor" />
          <circle cx="4" cy="18" r="1" fill="currentColor" />
        </svg>
      </button>
      {open && (
        <div
          role="menu"
          className="absolute bottom-full right-0 z-40 mb-2 max-h-96 w-80 overflow-y-auto rounded-md border border-white/15 bg-black/95 p-2 shadow-2xl"
        >
          <div className="mb-2 px-2 text-xs font-semibold uppercase tracking-wider text-white/45">
            Chapters
          </div>
          <ul className="space-y-1">
            {list.map((c) => (
              <li key={c.index}>
                <button
                  type="button"
                  onClick={() => pickChapter(c)}
                  className="flex w-full items-center gap-2 rounded p-1.5 text-left text-sm text-white/85 hover:bg-white/10"
                >
                  {thumbsReady && c.thumb_url ? (
                    // eslint-disable-next-line @next/next/no-img-element
                    <img
                      src={c.thumb_url}
                      alt=""
                      loading="lazy"
                      className="h-12 w-20 shrink-0 rounded bg-black object-cover"
                    />
                  ) : (
                    <div className="flex h-12 w-20 shrink-0 items-center justify-center rounded bg-white/5 text-xs text-white/40">
                      {c.index + 1}
                    </div>
                  )}
                  <div className="min-w-0 grow">
                    <div className="truncate text-xs font-medium">
                      {c.title ?? `Chapter ${c.index + 1}`}
                    </div>
                    <div className="text-[10px] tabular-nums text-white/45">
                      {formatTime(c.start_ms)}
                    </div>
                  </div>
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

function formatTime(ms: number): string {
  const total = Math.floor(ms / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h > 0) return `${h}:${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

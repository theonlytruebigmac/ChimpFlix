"use client";

import { useEffect, useMemo, useState } from "react";
import {
  items as itemsApi,
  type ManualMarkerInput,
  type MarkerRow,
  type MediaFileMarkersResponse,
} from "@/lib/chimpflix-api";

interface Props {
  mediaFileId: number;
  /// Pretty label shown in the editor header — usually the episode
  /// title (or the movie title for movies). The caller passes whatever
  /// makes sense for the surface; the editor itself doesn't care.
  fileLabel: string;
  open: boolean;
  onClose: () => void;
  /// Called after a successful save so the parent can refetch the
  /// item detail (and the player's `markers` prop with it). Optional.
  onSaved?: (response: MediaFileMarkersResponse) => void;
}

type DraftKind = ManualMarkerInput["kind"];
const KIND_OPTIONS: ReadonlyArray<{ value: DraftKind; label: string }> = [
  { value: "intro", label: "Intro" },
  { value: "credits", label: "Credits" },
  { value: "commercial", label: "Ad / commercial" },
];

interface DraftMarker {
  /// Stable client-side key for React reconciliation. NOT sent to the
  /// backend — the PUT endpoint always replaces the whole manual set,
  /// so identity is only useful within this editor session.
  key: string;
  kind: DraftKind;
  startSec: string;
  endSec: string;
  label: string;
}

/// Per-media-file marker editor. Loads the full marker list, lets the
/// operator add/remove/edit manual markers, and PUTs the replacement
/// set on save. Auto markers (from the scheduled detect_markers task)
/// are shown as read-only rows so the operator can see what the
/// heuristic produced — but only the manual rows are mutable here.
///
/// Time inputs are seconds with two decimals (e.g. 142.5). Plex-style
/// MM:SS parsing would be friendlier; we keep seconds today because
/// it round-trips losslessly and the editor is rare-use enough that
/// the operator can do the math.
export function MarkerEditor({
  mediaFileId,
  fileLabel,
  open,
  onClose,
  onSaved,
}: Props) {
  const [loaded, setLoaded] = useState<MediaFileMarkersResponse | null>(null);
  const [drafts, setDrafts] = useState<DraftMarker[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedAt, setSavedAt] = useState<number | null>(null);
  // Fetch on open. The parent unmounts us between sessions (via the
  // `open` gate at the top of the render), so this only fires once
  // per editor invocation — no need to reset loaded/error here.
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    itemsApi
      .listMarkers(mediaFileId)
      .then((r) => {
        if (cancelled) return;
        setLoaded(r);
        setDrafts(toDrafts(r.markers));
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [open, mediaFileId]);

  const autoRows = useMemo(
    () => (loaded ? loaded.markers.filter((m) => m.source === "auto") : []),
    [loaded],
  );

  const dirty = useMemo(() => {
    if (!loaded) return false;
    return JSON.stringify(drafts) !== JSON.stringify(toDrafts(loaded.markers));
  }, [loaded, drafts]);

  function addDraft() {
    setDrafts((d) => [
      ...d,
      {
        key: `new-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
        kind: "intro",
        startSec: "0.00",
        endSec: "30.00",
        label: "",
      },
    ]);
  }

  function update(key: string, patch: Partial<DraftMarker>) {
    setDrafts((d) => d.map((m) => (m.key === key ? { ...m, ...patch } : m)));
  }

  function remove(key: string) {
    setDrafts((d) => d.filter((m) => m.key !== key));
  }

  async function save() {
    setBusy(true);
    setError(null);
    setSavedAt(null);
    try {
      const payload: ManualMarkerInput[] = drafts.map((d) => {
        const start = Math.max(0, Math.round(parseFloat(d.startSec) * 1000));
        const end = Math.max(start + 1, Math.round(parseFloat(d.endSec) * 1000));
        return {
          kind: d.kind,
          start_ms: start,
          end_ms: end,
          label: d.label.trim() || null,
        };
      });
      const next = await itemsApi.replaceManualMarkers(mediaFileId, payload);
      setLoaded(next);
      setDrafts(toDrafts(next.markers));
      setSavedAt(Date.now());
      onSaved?.(next);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-80 flex items-start justify-center overflow-y-auto bg-black/70 p-4 backdrop-blur-sm sm:p-8"
      role="dialog"
      aria-modal="true"
      aria-label="Edit markers"
      onClick={onClose}
    >
      <div
        className="relative w-full max-w-2xl rounded-lg border border-white/10 bg-(--color-surface) shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="flex items-start justify-between gap-3 border-b border-white/10 px-5 py-4">
          <div className="min-w-0 flex-1">
            <h2 className="m-0 truncate text-base font-semibold">
              Edit markers
            </h2>
            <p className="m-0 mt-0.5 truncate text-xs text-white/55">
              {fileLabel}
              {loaded?.duration_ms != null && (
                <span> · {formatSeconds(loaded.duration_ms / 1000)}</span>
              )}
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close editor"
            className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-white/70 hover:bg-white/5 hover:text-white"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <line x1="18" y1="6" x2="6" y2="18" />
              <line x1="6" y1="6" x2="18" y2="18" />
            </svg>
          </button>
        </header>

        <div className="px-5 py-4">
          {error && (
            <div className="mb-3 rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
              {error}
            </div>
          )}
          {loaded == null && !error ? (
            <p className="text-sm text-white/55">Loading…</p>
          ) : (
            <>
              {autoRows.length > 0 && (
                <section className="mb-4">
                  <h3 className="mb-1 text-[11px] font-semibold uppercase tracking-[0.06em] text-white/45">
                    Auto-detected (read-only)
                  </h3>
                  <p className="mb-2 text-[11.5px] text-white/45">
                    Re-runs of the scheduled marker-detection task replace
                    these. Add a manual marker below to override any of
                    them — manual rows always win on the player.
                  </p>
                  <ul className="divide-y divide-white/6 rounded-md border border-white/10 bg-white/2">
                    {autoRows.map((m) => (
                      <li
                        key={m.id}
                        className="grid grid-cols-[60px_1fr_auto] items-center gap-3 px-3 py-2 text-[12.5px]"
                      >
                        <span className="text-white/55">
                          {labelForKind(m.kind)}
                        </span>
                        <span className="tabular-nums text-white/65">
                          {formatSeconds(m.start_ms / 1000)} →{" "}
                          {formatSeconds(m.end_ms / 1000)}
                        </span>
                        <span className="text-[11px] text-white/40">auto</span>
                      </li>
                    ))}
                  </ul>
                </section>
              )}

              <section>
                <div className="mb-1 flex items-baseline justify-between">
                  <h3 className="text-[11px] font-semibold uppercase tracking-[0.06em] text-white/45">
                    Manual markers ({drafts.length})
                  </h3>
                  <button
                    type="button"
                    onClick={addDraft}
                    className="rounded border border-white/15 px-2 py-1 text-xs text-white/85 hover:bg-white/5"
                  >
                    + Add marker
                  </button>
                </div>
                {drafts.length === 0 ? (
                  <p className="mt-2 text-[12.5px] text-white/55">
                    No manual markers. Add one to override an auto-detected
                    range or to mark a section the detector missed.
                  </p>
                ) : (
                  <ul className="mt-2 space-y-2">
                    {drafts.map((d) => (
                      <li
                        key={d.key}
                        className="rounded-md border border-white/10 bg-white/2 p-3"
                      >
                        <div className="grid grid-cols-1 gap-2 sm:grid-cols-[120px_1fr_1fr_auto]">
                          <select
                            value={d.kind}
                            onChange={(e) =>
                              update(d.key, {
                                kind: e.target.value as DraftKind,
                              })
                            }
                            className="rounded-md border border-white/10 bg-black/30 px-2 py-1.5 text-sm outline-none focus:border-white/30"
                          >
                            {KIND_OPTIONS.map((opt) => (
                              <option key={opt.value} value={opt.value}>
                                {opt.label}
                              </option>
                            ))}
                          </select>
                          <label className="text-xs text-white/55">
                            <span className="mb-0.5 block">Start (sec)</span>
                            <input
                              type="number"
                              step="0.01"
                              min="0"
                              value={d.startSec}
                              onChange={(e) =>
                                update(d.key, { startSec: e.target.value })
                              }
                              className="w-full rounded-md border border-white/10 bg-black/30 px-2 py-1.5 font-mono text-sm text-white tabular-nums outline-none focus:border-white/30"
                            />
                          </label>
                          <label className="text-xs text-white/55">
                            <span className="mb-0.5 block">End (sec)</span>
                            <input
                              type="number"
                              step="0.01"
                              min="0"
                              value={d.endSec}
                              onChange={(e) =>
                                update(d.key, { endSec: e.target.value })
                              }
                              className="w-full rounded-md border border-white/10 bg-black/30 px-2 py-1.5 font-mono text-sm text-white tabular-nums outline-none focus:border-white/30"
                            />
                          </label>
                          <button
                            type="button"
                            onClick={() => remove(d.key)}
                            aria-label="Remove this marker"
                            className="self-end rounded border border-red-500/30 px-2 py-1.5 text-xs text-red-300 hover:bg-red-500/10"
                          >
                            Remove
                          </button>
                        </div>
                        <input
                          type="text"
                          value={d.label}
                          onChange={(e) =>
                            update(d.key, { label: e.target.value })
                          }
                          placeholder="Optional label (e.g. “Long OP”)"
                          maxLength={120}
                          className="mt-2 w-full rounded-md border border-white/10 bg-black/30 px-2 py-1.5 text-sm outline-none focus:border-white/30"
                        />
                      </li>
                    ))}
                  </ul>
                )}
              </section>
            </>
          )}
        </div>

        <footer className="flex items-center justify-between gap-3 border-t border-white/10 px-5 py-3">
          <span className="text-xs text-white/50">
            {savedAt && !dirty
              ? "Saved."
              : dirty
                ? "Unsaved changes"
                : "No changes"}
          </span>
          <div className="flex gap-2">
            <button
              type="button"
              onClick={onClose}
              className="rounded-md border border-transparent px-3 py-1.5 text-sm text-white/70 hover:bg-white/5"
            >
              Close
            </button>
            <button
              type="button"
              onClick={save}
              disabled={!dirty || busy || loaded == null}
              className="rounded-md border border-accent bg-accent px-3 py-1.5 text-sm font-medium text-white hover:bg-accent-hover disabled:cursor-not-allowed disabled:opacity-50"
            >
              {busy ? "Saving…" : "Save manual markers"}
            </button>
          </div>
        </footer>
      </div>
    </div>
  );
}

function toDrafts(markers: MarkerRow[]): DraftMarker[] {
  return markers
    .filter((m) => m.source === "manual")
    .map((m) => ({
      key: `db-${m.id}`,
      kind: (m.kind as DraftKind) ?? "intro",
      startSec: (m.start_ms / 1000).toFixed(2),
      endSec: (m.end_ms / 1000).toFixed(2),
      label: m.label ?? "",
    }));
}

function labelForKind(kind: string): string {
  if (kind === "intro") return "Intro";
  if (kind === "credits") return "Credits";
  if (kind === "commercial") return "Ad";
  return kind;
}

function formatSeconds(s: number): string {
  if (!Number.isFinite(s) || s < 0) return "—";
  const total = Math.floor(s);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const sec = total % 60;
  if (h > 0) {
    return `${h}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`;
  }
  return `${m}:${String(sec).padStart(2, "0")}`;
}


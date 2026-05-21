"use client";

import { useEffect, useRef, useState } from "react";

/// Sticky save bar that appears at the bottom of every settings form
/// once the local state diverges from the server-side baseline. The
/// `dirtyCount` is reported by the parent so the bar can show "3
/// unsaved changes" without owning any of the per-field comparison
/// logic. `summary` is a short hint (e.g. the names of the changed
/// fields) that hints what the operator is about to save.
///
/// A 2.5s "Saved." flash is rendered after `onSave` resolves; if the
/// component unmounts mid-flash, the timer is cleared so we don't
/// setState against a torn-down node.
export function SaveBar({
  dirtyCount,
  summary,
  onSave,
  onDiscard,
  saveLabel = "Save changes",
  disabled,
}: {
  dirtyCount: number;
  summary?: string;
  onSave: () => Promise<void> | void;
  onDiscard?: () => void;
  saveLabel?: string;
  disabled?: boolean;
}) {
  const [busy, setBusy] = useState(false);
  const [savedFlash, setSavedFlash] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const flashTimerRef = useRef<number | null>(null);
  useEffect(() => {
    return () => {
      if (flashTimerRef.current !== null) {
        window.clearTimeout(flashTimerRef.current);
        flashTimerRef.current = null;
      }
    };
  }, []);

  async function handleSave() {
    if (busy || dirtyCount === 0) return;
    setBusy(true);
    setError(null);
    try {
      await onSave();
      setSavedFlash(true);
      if (flashTimerRef.current !== null) {
        window.clearTimeout(flashTimerRef.current);
      }
      flashTimerRef.current = window.setTimeout(() => {
        flashTimerRef.current = null;
        setSavedFlash(false);
      }, 2500);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  const dirty = dirtyCount > 0;
  if (!dirty && !savedFlash && !error) return null;

  return (
    <div
      className="sticky bottom-0 z-10 mt-6 -mx-5 flex flex-wrap items-center justify-between gap-3 border-t border-white/15 bg-white/4 px-5 py-3 backdrop-blur md:-mx-7 md:px-7"
      role="region"
      aria-label="Save changes"
    >
      <div className="min-w-0 text-[13px]">
        {error ? (
          <span className="text-red-300">
            <strong>Save failed:</strong> {error}
          </span>
        ) : savedFlash && !dirty ? (
          <span className="flex items-center gap-2 text-emerald-300">
            <span aria-hidden className="inline-block h-2 w-2 rounded-full bg-emerald-400" />
            Saved.
          </span>
        ) : (
          <span className="flex items-center gap-2 text-amber-300">
            <span aria-hidden className="inline-block h-2 w-2 rounded-full bg-amber-400" />
            <span>
              <strong>
                {dirtyCount} unsaved {dirtyCount === 1 ? "change" : "changes"}
              </strong>
              {summary && (
                <span className="text-white/55"> · {summary}</span>
              )}
            </span>
          </span>
        )}
      </div>
      <div className="flex shrink-0 gap-2">
        {onDiscard && (
          <button
            type="button"
            onClick={onDiscard}
            disabled={busy || !dirty}
            className="rounded-md border border-transparent px-3 py-1.5 text-[13px] text-white/70 hover:bg-white/5 disabled:opacity-40"
          >
            Discard
          </button>
        )}
        <button
          type="button"
          onClick={handleSave}
          disabled={busy || disabled || !dirty}
          className="rounded-md border border-accent bg-accent px-3 py-1.5 text-[13px] font-medium text-white hover:bg-accent-hover disabled:cursor-not-allowed disabled:opacity-50"
        >
          {busy ? "Saving…" : saveLabel}
        </button>
      </div>
    </div>
  );
}

"use client";

import { useEffect, useRef, useState } from "react";
import {
  admin as adminApi,
  preroll as prerollApi,
  type PrerollStatus,
} from "@/lib/chimpflix-api";

interface Props {
  initialStatus: PrerollStatus;
  initialEnabled: boolean;
  initialVolume: number;
}

/// Tiny admin page: file picker for upload, toggle for enable, clear
/// button to drop the current file. One pre-roll at a time — see
/// migration phase42 for the rationale.
export function AdminPrerollClient({
  initialStatus,
  initialEnabled,
  initialVolume,
}: Props) {
  const [status, setStatus] = useState(initialStatus);
  const [enabled, setEnabled] = useState(initialEnabled);
  const [volume, setVolume] = useState(initialVolume);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [savedAt, setSavedAt] = useState<number | null>(null);
  const previewRef = useRef<HTMLVideoElement | null>(null);
  // Set false on unmount so the debounced volume PATCH below doesn't
  // call setState against a torn-down component.
  const aliveRef = useRef(true);

  async function upload(file: File) {
    setBusy("upload");
    setError(null);
    try {
      const next = await prerollApi.upload(file);
      setStatus(next);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function clear() {
    if (!window.confirm("Remove the current pre-roll? Disables pre-roll playback.")) return;
    setBusy("clear");
    setError(null);
    try {
      await prerollApi.clear();
      setStatus({
        enabled: false,
        configured: false,
        url: null,
        size_bytes: null,
        volume,
      });
      setEnabled(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function toggle(next: boolean) {
    setBusy("toggle");
    setError(null);
    try {
      await adminApi.settings.patch({ preroll_enabled: next });
      setEnabled(next);
      setStatus((s) => ({ ...s, enabled: next }));
      setSavedAt(Date.now());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  // Live-update the preview video as the operator drags the slider so
  // they can hear the chosen level immediately. The persisted value
  // saves on slider release (onChange fires per-step; we coalesce via
  // a debounce ref instead of writing on every pixel of drag).
  useEffect(() => {
    if (previewRef.current) previewRef.current.volume = volume / 100;
  }, [volume]);

  const saveVolumeRef = useRef<number | null>(null);
  useEffect(() => {
    return () => {
      aliveRef.current = false;
      if (saveVolumeRef.current) {
        window.clearTimeout(saveVolumeRef.current);
        saveVolumeRef.current = null;
      }
    };
  }, []);
  function setVolumeAndPersist(next: number) {
    setVolume(next);
    if (saveVolumeRef.current) window.clearTimeout(saveVolumeRef.current);
    saveVolumeRef.current = window.setTimeout(async () => {
      saveVolumeRef.current = null;
      try {
        await adminApi.settings.patch({ preroll_volume: next });
        if (aliveRef.current) setSavedAt(Date.now());
      } catch (e) {
        if (aliveRef.current) {
          setError(e instanceof Error ? e.message : String(e));
        }
      }
    }, 350);
  }

  return (
    <div className="space-y-6">
      {error && (
        <div className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {error}
        </div>
      )}

      <section className="rounded-lg border border-white/10 bg-white/2 p-6 space-y-4">
        <h2 className="text-base font-semibold">Current pre-roll</h2>
        {status.configured ? (
          <div className="space-y-3">
            <div className="text-sm text-white/75">
              File configured ({formatBytes(status.size_bytes ?? 0)}).{" "}
              {enabled ? (
                <span className="text-green-300">Active — plays before each session.</span>
              ) : (
                <span className="text-amber-300">Disabled — toggle to enable.</span>
              )}
            </div>
            {status.url && (
              <video
                ref={previewRef}
                src={status.url}
                controls
                preload="metadata"
                className="max-h-72 w-full rounded border border-white/10 bg-black"
              />
            )}
            <div className="flex flex-wrap gap-2">
              <label className="rounded-md border border-white/15 px-3 py-1.5 text-sm cursor-pointer hover:bg-white/5">
                <input
                  type="file"
                  accept="video/mp4,video/webm,video/x-matroska,.mkv"
                  className="hidden"
                  onChange={(e) => {
                    const f = e.target.files?.[0];
                    e.target.value = "";
                    if (f) upload(f);
                  }}
                  disabled={busy === "upload"}
                />
                {busy === "upload" ? "Uploading…" : "Replace…"}
              </label>
              <button
                type="button"
                onClick={clear}
                disabled={busy === "clear"}
                className="rounded-md border border-red-500/40 px-3 py-1.5 text-sm text-red-300 hover:bg-red-500/10 disabled:opacity-50"
              >
                {busy === "clear" ? "Removing…" : "Remove"}
              </button>
            </div>
          </div>
        ) : (
          <div>
            <div className="mb-3 rounded border border-dashed border-white/15 bg-white/2 px-4 py-6 text-center text-sm text-white/55">
              No pre-roll uploaded.
            </div>
            <label className="rounded-md bg-red-500 px-4 py-2 text-sm font-semibold text-white hover:bg-red-600 cursor-pointer">
              <input
                type="file"
                accept="video/mp4,video/webm,video/x-matroska,.mkv"
                className="hidden"
                onChange={(e) => {
                  const f = e.target.files?.[0];
                  e.target.value = "";
                  if (f) upload(f);
                }}
                disabled={busy === "upload"}
              />
              {busy === "upload" ? "Uploading…" : "Upload pre-roll…"}
            </label>
            <p className="mt-2 text-xs text-white/45">
              MP4 / WebM / MKV, up to 200 MiB. Aim for &lt; 30s — every viewer
              sits through this on every session.
            </p>
          </div>
        )}
      </section>

      <section className="rounded-lg border border-white/10 bg-white/2 p-6 space-y-4">
        <h2 className="text-base font-semibold">Playback</h2>
        <label className="flex items-start gap-3 text-sm">
          <input
            type="checkbox"
            checked={enabled}
            onChange={(e) => toggle(e.target.checked)}
            disabled={!status.configured || busy === "toggle"}
            className="mt-1"
          />
          <div>
            <div className="font-medium">Play pre-roll before each session</div>
            <p className="mt-1 text-xs text-white/55">
              When on, the player runs the pre-roll then transitions to the
              main content. The viewer can skip after the first frame.
              Disabled automatically when no pre-roll is uploaded. Skipped
              when the viewer is resuming a partially-watched item.
            </p>
          </div>
        </label>

        <div className="border-t border-white/10 pt-4">
          <div className="flex items-baseline justify-between">
            <div className="text-sm font-medium">Volume</div>
            <div className="text-xs tabular-nums text-white/55">{volume}%</div>
          </div>
          <p className="mt-1 text-xs text-white/55">
            Output level applied to the pre-roll when the player runs it.
            Useful for taming stings mastered at theatre levels so they
            don&apos;t blow out speakers before the show starts.
          </p>
          <input
            type="range"
            min={0}
            max={100}
            step={1}
            value={volume}
            onChange={(e) => setVolumeAndPersist(Number(e.target.value))}
            className="mt-3 w-full"
            aria-label="Pre-roll volume"
          />
          <div className="mt-1 flex justify-between text-[10px] uppercase tracking-wide text-white/40">
            <span>Mute</span>
            <span>Source level</span>
          </div>
        </div>

        {savedAt && (
          <div className="text-xs text-white/50">Saved.</div>
        )}
      </section>
    </div>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GiB`;
}

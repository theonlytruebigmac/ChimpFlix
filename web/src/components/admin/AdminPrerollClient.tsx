"use client";

import { useState } from "react";
import {
  admin as adminApi,
  preroll as prerollApi,
  type PrerollStatus,
} from "@/lib/chimpflix-api";

interface Props {
  initialStatus: PrerollStatus;
  initialEnabled: boolean;
}

/// Tiny admin page: file picker for upload, toggle for enable, clear
/// button to drop the current file. One pre-roll at a time — see
/// migration phase42 for the rationale.
export function AdminPrerollClient({ initialStatus, initialEnabled }: Props) {
  const [status, setStatus] = useState(initialStatus);
  const [enabled, setEnabled] = useState(initialEnabled);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [savedAt, setSavedAt] = useState<number | null>(null);

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
      setStatus({ enabled: false, configured: false, url: null, size_bytes: null });
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
              main content. The viewer can skip after 5 seconds. Disabled
              automatically when no pre-roll is uploaded.
            </p>
          </div>
        </label>
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

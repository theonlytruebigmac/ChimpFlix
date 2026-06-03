"use client";

import { useEffect, useRef, useState } from "react";
import {
  admin as adminApi,
  preroll as prerollApi,
  type PrerollStatus,
} from "@/lib/chimpflix-api";
import { ConfirmDialog } from "../ConfirmDialog";

interface Props {
  initialStatus: PrerollStatus;
  initialEnabled: boolean;
  initialVolume: number;
}

/// Pre-roll bumper tab: file picker for upload/replace, toggle for enable,
/// remove button to drop the current file, and a volume slider. One
/// pre-roll at a time — see migration phase42 for the rationale.
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
  const [askClear, setAskClear] = useState(false);
  const previewRef = useRef<HTMLVideoElement | null>(null);
  // Set false on unmount so the debounced volume PATCH below doesn't
  // call setState against a torn-down component.
  const aliveRef = useRef(true);

  async function upload(file: File) {
    setBusy("upload");
    setError(null);
    try {
      const next = await prerollApi.upload(file);
      if (aliveRef.current) setStatus(next);
    } catch (e) {
      if (aliveRef.current) setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (aliveRef.current) setBusy(null);
    }
  }

  async function clear() {
    setAskClear(false);
    setBusy("clear");
    setError(null);
    try {
      await prerollApi.clear();
      if (aliveRef.current) {
        setStatus({
          enabled: false,
          configured: false,
          url: null,
          size_bytes: null,
          volume,
        });
        setEnabled(false);
      }
    } catch (e) {
      if (aliveRef.current) setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (aliveRef.current) setBusy(null);
    }
  }

  async function toggle(next: boolean) {
    setBusy("toggle");
    setError(null);
    try {
      await adminApi.settings.patch({ preroll_enabled: next });
      if (aliveRef.current) {
        setEnabled(next);
        setStatus((s) => ({ ...s, enabled: next }));
        setSavedAt(Date.now());
      }
    } catch (e) {
      if (aliveRef.current) setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (aliveRef.current) setBusy(null);
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
    <div>
      {error && (
        <div role="alert" aria-live="assertive" className="cf-banner cf-err">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 8v4M12 16v.5" />
          </svg>
          <div>{error}</div>
        </div>
      )}

      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Pre-roll bumper</div>
            <div className="cf-sub">
              A short clip that plays before every title.
            </div>
          </div>
          <div className="cf-head-aside">
            {status.configured ? (
              <span className="cf-pill cf-ok">
                <span className="cf-dot" />
                Loaded
              </span>
            ) : (
              <span className="cf-pill cf-warn">
                <span className="cf-dot" />
                No file
              </span>
            )}
          </div>
        </div>
        <div className="cf-card-body">
          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Enable pre-roll</div>
              <div className="cf-row-help">
                When on, the player runs the pre-roll then transitions to the
                main content. The viewer can skip after the first frame.
                Disabled automatically when no file is uploaded, and skipped
                when resuming a partially-watched item.
              </div>
            </div>
            <div className="cf-row-control">
              <button
                type="button"
                role="switch"
                aria-checked={enabled}
                aria-label="Enable pre-roll"
                disabled={!status.configured || busy === "toggle"}
                className={"cf-switch" + (enabled ? " cf-on" : "")}
                onClick={() => toggle(!enabled)}
              />
            </div>
          </div>

          <div className="cf-row cf-col">
            <div className="cf-row-main cf-flex cf-between" style={{ width: "100%" }}>
              <div className="cf-row-label">
                Volume{" "}
                <span className="cf-faint" style={{ fontWeight: 400 }}>
                  — {volume}%
                </span>
              </div>
              {savedAt && (
                <span
                  role="status"
                  aria-live="polite"
                  className="cf-pill cf-ok"
                  style={{ padding: "2px 8px" }}
                >
                  Saved
                </span>
              )}
            </div>
            <div className="cf-row-control" style={{ width: "100%" }}>
              <input
                type="range"
                min={0}
                max={100}
                step={1}
                className="cf-range"
                value={volume}
                onChange={(e) => setVolumeAndPersist(Number(e.target.value))}
                aria-label="Pre-roll volume"
              />
            </div>
            <div
              className="cf-flex cf-between"
              style={{
                width: "100%",
                fontSize: 10,
                textTransform: "uppercase",
                letterSpacing: "0.04em",
                color: "var(--faint)",
              }}
            >
              <span>Mute</span>
              <span>Source level</span>
            </div>
          </div>

          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Current file</div>
              <div className="cf-row-help">
                {status.configured ? (
                  <>
                    File configured ({formatBytes(status.size_bytes ?? 0)}).{" "}
                    {enabled ? (
                      <span style={{ color: "var(--ok)" }}>
                        Active — plays before each session.
                      </span>
                    ) : (
                      <span style={{ color: "var(--warn)" }}>
                        Disabled — toggle to enable.
                      </span>
                    )}
                  </>
                ) : (
                  "No pre-roll uploaded. MP4 / WebM / MKV, up to 200 MiB. Aim for under 30s — every viewer sits through this on every session."
                )}
              </div>
            </div>
            <div className="cf-row-control">
              <label
                className={
                  "cf-btn cf-sm" + (status.configured ? "" : " cf-primary")
                }
                style={{ cursor: busy === "upload" ? "default" : "pointer" }}
              >
                <input
                  type="file"
                  accept="video/mp4,video/webm,video/x-matroska,.mkv"
                  style={{ display: "none" }}
                  onChange={(e) => {
                    const f = e.target.files?.[0];
                    e.target.value = "";
                    if (f) upload(f);
                  }}
                  disabled={busy === "upload"}
                />
                {busy === "upload"
                  ? "Uploading…"
                  : status.configured
                    ? "Replace"
                    : "Upload pre-roll"}
              </label>
              {status.configured && (
                <button
                  type="button"
                  className="cf-btn cf-ghost cf-sm cf-danger"
                  onClick={() => setAskClear(true)}
                  disabled={busy === "clear"}
                >
                  {busy === "clear" ? "Removing…" : "Remove"}
                </button>
              )}
            </div>
          </div>

          {status.configured && status.url && (
            <div className="cf-row">
              <div className="cf-row-main" style={{ width: "100%" }}>
                <video
                  ref={previewRef}
                  src={status.url}
                  controls
                  preload="metadata"
                  style={{
                    maxHeight: 288,
                    width: "100%",
                    borderRadius: "var(--r)",
                    border: "1px solid var(--line)",
                    background: "#000",
                  }}
                />
              </div>
            </div>
          )}
        </div>
      </div>

      {askClear && (
        <ConfirmDialog
          title="Remove the current pre-roll?"
          body="The uploaded file will be deleted and pre-roll playback will be disabled until a new file is uploaded."
          confirmLabel="Remove"
          destructive
          busy={busy === "clear"}
          onConfirm={() => void clear()}
          onCancel={() => setAskClear(false)}
        />
      )}
    </div>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  if (bytes < 1024 * 1024 * 1024)
    return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GiB`;
}

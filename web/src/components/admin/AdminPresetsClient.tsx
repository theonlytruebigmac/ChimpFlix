"use client";

import { useState } from "react";
import {
  admin as adminApi,
  type TranscoderPreset,
} from "@/lib/chimpflix-api";
import { ConfirmDialog } from "../ConfirmDialog";

interface Props {
  presets: TranscoderPreset[];
}

/// Presets tab: the quality-ladder table offered to clients and used by
/// optimize jobs. Was the "Quality presets" section of the transcoder
/// client; split out so it can be its own tab matching the mockup.
export function AdminPresetsClient({ presets }: Props) {
  const [allPresets, setAllPresets] = useState(presets);
  const [showAdd, setShowAdd] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function refreshPresets() {
    try {
      const r = await adminApi.transcoder.listPresets();
      setAllPresets(r.presets);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
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

      <div className="cf-flex cf-between cf-wrap cf-gap12" style={{ marginBottom: 14 }}>
        <div className="cf-muted" style={{ fontSize: 13 }}>
          Quality ladders offered to clients and used by optimize jobs.
        </div>
        <button
          type="button"
          className="cf-btn cf-primary cf-sm"
          onClick={() => setShowAdd((v) => !v)}
        >
          {showAdd ? "Cancel" : "New preset"}
        </button>
      </div>

      {showAdd && (
        <NewPresetForm
          onCreated={async () => {
            setShowAdd(false);
            await refreshPresets();
          }}
          onError={setError}
        />
      )}

      <div className="cf-card">
        <table className="cf-table">
          <thead>
            <tr>
              <th>Preset</th>
              <th>Max height</th>
              <th>Max video kbps</th>
              <th>Audio</th>
              <th>Enabled</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {allPresets.map((p) => (
              <PresetRow
                key={p.id}
                preset={p}
                onChanged={refreshPresets}
                onError={setError}
              />
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function PresetRow({
  preset,
  onChanged,
  onError,
}: {
  preset: TranscoderPreset;
  onChanged: () => Promise<void>;
  onError: (msg: string | null) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [askDelete, setAskDelete] = useState(false);

  async function toggle() {
    setBusy(true);
    onError(null);
    try {
      await adminApi.transcoder.updatePreset(preset.id, {
        enabled: !preset.enabled,
      });
      await onChanged();
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    setAskDelete(false);
    setBusy(true);
    onError(null);
    try {
      await adminApi.transcoder.deletePreset(preset.id);
      await onChanged();
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <tr>
        <td>
          <b>{preset.name}</b>
        </td>
        <td className="cf-mono">
          {preset.max_height === 0 ? "—" : preset.max_height}
        </td>
        <td className="cf-mono">
          {preset.max_video_bitrate_kbps === 0
            ? "—"
            : preset.max_video_bitrate_kbps}
        </td>
        <td className="cf-muted">
          {preset.audio_codec} @ {preset.audio_bitrate_kbps}k
        </td>
        <td>
          <button
            type="button"
            disabled={busy}
            onClick={toggle}
            className={
              "cf-pill " + (preset.enabled ? "cf-ok" : "")
            }
            style={{ cursor: "pointer", padding: "2px 9px" }}
          >
            {preset.enabled && <span className="cf-dot" />}
            {preset.enabled ? "Enabled" : "Disabled"}
          </button>
        </td>
        <td className="cf-num">
          <button
            type="button"
            disabled={busy}
            onClick={() => setAskDelete(true)}
            className="cf-btn cf-ghost cf-tiny cf-danger"
          >
            Delete
          </button>
        </td>
      </tr>
      {askDelete && (
        <ConfirmDialog
          title={`Delete preset "${preset.name}"?`}
          body="Active sessions using this preset keep running. New sessions will fall back to the default."
          confirmLabel="Delete"
          destructive
          busy={busy}
          onConfirm={() => void remove()}
          onCancel={() => setAskDelete(false)}
        />
      )}
    </>
  );
}

function NewPresetForm({
  onCreated,
  onError,
}: {
  onCreated: () => Promise<void>;
  onError: (msg: string | null) => void;
}) {
  const [name, setName] = useState("");
  const [maxHeight, setMaxHeight] = useState(720);
  const [maxBitrate, setMaxBitrate] = useState(4000);
  const [busy, setBusy] = useState(false);

  async function submit() {
    setBusy(true);
    onError(null);
    try {
      await adminApi.transcoder.createPreset({
        name: name.trim(),
        max_video_bitrate_kbps: maxBitrate,
        max_height: maxHeight,
      });
      setName("");
      await onCreated();
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="cf-card">
      <div className="cf-card-body cf-pad">
        <div className="cf-grid cf-c4">
          <div className="cf-field" style={{ marginBottom: 0 }}>
            <label className="cf-field-label">Name</label>
            <input
              type="text"
              className="cf-input"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Mobile 360p"
            />
          </div>
          <div className="cf-field" style={{ marginBottom: 0 }}>
            <label className="cf-field-label">Max height (px)</label>
            <input
              type="number"
              className="cf-input"
              value={maxHeight}
              min={0}
              max={4320}
              onChange={(e) => setMaxHeight(Number(e.target.value))}
            />
          </div>
          <div className="cf-field" style={{ marginBottom: 0 }}>
            <label className="cf-field-label">Max bitrate (kbps)</label>
            <input
              type="number"
              className="cf-input"
              value={maxBitrate}
              min={0}
              max={200_000}
              onChange={(e) => setMaxBitrate(Number(e.target.value))}
            />
          </div>
          <div className="cf-flex" style={{ alignItems: "flex-end" }}>
            <button
              type="button"
              disabled={busy || !name.trim()}
              onClick={submit}
              className="cf-btn cf-primary"
              style={{ width: "100%", justifyContent: "center" }}
            >
              {busy ? "Creating…" : "Create"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

"use client";

import { useEffect, useState } from "react";
import {
  admin as adminApi,
  type OptimizedVersion,
  type TranscoderPreset,
} from "@/lib/chimpflix-api";
import { ConfirmDialog } from "../ConfirmDialog";

interface Props {
  initial: OptimizedVersion[];
  presets: TranscoderPreset[];
}

export function AdminOptimizedClient({ initial, presets }: Props) {
  const [versions, setVersions] = useState(initial);
  const [showAdd, setShowAdd] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [askDeleteId, setAskDeleteId] = useState<number | null>(null);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [askCancelId, setAskCancelId] = useState<number | null>(null);
  const [cancelBusy, setCancelBusy] = useState(false);

  async function refresh() {
    try {
      const r = await adminApi.versions.list();
      setVersions(r.versions);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  // While any row is still in flight (queued/running), poll so the
  // progress bar advances and a cancel/finish reflects without a manual
  // reload. Stops polling once everything is in a terminal state.
  const hasActive = versions.some(
    (v) => v.status === "queued" || v.status === "running",
  );
  useEffect(() => {
    if (!hasActive) return;
    const t = setInterval(() => void refresh(), 3000);
    return () => clearInterval(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hasActive]);

  function remove(id: number) {
    setAskDeleteId(id);
  }

  async function confirmDelete() {
    if (askDeleteId == null) return;
    setDeleteBusy(true);
    try {
      await adminApi.versions.delete(askDeleteId);
      await refresh();
      setAskDeleteId(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setDeleteBusy(false);
    }
  }

  async function confirmCancel() {
    if (askCancelId == null) return;
    setCancelBusy(true);
    try {
      await adminApi.versions.cancel(askCancelId);
      await refresh();
      setAskCancelId(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setCancelBusy(false);
    }
  }

  return (
    <div>
      {error && (
        <div className="cf-banner cf-err">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 8v4M12 16v.5" />
          </svg>
          <div>{error}</div>
        </div>
      )}

      <div
        className="cf-flex cf-between cf-wrap cf-gap12"
        style={{ marginBottom: 14 }}
      >
        <div className="cf-muted" style={{ fontSize: 13 }}>
          Pre-transcoded, direct-playable copies. Queue a{" "}
          <b style={{ color: "#fff" }}>file × preset</b> pair; the{" "}
          <span className="cf-mono">optimize_versions</span> task produces it.
        </div>
        <button
          type="button"
          onClick={() => setShowAdd((v) => !v)}
          className="cf-btn cf-primary cf-sm"
        >
          {showAdd ? "Cancel" : "Queue version"}
        </button>
      </div>

      {showAdd && (
        <NewOptimizedForm
          presets={presets}
          onQueued={async () => {
            setShowAdd(false);
            await refresh();
          }}
          onError={setError}
        />
      )}

      {versions.length === 0 ? (
        <div className="cf-card">
          <div
            className="cf-card-body cf-pad cf-faint cf-center"
            style={{ fontSize: 13 }}
          >
            No optimized versions queued.
          </div>
        </div>
      ) : (
        <div className="cf-card" style={{ marginBottom: 0 }}>
          <table className="cf-table">
            <thead>
              <tr>
                <th>Source file</th>
                <th>Preset</th>
                <th>Status</th>
                <th>Size</th>
                <th>Output</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {versions.map((v) => {
                const preset = presets.find((p) => p.id === v.preset_id);
                return (
                  <tr key={v.id}>
                    <td className="cf-mono">file #{v.source_file_id}</td>
                    <td className="cf-muted">
                      {preset?.name ?? `preset #${v.preset_id}`}
                    </td>
                    <td>
                      <StatusBadge status={v.status} />
                      {v.status === "running" && (
                        <ProgressBar permille={v.progress_permille} />
                      )}
                      {v.error && (
                        <div
                          className="cf-mono"
                          style={{
                            marginTop: 2,
                            maxWidth: 300,
                            overflow: "hidden",
                            textOverflow: "ellipsis",
                            whiteSpace: "nowrap",
                            fontSize: 10,
                            color: "var(--err)",
                          }}
                          title={v.error}
                        >
                          {v.error}
                        </div>
                      )}
                    </td>
                    <td className="cf-muted">
                      {v.output_size_bytes != null
                        ? formatBytes(v.output_size_bytes)
                        : "—"}
                    </td>
                    <td className="cf-mono cf-faint">
                      <div
                        style={{
                          maxWidth: 280,
                          overflow: "hidden",
                          textOverflow: "ellipsis",
                          whiteSpace: "nowrap",
                        }}
                        title={v.output_path}
                      >
                        {v.output_path || "—"}
                      </div>
                    </td>
                    <td className="cf-num">
                      {v.status === "queued" || v.status === "running" ? (
                        <button
                          type="button"
                          onClick={() => setAskCancelId(v.id)}
                          className="cf-btn cf-ghost cf-tiny cf-danger"
                        >
                          Cancel
                        </button>
                      ) : (
                        <button
                          type="button"
                          onClick={() => remove(v.id)}
                          className="cf-btn cf-ghost cf-tiny cf-danger"
                        >
                          Delete
                        </button>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
      {askDeleteId != null && (
        <ConfirmDialog
          title="Delete this optimized version?"
          body="The transcoded file on disk and the DB row will both be removed. The source file is untouched."
          confirmLabel="Delete"
          destructive
          busy={deleteBusy}
          onConfirm={() => void confirmDelete()}
          onCancel={() => setAskDeleteId(null)}
        />
      )}
      {askCancelId != null && (
        <ConfirmDialog
          title="Cancel this optimized version?"
          body="The re-encode will be stopped and the row marked cancelled. If it's already running, the ffmpeg process is killed and the partial output file is discarded. The source file is untouched. You can re-queue the same file × preset later."
          confirmLabel="Cancel re-encode"
          destructive
          busy={cancelBusy}
          onConfirm={() => void confirmCancel()}
          onCancel={() => setAskCancelId(null)}
        />
      )}
    </div>
  );
}

/**
 * Progress bar for a running optimized version. Determinate (fill driven
 * by the measured permille) when the worker has stamped progress;
 * otherwise an indeterminate sweep so a freshly-started or
 * unknown-duration encode reads as alive without a fake number.
 */
function ProgressBar({ permille }: { permille: number | null }) {
  const determinate = permille != null;
  const pct = determinate
    ? Math.max(0, Math.min(100, permille / 10))
    : 0;
  return (
    <div
      className="cf-flex cf-gap8"
      style={{ alignItems: "center", marginTop: 5, maxWidth: 220 }}
    >
      <div
        className={`cf-prog${determinate ? "" : " cf-indet"}`}
        style={{ flex: 1 }}
        role="progressbar"
        aria-valuenow={determinate ? Math.round(pct) : undefined}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <i style={{ width: `${pct}%` }} />
      </div>
      {determinate && (
        <span
          className="cf-mono cf-faint"
          style={{ fontSize: 10, minWidth: 30, textAlign: "right" }}
        >
          {pct.toFixed(0)}%
        </span>
      )}
    </div>
  );
}

function NewOptimizedForm({
  presets,
  onQueued,
  onError,
}: {
  presets: TranscoderPreset[];
  onQueued: () => Promise<void>;
  onError: (m: string | null) => void;
}) {
  const [sourceFileId, setSourceFileId] = useState<number | "">("");
  // Initialize from the first *enabled* preset so the state matches the rendered <option> list.
  const [presetId, setPresetId] = useState<number>(presets.find((p) => p.enabled)?.id ?? 0);
  const [busy, setBusy] = useState(false);

  async function submit() {
    if (sourceFileId === "" || !presetId) return;
    setBusy(true);
    onError(null);
    try {
      await adminApi.versions.enqueue({
        source_file_id: Number(sourceFileId),
        preset_id: presetId,
      });
      setSourceFileId("");
      await onQueued();
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="cf-card">
      <div className="cf-card-body cf-pad">
        <div className="cf-grid cf-c3">
          <Field
            label="Source media_file ID"
            hint="Find media_file IDs via the items detail JSON or the items API."
          >
            <input
              type="number"
              value={sourceFileId}
              onChange={(e) =>
                setSourceFileId(
                  e.target.value === "" ? "" : Number(e.target.value),
                )
              }
              className="cf-input"
            />
          </Field>
          <Field label="Preset">
            <select
              value={presetId}
              onChange={(e) => setPresetId(Number(e.target.value))}
              className="cf-select"
            >
              {presets
                .filter((p) => p.enabled)
                .map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                  </option>
                ))}
            </select>
          </Field>
          <div className="cf-flex" style={{ alignItems: "flex-end" }}>
            <button
              type="button"
              disabled={busy || sourceFileId === ""}
              onClick={submit}
              className="cf-btn cf-primary cf-sm"
              style={{ width: "100%", justifyContent: "center" }}
            >
              {busy ? "Queueing…" : "Queue"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function StatusBadge({ status }: { status: string }) {
  const tone =
    status === "success"
      ? " cf-ok"
      : status === "running"
        ? " cf-info"
        : status === "queued"
          ? " cf-warn"
          : status === "failed"
            ? " cf-err"
            : // "cancelled" (and any unknown) fall through to the neutral
              // grey base pill.
              "";
  const label =
    status === "success" ? "Ready" : status.charAt(0).toUpperCase() + status.slice(1);
  return (
    <span className={`cf-pill${tone}`}>
      <span className="cf-dot" />
      {label}
    </span>
  );
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="cf-field" style={{ marginBottom: 0 }}>
      <label className="cf-field-label">{label}</label>
      {children}
      {hint && (
        <p className="cf-faint" style={{ marginTop: 6, fontSize: 11.5 }}>
          {hint}
        </p>
      )}
    </div>
  );
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v.toFixed(v >= 100 ? 0 : 1)} ${units[i]}`;
}

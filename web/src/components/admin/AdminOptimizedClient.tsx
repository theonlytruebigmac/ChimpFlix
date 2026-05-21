"use client";

import { useState } from "react";
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

  async function refresh() {
    try {
      const r = await adminApi.optimized.list();
      setVersions(r.versions);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  function remove(id: number) {
    setAskDeleteId(id);
  }

  async function confirmDelete() {
    if (askDeleteId == null) return;
    setDeleteBusy(true);
    try {
      await adminApi.optimized.delete(askDeleteId);
      await refresh();
      setAskDeleteId(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setDeleteBusy(false);
    }
  }

  return (
    <div className="space-y-6">
      {error && (
        <div className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {error}
        </div>
      )}

      <div className="flex items-center justify-between">
        <span className="text-sm text-white/60">
          {versions.length} version{versions.length === 1 ? "" : "s"}
        </span>
        <button
          onClick={() => setShowAdd((v) => !v)}
          className="rounded-md bg-red-500 px-4 py-2.5 text-sm font-semibold sm:px-3 sm:py-1.5 text-white hover:bg-red-600"
        >
          {showAdd ? "Cancel" : "+ Queue version"}
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
        <div className="rounded-lg border border-dashed border-white/15 bg-white/2 p-8 text-center text-sm text-white/50">
          No optimized versions queued.
        </div>
      ) : (
        <div className="overflow-hidden rounded-lg border border-white/10">
          <table className="w-full text-sm">
            <thead className="bg-white/5 text-left text-xs uppercase tracking-wider text-white/40">
              <tr>
                <th className="px-4 py-2">Source file</th>
                <th className="px-4 py-2">Preset</th>
                <th className="px-4 py-2">Status</th>
                <th className="px-4 py-2">Size</th>
                <th className="px-4 py-2">Output</th>
                <th className="px-4 py-2" />
              </tr>
            </thead>
            <tbody>
              {versions.map((v) => {
                const preset = presets.find((p) => p.id === v.preset_id);
                return (
                  <tr key={v.id} className="border-t border-white/5">
                    <td className="whitespace-nowrap px-4 py-2 font-mono text-xs text-white/60">
                      file #{v.source_file_id}
                    </td>
                    <td className="whitespace-nowrap px-4 py-2 text-white/70">
                      {preset?.name ?? `preset #${v.preset_id}`}
                    </td>
                    <td className="whitespace-nowrap px-4 py-2">
                      <StatusBadge status={v.status} />
                      {v.error && (
                        <div className="mt-0.5 max-w-md truncate text-[10px] text-red-300" title={v.error}>
                          {v.error}
                        </div>
                      )}
                    </td>
                    <td className="whitespace-nowrap px-4 py-2 text-white/60">
                      {v.output_size_bytes != null ? formatBytes(v.output_size_bytes) : "—"}
                    </td>
                    <td className="px-4 py-2 font-mono text-xs text-white/40">
                      <div className="line-clamp-1 max-w-md" title={v.output_path}>
                        {v.output_path || "—"}
                      </div>
                    </td>
                    <td className="whitespace-nowrap px-4 py-2 text-right">
                      <button
                        onClick={() => remove(v.id)}
                        className="rounded border border-white/15 px-2 py-1 text-xs text-white/50 hover:border-red-500/50 hover:text-red-300"
                      >
                        Delete
                      </button>
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
  const [presetId, setPresetId] = useState<number>(presets[0]?.id ?? 0);
  const [busy, setBusy] = useState(false);

  async function submit() {
    if (sourceFileId === "" || !presetId) return;
    setBusy(true);
    onError(null);
    try {
      await adminApi.optimized.enqueue({
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
    <div className="grid grid-cols-1 gap-3 rounded-lg border border-white/10 bg-white/2 p-4 md:grid-cols-3">
      <Field
        label="Source media_file ID"
        hint="Find media_file IDs via the items detail JSON or the items API."
      >
        <input
          type="number"
          value={sourceFileId}
          onChange={(e) =>
            setSourceFileId(e.target.value === "" ? "" : Number(e.target.value))
          }
          className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
        />
      </Field>
      <Field label="Preset">
        <select
          value={presetId}
          onChange={(e) => setPresetId(Number(e.target.value))}
          className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
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
      <div className="flex items-end">
        <button
          disabled={busy || sourceFileId === ""}
          onClick={submit}
          className="w-full rounded-md bg-red-500 px-3 py-2 text-sm font-semibold text-white hover:bg-red-600 disabled:opacity-50"
        >
          {busy ? "Queueing…" : "Queue"}
        </button>
      </div>
    </div>
  );
}

function StatusBadge({ status }: { status: string }) {
  const cls =
    status === "success"
      ? "bg-emerald-500/15 text-emerald-300"
      : status === "running"
        ? "bg-blue-500/15 text-blue-300"
        : status === "queued"
          ? "bg-amber-500/15 text-amber-300"
          : status === "failed"
            ? "bg-red-500/15 text-red-300"
            : "bg-white/10 text-white/60";
  return (
    <span
      className={`rounded px-1.5 py-0.5 text-[10px] uppercase tracking-wider ${cls}`}
    >
      {status}
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
    <div>
      <label className="mb-1 block text-xs font-medium uppercase tracking-wider text-white/50">
        {label}
      </label>
      {children}
      {hint && <p className="mt-1 text-xs text-white/50">{hint}</p>}
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

"use client";

import { useEffect, useState } from "react";
import {
  backups as backupsApi,
  type BackupEntry,
  type ListBackupsResponse,
} from "@/lib/chimpflix-api";

/// Admin surface for the persisted auto-backup snapshots
/// (`<data_dir>/backups/auto/`). Lists every snapshot with size +
/// age, exposes Download / Delete / Restore per row, and surfaces a
/// big amber banner when a restore is staged so the operator
/// remembers to restart.
export function AdminBackupRestoreClient() {
  const [data, setData] = useState<ListBackupsResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null); // filename or "cancel"
  const [confirmRestore, setConfirmRestore] = useState<BackupEntry | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  // Captured once at mount so render is a pure function of state.
  // Refreshed on every backup list reload so the "Xh ago" labels
  // stay roughly accurate as the page sits open.
  const [nowMs, setNowMs] = useState<number>(0);

  async function refresh() {
    setError(null);
    try {
      const r = await backupsApi.list();
      setData(r);
      setNowMs(Date.now());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    // Mount-time fetch. Inlined (not a call to `refresh()`) so the
    // react-hooks/set-state-in-effect lint can see the setState
    // happens in the async .then callback (the rule allows that
    // shape; "function that internally setStates" trips it).
    let cancelled = false;
    backupsApi
      .list()
      .then((r) => {
        if (cancelled) return;
        setData(r);
        setNowMs(Date.now());
      })
      .catch((e) => {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function downloadOne(entry: BackupEntry) {
    setBusy(entry.filename);
    setError(null);
    try {
      await backupsApi.download(entry.filename);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function deleteOne(entry: BackupEntry) {
    if (!window.confirm(`Delete backup ${entry.filename}? This cannot be undone.`)) {
      return;
    }
    setBusy(entry.filename);
    setError(null);
    try {
      await backupsApi.delete(entry.filename);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function performStageRestore() {
    if (!confirmRestore) return;
    setBusy(confirmRestore.filename);
    setError(null);
    try {
      const r = await backupsApi.stageRestore(confirmRestore.filename);
      setToast(r.message);
      setConfirmRestore(null);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function cancelRestore() {
    if (!window.confirm("Cancel the staged restore? The current database will keep loading on next restart.")) {
      return;
    }
    setBusy("cancel");
    setError(null);
    try {
      await backupsApi.cancelRestore();
      setToast("Staged restore cancelled.");
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="space-y-4">
      {error && (
        <div className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {error}
        </div>
      )}
      {toast && (
        <div className="rounded-md border border-white/15 bg-white/5 px-3 py-2 text-xs text-white/80">
          {toast}
        </div>
      )}

      {data?.pending_restore && (
        <div className="rounded-lg border border-amber-500/40 bg-amber-500/10 p-4">
          <div className="flex items-start justify-between gap-3">
            <div className="text-sm text-amber-200">
              <div className="font-semibold">Restore staged — restart required</div>
              <p className="mt-1 text-xs text-amber-200/80">
                A backup is queued as the next-boot database. Restart the
                server to apply it. Your current database will be preserved
                as <code className="font-mono">chimpflix.db.pre-restore-&lt;timestamp&gt;.db</code> in
                the data directory.
              </p>
            </div>
            <button
              type="button"
              onClick={cancelRestore}
              disabled={busy === "cancel"}
              className="shrink-0 rounded border border-amber-500/40 px-3 py-1.5 text-xs text-amber-200 hover:bg-amber-500/10 disabled:opacity-50"
            >
              {busy === "cancel" ? "Cancelling…" : "Cancel restore"}
            </button>
          </div>
        </div>
      )}

      <section className="rounded-lg border border-white/10 bg-white/2">
        <div className="flex items-baseline justify-between border-b border-white/10 px-4 py-3">
          <div>
            <h3 className="text-sm font-semibold">Auto-backup snapshots</h3>
            <p className="mt-0.5 text-xs text-white/50">
              Persisted under <code className="font-mono text-white/65">&lt;data_dir&gt;/backups/auto/</code>.
              Written by the <code className="font-mono text-white/65">backup_db</code> scheduled task.
            </p>
          </div>
          {data && data.backups.length > 0 && (
            <div className="text-xs tabular-nums text-white/55">
              {data.backups.length} file{data.backups.length === 1 ? "" : "s"}
              {" · "}
              {formatBytes(data.total_bytes)}
            </div>
          )}
        </div>

        {loading ? (
          <div className="px-4 py-6 text-center text-sm text-white/50">Loading…</div>
        ) : !data || data.backups.length === 0 ? (
          <div className="px-4 py-6 text-center text-sm text-white/50">
            No backups yet. The <code className="font-mono text-white/65">backup_db</code> scheduled task
            writes one daily during the maintenance window.
          </div>
        ) : (
          <ul className="divide-y divide-white/5">
            {data.backups.map((b) => (
              <li
                key={b.filename}
                className="flex flex-wrap items-center gap-3 px-4 py-3 sm:flex-nowrap"
              >
                <div className="min-w-0 flex-1">
                  <div className="truncate font-mono text-sm">{b.filename}</div>
                  <div className="mt-0.5 text-xs text-white/50">
                    {formatBytes(b.size_bytes)}
                    {" · "}
                    {nowMs > 0 ? `${formatRelative(nowMs - b.modified_ms)} ago` : ""}
                  </div>
                </div>
                <button
                  type="button"
                  disabled={busy === b.filename}
                  onClick={() => downloadOne(b)}
                  className="rounded border border-white/15 px-2 py-1 text-xs text-white/80 hover:bg-white/5 disabled:opacity-50"
                >
                  Download
                </button>
                <button
                  type="button"
                  disabled={busy === b.filename}
                  onClick={() => setConfirmRestore(b)}
                  className="rounded border border-amber-500/40 px-2 py-1 text-xs text-amber-300 hover:bg-amber-500/10 disabled:opacity-50"
                >
                  Restore…
                </button>
                <button
                  type="button"
                  disabled={busy === b.filename}
                  onClick={() => deleteOne(b)}
                  className="rounded border border-red-500/40 px-2 py-1 text-xs text-red-300 hover:bg-red-500/10 disabled:opacity-50"
                >
                  Delete
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>

      {confirmRestore && (
        <RestoreConfirmDialog
          entry={confirmRestore}
          onConfirm={performStageRestore}
          onCancel={() => setConfirmRestore(null)}
          busy={busy === confirmRestore.filename}
        />
      )}
    </div>
  );
}

function RestoreConfirmDialog({
  entry,
  onConfirm,
  onCancel,
  busy,
}: {
  entry: BackupEntry;
  onConfirm: () => void;
  onCancel: () => void;
  busy: boolean;
}) {
  return (
    <div
      role="dialog"
      aria-modal="true"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4"
      onClick={(e) => {
        if (e.target === e.currentTarget && !busy) onCancel();
      }}
    >
      <div className="w-full max-w-md rounded-lg border border-amber-500/30 bg-neutral-950 p-6 shadow-2xl">
        <h2 className="text-lg font-semibold text-amber-200">Stage restore</h2>
        <p className="mt-3 text-sm text-white/80">
          Queue <span className="font-mono text-amber-200">{entry.filename}</span> as
          the next-boot database. The actual restore happens when you restart
          the server — your current database will be preserved as a rollback file.
        </p>
        <p className="mt-3 text-xs text-white/55">
          Any changes made between now and restart (new play state, scans,
          metadata edits) will be in the rollback file, not the restored DB.
        </p>
        <div className="mt-5 flex justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            disabled={busy}
            className="rounded-md border border-white/15 px-4 py-2 text-sm text-white/80 hover:bg-white/5 disabled:opacity-50"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={onConfirm}
            disabled={busy}
            className="rounded-md bg-amber-500 px-4 py-2 text-sm font-semibold text-black hover:bg-amber-400 disabled:opacity-50"
          >
            {busy ? "Staging…" : "Stage for next restart"}
          </button>
        </div>
      </div>
    </div>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GiB`;
}

function formatRelative(ms: number): string {
  if (ms < 1000) return "just now";
  const seconds = Math.floor(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h`;
  return `${Math.floor(seconds / 86400)}d`;
}

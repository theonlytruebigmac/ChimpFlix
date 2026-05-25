"use client";

import { useEffect, useRef, useState } from "react";
import {
  admin as adminApi,
  backups as backupsApi,
  type BackupEntry,
  type ListBackupsResponse,
} from "@/lib/chimpflix-api";
import { ConfirmDialog } from "../ConfirmDialog";
import { ErrorBanner } from "./ui";
import { LoadingPlaceholder } from "../ui/LoadingPlaceholder";

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
  const [askDelete, setAskDelete] = useState<BackupEntry | null>(null);
  const [askCancelRestore, setAskCancelRestore] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const [retentionDraft, setRetentionDraft] = useState<number | null>(null);
  const [retentionSaving, setRetentionSaving] = useState(false);
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

  function deleteOne(entry: BackupEntry) {
    setAskDelete(entry);
  }

  async function confirmDeleteOne() {
    if (!askDelete) return;
    const entry = askDelete;
    setAskDelete(null);
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

  // Watch the server-restart progress whenever a restore is staged.
  // The actual restore happens on next boot, so the operator clicks
  // "Stage" then has to wait through a manual restart cycle. The
  // amber banner above tells them what's happening; this effect adds
  // observability — once a stage is queued, we poll `/api/v1/health`
  // every 5s and show the operator whether the server is up, down, or
  // freshly restarted (uptime reset back to near-zero).
  //
  // `pollingStatus` only carries the in-flight signal; the idle case
  // is derived at render time from `data?.pending_restore` so we
  // don't have to call `setState` synchronously from inside the
  // effect just to "reset" it.
  const restoredOnce = useRef(false);
  const initialUptimeRef = useRef<number | null>(null);
  const [pollingStatus, setPollingStatus] = useState<
    "polling" | "down" | "back"
  >("polling");
  useEffect(() => {
    if (!data?.pending_restore) {
      restoredOnce.current = false;
      initialUptimeRef.current = null;
      return;
    }
    let cancelled = false;
    const tick = async () => {
      try {
        const r = await fetch("/api/v1/health", { cache: "no-store" });
        if (!r.ok) {
          if (!cancelled) setPollingStatus("down");
          return;
        }
        const body = (await r.json()) as { uptime_s?: number };
        const uptime = body.uptime_s ?? 0;
        if (initialUptimeRef.current === null) {
          initialUptimeRef.current = uptime;
        } else if (
          uptime < initialUptimeRef.current &&
          !restoredOnce.current
        ) {
          // Server restarted (new uptime is less than what we
          // recorded before). Refresh the list to see if the
          // pending_restore flag has cleared.
          restoredOnce.current = true;
          if (!cancelled) {
            setPollingStatus("back");
            setToast("Server restarted — restore applied.");
            await refresh();
          }
        } else if (!cancelled) {
          setPollingStatus("polling");
        }
      } catch {
        if (!cancelled) setPollingStatus("down");
      }
    };
    void tick();
    const id = window.setInterval(tick, 5000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [data?.pending_restore]);
  const restartStatus: "idle" | "polling" | "down" | "back" = data?.pending_restore
    ? pollingStatus
    : "idle";

  function cancelRestore() {
    setAskCancelRestore(true);
  }

  async function confirmCancelRestore() {
    setAskCancelRestore(false);
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
      <ErrorBanner error={error} />
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
              <p
                className="mt-2 text-xs text-amber-200/80"
                role="status"
                aria-live="polite"
              >
                {restartStatus === "down"
                  ? "Server is offline — waiting for it to come back…"
                  : restartStatus === "back"
                    ? "Server is back online. Reload this page to confirm."
                    : "Watching server uptime — we'll let you know the moment it restarts."}
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

      {data?.vault_key_required && (
        <div className="rounded-lg border border-amber-500/40 bg-amber-500/10 p-4 text-sm text-amber-200">
          <div className="font-semibold">Back up your vault key alongside these snapshots</div>
          <p className="mt-1 text-xs text-amber-200/80">
            This server has encrypted credentials at rest (SMTP password,
            Trakt tokens, TOTP secrets, session HMAC). Backups are useless
            without the matching{" "}
            <code className="font-mono">CHIMPFLIX_SECRET_KEY</code>. Store the
            current key off-box in a password manager — restoring a snapshot
            against a different key bricks every encrypted credential.
            {" "}
            See <a
              href="https://github.com/soybigmac/ChimpFlix/blob/main/docs/PUBLIC_RELEASE_HARDENING.md#1-backupvault-decoupling-silently-bricks-restores"
              target="_blank"
              rel="noreferrer"
              className="underline hover:text-amber-100"
            >
              the hardening doc
            </a> for the recovery procedure.
          </p>
        </div>
      )}

      {data && (
        <section className="rounded-lg border border-white/10 bg-white/2 p-4">
          <div className="flex items-center justify-between gap-3">
            <div>
              <h3 className="text-sm font-semibold">Retention</h3>
              <p className="mt-0.5 text-xs text-white/50">
                Daily snapshots past this count get pruned after each
                <code className="ml-1 font-mono text-white/65">backup_db</code> run.
                Set to 0 to disable pruning entirely. Range 0&ndash;365.
              </p>
            </div>
            <div className="flex items-center gap-2">
              <input
                type="number"
                min={0}
                max={365}
                value={retentionDraft ?? data.retention_count}
                onChange={(e) =>
                  setRetentionDraft(Number.parseInt(e.target.value, 10) || 0)
                }
                className="w-20 rounded-md border border-white/15 bg-black/30 px-2 py-1 text-right text-sm tabular-nums outline-none focus:border-(--color-accent)"
              />
              <button
                type="button"
                disabled={
                  retentionSaving ||
                  retentionDraft == null ||
                  retentionDraft === data.retention_count
                }
                onClick={async () => {
                  if (retentionDraft == null) return;
                  setRetentionSaving(true);
                  try {
                    await adminApi.settings.patch({
                      backup_retention_count: retentionDraft,
                    });
                    await refresh();
                    setToast(`Retention updated to ${retentionDraft}.`);
                    setRetentionDraft(null);
                  } catch (e) {
                    setError(e instanceof Error ? e.message : String(e));
                  } finally {
                    setRetentionSaving(false);
                  }
                }}
                className="rounded bg-(--color-accent) px-3 py-1 text-xs font-semibold text-white disabled:opacity-50"
              >
                {retentionSaving ? "Saving…" : "Save"}
              </button>
            </div>
          </div>
        </section>
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
            (() => {
              const pressure =
                data.retention_count > 0
                  ? data.backups.length / data.retention_count
                  : 0;
              const colour =
                pressure >= 1
                  ? "text-amber-300"
                  : pressure >= 0.85
                    ? "text-amber-200/80"
                    : "text-white/55";
              return (
                <div className={`text-xs tabular-nums ${colour}`}>
                  {data.retention_count > 0 ? (
                    <>
                      {data.backups.length} of {data.retention_count} retained
                      {pressure >= 0.85 && (
                        <span className="ml-1">(next run will prune)</span>
                      )}
                    </>
                  ) : (
                    <>
                      {data.backups.length} file{data.backups.length === 1 ? "" : "s"} (retention disabled)
                    </>
                  )}
                  {" · "}
                  {formatBytes(data.total_bytes)}
                </div>
              );
            })()
          )}
        </div>

        {loading ? (
          <LoadingPlaceholder />
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
      {askDelete && (
        <ConfirmDialog
          title={`Delete backup ${askDelete.filename}?`}
          body="The snapshot file is removed from disk. This cannot be undone — restoring later would require re-creating a backup."
          confirmLabel="Delete"
          destructive
          busy={busy === askDelete.filename}
          onConfirm={() => void confirmDeleteOne()}
          onCancel={() => setAskDelete(null)}
        />
      )}
      {askCancelRestore && (
        <ConfirmDialog
          title="Cancel the staged restore?"
          body="The current database keeps loading on next restart. The chosen snapshot is no longer queued for restore."
          confirmLabel="Cancel restore"
          destructive
          busy={busy === "cancel"}
          onConfirm={() => void confirmCancelRestore()}
          onCancel={() => setAskCancelRestore(false)}
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

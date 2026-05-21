"use client";

import { useState } from "react";
import { downloadBackup } from "@/lib/chimpflix-api";

// Owner-only "Download backup" button. Hits POST /admin/backup which
// runs VACUUM INTO server-side and streams back the resulting .db file.
// The browser handles the save dialog via a Blob download.
export function SettingsBackupClient() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lastDownload, setLastDownload] = useState<Date | null>(null);

  async function run() {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await downloadBackup();
      setLastDownload(new Date());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-3">
      <p className="text-xs text-white/55">
        Atomic SQLite snapshot covering users, library metadata, play state,
        and reviews. Doesn&apos;t include the media files themselves — keep
        those backed up separately.
      </p>
      <div className="flex items-center gap-3">
        <button
          type="button"
          onClick={run}
          disabled={busy}
          className="rounded bg-(--color-accent) px-4 py-2.5 text-sm font-semibold text-white sm:px-3 sm:py-2 sm:text-xs transition disabled:opacity-50"
        >
          {busy ? "Preparing…" : "Download backup"}
        </button>
        {lastDownload && !busy && (
          <span className="text-xs text-white/55">
            Downloaded at {lastDownload.toLocaleTimeString()}
          </span>
        )}
      </div>
      {error && (
        <p className="text-xs text-(--color-accent)">{error}</p>
      )}
      <details className="text-xs text-white/55">
        <summary className="cursor-pointer text-white/70 transition-colors hover:text-white">
          Restore instructions
        </summary>
        <div className="mt-2 space-y-2 rounded border border-white/10 bg-white/5 p-3">
          <p>To restore from a snapshot:</p>
          <ol className="ml-5 list-decimal space-y-1">
            <li>Stop the server: <code className="rounded bg-black/40 px-1 py-0.5">docker compose stop server</code></li>
            <li>Replace <code className="rounded bg-black/40 px-1 py-0.5">/data/chimpflix.db</code> with the downloaded file</li>
            <li>Delete <code className="rounded bg-black/40 px-1 py-0.5">/data/chimpflix.db-shm</code> and <code className="rounded bg-black/40 px-1 py-0.5">-wal</code> if present</li>
            <li>Restart: <code className="rounded bg-black/40 px-1 py-0.5">docker compose up -d server</code></li>
          </ol>
          <p className="text-white/45">
            Restore is intentionally not a one-click action — replacing the
            DB is destructive and you should confirm the snapshot before
            swapping it in.
          </p>
        </div>
      </details>
    </div>
  );
}

"use client";

import { useState } from "react";
import { downloadBackup } from "@/lib/chimpflix-api";

// Owner-only "Download backup" button. Hits POST /admin/backups which
// runs VACUUM INTO server-side and streams back the resulting .db file.
// The browser handles the save dialog via a Blob download.
//
// Rendered as the "On-demand" card in the Maintenance → Backups tab,
// styled with the console `cf-*` design system.
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
    <div className="cf-card">
      <div className="cf-card-head">
        <div>
          <div className="cf-ttl">On-demand</div>
          <div className="cf-sub">
            Writes a consistent copy with{" "}
            <span className="cf-mono">VACUUM INTO</span> — safe while the server
            is running. Atomic SQLite snapshot covering users, library metadata,
            play state, and reviews. Doesn&apos;t include the media files
            themselves — keep those backed up separately.
          </div>
        </div>
      </div>
      <div className="cf-card-body cf-pad">
        <div className="cf-flex cf-wrap cf-gap12">
          <button
            type="button"
            onClick={run}
            disabled={busy}
            className="cf-btn cf-primary"
          >
            <svg
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <path d="M12 4v11M8 11l4 4 4-4" />
              <path d="M4 19h16" />
            </svg>
            {busy ? "Preparing…" : "Download backup now"}
          </button>
          {lastDownload && !busy && (
            <span className="cf-faint" style={{ fontSize: 12.5 }}>
              Downloaded at {lastDownload.toLocaleTimeString()}
            </span>
          )}
        </div>
        {error && (
          <div
            role="alert"
            aria-live="assertive"
            className="cf-banner cf-err"
            style={{ marginTop: 12, marginBottom: 0 }}
          >
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <circle cx="12" cy="12" r="9" />
              <path d="M12 8v4M12 16v.5" />
            </svg>
            <div>{error}</div>
          </div>
        )}
        <details className="cf-muted" style={{ fontSize: 12.5, marginTop: 14 }}>
          <summary style={{ cursor: "pointer" }}>Restore instructions</summary>
          <div
            style={{
              marginTop: 8,
              borderRadius: 8,
              border: "1px solid var(--line)",
              background: "rgba(255,255,255,0.04)",
              padding: 12,
            }}
          >
            <p>To restore from a snapshot:</p>
            <ol style={{ marginLeft: 20, listStyle: "decimal", marginTop: 6 }}>
              <li>
                Stop the server:{" "}
                <code className="cf-mono">docker compose stop server</code>
              </li>
              <li>
                Replace <code className="cf-mono">/data/chimpflix.db</code> with
                the downloaded file
              </li>
              <li>
                Delete <code className="cf-mono">/data/chimpflix.db-shm</code> and{" "}
                <code className="cf-mono">-wal</code> if present
              </li>
              <li>
                Restart:{" "}
                <code className="cf-mono">docker compose up -d server</code>
              </li>
            </ol>
            <p className="cf-faint" style={{ marginTop: 8 }}>
              Restore is intentionally not a one-click action — replacing the DB
              is destructive and you should confirm the snapshot before swapping
              it in.
            </p>
          </div>
        </details>
      </div>
    </div>
  );
}

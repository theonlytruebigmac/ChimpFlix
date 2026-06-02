"use client";

import { useState } from "react";
import {
  admin as adminApi,
  type ClearTranscodeCacheResult,
  type MaintenancePurgeResult,
  type VacuumResult,
  type VerifyAllResult,
} from "@/lib/chimpflix-api";
import { ConfirmDialog } from "../ConfirmDialog";

/// Instance-wide maintenance dashboard. Each card maps 1:1 to a
/// scheduled task so the operator can run any of them on demand and
/// see a structured response inline. Layout intentionally mirrors
/// Plex's Settings → Manage → Troubleshooting:
///
///   1. Verify all libraries — find missing files server-wide
///   2. Purge orphans       — hard-delete soft-deleted rows
///   3. Vacuum database     — defragment SQLite, reclaim space
///   4. Clear transcode     — wipe stale segment dirs from disk
///
/// Restyled to the console `cf-*` design system (card + card-head +
/// card-body) per docs/redesign/admin-maintenance.html.
export function AdminMaintenanceDashboardClient() {
  return (
    <div className="cf-grid cf-c2">
      <VerifyAllCard />
      <PurgeAllCard />
      <VacuumCard />
      <ClearTranscodeCacheCard />
    </div>
  );
}

function VerifyAllCard() {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<VerifyAllResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function run() {
    setBusy(true);
    setError(null);
    try {
      setResult(await adminApi.maintenance.verifyAll());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card
      title="Verify all libraries"
      description="Stat every media file across every library. Files whose disk path is gone get soft-deleted (the row stays for 7 days so watch history and markers survive a temporary unmount). Same as the weekly scheduled task — run on demand when you've moved files around."
      action={<ActionButton onClick={run} busy={busy} label="Verify all" />}
      error={error}
    >
      {result && (
        <StatGrid>
          <Stat label="Libraries" value={result.libraries_checked} />
          <Stat label="Files checked" value={result.files_checked} />
          <Stat
            label="Missing"
            value={result.files_missing}
            emphasis={result.files_missing > 0}
          />
          <Stat
            label="Newly removed"
            value={result.newly_marked_removed}
            emphasis={result.newly_marked_removed > 0}
          />
          <Stat
            label="Returned"
            value={result.returned_files}
            emphasis={result.returned_files > 0}
          />
        </StatGrid>
      )}
    </Card>
  );
}

function PurgeAllCard() {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<MaintenancePurgeResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [askImmediate, setAskImmediate] = useState(false);

  async function run(immediate: boolean) {
    setBusy(true);
    setError(null);
    try {
      setResult(
        await adminApi.maintenance.purgeAll(immediate ? 0 : undefined),
      );
      if (immediate) setAskImmediate(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <Card
        title="Purge orphan files"
        description="Hard-delete media_file rows whose grace window has expired (default 7 days). Cascade-sweeps orphan episodes (no files), seasons (no episodes), and items (no files or seasons). Use 'now' to bypass the grace window — only after you've verified the files are gone for good."
        action={
          <>
            <ActionButton
              onClick={() => run(false)}
              busy={busy}
              label="Purge expired"
            />
            <button
              type="button"
              onClick={() => setAskImmediate(true)}
              disabled={busy}
              className="cf-btn cf-danger cf-sm"
            >
              <svg
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="M4 7h16M9 7V5h6v2M6 7l1 13h10l1-13" />
              </svg>
              Purge all now
            </button>
          </>
        }
        error={error}
      >
        {result && (
          <StatGrid>
            <Stat label="Files" value={result.files_purged} emphasis />
            <Stat label="Episodes" value={result.episodes_purged} emphasis />
            <Stat label="Seasons" value={result.seasons_purged} emphasis />
            <Stat label="Items" value={result.items_purged} emphasis />
          </StatGrid>
        )}
      </Card>
      {askImmediate && (
        <ConfirmDialog
          title="Purge every orphan row right now?"
          body="This hard-deletes every soft-deleted media_file row across all libraries, plus cascade-sweeps orphan episodes / seasons / items left without children. The 7-day grace window is bypassed. Cannot be undone — use only after you've verified those files won't return (e.g. you removed the source for good)."
          confirmLabel="Purge all now"
          destructive
          busy={busy}
          onConfirm={() => void run(true)}
          onCancel={() => setAskImmediate(false)}
        />
      )}
    </>
  );
}

function VacuumCard() {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<VacuumResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [askVacuum, setAskVacuum] = useState(false);

  async function run() {
    setBusy(true);
    setError(null);
    try {
      setResult(await adminApi.maintenance.vacuumDatabase());
      setAskVacuum(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <Card
        title="Vacuum database"
        description={
          <>
            Rebuild the SQLite file from scratch with{" "}
            <span className="cf-mono">VACUUM</span>, defragmenting pages and
            shrinking the on-disk size. Reclaims space after big deletions
            (purges, library removals). Blocks other queries while it runs —
            usually a few seconds at our scale.
          </>
        }
        action={
          <ActionButton
            onClick={() => setAskVacuum(true)}
            busy={busy}
            label="Vacuum"
          />
        }
        error={error}
      >
        {result && (
          <StatGrid>
            <Stat label="Before" value={result.before_bytes} format="bytes" />
            <Stat label="After" value={result.after_bytes} format="bytes" />
            <Stat
              label="Reclaimed"
              value={result.bytes_reclaimed}
              format="bytes"
              emphasis={result.bytes_reclaimed > 0}
            />
            <Stat label="Duration" value={result.duration_ms} format="ms" />
          </StatGrid>
        )}
      </Card>
      {askVacuum && (
        <ConfirmDialog
          title="Vacuum the database now?"
          body="Vacuum holds an exclusive write lock on the database for the duration. Most queries (browse, search, scans) will pause until it finishes. Usually a few seconds at our scale; longer after a big purge."
          confirmLabel="Vacuum"
          busy={busy}
          onConfirm={() => void run()}
          onCancel={() => setAskVacuum(false)}
        />
      )}
    </>
  );
}

function ClearTranscodeCacheCard() {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<ClearTranscodeCacheResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [askClear, setAskClear] = useState(false);

  async function run() {
    setBusy(true);
    setError(null);
    try {
      setResult(await adminApi.maintenance.clearTranscodeCache());
      setAskClear(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <Card
        title="Clear transcoder cache"
        description={
          <>
            Wipe orphan session directories from{" "}
            <span className="cf-mono">/data/cache/sessions/</span>. Active
            sessions skip — only directories left behind by previous crashes or
            unclean shutdowns get reaped. Useful when the cache disk is filling
            up and you&apos;ve confirmed no one is mid-playback.
          </>
        }
        action={
          <ActionButton
            onClick={() => setAskClear(true)}
            busy={busy}
            label="Clear cache"
          />
        }
        error={error}
      >
        {result && (
          <StatGrid>
            <Stat
              label="Sessions removed"
              value={result.sessions_removed}
              emphasis={result.sessions_removed > 0}
            />
            <Stat
              label="Bytes freed"
              value={result.bytes_freed}
              format="bytes"
              emphasis={result.bytes_freed > 0}
            />
          </StatGrid>
        )}
      </Card>
      {askClear && (
        <ConfirmDialog
          title="Clear orphan transcoder cache?"
          body="Remove every transcoder session directory on disk that's not currently in use. Active sessions are detected by the manager and skipped, so this won't interrupt anyone mid-playback."
          confirmLabel="Clear cache"
          busy={busy}
          onConfirm={() => void run()}
          onCancel={() => setAskClear(false)}
        />
      )}
    </>
  );
}

function Card({
  title,
  description,
  action,
  error,
  children,
}: {
  title: string;
  description: React.ReactNode;
  action: React.ReactNode;
  error: string | null;
  children?: React.ReactNode;
}) {
  return (
    <div className="cf-card" style={{ marginBottom: 0 }}>
      <div className="cf-card-head">
        <div>
          <div className="cf-ttl">{title}</div>
          <div className="cf-sub">{description}</div>
        </div>
      </div>
      <div className="cf-card-body cf-pad">
        <div className="cf-flex cf-wrap cf-gap12">{action}</div>
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
        {children}
      </div>
    </div>
  );
}

function ActionButton({
  onClick,
  busy,
  label,
}: {
  onClick: () => void;
  busy: boolean;
  label: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={busy}
      className="cf-btn cf-primary"
    >
      {busy ? "Running…" : label}
    </button>
  );
}

function StatGrid({ children }: { children: React.ReactNode }) {
  return (
    <div
      role="status"
      aria-live="polite"
      className="cf-grid cf-c4"
      style={{ marginTop: 16, gap: 10 }}
    >
      {children}
    </div>
  );
}

function Stat({
  label,
  value,
  emphasis = false,
  format = "number",
}: {
  label: string;
  value: number;
  emphasis?: boolean;
  format?: "number" | "bytes" | "ms";
}) {
  let rendered: string;
  if (format === "bytes") rendered = formatBytes(value);
  else if (format === "ms") rendered = `${value.toLocaleString()} ms`;
  else rendered = value.toLocaleString();
  return (
    <div
      style={{
        borderRadius: 8,
        border: "1px solid var(--line-faint)",
        background: "rgba(0,0,0,0.2)",
        padding: "8px 10px",
      }}
    >
      <div
        style={{
          fontSize: 10,
          textTransform: "uppercase",
          letterSpacing: "0.08em",
          color: "var(--faint)",
        }}
      >
        {label}
      </div>
      <div
        className="cf-mono"
        style={{
          fontVariantNumeric: "tabular-nums",
          color: emphasis ? "var(--warn)" : "var(--fg)",
          marginTop: 2,
        }}
      >
        {rendered}
      </div>
    </div>
  );
}

function formatBytes(bytes: number): string {
  if (bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let i = 0;
  let n = bytes;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  return `${n.toFixed(i >= 2 ? 1 : 0)} ${units[i]}`;
}

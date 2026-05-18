"use client";

import { useState } from "react";
import {
  admin as adminApi,
  type ClearTranscodeCacheResult,
  type MaintenancePurgeResult,
  type VacuumResult,
  type VerifyAllResult,
} from "@/lib/chimpflix-api";

/// Instance-wide maintenance dashboard. Each card maps 1:1 to a
/// scheduled task so the operator can run any of them on demand and
/// see a structured response inline. Layout intentionally mirrors
/// Plex's Settings → Manage → Troubleshooting:
///
///   1. Verify all libraries — find missing files server-wide
///   2. Purge orphans       — hard-delete soft-deleted rows
///   3. Vacuum database     — defragment SQLite, reclaim space
///   4. Clear transcode     — wipe stale segment dirs from disk
export function AdminMaintenanceDashboardClient() {
  return (
    <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
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
      action={
        <ActionButton onClick={run} busy={busy} label="Verify all" />
      }
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

  async function run(immediate: boolean) {
    if (
      immediate &&
      !confirm(
        "Immediately hard-delete every soft-deleted row across every library, plus cascade-sweep orphan episodes / seasons / items. This can't be undone.",
      )
    ) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      setResult(
        await adminApi.maintenance.purgeAll(immediate ? 0 : undefined),
      );
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card
      title="Purge orphan files"
      description="Hard-delete media_file rows whose grace window has expired (default 7 days). Cascade-sweeps orphan episodes (no files), seasons (no episodes), and items (no files or seasons). Use 'now' to bypass the grace window — only after you've verified the files are gone for good."
      action={
        <div className="flex flex-wrap gap-2">
          <ActionButton onClick={() => run(false)} busy={busy} label="Purge expired" />
          <button
            onClick={() => run(true)}
            disabled={busy}
            className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-1.5 text-sm font-medium text-red-300 hover:bg-red-500/20 disabled:opacity-50"
          >
            Purge all now
          </button>
        </div>
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
  );
}

function VacuumCard() {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<VacuumResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function run() {
    if (
      !confirm(
        "Vacuum holds an exclusive lock on the database for the duration. Most queries will pause until it finishes. Continue?",
      )
    ) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      setResult(await adminApi.maintenance.vacuumDatabase());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card
      title="Vacuum database"
      description="Rebuild the SQLite file from scratch, defragmenting pages and shrinking the on-disk size. Reclaims space after big deletions (purges, library removals). Blocks other queries while it runs — usually a few seconds at our scale."
      action={
        <ActionButton onClick={run} busy={busy} label="Vacuum" />
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
  );
}

function ClearTranscodeCacheCard() {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<ClearTranscodeCacheResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function run() {
    if (
      !confirm(
        "Remove every transcoder session directory on disk that's not currently in use. Active sessions are skipped. Continue?",
      )
    ) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      setResult(await adminApi.maintenance.clearTranscodeCache());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card
      title="Clear transcoder cache"
      description="Wipe orphan session directories from /data/cache/sessions/. Active sessions skip — only directories left behind by previous crashes or unclean shutdowns get reaped. Useful when the cache disk is filling up and you've confirmed no one is mid-playback."
      action={
        <ActionButton onClick={run} busy={busy} label="Clear cache" />
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
  description: string;
  action: React.ReactNode;
  error: string | null;
  children?: React.ReactNode;
}) {
  return (
    <section className="rounded-lg border border-white/10 bg-white/2 p-5">
      <h2 className="mb-1 text-base font-semibold">{title}</h2>
      <p className="mb-4 text-xs text-white/55">{description}</p>
      <div className="mb-3">{action}</div>
      {error && (
        <div className="mb-3 rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {error}
        </div>
      )}
      {children}
    </section>
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
      onClick={onClick}
      disabled={busy}
      className="rounded-md bg-red-500 px-4 py-1.5 text-sm font-semibold text-white hover:bg-red-600 disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/40"
    >
      {busy ? "Running…" : label}
    </button>
  );
}

function StatGrid({ children }: { children: React.ReactNode }) {
  return <div className="grid grid-cols-2 gap-2 md:grid-cols-4">{children}</div>;
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
    <div className="rounded border border-white/5 bg-black/20 px-2.5 py-1.5">
      <div className="text-[10px] uppercase tracking-wider text-white/40">
        {label}
      </div>
      <div
        className={`tabular-nums ${emphasis ? "text-amber-300" : "text-white/85"}`}
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

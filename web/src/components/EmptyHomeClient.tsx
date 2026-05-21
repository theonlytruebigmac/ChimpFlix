"use client";

/// Empty-state Home for fresh deployments. Renders when the user
/// has libraries configured but zero items have been indexed yet —
/// usually because the initial scan is still walking the
/// directory tree or because the operator finished onboarding
/// before the scan had time to find anything.
///
/// Without this card the Home page is a black void with no
/// explanation, which reads as "the app is broken." Showing
/// scan progress + a clear "this is normal, hang tight" message
/// makes the wait obviously intentional.

import Link from "next/link";
import { useEffect, useState } from "react";

import {
  friendlyErrorMessage,
  libraries as librariesApi,
  type Library,
  type ScanJob,
} from "@/lib/chimpflix-api";

interface Props {
  libraries: Library[];
  /// True when the current user can run the onboarding wizard /
  /// trigger scans manually. Viewers get the "tell your admin"
  /// version of the copy.
  isAdmin: boolean;
}

export function EmptyHomeClient({ libraries, isAdmin }: Props) {
  if (libraries.length === 0) {
    return <NoLibraries isAdmin={isAdmin} />;
  }
  return <ScanningLibraries libraries={libraries} isAdmin={isAdmin} />;
}

// ─── No libraries at all ──────────────────────────────────────────────

function NoLibraries({ isAdmin }: { isAdmin: boolean }) {
  return (
    <div className="mx-auto flex min-h-[60vh] max-w-xl flex-col items-center justify-center px-6 text-center">
      <div className="mb-4 text-[0.7rem] font-semibold uppercase tracking-[0.18em] text-accent">
        Welcome
      </div>
      <h1 className="text-2xl font-bold tracking-tight text-white/95">
        Nothing here yet
      </h1>
      <p className="mt-2 max-w-md text-sm text-white/55">
        {isAdmin
          ? "No libraries have been added. Walk through the setup wizard to point ChimpFlix at your media."
          : "The owner hasn't added any libraries yet. Once they do, the rails on this page will fill in."}
      </p>
      {isAdmin && (
        <Link
          href="/onboarding"
          className="mt-6 rounded-md bg-accent px-5 py-2.5 text-sm font-semibold text-white transition-colors hover:bg-accent/85"
        >
          Run setup wizard →
        </Link>
      )}
    </div>
  );
}

// ─── Libraries exist but nothing indexed yet ──────────────────────────

function ScanningLibraries({
  libraries,
  isAdmin,
}: {
  libraries: Library[];
  isAdmin: boolean;
}) {
  return (
    <div className="mx-auto max-w-3xl px-6 py-12">
      <div className="mb-2 text-[0.7rem] font-semibold uppercase tracking-[0.18em] text-accent">
        Library
      </div>
      <h1 className="text-2xl font-bold tracking-tight text-white/95">
        Your library is being indexed
      </h1>
      <p className="mt-2 max-w-2xl text-sm text-white/55">
        ChimpFlix is walking the directory tree and pulling metadata for each
        file. Posters, descriptions, and the Continue Watching rail appear as
        items finish. This is usually quick — a few seconds per hundred files
        — but can take longer on large collections or slow disks.
      </p>

      <div className="mt-8 space-y-3">
        {libraries.map((lib) => (
          <LibraryScanRow
            key={lib.id}
            library={lib}
            canRescan={isAdmin}
          />
        ))}
      </div>

      <div className="mt-8 flex flex-wrap items-center gap-3 text-[12.5px] text-white/55">
        <span>This page auto-refreshes — no need to reload.</span>
        {isAdmin && (
          <Link
            href="/settings/admin/library/libraries"
            className="text-white/75 underline-offset-2 hover:text-white hover:underline"
          >
            Manage libraries →
          </Link>
        )}
      </div>
    </div>
  );
}

function LibraryScanRow({
  library,
  canRescan,
}: {
  library: Library;
  canRescan: boolean;
}) {
  const [scan, setScan] = useState<ScanJob | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [retrying, setRetrying] = useState(false);

  useEffect(() => {
    let cancelled = false;
    async function tick() {
      try {
        const { scans } = await librariesApi.listScans(library.id);
        if (cancelled) return;
        setScan(scans[0] ?? null);
        setError(null);
      } catch (e) {
        if (cancelled) return;
        setError(friendlyErrorMessage(e));
      }
    }
    tick();
    // 2s cadence — slightly longer than the wizard's 1.5s since
    // this is a steady-state page rather than a "watch it happen"
    // surface. Live enough that newly-indexed items show up
    // promptly without hammering the API.
    const id = window.setInterval(tick, 2000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [library.id]);

  // Auto-refresh Home when a scan transitions to completed so the
  // empty state disappears and rails take over without the
  // operator having to manually reload. We bounce through
  // window.location to invalidate every server-component cache
  // (router.refresh would also work but reuses the existing tree).
  useEffect(() => {
    if (scan?.status === "completed" && (scan.files_added ?? 0) > 0) {
      const t = window.setTimeout(() => {
        window.location.reload();
      }, 1500);
      return () => window.clearTimeout(t);
    }
  }, [scan?.status, scan?.files_added]);

  async function rescan() {
    setRetrying(true);
    setError(null);
    try {
      await librariesApi.triggerScan(library.id);
    } catch (e) {
      setError(friendlyErrorMessage(e));
    } finally {
      setRetrying(false);
    }
  }

  const status = scan?.status ?? "queued";
  const isFinishedEmpty =
    status === "completed" && (scan?.files_added ?? 0) === 0;

  return (
    <div className="overflow-hidden rounded-lg border border-white/10 bg-white/2">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-white/8 px-4 py-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-[13.5px] font-semibold text-white/95">
              {library.name}
            </span>
            <span className="rounded bg-white/8 px-1.5 py-px text-[10px] font-semibold uppercase tracking-wide text-white/55">
              {library.kind}
            </span>
          </div>
          <div className="mt-0.5 truncate font-mono text-[11.5px] text-white/45">
            {library.paths.join(" · ")}
          </div>
        </div>
        <Pill status={status} />
      </div>
      <div className="grid grid-cols-2 gap-3 px-4 py-3 sm:grid-cols-4">
        <Stat label="Seen" value={scan?.files_seen ?? 0} />
        <Stat label="Added" value={scan?.files_added ?? 0} tone="ok" />
        <Stat label="Updated" value={scan?.files_updated ?? 0} />
        <Stat label="Removed" value={scan?.files_removed ?? 0} />
      </div>
      {isFinishedEmpty && (
        <div className="border-t border-white/8 bg-amber-500/5 px-4 py-3 text-[12px] text-amber-200">
          The scan finished but didn&rsquo;t find any media in{" "}
          <code className="rounded bg-amber-500/10 px-1.5 py-0.5 font-mono text-[11.5px]">
            {library.paths[0] ?? "(no path)"}
          </code>
          . Double-check the path is correct and the server can read it
          (volume mounts inside Docker, file permissions on bare metal).
          {canRescan && (
            <button
              type="button"
              onClick={rescan}
              disabled={retrying}
              className="ml-2 underline underline-offset-2 hover:text-white disabled:opacity-50"
            >
              {retrying ? "Re-scanning…" : "Re-scan"}
            </button>
          )}
        </div>
      )}
      {scan?.error_message && status === "failed" && (
        <div className="border-t border-white/8 bg-red-500/10 px-4 py-3 font-mono text-[11.5px] text-red-300">
          {scan.error_message}
          {canRescan && (
            <button
              type="button"
              onClick={rescan}
              disabled={retrying}
              className="ml-2 font-sans underline underline-offset-2 hover:text-white disabled:opacity-50"
            >
              {retrying ? "Re-scanning…" : "Re-scan"}
            </button>
          )}
        </div>
      )}
      {error && (
        <div className="border-t border-white/8 bg-red-500/10 px-4 py-2 text-[11.5px] text-red-300">
          Couldn&apos;t fetch scan progress: {error}
        </div>
      )}
    </div>
  );
}

function Pill({ status }: { status: ScanJob["status"] }) {
  const map: Record<ScanJob["status"], { label: string; cls: string }> = {
    queued: {
      label: "Queued",
      cls: "bg-white/8 text-white/65 ring-white/15",
    },
    running: {
      label: "Scanning…",
      cls: "bg-blue-500/15 text-blue-300 ring-blue-500/30",
    },
    completed: {
      label: "Done",
      cls: "bg-emerald-500/15 text-emerald-300 ring-emerald-500/30",
    },
    failed: {
      label: "Failed",
      cls: "bg-red-500/15 text-red-300 ring-red-500/30",
    },
    canceled: {
      label: "Canceled",
      cls: "bg-white/8 text-white/55 ring-white/15",
    },
  };
  const m = map[status];
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-[11px] font-medium ring-1 ${m.cls}`}
    >
      {status === "running" && (
        <span
          aria-hidden
          className="inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-blue-300"
        />
      )}
      {m.label}
    </span>
  );
}

function Stat({
  label,
  value,
  tone,
}: {
  label: string;
  value: number;
  tone?: "ok";
}) {
  const valueCls = tone === "ok" ? "text-emerald-300" : "text-white/90";
  return (
    <div>
      <div className="text-[10.5px] font-semibold uppercase tracking-[0.07em] text-white/45">
        {label}
      </div>
      <div className={`mt-0.5 text-xl font-semibold tabular-nums ${valueCls}`}>
        {value.toLocaleString()}
      </div>
    </div>
  );
}

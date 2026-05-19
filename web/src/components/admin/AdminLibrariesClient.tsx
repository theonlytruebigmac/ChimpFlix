"use client";

import { useEffect, useState } from "react";
import {
  admin as adminApi,
  libraries as librariesApi,
  type AgentInfo,
  type EpisodeNaming,
  type EpisodeSortOrder,
  type Library,
  type LibraryAgent,
  type LibraryStatsResult,
  type LibraryUpdateInput,
  type LibraryVisibility,
} from "@/lib/chimpflix-api";

interface Props {
  initialLibraries: Library[];
  availableAgents: AgentInfo[];
}

// One panel per library. Inline "Advanced" disclosure exposes the Plex-style
// extras (episode sort, naming, certification country, visibility) plus the
// per-library metadata agent priority list. The basic name/paths/scan editor
// is intentionally minimal — full CRUD lives on the existing user-facing
// /settings page until Phase 8 consolidates.
export function AdminLibrariesClient({
  initialLibraries,
  availableAgents,
}: Props) {
  const [libraries, setLibraries] = useState(initialLibraries);

  return (
    <div className="space-y-4">
      {libraries.length === 0 && (
        <div className="rounded-lg border border-dashed border-white/15 bg-white/2 p-8 text-center text-sm text-white/50">
          No libraries yet. Create one from the user-facing Settings page;
          it will appear here automatically.
        </div>
      )}
      {libraries.map((lib) => (
        <LibraryCard
          key={lib.id}
          lib={lib}
          availableAgents={availableAgents}
          onUpdated={(next) =>
            setLibraries((all) => all.map((l) => (l.id === next.id ? next : l)))
          }
        />
      ))}
    </div>
  );
}

function LibraryCard({
  lib,
  availableAgents,
  onUpdated,
}: {
  lib: Library;
  availableAgents: AgentInfo[];
  onUpdated: (lib: Library) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const [sortOrder, setSortOrder] = useState<EpisodeSortOrder>(
    lib.episode_sort_order,
  );
  const [naming, setNaming] = useState<EpisodeNaming>(lib.episode_naming);
  const [country, setCountry] = useState(lib.certification_country);
  const [visibility, setVisibility] = useState<LibraryVisibility>(
    lib.visibility,
  );
  const [allowMediaDeletion, setAllowMediaDeletion] = useState<boolean>(
    lib.allow_media_deletion ?? false,
  );
  const [agents, setAgents] = useState<LibraryAgent[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  /// Result of the most recent verify run, surfaced inline so the
  /// operator can see "checked N files, M missing" without opening
  /// the scheduled-tasks history page. Reset on next run.
  const [verifyResult, setVerifyResult] = useState<{
    files_checked: number;
    files_missing: number;
    newly_marked_removed: number;
    returned_files: number;
    orphan_count: number;
  } | null>(null);
  const [verifying, setVerifying] = useState(false);
  const [purging, setPurging] = useState(false);
  const [purgeResult, setPurgeResult] = useState<{
    files_purged: number;
    episodes_purged: number;
    seasons_purged: number;
    items_purged: number;
  } | null>(null);
  /// At-a-glance library stats — items / episodes / files / size /
  /// orphans / last-scanned. Fetched on first expand so the card
  /// header can show the headline numbers without an extra click.
  const [stats, setStats] = useState<LibraryStatsResult | null>(null);
  /// Per-action in-flight flags so multiple Maintenance buttons can
  /// be hit concurrently without their spinners clobbering each
  /// other.
  const [running, setRunning] = useState<{
    scan?: boolean;
    refreshMeta?: boolean;
    markers?: boolean;
    previews?: boolean;
  }>({});
  /// Toast-style ack for fire-and-forget actions (the work happens
  /// in the background; we want the UI to confirm we kicked it).
  const [actionMsg, setActionMsg] = useState<string | null>(null);

  useEffect(() => {
    if (!expanded || agents != null) return;
    adminApi.agents
      .getForLibrary(lib.id)
      .then((r) => setAgents(r.agents))
      .catch((e) =>
        setError(e instanceof Error ? e.message : String(e)),
      );
  }, [expanded, agents, lib.id]);

  useEffect(() => {
    if (!expanded || stats != null) return;
    librariesApi
      .stats(lib.id)
      .then(setStats)
      .catch(() => {});
  }, [expanded, stats, lib.id]);

  async function refreshStats() {
    try {
      const s = await librariesApi.stats(lib.id);
      setStats(s);
    } catch {
      // best-effort
    }
  }

  async function runScan() {
    setRunning((r) => ({ ...r, scan: true }));
    setActionMsg(null);
    try {
      await librariesApi.triggerScan(lib.id);
      setActionMsg("Scan queued.");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setRunning((r) => ({ ...r, scan: false }));
    }
  }

  async function runRefreshMetadata() {
    setRunning((r) => ({ ...r, refreshMeta: true }));
    setActionMsg(null);
    try {
      const r = await librariesApi.refreshMetadata(lib.id);
      setActionMsg(`Metadata refresh queued for ${r.queued} item(s).`);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setRunning((r) => ({ ...r, refreshMeta: false }));
    }
  }

  async function runDetectMarkers() {
    setRunning((r) => ({ ...r, markers: true }));
    setActionMsg(null);
    try {
      const r = await librariesApi.detectMarkers(lib.id);
      setActionMsg(`Marker detection queued for ${r.queued} file(s).`);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setRunning((r) => ({ ...r, markers: false }));
    }
  }

  async function runGeneratePreviews() {
    setRunning((r) => ({ ...r, previews: true }));
    setActionMsg(null);
    try {
      const r = await librariesApi.generatePreviews(lib.id);
      setActionMsg(
        r.queued === 0
          ? "All files already have previews."
          : `Preview generation queued for ${r.queued} file(s).`,
      );
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setRunning((r) => ({ ...r, previews: false }));
    }
  }

  async function runVerify() {
    setVerifying(true);
    setError(null);
    try {
      const r = await librariesApi.verify(lib.id);
      setVerifyResult({
        files_checked: r.files_checked,
        files_missing: r.files_missing,
        newly_marked_removed: r.newly_marked_removed,
        returned_files: r.returned_files,
        orphan_count: r.orphan_count,
      });
      void refreshStats();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setVerifying(false);
    }
  }

  async function runPurge(immediate: boolean) {
    if (
      !confirm(
        immediate
          ? "Immediately hard-delete every orphan file row for this library, plus any episodes/seasons/items left without children? This can't be undone."
          : "Purge orphan files older than the 7-day grace window. Files marked removed today will stay."
      )
    ) {
      return;
    }
    setPurging(true);
    setError(null);
    try {
      const r = await librariesApi.purge(lib.id, immediate ? 0 : undefined);
      setPurgeResult(r);
      // Refresh the verify count so the orphan badge updates.
      try {
        const v = await librariesApi.verify(lib.id);
        setVerifyResult({
          files_checked: v.files_checked,
          files_missing: v.files_missing,
          newly_marked_removed: v.newly_marked_removed,
          returned_files: v.returned_files,
          orphan_count: v.orphan_count,
        });
      } catch {
        // best-effort refresh
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPurging(false);
    }
  }

  const fieldsDirty =
    sortOrder !== lib.episode_sort_order ||
    naming !== lib.episode_naming ||
    country !== lib.certification_country ||
    visibility !== lib.visibility ||
    allowMediaDeletion !== (lib.allow_media_deletion ?? false);

  async function saveFields() {
    setBusy(true);
    setError(null);
    setSaved(false);
    try {
      const patch: LibraryUpdateInput = {};
      if (sortOrder !== lib.episode_sort_order) patch.episode_sort_order = sortOrder;
      if (naming !== lib.episode_naming) patch.episode_naming = naming;
      if (country !== lib.certification_country) patch.certification_country = country;
      if (visibility !== lib.visibility) patch.visibility = visibility;
      if (allowMediaDeletion !== (lib.allow_media_deletion ?? false)) {
        patch.allow_media_deletion = allowMediaDeletion;
      }
      const { library } = await librariesApi.update(lib.id, patch);
      onUpdated(library);
      setSaved(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="rounded-lg border border-white/10 bg-white/2">
      <div className="flex flex-wrap items-center justify-between gap-3 p-4">
        <div className="flex flex-wrap items-center gap-3">
          <span className="text-lg font-semibold">{lib.name}</span>
          <span className="rounded bg-white/10 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-white/60">
            {lib.kind}
          </span>
          <span className="text-xs text-white/40">
            {lib.paths.length} path{lib.paths.length === 1 ? "" : "s"}
          </span>
          {stats && (
            <>
              <span className="text-xs text-white/55">
                {stats.items.toLocaleString()} item
                {stats.items === 1 ? "" : "s"}
                {stats.episodes > 0 &&
                  ` · ${stats.episodes.toLocaleString()} ep${stats.episodes === 1 ? "" : "s"}`}
              </span>
              <span className="text-xs text-white/55">
                {formatBytes(stats.total_bytes)}
              </span>
              {stats.orphan_files > 0 && (
                <span className="rounded-full bg-amber-500/20 px-2.5 py-0.5 text-[10px] font-semibold text-amber-300">
                  {stats.orphan_files} orphan
                  {stats.orphan_files === 1 ? "" : "s"}
                </span>
              )}
              {stats.last_scanned_at && (
                <span className="text-xs text-white/40">
                  scanned {formatRelativeTime(stats.last_scanned_at)}
                </span>
              )}
            </>
          )}
        </div>
        <button
          onClick={() => setExpanded((v) => !v)}
          className="text-sm text-white/60 hover:text-white"
        >
          {expanded ? "Collapse" : "Advanced ▾"}
        </button>
      </div>
      {expanded && (
        <div className="space-y-6 border-t border-white/10 p-6">
          <section>
            <h3 className="mb-3 text-sm font-semibold uppercase tracking-wider text-white/40">
              Episode &amp; metadata defaults
            </h3>
            <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
              <SelectField
                label="Episode sorting"
                hint="Order episodes within a season."
                value={sortOrder}
                onChange={(v) => setSortOrder(v as EpisodeSortOrder)}
                options={[
                  { value: "oldest_first", label: "Oldest first" },
                  { value: "newest_first", label: "Newest first" },
                ]}
              />
              <SelectField
                label="Episode naming"
                hint="Which source supplies episode titles."
                value={naming}
                onChange={(v) => setNaming(v as EpisodeNaming)}
                options={[
                  { value: "tmdb", label: "The Movie Database" },
                  { value: "original", label: "Original air name" },
                  { value: "absolute", label: "Absolute numbering" },
                ]}
              />
              <Field label="Certification country" hint="ISO 3166-1 alpha-2 (e.g. US, GB, DE).">
                <input
                  type="text"
                  value={country}
                  onChange={(e) => setCountry(e.target.value.toUpperCase().slice(0, 2))}
                  maxLength={2}
                  className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm uppercase outline-none focus:border-white/30"
                />
              </Field>
              <SelectField
                label="Visibility"
                hint="Where this library appears."
                value={visibility}
                onChange={(v) => setVisibility(v as LibraryVisibility)}
                options={[
                  { value: "home_and_search", label: "Home & search" },
                  { value: "search_only", label: "Search only" },
                  { value: "hidden", label: "Hidden" },
                ]}
              />
            </div>
            <div className="mt-4 flex items-center gap-3">
              <button
                onClick={saveFields}
                disabled={!fieldsDirty || busy}
                className="rounded-md bg-red-500 px-4 py-2.5 text-sm font-semibold sm:py-2 text-white hover:bg-red-600 disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/40"
              >
                {busy ? "Saving…" : "Save"}
              </button>
              {saved && !fieldsDirty && (
                <span className="text-xs text-white/50">Saved.</span>
              )}
              {error && <span className="text-xs text-red-400">{error}</span>}
            </div>
          </section>

          <section>
            <h3 className="mb-3 text-sm font-semibold uppercase tracking-wider text-white/40">
              Danger zone
            </h3>
            <label className="flex items-start gap-3 rounded-md border border-amber-500/30 bg-amber-500/5 p-3 text-sm">
              <input
                type="checkbox"
                checked={allowMediaDeletion}
                onChange={(e) => setAllowMediaDeletion(e.target.checked)}
                className="mt-1"
              />
              <div>
                <div className="font-medium text-amber-200">
                  Allow operator to delete media files from disk
                </div>
                <div className="mt-1 text-xs text-white/55">
                  When on, the item detail modal exposes a Delete button
                  that hard-deletes media files and orphan rows
                  immediately — no grace window, no undo. Off by default
                  so a wrong click can&apos;t blow away a library. Save
                  after toggling to apply.
                </div>
              </div>
            </label>
          </section>

          <section>
            <h3 className="mb-3 text-sm font-semibold uppercase tracking-wider text-white/40">
              Metadata agents
            </h3>
            {agents == null ? (
              <div className="text-sm text-white/50">Loading…</div>
            ) : (
              <AgentPriorityEditor
                libraryId={lib.id}
                libraryKind={lib.kind}
                available={availableAgents}
                initial={agents}
                onSaved={(next) => setAgents(next)}
              />
            )}
          </section>

          <section>
            <h3 className="mb-3 text-sm font-semibold uppercase tracking-wider text-white/40">
              Actions
            </h3>
            <p className="mb-3 text-xs text-white/55">
              On-demand triggers for everything the scheduler also
              runs in the background. Scan walks the disk for new
              files; Refresh metadata re-queries TMDB/TVDB for every
              item; Detect markers and Generate previews populate
              skip/scrub data. None of these block — work runs in
              the background and the toast reports what got queued.
            </p>
            <div className="flex flex-wrap items-center gap-2">
              <ActionButton
                label="Scan"
                running={running.scan}
                runningLabel="Queuing…"
                onClick={runScan}
              />
              <ActionButton
                label="Refresh metadata"
                running={running.refreshMeta}
                runningLabel="Queuing…"
                onClick={runRefreshMetadata}
              />
              <ActionButton
                label="Detect markers"
                running={running.markers}
                runningLabel="Queuing…"
                onClick={runDetectMarkers}
              />
              <ActionButton
                label="Generate previews"
                running={running.previews}
                runningLabel="Queuing…"
                onClick={runGeneratePreviews}
              />
            </div>
            {actionMsg && (
              <div className="mt-3 rounded-md border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-xs text-emerald-300">
                {actionMsg}
              </div>
            )}
          </section>

          <section>
            <h3 className="mb-3 text-sm font-semibold uppercase tracking-wider text-white/40">
              Maintenance
            </h3>
            <p className="mb-3 text-xs text-white/55">
              Verify checks every file against disk and soft-deletes
              missing ones — watch history is preserved during the
              grace window. Purge hard-deletes soft-deleted rows and
              cascades to orphan episodes / seasons / items. The
              scheduled tasks (weekly verify, daily purge after 7
              days) run automatically; these buttons are the on-demand
              path for &ldquo;I just deleted something and want it cleaned
              up now&rdquo;.
            </p>
            <div className="flex flex-wrap items-center gap-2">
              <ActionButton
                label="Verify now"
                running={verifying}
                runningLabel="Verifying…"
                onClick={runVerify}
              />
              <ActionButton
                label="Purge expired"
                running={purging}
                runningLabel="Purging…"
                disabled={(stats?.orphan_files ?? 0) === 0}
                title="Purge orphans older than the 7-day grace window."
                onClick={() => runPurge(false)}
              />
              <button
                onClick={() => runPurge(true)}
                disabled={purging || (stats?.orphan_files ?? 0) === 0}
                className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-1.5 text-sm font-medium text-red-300 hover:bg-red-500/20 disabled:cursor-not-allowed disabled:opacity-50"
                title="Hard-delete every orphan row now. Skips the grace window."
              >
                Purge all orphans now
              </button>
            </div>
            {verifyResult && (
              <div className="mt-3 grid grid-cols-2 gap-2 text-xs text-white/60 md:grid-cols-4">
                <Stat label="Checked" value={verifyResult.files_checked} />
                <Stat
                  label="Missing"
                  value={verifyResult.files_missing}
                  emphasis={verifyResult.files_missing > 0}
                />
                <Stat
                  label="Newly removed"
                  value={verifyResult.newly_marked_removed}
                  emphasis={verifyResult.newly_marked_removed > 0}
                />
                <Stat
                  label="Returned"
                  value={verifyResult.returned_files}
                  emphasis={verifyResult.returned_files > 0}
                />
              </div>
            )}
            {purgeResult && (
              <div className="mt-3 rounded-md border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-xs text-emerald-300">
                Purged: {purgeResult.files_purged} files,{" "}
                {purgeResult.episodes_purged} episodes,{" "}
                {purgeResult.seasons_purged} seasons,{" "}
                {purgeResult.items_purged} items.
              </div>
            )}
          </section>
        </div>
      )}
    </div>
  );
}

/// Uniform action button so the Actions row doesn't look like a
/// patchwork of Plex / shadcn / random Tailwind classes per button.
function ActionButton({
  label,
  runningLabel,
  running = false,
  disabled = false,
  title,
  onClick,
}: {
  label: string;
  runningLabel: string;
  running?: boolean;
  disabled?: boolean;
  title?: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      disabled={running || disabled}
      title={title}
      className="rounded-md border border-white/15 bg-white/5 px-3 py-1.5 text-sm font-medium text-white/85 hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-50"
    >
      {running ? runningLabel : label}
    </button>
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

function formatRelativeTime(ms: number): string {
  const diff = Date.now() - ms;
  if (diff < 60_000) return "just now";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  return `${Math.floor(diff / 86_400_000)}d ago`;
}

function Stat({
  label,
  value,
  emphasis = false,
}: {
  label: string;
  value: number;
  emphasis?: boolean;
}) {
  return (
    <div className="rounded border border-white/5 bg-black/20 px-2.5 py-1.5">
      <div className="text-[10px] uppercase tracking-wider text-white/40">
        {label}
      </div>
      <div
        className={`tabular-nums ${emphasis ? "text-amber-300" : "text-white/85"}`}
      >
        {value.toLocaleString()}
      </div>
    </div>
  );
}

function AgentPriorityEditor({
  libraryId,
  libraryKind,
  available,
  initial,
  onSaved,
}: {
  libraryId: number;
  libraryKind: string;
  available: AgentInfo[];
  initial: LibraryAgent[];
  onSaved: (next: LibraryAgent[]) => void;
}) {
  const [agents, setAgents] = useState<LibraryAgent[]>(initial);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const dirty = JSON.stringify(agents) !== JSON.stringify(initial);

  function move(idx: number, delta: number) {
    const ni = idx + delta;
    if (ni < 0 || ni >= agents.length) return;
    const next = agents.slice();
    [next[idx], next[ni]] = [next[ni], next[idx]];
    setAgents(next.map((a, i) => ({ ...a, priority: i })));
  }

  function toggle(idx: number) {
    setAgents(
      agents.map((a, i) => (i === idx ? { ...a, enabled: !a.enabled } : a)),
    );
  }

  function add(name: string) {
    if (agents.some((a) => a.agent_name === name)) return;
    setAgents([
      ...agents,
      {
        agent_name: name,
        priority: agents.length,
        enabled: true,
        config_json: "{}",
      },
    ]);
  }

  function remove(idx: number) {
    const next = agents.filter((_, i) => i !== idx).map((a, i) => ({ ...a, priority: i }));
    setAgents(next);
  }

  async function save() {
    setBusy(true);
    setError(null);
    setSaved(false);
    try {
      const r = await adminApi.agents.setForLibrary(libraryId, agents);
      onSaved(r.agents);
      setAgents(r.agents);
      setSaved(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  // Agents not yet added that support this library's item kind.
  const itemKind = libraryKind === "movies" ? "movie" : "show";
  const addable = available.filter(
    (a) =>
      a.supported_kinds.includes(itemKind) &&
      !agents.some((existing) => existing.agent_name === a.name),
  );

  return (
    <div className="space-y-3">
      <ul className="space-y-2">
        {agents.map((a, i) => {
          const info = available.find((x) => x.name === a.agent_name);
          return (
            <li
              key={a.agent_name}
              className="flex items-center gap-3 rounded-md border border-white/10 bg-black/20 px-3 py-2"
            >
              <span className="w-6 text-center font-mono text-xs text-white/40">
                {i + 1}
              </span>
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="font-medium">
                    {info?.display_name ?? a.agent_name}
                  </span>
                  {info && !info.configured && (
                    <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-amber-300">
                      Not configured
                    </span>
                  )}
                </div>
                {info && !info.configured && (
                  <p className="mt-0.5 text-xs text-white/40">
                    Set the required credential (e.g. TMDB_READ_TOKEN) for
                    this agent to do any work.
                  </p>
                )}
              </div>
              <button
                title={a.enabled ? "Disable" : "Enable"}
                onClick={() => toggle(i)}
                className={`rounded border px-2 py-1 text-xs ${a.enabled ? "border-emerald-500/40 text-emerald-300" : "border-white/15 text-white/50"}`}
              >
                {a.enabled ? "Enabled" : "Disabled"}
              </button>
              <button
                disabled={i === 0}
                onClick={() => move(i, -1)}
                className="rounded border border-white/10 px-2 py-1 text-xs text-white/70 hover:bg-white/5 disabled:opacity-30"
              >
                ↑
              </button>
              <button
                disabled={i === agents.length - 1}
                onClick={() => move(i, 1)}
                className="rounded border border-white/10 px-2 py-1 text-xs text-white/70 hover:bg-white/5 disabled:opacity-30"
              >
                ↓
              </button>
              <button
                onClick={() => remove(i)}
                className="rounded border border-white/10 px-2 py-1 text-xs text-white/50 hover:border-red-500/50 hover:text-red-300"
              >
                Remove
              </button>
            </li>
          );
        })}
      </ul>

      {addable.length > 0 && (
        <div className="flex items-center gap-2 text-sm">
          <span className="text-white/50">Add:</span>
          {addable.map((a) => (
            <button
              key={a.name}
              onClick={() => add(a.name)}
              className="rounded border border-white/10 px-2 py-1 text-xs text-white/70 hover:bg-white/5"
            >
              + {a.display_name}
            </button>
          ))}
        </div>
      )}

      <div className="flex items-center gap-3 pt-2">
        <button
          onClick={save}
          disabled={!dirty || busy}
          className="rounded-md bg-red-500 px-4 py-2.5 text-sm font-semibold sm:py-2 text-white hover:bg-red-600 disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/40"
        >
          {busy ? "Saving…" : "Save agent order"}
        </button>
        {saved && !dirty && (
          <span className="text-xs text-white/50">Saved.</span>
        )}
        {error && <span className="text-xs text-red-400">{error}</span>}
        <p className="ml-auto text-xs text-white/40">
          Agents run in listed order; later agents fill nulls without
          overwriting earlier results.
        </p>
      </div>
    </div>
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
      <label className="mb-1 block text-sm font-medium">{label}</label>
      {children}
      {hint && <p className="mt-1 text-xs text-white/50">{hint}</p>}
    </div>
  );
}

function SelectField({
  label,
  hint,
  value,
  onChange,
  options,
}: {
  label: string;
  hint?: string;
  value: string;
  onChange: (v: string) => void;
  options: Array<{ value: string; label: string }>;
}) {
  return (
    <Field label={label} hint={hint}>
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
      >
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    </Field>
  );
}

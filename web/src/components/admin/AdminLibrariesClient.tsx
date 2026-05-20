"use client";

import { useEffect, useMemo, useState } from "react";
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
import {
  Drawer,
  DrawerBody,
  DrawerHeader,
  DrawerKV,
  DrawerSection,
  DrawerTabs,
  Pill,
  SaveBar,
  SettingsCard,
  SettingsRow,
  type DrawerTab,
  type PillTone,
} from "./ui";

interface Props {
  initialLibraries: Library[];
  availableAgents: AgentInfo[];
}

const INPUT_CLASS =
  "w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30";
const INPUT_CHANGED_CLASS =
  "w-full rounded-md border border-amber-400/40 bg-black/30 px-3 py-2 text-sm outline-none focus:border-amber-300";

/// List of library cards on the left; a sticky detail drawer on the
/// right when one is selected. The 1fr+420px grid collapses to a single
/// column on narrow viewports.
export function AdminLibrariesClient({
  initialLibraries,
  availableAgents,
}: Props) {
  const [libraries, setLibraries] = useState(initialLibraries);
  const [selectedId, setSelectedId] = useState<number | null>(
    initialLibraries[0]?.id ?? null,
  );
  const selected = useMemo(
    () => libraries.find((l) => l.id === selectedId) ?? null,
    [libraries, selectedId],
  );

  function onUpdated(next: Library) {
    setLibraries((all) => all.map((l) => (l.id === next.id ? next : l)));
  }

  if (libraries.length === 0) {
    return (
      <div className="rounded-lg border border-dashed border-white/15 bg-white/2 p-8 text-center text-sm text-white/50">
        No libraries yet. Create one from the user-facing Settings page; it
        will appear here automatically.
      </div>
    );
  }

  // Mirror the Users page — only reserve the drawer slot when a
  // library is actually selected. Avoids a dead 420px column when
  // the operator dismisses the drawer.
  const gridClass = selected
    ? "grid grid-cols-1 gap-4 lg:grid-cols-[1fr_420px]"
    : "grid grid-cols-1 gap-4";

  return (
    <div className={gridClass}>
      <ul className="space-y-2 self-start">
        {libraries.map((lib) => (
          <LibraryListCard
            key={lib.id}
            lib={lib}
            selected={selectedId === lib.id}
            onSelect={() => setSelectedId(lib.id)}
          />
        ))}
      </ul>
      {selected && (
        <LibraryDrawer
          // Keyed so per-library state resets when the operator
          // switches between libraries in the list.
          key={selected.id}
          lib={selected}
          availableAgents={availableAgents}
          onUpdated={onUpdated}
          onClose={() => setSelectedId(null)}
        />
      )}
    </div>
  );
}

/// Compact library row used on the left side. Click anywhere on the
/// card to make it the drawer's subject. Shows just enough for triage
/// — full detail lives in the drawer.
function LibraryListCard({
  lib,
  selected,
  onSelect,
}: {
  lib: Library;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <li>
      <button
        type="button"
        onClick={onSelect}
        aria-pressed={selected}
        className={`grid w-full grid-cols-[44px_1fr_auto] items-center gap-3 rounded-lg border px-3 py-3 text-left transition-colors ${
          selected
            ? "border-accent/40 bg-accent/[0.07]"
            : "border-white/10 bg-white/2 hover:border-white/20 hover:bg-white/4"
        }`}
      >
        <span className="grid h-11 w-11 place-items-center rounded-lg border border-white/10 bg-linear-to-br from-white/8 to-black/30 text-white/70">
          <LibraryIcon kind={lib.kind} />
        </span>
        <span className="min-w-0">
          <span className="block text-[14.5px] font-semibold leading-tight">
            {lib.name}
          </span>
          <span className="mt-0.5 block truncate text-[11.5px] text-white/55">
            {lib.kind} · {lib.paths.length} path
            {lib.paths.length === 1 ? "" : "s"}
            {lib.paths.length > 0 && ` · ${lib.paths[0]}`}
          </span>
        </span>
        <Pill tone={visibilityTone(lib.visibility)}>
          {visibilityLabel(lib.visibility)}
        </Pill>
      </button>
    </li>
  );
}

function visibilityTone(v: LibraryVisibility): PillTone {
  if (v === "hidden") return "muted";
  if (v === "search_only") return "warn";
  return "ok";
}
function visibilityLabel(v: LibraryVisibility): string {
  if (v === "home_and_search") return "Visible";
  if (v === "search_only") return "Search only";
  return "Hidden";
}

function LibraryIcon({ kind }: { kind: string }) {
  if (kind === "movies") {
    return (
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
        <rect x="2" y="3" width="20" height="14" rx="2" />
        <line x1="8" y1="21" x2="16" y2="21" />
        <line x1="12" y1="17" x2="12" y2="21" />
      </svg>
    );
  }
  if (kind === "shows" || kind === "tv") {
    return (
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
        <rect x="3" y="4" width="18" height="18" rx="2" />
        <line x1="16" y1="2" x2="16" y2="6" />
        <line x1="8" y1="2" x2="8" y2="6" />
        <line x1="3" y1="10" x2="21" y2="10" />
      </svg>
    );
  }
  if (kind === "anime") {
    return (
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
        <circle cx="12" cy="12" r="10" />
        <path d="M12 6v6l4 2" />
      </svg>
    );
  }
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
      <polygon points="23 7 16 12 23 17 23 7" />
      <rect x="1" y="5" width="15" height="14" rx="2" />
    </svg>
  );
}

// ─── Drawer (per-library detail) ─────────────────────────────────────

const DRAWER_TABS: DrawerTab[] = [
  { id: "overview", label: "Overview" },
  { id: "settings", label: "Settings" },
  { id: "health", label: "Health" },
  { id: "agents", label: "Agents" },
];

function LibraryDrawer({
  lib,
  availableAgents,
  onUpdated,
  onClose,
}: {
  lib: Library;
  availableAgents: AgentInfo[];
  onUpdated: (lib: Library) => void;
  onClose: () => void;
}) {
  const [tab, setTab] = useState("overview");
  const [error, setError] = useState<string | null>(null);
  const [actionMsg, setActionMsg] = useState<string | null>(null);
  const [running, setRunning] = useState<{
    scan?: boolean;
    refreshMeta?: boolean;
    markers?: boolean;
    previews?: boolean;
  }>({});
  const [stats, setStats] = useState<LibraryStatsResult | null>(null);
  const [agents, setAgents] = useState<LibraryAgent[] | null>(null);
  // Health-tab state — `verifyResult` is also surfaced on Overview.
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

  // Settings-tab form state — baseline mirrors the library prop so a
  // save success can update both via `onUpdated`.
  const [baseline, setBaseline] = useState({
    episode_sort_order: lib.episode_sort_order,
    episode_naming: lib.episode_naming,
    certification_country: lib.certification_country,
    visibility: lib.visibility,
    allow_media_deletion: lib.allow_media_deletion ?? false,
  });
  const [sortOrder, setSortOrder] = useState<EpisodeSortOrder>(
    baseline.episode_sort_order,
  );
  const [naming, setNaming] = useState<EpisodeNaming>(baseline.episode_naming);
  const [country, setCountry] = useState(baseline.certification_country);
  const [visibility, setVisibility] = useState<LibraryVisibility>(
    baseline.visibility,
  );
  const [allowMediaDeletion, setAllowMediaDeletion] = useState(
    baseline.allow_media_deletion,
  );

  useEffect(() => {
    librariesApi
      .stats(lib.id)
      .then(setStats)
      .catch(() => {});
  }, [lib.id]);

  useEffect(() => {
    if (tab !== "agents" || agents != null) return;
    adminApi.agents
      .getForLibrary(lib.id)
      .then((r) => setAgents(r.agents))
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, [tab, agents, lib.id]);

  async function refreshStats() {
    try {
      const s = await librariesApi.stats(lib.id);
      setStats(s);
    } catch {
      // best-effort
    }
  }

  // ─── Action handlers ───────────────────────────────────────────
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
          : "Purge orphan files older than the 7-day grace window. Files marked removed today will stay.",
      )
    ) {
      return;
    }
    setPurging(true);
    setError(null);
    try {
      const r = await librariesApi.purge(lib.id, immediate ? 0 : undefined);
      setPurgeResult(r);
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

  // ─── Settings save ─────────────────────────────────────────────
  const dirtyFields: Record<string, boolean> = {
    "Episode sorting": sortOrder !== baseline.episode_sort_order,
    "Episode naming": naming !== baseline.episode_naming,
    "Certification country": country !== baseline.certification_country,
    Visibility: visibility !== baseline.visibility,
    "Allow media deletion":
      allowMediaDeletion !== baseline.allow_media_deletion,
  };
  const dirtyLabels = Object.entries(dirtyFields)
    .filter(([, v]) => v)
    .map(([k]) => k);

  async function saveFields() {
    setError(null);
    const patch: LibraryUpdateInput = {};
    if (dirtyFields["Episode sorting"]) patch.episode_sort_order = sortOrder;
    if (dirtyFields["Episode naming"]) patch.episode_naming = naming;
    if (dirtyFields["Certification country"]) {
      patch.certification_country = country;
    }
    if (dirtyFields.Visibility) patch.visibility = visibility;
    if (dirtyFields["Allow media deletion"]) {
      patch.allow_media_deletion = allowMediaDeletion;
    }
    const { library } = await librariesApi.update(lib.id, patch);
    onUpdated(library);
    setBaseline({
      episode_sort_order: library.episode_sort_order,
      episode_naming: library.episode_naming,
      certification_country: library.certification_country,
      visibility: library.visibility,
      allow_media_deletion: library.allow_media_deletion ?? false,
    });
  }
  function discardFields() {
    setSortOrder(baseline.episode_sort_order);
    setNaming(baseline.episode_naming);
    setCountry(baseline.certification_country);
    setVisibility(baseline.visibility);
    setAllowMediaDeletion(baseline.allow_media_deletion);
  }

  const orphanCount = stats?.orphan_files ?? 0;
  const healthTone: PillTone =
    orphanCount > 0
      ? "warn"
      : verifyResult?.files_missing
        ? "bad"
        : "ok";

  const tabsWithCounts: DrawerTab[] = DRAWER_TABS.map((t) =>
    t.id === "health" && orphanCount > 0 ? { ...t, count: orphanCount } : t,
  );

  return (
    <Drawer>
      <DrawerHeader onClose={onClose}>
        <div className="mb-2 flex items-center gap-3">
          <span className="grid h-9 w-9 shrink-0 place-items-center rounded-md border border-white/10 bg-linear-to-br from-white/8 to-black/30 text-white/70">
            <LibraryIcon kind={lib.kind} />
          </span>
          <div className="min-w-0">
            <div className="truncate text-base font-bold tracking-tight">
              {lib.name}
            </div>
            <div className="truncate text-[11.5px] text-white/50">
              {lib.paths[0] ?? lib.kind}
            </div>
          </div>
        </div>
        <div className="flex flex-wrap gap-1.5">
          <Pill tone={healthTone} dot>
            {healthTone === "ok"
              ? "healthy"
              : orphanCount > 0
                ? `${orphanCount} orphan${orphanCount === 1 ? "" : "s"}`
                : "issues"}
          </Pill>
          <Pill tone={visibilityTone(lib.visibility)}>
            {visibilityLabel(lib.visibility)}
          </Pill>
          <Pill>{lib.kind}</Pill>
        </div>
      </DrawerHeader>

      <DrawerTabs tabs={tabsWithCounts} activeId={tab} onSelect={setTab} />

      <DrawerBody>
        {error && (
          <div className="mb-3 rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
            {error}
          </div>
        )}

        {tab === "overview" && (
          <>
            <DrawerKV
              rows={[
                {
                  label: "Items",
                  value: stats
                    ? `${stats.items.toLocaleString()}${stats.episodes ? ` · ${stats.episodes.toLocaleString()} eps` : ""}`
                    : "—",
                },
                {
                  label: "Files",
                  value: stats ? stats.files.toLocaleString() : "—",
                },
                {
                  label: "Size",
                  value: stats ? formatBytes(stats.total_bytes) : "—",
                },
                {
                  label: "Last scanned",
                  value: stats?.last_scanned_at
                    ? formatRelativeTime(stats.last_scanned_at)
                    : "never",
                },
                {
                  label: "Paths",
                  value:
                    lib.paths.length === 0
                      ? "—"
                      : (
                        <div className="space-y-0.5">
                          {lib.paths.map((p) => (
                            <code
                              key={p}
                              className="block font-mono text-[11.5px] text-white/75"
                            >
                              {p}
                            </code>
                          ))}
                        </div>
                      ),
                },
              ]}
            />

            <DrawerSection title="Quick actions">
              <div className="grid grid-cols-2 gap-2">
                <DrawerAction
                  label="Scan"
                  busy={running.scan}
                  onClick={runScan}
                />
                <DrawerAction
                  label="Refresh metadata"
                  busy={running.refreshMeta}
                  onClick={runRefreshMetadata}
                />
                <DrawerAction
                  label="Detect markers"
                  busy={running.markers}
                  onClick={runDetectMarkers}
                />
                <DrawerAction
                  label="Generate previews"
                  busy={running.previews}
                  onClick={runGeneratePreviews}
                />
              </div>
              {actionMsg && (
                <div className="mt-2 rounded-md border border-emerald-500/30 bg-emerald-500/10 px-3 py-1.5 text-[11.5px] text-emerald-300">
                  {actionMsg}
                </div>
              )}
            </DrawerSection>
          </>
        )}

        {tab === "settings" && (
          <>
            <SettingsCard
              flat
              title="Episode & metadata defaults"
              description="Sorting + naming conventions for new and existing items."
            >
              <SettingsRow
                stacked
                flat
                label="Episode sorting"
                help="Order episodes within a season."
                changed={dirtyFields["Episode sorting"]}
              >
                <select
                  value={sortOrder}
                  onChange={(e) =>
                    setSortOrder(e.target.value as EpisodeSortOrder)
                  }
                  className={
                    dirtyFields["Episode sorting"]
                      ? INPUT_CHANGED_CLASS
                      : INPUT_CLASS
                  }
                >
                  <option value="oldest_first">Oldest first</option>
                  <option value="newest_first">Newest first</option>
                </select>
              </SettingsRow>
              <SettingsRow
                stacked
                flat
                label="Episode naming"
                help="Which source supplies episode titles."
                changed={dirtyFields["Episode naming"]}
              >
                <select
                  value={naming}
                  onChange={(e) => setNaming(e.target.value as EpisodeNaming)}
                  className={
                    dirtyFields["Episode naming"]
                      ? INPUT_CHANGED_CLASS
                      : INPUT_CLASS
                  }
                >
                  <option value="tmdb">The Movie Database</option>
                  <option value="original">Original air name</option>
                  <option value="absolute">Absolute numbering</option>
                </select>
              </SettingsRow>
              <SettingsRow
                stacked
                flat
                label="Certification country"
                help="ISO 3166-1 alpha-2 (e.g. US, GB, DE)."
                changed={dirtyFields["Certification country"]}
              >
                <input
                  type="text"
                  value={country}
                  onChange={(e) =>
                    setCountry(e.target.value.toUpperCase().slice(0, 2))
                  }
                  maxLength={2}
                  className={`w-24 uppercase ${
                    dirtyFields["Certification country"]
                      ? INPUT_CHANGED_CLASS
                      : INPUT_CLASS
                  }`}
                />
              </SettingsRow>
              <SettingsRow
                stacked
                flat
                label="Visibility"
                help="Where this library appears in the UI."
                changed={dirtyFields.Visibility}
              >
                <select
                  value={visibility}
                  onChange={(e) =>
                    setVisibility(e.target.value as LibraryVisibility)
                  }
                  className={
                    dirtyFields.Visibility
                      ? INPUT_CHANGED_CLASS
                      : INPUT_CLASS
                  }
                >
                  <option value="home_and_search">Home &amp; search</option>
                  <option value="search_only">Search only</option>
                  <option value="hidden">Hidden</option>
                </select>
              </SettingsRow>
            </SettingsCard>

            <SettingsCard
              flat
              title="Danger zone"
              description="Toggles that put media on disk at risk if mis-used."
            >
              <SettingsRow
                stacked
                flat
                label="Allow media deletion"
                help="When on, the item detail modal exposes a Delete button that hard-deletes media files and orphan rows immediately — no grace window, no undo."
                changed={dirtyFields["Allow media deletion"]}
              >
                <label className="inline-flex items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={allowMediaDeletion}
                    onChange={(e) => setAllowMediaDeletion(e.target.checked)}
                  />
                  <span>Enable</span>
                </label>
              </SettingsRow>
            </SettingsCard>

            <SaveBar
              dirtyCount={dirtyLabels.length}
              summary={dirtyLabels.slice(0, 3).join(", ") +
                (dirtyLabels.length > 3 ? `, +${dirtyLabels.length - 3} more` : "")}
              onSave={saveFields}
              onDiscard={discardFields}
            />
          </>
        )}

        {tab === "health" && (
          <>
            <p className="mb-3 text-[12px] text-white/55 leading-relaxed">
              Verify checks every file against disk and soft-deletes missing
              ones — watch history is preserved during the grace window. Purge
              hard-deletes soft-deleted rows and cascades to orphan episodes /
              seasons / items.
            </p>
            <div className="flex flex-wrap gap-2">
              <DrawerAction
                label="Verify now"
                busy={verifying}
                busyLabel="Verifying…"
                onClick={runVerify}
              />
              <DrawerAction
                label="Purge expired"
                busy={purging}
                busyLabel="Purging…"
                disabled={orphanCount === 0}
                onClick={() => runPurge(false)}
              />
              <button
                onClick={() => runPurge(true)}
                disabled={purging || orphanCount === 0}
                className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-1.5 text-[12px] font-medium text-red-300 hover:bg-red-500/20 disabled:cursor-not-allowed disabled:opacity-50"
                title="Hard-delete every orphan row now. Skips the grace window."
              >
                Purge all now
              </button>
            </div>
            {verifyResult && (
              <div className="mt-3 grid grid-cols-2 gap-2 text-[11.5px]">
                <HealthStat label="Checked" value={verifyResult.files_checked} />
                <HealthStat
                  label="Missing"
                  value={verifyResult.files_missing}
                  alert={verifyResult.files_missing > 0}
                />
                <HealthStat
                  label="Newly removed"
                  value={verifyResult.newly_marked_removed}
                  alert={verifyResult.newly_marked_removed > 0}
                />
                <HealthStat
                  label="Returned"
                  value={verifyResult.returned_files}
                  alert={verifyResult.returned_files > 0}
                />
              </div>
            )}
            {purgeResult && (
              <div className="mt-3 rounded-md border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-[11.5px] text-emerald-300">
                Purged: {purgeResult.files_purged} files,{" "}
                {purgeResult.episodes_purged} episodes,{" "}
                {purgeResult.seasons_purged} seasons,{" "}
                {purgeResult.items_purged} items.
              </div>
            )}
          </>
        )}

        {tab === "agents" && (
          <>
            <p className="mb-3 text-[12px] text-white/55 leading-relaxed">
              Agents run in listed order; later agents fill nulls without
              overwriting earlier results.
            </p>
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
          </>
        )}
      </DrawerBody>
    </Drawer>
  );
}

function DrawerAction({
  label,
  busyLabel,
  busy,
  disabled,
  onClick,
}: {
  label: string;
  busyLabel?: string;
  busy?: boolean;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={busy || disabled}
      className="rounded-md border border-white/15 bg-white/4 px-3 py-1.5 text-[12px] font-medium text-white/85 hover:bg-white/8 disabled:cursor-not-allowed disabled:opacity-50"
    >
      {busy ? (busyLabel ?? "Working…") : label}
    </button>
  );
}

function HealthStat({
  label,
  value,
  alert = false,
}: {
  label: string;
  value: number;
  alert?: boolean;
}) {
  return (
    <div className="rounded border border-white/10 bg-black/20 px-2.5 py-1.5">
      <div className="text-[10px] uppercase tracking-wider text-white/45">
        {label}
      </div>
      <div
        className={`tabular-nums ${alert ? "text-amber-300" : "text-white/85"}`}
      >
        {value.toLocaleString()}
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

function formatRelativeTime(ms: number): string {
  const diff = Date.now() - ms;
  if (diff < 60_000) return "just now";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  return `${Math.floor(diff / 86_400_000)}d ago`;
}

// ─── Agent priority editor (kept from previous version) ──────────────

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
    const next = agents
      .filter((_, i) => i !== idx)
      .map((a, i) => ({ ...a, priority: i }));
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
              className="flex items-center gap-2 rounded-md border border-white/10 bg-black/20 px-2.5 py-1.5 text-[12px]"
            >
              <span className="w-5 text-center font-mono text-[11px] text-white/40">
                {i + 1}
              </span>
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-1.5">
                  <span className="font-medium">
                    {info?.display_name ?? a.agent_name}
                  </span>
                  {info && !info.configured && (
                    <Pill tone="warn">Not configured</Pill>
                  )}
                </div>
              </div>
              <button
                type="button"
                onClick={() => toggle(i)}
                title={a.enabled ? "Disable" : "Enable"}
                className={`rounded border px-1.5 py-0.5 text-[10px] uppercase tracking-wider ${a.enabled ? "border-emerald-500/40 text-emerald-300" : "border-white/15 text-white/50"}`}
              >
                {a.enabled ? "On" : "Off"}
              </button>
              <button
                type="button"
                disabled={i === 0}
                onClick={() => move(i, -1)}
                className="rounded border border-white/10 px-1.5 py-0.5 text-[11px] text-white/70 hover:bg-white/5 disabled:opacity-30"
              >
                ↑
              </button>
              <button
                type="button"
                disabled={i === agents.length - 1}
                onClick={() => move(i, 1)}
                className="rounded border border-white/10 px-1.5 py-0.5 text-[11px] text-white/70 hover:bg-white/5 disabled:opacity-30"
              >
                ↓
              </button>
              <button
                type="button"
                onClick={() => remove(i)}
                className="rounded border border-white/10 px-1.5 py-0.5 text-[11px] text-white/50 hover:border-red-500/50 hover:text-red-300"
              >
                ×
              </button>
            </li>
          );
        })}
      </ul>

      {addable.length > 0 && (
        <div className="flex flex-wrap items-center gap-1.5 text-[12px]">
          <span className="text-white/50">Add:</span>
          {addable.map((a) => (
            <button
              key={a.name}
              type="button"
              onClick={() => add(a.name)}
              className="rounded border border-white/10 px-2 py-0.5 text-[11px] text-white/70 hover:bg-white/5"
            >
              + {a.display_name}
            </button>
          ))}
        </div>
      )}

      <div className="flex items-center gap-2 pt-1">
        <button
          type="button"
          onClick={save}
          disabled={!dirty || busy}
          className="rounded-md bg-accent px-3 py-1.5 text-[12px] font-medium text-white hover:bg-accent-hover disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/40"
        >
          {busy ? "Saving…" : "Save order"}
        </button>
        {saved && !dirty && (
          <span className="text-[11.5px] text-white/50">Saved.</span>
        )}
        {error && <span className="text-[11.5px] text-red-300">{error}</span>}
      </div>
    </div>
  );
}

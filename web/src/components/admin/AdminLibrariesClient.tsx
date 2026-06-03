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
  type PrimaryMetadataAgent,
  type ScanJob,
} from "@/lib/chimpflix-api";
import { formatDateTime } from "@/lib/format";
import { ConfirmDialog } from "../ConfirmDialog";
import { LoadingPlaceholder } from "../ui/LoadingPlaceholder";

interface Props {
  initialLibraries: Library[];
  availableAgents: AgentInfo[];
}

type PillTone = "ok" | "warn" | "bad" | "muted" | "info";

/// List of libraries on the left (cf-md master list); a sticky cf-drawer
/// detail on the right when one is selected, with inner cf-mtabs.
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
      <div className="cf-card">
        <div className="cf-card-body cf-pad cf-center cf-faint" style={{ fontSize: 13 }}>
          No libraries yet. Create one from the user-facing Settings page; it
          will appear here automatically.
        </div>
      </div>
    );
  }

  return (
    <div className="cf-md">
      <div className="cf-md-list">
        {libraries.map((lib) => (
          <LibraryListItem
            key={lib.id}
            lib={lib}
            selected={selectedId === lib.id}
            onSelect={() => setSelectedId(lib.id)}
          />
        ))}
      </div>
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
/// row to make it the drawer's subject.
function LibraryListItem({
  lib,
  selected,
  onSelect,
}: {
  lib: Library;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-pressed={selected}
      className={`cf-md-item${selected ? " cf-active" : ""}`}
    >
      <span className="cf-md-ico">
        <LibraryIcon kind={lib.kind} />
      </span>
      <div style={{ minWidth: 0 }}>
        <div className="cf-md-name">{lib.name}</div>
        <div className="cf-md-sub">
          {lib.kind} · {lib.paths.length} path
          {lib.paths.length === 1 ? "" : "s"}
          {lib.paths.length > 0 && ` · ${lib.paths[0]}`}
        </div>
      </div>
      <div className="cf-md-trail">
        <PillTag tone={visibilityTone(lib.visibility)} dot>
          {visibilityLabel(lib.visibility)}
        </PillTag>
      </div>
    </button>
  );
}

/// A console pill, inlined so this page no longer depends on the shared
/// ./ui Pill primitive.
function PillTag({
  tone = "muted",
  dot = false,
  children,
}: {
  tone?: PillTone;
  dot?: boolean;
  children: React.ReactNode;
}) {
  const cls =
    tone === "ok"
      ? " cf-ok"
      : tone === "warn"
        ? " cf-warn"
        : tone === "bad"
          ? " cf-err"
          : tone === "info"
            ? " cf-info"
            : "";
  return (
    <span className={`cf-pill${cls}`}>
      {dot && <span className="cf-dot" />}
      {children}
    </span>
  );
}

function visibilityTone(v: LibraryVisibility): PillTone {
  if (v === "hidden") return "info";
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
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
        <rect x="3" y="4" width="18" height="16" rx="2" />
        <path d="M7 4v16M17 4v16" />
      </svg>
    );
  }
  if (kind === "shows" || kind === "tv") {
    return (
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
        <rect x="3" y="4" width="18" height="14" rx="2" />
        <path d="M8 20h8" />
      </svg>
    );
  }
  if (kind === "anime") {
    return (
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
        <path d="M12 3l9 5-9 5-9-5z" />
        <path d="M3 13l9 5 9-5" />
      </svg>
    );
  }
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
      <polygon points="23 7 16 12 23 17 23 7" />
      <rect x="1" y="5" width="15" height="14" rx="2" />
    </svg>
  );
}

// ─── Drawer (per-library detail) ─────────────────────────────────────

const DRAWER_TABS = [
  { id: "settings", label: "Settings" },
  { id: "agents", label: "Agents" },
  { id: "stats", label: "Stats" },
  { id: "scans", label: "Scans" },
  { id: "health", label: "Health" },
] as const;

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
  const [tab, setTab] = useState<string>("settings");
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
  // Scans-tab state — lazily loaded the first time the tab is opened.
  const [scans, setScans] = useState<ScanJob[] | null>(null);
  const [scansLoading, setScansLoading] = useState(false);
  // Health-tab state — `verifyResult` is also surfaced on Stats.
  const [verifyResult, setVerifyResult] = useState<{
    files_checked: number;
    files_missing: number;
    newly_marked_removed: number;
    returned_files: number;
    orphan_count: number;
  } | null>(null);
  const [verifying, setVerifying] = useState(false);
  const [purging, setPurging] = useState(false);
  const [askPurge, setAskPurge] = useState<"grace" | "immediate" | null>(null);
  const [purgeResult, setPurgeResult] = useState<{
    files_purged: number;
    episodes_purged: number;
    seasons_purged: number;
    items_purged: number;
  } | null>(null);

  // Settings-tab form state — baseline mirrors the library prop so a
  // save success can update both via `onUpdated`. Paths are edited as a
  // newline-joined string and split back to an array on save.
  const [baseline, setBaseline] = useState({
    name: lib.name,
    paths: lib.paths.join("\n"),
    episode_sort_order: lib.episode_sort_order,
    episode_naming: lib.episode_naming,
    certification_country: lib.certification_country,
    visibility: lib.visibility,
    allow_media_deletion: lib.allow_media_deletion ?? false,
    primary_metadata_agent: lib.primary_metadata_agent ?? "tmdb",
  });
  const [name, setName] = useState(baseline.name);
  const [pathsText, setPathsText] = useState(baseline.paths);
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
  const [primaryAgent, setPrimaryAgent] = useState<PrimaryMetadataAgent>(
    baseline.primary_metadata_agent,
  );

  // Normalize the textarea into a clean path list: trim each line, drop
  // blanks, dedupe (preserving order). Used both for dirty-detection and
  // the save payload so what the operator sees is what gets persisted.
  const parsedPaths = useMemo(() => {
    const seen = new Set<string>();
    const out: string[] = [];
    for (const raw of pathsText.split("\n")) {
      const p = raw.trim();
      if (p && !seen.has(p)) {
        seen.add(p);
        out.push(p);
      }
    }
    return out;
  }, [pathsText]);

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

  useEffect(() => {
    if (tab !== "scans" || scans != null || scansLoading) return;
    setScansLoading(true);
    librariesApi
      .listScans(lib.id)
      .then((r) => setScans(r.scans))
      .catch((e) => setError(e instanceof Error ? e.message : String(e)))
      .finally(() => setScansLoading(false));
  }, [tab, scans, scansLoading, lib.id]);

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
      // Invalidate the cached scan history so the Scans tab refetches
      // (and surfaces the newly-queued run) the next time it's opened.
      setScans(null);
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
  async function runPurgeConfirmed(immediate: boolean) {
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
      setAskPurge(null);
      void refreshStats(); // sync orphan count / health pill after purge
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPurging(false);
    }
  }

  // ─── Settings save ─────────────────────────────────────────────
  const trimmedName = name.trim();
  const pathsDirty = parsedPaths.join("\n") !== baseline.paths;
  const dirtyFields: Record<string, boolean> = {
    Name: trimmedName !== baseline.name,
    Paths: pathsDirty,
    "Primary metadata source":
      primaryAgent !== baseline.primary_metadata_agent,
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
  // Block save on invalid edits — an empty name or zero paths would be
  // rejected by the backend (paths) or persist a blank label (name).
  const saveBlocked =
    (dirtyFields.Name && trimmedName.length === 0) ||
    (dirtyFields.Paths && parsedPaths.length === 0);

  async function saveFields() {
    setError(null);
    setActionMsg(null);
    if (saveBlocked) {
      setError(
        trimmedName.length === 0
          ? "Library name can't be empty."
          : "A library needs at least one path.",
      );
      return;
    }
    const patch: LibraryUpdateInput = {};
    if (dirtyFields.Name) patch.name = trimmedName;
    if (dirtyFields.Paths) patch.paths = parsedPaths;
    if (dirtyFields["Primary metadata source"]) {
      patch.primary_metadata_agent = primaryAgent;
    }
    if (dirtyFields["Episode sorting"]) patch.episode_sort_order = sortOrder;
    if (dirtyFields["Episode naming"]) patch.episode_naming = naming;
    if (dirtyFields["Certification country"]) {
      patch.certification_country = country;
    }
    if (dirtyFields.Visibility) patch.visibility = visibility;
    if (dirtyFields["Allow media deletion"]) {
      patch.allow_media_deletion = allowMediaDeletion;
    }
    try {
      const library = await librariesApi.update(lib.id, patch);
      onUpdated(library);
      setBaseline({
        name: library.name,
        paths: library.paths.join("\n"),
        episode_sort_order: library.episode_sort_order,
        episode_naming: library.episode_naming,
        certification_country: library.certification_country,
        visibility: library.visibility,
        allow_media_deletion: library.allow_media_deletion ?? false,
        primary_metadata_agent: library.primary_metadata_agent ?? "tmdb",
      });
      setName(library.name);
      setPathsText(library.paths.join("\n"));
      // A path change can leave the index out of step with disk — nudge
      // the operator to re-scan rather than silently kicking one off.
      if (dirtyFields.Paths) {
        setActionMsg(
          "Paths saved. Run a scan to index media under the new paths.",
        );
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }
  function discardFields() {
    setName(baseline.name);
    setPathsText(baseline.paths);
    setSortOrder(baseline.episode_sort_order);
    setNaming(baseline.episode_naming);
    setCountry(baseline.certification_country);
    setVisibility(baseline.visibility);
    setAllowMediaDeletion(baseline.allow_media_deletion);
    setPrimaryAgent(baseline.primary_metadata_agent);
  }

  const orphanCount = stats?.orphan_files ?? 0;
  const healthTone: PillTone =
    orphanCount > 0 ? "warn" : verifyResult?.files_missing ? "bad" : "ok";

  return (
    <div className="cf-drawer">
      <div className="cf-drawer-head">
        <span
          className="cf-md-ico"
          style={{ background: "var(--accent-soft)", color: "#ff7a82" }}
        >
          <LibraryIcon kind={lib.kind} />
        </span>
        <div style={{ minWidth: 0 }}>
          <div className="cf-ttl" style={{ fontSize: 16, fontWeight: 700 }}>
            {lib.name}
          </div>
          <div className="cf-sub">{lib.paths[0] ?? lib.kind}</div>
        </div>
        <button
          type="button"
          className="cf-btn cf-ghost cf-sm"
          onClick={onClose}
          aria-label="Close"
          style={{ marginLeft: "auto" }}
        >
          ✕
        </button>
      </div>

      <div className="cf-drawer-body">
        <div className="cf-flex cf-gap8 cf-wrap" style={{ marginTop: 8 }}>
          <PillTag tone={healthTone} dot>
            {healthTone === "ok"
              ? "healthy"
              : orphanCount > 0
                ? `${orphanCount} orphan${orphanCount === 1 ? "" : "s"}`
                : "issues"}
          </PillTag>
          <PillTag tone={visibilityTone(lib.visibility)}>
            {visibilityLabel(lib.visibility)}
          </PillTag>
          <span className="cf-tag">{lib.kind}</span>
        </div>

        <div className="cf-mtabs" role="tablist">
          {DRAWER_TABS.map((t) => (
            <button
              key={t.id}
              type="button"
              role="tab"
              aria-selected={tab === t.id}
              className={`cf-mtab${tab === t.id ? " cf-on" : ""}`}
              onClick={() => setTab(t.id)}
            >
              {t.label}
              {t.id === "health" && orphanCount > 0 ? ` (${orphanCount})` : ""}
            </button>
          ))}
        </div>

        {error && (
          <div className="cf-banner cf-err">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <circle cx="12" cy="12" r="9" />
              <path d="M12 8v4M12 16v.5" />
            </svg>
            <div>{error}</div>
          </div>
        )}

        {tab === "settings" && (
          <>
            {actionMsg && (
              <div className="cf-banner cf-ok" style={{ marginBottom: 4 }}>
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                  <path d="M20 6L9 17l-5-5" />
                </svg>
                <div>{actionMsg}</div>
              </div>
            )}
            <div className="cf-field">
              <label className="cf-field-label">Name</label>
              <input
                type="text"
                value={name}
                onChange={(e) => setName(e.target.value)}
                className={`cf-input${dirtyFields.Name ? " cf-changed" : ""}`}
                placeholder="Library name"
              />
              {dirtyFields.Name && trimmedName.length === 0 && (
                <p style={{ marginTop: 6, fontSize: 11.5, color: "var(--err)" }}>
                  Name can&apos;t be empty.
                </p>
              )}
            </div>
            <div className="cf-field">
              <label className="cf-field-label">
                Paths · one per line, in-container
              </label>
              <textarea
                value={pathsText}
                onChange={(e) => setPathsText(e.target.value)}
                rows={Math.min(Math.max(parsedPaths.length + 1, 2), 8)}
                spellCheck={false}
                className={`cf-textarea${dirtyFields.Paths ? " cf-changed" : ""}`}
                style={{ lineHeight: 1.6 }}
                placeholder="/media/movies"
              />
              <p className="cf-faint" style={{ marginTop: 6, fontSize: 11.5 }}>
                {dirtyFields.Paths && parsedPaths.length === 0
                  ? "At least one path is required."
                  : dirtyFields.Paths
                    ? "Saving paths won't re-index automatically — run a scan afterward to pick up media under the new paths."
                    : "Container-internal paths (as mounted). Blank lines and duplicates are dropped on save."}
              </p>
            </div>
            <div className="cf-field">
              <label className="cf-field-label">Kind · fixed</label>
              <span className="cf-tag">{lib.kind}</span>
            </div>
            <div className="cf-field">
              <label className="cf-field-label">
                Primary metadata source
              </label>
              <select
                value={primaryAgent}
                onChange={(e) =>
                  setPrimaryAgent(e.target.value as PrimaryMetadataAgent)
                }
                className={`cf-select${
                  dirtyFields["Primary metadata source"] ? " cf-changed" : ""
                }`}
              >
                <option value="tmdb">The Movie Database (TMDB)</option>
                <option value="tvdb">TheTVDB</option>
              </select>
              <p className="cf-faint" style={{ marginTop: 6, fontSize: 11.5 }}>
                Which agent runs first for this library. The other still runs
                after it as a fill-nulls fallback.
              </p>
            </div>
            <div className="cf-field">
              <label className="cf-field-label">Visibility</label>
              <select
                value={visibility}
                onChange={(e) =>
                  setVisibility(e.target.value as LibraryVisibility)
                }
                className={`cf-select${dirtyFields.Visibility ? " cf-changed" : ""}`}
              >
                <option value="home_and_search">Home &amp; search</option>
                <option value="search_only">Search only</option>
                <option value="hidden">Hidden</option>
              </select>
            </div>
            <div className="cf-grid cf-c2">
              <div className="cf-field">
                <label className="cf-field-label">Episode sort order</label>
                <select
                  value={sortOrder}
                  onChange={(e) =>
                    setSortOrder(e.target.value as EpisodeSortOrder)
                  }
                  className={`cf-select${
                    dirtyFields["Episode sorting"] ? " cf-changed" : ""
                  }`}
                >
                  <option value="oldest_first">Oldest first</option>
                  <option value="newest_first">Newest first</option>
                </select>
              </div>
              <div className="cf-field">
                <label className="cf-field-label">Certification</label>
                <input
                  type="text"
                  value={country}
                  onChange={(e) =>
                    setCountry(e.target.value.toUpperCase().slice(0, 2))
                  }
                  maxLength={2}
                  className={`cf-input cf-w-auto${
                    dirtyFields["Certification country"] ? " cf-changed" : ""
                  }`}
                  style={{ minWidth: 0, width: 90, textTransform: "uppercase" }}
                  placeholder="US"
                />
              </div>
            </div>
            <div className="cf-field">
              <label className="cf-field-label">Episode naming</label>
              <select
                value={naming}
                onChange={(e) => setNaming(e.target.value as EpisodeNaming)}
                className={`cf-select${
                  dirtyFields["Episode naming"] ? " cf-changed" : ""
                }`}
              >
                <option value="tmdb">The Movie Database</option>
                <option value="original">Original air name</option>
                <option value="absolute">Absolute numbering</option>
              </select>
            </div>

            <div className="cf-section-title">Danger zone</div>
            <div className="cf-row cf-stack">
              <div className="cf-row-main">
                <div className="cf-row-label">Allow media deletion</div>
                <div className="cf-row-help">
                  When on, the item detail modal exposes a Delete button that
                  hard-deletes media files and orphan rows immediately — no
                  grace window, no undo.
                </div>
              </div>
              <div className="cf-row-control">
                <button
                  type="button"
                  role="switch"
                  aria-checked={allowMediaDeletion}
                  className={`cf-switch${allowMediaDeletion ? " cf-on" : ""}`}
                  onClick={() => setAllowMediaDeletion((v) => !v)}
                />
              </div>
            </div>

            <SaveRow
              dirtyCount={dirtyLabels.length}
              disabled={saveBlocked}
              summary={
                dirtyLabels.slice(0, 3).join(", ") +
                (dirtyLabels.length > 3
                  ? `, +${dirtyLabels.length - 3} more`
                  : "")
              }
              onSave={saveFields}
              onDiscard={discardFields}
            />
          </>
        )}

        {tab === "agents" && (
          <>
            <p
              className="cf-muted"
              style={{ marginTop: 0, fontSize: 12.5 }}
            >
              Priority order for this library. The first that returns a match
              wins; the rest fill gaps without overwriting earlier results.
            </p>
            {agents == null ? (
              <LoadingPlaceholder variant="inline" />
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

        {tab === "stats" && (
          <>
            <div className="cf-grid cf-c2">
              <StatTile
                tone="cf-tone-red"
                label="Items"
                value={stats ? stats.items.toLocaleString() : "—"}
                meta={
                  stats?.episodes
                    ? `${stats.episodes.toLocaleString()} episodes`
                    : undefined
                }
              />
              <StatTile
                tone="cf-tone-blue"
                label="Files"
                value={stats ? stats.files.toLocaleString() : "—"}
                meta={stats ? formatBytes(stats.total_bytes) : undefined}
              />
              <StatTile
                tone="cf-tone-green"
                label="Last scanned"
                value={
                  stats?.last_scanned_at
                    ? formatRelativeTime(stats.last_scanned_at)
                    : "never"
                }
              />
              <StatTile
                tone="cf-tone-amber"
                label="Orphans"
                value={orphanCount.toLocaleString()}
              />
              <StatTile
                tone="cf-tone-blue"
                label="Total runtime"
                value={stats ? formatRuntime(stats.total_runtime_ms) : "—"}
                meta={
                  stats && stats.total_runtime_ms > 0
                    ? `${Math.round(stats.total_runtime_ms / 3_600_000).toLocaleString()} h`
                    : undefined
                }
              />
              <StatTile
                tone="cf-tone-green"
                label="With poster"
                value={
                  stats
                    ? stats.items > 0
                      ? `${Math.round((stats.items_with_poster / stats.items) * 100)}%`
                      : "—"
                    : "—"
                }
                meta={
                  stats
                    ? `${stats.items_with_poster.toLocaleString()} / ${stats.items.toLocaleString()} items`
                    : undefined
                }
              />
              <StatTile
                tone={
                  stats && stats.items_missing_ids > 0
                    ? "cf-tone-amber"
                    : "cf-tone-green"
                }
                label="Missing IDs"
                value={stats ? stats.items_missing_ids.toLocaleString() : "—"}
                meta={
                  stats && stats.items_missing_ids > 0
                    ? "no tmdb / tvdb / imdb"
                    : stats
                      ? "all matched"
                      : undefined
                }
              />
            </div>

            <div className="cf-field" style={{ marginTop: 18 }}>
              <label className="cf-field-label">
                Paths · in-container
              </label>
              {lib.paths.length === 0 ? (
                <span className="cf-faint cf-mono">—</span>
              ) : (
                <div className="cf-stack">
                  {lib.paths.map((p) => (
                    <code
                      key={p}
                      className="cf-mono"
                      style={{
                        display: "block",
                        fontSize: 11.5,
                        color: "var(--muted)",
                      }}
                    >
                      {p}
                    </code>
                  ))}
                </div>
              )}
            </div>

            <div className="cf-section-title">Quick actions</div>
            <div className="cf-flex cf-wrap cf-gap8">
              <ActionButton label="Scan" busy={running.scan} onClick={runScan} />
              <ActionButton
                label="Refresh metadata"
                busy={running.refreshMeta}
                onClick={runRefreshMetadata}
              />
              <ActionButton
                label="Detect markers"
                busy={running.markers}
                onClick={runDetectMarkers}
              />
              <ActionButton
                label="Generate previews"
                busy={running.previews}
                onClick={runGeneratePreviews}
              />
            </div>
            {actionMsg && (
              <div className="cf-banner cf-ok" style={{ marginTop: 12 }}>
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                  <path d="M20 6L9 17l-5-5" />
                </svg>
                <div>{actionMsg}</div>
              </div>
            )}
          </>
        )}

        {tab === "scans" && (
          <>
            <p className="cf-muted" style={{ marginTop: 0, fontSize: 12.5 }}>
              The 50 most recent scan jobs for this library — newest first.
              Deltas are files added / updated / removed during that run.
            </p>
            {scans == null ? (
              <LoadingPlaceholder variant="inline" />
            ) : scans.length === 0 ? (
              <div
                className="cf-faint cf-center"
                style={{ padding: "24px 0", fontSize: 13 }}
              >
                No scans recorded yet.
              </div>
            ) : (
              <div
                className="cf-card"
                style={{ marginTop: 4, overflowX: "auto" }}
              >
                <table className="cf-table">
                  <thead>
                    <tr>
                      <th>Started</th>
                      <th>Status</th>
                      <th>Result</th>
                    </tr>
                  </thead>
                  <tbody>
                    {scans.map((s) => {
                      const startedMs = s.started_at ?? s.created_at;
                      return (
                        <tr key={s.id}>
                          <td
                            className="cf-faint"
                            style={{ whiteSpace: "nowrap" }}
                            title={formatDateTime(startedMs)}
                          >
                            {formatRelativeTime(startedMs)}
                          </td>
                          <td>
                            <ScanStatusPill scan={s} />
                          </td>
                          <td className="cf-faint">
                            <ScanResult scan={s} />
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            )}
          </>
        )}

        {tab === "health" && (
          <>
            <p
              className="cf-muted"
              style={{ marginTop: 0, fontSize: 12.5 }}
            >
              Verify checks every file against disk and soft-deletes missing
              ones — watch history is preserved during the grace window. Purge
              hard-deletes soft-deleted rows and cascades to orphan episodes /
              seasons / items.
            </p>
            <div className="cf-flex cf-wrap cf-gap8">
              <ActionButton
                label="Verify now"
                busyLabel="Verifying…"
                busy={verifying}
                onClick={runVerify}
              />
              <ActionButton
                label="Purge expired"
                busyLabel="Purging…"
                busy={purging}
                disabled={orphanCount === 0}
                onClick={() => setAskPurge("grace")}
              />
              <button
                type="button"
                onClick={() => setAskPurge("immediate")}
                disabled={purging || orphanCount === 0}
                className="cf-btn cf-danger cf-sm"
                title="Hard-delete every orphan row now. Skips the grace window."
              >
                Purge all now
              </button>
            </div>
            {verifyResult && (
              <div className="cf-grid cf-c2" style={{ marginTop: 14 }}>
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
              <div className="cf-banner cf-ok" style={{ marginTop: 14 }}>
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                  <path d="M20 6L9 17l-5-5" />
                </svg>
                <div>
                  Purged: {purgeResult.files_purged} files,{" "}
                  {purgeResult.episodes_purged} episodes,{" "}
                  {purgeResult.seasons_purged} seasons,{" "}
                  {purgeResult.items_purged} items.
                </div>
              </div>
            )}
          </>
        )}
      </div>

      <div className="cf-drawer-foot">
        <button
          type="button"
          className="cf-btn cf-primary cf-sm"
          onClick={saveFields}
          disabled={dirtyLabels.length === 0 || saveBlocked}
        >
          Save
        </button>
        <button
          type="button"
          className="cf-btn cf-ghost cf-sm"
          onClick={runDetectMarkers}
          disabled={running.markers}
        >
          {running.markers ? "Detecting…" : "Detect markers"}
        </button>
      </div>

      {askPurge && (
        <ConfirmDialog
          title={
            askPurge === "immediate"
              ? `Purge every orphan file in "${lib.name}" now?`
              : `Purge orphan files past the 7-day grace?`
          }
          body={
            askPurge === "immediate"
              ? "This hard-deletes every media_file row marked as removed for this library, plus any episodes/seasons/items left without children. Cannot be undone. Files marked today will be wiped — use this only if you're sure the missing files won't come back."
              : "Permanently removes media_files marked as removed more than 7 days ago, plus the episodes/seasons/items they were the last child of. Newly-removed files stay until they age out."
          }
          confirmLabel={
            askPurge === "immediate" ? "Purge all now" : "Purge expired"
          }
          destructive
          busy={purging}
          onConfirm={() => void runPurgeConfirmed(askPurge === "immediate")}
          onCancel={() => setAskPurge(null)}
        />
      )}
    </div>
  );
}

/// Inline drawer save row (replaces the shared SaveBar primitive for this
/// page). Only renders when there are pending edits.
function SaveRow({
  dirtyCount,
  disabled = false,
  summary,
  onSave,
  onDiscard,
}: {
  dirtyCount: number;
  disabled?: boolean;
  summary: string;
  onSave: () => void;
  onDiscard: () => void;
}) {
  if (dirtyCount === 0) return null;
  return (
    <div
      className="cf-flex cf-gap12"
      style={{
        marginTop: 8,
        paddingTop: 14,
        borderTop: "1px solid var(--line)",
      }}
    >
      <span className="cf-sb-status">
        <span
          className="cf-dot"
          style={{ background: "var(--warn)" }}
        />
        {dirtyCount} unsaved {dirtyCount === 1 ? "change" : "changes"} · {summary}
      </span>
      <div className="cf-sb-actions">
        <button type="button" className="cf-btn cf-ghost cf-sm" onClick={onDiscard}>
          Discard
        </button>
        <button
          type="button"
          className="cf-btn cf-primary cf-sm"
          onClick={onSave}
          disabled={disabled}
        >
          Save
        </button>
      </div>
    </div>
  );
}

function ActionButton({
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
      className="cf-btn cf-sm"
    >
      {busy ? (busyLabel ?? "Working…") : label}
    </button>
  );
}

function StatTile({
  tone,
  label,
  value,
  meta,
}: {
  tone: string;
  label: string;
  value: string;
  meta?: string;
}) {
  return (
    <div className={`cf-stat ${tone}`} style={{ padding: 14 }}>
      <div className="cf-stat-top">{label}</div>
      <div className="cf-stat-val" style={{ fontSize: 22 }}>
        {value}
      </div>
      {meta && <div className="cf-stat-meta">{meta}</div>}
    </div>
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
    <div
      style={{
        borderRadius: "var(--r)",
        border: "1px solid var(--line)",
        background: "rgba(255,255,255,0.02)",
        padding: "10px 12px",
      }}
    >
      <div
        className="cf-faint"
        style={{
          fontSize: 10,
          textTransform: "uppercase",
          letterSpacing: "0.08em",
        }}
      >
        {label}
      </div>
      <div
        className="cf-num"
        style={{ color: alert ? "var(--warn)" : "var(--fg)", fontSize: 16 }}
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

/// Human runtime from a total duration in ms — "1,587 h 19 m", or
/// "—" when there's nothing measured.
function formatRuntime(ms: number): string {
  if (ms <= 0) return "—";
  const totalMinutes = Math.floor(ms / 60_000);
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  if (hours === 0) return `${minutes} m`;
  return `${hours.toLocaleString()} h ${minutes} m`;
}

/// Elapsed wall-clock of a scan job. Uses finished−started when both
/// are present; otherwise renders a live-ish status hint.
function scanDuration(s: ScanJob): string {
  if (s.started_at != null && s.finished_at != null) {
    const secs = Math.max(0, Math.round((s.finished_at - s.started_at) / 1000));
    if (secs < 60) return `${secs}s`;
    const m = Math.floor(secs / 60);
    const r = secs % 60;
    return r === 0 ? `${m}m` : `${m}m ${r}s`;
  }
  if (s.status === "running") return "running…";
  if (s.status === "queued") return "queued";
  return "—";
}

function ScanStatusPill({ scan }: { scan: ScanJob }) {
  const tone: PillTone =
    scan.status === "succeeded"
      ? "ok"
      : scan.status === "failed"
        ? "bad"
        : scan.status === "running"
          ? "info"
          : scan.status === "canceled"
            ? "warn"
            : "muted";
  return (
    <PillTag tone={tone} dot>
      {scan.status}
    </PillTag>
  );
}

/// "+a ~u -r" deltas, color-coded; collapses to "no change" when a
/// successful scan touched nothing.
function ScanDeltas({ scan }: { scan: ScanJob }) {
  const { files_added: a, files_updated: u, files_removed: r } = scan;
  if (a === 0 && u === 0 && r === 0) {
    if (scan.status === "succeeded") {
      return <span className="cf-faint">no change</span>;
    }
    return <span className="cf-faint">—</span>;
  }
  return (
    <span className="cf-flex cf-gap8">
      {a > 0 && <span style={{ color: "var(--ok)" }}>+{a}</span>}
      {u > 0 && <span style={{ color: "var(--muted)" }}>~{u}</span>}
      {r > 0 && <span style={{ color: "var(--err)" }}>-{r}</span>}
    </span>
  );
}

/// One compact "Result" cell folding files-seen · duration · deltas onto a
/// single line so the table fits the ~440px library drawer without clipping.
function ScanResult({ scan }: { scan: ScanJob }) {
  const seen = scan.files_seen.toLocaleString();
  const dur = scanDuration(scan);
  return (
    <span
      className="cf-flex cf-gap8"
      style={{ alignItems: "baseline", flexWrap: "wrap", rowGap: 2 }}
    >
      <span>{seen} seen</span>
      {dur !== "—" && (
        <>
          <span className="cf-faint">·</span>
          <span>{dur}</span>
        </>
      )}
      <span className="cf-faint">·</span>
      <ScanDeltas scan={scan} />
    </span>
  );
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
      a.participates_in_chain &&
      !agents.some((existing) => existing.agent_name === a.name),
  );

  return (
    <div className="cf-stack" style={{ gap: 12 }}>
      <div className="cf-md-list" style={{ gap: 6 }}>
        {agents.map((a, i) => {
          const info = available.find((x) => x.name === a.agent_name);
          return (
            <div
              key={a.agent_name}
              className="cf-md-item"
              style={{ padding: "10px 12px", cursor: "default" }}
            >
              <span
                className="cf-num-badge"
                style={
                  i === 0
                    ? {
                        background: "var(--accent-soft)",
                        color: "#ff7a82",
                        borderColor: "var(--accent-line)",
                      }
                    : undefined
                }
              >
                {i + 1}
              </span>
              <div style={{ marginLeft: 10, minWidth: 0 }}>
                <div
                  className="cf-md-name cf-flex cf-wrap cf-gap8"
                  style={{ fontSize: 13 }}
                >
                  <span>{info?.display_name ?? a.agent_name}</span>
                  {info && !info.configured && (
                    <PillTag tone="warn">Not configured</PillTag>
                  )}
                </div>
                {info && <AgentCapabilityBadges info={info} />}
              </div>
              <div className="cf-md-trail">
                <div className="cf-flex cf-gap8">
                  <button
                    type="button"
                    onClick={() => toggle(i)}
                    title={a.enabled ? "Disable" : "Enable"}
                    className={`cf-btn cf-ghost cf-tiny${a.enabled ? "" : " cf-faint"}`}
                  >
                    {a.enabled ? "On" : "Off"}
                  </button>
                  <button
                    type="button"
                    disabled={i === 0}
                    onClick={() => move(i, -1)}
                    className="cf-btn cf-ghost cf-tiny"
                    aria-label="Move up"
                  >
                    ↑
                  </button>
                  <button
                    type="button"
                    disabled={i === agents.length - 1}
                    onClick={() => move(i, 1)}
                    className="cf-btn cf-ghost cf-tiny"
                    aria-label="Move down"
                  >
                    ↓
                  </button>
                  <button
                    type="button"
                    onClick={() => remove(i)}
                    className="cf-btn cf-ghost cf-tiny cf-danger"
                    aria-label="Remove"
                  >
                    ×
                  </button>
                </div>
              </div>
            </div>
          );
        })}
      </div>

      {addable.length > 0 && (
        <div className="cf-flex cf-wrap cf-gap8" style={{ fontSize: 12 }}>
          <span className="cf-faint">Add:</span>
          {addable.map((a) => (
            <button
              key={a.name}
              type="button"
              onClick={() => add(a.name)}
              className="cf-btn cf-ghost cf-tiny"
            >
              + {a.display_name}
            </button>
          ))}
        </div>
      )}

      <div className="cf-flex cf-gap8" style={{ paddingTop: 2 }}>
        <button
          type="button"
          onClick={save}
          disabled={!dirty || busy}
          className="cf-btn cf-primary cf-sm"
        >
          {busy ? "Saving…" : "Save order"}
        </button>
        {saved && !dirty && (
          <span className="cf-faint" style={{ fontSize: 11.5 }}>
            Saved.
          </span>
        )}
        {error && (
          <span style={{ fontSize: 11.5, color: "var(--err)" }}>{error}</span>
        )}
      </div>
    </div>
  );
}

/// Renders a capability matrix as compact badges plus a hover tooltip
/// with provider-specific limitations. Mirrors the
/// `AgentInfo.capabilities` flags one-for-one.
function AgentCapabilityBadges({ info }: { info: AgentInfo }) {
  const caps: Array<[keyof AgentInfo["capabilities"], string]> = [
    ["movie", "movies"],
    ["show", "shows"],
    ["episode", "episodes"],
    ["cast", "cast"],
    ["artwork", "artwork"],
    ["ratings", "ratings"],
  ];
  const active = caps.filter(([k]) => info.capabilities[k]);
  if (active.length === 0 && info.limitations.length === 0) return null;
  return (
    <div
      className="cf-flex cf-wrap"
      style={{ gap: 4, marginTop: 4 }}
    >
      {active.map(([k, label]) => (
        <span
          key={k}
          className="cf-tag"
          style={{ fontSize: 9, padding: "1px 5px", textTransform: "uppercase" }}
        >
          {label}
        </span>
      ))}
      {info.limitations.length > 0 && (
        <span
          title={info.limitations.join("\n\n")}
          className="cf-pill cf-warn"
          style={{ fontSize: 9, padding: "1px 6px", cursor: "help" }}
        >
          info
        </span>
      )}
    </div>
  );
}

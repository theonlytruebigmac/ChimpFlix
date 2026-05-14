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
  const [agents, setAgents] = useState<LibraryAgent[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    if (!expanded || agents != null) return;
    adminApi.agents
      .getForLibrary(lib.id)
      .then((r) => setAgents(r.agents))
      .catch((e) =>
        setError(e instanceof Error ? e.message : String(e)),
      );
  }, [expanded, agents, lib.id]);

  const fieldsDirty =
    sortOrder !== lib.episode_sort_order ||
    naming !== lib.episode_naming ||
    country !== lib.certification_country ||
    visibility !== lib.visibility;

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
      <div className="flex items-center justify-between gap-3 p-4">
        <div className="flex items-center gap-3">
          <span className="text-lg font-semibold">{lib.name}</span>
          <span className="rounded bg-white/10 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-white/60">
            {lib.kind}
          </span>
          <span className="text-xs text-white/40">
            {lib.paths.length} path{lib.paths.length === 1 ? "" : "s"}
          </span>
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
                className="rounded-md bg-red-500 px-4 py-2 text-sm font-semibold text-white hover:bg-red-600 disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/40"
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
        </div>
      )}
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
          className="rounded-md bg-red-500 px-4 py-2 text-sm font-semibold text-white hover:bg-red-600 disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/40"
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

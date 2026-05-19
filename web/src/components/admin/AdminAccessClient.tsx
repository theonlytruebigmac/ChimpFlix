"use client";

import { useMemo, useState } from "react";
import {
  admin as adminApi,
  type AccessMatrixEntry,
} from "@/lib/chimpflix-api";

interface Props {
  initial: AccessMatrixEntry[];
}

// Build a 2D toggle from the flat user × library matrix. Save commits
// per-library `set_library_user_ids` calls under a single bulk request.
export function AdminAccessClient({ initial }: Props) {
  // Baseline tracked in state (rather than reading `initial` directly)
  // so a successful save can update the dirty-check anchor without
  // mutating the prop array, which trips react-hooks/immutability.
  const [baseline, setBaseline] = useState(initial);
  const [entries, setEntries] = useState(initial);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const users = useMemo(() => {
    const seen = new Map<number, string>();
    for (const e of entries) seen.set(e.user_id, e.username);
    return Array.from(seen, ([id, username]) => ({ id, username })).sort((a, b) =>
      a.username.localeCompare(b.username),
    );
  }, [entries]);

  const libraries = useMemo(() => {
    const seen = new Map<number, string>();
    for (const e of entries) seen.set(e.library_id, e.library_name);
    return Array.from(seen, ([id, name]) => ({ id, name })).sort((a, b) =>
      a.name.localeCompare(b.name),
    );
  }, [entries]);

  function cellFor(userId: number, libraryId: number) {
    return entries.find(
      (e) => e.user_id === userId && e.library_id === libraryId,
    );
  }

  function toggle(userId: number, libraryId: number) {
    setEntries((all) =>
      all.map((e) =>
        e.user_id === userId && e.library_id === libraryId
          ? { ...e, allowed: !e.allowed }
          : e,
      ),
    );
  }

  const dirty =
    JSON.stringify(entries.map((e) => ({ u: e.user_id, l: e.library_id, a: e.allowed }))) !==
    JSON.stringify(baseline.map((e) => ({ u: e.user_id, l: e.library_id, a: e.allowed })));

  async function save() {
    setBusy(true);
    setError(null);
    setSaved(false);
    try {
      // For each library, compute the list of allowed user_ids.
      const libraries = new Map<number, number[]>();
      for (const e of entries) {
        if (!libraries.has(e.library_id)) libraries.set(e.library_id, []);
        if (e.allowed) libraries.get(e.library_id)!.push(e.user_id);
      }
      const payload = Array.from(libraries, ([library_id, user_ids]) => ({
        library_id,
        user_ids,
      }));
      const r = await adminApi.access.put(payload);
      setEntries(r.entries);
      // Refresh the baseline so the dirty-check returns false after
      // the save lands.
      setBaseline(r.entries);
      setSaved(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  if (users.length === 0 || libraries.length === 0) {
    return (
      <div className="rounded-lg border border-dashed border-white/15 bg-white/2 p-8 text-center text-sm text-white/50">
        Need at least one non-owner user and one library to manage access.
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {error && (
        <div className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {error}
        </div>
      )}
      <div className="overflow-x-auto rounded-lg border border-white/10">
        <table className="w-full text-sm">
          <thead className="bg-white/5 text-left text-xs uppercase tracking-wider text-white/40">
            <tr>
              <th className="px-4 py-2">User</th>
              {libraries.map((l) => (
                <th key={l.id} className="px-3 py-2 text-center">
                  {l.name}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {users.map((u) => (
              <tr key={u.id} className="border-t border-white/5">
                <td className="whitespace-nowrap px-4 py-2 font-medium">
                  @{u.username}
                </td>
                {libraries.map((l) => {
                  const cell = cellFor(u.id, l.id);
                  const allowed = cell?.allowed ?? false;
                  const viaGroups = cell?.via_groups ?? [];
                  return (
                    <td key={l.id} className="px-3 py-2 text-center align-top">
                      <input
                        type="checkbox"
                        checked={allowed}
                        onChange={() => toggle(u.id, l.id)}
                        className="h-4 w-4"
                        title="Direct grant — toggle to add/remove"
                      />
                      {viaGroups.length > 0 && (
                        <div
                          className="mt-1 text-[10px] uppercase tracking-wider text-emerald-300/70"
                          title={`Also granted via group${viaGroups.length > 1 ? "s" : ""}: ${viaGroups.join(", ")}`}
                        >
                          via {viaGroups.join(", ")}
                        </div>
                      )}
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="flex items-center gap-3">
        <button
          disabled={!dirty || busy}
          onClick={save}
          className="rounded-md bg-red-500 px-4 py-2.5 text-sm font-semibold sm:py-2 text-white hover:bg-red-600 disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/40"
        >
          {busy ? "Saving…" : "Save changes"}
        </button>
        {saved && !dirty && (
          <span className="text-xs text-white/50">Saved.</span>
        )}
      </div>
    </div>
  );
}

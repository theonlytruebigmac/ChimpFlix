"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  bulkItems as bulkApi,
  items as itemsApi,
  type BulkReport,
  type ItemKind,
  type Library,
  type ListedItem,
} from "@/lib/chimpflix-api";

interface Props {
  libraries: Library[];
}

/// Admin item browser with multi-select + bulk action bar.
///
/// Mirrors the existing search/browse UX (library filter, kind
/// filter, search box) but stripped down to the columns admins
/// care about: title / year / kind / library. Selection state is
/// per-page; navigating to a new page clears it (intentional — a
/// 500-item bulk op shouldn't accidentally span pages).
export function AdminBulkItemsClient({ libraries }: Props) {
  const [libraryId, setLibraryId] = useState<number | "all">("all");
  const [kind, setKind] = useState<"all" | ItemKind>("all");
  const [query, setQuery] = useState("");
  const [items, setItems] = useState<ListedItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [report, setReport] = useState<{ kind: string; r: BulkReport } | null>(
    null,
  );
  const [busy, setBusy] = useState<string | null>(null);
  const [tagInput, setTagInput] = useState("");
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const filter: Parameters<typeof itemsApi.list>[0] = { page_size: 50 };
      if (libraryId !== "all") filter.library_id = libraryId;
      if (kind !== "all") filter.kind = kind;
      const q = query.trim();
      if (q.length >= 2) filter.q = q;
      const res = await itemsApi.list(filter);
      setItems(res.items);
      setSelected(new Set());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [libraryId, kind, query]);

  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      void refresh();
    }, 200);
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, [refresh]);

  const allSelected = items.length > 0 && selected.size === items.length;
  const someSelected = selected.size > 0;

  const libNameById = useMemo(() => {
    const m = new Map<number, string>();
    for (const l of libraries) m.set(l.id, l.name);
    return m;
  }, [libraries]);

  function toggleAll() {
    if (allSelected) {
      setSelected(new Set());
    } else {
      setSelected(new Set(items.map((i) => i.id)));
    }
  }

  function toggleOne(id: number) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  async function runOp(opLabel: string, fn: () => Promise<BulkReport>) {
    setBusy(opLabel);
    setError(null);
    setReport(null);
    try {
      const r = await fn();
      setReport({ kind: opLabel, r });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  const ids = Array.from(selected);

  return (
    <div className="space-y-4">
      {error && (
        <div className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {error}
        </div>
      )}
      {report && (
        <div className="rounded-md border border-white/15 bg-white/5 px-3 py-2 text-xs text-white/80">
          <div className="font-semibold">{report.kind}</div>
          <div>
            {report.r.ok} ok · {report.r.failed} failed
          </div>
          {report.r.errors.length > 0 && (
            <details className="mt-1">
              <summary className="cursor-pointer text-white/60">
                Errors ({report.r.errors.length})
              </summary>
              <ul className="mt-1 space-y-0.5">
                {report.r.errors.map((e) => (
                  <li key={e.item_id} className="font-mono text-[10px]">
                    #{e.item_id}: {e.error}
                  </li>
                ))}
              </ul>
            </details>
          )}
        </div>
      )}

      <div className="flex flex-wrap items-end gap-3 rounded-lg border border-white/10 bg-white/2 p-3">
        <div>
          <label className="mb-1 block text-xs text-white/60">Library</label>
          <select
            value={libraryId}
            onChange={(e) =>
              setLibraryId(e.target.value === "all" ? "all" : Number(e.target.value))
            }
            className="rounded-md border border-white/10 bg-black/30 px-3 py-1.5 text-sm"
          >
            <option value="all">All libraries</option>
            {libraries.map((l) => (
              <option key={l.id} value={l.id}>
                {l.name}
              </option>
            ))}
          </select>
        </div>
        <div>
          <label className="mb-1 block text-xs text-white/60">Kind</label>
          <select
            value={kind}
            onChange={(e) => setKind(e.target.value as "all" | ItemKind)}
            className="rounded-md border border-white/10 bg-black/30 px-3 py-1.5 text-sm"
          >
            <option value="all">All</option>
            <option value="movie">Movies</option>
            <option value="show">Shows</option>
          </select>
        </div>
        <div className="grow">
          <label className="mb-1 block text-xs text-white/60">Search</label>
          <input
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Title contains…"
            className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-1.5 text-sm"
          />
        </div>
      </div>

      {someSelected && (
        <div className="sticky top-2 z-10 flex flex-wrap items-center gap-2 rounded-lg border border-red-500/40 bg-neutral-950/95 px-3 py-2 shadow-lg">
          <span className="text-sm font-semibold">{selected.size} selected</span>
          <button
            type="button"
            disabled={busy !== null}
            onClick={() =>
              runOp("Refresh metadata", () => bulkApi.refreshMetadata(ids))
            }
            className="rounded border border-white/15 px-3 py-1 text-xs hover:bg-white/5 disabled:opacity-50"
          >
            {busy === "Refresh metadata" ? "Refreshing…" : "Refresh metadata"}
          </button>
          <button
            type="button"
            disabled={busy !== null}
            onClick={() =>
              runOp("Detect markers", () => bulkApi.detectMarkers(ids))
            }
            className="rounded border border-white/15 px-3 py-1 text-xs hover:bg-white/5 disabled:opacity-50"
          >
            {busy === "Detect markers" ? "Queuing…" : "Detect markers"}
          </button>
          <input
            type="text"
            value={tagInput}
            onChange={(e) => setTagInput(e.target.value)}
            placeholder="Tag name"
            maxLength={64}
            className="w-32 rounded border border-white/10 bg-black/30 px-2 py-1 text-xs"
          />
          <button
            type="button"
            disabled={busy !== null || !tagInput.trim()}
            onClick={() =>
              runOp(`Add tag '${tagInput.trim()}'`, () =>
                bulkApi.addTag(ids, tagInput.trim()),
              )
            }
            className="rounded border border-white/15 px-3 py-1 text-xs hover:bg-white/5 disabled:opacity-50"
          >
            Add tag
          </button>
          <button
            type="button"
            disabled={busy !== null || !tagInput.trim()}
            onClick={() =>
              runOp(`Remove tag '${tagInput.trim()}'`, () =>
                bulkApi.removeTag(ids, tagInput.trim()),
              )
            }
            className="rounded border border-red-500/40 px-3 py-1 text-xs text-red-300 hover:bg-red-500/10 disabled:opacity-50"
          >
            Remove tag
          </button>
        </div>
      )}

      <div className="overflow-hidden rounded-lg border border-white/10">
        <table className="w-full text-sm">
          <thead className="bg-white/5 text-left text-xs uppercase tracking-wider text-white/40">
            <tr>
              <th className="px-3 py-2">
                <input
                  type="checkbox"
                  checked={allSelected}
                  onChange={toggleAll}
                />
              </th>
              <th className="px-3 py-2">Title</th>
              <th className="px-3 py-2">Year</th>
              <th className="px-3 py-2">Kind</th>
              <th className="px-3 py-2">Library</th>
            </tr>
          </thead>
          <tbody>
            {loading ? (
              <tr>
                <td colSpan={5} className="px-3 py-6 text-center text-white/50">
                  Loading…
                </td>
              </tr>
            ) : items.length === 0 ? (
              <tr>
                <td colSpan={5} className="px-3 py-6 text-center text-white/50">
                  No items.
                </td>
              </tr>
            ) : (
              items.map((it) => (
                <tr
                  key={it.id}
                  className={`border-t border-white/5 ${selected.has(it.id) ? "bg-red-500/5" : ""}`}
                >
                  <td className="px-3 py-2">
                    <input
                      type="checkbox"
                      checked={selected.has(it.id)}
                      onChange={() => toggleOne(it.id)}
                    />
                  </td>
                  <td className="px-3 py-2">{it.title}</td>
                  <td className="px-3 py-2 text-white/70">{it.year ?? "—"}</td>
                  <td className="px-3 py-2 text-white/70">{it.kind}</td>
                  <td className="px-3 py-2 text-white/70">
                    {libNameById.get(it.library_id) ?? `#${it.library_id}`}
                  </td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>
      {items.length === 50 && (
        <div className="text-xs text-white/45">
          Showing 50 most recent matches. Narrow your search to bulk-act
          on a smaller set.
        </div>
      )}
    </div>
  );
}

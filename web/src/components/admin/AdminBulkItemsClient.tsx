"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  bulkItems as bulkApi,
  bulkLibrary as bulkLibApi,
  items as itemsApi,
  libraries as librariesApi,
  type BulkReport,
  type ItemKind,
  type Library,
  type LibraryBulkResponse,
  type ListedItem,
} from "@/lib/chimpflix-api";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { LoadingPlaceholder } from "@/components/ui/LoadingPlaceholder";

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
///
/// Restyled to the console `cf-*` design system per
/// docs/redesign/admin-maintenance.html. The mockup reframes Bulk ops
/// as whole-library actions; both layers now ship:
///   * A "Whole-library operations" card (top) acts on an entire chosen
///     library in one pass — mark watched / unwatched (for the acting
///     operator only, Plex semantics), re-scan, and a destructive
///     content delete behind a typed-name confirmation.
///   * The original select-rows-on-page card (bottom) keeps per-item
///     ops (refresh metadata, add/remove tag, detect markers).
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
  const [confirmRemoveTag, setConfirmRemoveTag] = useState(false);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // ── whole-library section state ──────────────────────────────────
  // A concrete library must be picked (the per-library ops need a real
  // id; there's no "all libraries" target). `libItemCount` is fetched
  // for the delete-confirm so the operator sees what they're about to
  // destroy. `confirmName` is the typed-confirmation input.
  const [libOpTarget, setLibOpTarget] = useState<number | "">(
    libraries[0]?.id ?? "",
  );
  const [libItemCount, setLibItemCount] = useState<number | null>(null);
  const [libBusy, setLibBusy] = useState<string | null>(null);
  const [libResult, setLibResult] = useState<LibraryBulkResponse | null>(null);
  const [libError, setLibError] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [confirmName, setConfirmName] = useState("");

  const targetLibrary = useMemo(
    () =>
      libOpTarget === ""
        ? null
        : (libraries.find((l) => l.id === libOpTarget) ?? null),
    [libOpTarget, libraries],
  );

  // Refresh the target library's item count whenever the selection
  // changes — drives the count shown in the delete confirmation.
  useEffect(() => {
    let cancelled = false;
    if (libOpTarget === "") {
      setLibItemCount(null);
      return;
    }
    setLibItemCount(null);
    librariesApi
      .stats(libOpTarget)
      .then((s) => {
        if (!cancelled) setLibItemCount(s.items);
      })
      .catch(() => {
        if (!cancelled) setLibItemCount(null);
      });
    return () => {
      cancelled = true;
    };
  }, [libOpTarget]);

  async function runLibOp(
    opLabel: string,
    fn: () => Promise<LibraryBulkResponse>,
  ) {
    setLibBusy(opLabel);
    setLibError(null);
    setLibResult(null);
    try {
      const r = await fn();
      setLibResult(r);
      // A delete or re-scan changes the catalog — refresh the count and
      // the per-row table below so the operator sees the new state.
      if (r.op === "delete") {
        setLibItemCount(0);
        void refresh();
      }
    } catch (e) {
      setLibError(e instanceof Error ? e.message : String(e));
    } finally {
      setLibBusy(null);
    }
  }

  const deleteNameMatches =
    targetLibrary != null && confirmName.trim() === targetLibrary.name;

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
    <>
      {/* ══ Whole-library operations ══════════════════════════════ */}
      <div className="cf-card" style={{ marginBottom: 16 }}>
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Whole-library operations</div>
            <div className="cf-sub">
              Apply an action across an entire library in one pass. Mark
              watched / unwatched affects only your own play-state.
            </div>
          </div>
        </div>
        <div className="cf-card-body cf-pad">
          {libError && (
            <div
              role="alert"
              aria-live="assertive"
              className="cf-banner cf-err"
            >
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <circle cx="12" cy="12" r="9" />
                <path d="M12 8v4M12 16v.5" />
              </svg>
              <div>{libError}</div>
            </div>
          )}
          {libResult && (
            <div
              role="status"
              aria-live="polite"
              className={
                libResult.op === "delete"
                  ? "cf-banner cf-warn"
                  : "cf-banner cf-info"
              }
            >
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M20 6L9 17l-5-5" />
              </svg>
              <div>{libResult.message}</div>
            </div>
          )}

          <div className="cf-field" style={{ marginBottom: 0 }}>
            <label className="cf-field-label">Library</label>
            <select
              value={libOpTarget}
              onChange={(e) =>
                setLibOpTarget(
                  e.target.value === "" ? "" : Number(e.target.value),
                )
              }
              className="cf-select cf-w-auto"
            >
              {libraries.length === 0 && (
                <option value="">No libraries</option>
              )}
              {libraries.map((l) => (
                <option key={l.id} value={l.id}>
                  {l.name}
                </option>
              ))}
            </select>
          </div>

          {/* Watch state */}
          <div
            className="cf-flex cf-wrap cf-gap8"
            style={{ alignItems: "center", marginTop: 16 }}
          >
            <span style={{ fontWeight: 700, fontSize: 13, minWidth: 110 }}>
              Watch state
            </span>
            <button
              type="button"
              disabled={libBusy !== null || targetLibrary == null}
              onClick={() =>
                runLibOp("Mark watched", () =>
                  bulkLibApi.markWatched(libOpTarget as number),
                )
              }
              className="cf-btn cf-sm"
            >
              {libBusy === "Mark watched" ? "Marking…" : "Mark watched"}
            </button>
            <button
              type="button"
              disabled={libBusy !== null || targetLibrary == null}
              onClick={() =>
                runLibOp("Mark unwatched", () =>
                  bulkLibApi.markUnwatched(libOpTarget as number),
                )
              }
              className="cf-btn cf-sm"
            >
              {libBusy === "Mark unwatched" ? "Marking…" : "Mark unwatched"}
            </button>
          </div>

          {/* Re-scan */}
          <div
            className="cf-flex cf-wrap cf-gap8"
            style={{ alignItems: "center", marginTop: 12 }}
          >
            <span style={{ fontWeight: 700, fontSize: 13, minWidth: 110 }}>
              Scan
            </span>
            <button
              type="button"
              disabled={libBusy !== null || targetLibrary == null}
              onClick={() =>
                runLibOp("Re-scan", () =>
                  bulkLibApi.rescan(libOpTarget as number),
                )
              }
              className="cf-btn cf-sm"
            >
              {libBusy === "Re-scan" ? "Queuing…" : "Re-scan"}
            </button>
          </div>

          {/* Danger zone */}
          <div
            className="cf-flex cf-wrap cf-gap8"
            style={{ alignItems: "center", marginTop: 12 }}
          >
            <span style={{ fontWeight: 700, fontSize: 13, minWidth: 110 }}>
              Danger zone
            </span>
            <button
              type="button"
              disabled={libBusy !== null || targetLibrary == null}
              onClick={() => {
                setConfirmName("");
                setConfirmDelete(true);
              }}
              className="cf-btn cf-danger cf-sm"
            >
              Delete all items
            </button>
            {targetLibrary != null && (
              <span className="cf-faint" style={{ fontSize: 12 }}>
                Removes every item and its files/episodes from “
                {targetLibrary.name}”. Files on disk are untouched; the library
                itself remains.
              </span>
            )}
          </div>
        </div>
      </div>

      {/* ══ Per-item (select rows on this page) ═══════════════════ */}
      <div className="cf-card" style={{ marginBottom: 0 }}>
      <div className="cf-card-head">
        <div>
          <div className="cf-ttl">Bulk operations</div>
          <div className="cf-sub">
            Filter the catalog, select rows on this page, then apply an action
            to the selection. Selection is per-page — a new search clears it.
          </div>
        </div>
      </div>
      <div className="cf-card-body cf-pad">
        {error && (
          <div role="alert" aria-live="assertive" className="cf-banner cf-err">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <circle cx="12" cy="12" r="9" />
              <path d="M12 8v4M12 16v.5" />
            </svg>
            <div>{error}</div>
          </div>
        )}
        {report && (
          <div role="status" aria-live="polite" className="cf-banner cf-info">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M20 6L9 17l-5-5" />
            </svg>
            <div>
              <b>{report.kind}</b> — {report.r.ok} ok · {report.r.failed} failed
              {report.r.errors.length > 0 && (
                <details style={{ marginTop: 4 }}>
                  <summary style={{ cursor: "pointer" }} className="cf-muted">
                    Errors ({report.r.errors.length})
                  </summary>
                  <ul style={{ marginTop: 4 }}>
                    {report.r.errors.map((e) => (
                      <li
                        key={e.item_id}
                        className="cf-mono"
                        style={{ fontSize: 10 }}
                      >
                        #{e.item_id}: {e.error}
                      </li>
                    ))}
                  </ul>
                </details>
              )}
            </div>
          </div>
        )}

        {/* ── filters ───────────────────────────────────────────────── */}
        <div className="cf-flex cf-wrap cf-gap12" style={{ alignItems: "flex-end" }}>
          <div className="cf-field" style={{ marginBottom: 0 }}>
            <label className="cf-field-label">Library</label>
            <select
              value={libraryId}
              onChange={(e) =>
                setLibraryId(
                  e.target.value === "all" ? "all" : Number(e.target.value),
                )
              }
              className="cf-select cf-w-auto"
            >
              <option value="all">All libraries</option>
              {libraries.map((l) => (
                <option key={l.id} value={l.id}>
                  {l.name}
                </option>
              ))}
            </select>
          </div>
          <div className="cf-field" style={{ marginBottom: 0 }}>
            <label className="cf-field-label">Kind</label>
            <select
              value={kind}
              onChange={(e) => setKind(e.target.value as "all" | ItemKind)}
              className="cf-select cf-w-auto"
            >
              <option value="all">All</option>
              <option value="movie">Movies</option>
              <option value="show">Shows</option>
            </select>
          </div>
          <div className="cf-field" style={{ marginBottom: 0, flex: 1, minWidth: 200 }}>
            <label className="cf-field-label">Search</label>
            <input
              type="search"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Title contains…"
              className="cf-input"
            />
          </div>
        </div>

        {/* ── selection action bar ──────────────────────────────────── */}
        {someSelected && (
          <div
            className="cf-flex cf-wrap cf-gap8"
            style={{
              position: "sticky",
              top: 8,
              zIndex: 10,
              marginTop: 16,
              padding: "10px 12px",
              borderRadius: "var(--r)",
              border: "1px solid var(--accent-line)",
              background: "rgba(20,20,20,0.95)",
              boxShadow: "var(--shadow-pop)",
            }}
          >
            <span style={{ fontWeight: 700, fontSize: 13 }}>
              {selected.size} selected
            </span>
            <button
              type="button"
              disabled={busy !== null}
              onClick={() =>
                runOp("Refresh metadata", () => bulkApi.refreshMetadata(ids))
              }
              className="cf-btn cf-sm"
            >
              {busy === "Refresh metadata" ? "Refreshing…" : "Refresh metadata"}
            </button>
            <button
              type="button"
              disabled={busy !== null}
              onClick={() =>
                runOp("Detect markers", () => bulkApi.detectMarkers(ids))
              }
              className="cf-btn cf-sm"
            >
              {busy === "Detect markers" ? "Queuing…" : "Detect markers"}
            </button>
            <input
              type="text"
              value={tagInput}
              onChange={(e) => setTagInput(e.target.value)}
              placeholder="Tag name"
              maxLength={64}
              className="cf-input"
              style={{ width: 128 }}
            />
            <button
              type="button"
              disabled={busy !== null || !tagInput.trim()}
              onClick={() =>
                runOp(`Add tag '${tagInput.trim()}'`, () =>
                  bulkApi.addTag(ids, tagInput.trim()),
                )
              }
              className="cf-btn cf-sm"
            >
              Add tag
            </button>
            <button
              type="button"
              disabled={busy !== null || !tagInput.trim()}
              onClick={() => setConfirmRemoveTag(true)}
              className="cf-btn cf-danger cf-sm"
            >
              Remove tag
            </button>
          </div>
        )}

        {/* ── results table ─────────────────────────────────────────── */}
        <div style={{ marginTop: 16 }}>
          <table className="cf-table">
            <thead>
              <tr>
                <th style={{ width: 36 }}>
                  <input
                    type="checkbox"
                    checked={allSelected}
                    onChange={toggleAll}
                  />
                </th>
                <th>Title</th>
                <th>Year</th>
                <th>Kind</th>
                <th>Library</th>
              </tr>
            </thead>
            <tbody>
              {loading ? (
                <tr>
                  <td colSpan={5}>
                    <LoadingPlaceholder />
                  </td>
                </tr>
              ) : items.length === 0 ? (
                <tr>
                  <td colSpan={5} className="cf-center cf-faint">
                    No items.
                  </td>
                </tr>
              ) : (
                items.map((it) => (
                  <tr
                    key={it.id}
                    style={
                      selected.has(it.id)
                        ? { background: "var(--accent-soft)" }
                        : undefined
                    }
                  >
                    <td>
                      <input
                        type="checkbox"
                        checked={selected.has(it.id)}
                        onChange={() => toggleOne(it.id)}
                      />
                    </td>
                    <td>{it.title}</td>
                    <td className="cf-muted">{it.year ?? "—"}</td>
                    <td className="cf-muted">{it.kind}</td>
                    <td className="cf-muted">
                      {libNameById.get(it.library_id) ?? `#${it.library_id}`}
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
        {items.length === 50 && (
          <div className="cf-faint" style={{ fontSize: 12, marginTop: 12 }}>
            Showing 50 most recent matches. Narrow your search to bulk-act on a
            smaller set.
          </div>
        )}
        </div>
      </div>

      {confirmRemoveTag && (
        <ConfirmDialog
          title="Remove tag from selected items?"
          body={
            <>
              Remove the tag <strong>{tagInput.trim()}</strong> from{" "}
              <strong>{selected.size}</strong>{" "}
              {selected.size === 1 ? "item" : "items"}. This can be undone by
              re-adding the tag.
            </>
          }
          confirmLabel="Remove tag"
          destructive
          busy={busy !== null}
          onConfirm={async () => {
            const label = `Remove tag '${tagInput.trim()}'`;
            await runOp(label, () => bulkApi.removeTag(ids, tagInput.trim()));
            setConfirmRemoveTag(false);
          }}
          onCancel={() => setConfirmRemoveTag(false)}
        />
      )}

      {confirmDelete && targetLibrary != null && (
        <ConfirmDialog
          title={`Delete all items in “${targetLibrary.name}”?`}
          body={
            <div>
              <p>
                This permanently removes{" "}
                <strong>
                  {libItemCount == null ? "all" : libItemCount.toLocaleString()}
                </strong>{" "}
                item{libItemCount === 1 ? "" : "s"} and every season, episode,
                and media-row beneath them. Files on disk are{" "}
                <strong>not</strong> deleted, and the library itself is kept —
                a re-scan would re-add anything still present on disk.
              </p>
              <p style={{ marginTop: 10 }}>
                Type the library name <strong>{targetLibrary.name}</strong> to
                confirm:
              </p>
              <input
                type="text"
                value={confirmName}
                onChange={(e) => setConfirmName(e.target.value)}
                placeholder={targetLibrary.name}
                autoFocus
                className="cf-input"
                style={{ marginTop: 6, width: "100%" }}
              />
              {!deleteNameMatches && confirmName.length > 0 && (
                <div
                  className="cf-faint"
                  style={{ fontSize: 12, marginTop: 4 }}
                >
                  Name doesn’t match yet.
                </div>
              )}
            </div>
          }
          confirmLabel="Delete all items"
          destructive
          busy={libBusy !== null}
          // Block confirm until the typed name matches (no spinner —
          // it's a precondition, not an in-flight request). The backend
          // independently enforces the same contract (confirm_library_id
          // + confirm_name), so this is purely a UX guard.
          confirmDisabled={!deleteNameMatches}
          onConfirm={async () => {
            if (!deleteNameMatches) return;
            const libId = targetLibrary.id;
            await runLibOp("Delete", () =>
              bulkLibApi.deleteContent(libId, libId, confirmName.trim()),
            );
            setConfirmDelete(false);
            setConfirmName("");
          }}
          onCancel={() => {
            setConfirmDelete(false);
            setConfirmName("");
          }}
        />
      )}
    </>
  );
}

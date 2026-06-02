"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { SmartRuleBuilder } from "@/components/admin/SmartRuleBuilder";
import {
  ChimpFlixApiError,
  collections as collectionsApi,
  items as itemsApi,
  type Collection,
  type CollectionDetail,
  type ListedItem,
} from "@/lib/chimpflix-api";
import { ConfirmDialog } from "../ConfirmDialog";

interface Props {
  initial: Collection[];
}

/// Admin surface for manual + auto collections. Auto rows render with a
/// read-only badge — only manual collections are editable.
///
/// Layout: header with "+ New collection", grouped sections for Manual
/// and Auto, each row expandable to a detail panel that loads its items
/// on demand and exposes add/remove/reorder controls (manual only).
export function AdminCollectionsClient({ initial }: Props) {
  const [collections, setCollections] = useState(initial);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [expandedId, setExpandedId] = useState<number | null>(null);

  const manual = collections.filter((c) => c.kind === "manual");
  const smart = collections.filter((c) => c.kind === "smart");
  const auto = collections.filter((c) => c.kind === "auto");
  const [smartCreating, setSmartCreating] = useState(false);

  const refresh = useCallback(async () => {
    try {
      // Match the SSR fetch in app/.../collections/page.tsx — admin
      // sees every kind, including auto franchises.
      const r = await collectionsApi.list({ include_auto: true });
      setCollections(r.collections);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  const total = collections.length;

  return (
    <div>
      {error && (
        <div className="cf-banner cf-err">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 8v4M12 16v.5" />
          </svg>
          <div>{error}</div>
        </div>
      )}

      <div className="cf-flex cf-between cf-wrap cf-gap12" style={{ marginBottom: 14 }}>
        <div className="cf-muted" style={{ fontSize: 13 }}>
          <b style={{ color: "#fff" }}>
            {total} collection{total === 1 ? "" : "s"}
          </b>{" "}
          · {manual.length} manual · {smart.length} smart · {auto.length} auto
        </div>
        <div className="cf-flex cf-gap8">
          <button
            type="button"
            onClick={() => setSmartCreating((v) => !v)}
            className="cf-btn cf-sm"
          >
            {smartCreating ? "Cancel" : "New smart"}
          </button>
          <button
            type="button"
            onClick={() => setCreating((v) => !v)}
            className="cf-btn cf-primary cf-sm"
          >
            {creating ? "Cancel" : "New manual"}
          </button>
        </div>
      </div>

      {creating && (
        <NewCollectionForm
          onCreated={async () => {
            setCreating(false);
            await refresh();
          }}
          onError={setError}
        />
      )}

      {smartCreating && (
        <NewSmartCollectionForm
          onCreated={async () => {
            setSmartCreating(false);
            await refresh();
          }}
          onError={setError}
        />
      )}

      <Section
        title="Smart"
        description="Built from query rules — update themselves on scan."
        emptyText="No smart collections yet. Rules return up to 500 items each."
      >
        {smart.map((c) => (
          <CollectionRow
            key={c.id}
            collection={c}
            expanded={expandedId === c.id}
            onToggle={() =>
              setExpandedId((id) => (id === c.id ? null : c.id))
            }
            onChanged={refresh}
            onError={setError}
          />
        ))}
      </Section>

      <Section
        title="Manual"
        description="Hand-curated rows."
        emptyText="No manual collections yet."
      >
        {manual.map((c) => (
          <CollectionRow
            key={c.id}
            collection={c}
            expanded={expandedId === c.id}
            onToggle={() =>
              setExpandedId((id) => (id === c.id ? null : c.id))
            }
            onChanged={refresh}
            onError={setError}
          />
        ))}
      </Section>

      <Section
        title="Auto · TMDB franchises"
        description="Discovered automatically from movie metadata."
        emptyText="No auto collections — none of your movies belong to a TMDB collection yet."
        last
      >
        {auto.map((c) => (
          <CollectionRow
            key={c.id}
            collection={c}
            expanded={expandedId === c.id}
            onToggle={() =>
              setExpandedId((id) => (id === c.id ? null : c.id))
            }
            onChanged={refresh}
            onError={setError}
          />
        ))}
      </Section>
    </div>
  );
}

// ─── Subcomponents ──────────────────────────────────────────────────────

function Section({
  title,
  description,
  emptyText,
  last = false,
  children,
}: {
  title: string;
  description: string;
  emptyText: string;
  last?: boolean;
  children: React.ReactNode;
}) {
  const hasChildren = Array.isArray(children) && children.length > 0;
  return (
    <div className="cf-card" style={last ? { marginBottom: 0 } : undefined}>
      <div className="cf-card-head">
        <div>
          <div className="cf-ttl">{title}</div>
          <div className="cf-sub">{description}</div>
        </div>
      </div>
      {hasChildren ? (
        <table className="cf-table">
          <tbody>{children}</tbody>
        </table>
      ) : (
        <div className="cf-card-body cf-pad cf-faint cf-center" style={{ fontSize: 13 }}>
          {emptyText}
        </div>
      )}
    </div>
  );
}

function CollectionRow({
  collection,
  expanded,
  onToggle,
  onChanged,
  onError,
}: {
  collection: Collection;
  expanded: boolean;
  onToggle: () => void;
  onChanged: () => Promise<void>;
  onError: (e: string) => void;
}) {
  const [askDelete, setAskDelete] = useState(false);
  const [deleteBusy, setDeleteBusy] = useState(false);

  function remove() {
    // Trigger the inline confirm; actual deletion happens in
    // `confirmDelete` after the operator clicks through the dialog.
    setAskDelete(true);
  }

  async function confirmDelete() {
    setDeleteBusy(true);
    try {
      await collectionsApi.delete(collection.id);
      await onChanged();
      setAskDelete(false);
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setDeleteBusy(false);
    }
  }

  // Second cell mirrors the mockup: a rule snippet for smart, the
  // discovery source for auto, "Curated by you" for manual.
  const subText =
    collection.kind === "smart"
      ? (collection.rule_json ?? "rule")
      : collection.kind === "auto"
        ? "themoviedb.org"
        : (collection.description ?? "Curated by you");

  return (
    <>
      <tr>
        <td>
          <button
            type="button"
            onClick={onToggle}
            className="cf-btn cf-ghost cf-tiny"
            style={{ padding: 0, fontWeight: 700 }}
          >
            <b>{collection.name}</b>
          </button>
        </td>
        <td className="cf-muted" style={{ maxWidth: 360 }}>
          <span
            style={{
              display: "block",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
            title={subText}
          >
            {subText}
          </span>
        </td>
        <td className="cf-num cf-muted">
          {collection.item_count} title{collection.item_count === 1 ? "" : "s"}
        </td>
        <td className="cf-num">
          <div
            className="cf-flex cf-gap8"
            style={{ justifyContent: "flex-end" }}
          >
            <button
              type="button"
              onClick={onToggle}
              className="cf-btn cf-ghost cf-tiny"
            >
              {expanded ? "Hide" : collection.kind === "auto" ? "View" : "Edit"}
            </button>
            {collection.kind === "manual" && (
              <button
                type="button"
                onClick={remove}
                className="cf-btn cf-ghost cf-tiny cf-danger"
              >
                Delete
              </button>
            )}
          </div>
        </td>
      </tr>
      {expanded && (
        <tr>
          <td colSpan={4} style={{ background: "rgba(255,255,255,0.02)" }}>
            {collection.kind === "manual" ? (
              <ManualCollectionDetail
                collection={collection}
                onChanged={onChanged}
                onError={onError}
                onDelete={remove}
              />
            ) : collection.kind === "smart" ? (
              <SmartCollectionDetail
                collection={collection}
                onChanged={onChanged}
                onError={onError}
                onDelete={remove}
              />
            ) : (
              <AutoCollectionDetail collection={collection} onError={onError} />
            )}
          </td>
        </tr>
      )}
      {askDelete && (
        // Portal-rendered (escapes the table), so no wrapper row is
        // needed — it produces no in-table DOM.
        <ConfirmDialog
          title={`Delete collection "${collection.name}"?`}
          body="Member items remain in your library; only the grouping is removed."
          confirmLabel="Delete"
          destructive
          busy={deleteBusy}
          onConfirm={() => void confirmDelete()}
          onCancel={() => setAskDelete(false)}
        />
      )}
    </>
  );
}

function AutoCollectionDetail({
  collection,
  onError,
}: {
  collection: Collection;
  onError: (e: string) => void;
}) {
  const [detail, setDetail] = useState<CollectionDetail | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    collectionsApi
      .get(collection.id)
      .then((d) => {
        if (cancelled) return;
        setDetail(d);
      })
      .catch((e) => {
        if (cancelled) return;
        onError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [collection.id, onError]);

  return (
    <div className="space-y-2">
      <p className="text-xs text-white/55">
        TMDB-discovered franchise. Membership tracks{" "}
        <code className="rounded bg-white/10 px-1 py-0.5">items.collection_id</code>{" "}
        from the scan; members can&rsquo;t be added or removed by hand.
        Refresh metadata on the movies to update.
      </p>
      {loading ? (
        <div className="text-xs text-white/50">Loading members…</div>
      ) : detail && detail.items.length > 0 ? (
        <ul className="grid grid-cols-1 gap-1 sm:grid-cols-2">
          {detail.items.map((it) => (
            <li
              key={it.id}
              className="truncate rounded bg-white/5 px-2 py-1 text-xs text-white/75"
            >
              {it.title}
              {it.year ? (
                <span className="text-white/45"> ({it.year})</span>
              ) : null}
            </li>
          ))}
        </ul>
      ) : (
        <div className="text-xs text-white/50">No accessible members.</div>
      )}
    </div>
  );
}

function ManualCollectionDetail({
  collection,
  onChanged,
  onError,
  onDelete,
}: {
  collection: Collection;
  onChanged: () => Promise<void>;
  onError: (e: string) => void;
  onDelete: () => void;
}) {
  const [detail, setDetail] = useState<CollectionDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState(false);

  const loadDetail = useCallback(async () => {
    try {
      const d = await collectionsApi.get(collection.id);
      setDetail(d);
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [collection.id, onError]);

  useEffect(() => {
    let cancelled = false;
    collectionsApi
      .get(collection.id)
      .then((d) => {
        if (cancelled) return;
        setDetail(d);
      })
      .catch((e) => {
        if (cancelled) return;
        onError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [collection.id, onError]);

  async function reload() {
    await Promise.all([loadDetail(), onChanged()]);
  }

  async function addItems(itemIds: number[]) {
    try {
      await collectionsApi.addItems(collection.id, itemIds);
      await reload();
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    }
  }

  async function removeItem(itemId: number) {
    try {
      await collectionsApi.removeItem(collection.id, itemId);
      await reload();
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    }
  }

  async function move(itemId: number, direction: -1 | 1) {
    if (!detail) return;
    const ids = detail.items.map((i) => i.id);
    const idx = ids.indexOf(itemId);
    const dest = idx + direction;
    if (idx < 0 || dest < 0 || dest >= ids.length) return;
    [ids[idx], ids[dest]] = [ids[dest], ids[idx]];
    try {
      await collectionsApi.reorder(collection.id, ids);
      await reload();
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <div className="text-xs text-white/55">
          {detail?.description ?? collection.description ?? (
            <em className="text-white/40">No description.</em>
          )}
        </div>
        <div className="flex gap-2">
          <button
            type="button"
            onClick={() => setEditing((v) => !v)}
            className="rounded border border-white/15 px-2 py-1 text-xs text-white/80 hover:bg-white/5"
          >
            {editing ? "Cancel edit" : "Edit details"}
          </button>
          <button
            type="button"
            onClick={onDelete}
            className="rounded border border-red-500/40 px-2 py-1 text-xs text-red-300 hover:bg-red-500/10"
          >
            Delete collection
          </button>
        </div>
      </div>

      {editing && (
        <EditCollectionForm
          collection={collection}
          onSaved={async () => {
            setEditing(false);
            await reload();
          }}
          onError={onError}
        />
      )}

      <ArtUploaders
        collection={collection}
        currentPoster={detail?.poster_path ?? collection.poster_path}
        currentBackdrop={detail?.backdrop_path ?? collection.backdrop_path}
        onUploaded={reload}
        onError={onError}
      />

      {loading ? (
        <div className="text-xs text-white/50">Loading members…</div>
      ) : (
        <>
          <ItemPicker
            existingIds={new Set(detail?.items.map((i) => i.id) ?? [])}
            onAdd={addItems}
            onError={onError}
          />

          {detail && detail.items.length > 0 ? (
            <ul className="divide-y divide-white/5 rounded border border-white/10">
              {detail.items.map((it, idx) => (
                <li
                  key={it.id}
                  className="flex items-center gap-3 px-3 py-2"
                >
                  <span className="w-6 shrink-0 text-right text-xs tabular-nums text-white/40">
                    {idx + 1}
                  </span>
                  <span className="grow truncate text-sm">
                    {it.title}
                    {it.year && (
                      <span className="ml-1 text-white/45">
                        ({it.year})
                      </span>
                    )}
                  </span>
                  <button
                    type="button"
                    onClick={() => move(it.id, -1)}
                    disabled={idx === 0}
                    className="rounded border border-white/15 px-2 py-0.5 text-xs text-white/70 hover:bg-white/5 disabled:cursor-not-allowed disabled:opacity-30"
                    aria-label="Move up"
                  >
                    ↑
                  </button>
                  <button
                    type="button"
                    onClick={() => move(it.id, 1)}
                    disabled={idx === detail.items.length - 1}
                    className="rounded border border-white/15 px-2 py-0.5 text-xs text-white/70 hover:bg-white/5 disabled:cursor-not-allowed disabled:opacity-30"
                    aria-label="Move down"
                  >
                    ↓
                  </button>
                  <button
                    type="button"
                    onClick={() => removeItem(it.id)}
                    className="rounded border border-red-500/40 px-2 py-0.5 text-xs text-red-300 hover:bg-red-500/10"
                  >
                    Remove
                  </button>
                </li>
              ))}
            </ul>
          ) : (
            <div className="rounded border border-dashed border-white/15 bg-white/2 px-3 py-4 text-center text-xs text-white/50">
              No items yet. Use the search above to add some.
            </div>
          )}
        </>
      )}
    </div>
  );
}

/// Smart-collection detail panel: read-only members preview from the
/// live rule + JSON rule editor. The compiled SQL caps to 500 rows on
/// the backend so big libraries don't choke when we expand the row.
function SmartCollectionDetail({
  collection,
  onChanged,
  onError,
  onDelete,
}: {
  collection: Collection;
  onChanged: () => Promise<void>;
  onError: (e: string) => void;
  onDelete: () => void;
}) {
  const [detail, setDetail] = useState<CollectionDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState(false);
  const [ruleDraft, setRuleDraft] = useState(collection.rule_json ?? "");
  const [saving, setSaving] = useState(false);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      const d = await collectionsApi.get(collection.id);
      setDetail(d);
      setRuleDraft(d.rule_json ?? "");
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [collection.id, onError]);

  useEffect(() => {
    let cancelled = false;
    collectionsApi
      .get(collection.id)
      .then((d) => {
        if (cancelled) return;
        setDetail(d);
        setRuleDraft(d.rule_json ?? "");
      })
      .catch((e) => {
        if (cancelled) return;
        onError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [collection.id, onError]);

  async function saveRule() {
    setSaving(true);
    try {
      // Parse before sending so the user gets a clear error in the UI
      // rather than waiting for the server roundtrip.
      JSON.parse(ruleDraft);
      await collectionsApi.updateSmartRule(collection.id, ruleDraft);
      setEditing(false);
      await Promise.all([reload(), onChanged()]);
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <div className="text-xs text-white/55">
          {collection.description ?? (
            <em className="text-white/40">No description.</em>
          )}
        </div>
        <div className="flex gap-2">
          <button
            type="button"
            onClick={() => setEditing((v) => !v)}
            className="rounded border border-white/15 px-2 py-1 text-xs text-white/80 hover:bg-white/5"
          >
            {editing ? "Cancel edit" : "Edit rule"}
          </button>
          <button
            type="button"
            onClick={onDelete}
            className="rounded border border-red-500/40 px-2 py-1 text-xs text-red-300 hover:bg-red-500/10"
          >
            Delete collection
          </button>
        </div>
      </div>

      {editing ? (
        <div className="space-y-2">
          <SmartRuleBuilder
            initialJson={collection.rule_json ?? ""}
            onChange={setRuleDraft}
          />
          <button
            type="button"
            onClick={saveRule}
            disabled={saving}
            className="rounded-md bg-red-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-red-600 disabled:opacity-50"
          >
            {saving ? "Saving…" : "Save rule"}
          </button>
        </div>
      ) : (
        <pre className="overflow-x-auto rounded border border-white/10 bg-black/40 p-3 font-mono text-[11px] text-white/70">
          {collection.rule_json ?? "(no rule)"}
        </pre>
      )}

      {loading ? (
        <div className="text-xs text-white/50">Loading members…</div>
      ) : detail && detail.items.length > 0 ? (
        <div>
          <div className="mb-2 text-xs font-semibold uppercase tracking-wider text-white/45">
            Matches ({detail.items.length}, capped at 500)
          </div>
          <ul className="grid grid-cols-1 gap-1 sm:grid-cols-2">
            {detail.items.map((it) => (
              <li
                key={it.id}
                className="truncate rounded bg-white/5 px-2 py-1 text-xs text-white/75"
              >
                {it.title}
                {it.year ? <span className="text-white/45"> ({it.year})</span> : null}
              </li>
            ))}
          </ul>
        </div>
      ) : (
        <div className="rounded border border-dashed border-white/15 bg-white/2 px-3 py-4 text-center text-xs text-white/50">
          Rule matched no items in your library.
        </div>
      )}
    </div>
  );
}

function NewSmartCollectionForm({
  onCreated,
  onError,
}: {
  onCreated: () => Promise<void>;
  onError: (e: string) => void;
}) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [ruleJson, setRuleJson] = useState(
    JSON.stringify(
      {
        operator: "and",
        conditions: [
          { field: "kind", op: "eq", value: "movie" },
          { field: "year", op: "ge", value: 2020 },
        ],
      },
      null,
      2,
    ),
  );
  const [saving, setSaving] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!name.trim() || saving) return;
    setSaving(true);
    try {
      JSON.parse(ruleJson); // surfaces parse errors before the network hop
      await collectionsApi.createSmart({
        name: name.trim(),
        description: description.trim() || null,
        rule_json: ruleJson,
      });
      setName("");
      setDescription("");
      await onCreated();
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <form onSubmit={submit} className="cf-card">
      <div className="cf-card-head">
        <div>
          <div className="cf-ttl">New smart collection</div>
          <div className="cf-sub">
            Members are computed from a query rule and refresh on each scan.
          </div>
        </div>
      </div>
      <div className="cf-card-body cf-pad">
        <div className="cf-field">
          <label className="cf-field-label">Name</label>
          <input
            autoFocus
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            maxLength={200}
            placeholder="e.g. 2020s Action Movies"
            className="cf-input"
            required
          />
        </div>
        <div className="cf-field">
          <label className="cf-field-label">Description (optional)</label>
          <textarea
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            maxLength={4000}
            rows={2}
            className="cf-textarea"
          />
        </div>
        <div className="cf-field">
          <label className="cf-field-label">Rule</label>
          <SmartRuleBuilder initialJson={ruleJson} onChange={setRuleJson} />
        </div>
        <button
          type="submit"
          disabled={!name.trim() || saving}
          className="cf-btn cf-primary cf-sm"
        >
          {saving ? "Creating…" : "Create smart collection"}
        </button>
      </div>
    </form>
  );
}

function NewCollectionForm({
  onCreated,
  onError,
}: {
  onCreated: () => Promise<void>;
  onError: (e: string) => void;
}) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [saving, setSaving] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!name.trim() || saving) return;
    setSaving(true);
    try {
      await collectionsApi.create({
        name: name.trim(),
        description: description.trim() || null,
      });
      setName("");
      setDescription("");
      await onCreated();
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <form onSubmit={submit} className="cf-card">
      <div className="cf-card-head">
        <div>
          <div className="cf-ttl">New manual collection</div>
          <div className="cf-sub">A hand-curated row you fill with items.</div>
        </div>
      </div>
      <div className="cf-card-body cf-pad">
        <div className="cf-field">
          <label className="cf-field-label">Name</label>
          <input
            autoFocus
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            maxLength={200}
            placeholder="e.g. Sunday Night Sci-Fi"
            className="cf-input"
            required
          />
        </div>
        <div className="cf-field">
          <label className="cf-field-label">Description (optional)</label>
          <textarea
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            maxLength={4000}
            rows={2}
            className="cf-textarea"
          />
        </div>
        <button
          type="submit"
          disabled={!name.trim() || saving}
          className="cf-btn cf-primary cf-sm"
        >
          {saving ? "Creating…" : "Create collection"}
        </button>
      </div>
    </form>
  );
}

function EditCollectionForm({
  collection,
  onSaved,
  onError,
}: {
  collection: Collection;
  onSaved: () => Promise<void>;
  onError: (e: string) => void;
}) {
  const [name, setName] = useState(collection.name);
  const [description, setDescription] = useState(collection.description ?? "");
  const [sortTitle, setSortTitle] = useState(collection.sort_title ?? "");
  const [saving, setSaving] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setSaving(true);
    try {
      await collectionsApi.update(collection.id, {
        name: name.trim(),
        description: description.trim() || null,
        sort_title: sortTitle.trim() || null,
      });
      await onSaved();
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <form
      onSubmit={submit}
      className="rounded border border-white/10 bg-black/30 p-3 space-y-2"
    >
      <div className="grid gap-2 sm:grid-cols-2">
        <div>
          <label className="mb-1 block text-xs font-medium text-white/80">
            Name
          </label>
          <input
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            maxLength={200}
            className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-1.5 text-sm outline-none focus:border-white/30"
            required
          />
        </div>
        <div>
          <label className="mb-1 block text-xs font-medium text-white/80">
            Sort title <span className="text-white/40">(optional)</span>
          </label>
          <input
            type="text"
            value={sortTitle}
            onChange={(e) => setSortTitle(e.target.value)}
            maxLength={200}
            placeholder="Defaults to name"
            className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-1.5 text-sm outline-none focus:border-white/30"
          />
        </div>
      </div>
      <div>
        <label className="mb-1 block text-xs font-medium text-white/80">
          Description
        </label>
        <textarea
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          maxLength={4000}
          rows={2}
          className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-1.5 text-sm outline-none focus:border-white/30"
        />
      </div>
      <button
        type="submit"
        disabled={!name.trim() || saving}
        className="rounded-md bg-red-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-red-600 disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/40"
      >
        {saving ? "Saving…" : "Save"}
      </button>
    </form>
  );
}

/// Side-by-side poster (2:3) and backdrop (16:9) upload widgets for a
/// manual collection. Empty state shows a dashed placeholder + file
/// picker; populated state shows the current image with a Replace
/// button. Files are constrained client-side to image/* but the server
/// re-validates content-type + 8 MiB cap before writing.
function ArtUploaders({
  collection,
  currentPoster,
  currentBackdrop,
  onUploaded,
  onError,
}: {
  collection: Collection;
  currentPoster: string | null | undefined;
  currentBackdrop: string | null | undefined;
  onUploaded: () => Promise<void>;
  onError: (e: string) => void;
}) {
  return (
    <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
      <ArtTile
        kind="poster"
        currentUrl={currentPoster}
        aspect="aspect-[2/3]"
        upload={(f) => collectionsApi.uploadPoster(collection.id, f)}
        onUploaded={onUploaded}
        onError={onError}
      />
      <ArtTile
        kind="backdrop"
        currentUrl={currentBackdrop}
        aspect="aspect-video"
        upload={(f) => collectionsApi.uploadBackdrop(collection.id, f)}
        onUploaded={onUploaded}
        onError={onError}
      />
    </div>
  );
}

function ArtTile({
  kind,
  currentUrl,
  aspect,
  upload,
  onUploaded,
  onError,
}: {
  kind: "poster" | "backdrop";
  currentUrl: string | null | undefined;
  aspect: string;
  upload: (file: File) => Promise<void>;
  onUploaded: () => Promise<void>;
  onError: (e: string) => void;
}) {
  const [busy, setBusy] = useState(false);

  async function pick(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    e.target.value = "";
    if (!file) return;
    setBusy(true);
    try {
      await upload(file);
      await onUploaded();
    } catch (err) {
      onError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div>
      <div className="mb-1 text-xs font-medium text-white/80 capitalize">{kind}</div>
      <label
        className={`relative block ${aspect} cursor-pointer overflow-hidden rounded border border-white/15 bg-black/30 transition-colors hover:border-white/30`}
      >
        {currentUrl ? (
          // eslint-disable-next-line @next/next/no-img-element
          <img
            src={currentUrl}
            alt={`${kind} preview`}
            className="h-full w-full object-cover"
          />
        ) : (
          <div className="flex h-full w-full flex-col items-center justify-center text-xs text-white/45">
            <span>+ Upload {kind}</span>
            <span className="mt-0.5 text-white/30">JPEG / PNG / WebP, ≤ 8 MiB</span>
          </div>
        )}
        {busy && (
          <div className="absolute inset-0 flex items-center justify-center bg-black/60 text-xs text-white">
            Uploading…
          </div>
        )}
        <input
          type="file"
          accept="image/jpeg,image/png,image/webp"
          className="absolute inset-0 cursor-pointer opacity-0"
          onChange={pick}
          disabled={busy}
        />
      </label>
    </div>
  );
}

function ItemPicker({
  existingIds,
  onAdd,
  onError,
}: {
  existingIds: Set<number>;
  onAdd: (ids: number[]) => Promise<void>;
  onError: (e: string) => void;
}) {
  const [q, setQ] = useState("");
  // Results are paired with the query they were fetched for, so a
  // stale debounce can't show results from a previous search after the
  // user has retyped. The render gate `q.trim() === results.forQuery`
  // doubles as the "don't show pre-typing data" guard, which keeps the
  // short-q case from needing a synchronous setState in the effect
  // (which trips react-hooks/set-state-in-effect).
  const [results, setResults] = useState<{ forQuery: string; items: ListedItem[] }>({
    forQuery: "",
    items: [],
  });
  const [loading, setLoading] = useState(false);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    const trimmed = q.trim();
    if (trimmed.length < 2) {
      return;
    }
    debounceRef.current = setTimeout(async () => {
      setLoading(true);
      try {
        const r = await itemsApi.list({ q: trimmed, page_size: 20 });
        setResults({ forQuery: trimmed, items: r.items });
      } catch (e) {
        if (e instanceof ChimpFlixApiError) {
          onError(e.message);
        } else {
          onError(e instanceof Error ? e.message : String(e));
        }
      } finally {
        setLoading(false);
      }
    }, 220);
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, [q, onError]);

  const trimmedQ = q.trim();
  const showResults = trimmedQ.length >= 2;
  const matchesCurrentQuery = results.forQuery === trimmedQ;

  return (
    <div className="space-y-2">
      <input
        type="search"
        value={q}
        onChange={(e) => setQ(e.target.value)}
        placeholder="Search to add items…"
        className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-1.5 text-sm outline-none focus:border-white/30"
      />
      {showResults && (
        <div className="rounded border border-white/10 bg-black/30 max-h-60 overflow-y-auto">
          {loading || !matchesCurrentQuery ? (
            <div className="px-3 py-2 text-xs text-white/50">Searching…</div>
          ) : results.items.length === 0 ? (
            <div className="px-3 py-2 text-xs text-white/50">No matches.</div>
          ) : (
            <ul className="divide-y divide-white/5">
              {results.items.map((it) => {
                const already = existingIds.has(it.id);
                return (
                  <li
                    key={it.id}
                    className="flex items-center gap-2 px-3 py-1.5"
                  >
                    <span className="grow truncate text-xs">
                      {it.title}
                      {it.year && (
                        <span className="ml-1 text-white/45">
                          ({it.year})
                        </span>
                      )}
                    </span>
                    <button
                      type="button"
                      onClick={() => onAdd([it.id])}
                      disabled={already}
                      className="rounded border border-white/15 px-2 py-0.5 text-xs text-white/80 hover:bg-white/5 disabled:cursor-not-allowed disabled:opacity-40"
                    >
                      {already ? "In collection" : "Add"}
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}

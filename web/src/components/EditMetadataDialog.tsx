"use client";

import { useRouter } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  items as itemsApi,
  readCsrfToken,
  type Credit,
  type CreditEditInput,
  type ItemDetail,
  type ItemEditInput,
  type TmdbPoster,
} from "@/lib/chimpflix-api";
import { plexImage } from "@/lib/image";
import { useFocusTrap } from "@/lib/use-focus-trap";

// All fields the user can edit on the General tab. Each row has an
// optional "lock" indicator showing whether re-enrichment will overwrite
// the value. Saving any field locks it; the user can explicitly unlock
// via the small unlock icon.
const FIELDS: ReadonlyArray<{
  key: keyof ItemEditInput;
  label: string;
  kind: "text" | "textarea" | "number";
}> = [
  { key: "title", label: "Title", kind: "text" },
  { key: "sort_title", label: "Sort Title", kind: "text" },
  { key: "original_title", label: "Original Title", kind: "text" },
  { key: "tagline", label: "Tagline", kind: "text" },
  { key: "summary", label: "Summary", kind: "textarea" },
  { key: "year", label: "Year", kind: "number" },
  { key: "rating_age", label: "Content Rating", kind: "text" },
  { key: "rating_audience", label: "Rating (0-10)", kind: "number" },
];

type Tab = "general" | "credits" | "posters";

export function EditMetadataDialog({
  detail,
  onClose,
  onSaved,
}: {
  detail: ItemDetail;
  onClose: () => void;
  onSaved: (next: ItemDetail) => void;
}) {
  const router = useRouter();
  const [tab, setTab] = useState<Tab>("general");
  // Hold edited values as strings so empty / partial input renders cleanly.
  const [values, setValues] = useState<Record<string, string>>(() => ({
    title: detail.title ?? "",
    sort_title: detail.sort_title ?? "",
    original_title: detail.original_title ?? "",
    tagline: detail.tagline ?? "",
    summary: detail.summary ?? "",
    year: detail.year != null ? String(detail.year) : "",
    rating_age: "",
    rating_audience:
      detail.rating_audience != null ? String(detail.rating_audience) : "",
  }));
  const [unlock, setUnlock] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [posterUploading, setPosterUploading] = useState(false);
  const [posterUrl, setPosterUrl] = useState<string | null>(detail.poster_path);
  const posterInput = useRef<HTMLInputElement>(null);

  // Credits-tab state: editable copy of detail.credits.
  const [credits, setCredits] = useState<EditableCredit[]>(() =>
    (detail.credits ?? []).map(creditToEditable),
  );

  // Posters-tab state: lazy-loaded TMDB poster candidates.
  const [posters, setPosters] = useState<TmdbPoster[] | null>(null);
  const [postersLoading, setPostersLoading] = useState(false);
  const [postersError, setPostersError] = useState<string | null>(null);
  // Most recently applied TMDB poster (within this dialog session) so we
  // can show a "Active" checkmark in the grid. The server doesn't store
  // the original TMDB file_path, so this only persists for the open modal.
  const [appliedPosterPath, setAppliedPosterPath] = useState<string | null>(
    null,
  );
  const [applyingPosterPath, setApplyingPosterPath] = useState<string | null>(
    null,
  );

  // Fetch poster candidates the first time the user opens the Posters tab.
  // Synchronous setState before the await is the standard "show loading
  // skeleton, then resolve" pattern; the lint rule's preferred alternative
  // (useEffectEvent or external store) would be heavier than the case warrants.
  // Cancellation guard prevents setting state after unmount.
  useEffect(() => {
    if (tab !== "posters" || posters !== null || postersLoading) return;
    let cancelled = false;
    /* eslint-disable react-hooks/set-state-in-effect */
    setPostersLoading(true);
    setPostersError(null);
    /* eslint-enable react-hooks/set-state-in-effect */
    itemsApi
      .tmdbPosters(detail.id)
      .then((r) => {
        if (!cancelled) setPosters(r.posters);
      })
      .catch((e) => {
        if (!cancelled) {
          setPostersError(e instanceof Error ? e.message : String(e));
        }
      })
      .finally(() => {
        if (!cancelled) setPostersLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [tab, detail.id, posters, postersLoading]);

  async function applyTmdbPoster(p: TmdbPoster) {
    setBusy(true);
    setApplyingPosterPath(p.file_path);
    setError(null);
    try {
      const next = await itemsApi.applyTmdbPoster(detail.id, p.file_path);
      // The server already cache-busts the stored URL; we still append our
      // own suffix here for the in-modal preview state, which mirrors the
      // upload flow.
      setPosterUrl(
        next.poster_path
          ? next.poster_path +
              (next.poster_path.includes("?") ? "&" : "?") +
              "t=" +
              Date.now()
          : null,
      );
      setAppliedPosterPath(p.file_path);
      onSaved(next);
      // The picked poster is now in the DB with a new versioned URL, but
      // the underlying page (rails, home dashboard) was server-rendered
      // with the previous URL. Re-fetch the route's data so every consumer
      // re-renders with the fresh poster.
      router.refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
      setApplyingPosterPath(null);
    }
  }

  async function uploadPoster(file: File) {
    setPosterUploading(true);
    setError(null);
    try {
      const fd = new FormData();
      fd.append("file", file);
      // Multipart uploads can't go through apiFetch (it always
      // JSON-stringifies the body) so we use raw fetch — but we still
      // need to attach the CSRF token, otherwise the server's csrf
      // middleware rejects the mutating request with 403. The cookie
      // gets sent automatically via credentials:"include"; the header
      // is the double-submit half.
      const csrf = readCsrfToken();
      const headers: Record<string, string> = {};
      if (csrf) headers["X-CSRF-Token"] = csrf;
      const res = await fetch(`/api/v1/items/${detail.id}/poster`, {
        method: "POST",
        body: fd,
        credentials: "include",
        headers,
      });
      if (!res.ok) {
        const text = await res.text().catch(() => "");
        throw new Error(text || `upload failed: ${res.status}`);
      }
      const next: ItemDetail = await res.json();
      setPosterUrl(
        next.poster_path
          ? next.poster_path + (next.poster_path.includes("?") ? "&" : "?") + "v=" + Date.now()
          : null,
      );
      onSaved(next);
      router.refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPosterUploading(false);
    }
  }

  // Escape + Tab focus cycling + restore-focus-on-close. The shared
  // hook replaces the prior `keydown`-only listener so this dialog
  // now traps keyboard focus correctly (was a WCAG dialog gap).
  const dialogRef = useRef<HTMLDivElement>(null);
  const titleId = "edit-metadata-dialog-title";
  useFocusTrap(dialogRef, { onClose });

  function dirty(field: keyof ItemEditInput): boolean {
    const original = (() => {
      switch (field) {
        case "title":
          return detail.title ?? "";
        case "sort_title":
          return detail.sort_title ?? "";
        case "original_title":
          return detail.original_title ?? "";
        case "tagline":
          return detail.tagline ?? "";
        case "summary":
          return detail.summary ?? "";
        case "year":
          return detail.year != null ? String(detail.year) : "";
        case "rating_audience":
          return detail.rating_audience != null
            ? String(detail.rating_audience)
            : "";
        default:
          return "";
      }
    })();
    return values[field as string] !== original;
  }

  async function saveGeneral() {
    if (busy) return;
    setBusy(true);
    setError(null);
    const patch: ItemEditInput = {};
    for (const f of FIELDS) {
      if (!dirty(f.key)) continue;
      const raw = values[f.key as string];
      if (f.kind === "number") {
        if (raw.trim() === "") continue;
        const n = Number.parseFloat(raw);
        if (!Number.isFinite(n)) continue;
        (patch as Record<string, unknown>)[f.key as string] = n;
      } else {
        (patch as Record<string, unknown>)[f.key as string] = raw;
      }
    }
    if (unlock.size > 0) patch.unlock = Array.from(unlock);
    try {
      const next = await itemsApi.patch(detail.id, patch);
      onSaved(next);
      router.refresh();
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function saveCredits() {
    if (busy) return;
    setBusy(true);
    setError(null);
    // Re-sort by current array order; users drag rows to reorder.
    const payload: CreditEditInput[] = credits
      .filter((c) => c.name.trim() !== "")
      .map((c, i) => ({
        person_id: c.person_id ?? undefined,
        name: c.name.trim(),
        role_kind: c.role_kind,
        role: c.role.trim() || roleDefault(c.role_kind),
        character_name: c.character_name?.trim() || undefined,
        sort_order: i,
        photo_url: c.photo_url ?? undefined,
      }));
    try {
      const next = await itemsApi.patchCredits(detail.id, payload);
      onSaved(next);
      setCredits((next.credits ?? []).map(creditToEditable));
      router.refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  // Portal to document.body so the dialog escapes the TitleModalShell's
  // `.zf-modal-in` ancestor. That ancestor has a `transform` animation,
  // which establishes a new containing block for `position: fixed`
  // descendants — without the portal, this dialog opens "inset-0"
  // relative to the (possibly-scrolled) modal card, landing wherever
  // that card happens to be on screen. The portal renders directly
  // under <body> so `fixed inset-0` is finally viewport-relative.
  if (typeof document === "undefined") return null;
  return createPortal(
    <div
      className="fixed inset-0 z-[60] flex items-center justify-center bg-black/70 p-4 zf-modal-backdrop"
      onClick={onClose}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="zf-modal-in w-full max-w-3xl overflow-hidden rounded-lg border border-white/10 bg-(--color-surface) shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-white/10 px-6 py-4">
          <h2 id={titleId} className="text-lg font-semibold">
            Edit Metadata
          </h2>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="text-white/60 transition-colors hover:text-white"
          >
            ✕
          </button>
        </div>

        <div className="flex gap-1 border-b border-white/10 px-4 pt-2">
          <TabButton active={tab === "general"} onClick={() => setTab("general")}>
            General
          </TabButton>
          <TabButton active={tab === "credits"} onClick={() => setTab("credits")}>
            Cast &amp; Crew
            <span className="ml-1 text-xs text-white/40">
              {credits.length}
            </span>
          </TabButton>
          <TabButton active={tab === "posters"} onClick={() => setTab("posters")}>
            Posters
          </TabButton>
        </div>

        <div className="max-h-[70vh] space-y-4 overflow-y-auto px-6 py-5">
          {tab === "general" && (
            <>
              <div className="mb-2 flex items-start gap-4 rounded border border-white/5 bg-black/20 p-3">
                <div className="h-28 w-20 shrink-0 overflow-hidden rounded bg-black/40">
                  {posterUrl ? (
                    // eslint-disable-next-line @next/next/no-img-element
                    <img
                      src={plexImage(posterUrl, 120, 180) ?? ""}
                      alt="Current poster"
                      className="h-full w-full object-cover"
                    />
                  ) : (
                    <div className="flex h-full w-full items-center justify-center text-xs text-white/30">
                      No poster
                    </div>
                  )}
                </div>
                <div className="flex-1">
                  <div className="text-xs text-white/60">Poster</div>
                  <p className="mt-1 text-xs text-white/40">
                    Upload JPEG, PNG, or WebP. Max 8 MB. Or pick from TMDB on
                    the Posters tab. Either choice locks the poster.
                  </p>
                  <div className="mt-2 flex items-center gap-2">
                    <input
                      ref={posterInput}
                      type="file"
                      accept="image/jpeg,image/png,image/webp"
                      className="hidden"
                      onChange={(e) => {
                        const f = e.target.files?.[0];
                        if (f) uploadPoster(f);
                        if (posterInput.current) posterInput.current.value = "";
                      }}
                    />
                    <button
                      type="button"
                      disabled={posterUploading}
                      onClick={() => posterInput.current?.click()}
                      className="rounded border border-white/20 px-3 py-1.5 text-xs text-white transition-colors hover:border-white disabled:opacity-50"
                    >
                      {posterUploading ? "Uploading…" : "Upload poster…"}
                    </button>
                  </div>
                </div>
              </div>
              {FIELDS.map((f) => {
                const locked = detail.locked_fields.includes(f.key as string);
                const willUnlock = unlock.has(f.key as string);
                return (
                  <div key={f.key as string}>
                    <div className="mb-1 flex items-center gap-2 text-xs">
                      <span className="text-white/60">{f.label}</span>
                      {locked && !willUnlock && (
                        <button
                          type="button"
                          onClick={() =>
                            setUnlock((prev) => {
                              const next = new Set(prev);
                              next.add(f.key as string);
                              return next;
                            })
                          }
                          title="Locked — click to allow auto-refresh to overwrite"
                          className="rounded bg-white/10 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-white/85 transition-colors hover:bg-white/20"
                        >
                          🔒 locked
                        </button>
                      )}
                      {willUnlock && (
                        <button
                          type="button"
                          onClick={() =>
                            setUnlock((prev) => {
                              const next = new Set(prev);
                              next.delete(f.key as string);
                              return next;
                            })
                          }
                          className="rounded bg-(--color-accent)/20 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-(--color-accent)"
                        >
                          will unlock
                        </button>
                      )}
                    </div>
                    {f.kind === "textarea" ? (
                      <textarea
                        value={values[f.key as string]}
                        onChange={(e) =>
                          setValues((v) => ({
                            ...v,
                            [f.key as string]: e.target.value,
                          }))
                        }
                        rows={4}
                        className="w-full resize-y rounded bg-black/40 px-3 py-2 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
                      />
                    ) : (
                      <input
                        type={f.kind === "number" ? "number" : "text"}
                        step={f.kind === "number" ? "any" : undefined}
                        value={values[f.key as string]}
                        onChange={(e) =>
                          setValues((v) => ({
                            ...v,
                            [f.key as string]: e.target.value,
                          }))
                        }
                        className="w-full rounded bg-black/40 px-3 py-2 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
                      />
                    )}
                  </div>
                );
              })}
            </>
          )}

          {tab === "credits" && (
            <CreditsEditor
              credits={credits}
              onChange={setCredits}
              locked={detail.locked_fields.includes("credits")}
            />
          )}

          {tab === "posters" && (
            <PosterPicker
              loading={postersLoading}
              posters={posters}
              error={postersError}
              busy={busy}
              tmdbId={detail.tmdb_id ?? null}
              appliedPath={appliedPosterPath}
              applyingPath={applyingPosterPath}
              onPick={applyTmdbPoster}
            />
          )}
        </div>

        {error && (
          <div className="border-t border-(--color-accent)/30 bg-(--color-accent)/10 px-6 py-2 text-sm text-(--color-accent)">
            {error}
          </div>
        )}

        <div className="flex justify-end gap-3 border-t border-white/10 px-6 py-3">
          <button
            type="button"
            onClick={onClose}
            className="rounded border border-white/20 px-4 py-2 text-sm text-white transition-colors hover:border-white"
          >
            {tab === "posters" ? "Close" : "Cancel"}
          </button>
          {tab === "general" && (
            <button
              type="button"
              onClick={saveGeneral}
              disabled={busy}
              className="rounded bg-(--color-accent) px-4 py-2 text-sm font-semibold text-white transition disabled:opacity-50"
            >
              {busy ? "Saving…" : "Save Changes"}
            </button>
          )}
          {tab === "credits" && (
            <button
              type="button"
              onClick={saveCredits}
              disabled={busy}
              className="rounded bg-(--color-accent) px-4 py-2 text-sm font-semibold text-white transition disabled:opacity-50"
            >
              {busy ? "Saving…" : "Save Cast & Crew"}
            </button>
          )}
        </div>
      </div>
    </div>,
    document.body,
  );
}

interface EditableCredit {
  // Stable key for React. Existing credits use the row id; brand-new rows
  // get a synthetic key generated on insertion.
  key: string;
  person_id: number | null;
  name: string;
  role_kind: CreditEditInput["role_kind"];
  role: string;
  character_name: string | null;
  photo_url: string | null;
}

function creditToEditable(c: Credit): EditableCredit {
  const kind = (() => {
    switch (c.role_kind) {
      case "cast":
      case "director":
      case "writer":
      case "producer":
      case "crew":
        return c.role_kind;
      default:
        return "crew";
    }
  })() as EditableCredit["role_kind"];
  return {
    key: `c${c.id}`,
    person_id: c.person.id,
    name: c.person.name,
    role_kind: kind,
    role: c.role,
    character_name: c.character_name,
    photo_url: c.person.photo_url,
  };
}

function roleDefault(kind: EditableCredit["role_kind"]): string {
  switch (kind) {
    case "cast":
      return "Actor";
    case "director":
      return "Director";
    case "writer":
      return "Writer";
    case "producer":
      return "Producer";
    case "crew":
      return "Crew";
  }
}

function CreditsEditor({
  credits,
  onChange,
  locked,
}: {
  credits: EditableCredit[];
  onChange: (next: EditableCredit[]) => void;
  locked: boolean;
}) {
  function update(idx: number, patch: Partial<EditableCredit>) {
    onChange(credits.map((c, i) => (i === idx ? { ...c, ...patch } : c)));
  }
  function remove(idx: number) {
    onChange(credits.filter((_, i) => i !== idx));
  }
  function move(idx: number, delta: -1 | 1) {
    const j = idx + delta;
    if (j < 0 || j >= credits.length) return;
    const next = credits.slice();
    [next[idx], next[j]] = [next[j], next[idx]];
    onChange(next);
  }
  function add() {
    onChange([
      ...credits,
      {
        key: `n${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
        person_id: null,
        name: "",
        role_kind: "cast",
        role: "Actor",
        character_name: "",
        photo_url: null,
      },
    ]);
  }

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <p className="text-xs text-white/50">
          Drag-free reorder with the arrows. Saving any edit locks Cast &amp;
          Crew so re-enrichment won&apos;t replace it.
          {locked && (
            <span className="ml-2 rounded bg-white/10 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-white/85">
              🔒 locked
            </span>
          )}
        </p>
        <button
          type="button"
          onClick={add}
          className="rounded border border-white/20 px-2 py-1 text-xs text-white transition-colors hover:border-white"
        >
          + Add person
        </button>
      </div>
      {credits.length === 0 ? (
        <div className="rounded border border-dashed border-white/15 p-4 text-center text-xs text-white/40">
          No credits yet. Click “Add person” to start.
        </div>
      ) : (
        <ul className="space-y-2">
          {credits.map((c, i) => (
            <li
              key={c.key}
              className="grid grid-cols-12 items-center gap-2 rounded border border-white/10 bg-white/2 p-2 text-sm"
            >
              <div className="col-span-1 flex flex-col gap-0.5">
                <button
                  type="button"
                  disabled={i === 0}
                  onClick={() => move(i, -1)}
                  className="text-xs text-white/50 hover:text-white disabled:opacity-30"
                  aria-label="Move up"
                >
                  ▲
                </button>
                <button
                  type="button"
                  disabled={i === credits.length - 1}
                  onClick={() => move(i, 1)}
                  className="text-xs text-white/50 hover:text-white disabled:opacity-30"
                  aria-label="Move down"
                >
                  ▼
                </button>
              </div>
              <input
                value={c.name}
                placeholder="Name"
                onChange={(e) => update(i, { name: e.target.value })}
                className="col-span-3 rounded bg-black/40 px-2 py-1 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
              />
              <select
                value={c.role_kind}
                onChange={(e) =>
                  update(i, {
                    role_kind: e.target.value as EditableCredit["role_kind"],
                    role: roleDefault(
                      e.target.value as EditableCredit["role_kind"],
                    ),
                  })
                }
                className="col-span-2 rounded bg-black/40 px-2 py-1 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
              >
                <option value="cast">Cast</option>
                <option value="director">Director</option>
                <option value="writer">Writer</option>
                <option value="producer">Producer</option>
                <option value="crew">Crew</option>
              </select>
              <input
                value={c.role}
                placeholder="Role / job"
                onChange={(e) => update(i, { role: e.target.value })}
                className="col-span-2 rounded bg-black/40 px-2 py-1 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
              />
              <input
                value={c.character_name ?? ""}
                placeholder={c.role_kind === "cast" ? "Character" : "—"}
                disabled={c.role_kind !== "cast"}
                onChange={(e) =>
                  update(i, { character_name: e.target.value })
                }
                className="col-span-3 rounded bg-black/40 px-2 py-1 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent) disabled:opacity-30"
              />
              <button
                type="button"
                onClick={() => remove(i)}
                aria-label="Remove"
                className="col-span-1 text-right text-white/40 hover:text-(--color-accent)"
              >
                ✕
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function PosterPicker({
  loading,
  posters,
  error,
  busy,
  tmdbId,
  appliedPath,
  applyingPath,
  onPick,
}: {
  loading: boolean;
  posters: TmdbPoster[] | null;
  error: string | null;
  busy: boolean;
  tmdbId: number | null;
  appliedPath: string | null;
  applyingPath: string | null;
  onPick: (p: TmdbPoster) => void;
}) {
  if (tmdbId == null) {
    return (
      <div className="rounded border border-dashed border-white/15 p-6 text-center text-xs text-white/40">
        This item isn&apos;t matched to TMDB yet, so no poster candidates are
        available. Use Fix Match to attach a TMDB id first.
      </div>
    );
  }
  if (loading) {
    return (
      <div className="py-8 text-center text-xs text-white/50">
        Loading posters from TMDB…
      </div>
    );
  }
  if (error) {
    return (
      <div className="rounded border border-(--color-accent)/30 bg-(--color-accent)/10 p-3 text-xs text-(--color-accent)">
        Could not load posters: {error}
      </div>
    );
  }
  if (!posters || posters.length === 0) {
    return (
      <div className="rounded border border-dashed border-white/15 p-6 text-center text-xs text-white/40">
        TMDB has no poster candidates for this title.
      </div>
    );
  }
  return (
    <div>
      <p className="mb-3 text-xs text-white/50">
        Click a poster to set it as the primary artwork. The server
        downloads the original-size image and locks the poster field.
        Re-open the title modal to see the change everywhere else.
      </p>
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-4">
        {posters.map((p) => {
          const isApplied = appliedPath === p.file_path;
          const isApplying = applyingPath === p.file_path;
          return (
            <button
              key={p.file_path}
              type="button"
              disabled={busy}
              onClick={() => onPick(p)}
              className={`group relative aspect-[2/3] overflow-hidden rounded border bg-black/40 transition disabled:opacity-50 ${
                isApplied
                  ? "border-(--color-accent) ring-2 ring-(--color-accent)"
                  : "border-white/10 hover:border-(--color-accent)"
              }`}
              title={
                p.language
                  ? `${p.width}×${p.height} · ${p.language.toUpperCase()}`
                  : `${p.width}×${p.height}`
              }
            >
              {/* eslint-disable-next-line @next/next/no-img-element */}
              <img
                src={p.thumb_url}
                alt=""
                loading="lazy"
                className="h-full w-full object-cover"
              />
              {isApplied && (
                <div className="absolute right-2 top-2 rounded-full bg-(--color-accent) px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-white shadow">
                  ✓ Active
                </div>
              )}
              {isApplying && (
                <div className="absolute inset-0 flex items-center justify-center bg-black/60 text-xs font-semibold text-white">
                  Applying…
                </div>
              )}
              <div className="absolute inset-x-0 bottom-0 flex items-center justify-between bg-black/70 px-2 py-1 text-[10px] text-white/70">
                <span>{p.language ? p.language.toUpperCase() : "—"}</span>
                <span>
                  {p.width}×{p.height}
                </span>
              </div>
            </button>
          );
        })}
      </div>
    </div>
  );
}

function TabButton({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`rounded-t px-3 py-2 text-sm transition ${
        active
          ? "bg-(--color-surface) text-white"
          : "text-white/60 hover:text-white"
      }`}
    >
      {children}
    </button>
  );
}

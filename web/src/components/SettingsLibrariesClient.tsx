"use client";

import { useState } from "react";
import {
  libraries as librariesApi,
  ChimpFlixApiError,
  type Library,
  type LibraryKind,
  type ScanJob,
} from "@/lib/chimpflix-api";
import { LibraryAccessClient } from "./LibraryAccessClient";
import { ConfirmDialog } from "./ConfirmDialog";

interface Props {
  initial: Library[];
}

function kindLabel(kind: LibraryKind): string {
  switch (kind) {
    case "movies":
      return "Movies";
    case "shows":
      return "Shows";
    case "anime":
      return "Anime";
  }
}

export function SettingsLibrariesClient({ initial }: Props) {
  const [libs, setLibs] = useState(initial);
  const [busy, setBusy] = useState<number | "create" | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [showAdd, setShowAdd] = useState(false);
  const [editingId, setEditingId] = useState<number | null>(null);
  const [scansForId, setScansForId] = useState<number | null>(null);
  const [scans, setScans] = useState<ScanJob[]>([]);
  const [askDetect, setAskDetect] = useState<number | null>(null);
  const [askDelete, setAskDelete] = useState<{ id: number; name: string } | null>(null);
  const [scansLoading, setScansLoading] = useState(false);
  const [accessForId, setAccessForId] = useState<number | null>(null);

  async function triggerScan(id: number) {
    setBusy(id);
    setMessage(null);
    try {
      await librariesApi.triggerScan(id);
      setMessage(`Scan queued for library ${id}.`);
      await refresh();
    } catch (e) {
      setMessage(
        `Failed to start scan: ${e instanceof Error ? e.message : "unknown"}`,
      );
    } finally {
      setBusy(null);
    }
  }

  function detectMarkers(id: number) {
    setAskDetect(id);
  }

  async function confirmDetectMarkers() {
    if (askDetect == null) return;
    const id = askDetect;
    setAskDetect(null);
    setBusy(id);
    setMessage(null);
    try {
      const { queued } = await librariesApi.detectMarkers(id);
      setMessage(
        `Marker detection queued for ${queued} file${queued === 1 ? "" : "s"}. Results appear on /watch as they complete.`,
      );
    } catch (e) {
      setMessage(
        `Failed: ${e instanceof Error ? e.message : "unknown"}`,
      );
    } finally {
      setBusy(null);
    }
  }

  function deleteLibrary(id: number, name: string) {
    setAskDelete({ id, name });
  }

  async function confirmDeleteLibrary() {
    if (!askDelete) return;
    const { id, name } = askDelete;
    setAskDelete(null);
    setBusy(id);
    setMessage(null);
    try {
      await librariesApi.delete(id);
      setMessage(`Removed "${name}".`);
      await refresh();
    } catch (e) {
      setMessage(
        `Failed to delete: ${e instanceof Error ? e.message : "unknown"}`,
      );
    } finally {
      setBusy(null);
    }
  }

  async function createLibrary(name: string, kind: LibraryKind, paths: string[]) {
    setBusy("create");
    setMessage(null);
    try {
      await librariesApi.create({ name, kind, paths });
      setMessage(`Created "${name}". Trigger a scan to ingest media.`);
      setShowAdd(false);
      await refresh();
    } catch (e) {
      if (e instanceof ChimpFlixApiError) {
        try {
          const parsed = JSON.parse(e.body) as { error?: { message?: string } };
          setMessage(
            `Failed to create: ${parsed.error?.message ?? `HTTP ${e.status}`}`,
          );
        } catch {
          setMessage(`Failed to create: HTTP ${e.status}`);
        }
      } else {
        setMessage("Failed to create: network error");
      }
    } finally {
      setBusy(null);
    }
  }

  async function updateLibrary(id: number, name: string, paths: string[]) {
    setBusy(id);
    setMessage(null);
    try {
      await librariesApi.update(id, { name, paths });
      setMessage(`Updated "${name}".`);
      setEditingId(null);
      await refresh();
    } catch (e) {
      setMessage(
        `Failed to update: ${e instanceof Error ? e.message : "unknown"}`,
      );
    } finally {
      setBusy(null);
    }
  }

  async function refresh() {
    try {
      const { libraries } = await librariesApi.list();
      setLibs(libraries);
    } catch {
      // best-effort refresh
    }
  }

  async function toggleScans(id: number) {
    if (scansForId === id) {
      setScansForId(null);
      setScans([]);
      return;
    }
    setScansForId(id);
    setScans([]);
    setScansLoading(true);
    try {
      const { scans } = await librariesApi.listScans(id);
      setScans(scans);
    } catch {
      setScans([]);
    } finally {
      setScansLoading(false);
    }
  }

  return (
    <div>
      {libs.length === 0 ? (
        <p className="text-sm text-white/60">
          No libraries yet. Add one below — the path should be reachable from
          inside the server container.
        </p>
      ) : (
        <ul className="divide-y divide-white/5">
          {libs.map((lib) =>
            editingId === lib.id ? (
              <li key={lib.id} className="py-3">
                <EditLibraryForm
                  lib={lib}
                  busy={busy === lib.id}
                  onSubmit={(name, paths) =>
                    updateLibrary(lib.id, name, paths)
                  }
                  onCancel={() => setEditingId(null)}
                />
              </li>
            ) : (
              <li
                key={lib.id}
                className="flex items-start justify-between gap-4 py-3 text-sm"
              >
                <div className="flex-1">
                  <div className="font-medium">{lib.name}</div>
                  <div className="text-xs text-white/55">
                    {kindLabel(lib.kind)} · {lib.paths.join(", ")}
                  </div>
                  {lib.last_scan_at && (
                    <div className="mt-0.5 text-xs text-white/40">
                      Last scan {new Date(lib.last_scan_at).toLocaleString()}
                    </div>
                  )}
                </div>
                <div className="flex flex-wrap gap-2">
                  <button
                    type="button"
                    onClick={() => triggerScan(lib.id)}
                    disabled={busy === lib.id}
                    className="rounded bg-white/10 px-3 py-1.5 text-xs font-medium transition hover:bg-white/15 disabled:opacity-50"
                  >
                    {busy === lib.id ? "…" : "Scan"}
                  </button>
                  <button
                    type="button"
                    onClick={() => toggleScans(lib.id)}
                    className="rounded bg-white/10 px-3 py-1.5 text-xs font-medium transition hover:bg-white/15"
                  >
                    {scansForId === lib.id ? "Hide history" : "History"}
                  </button>
                  <button
                    type="button"
                    onClick={() =>
                      setAccessForId(accessForId === lib.id ? null : lib.id)
                    }
                    className="rounded bg-white/10 px-3 py-1.5 text-xs font-medium transition hover:bg-white/15"
                  >
                    {accessForId === lib.id ? "Hide access" : "Access"}
                  </button>
                  <button
                    type="button"
                    onClick={() => detectMarkers(lib.id)}
                    disabled={busy === lib.id}
                    className="rounded bg-white/10 px-3 py-1.5 text-xs font-medium transition hover:bg-white/15 disabled:opacity-50"
                  >
                    Detect markers
                  </button>
                  <button
                    type="button"
                    onClick={() => setEditingId(lib.id)}
                    disabled={busy === lib.id}
                    className="rounded bg-white/10 px-3 py-1.5 text-xs font-medium transition hover:bg-white/15 disabled:opacity-50"
                  >
                    Edit
                  </button>
                  <button
                    type="button"
                    onClick={() => deleteLibrary(lib.id, lib.name)}
                    disabled={busy === lib.id}
                    className="rounded bg-red-500/15 px-3 py-1.5 text-xs font-medium text-red-200 transition hover:bg-red-500/25 disabled:opacity-50"
                  >
                    Delete
                  </button>
                </div>
              </li>
            ),
          )}
        </ul>
      )}

      {scansForId !== null && (
        <ScanHistoryView
          loading={scansLoading}
          scans={scans}
          onClose={() => {
            setScansForId(null);
            setScans([]);
          }}
        />
      )}

      {accessForId !== null && (
        <LibraryAccessClient
          libraryId={accessForId}
          libraryName={
            libs.find((l) => l.id === accessForId)?.name ?? ""
          }
          onClose={() => setAccessForId(null)}
        />
      )}

      <div className="mt-4">
        {showAdd ? (
          <AddLibraryForm
            busy={busy === "create"}
            onSubmit={createLibrary}
            onCancel={() => setShowAdd(false)}
          />
        ) : (
          <button
            type="button"
            onClick={() => setShowAdd(true)}
            className="rounded bg-(--color-accent) px-3 py-1.5 text-xs font-semibold text-white transition hover:opacity-90"
          >
            + Add library
          </button>
        )}
      </div>

      <div className="mt-3 flex items-center gap-3 text-xs">
        <button
          type="button"
          onClick={refresh}
          className="text-white/60 underline-offset-2 hover:text-white hover:underline"
        >
          Refresh
        </button>
        {message && <span className="text-white/70">{message}</span>}
      </div>
      {askDetect != null && (
        <ConfirmDialog
          title="Run marker detection on this library?"
          body="Every file gets scanned for intro/credits markers. This can take a while — files are processed sequentially in the background. You can keep using the app meanwhile."
          confirmLabel="Run"
          busy={busy === askDetect}
          onConfirm={() => void confirmDetectMarkers()}
          onCancel={() => setAskDetect(null)}
        />
      )}
      {askDelete && (
        <ConfirmDialog
          title={`Delete library "${askDelete.name}"?`}
          body="Items and play state for this library will be removed. The underlying files on disk are not touched."
          confirmLabel="Delete library"
          destructive
          busy={busy === askDelete.id}
          onConfirm={() => void confirmDeleteLibrary()}
          onCancel={() => setAskDelete(null)}
        />
      )}
    </div>
  );
}

function ScanHistoryView({
  loading,
  scans,
  onClose,
}: {
  loading: boolean;
  scans: ScanJob[];
  onClose: () => void;
}) {
  return (
    <div className="mt-3 rounded-md border border-white/10 bg-white/2 p-4">
      <div className="mb-3 flex items-center justify-between">
        <h3 className="text-sm font-semibold">Scan history</h3>
        <button
          type="button"
          onClick={onClose}
          className="text-xs text-white/55 hover:text-white"
        >
          Close
        </button>
      </div>
      {loading ? (
        <p className="text-xs text-white/55">Loading…</p>
      ) : scans.length === 0 ? (
        <p className="text-xs text-white/55">No scans yet.</p>
      ) : (
        <ul className="divide-y divide-white/5 text-xs">
          {scans.map((s) => (
            <li key={s.id} className="flex items-start justify-between gap-3 py-2">
              <div>
                <span
                  className={`mr-2 inline-block rounded px-1.5 py-0.5 font-medium uppercase ${statusClass(
                    s.status,
                  )}`}
                >
                  {s.status}
                </span>
                <span className="text-white/70">
                  {s.started_at
                    ? new Date(s.started_at).toLocaleString()
                    : new Date(s.created_at).toLocaleString()}
                </span>
                {s.error_message && (
                  <div className="mt-1 text-red-300">{s.error_message}</div>
                )}
              </div>
              <div className="shrink-0 text-right text-white/55">
                <div>+{s.files_added} added</div>
                <div>~{s.files_updated} updated</div>
                {s.files_removed > 0 && (
                  <div>−{s.files_removed} removed</div>
                )}
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function statusClass(s: ScanJob["status"]): string {
  switch (s) {
    case "completed":
      return "bg-green-500/15 text-green-300";
    case "running":
      return "bg-blue-500/15 text-blue-300";
    case "failed":
      return "bg-red-500/15 text-red-300";
    case "canceled":
      return "bg-yellow-500/15 text-yellow-300";
    default:
      return "bg-white/10 text-white/70";
  }
}

function EditLibraryForm({
  lib,
  busy,
  onSubmit,
  onCancel,
}: {
  lib: Library;
  busy: boolean;
  onSubmit: (name: string, paths: string[]) => void;
  onCancel: () => void;
}) {
  const [name, setName] = useState(lib.name);
  const [pathInput, setPathInput] = useState(lib.paths.join("\n"));

  function submit(e: React.FormEvent) {
    e.preventDefault();
    const paths = pathInput
      .split(/[\n,]/)
      .map((s) => s.trim())
      .filter(Boolean);
    if (!name.trim() || paths.length === 0) return;
    onSubmit(name.trim(), paths);
  }

  return (
    <form
      onSubmit={submit}
      className="rounded-md border border-white/10 bg-white/5 p-4"
    >
      <div className="mb-3 text-xs text-white/50">
        Editing &ldquo;{lib.name}&rdquo; · kind is fixed ({kindLabel(lib.kind)})
      </div>
      <label className="block text-xs">
        <span className="mb-1 block text-white/60">Name</span>
        <input
          type="text"
          required
          value={name}
          onChange={(e) => setName(e.target.value)}
          className="w-full rounded bg-white/10 px-3 py-2 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
        />
      </label>
      <label className="mt-3 block text-xs">
        <span className="mb-1 block text-white/60">
          Paths (one per line, in-container)
        </span>
        <textarea
          required
          rows={2}
          value={pathInput}
          onChange={(e) => setPathInput(e.target.value)}
          className="w-full rounded bg-white/10 px-3 py-2 font-mono text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
        />
      </label>
      <div className="mt-3 flex gap-2">
        <button
          type="submit"
          disabled={busy}
          className="rounded bg-(--color-accent) px-3 py-1.5 text-xs font-semibold text-white transition disabled:opacity-50"
        >
          {busy ? "Saving…" : "Save"}
        </button>
        <button
          type="button"
          onClick={onCancel}
          className="rounded bg-white/10 px-3 py-1.5 text-xs font-medium text-white/85 transition hover:bg-white/15"
        >
          Cancel
        </button>
      </div>
    </form>
  );
}

function AddLibraryForm({
  busy,
  onSubmit,
  onCancel,
}: {
  busy: boolean;
  onSubmit: (name: string, kind: LibraryKind, paths: string[]) => void;
  onCancel: () => void;
}) {
  const [name, setName] = useState("");
  const [kind, setKind] = useState<LibraryKind>("movies");
  const [pathInput, setPathInput] = useState("");

  function submit(e: React.FormEvent) {
    e.preventDefault();
    const paths = pathInput
      .split(/[\n,]/)
      .map((s) => s.trim())
      .filter(Boolean);
    if (!name.trim() || paths.length === 0) return;
    onSubmit(name.trim(), kind, paths);
  }

  return (
    <form
      onSubmit={submit}
      className="mt-2 rounded-md border border-white/10 bg-white/2 p-4"
    >
      <div className="grid gap-3 sm:grid-cols-2">
        <label className="block text-xs">
          <span className="mb-1 block text-white/60">Name</span>
          <input
            type="text"
            required
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Movies"
            className="w-full rounded bg-white/10 px-3 py-2 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
          />
        </label>
        <label className="block text-xs">
          <span className="mb-1 block text-white/60">Kind</span>
          <select
            value={kind}
            onChange={(e) => setKind(e.target.value as LibraryKind)}
            className="w-full rounded bg-white/10 px-3 py-2 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
          >
            <option value="movies">Movies</option>
            <option value="shows">Shows</option>
            <option value="anime">Anime</option>
          </select>
        </label>
      </div>
      <label className="mt-3 block text-xs">
        <span className="mb-1 block text-white/60">
          Paths (one per line, in-container)
        </span>
        <textarea
          required
          rows={2}
          value={pathInput}
          onChange={(e) => setPathInput(e.target.value)}
          placeholder="/media/movies"
          className="w-full rounded bg-white/10 px-3 py-2 font-mono text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
        />
      </label>
      <div className="mt-3 flex gap-2">
        <button
          type="submit"
          disabled={busy}
          className="rounded bg-(--color-accent) px-3 py-1.5 text-xs font-semibold text-white transition disabled:opacity-50"
        >
          {busy ? "Creating…" : "Create"}
        </button>
        <button
          type="button"
          onClick={onCancel}
          className="rounded bg-white/10 px-3 py-1.5 text-xs font-medium text-white/85 transition hover:bg-white/15"
        >
          Cancel
        </button>
      </div>
    </form>
  );
}

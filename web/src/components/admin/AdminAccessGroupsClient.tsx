"use client";

import { useState } from "react";
import {
  admin as adminApi,
  ChimpFlixApiError,
  type AccessGroup,
  type AccessGroupDetail,
  type Library,
  type User,
} from "@/lib/chimpflix-api";

interface Props {
  initialGroups: AccessGroup[];
  users: User[];
  libraries: Library[];
}

export function AdminAccessGroupsClient({
  initialGroups,
  users,
  libraries,
}: Props) {
  const [groups, setGroups] = useState(initialGroups);
  const [activeId, setActiveId] = useState<number | null>(null);
  const [detail, setDetail] = useState<AccessGroupDetail | null>(null);
  const [loadingDetail, setLoadingDetail] = useState(false);
  const [newName, setNewName] = useState("");
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function refreshList() {
    try {
      const { groups } = await adminApi.accessGroups.list();
      setGroups(groups);
    } catch (e) {
      setError(parseError(e));
    }
  }

  async function loadDetail(id: number) {
    setActiveId(id);
    setLoadingDetail(true);
    setDetail(null);
    try {
      const d = await adminApi.accessGroups.get(id);
      setDetail(d);
    } catch (e) {
      setError(parseError(e));
    } finally {
      setLoadingDetail(false);
    }
  }

  async function create() {
    const name = newName.trim();
    if (!name) return;
    setCreating(true);
    setError(null);
    try {
      const group = await adminApi.accessGroups.create({ name });
      setNewName("");
      await refreshList();
      void loadDetail(group.id);
    } catch (e) {
      setError(parseError(e));
    } finally {
      setCreating(false);
    }
  }

  async function deleteGroup(id: number, name: string) {
    if (
      !window.confirm(
        `Delete group "${name}"? Members will lose any access that came only from this group. Direct grants are kept.`,
      )
    ) {
      return;
    }
    try {
      await adminApi.accessGroups.delete(id);
      if (activeId === id) {
        setActiveId(null);
        setDetail(null);
      }
      await refreshList();
    } catch (e) {
      setError(parseError(e));
    }
  }

  return (
    <div className="grid grid-cols-1 gap-6 lg:grid-cols-[18rem_1fr]">
      <aside className="space-y-3">
        <div className="space-y-2 rounded-lg border border-white/10 bg-white/2 p-3">
          <h3 className="text-xs font-semibold uppercase tracking-wider text-white/55">
            New group
          </h3>
          <div className="flex gap-2">
            <input
              type="text"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              placeholder="e.g. Family"
              maxLength={64}
              className="flex-1 rounded bg-white/10 px-2 py-1.5 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
            />
            <button
              type="button"
              onClick={create}
              disabled={creating || !newName.trim()}
              className="rounded bg-(--color-accent) px-3 py-1.5 text-xs font-semibold text-white disabled:opacity-50"
            >
              {creating ? "…" : "Add"}
            </button>
          </div>
        </div>

        {error && (
          <div className="rounded border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
            {error}
          </div>
        )}

        <ul className="divide-y divide-white/5 overflow-hidden rounded-lg border border-white/10 bg-white/2">
          {groups.length === 0 && (
            <li className="px-3 py-4 text-center text-xs text-white/45">
              No groups yet.
            </li>
          )}
          {groups.map((g) => {
            const active = activeId === g.id;
            return (
              <li key={g.id}>
                <button
                  type="button"
                  onClick={() => loadDetail(g.id)}
                  className={`block w-full px-3 py-2 text-left transition-colors ${
                    active ? "bg-white/10" : "hover:bg-white/5"
                  }`}
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className={`text-sm ${active ? "text-white" : "text-white/85"}`}>
                      {g.name}
                    </span>
                    <span className="text-[10px] uppercase tracking-wider text-white/40">
                      {g.member_count} · {g.library_count} libs
                    </span>
                  </div>
                  {g.description && (
                    <div className="mt-0.5 truncate text-[11px] text-white/45">
                      {g.description}
                    </div>
                  )}
                </button>
              </li>
            );
          })}
        </ul>
      </aside>

      <section className="rounded-lg border border-white/10 bg-white/2 p-5">
        {!activeId && (
          <p className="text-sm text-white/55">
            Select a group on the left, or create a new one to get started.
          </p>
        )}
        {activeId && loadingDetail && (
          <p className="text-sm text-white/55">Loading…</p>
        )}
        {detail && (
          <GroupEditor
            detail={detail}
            users={users}
            libraries={libraries}
            onChanged={async () => {
              await refreshList();
              await loadDetail(detail.id);
            }}
            onDeleted={() => deleteGroup(detail.id, detail.name)}
            onError={(e) => setError(e)}
          />
        )}
      </section>
    </div>
  );
}

function GroupEditor({
  detail,
  users,
  libraries,
  onChanged,
  onDeleted,
  onError,
}: {
  detail: AccessGroupDetail;
  users: User[];
  libraries: Library[];
  onChanged: () => Promise<void> | void;
  onDeleted: () => void;
  onError: (msg: string) => void;
}) {
  const [name, setName] = useState(detail.name);
  const [description, setDescription] = useState(detail.description ?? "");
  const [libs, setLibs] = useState<Set<number>>(new Set(detail.library_ids));
  const [members, setMembers] = useState<Set<number>>(new Set(detail.member_ids));
  const [savingMeta, setSavingMeta] = useState(false);
  const [savingLibs, setSavingLibs] = useState(false);
  const [savingMembers, setSavingMembers] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  const metaDirty =
    name !== detail.name || (description || null) !== (detail.description ?? null);
  const libsDirty = !setsEqual(libs, new Set(detail.library_ids));
  const membersDirty = !setsEqual(members, new Set(detail.member_ids));

  async function saveMeta() {
    setSavingMeta(true);
    setNotice(null);
    try {
      await adminApi.accessGroups.update(detail.id, {
        name: name.trim(),
        description: description.trim() || null,
      });
      await onChanged();
      setNotice("Saved.");
    } catch (e) {
      onError(parseError(e));
    } finally {
      setSavingMeta(false);
    }
  }

  async function saveLibs() {
    setSavingLibs(true);
    setNotice(null);
    try {
      await adminApi.accessGroups.setLibraries(detail.id, Array.from(libs));
      await onChanged();
      setNotice("Libraries updated.");
    } catch (e) {
      onError(parseError(e));
    } finally {
      setSavingLibs(false);
    }
  }

  async function saveMembers() {
    setSavingMembers(true);
    setNotice(null);
    try {
      await adminApi.accessGroups.setMembers(detail.id, Array.from(members));
      await onChanged();
      setNotice("Members updated.");
    } catch (e) {
      onError(parseError(e));
    } finally {
      setSavingMembers(false);
    }
  }

  function toggleLib(id: number) {
    setLibs((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }
  function toggleMember(id: number) {
    setMembers((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  return (
    <div className="space-y-6">
      <div className="flex items-start justify-between gap-3">
        <h2 className="text-lg font-semibold">{detail.name}</h2>
        <button
          type="button"
          onClick={onDeleted}
          className="rounded border border-red-500/40 bg-red-500/10 px-3 py-1.5 text-xs font-medium text-red-200 hover:bg-red-500/20"
        >
          Delete group
        </button>
      </div>
      {notice && (
        <div className="rounded border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-xs text-emerald-200">
          {notice}
        </div>
      )}

      {/* Name + description */}
      <section className="space-y-3">
        <h3 className="text-xs font-semibold uppercase tracking-wider text-white/55">
          Details
        </h3>
        <div className="grid gap-3 md:grid-cols-2">
          <label className="block text-xs">
            <span className="mb-1 block text-white/70">Name</span>
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              maxLength={64}
              className="w-full rounded bg-white/10 px-3 py-2 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
            />
          </label>
          <label className="block text-xs">
            <span className="mb-1 block text-white/70">
              Description <span className="text-white/40">(optional)</span>
            </span>
            <input
              type="text"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="e.g. Movies + TV for the family"
              maxLength={280}
              className="w-full rounded bg-white/10 px-3 py-2 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
            />
          </label>
        </div>
        <button
          type="button"
          onClick={saveMeta}
          disabled={!metaDirty || savingMeta}
          className="rounded bg-(--color-accent) px-3 py-1.5 text-xs font-semibold text-white disabled:opacity-50"
        >
          {savingMeta ? "…" : "Save details"}
        </button>
      </section>

      {/* Libraries */}
      <section className="space-y-3">
        <h3 className="text-xs font-semibold uppercase tracking-wider text-white/55">
          Libraries ({libs.size})
        </h3>
        {libraries.length === 0 ? (
          <p className="text-xs text-white/45">No libraries to bind yet.</p>
        ) : (
          <div className="flex flex-wrap gap-2">
            {libraries.map((l) => {
              const active = libs.has(l.id);
              return (
                <button
                  key={l.id}
                  type="button"
                  onClick={() => toggleLib(l.id)}
                  className={
                    "rounded-full border px-3 py-1 text-xs transition-colors " +
                    (active
                      ? "border-(--color-accent) bg-accent/20 text-white"
                      : "border-white/15 text-white/70 hover:border-white/30")
                  }
                >
                  {l.name}
                </button>
              );
            })}
          </div>
        )}
        <button
          type="button"
          onClick={saveLibs}
          disabled={!libsDirty || savingLibs}
          className="rounded bg-(--color-accent) px-3 py-1.5 text-xs font-semibold text-white disabled:opacity-50"
        >
          {savingLibs ? "…" : "Save libraries"}
        </button>
      </section>

      {/* Members */}
      <section className="space-y-3">
        <h3 className="text-xs font-semibold uppercase tracking-wider text-white/55">
          Members ({members.size})
        </h3>
        {users.length === 0 ? (
          <p className="text-xs text-white/45">No users to add yet.</p>
        ) : (
          <ul className="divide-y divide-white/5 overflow-hidden rounded-md border border-white/10">
            {users.map((u) => {
              const active = members.has(u.id);
              return (
                <li key={u.id} className="flex items-center justify-between gap-3 px-3 py-2">
                  <div>
                    <div className="text-sm text-white/90">
                      {u.display_name ?? u.username}
                    </div>
                    <div className="text-[11px] text-white/50">
                      @{u.username}
                      {u.role === "owner" && (
                        <span className="ml-1 text-(--color-accent)">· owner</span>
                      )}
                    </div>
                  </div>
                  <label className="flex cursor-pointer items-center gap-2 text-xs text-white/75">
                    <input
                      type="checkbox"
                      checked={active}
                      onChange={() => toggleMember(u.id)}
                    />
                    {active ? "Member" : "Add"}
                  </label>
                </li>
              );
            })}
          </ul>
        )}
        <button
          type="button"
          onClick={saveMembers}
          disabled={!membersDirty || savingMembers}
          className="rounded bg-(--color-accent) px-3 py-1.5 text-xs font-semibold text-white disabled:opacity-50"
        >
          {savingMembers ? "…" : "Save members"}
        </button>
      </section>
    </div>
  );
}

function setsEqual(a: Set<number>, b: Set<number>): boolean {
  if (a.size !== b.size) return false;
  for (const x of a) if (!b.has(x)) return false;
  return true;
}

function parseError(e: unknown): string {
  if (e instanceof ChimpFlixApiError) {
    try {
      const parsed = JSON.parse(e.body) as {
        error?: { code?: string; message?: string };
      };
      if (parsed.error?.message) return parsed.error.message;
    } catch {
      // fall through
    }
    return `HTTP ${e.status}`;
  }
  return e instanceof Error ? e.message : String(e);
}

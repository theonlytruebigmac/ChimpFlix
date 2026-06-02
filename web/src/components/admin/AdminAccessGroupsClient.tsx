"use client";

import { useState } from "react";
import {
  admin as adminApi,
  ChimpFlixApiError,
  type AccessGroup,
  type AccessGroupDetail,
  type AccessLevel,
  type Library,
  type User,
} from "@/lib/chimpflix-api";
import { ConfirmDialog } from "../ConfirmDialog";
import { LoadingPlaceholder } from "../ui/LoadingPlaceholder";

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
  const [showNew, setShowNew] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [askDelete, setAskDelete] = useState<{ id: number; name: string } | null>(null);
  const [deleteBusy, setDeleteBusy] = useState(false);

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
      setShowNew(false);
      await refreshList();
      void loadDetail(group.id);
    } catch (e) {
      setError(parseError(e));
    } finally {
      setCreating(false);
    }
  }

  function deleteGroup(id: number, name: string) {
    setAskDelete({ id, name });
  }

  async function confirmDeleteGroup() {
    if (!askDelete) return;
    setDeleteBusy(true);
    try {
      await adminApi.accessGroups.delete(askDelete.id);
      if (activeId === askDelete.id) {
        setActiveId(null);
        setDetail(null);
      }
      await refreshList();
      setAskDelete(null);
    } catch (e) {
      setError(parseError(e));
    } finally {
      setDeleteBusy(false);
    }
  }

  return (
    <div>
      <div
        className="cf-flex cf-between cf-wrap cf-gap12"
        style={{ marginBottom: 14 }}
      >
        <div className="cf-muted" style={{ fontSize: 13 }}>
          Apply access to several users at once.
        </div>
        <button
          type="button"
          className="cf-btn cf-primary cf-sm"
          onClick={() => setShowNew((v) => !v)}
        >
          New group
        </button>
      </div>

      {showNew && (
        <div className="cf-card">
          <div className="cf-card-body cf-pad">
            <div className="cf-field" style={{ marginBottom: 0 }}>
              <label className="cf-field-label">Group name</label>
              <div className="cf-flex cf-gap8">
                <input
                  type="text"
                  className="cf-input"
                  value={newName}
                  onChange={(e) => setNewName(e.target.value)}
                  placeholder="e.g. Family"
                  maxLength={64}
                />
                <button
                  type="button"
                  className="cf-btn cf-primary cf-sm"
                  onClick={create}
                  disabled={creating || !newName.trim()}
                >
                  {creating ? "…" : "Create"}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {error && (
        <div role="alert" aria-live="assertive" className="cf-banner cf-err">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 8v4M12 16v.5" />
          </svg>
          <div>{error}</div>
        </div>
      )}

      <div className="cf-card" style={{ marginBottom: 0 }}>
        {groups.length === 0 ? (
          <div
            className="cf-card-body cf-pad cf-center cf-muted"
            style={{ fontSize: 13 }}
          >
            No groups yet. Create one above.
          </div>
        ) : (
          <table className="cf-table">
            <thead>
              <tr>
                <th>Group</th>
                <th>Members</th>
                <th>Libraries</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {groups.map((g) => (
                <tr key={g.id}>
                  <td>
                    <b>{g.name}</b>
                    {g.description && (
                      <div className="cf-faint" style={{ fontSize: 11.5, marginTop: 2 }}>
                        {g.description}
                      </div>
                    )}
                  </td>
                  <td className="cf-muted">
                    {g.member_count} member{g.member_count === 1 ? "" : "s"}
                  </td>
                  <td className="cf-muted">
                    {g.library_count} librar{g.library_count === 1 ? "y" : "ies"}
                  </td>
                  <td className="cf-num">
                    <button
                      type="button"
                      className="cf-btn cf-ghost cf-tiny"
                      onClick={() => loadDetail(g.id)}
                    >
                      Edit
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>

      {activeId && (
        <div className="cf-card" style={{ marginTop: 18, marginBottom: 0 }}>
          <div className="cf-card-body cf-pad">
            {loadingDetail && <LoadingPlaceholder />}
            {detail && (
              <GroupEditor
                detail={detail}
                users={users}
                libraries={libraries}
                onClose={() => {
                  setActiveId(null);
                  setDetail(null);
                }}
                onChanged={async () => {
                  await refreshList();
                  await loadDetail(detail.id);
                }}
                onDeleted={() => deleteGroup(detail.id, detail.name)}
                onError={(e) => setError(e)}
              />
            )}
          </div>
        </div>
      )}

      {askDelete && (
        <ConfirmDialog
          title={`Delete group "${askDelete.name}"?`}
          body="Members will lose any access that came only from this group. Direct grants on each user are kept."
          confirmLabel="Delete group"
          destructive
          busy={deleteBusy}
          onConfirm={() => void confirmDeleteGroup()}
          onCancel={() => setAskDelete(null)}
        />
      )}
    </div>
  );
}

function GroupEditor({
  detail,
  users,
  libraries,
  onClose,
  onChanged,
  onDeleted,
  onError,
}: {
  detail: AccessGroupDetail;
  users: User[];
  libraries: Library[];
  onClose: () => void;
  onChanged: () => Promise<void> | void;
  onDeleted: () => void;
  onError: (msg: string) => void;
}) {
  const [name, setName] = useState(detail.name);
  const [description, setDescription] = useState(detail.description ?? "");
  // Per-library level map. Libraries absent from the map are unbound ("none").
  const [libLevels, setLibLevels] = useState<Map<number, AccessLevel>>(
    () => initialLibLevels(detail),
  );
  const [members, setMembers] = useState<Set<number>>(new Set(detail.member_ids));
  const [savingMeta, setSavingMeta] = useState(false);
  const [savingLibs, setSavingLibs] = useState(false);
  const [savingMembers, setSavingMembers] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  const metaDirty =
    name !== detail.name || (description || null) !== (detail.description ?? null);
  const libsDirty = !levelMapsEqual(libLevels, initialLibLevels(detail));
  const membersDirty = !setsEqual(members, new Set(detail.member_ids));
  const boundCount = Array.from(libLevels.values()).filter((v) => v !== "none").length;

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
      const grants = Array.from(libLevels, ([library_id, level]) => ({
        library_id,
        level,
      })).filter((g) => g.level !== "none");
      await adminApi.accessGroups.setLibraries(detail.id, grants);
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

  // Cycle a library's level: none → view → full → none.
  function cycleLib(id: number) {
    setLibLevels((prev) => {
      const next = new Map(prev);
      const cur = next.get(id) ?? "none";
      const order: AccessLevel[] = ["none", "view", "full"];
      const nextLevel = order[(order.indexOf(cur) + 1) % order.length];
      if (nextLevel === "none") next.delete(id);
      else next.set(id, nextLevel);
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
    <div>
      <div className="cf-flex cf-between" style={{ marginBottom: 16 }}>
        <div style={{ fontSize: 16, fontWeight: 700 }}>{detail.name}</div>
        <div className="cf-flex cf-gap8">
          <button type="button" className="cf-btn cf-ghost cf-sm" onClick={onClose}>
            Close
          </button>
          <button type="button" className="cf-btn cf-danger cf-sm" onClick={onDeleted}>
            Delete group
          </button>
        </div>
      </div>

      {notice && (
        <div role="status" aria-live="polite" className="cf-banner cf-ok">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M20 6L9 17l-5-5" />
          </svg>
          <div>{notice}</div>
        </div>
      )}

      {/* Name + description */}
      <div className="cf-section-title">Details</div>
      <div className="cf-grid cf-c2">
        <div className="cf-field" style={{ marginBottom: 0 }}>
          <label className="cf-field-label">Name</label>
          <input
            type="text"
            className="cf-input"
            value={name}
            onChange={(e) => setName(e.target.value)}
            maxLength={64}
          />
        </div>
        <div className="cf-field" style={{ marginBottom: 0 }}>
          <label className="cf-field-label">
            Description <span className="cf-faint">(optional)</span>
          </label>
          <input
            type="text"
            className="cf-input"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="e.g. Movies + TV for the family"
            maxLength={280}
          />
        </div>
      </div>
      <div style={{ marginTop: 12 }}>
        <button
          type="button"
          className="cf-btn cf-primary cf-sm"
          onClick={saveMeta}
          disabled={!metaDirty || savingMeta}
        >
          {savingMeta ? "…" : "Save details"}
        </button>
      </div>

      {/* Libraries */}
      <div className="cf-section-title">Libraries ({boundCount})</div>
      <p className="cf-faint" style={{ fontSize: 12, marginTop: 0 }}>
        Click a library to cycle the level it grants members: unbound →{" "}
        <b>View</b> (browse only) → <b>Full</b> (browse + play).
      </p>
      <AccessLevelLegend />
      {libraries.length === 0 ? (
        <p className="cf-faint" style={{ fontSize: 12 }}>No libraries to bind yet.</p>
      ) : (
        <div className="cf-flex cf-wrap cf-gap8">
          {libraries.map((l) => {
            const level = libLevels.get(l.id) ?? "none";
            // Canonical tri-state palette (matches the access matrix):
            // View = amber (cf-warn), Full = green (cf-ok), unbound = muted.
            const cls =
              level === "full" ? " cf-ok" : level === "view" ? " cf-warn" : "";
            const suffix =
              level === "full" ? " · Full" : level === "view" ? " · View" : "";
            return (
              <button
                key={l.id}
                type="button"
                onClick={() => cycleLib(l.id)}
                className={`cf-pill${cls}`}
                style={{ cursor: "pointer", padding: "5px 12px" }}
                title={`Level: ${level}`}
              >
                {l.name}
                {suffix}
              </button>
            );
          })}
        </div>
      )}
      <div style={{ marginTop: 12 }}>
        <button
          type="button"
          className="cf-btn cf-primary cf-sm"
          onClick={saveLibs}
          disabled={!libsDirty || savingLibs}
        >
          {savingLibs ? "…" : "Save libraries"}
        </button>
      </div>

      {/* Members */}
      <div className="cf-section-title">Members ({members.size})</div>
      {users.length === 0 ? (
        <p className="cf-faint" style={{ fontSize: 12 }}>No users to add yet.</p>
      ) : (
        <div className="cf-card" style={{ marginBottom: 0 }}>
          <table className="cf-table">
            <tbody>
              {users.map((u) => {
                const active = members.has(u.id);
                return (
                  <tr key={u.id}>
                    <td>
                      <div style={{ fontWeight: 600 }}>
                        {u.display_name ?? u.username}
                      </div>
                      <div className="cf-faint" style={{ fontSize: 11.5 }}>
                        @{u.username}
                        {u.role === "owner" && (
                          <span style={{ color: "var(--accent)" }}> · owner</span>
                        )}
                      </div>
                    </td>
                    <td className="cf-num">
                      <button
                        type="button"
                        role="switch"
                        aria-checked={active}
                        aria-label={`${active ? "Remove" : "Add"} ${u.display_name ?? u.username}`}
                        className={"cf-switch" + (active ? " cf-on" : "")}
                        onClick={() => toggleMember(u.id)}
                      />
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
      <div style={{ marginTop: 12 }}>
        <button
          type="button"
          className="cf-btn cf-primary cf-sm"
          onClick={saveMembers}
          disabled={!membersDirty || savingMembers}
        >
          {savingMembers ? "…" : "Save members"}
        </button>
      </div>
    </div>
  );
}

// Three-tier legend, shared visual language with the access matrix.
// Unbound = muted, View = amber (cf-warn), Full = green (cf-ok).
function AccessLevelLegend() {
  return (
    <div
      className="cf-flex cf-wrap cf-gap8"
      style={{ margin: "2px 0 10px", alignItems: "center" }}
    >
      <span className="cf-pill" style={{ padding: "1px 8px", fontSize: 10.5 }}>
        Unbound · hidden
      </span>
      <span
        className="cf-pill cf-warn"
        style={{ padding: "1px 8px", fontSize: 10.5 }}
      >
        View · browse only
      </span>
      <span
        className="cf-pill cf-ok"
        style={{ padding: "1px 8px", fontSize: 10.5 }}
      >
        Full · browse + play
      </span>
    </div>
  );
}

function setsEqual(a: Set<number>, b: Set<number>): boolean {
  if (a.size !== b.size) return false;
  for (const x of a) if (!b.has(x)) return false;
  return true;
}

// Build the per-library level map from a group detail. Prefers the
// `library_grants` (level-aware) list; falls back to treating bare
// `library_ids` as "full" for any older response shape.
function initialLibLevels(detail: AccessGroupDetail): Map<number, AccessLevel> {
  const m = new Map<number, AccessLevel>();
  if (detail.library_grants && detail.library_grants.length > 0) {
    for (const g of detail.library_grants) m.set(g.library_id, g.level);
  } else {
    for (const id of detail.library_ids) m.set(id, "full");
  }
  return m;
}

function levelMapsEqual(
  a: Map<number, AccessLevel>,
  b: Map<number, AccessLevel>,
): boolean {
  if (a.size !== b.size) return false;
  for (const [k, v] of a) if (b.get(k) !== v) return false;
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
    // No structured body — fall back to a human-friendly synonym keyed
    // off the status class. Operators still get the precise code in the
    // browser network panel; the message just stops reading as a raw
    // HTTP probe.
    if (e.status === 401 || e.status === 403)
      return "You don't have permission to do that.";
    if (e.status === 404) return "Not found.";
    if (e.status >= 500) return "Server error. Try again in a moment.";
    return "Couldn't save. Try again.";
  }
  return "Network error. Check your connection and try again.";
}

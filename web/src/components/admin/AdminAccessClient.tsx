"use client";

import { useMemo, useState } from "react";
import {
  admin as adminApi,
  type AccessLevel,
  type AccessMatrixEntry,
} from "@/lib/chimpflix-api";

interface Props {
  initial: AccessMatrixEntry[];
}

// Tri-state cycle: None → View → Full → None. Click advances the cell.
// "View" lets a user browse the library + its item metadata; "Full" adds
// playback. "None" hides the library entirely.
const CYCLE: AccessLevel[] = ["none", "view", "full"];
function nextLevel(level: AccessLevel): AccessLevel {
  const i = CYCLE.indexOf(level);
  return CYCLE[(i + 1) % CYCLE.length];
}

const LEVEL_LABEL: Record<AccessLevel, string> = {
  none: "None",
  view: "View",
  full: "Full",
};

// Click-to-cycle cell. Colors per level: full = ok (green), view = warn
// (amber), none = ghost. Inline since the mockup's `.acc` styling lives
// outside console.css.
function AccessCell({
  level,
  viaGroups,
  onCycle,
}: {
  level: AccessLevel;
  viaGroups: string[];
  onCycle: () => void;
}) {
  const base = "Direct grant — click to cycle None → View → Full.";
  const title =
    viaGroups.length > 0
      ? `${base} Also granted via group${viaGroups.length > 1 ? "s" : ""}: ${viaGroups.join(", ")}`
      : base;
  const palette: Record<AccessLevel, React.CSSProperties> = {
    full: {
      background: "var(--ok-soft)",
      color: "var(--ok)",
      borderColor: "rgba(52,211,153,.3)",
    },
    view: {
      background: "var(--warn-soft, rgba(245,200,80,.12))",
      color: "var(--warn)",
      borderColor: "rgba(245,200,80,.3)",
    },
    none: {
      background: "rgba(255,255,255,.04)",
      color: "var(--ghost)",
      borderColor: "var(--line)",
    },
  };
  const style: React.CSSProperties = {
    display: "inline-flex",
    alignItems: "center",
    justifyContent: "center",
    minWidth: 62,
    padding: "4px 10px",
    borderRadius: 7,
    fontSize: 11.5,
    fontWeight: 700,
    cursor: "pointer",
    border: "1px solid transparent",
    ...palette[level],
  };
  return (
    <div style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: 4 }}>
      <button type="button" onClick={onCycle} style={style} title={title}>
        {LEVEL_LABEL[level]}
      </button>
      {viaGroups.length > 0 && (
        <div className="cf-flex cf-wrap cf-gap8" style={{ justifyContent: "center", gap: 4 }}>
          {viaGroups.map((g) => (
            <span key={g} className="cf-pill cf-ok" style={{ padding: "1px 6px", fontSize: 10 }}>
              via {g}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}

// Build a 2D selector from the flat user × library matrix. Save commits
// per-library `set_library_access_levels` (tri-state `grants`) under a
// single bulk request.
export function AdminAccessClient({ initial }: Props) {
  // Baseline tracked in state (rather than reading `initial` directly)
  // so a successful save can update the dirty-check anchor without
  // mutating the prop array, which trips react-hooks/immutability.
  const [baseline, setBaseline] = useState(initial);
  const [entries, setEntries] = useState(initial);
  const [error, setError] = useState<string | null>(null);
  const [saveBusy, setSaveBusy] = useState(false);

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

  function cycle(userId: number, libraryId: number) {
    setEntries((all) =>
      all.map((e) => {
        if (e.user_id !== userId || e.library_id !== libraryId) return e;
        const level = nextLevel(e.level);
        return { ...e, level, allowed: level !== "none" };
      }),
    );
  }

  const dirty =
    JSON.stringify(entries.map((e) => ({ u: e.user_id, l: e.library_id, v: e.level }))) !==
    JSON.stringify(baseline.map((e) => ({ u: e.user_id, l: e.library_id, v: e.level })));

  async function save() {
    setError(null);
    setSaveBusy(true);
    try {
      // For each library, compute the desired per-user grants. Users left
      // at "none" are sent explicitly so the backend revokes them.
      const byLib = new Map<number, { user_id: number; level: AccessLevel }[]>();
      for (const e of entries) {
        if (!byLib.has(e.library_id)) byLib.set(e.library_id, []);
        byLib.get(e.library_id)!.push({ user_id: e.user_id, level: e.level });
      }
      const payload = Array.from(byLib, ([library_id, grants]) => ({
        library_id,
        grants,
      }));
      const r = await adminApi.access.put(payload);
      setEntries(r.entries);
      // Refresh the baseline so the dirty-check returns false after
      // the save lands.
      setBaseline(r.entries);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaveBusy(false);
    }
  }

  function discard() {
    setEntries(baseline);
  }

  if (users.length === 0 || libraries.length === 0) {
    return (
      <div className="cf-card" style={{ marginBottom: 0 }}>
        <div
          className="cf-card-body cf-pad cf-center cf-muted"
          style={{ fontSize: 13 }}
        >
          Need at least one non-owner user and one library to manage access.
        </div>
      </div>
    );
  }

  return (
    <div>
      <p className="cf-muted" style={{ marginTop: 0, fontSize: 13 }}>
        Click any cell to cycle access: <b>None</b> (hidden) → <b>View</b>{" "}
        (can browse, cannot play) → <b>Full</b> (browse + play). Rows are users,
        columns are libraries.
      </p>

      {error && (
        <div role="alert" aria-live="assertive" className="cf-banner cf-err">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 8v4M12 16v.5" />
          </svg>
          <div>{error}</div>
        </div>
      )}

      <div className="cf-card" style={{ marginBottom: 0, overflowX: "auto" }}>
        <table className="cf-table" style={{ textAlign: "center" }}>
          <thead>
            <tr>
              <th style={{ textAlign: "left" }}>User</th>
              {libraries.map((l) => (
                <th key={l.id} style={{ textAlign: "center" }}>
                  {l.name}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {users.map((u) => (
              <tr key={u.id}>
                <td style={{ textAlign: "left" }}>
                  <span className="cf-mono">@{u.username}</span>
                </td>
                {libraries.map((l) => {
                  const cell = cellFor(u.id, l.id);
                  return (
                    <td key={l.id} style={{ textAlign: "center" }}>
                      <AccessCell
                        level={cell?.level ?? "none"}
                        viaGroups={cell?.via_groups ?? []}
                        onCycle={() => cycle(u.id, l.id)}
                      />
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {dirty && (
        <div className="cf-savebar">
          <div className="cf-sb-status">
            <span className="cf-dot" style={{ background: "var(--warn)" }} />
            <span>
              <b>Unsaved changes</b> · library access grants
            </span>
          </div>
          <div className="cf-sb-actions">
            <button
              type="button"
              className="cf-btn cf-ghost cf-sm"
              onClick={discard}
              disabled={saveBusy}
            >
              Discard
            </button>
            <button
              type="button"
              className="cf-btn cf-primary cf-sm"
              onClick={save}
              disabled={saveBusy}
            >
              {saveBusy ? "Saving…" : "Save changes"}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

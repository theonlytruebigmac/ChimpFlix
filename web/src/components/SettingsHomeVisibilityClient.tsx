"use client";

import { useEffect, useRef, useState } from "react";
import {
  auth as authApi,
  prefs as prefsApi,
  HOME_RAIL_CATALOGUE,
  type Library,
  type LibraryKind,
  type User,
} from "@/lib/chimpflix-api";

/// Home & visibility — the personal "You" surface that controls which
/// libraries appear on the home page + browse rails, the order + on/off
/// state of the home rails themselves, and two browse filters (hide watched
/// from Continue Watching, kids-safe). Rebuilt in the console design language
/// to match docs/redesign/home-visibility.html.
///
/// Persistence splits across two endpoints, by design:
///   * Hidden libraries → PUT /auth/me/hidden-libraries (prefs API). The
///     mockup's per-library switch is ON when the library is SHOWN.
///   * Rail order + the kids-safe switch → PATCH /auth/me (auth API):
///       - `home_rails_json`  the full ordered rail layout overlay,
///       - `kids_safe`        the kid-safe browse filter.
///
/// NOTE: the old "Hide watched from Continue Watching" switch (DB column
/// `hide_watched_cw`) was removed — it was a guaranteed no-op because the
/// on-deck query already excludes finished titles. The column stays in the
/// schema (harmless, unread); the UI now just states this behavior is
/// automatic.
///
/// Saves fire immediately on each change (optimistic), with a sticky savebar
/// reflecting saved / saving / error. The default home experience is never
/// regressed: an untouched profile keeps `home_rails_json = "[]"` (stock),
/// and we only ever PATCH the field that changed.
interface RailRow {
  rail_id: string;
  label: string;
  enabled: boolean;
}

type SaveState =
  | { kind: "saved" }
  | { kind: "saving" }
  | { kind: "error"; message: string };

interface Props {
  user: User;
  libraries: Library[];
}

/// Build the initial rail rows by merging the user's saved overlay over the
/// default catalogue, mirroring the sparse-overlay merge in
/// web/src/app/page.tsx::orderRailIds:
///   1. rails the user explicitly ordered come first, in their order;
///   2. remaining catalogue rails keep their default relative position;
///   3. a rail's enabled flag defaults to ON unless the overlay set it false.
function railRowsFromOverlay(homeRailsJson: string): RailRow[] {
  const byId = new Map(HOME_RAIL_CATALOGUE.map((e) => [e.rail_id, e.label]));
  let overlay: Array<{ rail_id: string; enabled: boolean }> = [];
  try {
    const parsed: unknown = JSON.parse(homeRailsJson);
    if (Array.isArray(parsed)) {
      for (const entry of parsed) {
        if (
          entry &&
          typeof entry === "object" &&
          typeof (entry as { rail_id?: unknown }).rail_id === "string"
        ) {
          const id = (entry as { rail_id: string }).rail_id;
          if (!byId.has(id)) continue; // forward-compat: drop unknown ids
          overlay.push({
            rail_id: id,
            enabled: (entry as { enabled?: unknown }).enabled !== false,
          });
        }
      }
    }
  } catch {
    overlay = [];
  }

  const enabledById = new Map(overlay.map((o) => [o.rail_id, o.enabled]));
  const rows: RailRow[] = [];
  const placed = new Set<string>();
  // 1. user-ordered rails first
  for (const o of overlay) {
    if (placed.has(o.rail_id)) continue;
    placed.add(o.rail_id);
    rows.push({ rail_id: o.rail_id, label: byId.get(o.rail_id)!, enabled: o.enabled });
  }
  // 2. remaining catalogue rails in default order
  for (const e of HOME_RAIL_CATALOGUE) {
    if (placed.has(e.rail_id)) continue;
    rows.push({
      rail_id: e.rail_id,
      label: e.label,
      enabled: enabledById.get(e.rail_id) ?? true,
    });
  }
  return rows;
}

/// Serialize the working rail rows into a `home_rails_json` overlay. We emit
/// the FULL ordered list (every catalogue rail, with its enabled flag) so the
/// stored order is unambiguous and reordering is lossless. An array equal to
/// the default catalogue order with everything enabled is still a valid
/// overlay; the home page renders it identically to the stock default.
function railRowsToJson(rows: RailRow[]): string {
  return JSON.stringify(
    rows.map((r) => ({ rail_id: r.rail_id, enabled: r.enabled })),
  );
}

const LIBRARY_KIND_LABEL: Record<LibraryKind, string> = {
  movies: "Movies",
  shows: "Shows",
  anime: "Anime",
};

export function SettingsHomeVisibilityClient({ user, libraries }: Props) {
  // ── hidden libraries (separate endpoint) ──────────────────────────────
  // `null` until the first prefs fetch resolves. Stored as the HIDDEN set;
  // the switch shows the inverse (ON = shown).
  const [hidden, setHidden] = useState<Set<number> | null>(null);

  // ── rails + filters (PATCH /auth/me) ──────────────────────────────────
  const [rails, setRails] = useState<RailRow[]>(() =>
    railRowsFromOverlay(user.home_rails_json),
  );
  const [kidsSafe, setKidsSafe] = useState(user.kids_safe);

  const [save, setSave] = useState<SaveState>({ kind: "saved" });
  const [busy, setBusy] = useState(false);

  // Coalesce the "saving → saved" flash so a burst of rapid toggles doesn't
  // strobe the savebar. The latest in-flight request wins.
  const reqSeq = useRef(0);

  useEffect(() => {
    let cancelled = false;
    prefsApi
      .hiddenLibraries()
      .then((res) => {
        if (!cancelled) setHidden(new Set(res.library_ids));
      })
      .catch(() => {
        if (!cancelled) setHidden(new Set());
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function runSave(fn: () => Promise<unknown>) {
    const seq = ++reqSeq.current;
    setBusy(true);
    setSave({ kind: "saving" });
    try {
      await fn();
      if (reqSeq.current === seq) setSave({ kind: "saved" });
    } catch {
      if (reqSeq.current === seq)
        setSave({ kind: "error", message: "Couldn't save. Try again." });
    } finally {
      if (reqSeq.current === seq) setBusy(false);
    }
  }

  // ── hidden libraries ──────────────────────────────────────────────────
  function toggleLibrary(id: number) {
    if (!hidden) return;
    const next = new Set(hidden);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    const prev = hidden;
    setHidden(next);
    void runSave(async () => {
      try {
        await prefsApi.setHiddenLibraries([...next]);
      } catch (e) {
        setHidden(prev); // roll back the optimistic flip
        throw e;
      }
    });
  }

  // ── rails ─────────────────────────────────────────────────────────────
  function persistRails(next: RailRow[]) {
    const prev = rails;
    setRails(next);
    void runSave(async () => {
      try {
        await authApi.updateMe({ home_rails_json: railRowsToJson(next) });
      } catch (e) {
        setRails(prev);
        throw e;
      }
    });
  }

  function toggleRail(railId: string) {
    persistRails(
      rails.map((r) =>
        r.rail_id === railId ? { ...r, enabled: !r.enabled } : r,
      ),
    );
  }

  function moveRail(index: number, dir: -1 | 1) {
    const target = index + dir;
    if (target < 0 || target >= rails.length) return;
    const next = rails.slice();
    [next[index], next[target]] = [next[target], next[index]];
    persistRails(next);
  }

  // ── filters ───────────────────────────────────────────────────────────
  function toggleKidsSafe() {
    const next = !kidsSafe;
    const prev = kidsSafe;
    setKidsSafe(next);
    void runSave(async () => {
      try {
        await authApi.updateMe({ kids_safe: next });
      } catch (e) {
        setKidsSafe(prev);
        throw e;
      }
    });
  }

  // ── reset to defaults ─────────────────────────────────────────────────
  // Restores the stock home: every library shown, the default rail order
  // with all rails enabled (persisted as `[]` so the home page takes its
  // byte-for-byte default path), and the kids-safe filter off. Hidden
  // libraries and the /auth/me fields persist independently, so we issue
  // both writes.
  function resetDefaults() {
    const prevHidden = hidden;
    const prevRails = rails;
    const prevKids = kidsSafe;
    setHidden(new Set());
    setRails(railRowsFromOverlay("[]"));
    setKidsSafe(false);
    void runSave(async () => {
      try {
        await Promise.all([
          prefsApi.setHiddenLibraries([]),
          authApi.updateMe({
            home_rails_json: "",
            kids_safe: false,
          }),
        ]);
      } catch (e) {
        setHidden(prevHidden);
        setRails(prevRails);
        setKidsSafe(prevKids);
        throw e;
      }
    });
  }

  return (
    <div>
      {/* ── libraries on my home ───────────────────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Libraries on my home</div>
            <div className="cf-sub">
              Hidden libraries are excluded from your home page and browse
              rails.
            </div>
          </div>
          <div className="cf-head-aside">
            <span className="cf-pill cf-info">
              <svg
                style={{ width: 13, height: 13 }}
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                aria-hidden
              >
                <path d="M2 12s4-7 10-7 10 7 10 7-4 7-10 7-10-7-10-7z" />
                <circle cx="12" cy="12" r="3" />
              </svg>
              Just for you
            </span>
          </div>
        </div>
        <div className="cf-card-body">
          {libraries.length === 0 ? (
            <div className="cf-row">
              <div className="cf-row-main">
                <div className="cf-row-help">
                  Add a library first to choose which ones appear on your home.
                </div>
              </div>
            </div>
          ) : (
            libraries.map((lib) => {
              const isHidden = hidden?.has(lib.id) ?? false;
              const on = hidden !== null && !isHidden;
              return (
                <div className="cf-row" key={lib.id}>
                  <div className="cf-row-main">
                    <div className="cf-row-label">
                      {lib.name}{" "}
                      <span
                        className="cf-muted"
                        style={{ fontWeight: 400, fontSize: "12.5px" }}
                      >
                        · {LIBRARY_KIND_LABEL[lib.kind]}
                      </span>
                    </div>
                  </div>
                  <div className="cf-row-control">
                    <button
                      type="button"
                      role="switch"
                      aria-checked={on}
                      aria-label={`Show ${lib.name} on my home`}
                      disabled={hidden === null || busy}
                      onClick={() => toggleLibrary(lib.id)}
                      className={"cf-switch" + (on ? " cf-on" : "")}
                    />
                  </div>
                </div>
              );
            })
          )}
        </div>
      </div>

      {/* ── home rails ─────────────────────────────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Home rails</div>
            <div className="cf-sub">
              Use the arrows to reorder. Rails play top-to-bottom on your home
              page — turn off the ones you don&rsquo;t use.
            </div>
          </div>
        </div>
        <div className="cf-card-body">
          {rails.map((rail, i) => (
            <div className="cf-row" key={rail.rail_id}>
              <div className="cf-row-main cf-flex cf-gap12">
                <div className="cf-row-label" style={{ margin: 0 }}>
                  {rail.label}
                </div>
              </div>
              <div className="cf-row-control">
                <div className="cf-flex cf-gap8">
                  <button
                    type="button"
                    className="cf-btn cf-ghost cf-tiny"
                    aria-label={`Move ${rail.label} up`}
                    disabled={i === 0 || busy}
                    onClick={() => moveRail(i, -1)}
                  >
                    ↑
                  </button>
                  <button
                    type="button"
                    className="cf-btn cf-ghost cf-tiny"
                    aria-label={`Move ${rail.label} down`}
                    disabled={i === rails.length - 1 || busy}
                    onClick={() => moveRail(i, 1)}
                  >
                    ↓
                  </button>
                </div>
                <button
                  type="button"
                  role="switch"
                  aria-checked={rail.enabled}
                  aria-label={`Show the ${rail.label} rail`}
                  disabled={busy}
                  onClick={() => toggleRail(rail.rail_id)}
                  className={"cf-switch" + (rail.enabled ? " cf-on" : "")}
                />
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* ── filters ────────────────────────────────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Filters</div>
            <div className="cf-sub">
              Fine-tune what shows up while you browse on this profile.
            </div>
          </div>
        </div>
        <div className="cf-card-body">
          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">
                Default to Kids-safe browsing on this profile
              </div>
              <div className="cf-row-help">
                Hide titles with an explicit mature rating from home and browse.
                Unrated titles still appear, so this has no effect until your
                library&rsquo;s age ratings are filled in. Hidden libraries stay
                hidden regardless.
              </div>
            </div>
            <div className="cf-row-control">
              <button
                type="button"
                role="switch"
                aria-checked={kidsSafe}
                aria-label="Default to Kids-safe browsing on this profile"
                disabled={busy}
                onClick={toggleKidsSafe}
                className={"cf-switch" + (kidsSafe ? " cf-on" : "")}
              />
            </div>
          </div>
        </div>
      </div>

      {/* ── savebar ────────────────────────────────────────────────────── */}
      <div className="cf-savebar">
        <div className="cf-sb-status">
          {save.kind === "error" ? (
            <>
              <span
                className="cf-dot"
                style={{ background: "var(--err)" }}
              />
              {save.message}
            </>
          ) : save.kind === "saving" ? (
            <>
              <span
                className="cf-dot"
                style={{ background: "var(--warn)" }}
              />
              Saving…
            </>
          ) : (
            <>
              <span className="cf-dot" style={{ background: "var(--ok)" }} />
              All changes saved
            </>
          )}
        </div>
        <div className="cf-sb-actions">
          <button
            type="button"
            className="cf-btn cf-ghost cf-sm"
            disabled={busy || hidden === null}
            onClick={resetDefaults}
          >
            Reset to defaults
          </button>
        </div>
      </div>
    </div>
  );
}

"use client";

import { useEffect, useRef, useState } from "react";
import { admin as adminApi, type LogLine } from "@/lib/chimpflix-api";

const REFRESH_INTERVAL_MS = 3_000;

const LEVELS = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"] as const;

export function AdminLogsClient({ initial }: { initial: LogLine[] }) {
  const [lines, setLines] = useState(initial);
  const [level, setLevel] = useState<string>("INFO");
  const [paused, setPaused] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Client-side module/target filter. Logs are an in-memory ring on the
  // server; filtering by `target` here avoids a backend round-trip and
  // keeps the live tail responsive. Case-insensitive substring match.
  const [moduleFilter, setModuleFilter] = useState("");
  // Mirror `paused` into a ref so the polling interval (which runs with
  // an empty dep array to avoid re-creating the timer every render) can
  // read the latest value. The ref writes have to live in an effect;
  // assigning .current during render violates React purity.
  const pausedRef = useRef(paused);
  useEffect(() => {
    pausedRef.current = paused;
  }, [paused]);

  // Fetch immediately whenever `level` changes (the seg), and re-arm a
  // polling tick that uses the same level. Without this the seg sat on
  // stale results for up to one REFRESH_INTERVAL_MS tick — making it
  // look like the filter wasn't doing anything.
  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | null = null;
    async function fetchOnce() {
      try {
        const r = await adminApi.logs({ level, limit: 200 });
        if (cancelled) return;
        setLines(r.lines);
        setError(null);
      } catch (e) {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
      }
    }
    void fetchOnce();
    function schedule() {
      timer = setTimeout(async () => {
        if (cancelled) return;
        if (!pausedRef.current) await fetchOnce();
        if (!cancelled) schedule();
      }, REFRESH_INTERVAL_MS);
    }
    schedule();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [level]);

  // The "Auto-scroll" toggle drives the existing pause logic: on = live
  // (following / polling), off = paused. Same single boolean, just
  // surfaced as a switch to match the console design language.
  const following = !paused;

  // Apply the module filter to the already-fetched lines. Both the
  // rendered list and the export operate on this filtered view so what
  // you download matches what you see.
  const filter = moduleFilter.trim().toLowerCase();
  const shown = filter
    ? lines.filter((l) => l.target.toLowerCase().includes(filter))
    : lines;

  // Export the currently-shown lines as a plain-text log file. No backend
  // call — we serialize the in-memory `shown` array client-side and
  // synthesize a download click.
  function exportLog() {
    const body = shown
      .map(
        (l) =>
          `${new Date(l.timestamp_ms).toISOString()} ${l.level.toUpperCase().padEnd(5)} ${l.target}  ${l.message}`,
      )
      .join("\n");
    const blob = new Blob([body + (body ? "\n" : "")], {
      type: "text/plain;charset=utf-8",
    });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `chimpflix-logs-${new Date()
      .toISOString()
      .replace(/[:.]/g, "-")}.log`;
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
  }

  return (
    <div>
      {/* ── controls: level seg (left) + auto-scroll toggle (right) ──── */}
      <div
        className="cf-flex cf-between cf-wrap cf-gap8"
        style={{ marginBottom: 14 }}
      >
        <div className="cf-seg" role="group" aria-label="Minimum log level">
          {LEVELS.map((lvl) => (
            <button
              key={lvl}
              type="button"
              aria-pressed={level === lvl}
              onClick={() => setLevel(lvl)}
              className={level === lvl ? "cf-on" : undefined}
            >
              {lvl}
            </button>
          ))}
        </div>
        <div className="cf-flex cf-gap8" style={{ alignItems: "center" }}>
          <input
            type="text"
            className="cf-input"
            placeholder="Filter by module…"
            aria-label="Filter by module"
            value={moduleFilter}
            onChange={(e) => setModuleFilter(e.target.value)}
            style={{ minWidth: 180 }}
          />
          <button
            type="button"
            className="cf-btn"
            onClick={exportLog}
            disabled={shown.length === 0}
            title="Download the currently-shown log lines"
          >
            Export
          </button>
          <button
            type="button"
            role="switch"
            aria-checked={following}
            aria-label="Auto-scroll"
            onClick={() => setPaused((v) => !v)}
            className={`cf-switch${following ? " cf-on" : ""}`}
          />
          <span
            className="cf-faint"
            style={{ fontSize: 12, whiteSpace: "nowrap" }}
          >
            Auto-scroll
          </span>
        </div>
      </div>

      {/* ── live log card ─────────────────────────────────────────────── */}
      <div className="cf-card" style={{ marginBottom: 0 }}>
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Live log</div>
            <div className="cf-sub">
              tail -f · {shown.length} line{shown.length === 1 ? "" : "s"}
              {filter && lines.length !== shown.length
                ? ` (filtered from ${lines.length})`
                : ""}
            </div>
          </div>
          <div className="cf-head-aside">
            {error && (
              <span className="cf-pill cf-err">
                <span className="cf-dot" />
                {error}
              </span>
            )}
            {following ? (
              <span className="cf-pill cf-accent">
                <span className="cf-dot" />
                following
              </span>
            ) : (
              <span className="cf-pill cf-warn">
                <span className="cf-dot" />
                paused
              </span>
            )}
          </div>
        </div>
        <div className="cf-card-body cf-pad">
          <div
            style={{
              fontFamily: "ui-monospace, 'SFMono-Regular', Menlo, monospace",
              fontSize: 12,
              lineHeight: 1.7,
              maxHeight: "70vh",
              overflowY: "auto",
            }}
          >
            {shown.length === 0 ? (
              <div className="cf-faint">
                {filter ? "No lines match that module." : "No log lines yet."}
              </div>
            ) : (
              shown.map((l, i) => (
                <div
                  key={`${l.timestamp_ms}-${i}`}
                  className="cf-flex cf-gap8"
                  style={{ alignItems: "baseline", padding: "1px 0" }}
                >
                  <span className="cf-faint" style={{ flex: "none" }}>
                    {formatTime(l.timestamp_ms)}
                  </span>
                  <span style={{ flex: "none" }}>
                    <LevelPill level={l.level} />
                  </span>
                  <span
                    style={{ flex: "none", color: "var(--violet)" }}
                  >
                    {l.target}
                  </span>
                  <span
                    className="cf-muted"
                    style={{
                      minWidth: 0,
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                      whiteSpace: "nowrap",
                    }}
                  >
                    {l.message}
                  </span>
                </div>
              ))
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

/// Log level rendered as a console `cf-pill` with the level-appropriate
/// tone. Tightened padding so the inline tag sits cleanly in the line.
function LevelPill({ level }: { level: string }) {
  const tone = levelTone(level);
  return (
    <span className={`cf-pill${tone}`} style={{ padding: "1px 7px" }}>
      {level.toUpperCase()}
    </span>
  );
}

function levelTone(level: string): string {
  switch (level.toUpperCase()) {
    case "ERROR":
      return " cf-err";
    case "WARN":
      return " cf-warn";
    case "INFO":
      return " cf-info";
    case "DEBUG":
      return " cf-info";
    default:
      return "";
  }
}

function formatTime(epochMs: number): string {
  const d = new Date(epochMs);
  return d.toLocaleTimeString(undefined, { hour12: false });
}

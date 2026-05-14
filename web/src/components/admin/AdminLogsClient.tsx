"use client";

import { useEffect, useRef, useState } from "react";
import { admin as adminApi, type LogLine } from "@/lib/chimpflix-api";

const REFRESH_INTERVAL_MS = 3_000;

export function AdminLogsClient({ initial }: { initial: LogLine[] }) {
  const [lines, setLines] = useState(initial);
  const [level, setLevel] = useState<string>("INFO");
  const [paused, setPaused] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const levelRef = useRef(level);
  const pausedRef = useRef(paused);
  levelRef.current = level;
  pausedRef.current = paused;

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | null = null;
    async function tick() {
      if (!pausedRef.current) {
        try {
          const r = await adminApi.logs({ level: levelRef.current, limit: 200 });
          if (cancelled) return;
          setLines(r.lines);
          setError(null);
        } catch (e) {
          if (cancelled) return;
          setError(e instanceof Error ? e.message : String(e));
        }
      }
      if (!cancelled) timer = setTimeout(tick, REFRESH_INTERVAL_MS);
    }
    timer = setTimeout(tick, REFRESH_INTERVAL_MS);
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, []);

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-3">
        <label className="text-xs text-white/50">Min level</label>
        <select
          value={level}
          onChange={(e) => setLevel(e.target.value)}
          className="rounded-md border border-white/10 bg-black/30 px-3 py-1.5 text-sm outline-none focus:border-white/30"
        >
          <option value="TRACE">TRACE</option>
          <option value="DEBUG">DEBUG</option>
          <option value="INFO">INFO</option>
          <option value="WARN">WARN</option>
          <option value="ERROR">ERROR</option>
        </select>
        <button
          onClick={() => setPaused((v) => !v)}
          className={`rounded border px-3 py-1 text-xs ${paused ? "border-amber-500/40 text-amber-300" : "border-white/15 text-white/70"}`}
        >
          {paused ? "Resume" : "Pause"}
        </button>
        <span className="text-xs text-white/40">
          {lines.length} line{lines.length === 1 ? "" : "s"}
        </span>
        {error && <span className="text-xs text-red-400">{error}</span>}
      </div>

      <div className="max-h-[70vh] overflow-y-auto rounded-lg border border-white/10 bg-black/40 p-3 font-mono text-xs">
        {lines.length === 0 ? (
          <div className="text-white/40">No log lines yet.</div>
        ) : (
          lines.map((l, i) => (
            <div
              key={`${l.timestamp_ms}-${i}`}
              className="flex gap-2 whitespace-pre-wrap"
            >
              <span className="shrink-0 text-white/40">
                {formatTime(l.timestamp_ms)}
              </span>
              <span className={`shrink-0 ${levelColor(l.level)}`}>
                {l.level.padEnd(5)}
              </span>
              <span className="shrink-0 text-white/50">{l.target}</span>
              <span className="text-white/80">{l.message}</span>
            </div>
          ))
        )}
      </div>
    </div>
  );
}

function levelColor(level: string): string {
  switch (level.toUpperCase()) {
    case "ERROR":
      return "text-red-400";
    case "WARN":
      return "text-amber-300";
    case "INFO":
      return "text-emerald-300";
    case "DEBUG":
      return "text-blue-300";
    default:
      return "text-white/40";
  }
}

function formatTime(epochMs: number): string {
  const d = new Date(epochMs);
  return d.toLocaleTimeString(undefined, { hour12: false });
}

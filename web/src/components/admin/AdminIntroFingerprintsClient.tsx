"use client";

import { useState } from "react";
import {
  admin as adminApi,
  type ShowIntroFingerprintListing,
} from "@/lib/chimpflix-api";
import { Pill } from "./ui";

/// Owner-only maintenance page: lists every captured intro
/// fingerprint with show title, source (manual vs auto), capture
/// time, and a per-row Clear button. Drives operator audit of
/// "what does the detector think the intros are for".
///
/// Confirms before clearing. The clear is destructive — next
/// detect-markers run on the show falls back to blackdetect (or an
/// auto-recapture from chapter metadata if that's available) until
/// a fresh capture seeds the show again.
export function AdminIntroFingerprintsClient({
  initial,
}: {
  initial: ShowIntroFingerprintListing[];
}) {
  // Rows + the wall-clock snapshot used by the relative-time column
  // are tracked together so the time anchor refreshes whenever the
  // list does — and we never call Date.now() during render (which
  // trips react-hooks/purity). The initial nowMs is 0 because we
  // don't have a snapshot from the server's render pass; the column
  // falls back to a static `toLocaleDateString` until the first
  // refresh sets a real timestamp.
  const [state, setState] = useState<{
    rows: ShowIntroFingerprintListing[];
    nowMs: number;
  }>({ rows: initial, nowMs: 0 });
  const rows = state.rows;
  const nowMs = state.nowMs;
  const [busyShowId, setBusyShowId] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function refresh() {
    try {
      const r = await adminApi.introFingerprints.list();
      // Date.now() inside an async event handler is fine, but
      // react-hooks/purity flags component-scope async functions
      // conservatively — same pattern + disable used in
      // AdminDevicesClient. Only runs after Clear succeeds.
      // eslint-disable-next-line react-hooks/purity
      setState({ rows: r.fingerprints, nowMs: Date.now() });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  async function clearForShow(showId: number, title: string) {
    if (
      !window.confirm(
        `Clear the intro fingerprint for "${title}"? Future detect-markers runs will fall back to blackdetect (or re-seed automatically from chapter metadata if available) until a new capture lands.`,
      )
    ) {
      return;
    }
    setBusyShowId(showId);
    setError(null);
    try {
      await adminApi.introFingerprints.deleteForShow(showId);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyShowId(null);
    }
  }

  if (rows.length === 0) {
    return (
      <div className="rounded-lg border border-dashed border-white/15 bg-white/2 p-8 text-center text-sm text-white/55">
        <p className="m-0 mb-2 text-white/75">No fingerprints captured yet.</p>
        <p className="m-0 text-xs text-white/45">
          Chapter-derived intros auto-capture during the next{" "}
          <code className="font-mono">detect_markers</code> run. For shows
          without chapter metadata, save a manual{" "}
          <code className="font-mono">intro</code> marker on episode 1 from
          its title modal — the fingerprint is captured in the background
          and used to anchor every other episode of the show.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {error && (
        <div className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {error}
        </div>
      )}

      <div className="overflow-hidden rounded-lg border border-white/10 bg-white/2">
        <table className="w-full text-sm">
          <thead className="bg-white/4 text-left text-[11.5px] uppercase tracking-wider text-white/45">
            <tr>
              <th className="px-4 py-2 font-semibold">Show</th>
              <th className="w-24 px-4 py-2 font-semibold whitespace-nowrap">
                Source
              </th>
              <th className="w-28 px-4 py-2 font-semibold whitespace-nowrap">
                Sample
              </th>
              <th className="w-32 px-4 py-2 font-semibold whitespace-nowrap">
                Captured
              </th>
              <th className="w-24 px-4 py-2"></th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.id} className="border-t border-white/6">
                <td className="px-4 py-3">
                  <div className="text-[13.5px] font-medium">
                    {r.show_title}
                  </div>
                  <div className="text-[11.5px] text-white/45">
                    show #{r.show_id}
                    {r.season_number != null && (
                      <> · season {r.season_number}</>
                    )}
                    {r.captured_from_media_file_id != null && (
                      <> · from file #{r.captured_from_media_file_id}</>
                    )}
                  </div>
                </td>
                <td className="whitespace-nowrap px-4 py-3">
                  {r.captured_by === "manual" ? (
                    <Pill tone="ok">manual</Pill>
                  ) : (
                    <Pill tone="info">auto</Pill>
                  )}
                </td>
                <td className="whitespace-nowrap px-4 py-3 tabular-nums text-[12.5px] text-white/70">
                  {formatSeconds(r.duration_ms / 1000)}
                </td>
                <td className="whitespace-nowrap px-4 py-3 text-[12.5px] text-white/65">
                  {nowMs > 0
                    ? relativeSince(r.captured_at, nowMs)
                    : new Date(r.captured_at).toLocaleDateString()}
                </td>
                <td className="whitespace-nowrap px-4 py-3 text-right">
                  <button
                    type="button"
                    onClick={() => clearForShow(r.show_id, r.show_title)}
                    disabled={busyShowId === r.show_id}
                    className="rounded border border-red-500/30 px-2 py-1 text-xs text-red-300 hover:bg-red-500/10 disabled:opacity-50"
                  >
                    {busyShowId === r.show_id ? "Clearing…" : "Clear"}
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <p className="text-xs text-white/45">
        {rows.length} captured fingerprint{rows.length === 1 ? "" : "s"} ·
        Clearing forces the next detect-markers run to re-derive the intro
        for that show, either from chapter metadata or blackdetect.
      </p>
    </div>
  );
}

function relativeSince(epochMs: number, nowMs: number): string {
  const diff = nowMs - epochMs;
  if (diff < 60_000) return "just now";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} min ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  return `${Math.floor(diff / 86_400_000)}d ago`;
}

function formatSeconds(s: number): string {
  if (!Number.isFinite(s) || s < 0) return "—";
  const total = Math.floor(s);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const sec = total % 60;
  if (h > 0) {
    return `${h}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`;
  }
  return `${m}:${String(sec).padStart(2, "0")}`;
}

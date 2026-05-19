"use client";

import { useState } from "react";
import {
  admin as adminApi,
  type AuditListResponse,
  type AuditLogEntry,
} from "@/lib/chimpflix-api";

interface Props {
  initial: AuditListResponse;
}

export function AdminAuditClient({ initial }: Props) {
  const [entries, setEntries] = useState<AuditLogEntry[]>(initial.entries);
  const [nextBefore, setNextBefore] = useState<number | null>(
    initial.next_before,
  );
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function loadMore() {
    if (nextBefore == null) return;
    setLoading(true);
    setError(null);
    try {
      const page = await adminApi.audit.list({ before: nextBefore, limit: 50 });
      setEntries((prev) => [...prev, ...page.entries]);
      setNextBefore(page.next_before);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }

  if (entries.length === 0) {
    return (
      <div className="rounded-lg border border-dashed border-white/15 bg-white/2 p-12 text-center text-sm text-white/50">
        No admin actions recorded yet.
      </div>
    );
  }

  return (
    <div className="overflow-hidden rounded-lg border border-white/10">
      <table className="w-full text-sm">
        <thead className="bg-white/5 text-left text-xs uppercase tracking-wider text-white/40">
          <tr>
            <th className="px-4 py-2">When</th>
            <th className="px-4 py-2">Action</th>
            <th className="px-4 py-2">Target</th>
            <th className="px-4 py-2">Actor</th>
            <th className="px-4 py-2">Payload</th>
          </tr>
        </thead>
        <tbody>
          {entries.map((e) => (
            <tr key={e.id} className="border-t border-white/5">
              <td className="whitespace-nowrap px-4 py-2 align-top text-white/60">
                {formatWhen(e.created_at)}
              </td>
              <td className="whitespace-nowrap px-4 py-2 align-top font-mono text-xs">
                {e.action}
              </td>
              <td className="whitespace-nowrap px-4 py-2 align-top text-white/70">
                {e.target_kind ?? "—"}
                {e.target_id ? (
                  <span className="text-white/40"> #{e.target_id}</span>
                ) : null}
              </td>
              <td className="whitespace-nowrap px-4 py-2 align-top text-white/70">
                {e.actor_user_id != null ? `user ${e.actor_user_id}` : "—"}
              </td>
              <td className="px-4 py-2 align-top text-xs">
                <Payload raw={e.payload_json} />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <div className="flex items-center justify-between gap-2 border-t border-white/5 bg-white/2 px-4 py-2 text-xs text-white/50">
        <span>{entries.length} entries</span>
        {nextBefore != null && (
          <button
            onClick={loadMore}
            disabled={loading}
            className="rounded border border-white/10 px-2 py-1 text-white/70 hover:bg-white/5 disabled:opacity-40"
          >
            {loading ? "Loading…" : "Load more"}
          </button>
        )}
        {error && <span className="text-red-400">{error}</span>}
      </div>
    </div>
  );
}

function Payload({ raw }: { raw: string | null }) {
  if (!raw) return <span className="text-white/30">—</span>;
  // Try to pretty-print JSON; fall back to raw text. Parse outside
  // the JSX construction so a parser error can't be confused for a
  // render error by react-hooks/error-boundaries.
  let pretty: string | null = null;
  try {
    pretty = JSON.stringify(JSON.parse(raw), null, 2);
  } catch {
    // Not JSON — render raw.
  }
  if (pretty !== null) {
    return (
      <pre className="max-w-md overflow-x-auto whitespace-pre-wrap wrap-break-word font-mono text-white/60">
        {pretty}
      </pre>
    );
  }
  return <code className="font-mono text-white/60">{raw}</code>;
}

function formatWhen(epochMs: number): string {
  const d = new Date(epochMs);
  return d.toLocaleString();
}

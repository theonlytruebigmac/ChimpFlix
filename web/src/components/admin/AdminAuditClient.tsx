"use client";

import { useEffect, useState } from "react";
import {
  admin as adminApi,
  type AuditListResponse,
  type AuditLogEntry,
} from "@/lib/chimpflix-api";
import { DEFAULT_PAGE_SIZE, Pagination } from "./ui";
import { LoadingPlaceholder } from "../ui/LoadingPlaceholder";

interface Props {
  initial: AuditListResponse;
}

export function AdminAuditClient({ initial }: Props) {
  const [entries, setEntries] = useState<AuditLogEntry[]>(initial.entries);
  const [total, setTotal] = useState<number>(initial.total);
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(DEFAULT_PAGE_SIZE);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Refetch whenever page or page-size changes. The server-rendered
  // initial page (limit=50) is good enough for first paint; once
  // the operator clicks a page button we take over with offset/limit.
  // The setLoading/setError calls are React's documented
  // "synchronise with external state" pattern — the URL/page/size
  // is the input, the fetched entries are the output. Inline the
  // disable so the rule doesn't flag this legitimate use.
  useEffect(() => {
    let cancelled = false;
    /* eslint-disable react-hooks/set-state-in-effect */
    setLoading(true);
    setError(null);
    /* eslint-enable react-hooks/set-state-in-effect */
    adminApi.audit
      .list({ limit: pageSize, offset: (page - 1) * pageSize })
      .then((res) => {
        if (cancelled) return;
        setEntries(res.entries);
        setTotal(res.total);
      })
      .catch((e) => {
        if (!cancelled) {
          setError(e instanceof Error ? e.message : String(e));
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [page, pageSize]);

  if (entries.length === 0 && page === 1 && total === 0) {
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
      <div className="border-t border-white/5 bg-white/2 px-4 py-2">
        {error && (
          <div className="mb-2 text-xs text-red-400">{error}</div>
        )}
        <Pagination
          page={page}
          pageSize={pageSize}
          total={total}
          onPageChange={setPage}
          onPageSizeChange={(s) => {
            setPageSize(s);
            setPage(1);
          }}
          noun="entries"
          leading={loading ? <LoadingPlaceholder variant="inline" /> : undefined}
        />
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

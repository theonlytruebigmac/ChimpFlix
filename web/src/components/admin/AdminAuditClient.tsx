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

  // ── Filters ────────────────────────────────────────────────────────────
  // `action` is the live text-input value; `from`/`to` are `YYYY-MM-DD`
  // date-input strings. `applied` is the snapshot the fetch actually uses
  // — committed via the form submit / Apply button so we don't refetch on
  // every keystroke. Changing filters resets to page 1.
  const [actionInput, setActionInput] = useState("");
  const [fromInput, setFromInput] = useState("");
  const [toInput, setToInput] = useState("");
  const [applied, setApplied] = useState<{
    action: string;
    from: string;
    to: string;
  }>({ action: "", from: "", to: "" });
  const [exporting, setExporting] = useState(false);

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
      .list({
        limit: pageSize,
        offset: (page - 1) * pageSize,
        action: applied.action || undefined,
        from: dateInputToEpochMs(applied.from, "start"),
        to: dateInputToEpochMs(applied.to, "end"),
      })
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
  }, [page, pageSize, applied]);

  function applyFilters() {
    setApplied({ action: actionInput.trim(), from: fromInput, to: toInput });
    setPage(1);
  }

  function clearFilters() {
    setActionInput("");
    setFromInput("");
    setToInput("");
    setApplied({ action: "", from: "", to: "" });
    setPage(1);
  }

  async function exportCsv() {
    setExporting(true);
    setError(null);
    try {
      await adminApi.audit.exportCsv({
        action: applied.action || undefined,
        from: dateInputToEpochMs(applied.from, "start"),
        to: dateInputToEpochMs(applied.to, "end"),
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setExporting(false);
    }
  }

  const hasFilters = Boolean(applied.action || applied.from || applied.to);

  return (
    <div className="cf-card" style={{ marginBottom: 0 }}>
      <div className="cf-card-head">
        <div>
          <div className="cf-ttl">Audit trail</div>
          <div className="cf-sub">Admin actions · last 30 days</div>
        </div>
        <div className="cf-head-aside">
          <button
            type="button"
            className="cf-btn"
            onClick={exportCsv}
            disabled={exporting}
            title="Download the current filtered audit set as CSV"
          >
            {exporting ? "Exporting…" : "Export (CSV)"}
          </button>
        </div>
      </div>

      {/* ── filter bar ──────────────────────────────────────────────────── */}
      <form
        className="cf-flex cf-wrap cf-gap8"
        style={{
          padding: "12px 14px",
          alignItems: "flex-end",
          borderBottom: "1px solid var(--line-faint)",
        }}
        onSubmit={(e) => {
          e.preventDefault();
          applyFilters();
        }}
      >
        <label className="cf-flex" style={{ flexDirection: "column", gap: 4 }}>
          <span className="cf-faint" style={{ fontSize: 12 }}>
            Action
          </span>
          <input
            type="text"
            className="cf-input"
            placeholder="e.g. settings.update"
            value={actionInput}
            onChange={(e) => setActionInput(e.target.value)}
            style={{ minWidth: 200 }}
          />
        </label>
        <label className="cf-flex" style={{ flexDirection: "column", gap: 4 }}>
          <span className="cf-faint" style={{ fontSize: 12 }}>
            From
          </span>
          <input
            type="date"
            className="cf-input"
            value={fromInput}
            max={toInput || undefined}
            onChange={(e) => setFromInput(e.target.value)}
          />
        </label>
        <label className="cf-flex" style={{ flexDirection: "column", gap: 4 }}>
          <span className="cf-faint" style={{ fontSize: 12 }}>
            To
          </span>
          <input
            type="date"
            className="cf-input"
            value={toInput}
            min={fromInput || undefined}
            onChange={(e) => setToInput(e.target.value)}
          />
        </label>
        <button type="submit" className="cf-btn cf-primary">
          Apply
        </button>
        {hasFilters && (
          <button type="button" className="cf-btn cf-ghost" onClick={clearFilters}>
            Clear
          </button>
        )}
      </form>

      {entries.length === 0 && page === 1 && total === 0 ? (
        <div
          className="cf-faint cf-center"
          style={{ padding: "48px 20px", fontSize: 13 }}
        >
          {hasFilters
            ? "No audit entries match these filters."
            : "No admin actions recorded yet."}
        </div>
      ) : (
        <table className="cf-table">
          <thead>
            <tr>
              <th>When</th>
              <th>Action</th>
              <th>Actor</th>
              <th>Target</th>
              <th>Payload</th>
            </tr>
          </thead>
          <tbody>
            {entries.map((e) => (
              <tr key={e.id}>
                <td className="cf-faint" style={{ whiteSpace: "nowrap" }}>
                  {formatWhen(e.created_at)}
                </td>
                <td className="cf-mono" style={{ whiteSpace: "nowrap" }}>
                  {e.action}
                </td>
                <td className="cf-muted" style={{ whiteSpace: "nowrap" }}>
                  {e.actor_name ??
                    (e.actor_user_id != null
                      ? `user #${e.actor_user_id}`
                      : "—")}
                </td>
                <td className="cf-muted" style={{ whiteSpace: "nowrap" }}>
                  {e.target_kind ?? "—"}
                  {e.target_id ? (
                    <span className="cf-faint"> #{e.target_id}</span>
                  ) : null}
                </td>
                <td style={{ verticalAlign: "top" }}>
                  <Payload raw={e.payload_json} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      <div
        style={{
          padding: "4px 14px 14px",
          borderTop: "1px solid var(--line-faint)",
        }}
      >
        {error && (
          <div
            className="cf-pill cf-err"
            style={{ margin: "8px 0", display: "inline-flex" }}
          >
            <span className="cf-dot" />
            {error}
          </div>
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
  if (!raw) return <span className="cf-faint">—</span>;
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
      <pre
        className="cf-mono cf-muted"
        style={{
          maxWidth: "28rem",
          overflowX: "auto",
          whiteSpace: "pre-wrap",
          wordBreak: "break-word",
          fontSize: 12,
          margin: 0,
        }}
      >
        {pretty}
      </pre>
    );
  }
  return (
    <code className="cf-mono cf-muted" style={{ fontSize: 12 }}>
      {raw}
    </code>
  );
}

function formatWhen(epochMs: number): string {
  const d = new Date(epochMs);
  return d.toLocaleString();
}

/// Convert a `<input type="date">` value (`YYYY-MM-DD`, local) to an epoch-ms
/// bound. `start` → local 00:00:00.000 of that day; `end` → local
/// 23:59:59.999 so the upper bound is inclusive of the whole day. Returns
/// `undefined` for an empty/blank value so the param is dropped.
function dateInputToEpochMs(
  value: string,
  edge: "start" | "end",
): number | undefined {
  if (!value) return undefined;
  const [y, m, d] = value.split("-").map(Number);
  if (!y || !m || !d) return undefined;
  const date =
    edge === "start"
      ? new Date(y, m - 1, d, 0, 0, 0, 0)
      : new Date(y, m - 1, d, 23, 59, 59, 999);
  const ms = date.getTime();
  return Number.isFinite(ms) ? ms : undefined;
}

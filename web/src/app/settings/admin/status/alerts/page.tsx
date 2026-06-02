import { admin as adminApi } from "@/lib/chimpflix-api";
import { formatDateTime } from "@/lib/format";

/// Server-side admin alerts surface, rendered in the console design
/// language: a "Warnings & errors" card (≥ WARN log lines) over an
/// "Audit feed" card (admin actions). Read-only — no client state.
export default async function AdminAlertsPage() {
  const data = await adminApi.alerts({ limit: 50 });

  return (
    <div>
      {/* ── warnings & errors ───────────────────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Warnings &amp; errors</div>
            <div className="cf-sub">≥ WARN, last 50</div>
          </div>
        </div>
        {data.log_alerts.length === 0 ? (
          <div className="cf-card-body cf-pad cf-faint" style={{ fontSize: 13 }}>
            No warnings or errors logged recently.
          </div>
        ) : (
          <table className="cf-table">
            <thead>
              <tr>
                <th>When</th>
                <th>Level</th>
                <th>Source</th>
                <th>Message</th>
              </tr>
            </thead>
            <tbody>
              {data.log_alerts.map((l, i) => (
                <tr key={i}>
                  <td className="cf-faint">{formatDateTime(l.timestamp_ms)}</td>
                  <td>
                    <span
                      className={`cf-pill ${l.level === "ERROR" ? "cf-err" : "cf-warn"}`}
                      style={{ padding: "1px 7px" }}
                    >
                      <span className="cf-dot" />
                      {l.level}
                    </span>
                  </td>
                  <td className="cf-mono">{l.target}</td>
                  <td>{l.message}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>

      {/* ── audit feed ──────────────────────────────────────────────── */}
      <div className="cf-card" style={{ marginBottom: 0 }}>
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Audit feed</div>
            <div className="cf-sub">Admin actions</div>
          </div>
        </div>
        {data.audit.length === 0 ? (
          <div className="cf-card-body cf-pad cf-faint" style={{ fontSize: 13 }}>
            No admin actions recorded.
          </div>
        ) : (
          <table className="cf-table">
            <thead>
              <tr>
                <th>When</th>
                <th>Action</th>
                <th>Actor</th>
                <th>Target</th>
              </tr>
            </thead>
            <tbody>
              {data.audit.map((e) => (
                <tr key={e.id}>
                  <td className="cf-faint">{formatDateTime(e.created_at)}</td>
                  <td className="cf-mono">{e.action}</td>
                  <td>user #{e.actor_user_id ?? "?"}</td>
                  <td className="cf-muted">
                    {e.target_kind ?? ""}
                    {e.target_id ? ` #${e.target_id}` : ""}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}

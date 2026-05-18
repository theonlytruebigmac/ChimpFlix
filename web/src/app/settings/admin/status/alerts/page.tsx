import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";

export default async function AdminAlertsPage() {
  const data = await adminApi.alerts({ limit: 50 });
  return (
    <div>
      <AdminPageHeader
        eyebrow="Status"
        title="Alerts"
        description="Recent warnings/errors from the server, plus the audit feed of admin actions."
      />

      <section className="space-y-2 rounded-lg border border-white/10 bg-white/2 p-4">
        <h2 className="text-sm font-semibold uppercase tracking-wider text-white/40">
          Log alerts (≥ WARN)
        </h2>
        {data.log_alerts.length === 0 ? (
          <div className="rounded border border-dashed border-white/10 p-6 text-center text-xs text-white/40">
            No warnings or errors logged recently.
          </div>
        ) : (
          <div className="overflow-hidden rounded border border-white/10">
            <table className="w-full text-xs">
              <thead className="bg-white/5 text-left text-white/40">
                <tr>
                  <th className="px-3 py-1.5">When</th>
                  <th className="px-3 py-1.5">Level</th>
                  <th className="px-3 py-1.5">Source</th>
                  <th className="px-3 py-1.5">Message</th>
                </tr>
              </thead>
              <tbody>
                {data.log_alerts.map((l, i) => (
                  <tr key={i} className="border-t border-white/5">
                    <td className="whitespace-nowrap px-3 py-1.5 text-white/60">
                      {new Date(l.timestamp_ms).toLocaleString()}
                    </td>
                    <td
                      className={`whitespace-nowrap px-3 py-1.5 font-mono ${l.level === "ERROR" ? "text-red-400" : "text-amber-300"}`}
                    >
                      {l.level}
                    </td>
                    <td className="px-3 py-1.5 font-mono text-white/60">
                      {l.target}
                    </td>
                    <td className="px-3 py-1.5 text-white/80">{l.message}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>

      <section className="mt-6 space-y-2 rounded-lg border border-white/10 bg-white/2 p-4">
        <h2 className="text-sm font-semibold uppercase tracking-wider text-white/40">
          Audit feed (admin actions)
        </h2>
        {data.audit.length === 0 ? (
          <div className="rounded border border-dashed border-white/10 p-6 text-center text-xs text-white/40">
            No admin actions recorded.
          </div>
        ) : (
          <div className="overflow-hidden rounded border border-white/10">
            <table className="w-full text-xs">
              <thead className="bg-white/5 text-left text-white/40">
                <tr>
                  <th className="px-3 py-1.5">When</th>
                  <th className="px-3 py-1.5">Action</th>
                  <th className="px-3 py-1.5">Actor</th>
                  <th className="px-3 py-1.5">Target</th>
                </tr>
              </thead>
              <tbody>
                {data.audit.map((e) => (
                  <tr key={e.id} className="border-t border-white/5">
                    <td className="whitespace-nowrap px-3 py-1.5 text-white/60">
                      {new Date(e.created_at).toLocaleString()}
                    </td>
                    <td className="whitespace-nowrap px-3 py-1.5 font-mono text-white/70">
                      {e.action}
                    </td>
                    <td className="whitespace-nowrap px-3 py-1.5 text-white/60">
                      user #{e.actor_user_id ?? "?"}
                    </td>
                    <td className="whitespace-nowrap px-3 py-1.5 text-white/60">
                      {e.target_kind ?? ""}
                      {e.target_id ? ` #${e.target_id}` : ""}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
    </div>
  );
}

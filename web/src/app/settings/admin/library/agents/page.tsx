import Link from "next/link";
import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";

export default async function AdminAgentsPage() {
  const { agents } = await adminApi.agents.list();
  return (
    <div>
      <AdminPageHeader
        eyebrow="Library"
        title="Metadata Agents"
        description={
          <>
            Available metadata providers. Set the priority order per library
            on the{" "}
            <Link
              href="/settings/admin/library/libraries"
              className="text-white underline hover:text-white/80"
            >
              Libraries
            </Link>{" "}
            page.
          </>
        }
      />
      <div className="overflow-hidden rounded-lg border border-white/10">
        <table className="w-full text-sm">
          <thead className="bg-white/5 text-left text-xs uppercase tracking-wider text-white/40">
            <tr>
              <th className="px-4 py-2">Agent</th>
              <th className="px-4 py-2">Supports</th>
              <th className="px-4 py-2">Status</th>
            </tr>
          </thead>
          <tbody>
            {agents.map((a) => (
              <tr key={a.name} className="border-t border-white/5">
                <td className="px-4 py-3">
                  <div className="font-medium">{a.display_name}</div>
                  <div className="font-mono text-xs text-white/40">{a.name}</div>
                </td>
                <td className="px-4 py-3 text-white/70">
                  {a.supported_kinds.join(", ")}
                </td>
                <td className="px-4 py-3">
                  {a.configured ? (
                    <span className="rounded bg-emerald-500/15 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-emerald-300">
                      Configured
                    </span>
                  ) : (
                    <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-amber-300">
                      Not configured
                    </span>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

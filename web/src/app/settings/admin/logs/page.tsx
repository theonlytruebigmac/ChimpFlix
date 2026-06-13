import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminLogsTabs } from "@/components/admin/AdminLogsTabs";
import { AdminLogsClient } from "@/components/admin/AdminLogsClient";
import { AdminAuditClient } from "@/components/admin/AdminAuditClient";

export const dynamic = "force-dynamic";

const TAB_IDS = ["logs", "audit"];

/// Logs & audit — the live application log tail and the admin audit trail as
/// tabs. Promoted out of Maintenance into its own top-level section.
export default async function AdminLogsAndAuditPage({
  searchParams,
}: {
  searchParams: Promise<{ tab?: string }>;
}) {
  const sp = await searchParams;
  const initialTab = TAB_IDS.includes(sp?.tab ?? "")
    ? (sp.tab as string)
    : "logs";
  // Match the client's INFO floor so the first paint isn't a flash of
  // unfiltered TRACE/DEBUG that the client's first effect then drops.
  const [logs, audit] = await Promise.all([
    adminApi.logs({ level: "INFO", limit: 200 }),
    adminApi.audit.list({ limit: 50 }),
  ]);
  return (
    <AdminLogsTabs
      initialTab={initialTab}
      logs={<AdminLogsClient initial={logs.lines} />}
      audit={<AdminAuditClient initial={audit} />}
    />
  );
}

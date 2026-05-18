import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminLogsClient } from "@/components/admin/AdminLogsClient";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";

export default async function AdminLogsPage() {
  const initial = await adminApi.logs({ limit: 200 });
  return (
    <div>
      <AdminPageHeader
        eyebrow="Maintenance"
        title="Logs"
        description="Live tail from the server's tracing buffer (last ~5,000 lines). Filter by minimum severity; refreshes every 3 seconds."
      />
      <AdminLogsClient initial={initial.lines} />
    </div>
  );
}

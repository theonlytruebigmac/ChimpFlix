import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminLogsClient } from "@/components/admin/AdminLogsClient";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";

export default async function AdminLogsPage() {
  // Fetch with the same `level=INFO` floor the client picks on mount.
  // Without it, the server sends every level (including TRACE/DEBUG),
  // the page paints them on first frame, then the client's first
  // effect re-fetches at INFO and the unfiltered noise disappears in a
  // visible flash. Matching the default here makes the flash go away.
  const initial = await adminApi.logs({ level: "INFO", limit: 200 });
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
